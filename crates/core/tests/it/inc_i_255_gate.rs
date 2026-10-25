//! INC-I-255 — slashing activation gate: per-network values, the effective gate, the context field.
//!
// covers: network_params defaults env_loader ordering types validation
//
// OUTPUT CONTRACT: fn NetworkParams::defaults(Network) / fn NetworkParams::load(Network)
//   O1 return .inc_i_255_slashing_activation_height
// OUTPUT CONTRACT: fn NetworkParams::effective_slashing_activation_height(&self) -> u64
//   O2 return value; must be >= each of the three gates and equal their max
// OUTPUT CONTRACT: fn ValidationContext::new(..) / fn with_inc_i_255_slashing_activation_height(self, u64) -> Self
//   O3 return .inc_i_255_slashing_activation_height; neighbour gate and height untouched
//   no mutable params, no receiver writes, no store writes (pure)
// PATHS: P-DEF (defaults) | P-ENV (load in a re-exec child, fresh OnceLock) | P-FN (hand-set gate fields)
// INPUT PARTITIONS:
//   P-DEF: mainnet -> 806_859; testnet -> 330_849; devnet -> 0
//   P-ENV: nothing set | slash env set | sig env raised (devnet) | root env raised (testnet, devnet) | all three set
//   P-FN:  each operand the strict max | all equal | one operand u64::MAX | all zero
//   ctx:   new() at heights 0, 1_000, u64::MAX - 1 | setter with a sentinel
// MATRIX:
//   O1 x P-DEF       -> inc_i_255_defaults_per_network
//   O2 x P-DEF       -> inc_i_255_effective_on_defaults_is_max_of_three_gates
//   O2 x P-FN        -> inc_i_255_effective_is_max_of_each_operand
//   O1,O2 x P-ENV    -> inc_i_255_env_* (five tests, one child process each)
//   O3               -> inc_i_255_context_default_fail_closed_and_setter_sets_only_its_field

use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationContext;
use doli_core::{Network, NetworkParams};

const SLASH_ENV: &str = "DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT";
const SIG_ENV: &str = "DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT";
const ROOT_ENV: &str = "DOLI_INC_I_181_PENDING_ROOT_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_255_GATE_ENV_CHILD";
const NETWORKS: [Network; 3] = [Network::Mainnet, Network::Testnet, Network::Devnet];
/// Pins of the four defense gates (user decisions); see
/// `network_params::tests_defense_pin`.
const MAINNET_PIN: u64 = 806_859;
const TESTNET_PIN: u64 = 330_849;

fn gates(p: &NetworkParams) -> [u64; 3] {
    [
        p.inc_i_255_slashing_activation_height,
        p.inc_i_236_block_signature_activation_height,
        p.inc_i_181_pending_root_activation_height,
    ]
}

fn assert_effective_is_max(p: &NetworkParams, what: &str) {
    let eff = p.effective_slashing_activation_height();
    let [slash, sig, root] = gates(p);
    assert!(
        eff >= slash,
        "{what}: effective {eff} below slash gate {slash}"
    );
    assert!(
        eff >= sig,
        "{what}: effective {eff} below INC-I-236 gate {sig}"
    );
    assert!(
        eff >= root,
        "{what}: effective {eff} below INC-I-181 gate {root}"
    );
    assert_eq!(
        eff,
        slash.max(sig).max(root),
        "{what}: effective must equal the max"
    );
}

/// Parent: re-exec this test alone with exactly `env` set (the three gate vars cleared
/// first) and return true. Child: return false so the caller runs its assertions.
fn spawned_child(test: &str, env: &[(&str, u64)]) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        return false;
    }
    let mut cmd = std::process::Command::new(std::env::current_exe().expect("current exe"));
    cmd.arg(format!("inc_i_255_gate::{test}"))
        .args(["--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1");
    for key in [SLASH_ENV, SIG_ENV, ROOT_ENV] {
        cmd.env_remove(key);
    }
    for (key, value) in env {
        cmd.env(key, value.to_string());
    }
    let out = cmd.output().expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "{test}: child failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    true
}

fn loaded() -> [&'static NetworkParams; 3] {
    NETWORKS.map(NetworkParams::load)
}

fn assert_loaded_ordering(what: &str) {
    for (network, p) in NETWORKS.iter().zip(loaded()) {
        assert_effective_is_max(p, &format!("{what}, {network:?}"));
    }
}

