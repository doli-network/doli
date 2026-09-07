//! INC-I-171 M7 — the CLI must consume the core vesting ladder, not re-derive it.
//!
//! OUTPUT CONTRACT: N/A — source-content tripwire (no fn under test); the only output is the
//! assertion verdict over the include_str!() text of bins/cli/src/cmd_producer/common.rs.
//! INPUT PARTITIONS: re-derived (red, today: the ladder arithmetic is inline at common.rs:35)
//! vs single-source (green, after M7).

const COMMON_SRC: &str = include_str!("../../src/cmd_producer/common.rs");

const LADDER_FN: &str = "penalized_bond_net";
const DIVIDE_BY_HUNDRED: &str = concat!("/ ", "100");
const MULTIPLY_BY_PCT: &str = concat!("* ", "pct");

// REQ-VEST-002 (Must) — Decision: reveals that the wallet still owns a second copy of the vesting
// ladder, the drift that lets an operator sign a payout the node then rejects.
#[test]
fn req_vest_002_cli_calls_the_core_ladder_helper() {
    assert!(
        COMMON_SRC.contains(LADDER_FN),
        "REQ-VEST-002: bins/cli/src/cmd_producer/common.rs must call \
         doli_core::validation::vesting::{LADDER_FN} — one ladder implementation, consumed by \
         the node bound and the CLI preview alike"
    );
}

// REQ-VEST-002 (Must) — Decision: reveals a re-introduced inline penalty formula, which is how
// the CLI and the node came to disagree on the same withdrawal in the first place.
#[test]
fn req_vest_002_cli_keeps_no_independent_ladder_arithmetic() {
    assert!(
        !COMMON_SRC.contains(DIVIDE_BY_HUNDRED),
        "REQ-VEST-002: common.rs must not divide a penalty by 100 itself (:35) — the percentage \
         arithmetic belongs to the core helper, which widens to u128"
    );
    assert!(
        !COMMON_SRC.contains(MULTIPLY_BY_PCT),
        "REQ-VEST-002: common.rs must not multiply an amount by a percentage itself (:35) — a u64 \
         multiply wraps, and the percentage must come from the ladder, not from the RPC field"
    );
}
