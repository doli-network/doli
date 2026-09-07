//! INC-I-180 M2 / S1 — the M1 withdrawal rule table, evaluated during block
//! assembly (INV-PROD-003, INV-VALIDATION-001).
//!
//! Refusal is a SKIP in the selection loop. A builder weaker than the gate
//! deterministically assembles blocks every node rejects, including its own,
//! and pays for it with `rollback_one_block()` on unauthenticated demand.
//!
//! In-block state is carried across the loop, so R1/R2 see the same allowance
//! and R4 the same lower-index hashes the gate will see. NODE-LOCAL policy:
//! the rules are the gate's, but skipping is not a consensus verdict.

use std::collections::{HashMap, HashSet};

use crypto::{Hash, PublicKey};
use doli_core::network::Network;
use doli_core::transaction::{OutputType, Transaction, TxType};
use mempool::vesting_bound::vesting_bound_verdict;
use mempool::{HoldingsLookup, ProducerHoldings};
use storage::producer::{resolve_withdrawal_inputs, WithdrawalInputs};
use storage::{ProducerSet, UtxoSet};
use tracing::debug;

/// INC-I-171 M5 — the vesting window and the slot the builder evaluates at,
/// read once at `WithdrawalParity::new`. The empty `Default` window keeps the
/// arm dormant for any construction that does not supply one.
#[derive(Clone, Copy, Default)]
pub(super) struct VestingGate {
    slot: u32,
    quarter_slots: u64,
    activation: u64,
    disable: u64,
}

impl VestingGate {
    /// INV-VEST-007: outside the window the arm costs nothing, not even the
    /// input resolution.
    fn open_at(&self, height: u64) -> bool {
        height >= self.activation && height < self.disable
    }

    pub(super) fn at(slot: u32, network: Network) -> Self {
        let params = network.params();
        Self {
            slot,
            quarter_slots: params.vesting_quarter_slots,
            activation: params.inc_i_171_vesting_penalty_activation_height,
            disable: params.inc_i_171_vesting_penalty_disable_height,
        }
    }
}

#[derive(Default)]
pub(super) struct WithdrawalParity {
    active: bool,
    /// INC-I-203: the AddBond arm's own gate. Independent of `active`.
    addbond_ah: u64,
    height: u64,
    holdings: HashMap<PublicKey, ProducerHoldings>,
    /// Keys the flushed set does not carry, with their `pending_addbond_count`.
    absent: HashMap<PublicKey, u32>,
    in_block_addbond: HashMap<PublicKey, u32>,
    in_block_withdrawn: HashMap<PublicKey, u32>,
    earlier_hashes: HashSet<Hash>,
    owned_live_bonds: HashMap<Hash, u32>,
    /// INC-I-171: independent of `active`, exactly as `addbond_ah` is.
    vesting: VestingGate,
}

impl WithdrawalParity {
    pub(super) fn new(active: bool, addbond_ah: u64, height: u64, vesting: VestingGate) -> Self {
        Self {
            active,
            addbond_ah,
            height,
            vesting,
            ..Self::default()
        }
    }

    pub(super) fn is_active(&self) -> bool {
        self.active
    }

    /// INC-I-203 REQ-BOND-005. Gated only by its own height; inheriting
    /// `active` would silence it on every band the INC-I-180 gate predates.
    pub(super) fn addbond_active(&self) -> bool {
        self.height >= self.addbond_ah
    }

    /// Either arm needs the producer guard read and the in-block tally kept.
    fn any_active(&self) -> bool {
        self.active || self.addbond_active()
    }

