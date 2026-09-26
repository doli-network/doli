//! Native DOLI value of an output — the E1 rule of AMM conservation.
//!
//! Native-amount outputs count their `amount`; a Pool output counts its
//! DOLI reserve (`reserve_a`, its `amount` is 0 by design); every other
//! output (tokens, LP shares) counts 0.

use crate::transaction::{Output, OutputType};

/// DOLI value of one output (see module doc).
pub fn doli_value(o: &Output) -> u128 {
    if o.output_type.is_native_amount() {
        o.amount as u128
    } else if o.output_type == OutputType::Pool {
        o.pool_metadata()
            .map(|pm| pm.reserve_a as u128)
            .unwrap_or(0)
    } else {
        0
    }
}

/// Native DOLI destroyed by a tx: Σ doli_value(consumed) - Σ doli_value(outputs).
/// `None` when the outputs carry more DOLI value than the inputs.
pub fn doli_surplus(consumed: &[Output], outputs: &[Output]) -> Option<u64> {
    let inp: u128 = consumed.iter().map(doli_value).sum();
    let out: u128 = outputs.iter().map(doli_value).sum();
    inp.checked_sub(out).map(|d| d.min(u64::MAX as u128) as u64)
}
