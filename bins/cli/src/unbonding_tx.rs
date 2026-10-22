//! Pure construction of a `RequestWithdrawal` and the wallet-send UTXO filter
//! (INC-I-257). No I/O, no RPC, no signing.

use crypto::{Hash, PublicKey};
use doli_core::transaction::{Input, Output, OutputType, Transaction};
use doli_core::validation::bond_owner;
use doli_core::NetworkParams;

/// Extra lock blocks so the tx stays valid while it waits up to this many blocks for inclusion.
pub const UNBONDING_LOCK_MARGIN: u64 = 10;

/// A wallet output offered to pay the fee below the gate.
#[derive(Clone, Debug)]
pub struct FeeUtxo {
    pub tx_hash: Hash,
    pub output_index: u32,
    pub amount: u64,
}

/// Everything the builder needs; the caller resolves it from the wallet and RPC.
#[derive(Clone, Debug)]
pub struct WithdrawalSpec {
    pub producer: PublicKey,
    pub bond_count: u32,
    pub bond_inputs: Vec<(Hash, u32)>,
    pub total_net: u64,
    pub destination: Hash,
    pub fee_utxos: Vec<FeeUtxo>,
    pub fee: u64,
}

/// True when a block mined after `best_height` must carry the gated shape.
pub fn unbonding_gate_open(params: &NetworkParams, best_height: u64) -> bool {
    best_height.saturating_add(1) >= params.inc_i_257_unbonding_activation_height
}

/// Lock height the gated payout carries when built at `best_height`.
pub fn unbonding_lock_until(params: &NetworkParams, best_height: u64) -> u64 {
    best_height
        .saturating_add(params.unbonding_period)
        .saturating_add(UNBONDING_LOCK_MARGIN)
}

/// Builds the unsigned withdrawal: one locked Unbonding output to the
/// producer's own address at/above the gate, today's Normal payout below it.
pub fn build_withdrawal_tx(
    params: &NetworkParams,
    best_height: u64,
    spec: &WithdrawalSpec,
) -> Result<Transaction, String> {
    let mut inputs: Vec<Input> = spec
        .bond_inputs
        .iter()
        .map(|(hash, index)| Input::new(*hash, *index))
        .collect();

    if unbonding_gate_open(params, best_height) {
        let own = bond_owner(&spec.producer);
        if spec.destination != own {
            return Err(
                "the unbonding payout is locked and must go to the producer's own address"
                    .to_string(),
            );
        }
        if spec.total_net <= spec.fee {
            return Err(format!(
                "withdrawn amount {} does not cover the fee {}",
                spec.total_net, spec.fee
            ));
        }
        let mut tx = Transaction::new_request_withdrawal(
            inputs,
            spec.producer,
            spec.bond_count,
            own,
            spec.total_net - spec.fee,
        );
        tx.outputs = vec![Output {
            output_type: OutputType::Unbonding,
            amount: spec.total_net - spec.fee,
            pubkey_hash: own,
            lock_until: unbonding_lock_until(params, best_height),
            extra_data: Vec::new(),
        }];
        return Ok(tx);
    }

    let mut fee_input_total = 0u64;
    for utxo in &spec.fee_utxos {
        if fee_input_total >= spec.fee {
            break;
        }
        inputs.push(Input::new(utxo.tx_hash, utxo.output_index));
        fee_input_total = fee_input_total.saturating_add(utxo.amount);
    }
    if fee_input_total < spec.fee {
        return Err(format!(
            "insufficient spendable balance for the tx fee: need {}, have {}",
            spec.fee, fee_input_total
        ));
    }
    // One output only, so the fee change is rolled into the payout.
    let payout = spec.total_net.saturating_add(fee_input_total - spec.fee);
    Ok(Transaction::new_request_withdrawal(
        inputs,
        spec.producer,
        spec.bond_count,
        spec.destination,
        payout,
    ))
}

/// Whether wallet send may use a UTXO of this RPC type name.
pub fn send_selectable(output_type: &str, spendable: bool) -> bool {
    matches!(output_type, "normal" | "unbonding") && spendable
}