    /// Resolve every producer the candidate set names, under the producer guard
    /// alone. The selection loop then runs under the UTXO guard alone: one lock
    /// at a time, which is what keeps this off the apply/rollback lock cycle.
    pub(super) fn load(&mut self, producers: &ProducerSet, txs: &[Transaction]) {
        if !self.any_active() {
            return;
        }
        for tx in txs {
            let named = match tx.tx_type {
                TxType::RequestWithdrawal => {
                    tx.withdrawal_request_data().map(|d| d.producer_pubkey)
                }
                TxType::Exit => tx.exit_data().map(|d| d.public_key),
                TxType::AddBond => tx.add_bond_data().map(|d| d.producer_pubkey),
                _ => None,
            };
            let Some(pk) = named else { continue };
            if self.holdings.contains_key(&pk) || self.absent.contains_key(&pk) {
                continue;
            }
            match mempool::holdings::of_producer_set(producers, &pk) {
                HoldingsLookup::Found(h) => {
                    self.holdings.insert(pk, h);
                }
                HoldingsLookup::Unregistered { pending_addbond } => {
                    self.absent.insert(pk, pending_addbond);
                }
                HoldingsLookup::Unavailable => {}
            }
        }
    }

    /// `Err` means the assembled block would be rejected — the caller skips.
    ///
    /// One `resolve_withdrawal_inputs` pass feeds both arms (INV-VEST-008). The
    /// vesting arm runs whatever `active` says: its window is its own.
    pub(super) fn allow(&mut self, tx: &Transaction, utxo: &UtxoSet) -> Result<(), String> {
        if tx.tx_type == TxType::AddBond {
            return self.allow_add_bond(tx);
        }
        if tx.tx_type != TxType::RequestWithdrawal {
            return Ok(());
        }
        let Some(wd) = tx.withdrawal_request_data() else {
            return Ok(());
        };
        if !self.active && !self.vesting.open_at(self.height) {
            return Ok(());
        }
        let pk = wd.producer_pubkey;
        let owner = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes());
        let resolved = resolve_withdrawal_inputs(tx, utxo, &owner);
        if self.active {
            self.allow_holdings(tx, pk, wd.bond_count, &owner, &resolved, utxo)?;
        }
        self.allow_vesting(tx, &resolved)
    }

    /// INC-I-171 M5 (REQ-VEST-008). A refusal is a SKIP upstream, never a bail.
    fn allow_vesting(&self, tx: &Transaction, resolved: &WithdrawalInputs) -> Result<(), String> {
        let verdict = vesting_bound_verdict(
            tx,
            resolved,
            self.vesting.slot,
            self.height,
            self.vesting.quarter_slots,
            self.vesting.activation,
            self.vesting.disable,
        );
        if let Err(reason) = &verdict {
            debug!(
                "INC-I-171: skipping withdrawal {} at height {} — {}",
                tx.hash(),
                self.height,
                reason
            );
        }
        verdict
    }

    /// INC-I-180 M2 / S1, unchanged: R1/R2/R4 against the in-block tallies.
    fn allow_holdings(
        &mut self,
        tx: &Transaction,
        pk: PublicKey,
        declared: u32,
        owner: &Hash,
        resolved: &WithdrawalInputs,
        utxo: &UtxoSet,
    ) -> Result<(), String> {
        let Some(info) = self.holdings.get(&pk).copied() else {
            return Err(format!(
                "[ECON_WITHDRAWAL_UNKNOWN_PRODUCER] producer={pk:?}"
            ));
        };
        let allowance = info.allowance_with(
            self.in_block_addbond.get(&pk).copied().unwrap_or(0),
            self.in_block_withdrawn.get(&pk).copied().unwrap_or(0),
        );
        if declared > allowance {
            return Err(format!(
                "[ECON_WITHDRAWAL_OVER_HOLDINGS] declares {declared} against allowance {allowance}"
            ));
        }
        if tx
            .inputs
            .iter()
            .any(|inp| self.earlier_hashes.contains(&inp.prev_tx_hash))
        {
            return Err(
                "[ECON_WITHDRAWAL_SAME_BLOCK_INPUT] spends an output created \
                        by an earlier transaction of this same block"
                    .to_string(),
            );
        }

        let (bond_inputs, all_bond_inputs) = (resolved.owned_bonds, resolved.all_bonds);
        if all_bond_inputs != bond_inputs {
            return Err(format!(
                "[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH] {bond_inputs} of {all_bond_inputs} \
                 Bond inputs are owned by the named producer"
            ));
        }
        if declared == allowance && declared > 0 {
            let owned = *self.owned_live_bonds.entry(*owner).or_insert_with(|| {
                u32::try_from(utxo.get_bond_entries(owner).len()).unwrap_or(u32::MAX)
            });
            if bond_inputs != owned {
                return Err(format!(
                    "[ECON_WITHDRAWAL_INCOMPLETE_DRAIN] full allowance of {declared} but \
                     spends {bond_inputs} of {owned} owned Bond UTXOs"
                ));
            }
        } else if declared != bond_inputs {
            return Err(format!(
                "[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH] declares {declared} but spends \
                 {bond_inputs} owned Bond UTXO inputs"
            ));
        }
        Ok(())
    }

    /// INC-I-203 REQ-BOND-001. `load()` takes a blocking `producer_set.read()`,
    /// so the builder always HAS a source: a key the flushed set does not carry
    /// is `Unregistered` and is evaluated with `current = 0`, exactly as the
    /// gate does. `Unavailable` is left only for a key never resolved at all.
    fn allow_add_bond(&self, tx: &Transaction) -> Result<(), String> {
        let Some(ab) = tx.add_bond_data() else {
            return Ok(());
        };
        let pk = ab.producer_pubkey;
        let lookup = match self.holdings.get(&pk).copied() {
            Some(h) => HoldingsLookup::Found(h),
            None => match self.absent.get(&pk).copied() {
                Some(pending_addbond) => HoldingsLookup::Unregistered { pending_addbond },
                None => HoldingsLookup::Unavailable,
            },
        };
        mempool::addbond_cap::addbond_cap_verdict(
            tx,
            &lookup,
            self.in_block_addbond.get(&pk).copied().unwrap_or(0),
            self.height,
            self.addbond_ah,
        )
    }

    /// Record a transaction that WAS selected. Mirrors the gate's per-type
    /// accounting, including its `+=` on repeated Exits for one producer.
    pub(super) fn accept(&mut self, tx: &Transaction) {
        if !self.any_active() {
            return;
        }
        self.earlier_hashes.insert(tx.hash());
        match tx.tx_type {
            TxType::AddBond => {
                let Some(ab) = tx.add_bond_data() else { return };
                let bonds = u32::try_from(
                    tx.outputs
                        .iter()
                        .filter(|o| o.output_type == OutputType::Bond)
                        .count(),
                )
                .unwrap_or(u32::MAX);
                let prior = self
                    .in_block_addbond
                    .get(&ab.producer_pubkey)
                    .copied()
                    .unwrap_or(0);
                self.in_block_addbond
                    .insert(ab.producer_pubkey, prior.saturating_add(bonds));
            }
            TxType::Exit => {
                let Some(ed) = tx.exit_data() else { return };
                let held = self
                    .holdings
                    .get(&ed.public_key)
                    .map(|h| h.bond_count)
                    .unwrap_or(0);
                let prior = self
                    .in_block_withdrawn
                    .get(&ed.public_key)
                    .copied()
                    .unwrap_or(0);
                self.in_block_withdrawn
                    .insert(ed.public_key, prior.saturating_add(held));
            }
            TxType::RequestWithdrawal => {
                let Some(wd) = tx.withdrawal_request_data() else {
                    return;
                };
                let prior = self
                    .in_block_withdrawn
                    .get(&wd.producer_pubkey)
                    .copied()
                    .unwrap_or(0);
                self.in_block_withdrawn
                    .insert(wd.producer_pubkey, prior.saturating_add(wd.bond_count));
            }
            _ => {}
        }
    }

    /// The height this predicate was armed for — the block being built.
    pub(super) fn height(&self) -> u64 {
        self.height
    }
}
