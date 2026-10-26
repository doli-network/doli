//! Live-defense M1 — the CLI withdrawal and exit builders take the gate
//! heights and the unbonding period from the same loader the node uses, so
//! devnet env overrides reach the CLI while mainnet and testnet stay compiled.
//!
// covers: unbonding_tx withdrawal exit producer_tx network_params env_loader
//
// OUTPUT CONTRACT: fn doli_cli::unbonding_tx::builder_params(Network) -> &'static NetworkParams
//   O1 return: devnet = NetworkParams::load(Devnet) (process env: 257/182 gates, DOLI_UNBONDING_PERIOD);
//      mainnet and testnet = the compiled gates and period whatever DOLI_* variables are set
// OUTPUT CONTRACT: fn build_withdrawal_tx(builder_params(net), best_height, &WithdrawalSpec) -> Result<Transaction, String>
//   O2 return: devnet best+1 < env gate -> one Normal output, lock 0, tail appended by finalize;
//      best+1 >= env gate -> one Unbonding output, lock best + env period + UNBONDING_LOCK_MARGIN;
//      mainnet/testnet serialized bytes equal with and without the variables
// OUTPUT CONTRACT: source of cmd_producer/withdrawal.rs, cmd_producer/exit.rs, crates/producer-tx/src/unbonding_tx.rs,
//   bins/cli/src/unbonding_tx.rs
//   O3 both call sites call builder_params( and never NetworkParams::defaults(; builder_params is
//      defined in the shared unbonding_tx.rs, calls NetworkParams::load( and never load_env_for_network;
//      the CLI unbonding_tx.rs re-exports the shared module and defines no builder_params of its own
// PATHS: P-BELOW best+1 < gate | P-EDGE best+1 == gate | P-ABOVE best+1 > gate | P-LOCKED mainnet, testnet
// MATRIX
//   O1, O2 x {P-BELOW (best 0, gate-2), P-EDGE (gate-1), P-ABOVE (gate+5)} -> ..._devnet_cli_params_follow_env_gate_and_period
//   O1, O2 x P-LOCKED x {best 1_000, gate-1} -> ..._mainnet_testnet_withdrawal_bytes_ignore_env (+ devnet differs: env live)
//   O3 -> ..._call_sites_load_builder_params (+ planted old shape and planted shim-local builder_params flagged)
// INPUT PARTITIONS:
//   child env: four gates = 30, pending-root gate 0, DOLI_UNBONDING_PERIOD = 77 (devnet compiled: gates 0, period 60)
//   spec: one Bond input, one fee UTXO 5_000, fee 1, destination = bond_owner(producer)

use crypto::{Hash, KeyPair};
use doli_cli::producer_tx::finalize_producer_tx;
use doli_cli::unbonding_tx::{
    build_withdrawal_tx, builder_params, FeeUtxo, WithdrawalSpec, UNBONDING_LOCK_MARGIN,
};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::OutputType;
use doli_core::validation::{bond_owner, validate_transaction, ValidationContext};
use doli_core::{Network, NetworkParams};

const CHILD: &str = "LDP_M1_CLI_CHILD";
const MOD: &str = "ldp_m1_cli_gate_source::";
const EXPECT_LOCKED: &str = "LDP_M1_EXPECT_LOCKED_HEX";
const EXPECT_DEVNET: &str = "LDP_M1_EXPECT_DEVNET_COMPILED_HEX";
const GATE: u64 = 30;
const PERIOD: u64 = 77;
const BEST: u64 = 1_000;
const TAIL: usize = 64;
const GATE_ENVS: [&str; 4] = [
    "DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT",
    "DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT",
    "DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT",
    "DOLI_INC_I_182_PRODUCER_AUTH_ACTIVATION_HEIGHT",
];
const LOCKED: [Network; 2] = [Network::Mainnet, Network::Testnet];

fn in_child() -> bool {
    std::env::var_os(CHILD).is_some()
}

/// Re-runs `test` alone with every gate variable set; `NetworkParams::load` caches per process.
fn run_in_child(test: &str, extra: &[(&str, String)]) {
    let mut cmd = std::process::Command::new(std::env::current_exe().expect("current exe"));
    cmd.args([
        &format!("{MOD}{test}"),
        "--exact",
        "--test-threads=1",
        "--nocapture",
    ])
    .env(CHILD, "1")
    .env("DOLI_INC_I_181_PENDING_ROOT_ACTIVATION_HEIGHT", "0")
    .env("DOLI_UNBONDING_PERIOD", PERIOD.to_string());
    for g in GATE_ENVS {
        cmd.env(g, GATE.to_string());
    }
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    if out.status.success() && stdout.contains("1 passed") {
        return;
    }
    panic!(
        "child {test} failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn wallet() -> KeyPair {
    KeyPair::from_seed([0x3C; 32])
}

fn genesis() -> Hash {
    ConsensusParams::devnet().genesis_hash
}

fn spec(network: Network) -> WithdrawalSpec {
    let pk = *wallet().public_key();
    WithdrawalSpec {
        producer: pk,
        bond_count: 1,
        bond_inputs: vec![(Hash::from_bytes([0xB1; 32]), 0)],
        total_net: network.bond_unit(),
        destination: bond_owner(&pk),
        fee_utxos: vec![FeeUtxo {
            tx_hash: Hash::from_bytes([0xF1; 32]),
            output_index: 1,
            amount: 5_000,
        }],
        fee: 1,
    }
}

fn tx_hex(network: Network, params: &NetworkParams, best: u64) -> String {
    let tx = build_withdrawal_tx(params, best, &spec(network))
        .unwrap_or_else(|e| panic!("{network:?} build at best {best}: {e}"));
    hex::encode(tx.serialize())
}

fn ctx(best: u64, p: &NetworkParams) -> ValidationContext {
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, best + 1)
        .with_inc_i_234_activation_height(0)
        .with_inc_i_257_unbonding(p.inc_i_257_unbonding_activation_height, p.unbonding_period)
        .with_inc_i_182_producer_auth(p.inc_i_182_producer_auth_activation_height)
}

