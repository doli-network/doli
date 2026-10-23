//! Pure construction of a `RequestWithdrawal` and the wallet-send UTXO filter
//! (INC-I-257). No I/O, no RPC, no signing.

use crypto::{Hash, PublicKey};
use doli_core::transaction::{Input, Output, OutputType, Transaction};
use doli_core::validation::bond_owner;
use doli_core::{Network, NetworkParams};

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

/// Params the producer-tx builders use: the node's loader, so devnet env gate overrides apply.
pub fn builder_params(network: Network) -> &'static NetworkParams {
    NetworkParams::load(network)
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

/// Appends `utxos` in order while their total is below `target`; returns the total.
fn take_fee_inputs(utxos: &[FeeUtxo], target: u64, inputs: &mut Vec<Input>) -> u64 {
    let mut total = 0u64;
    for utxo in utxos {
        if total >= target {
            break;
        }
        inputs.push(Input::new(utxo.tx_hash, utxo.output_index));
        total = total.saturating_add(utxo.amount);
    }
    total
}

/// Builds the unsigned withdrawal: one locked Unbonding output to the
/// producer's own address at/above the gate, today's Normal payout below it.
/// At/above the gate with no Bond inputs, fee UTXOs fund the fee and the payout.
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
        let mut gross = spec.total_net;
        if spec.bond_inputs.is_empty() {
            // A zero-value payout is refused, so the inputs must EXCEED the fee.
            let paid_in = take_fee_inputs(&spec.fee_utxos, spec.fee.saturating_add(1), &mut inputs);
            if paid_in <= spec.fee {
                return Err(format!(
                    "insufficient spendable balance for the tx fee and payout: need more than {}, have {}",
                    spec.fee, paid_in
                ));
            }
            gross = gross.saturating_add(paid_in);
        }
        if gross <= spec.fee {
            return Err(format!(
                "withdrawn amount {} does not cover the fee {}",
                gross, spec.fee
            ));
        }
        let mut tx = Transaction::new_request_withdrawal(
            inputs,
            spec.producer,
            spec.bond_count,
            own,
            gross - spec.fee,
        );
        tx.outputs = vec![Output {
            output_type: OutputType::Unbonding,
            amount: gross - spec.fee,
            pubkey_hash: own,
            lock_until: unbonding_lock_until(params, best_height),
            extra_data: Vec::new(),
        }];
        return Ok(tx);
    }

    let fee_input_total = take_fee_inputs(&spec.fee_utxos, spec.fee, &mut inputs);
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
