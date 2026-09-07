//! INC-I-171 M5 — the INV-VEST-008 row table.
//!
//! A submodule of the SAME test binary, so the per-network `NetworkParams`
//! `OnceLock` the env override arms stays one process (module-size budget).

/// Bond ages, centred inside their quarter so a one-slot build drift cannot
/// move a fixture across a tier boundary.
pub const AGE_Q1: u32 = 30;
pub const AGE_Q2: u32 = 90;
pub const AGE_Q3: u32 = 150;
pub const AGE_Q4: u32 = 210;

pub const PAYOUT_EXCEEDS: &str = "ECON_WITHDRAWAL_PAYOUT_EXCEEDS_NET";
pub const EXTRA_DATA_MALFORMED: &str = "ECON_WITHDRAWAL_BOND_EXTRA_DATA_MALFORMED";

/// One row of the INV-VEST-008 table.
pub struct Row {
    pub label: &'static str,
    pub age: u32,
    pub tag: u8,
    pub malformed: bool,
    /// `None` = the exact bound; `Some(d)` = bound + d; `Payout::Raw` handled by
    /// `raw_value`.
    pub over: i64,
    pub raw_value: bool,
    pub expect_accept: bool,
    pub expect_code: &'static str,
}

pub const ROWS: [Row; 8] = [
    Row {
        label: "Q1 75% honest at the exact bound",
        age: AGE_Q1,
        tag: 0xA1,
        malformed: false,
        over: 0,
        raw_value: false,
        expect_accept: true,
        expect_code: "",
    },
    Row {
        label: "Q2 50% honest at the exact bound",
        age: AGE_Q2,
        tag: 0xA2,
        malformed: false,
        over: 0,
        raw_value: false,
        expect_accept: true,
        expect_code: "",
    },
    Row {
        label: "Q3 25% honest at the exact bound",
        age: AGE_Q3,
        tag: 0xA3,
        malformed: false,
        over: 0,
        raw_value: false,
        expect_accept: true,
        expect_code: "",
    },
    // An exact-bound Q4 payout leaves a ZERO fee, which `minimum_fee()` refuses
    // for its own reason, so this row sits one fee-input below the bound.
    Row {
        label: "Q4 0% honest one fee-input below the bound",
        age: AGE_Q4,
        tag: 0xA4,
        malformed: false,
        over: -1,
        raw_value: false,
        expect_accept: true,
        expect_code: "",
    },
    Row {
        label: "Q1 over-burn one fee-input below the bound",
        age: AGE_Q1,
        tag: 0xA8,
        malformed: false,
        over: -1,
        raw_value: false,
        expect_accept: true,
        expect_code: "",
    },
    Row {
        label: "Q1 over-payout at bound+1",
        age: AGE_Q1,
        tag: 0xA5,
        malformed: false,
        over: 1,
        raw_value: false,
        expect_accept: false,
        expect_code: PAYOUT_EXCEEDS,
    },
    Row {
        label: "Q1 full raw bond value",
        age: AGE_Q1,
        tag: 0xA6,
        malformed: false,
        over: 0,
        raw_value: true,
        expect_accept: false,
        expect_code: PAYOUT_EXCEEDS,
    },
    Row {
        label: "Q1 malformed 3-byte Bond extra_data",
        age: AGE_Q1,
        tag: 0xA7,
        malformed: true,
        over: 0,
        raw_value: false,
        expect_accept: false,
        expect_code: EXTRA_DATA_MALFORMED,
    },
];