// REQ-SLASH-001 — Decision: a failure means slashing engages on mainnet or testnet at a height other than the pinned one, or devnet loses its armed default.
#[test]
fn inc_i_255_defaults_per_network() {
    for (network, expected) in [
        (Network::Mainnet, MAINNET_PIN),
        (Network::Testnet, TESTNET_PIN),
        (Network::Devnet, 0),
    ] {
        assert_eq!(
            NetworkParams::defaults(network).inc_i_255_slashing_activation_height,
            expected,
            "{network:?}"
        );
    }
}

// REQ-SLASH-001 — Decision: a failure means slash evidence becomes valid on some network before blocks are signed or before the pending queue is root-bound.
#[test]
fn inc_i_255_effective_on_defaults_is_max_of_three_gates() {
    for network in NETWORKS {
        let p = NetworkParams::defaults(network);
        assert_effective_is_max(&p, &format!("defaults {network:?}"));
    }
    assert_eq!(
        NetworkParams::defaults(Network::Mainnet).effective_slashing_activation_height(),
        MAINNET_PIN
    );
    assert_eq!(
        NetworkParams::defaults(Network::Testnet).effective_slashing_activation_height(),
        TESTNET_PIN,
        "testnet: 236 == 255 == pin and 181 (97_886) is below it"
    );
    assert_eq!(
        NetworkParams::defaults(Network::Devnet).effective_slashing_activation_height(),
        0
    );
}

// REQ-SLASH-001 — Decision: a failure means the effective gate ignores one of the three gates, so an override of that gate inverts the order.
#[test]
fn inc_i_255_effective_is_max_of_each_operand() {
    const M: u64 = u64::MAX;
    for (slash, sig, root, expected) in [
        (10, 5, 3, 10),
        (5, 10, 3, 10),
        (5, 3, 10, 10),
        (7, 7, 7, 7),
        (M, 0, 0, M),
        (0, M, 0, M),
        (0, 0, M, M),
        (0, 0, 0, 0),
    ] {
        let mut p = NetworkParams::defaults(Network::Devnet);
        p.inc_i_255_slashing_activation_height = slash;
        p.inc_i_236_block_signature_activation_height = sig;
        p.inc_i_181_pending_root_activation_height = root;
        assert_eq!(
            p.effective_slashing_activation_height(),
            expected,
            "slash={slash} sig={sig} root={root}"
        );
    }
}

// REQ-SLASH-001 — Decision: a failure means a context site that forgets the builder arms slashing (fail-open), or the builder clobbers a neighbour gate.
#[test]
fn inc_i_255_context_default_fail_closed_and_setter_sets_only_its_field() {
    for height in [0, 1_000, u64::MAX - 1] {
        let ctx = ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, height);
        assert_eq!(
            ctx.inc_i_255_slashing_activation_height,
            u64::MAX,
            "default at height {height}"
        );
        let set = ctx
            .clone()
            .with_inc_i_236_block_signature_activation_height(42)
            .with_inc_i_255_slashing_activation_height(255_255);
        assert_eq!(set.inc_i_255_slashing_activation_height, 255_255);
        assert_eq!(set.inc_i_236_block_signature_activation_height, 42);
        assert_eq!(set.current_height, height);
        assert_eq!(set.oracle_activation_height, ctx.oracle_activation_height);
        assert_eq!(
            set.inc_i_173_activation_height,
            ctx.inc_i_173_activation_height
        );
    }
}

// REQ-SLASH-001 — Decision: a failure means the runtime path diverges from defaults() when no override is set.
#[test]
fn inc_i_255_env_unset_loads_defaults_and_ordering_holds() {
    if spawned_child("inc_i_255_env_unset_loads_defaults_and_ordering_holds", &[]) {
        return;
    }
    let [mainnet, testnet, devnet] = loaded();
    assert_eq!(mainnet.inc_i_255_slashing_activation_height, MAINNET_PIN);
    assert_eq!(testnet.inc_i_255_slashing_activation_height, TESTNET_PIN);
    assert_eq!(devnet.inc_i_255_slashing_activation_height, 0);
    for network in NETWORKS {
        assert_eq!(
            NetworkParams::load(network).effective_slashing_activation_height(),
            NetworkParams::defaults(network).effective_slashing_activation_height(),
            "{network:?}"
        );
    }
    assert_loaded_ordering("env unset");
}