// REQ-LDP-001 — Decision: a failure means a devnet CLI still builds the compiled-gate payout while the nodes run an env gate, so GS-028 and GS-029 transactions fail for a tooling reason instead of a protocol one.
#[test]
fn ldp_m1_req_ldp_001_devnet_cli_params_follow_env_gate_and_period() {
    if !in_child() {
        return run_in_child(
            "ldp_m1_req_ldp_001_devnet_cli_params_follow_env_gate_and_period",
            &[],
        );
    }
    let compiled = NetworkParams::defaults(Network::Devnet);
    assert!(
        compiled.inc_i_257_unbonding_activation_height != GATE
            && compiled.unbonding_period != PERIOD,
        "NON-VACUITY: the compiled devnet gate and period must differ from the env values"
    );
    let p = builder_params(Network::Devnet);
    assert_eq!(
        (
            p.inc_i_257_unbonding_activation_height,
            p.inc_i_182_producer_auth_activation_height,
            p.unbonding_period
        ),
        (GATE, GATE, PERIOD),
        "devnet CLI params must carry the env gates and the env unbonding period"
    );
    for (best, want_type, want_lock) in [
        (0, OutputType::Normal, 0),
        (GATE - 2, OutputType::Normal, 0),
        (
            GATE - 1,
            OutputType::Unbonding,
            GATE - 1 + PERIOD + UNBONDING_LOCK_MARGIN,
        ),
        (
            GATE + 5,
            OutputType::Unbonding,
            GATE + 5 + PERIOD + UNBONDING_LOCK_MARGIN,
        ),
    ] {
        let mut tx = build_withdrawal_tx(p, best, &spec(Network::Devnet))
            .unwrap_or_else(|e| panic!("best {best}: devnet build: {e}"));
        let out = &tx.outputs[0];
        assert_eq!(
            (tx.outputs.len(), out.output_type, out.lock_until),
            (1, want_type, want_lock),
            "best {best}: payout shape against the env gate {GATE} and env period {PERIOD}"
        );
        let base = tx.extra_data.len();
        finalize_producer_tx(&mut tx, &wallet(), &genesis());
        assert_eq!(
            tx.extra_data.len(),
            base + TAIL,
            "best {best}: the producer-auth tail is appended"
        );
        let r = validate_transaction(&tx, &ctx(best, p));
        assert!(
            r.is_ok(),
            "best {best}: a node on the same env gates refuses the CLI tx: {r:?}"
        );
    }
}

// REQ-LDP-002 — Decision: a failure means a DOLI_* variable in an operator shell changes the withdrawal a mainnet or testnet wallet signs, so the CLI fix opened a gate-moving path outside devnet.
#[test]
fn ldp_m1_req_ldp_002_mainnet_testnet_withdrawal_bytes_ignore_env() {
    let name = "ldp_m1_req_ldp_002_mainnet_testnet_withdrawal_bytes_ignore_env";
    if !in_child() {
        let locked: Vec<String> = LOCKED
            .iter()
            .flat_map(|n| {
                let d = NetworkParams::defaults(*n);
                [tx_hex(*n, &d, BEST), tx_hex(*n, &d, GATE - 1)]
            })
            .collect();
        let devnet = tx_hex(
            Network::Devnet,
            &NetworkParams::defaults(Network::Devnet),
            BEST,
        );
        return run_in_child(
            name,
            &[(EXPECT_LOCKED, locked.join(",")), (EXPECT_DEVNET, devnet)],
        );
    }
    let want: Vec<String> = std::env::var(EXPECT_LOCKED)
        .expect("parent expectation")
        .split(',')
        .map(str::to_string)
        .collect();
    assert_eq!(want.len(), 4, "parent sent two heights per locked network");
    for (i, n) in LOCKED.iter().enumerate() {
        let (p, d) = (builder_params(*n), NetworkParams::defaults(*n));
        assert_eq!(
            (
                p.inc_i_236_block_signature_activation_height,
                p.inc_i_255_slashing_activation_height,
                p.inc_i_257_unbonding_activation_height,
                p.inc_i_182_producer_auth_activation_height,
                p.unbonding_period
            ),
            (
                d.inc_i_236_block_signature_activation_height,
                d.inc_i_255_slashing_activation_height,
                d.inc_i_257_unbonding_activation_height,
                d.inc_i_182_producer_auth_activation_height,
                d.unbonding_period
            ),
            "{n:?}: an env variable moved a compiled gate or the unbonding period"
        );
        assert_eq!(
            tx_hex(*n, p, BEST),
            want[2 * i],
            "{n:?}: withdrawal bytes at best {BEST} differ with DOLI_* variables set"
        );
        assert_eq!(
            tx_hex(*n, p, GATE - 1),
            want[2 * i + 1],
            "{n:?}: withdrawal bytes at best {} differ with DOLI_* variables set",
            GATE - 1
        );
    }
    let devnet_compiled = std::env::var(EXPECT_DEVNET).expect("parent devnet bytes");
    assert_ne!(
        tx_hex(Network::Devnet, builder_params(Network::Devnet), BEST),
        devnet_compiled,
        "NON-VACUITY: the variables must be live in this process (devnet bytes change)"
    );
}

