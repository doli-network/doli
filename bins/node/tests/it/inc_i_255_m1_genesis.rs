//! INC-I-255 M1 — every production `ValidationContext` site sees the same
//! chain genesis and the same slash gate, so apply cannot refuse evidence the
//! other paths accepted.
//!
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return; O2 chain tip
// OUTPUT CONTRACT: source census — each production `ValidationContext::new` site is one cell
//   O1 params source carries the chainspec genesis; O2 site sets the slash gate
// PATHS: devnet with a chainspec file (genesis timestamp != 0) | the six context sites
//   | embedded mainnet/testnet chainspec applied to for_network params
// MATRIX
//   apply O1,O2 x chainspec devnet x evidence carrying that chain's genesis -> Ok, tip = block
//   census O1 x {pool x2, validation_checks x2, tx_processing, assembly} -> chainspec-applied params
//   census O2 x the same six sites -> slash gate threaded from effective_slashing_activation_height
//   diff O1 x mainnet embedded chainspec -> the six apply_chainspec fields unchanged
//   diff O1 x testnet embedded chainspec -> changed set == {slots_per_reward_epoch, genesis_time}
// INPUT PARTITIONS:
//   genesis: built-in devnet | chainspec with timestamp != 0 | embedded mainnet/testnet chainspec
//   site: planted violation (scanner must flag) | planted fix (scanner must pass) | production sources

use crypto::Hash;
use doli_core::block::{compute_merkle_root, Block, BlockHeader};
use doli_core::chainspec::ChainSpec;
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::Transaction;
use doli_core::validation::ValidationMode;
use doli_core::Network;
use vdf::{VdfOutput, VdfProof};

use super::inc_i_236_m2_ctx_parity::sites;
use super::inc_i_255_m1_common::*;
use crate::block_sig::sign_header;
use crate::inc_i_156_m1_harness::coinbase_for;

const SLASH_GATE_SETTER: &str = "with_inc_i_255_slashing_activation_height";
const SLASH_GATE_SOURCE: &str = "effective_slashing_activation_height";

fn production_sites() -> [(&'static str, &'static str, usize); 4] {
    [
        (
            "mempool/src/pool.rs",
            include_str!("../../../../crates/mempool/src/pool.rs"),
            2,
        ),
        (
            "validation_checks/mod.rs",
            include_str!("../../src/node/validation_checks/mod.rs"),
            2,
        ),
        (
            "apply_block/tx_processing.rs",
            include_str!("../../src/node/apply_block/tx_processing.rs"),
            1,
        ),
        (
            "production/assembly.rs",
            include_str!("../../src/node/production/assembly.rs"),
            1,
        ),
    ]
}

fn chainspec_consistent(site: &str) -> bool {
    !site.contains("ConsensusParams::for_network(") || site.contains("apply_chainspec")
}

const PLANTED: &str = "fn a() {\n    let c = ValidationContext::new(ConsensusParams::for_network(n), n, 0, h)\n        .with_x(1);\n    }\n";

async fn chainspec_block(w: &World, genesis: Hash, extra: Vec<Transaction>) -> Block {
    let params = &w.node.params;
    let prev_hash = w.node.chain_state.read().await.best_hash;
    let slot = 5u32;
    let mut txs = vec![coinbase_for(1, slot, params)];
    txs.extend(extra);
    let mut header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root: compute_merkle_root(&txs),
        presence_root: Hash::ZERO,
        genesis_hash: genesis,
        timestamp: params.genesis_time + slot as u64 * params.slot_duration,
        slot,
        producer: *w.reporter.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    sign_header(&mut header, &w.reporter);
    Block::new(header, txs)
}

// REQ-SLASH-001 — Decision: a failure means apply checks evidence against the built-in devnet genesis on a chainspec devnet, so the producer's own slash block fails apply (N1).
#[tokio::test]
async fn inc_i_255_n1_custom_chainspec_slash_accepted_at_apply() {
    let mut w = world().await;
    let mut spec = ChainSpec::devnet();
    spec.genesis.timestamp = 1;
    let genesis = spec.genesis_hash();
    assert_ne!(
        genesis,
        chain_genesis(),
        "PRECONDITION: the chainspec moves the genesis"
    );
    w.node.config.chainspec = Some(spec.clone());
    w.node.params.apply_chainspec(&spec);

    let (a, b) = pair(&w.accused, genesis);
    let slash = canonical(w.accused_pk(), a, b);
    let block = chainspec_block(&w, genesis, vec![slash]).await;
    let pre = w
        .node
        .validate_block_for_apply(&block, 1, ValidationMode::Light)
        .await;
    assert!(
        pre.is_ok(),
        "PRECONDITION: block-level validation (chainspec genesis) accepts the block; got {pre:?}"
    );

    let r = w
        .node
        .apply_block(block.clone(), ValidationMode::Light)
        .await;
    assert!(
        r.is_ok(),
        "N1: apply must judge the evidence against this chain's genesis; got {:?}",
        r.as_ref().err().map(|e| format!("{e:#}"))
    );
    assert_eq!(
        tip(&w.node).await,
        (1, block.hash()),
        "tip must be the slash block"
    );
}