// REQ-SLASH-001 — Decision: a failure means an operator env var arms or moves slashing on a mainnet or testnet build (a local consensus split).
#[test]
fn inc_i_255_env_override_honoured_on_devnet_only() {
    if spawned_child(
        "inc_i_255_env_override_honoured_on_devnet_only",
        &[(SLASH_ENV, 777)],
    ) {
        return;
    }
    assert_eq!(
        std::env::var(SLASH_ENV).as_deref(),
        Ok("777"),
        "PRECONDITION"
    );
    let [mainnet, testnet, devnet] = loaded();
    assert_eq!(
        mainnet.inc_i_255_slashing_activation_height, MAINNET_PIN,
        "mainnet must ignore the override and keep its pin"
    );
    assert_eq!(
        testnet.inc_i_255_slashing_activation_height, TESTNET_PIN,
        "testnet must ignore the override"
    );
    assert_eq!(
        devnet.inc_i_255_slashing_activation_height, 777,
        "devnet must honour the override"
    );
    assert_eq!(devnet.effective_slashing_activation_height(), 777);
    assert_loaded_ordering("slash env 777");
}

// REQ-SLASH-001 — Decision: a failure means a devnet env that raises the signature gate leaves slashing armed over unauthenticated evidence.
#[test]
fn inc_i_255_env_raised_sig_gate_lifts_effective_on_devnet() {
    if spawned_child(
        "inc_i_255_env_raised_sig_gate_lifts_effective_on_devnet",
        &[(SIG_ENV, 500)],
    ) {
        return;
    }
    let devnet = NetworkParams::load(Network::Devnet);
    assert_eq!(
        devnet.inc_i_236_block_signature_activation_height, 500,
        "PRECONDITION: the env inverts the raw gate order"
    );
    assert_eq!(devnet.inc_i_255_slashing_activation_height, 0);
    assert_eq!(devnet.effective_slashing_activation_height(), 500);
    assert_loaded_ordering("sig env 500");
}

// REQ-SLASH-001 — Decision: a failure means an env that raises the pending-root gate lets a slash be queued while the queue is outside the state root.
#[test]
fn inc_i_255_env_raised_root_gate_lifts_effective() {
    if spawned_child(
        "inc_i_255_env_raised_root_gate_lifts_effective",
        &[(ROOT_ENV, 900)],
    ) {
        return;
    }
    let [mainnet, testnet, devnet] = loaded();
    assert_eq!(
        testnet.inc_i_181_pending_root_activation_height, 900,
        "PRECONDITION: testnet honours the root env"
    );
    assert_eq!(
        devnet.inc_i_181_pending_root_activation_height, 900,
        "PRECONDITION: devnet honours the root env"
    );
    assert_eq!(
        mainnet.inc_i_181_pending_root_activation_height,
        NetworkParams::defaults(Network::Mainnet).inc_i_181_pending_root_activation_height
    );
    assert_eq!(devnet.effective_slashing_activation_height(), 900);
    assert_eq!(
        testnet.effective_slashing_activation_height(),
        TESTNET_PIN,
        "a root env of 900 stays below the testnet pin"
    );
    assert_eq!(mainnet.effective_slashing_activation_height(), MAINNET_PIN);
    assert_loaded_ordering("root env 900");
}

// REQ-SLASH-001 — Decision: a failure means the effective gate on loaded params is not the max when all three overrides are set together.
#[test]
fn inc_i_255_env_all_three_set_effective_is_max() {
    if spawned_child(
        "inc_i_255_env_all_three_set_effective_is_max",
        &[(SLASH_ENV, 300), (SIG_ENV, 700), (ROOT_ENV, 100)],
    ) {
        return;
    }
    let [mainnet, testnet, devnet] = loaded();
    assert_eq!(
        gates(devnet),
        [300, 700, 100],
        "PRECONDITION: devnet reads all three"
    );
    assert_eq!(devnet.effective_slashing_activation_height(), 700);
    assert_eq!(mainnet.inc_i_255_slashing_activation_height, MAINNET_PIN);
    assert_eq!(testnet.inc_i_255_slashing_activation_height, TESTNET_PIN);
    assert_eq!(testnet.effective_slashing_activation_height(), TESTNET_PIN);
    assert_loaded_ordering("all three env set");
}