const WITHDRAWAL_SRC: &str = include_str!("../../src/cmd_producer/withdrawal.rs");
const EXIT_SRC: &str = include_str!("../../src/cmd_producer/exit.rs");
const CLI_UNBONDING_SRC: &str = include_str!("../../src/unbonding_tx.rs");
const SHARED_UNBONDING_SRC: &str =
    include_str!("../../../../crates/producer-tx/src/unbonding_tx.rs");

fn code(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn call_site_violations(src: &str) -> Vec<&'static str> {
    let c = code(src);
    [
        (
            "no NetworkParams::defaults( call",
            !c.contains("NetworkParams::defaults("),
        ),
        ("calls builder_params(", c.contains("builder_params(")),
    ]
    .into_iter()
    .filter(|(_, ok)| !ok)
    .map(|(what, _)| what)
    .collect()
}

fn helper_violations(src: &str) -> Vec<&'static str> {
    let c = code(src);
    [
        (
            "defines pub fn builder_params(",
            c.contains("pub fn builder_params("),
        ),
        (
            "delegates to NetworkParams::load(",
            c.contains("NetworkParams::load("),
        ),
        (
            "never reads a data-dir .env",
            !c.contains("load_env_for_network"),
        ),
    ]
    .into_iter()
    .filter(|(_, ok)| !ok)
    .map(|(what, _)| what)
    .collect()
}

fn shim_violations(src: &str) -> Vec<&'static str> {
    let c = code(src);
    [
        (
            "re-exports doli_producer_tx::unbonding_tx",
            c.contains("pub use doli_producer_tx::unbonding_tx::"),
        ),
        (
            "defines no builder_params of its own",
            !c.contains("fn builder_params("),
        ),
    ]
    .into_iter()
    .filter(|(_, ok)| !ok)
    .map(|(what, _)| what)
    .collect()
}

// REQ-LDP-001 — Decision: a failure names the CLI call site that still reads compiled params, or a helper that reads a data-dir .env, so a CLI and a node on one devnet can still disagree on the gate.
#[test]
fn ldp_m1_req_ldp_001_call_sites_load_builder_params() {
    let old = "let net_params = NetworkParams::defaults(network);";
    assert_eq!(
        call_site_violations(old).len(),
        2,
        "NON-VACUITY: the scan flags the old call-site shape"
    );
    let planted = "pub fn builder_params(n: Network) -> &'static NetworkParams { load_env_for_network(\"devnet\", p); NetworkParams::load(n) }";
    assert_eq!(
        helper_violations(planted),
        vec!["never reads a data-dir .env"],
        "NON-VACUITY: the scan flags a helper that reads a data-dir .env"
    );
    for (file, src) in [
        ("cmd_producer/withdrawal.rs", WITHDRAWAL_SRC),
        ("cmd_producer/exit.rs", EXIT_SRC),
    ] {
        let bad = call_site_violations(src);
        assert!(bad.is_empty(), "{file} violates: {bad:?}");
    }
    let pure_shim = "pub use doli_producer_tx::unbonding_tx::*;";
    assert!(
        shim_violations(pure_shim).is_empty(),
        "NON-VACUITY: a pure re-export shim passes the shim scan"
    );
    let local = format!("{pure_shim}\npub fn builder_params(n: Network) -> &'static NetworkParams {{ NetworkParams::load(n) }}");
    assert_eq!(
        shim_violations(&local),
        vec!["defines no builder_params of its own"],
        "NON-VACUITY: the scan flags a shim that keeps its own builder_params"
    );
    let bad = helper_violations(SHARED_UNBONDING_SRC);
    assert!(
        bad.is_empty(),
        "crates/producer-tx/src/unbonding_tx.rs violates: {bad:?}"
    );
    let bad = shim_violations(CLI_UNBONDING_SRC);
    assert!(
        bad.is_empty(),
        "bins/cli/src/unbonding_tx.rs violates: {bad:?}"
    );
}