// REQ-SLASH-001 — Decision: a failure names the context site whose genesis is not the chainspec's, the apply/builder/mempool split behind N1.
#[test]
fn inc_i_255_n1_every_ctx_site_takes_chainspec_genesis() {
    let planted = sites(PLANTED);
    assert_eq!(
        planted.len(),
        1,
        "NON-VACUITY: scanner must see the planted site"
    );
    assert!(
        !chainspec_consistent(&planted[0]),
        "NON-VACUITY: scanner must flag a bare for_network site"
    );
    let fixed = PLANTED.replace(
        ".with_x(1);",
        ".with_x(1);\n        c.params.apply_chainspec(s);",
    );
    assert!(
        chainspec_consistent(&sites(&fixed)[0]),
        "NON-VACUITY: scanner must accept a chainspec-applied site"
    );

    for (file, src, expected) in production_sites() {
        let found = sites(src);
        assert_eq!(
            found.len(),
            expected,
            "NON-VACUITY: {file} site count moved; re-audit"
        );
        for (i, site) in found.iter().enumerate() {
            assert!(
                chainspec_consistent(site),
                "N1: {file} site #{i} builds its params with for_network and never applies the chainspec"
            );
        }
    }
}

// REQ-SLASH-001 — Decision: a failure names a context site left at the u64::MAX default, where the slash rules would never arm and paths would disagree (B13).
#[test]
fn inc_i_255_b13_every_ctx_site_sets_slash_gate() {
    let planted = sites(PLANTED);
    assert!(
        !planted[0].contains(SLASH_GATE_SETTER),
        "NON-VACUITY: scanner must report the planted site as unthreaded"
    );
    let fixed = PLANTED.replace(
        ".with_x(1)",
        ".with_inc_i_255_slashing_activation_height(g)",
    );
    assert!(
        sites(&fixed)[0].contains(SLASH_GATE_SETTER),
        "NON-VACUITY: scanner must accept a threaded site"
    );

    for (file, src, expected) in production_sites() {
        let found = sites(src);
        assert_eq!(
            found.len(),
            expected,
            "NON-VACUITY: {file} site count moved; re-audit"
        );
        for (i, site) in found.iter().enumerate() {
            assert!(
                site.contains(SLASH_GATE_SETTER),
                "B13: {file} site #{i} does not set the slash gate"
            );
        }
        assert!(
            src.contains(SLASH_GATE_SOURCE),
            "B13: {file} must take the gate from {SLASH_GATE_SOURCE}() on loaded params"
        );
    }
}

const EMBEDDED_MAINNET: &str = include_str!("../../../../chainspec.mainnet.json");
const EMBEDDED_TESTNET: &str = include_str!("../../../../chainspec.testnet.json");

fn embedded(network: Network, json: &str) -> ChainSpec {
    let spec: ChainSpec = serde_json::from_str(json).expect("embedded chainspec parses");
    assert_eq!(
        spec.network, network,
        "PRECONDITION: embedded chainspec network"
    );
    spec
}

fn applied_diffs(network: Network, spec: &ChainSpec) -> Vec<(&'static str, String)> {
    let base = ConsensusParams::for_network(network);
    let mut applied = base.clone();
    applied.apply_chainspec(spec);
    let mut diffs = Vec::new();
    macro_rules! cmp {
        ($($f:ident),*) => {$(
            if base.$f != applied.$f {
                diffs.push((
                    stringify!($f),
                    format!("{network:?}.{}: built-in={:?} applied={:?}", stringify!($f), base.$f, applied.$f),
                ));
            }
        )*};
    }
    cmp!(
        slot_duration,
        slots_per_reward_epoch,
        initial_bond,
        initial_reward,
        genesis_time,
        genesis_hash
    );
    diffs
}

fn changed_fields(diffs: &[(&'static str, String)]) -> std::collections::BTreeSet<&'static str> {
    diffs.iter().map(|(f, _)| *f).collect()
}

// REQ-SLASH-001 — Decision: a failure names a field the apply-context switch to chainspec-applied params moves beyond the pinned header-only set, so apply verdicts could change.
#[test]
fn inc_i_255_n1_apply_params_diff_pinned_on_mainnet_and_testnet() {
    let mut planted = embedded(Network::Testnet, EMBEDDED_TESTNET);
    planted.consensus.bond_amount += 1;
    assert!(
        changed_fields(&applied_diffs(Network::Testnet, &planted)).contains("initial_bond"),
        "NON-VACUITY: comparator must flag a moved field"
    );

    let mainnet = applied_diffs(
        Network::Mainnet,
        &embedded(Network::Mainnet, EMBEDDED_MAINNET),
    );
    assert!(
        mainnet.is_empty(),
        "N1 mainnet: fields changed: {mainnet:#?}"
    );

    let testnet = applied_diffs(
        Network::Testnet,
        &embedded(Network::Testnet, EMBEDDED_TESTNET),
    );
    let changed = changed_fields(&testnet);
    for f in [
        "genesis_hash",
        "initial_bond",
        "initial_reward",
        "slot_duration",
    ] {
        assert!(
            !changed.contains(f),
            "N1 testnet: {f} changed: {testnet:#?}"
        );
    }
    let expected: std::collections::BTreeSet<&str> = ["slots_per_reward_epoch", "genesis_time"]
        .into_iter()
        .collect();
    assert_eq!(changed, expected, "N1 testnet: changed set: {testnet:#?}");
}
