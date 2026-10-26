//! Pure construction of a `RequestWithdrawal`, its exit summary and the
//! wallet-send UTXO filter (INC-I-257, INC-I-262). No I/O, no RPC, no signing.

use crypto::{Hash, PublicKey};
use doli_core::transaction::{Input, Output, OutputType, Transaction};
use doli_core::types::UNITS_PER_COIN;
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

/// The smallest offered coin above `fee`, independent of the offered order.
fn smallest_coin_above(utxos: &[FeeUtxo], fee: u64) -> Option<&FeeUtxo> {
    utxos
        .iter()
        .filter(|u| u.amount > fee)
        .min_by_key(|u| (u.amount, *u.tx_hash.as_bytes(), u.output_index))
}

/// Builds the unsigned withdrawal: one locked Unbonding output to the
/// producer's own address at/above the gate, today's Normal payout below it.
/// At/above the gate with no Bond inputs, one coin (the smallest above the fee)
/// funds the fee and the payout, because that payout is locked.
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
            // A zero-value payout is refused, so the coin must EXCEED the fee.
            let Some(coin) = smallest_coin_above(&spec.fee_utxos, spec.fee) else {
                return Err(format!(
                    "insufficient spendable balance for the tx fee and payout: need one coin above {}, have none",
                    spec.fee
                ));
            };
            inputs.push(Input::new(coin.tx_hash, coin.output_index));
            gross = gross.saturating_add(coin.amount);
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

fn format_doli(units: u64) -> String {
    format!(
        "{}.{:08} DOLI",
        units / UNITS_PER_COIN,
        units % UNITS_PER_COIN
    )
}

/// The exit summary for `tx`, built from `spec` at `best_height`.
/// A zero-Bond exit at/above the gate reports the locked amount, the lock
/// height and the offered coins that stay spendable; every other exit prints
/// the received amount, the penalty and, at/above the gate, the lock height.
pub fn exit_summary_lines(
    params: &NetworkParams,
    best_height: u64,
    spec: &WithdrawalSpec,
    tx: &Transaction,
    total_penalty: u64,
) -> Vec<String> {
    let gated = unbonding_gate_open(params, best_height);
    let lock_line = format!(
        "Unbonding: the payout is locked until height {} (unbonding period {} blocks).",
        unbonding_lock_until(params, best_height),
        params.unbonding_period
    );
    if gated && spec.bond_inputs.is_empty() {
        let locked: u64 = tx.outputs.iter().map(|o| o.amount).sum();
        let spendable: u64 = spec
            .fee_utxos
            .iter()
            .filter(|u| {
                !tx.inputs
                    .iter()
                    .any(|i| i.prev_tx_hash == u.tx_hash && i.output_index == u.output_index)
            })
            .map(|u| u.amount)
            .sum();
        return vec![
            format!(
                "No Bond is withdrawn. This exit locks {} from one coin.",
                format_doli(locked)
            ),
            lock_line,
            format!("Stays spendable: {}", format_doli(spendable)),
        ];
    }
    let mut lines = vec![format!("You receive: {}", format_doli(spec.total_net))];
    if total_penalty > 0 {
        lines.push(format!("Penalty burned: {}", format_doli(total_penalty)));
        let pct = u128::from(total_penalty) * 100
            / (u128::from(spec.total_net) + u128::from(total_penalty));
        if pct >= 50 {
            lines.push(format!(
                "WARNING: High penalty — {}% of bond value will be burned.",
                pct
            ));
        }
    } else {
        lines.push("No penalty — all bonds fully vested.".to_string());
    }
    if gated {
        lines.push(lock_line);
    }
    lines
}

/// Whether wallet send may use a UTXO of this RPC type name.
pub fn send_selectable(output_type: &str, spendable: bool) -> bool {
    matches!(output_type, "normal" | "unbonding") && spendable
}
