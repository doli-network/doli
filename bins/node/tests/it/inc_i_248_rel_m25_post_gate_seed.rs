//! INC-I-248 REL-M2.5 — start-time reset of a maintainer root that came from a post-gate producer seed.
//!
// covers: bins/node/src/node/maintainer_sanitize.rs, bins/node/src/run.rs (start-time sanitize)
//!
// OUTPUT CONTRACT: Node::sanitize_retired_maintainer_set(&self) -> bool (async, start-time entry)
//   O1: none
//   O2: maintainer_state.set / last_derived_height; REL-M1 eligibility; maintainer_probe_phase()
//   O3: true iff it reset
//   O4: maintainer_state.bin (EMPTY@tip on reset, bytes unchanged otherwise)
//   O6: update recheck Notify (on the later quorum Adopt only)
// PATHS: R1 non-empty, G > 0, D >= G, last_updated == D, tip > D, block D present without a
//   maintainer change -> EMPTY@tip, eligible, startup probe arms, 3 honest answers -> Adopt;
//   K1 block D holds RemoveMaintainer | AddMaintainer; K2 D < G; K3 adopted set
//   (last_updated < D); K4 doli snap write (D = H, last_updated < H); K5 block D absent
//   (pruned | never stored); K6 G == 0 (devnet); K7 tip == D.
// INPUT PARTITIONS: D = 1 | G-1 | G | S-1 | S | S+1 | TIP-1 | TIP | CUTOFF-1;
//   store = S+1..=TIP | G..=G+1 + S+1..=TIP | 1..=2 + G-1..=G + S+1..=TIP | S..=TIP |
//   0 + CUTOFF..=CUTOFF+2_000 (pruned) | S+1..=TIP+1; block D = coinbase only | + Remove | + Add;
//   set = producer seed (real seed path) | producer keys stamped at D | canonical rotated set.
// MATRIX: R1 x 2 partitions x (O2 set, O2 ldh, O2 eligible, O3, O4) + R1 probe phase, digest,
//   last_change_block, release, O6 + K1-K7 x 13 cases x (O2 set, O2 ldh, O3, O4 bytes, phase).

use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use crate::inc_i_248_harness as h;
use crate::inc_i_248_rel_m1_fixture as fx;
use crate::inc_i_248_rel_m2_fixture as m2;
use doli_core::{Network, Transaction};
use doli_node::node::Node;
use tempfile::TempDir;

const G: u64 = h::GATE;
const D: u64 = m2::S + 1;

const _: () = assert!(G > 0 && D > G && m2::TIP > D);

/// A maintainer change `op` (Add or Remove) bound to the node's genesis at `height`.
fn change_tx(node: &Node, height: u64, op: h::Op) -> Transaction {
    let step = h::Step {
        label: "fixture: maintainer change",
        height,
        op,
        signers: vec![2, 3, 4],
        accept: true,
    };
    let tx = h::step_tx(&h::node_genesis(node), &step);
    assert!(
        tx.is_maintainer_change(),
        "fixture: {op:?} is a maintainer change"
    );
    tx
}

/// Pre-fix snap data dir of the second shape: producer seed at `d` (real seed path, read
/// back from disk), store = `ranges`, block `d` coinbase only, chain tip TIP.
async fn post_gate_seed_node(d: u64, ranges: &[RangeInclusive<u64>]) -> (Node, TempDir) {
    let (mut node, temp) = h::testnet_node(&h::producer_keys()).await;
    m2::assert_retired_check_off(&node);
    node.maybe_bootstrap_maintainer_set(d).await;
    let disk = h::disk_state_of(&node);
    assert!(
        h::is_producer_seed(&disk.set),
        "fixture: producer seed on disk"
    );
    assert_eq!(disk.set.last_updated, d, "fixture: seed last_updated == D");
    assert_eq!(disk.last_derived_height, d, "fixture: seed stamped at D");
    m2::install_state(&mut node, &disk);
    m2::put_heights(&node, ranges);
    m2::set_tip_to_store(&node, m2::TIP).await;
    assert!(
        m2::has_block(&node, d)
            && m2::has_block(&node, d + 1)
            && !m2::block_has_maintainer_change(&node, d),
        "fixture: block D (coinbase only) and block D+1 stored"
    );
    (node, temp)
}

// REQ-248-009 (Must) — Decision: a failure means a pre-fix snap node that seeded from live producers at its first post-snapshot block keeps that stale root after the upgrade and never converges to the fleet set.
#[tokio::test]
async fn inc_i_248_post_gate_seed_root_repaired_on_start() {
    let (mut node, _t) = post_gate_seed_node(D, &[D..=m2::TIP]).await;
    assert!(
        !m2::has_block(&node, 1) && !m2::has_block(&node, m2::S),
        "fixture: store = S+1..=TIP only"
    );
    let notify = fx::attach_notify(&mut node);
    assert!(
        !m2::eligible(&node).await,
        "fixture: the post-gate seed is usable"
    );

    assert!(
        node.sanitize_retired_maintainer_set().await,
        "the start-time check must reset a root seeded from live producers above the gate"
    );
    fx::assert_empty_at(&node, m2::TIP, "after the start-time check").await;
    assert!(
        m2::eligible(&node).await,
        "EMPTY@tip must be quorum-eligible"
    );
    assert!(!h::resolved_root(&node).await.is_usable());
    assert!(
        !node.sanitize_retired_maintainer_set().await,
        "second call is a no-op"
    );
    fx::assert_empty_at(&node, m2::TIP, "after the second call").await;

    fx::add_sync_peer(&node, m2::TIP).await;
    let t0 = Instant::now();
    node.maintainer_quorum_tick_at(t0 + fx::STARTUP).await;
    assert_eq!(
        node.maintainer_probe_phase(),
        "collecting",
        "the startup probe arms"
    );
    let canonical = fx::canonical(h::last_rotation_height());
    let good = fx::frame(&node, &canonical, m2::TIP);
    h::deliver(&mut node, &good, 3).await;
    fx::assert_empty_at(&node, m2::TIP, "before the window end").await;
    node.maintainer_quorum_tick_at(t0 + fx::WINDOW_END).await;

    fx::assert_state(&node, &canonical, m2::TIP, "window end").await;
    assert_eq!(node.maintainer_probe_phase(), "done");
    let st = h::state_of(&node).await;
    assert_eq!(
        h::node_digest(&node, &st.set),
        h::node_digest(&node, &canonical),
        "resolved digest == canonical digest"
    );
    assert_eq!(
        st.set.last_updated, canonical.last_updated,
        "last_change_block == canonical"
    );
    assert_eq!(
        h::verdict_name(&h::verify_rotated_release(&node).await),
        "Ok(3)"
    );
    assert!(
        fx::fired(&notify, Duration::from_secs(1)).await,
        "Adopt must fire the recheck"
    );

    let (gate_node, _tg) = post_gate_seed_node(G, &[G..=G + 1, m2::S + 1..=m2::TIP]).await;
    assert!(
        gate_node.sanitize_retired_maintainer_set().await,
        "a seed at D == G must reset"
    );
    fx::assert_empty_at(&gate_node, m2::TIP, "D == G").await;
    assert!(
        m2::eligible(&gate_node).await,
        "D == G: EMPTY@tip must be quorum-eligible"
    );
}

// REQ-248-009 (Must) — Decision: a failure means the start-time check resets a root that a governance block at D set, so a replay node loses its replayed root to the peer quorum.
#[tokio::test]
async fn inc_i_248_rel_m25_governance_block_d_keeps_root() {
    let producers: Vec<_> = h::PRODUCER_SEEDS.iter().map(|s| h::pk(*s)).collect();
    let after_remove = producers[1..].to_vec();
    let after_add = [after_remove.clone(), vec![h::pk(101)]].concat();
    let cases = [
        (
            "block D holds RemoveMaintainer",
            h::Op::Remove(1),
            after_remove,
        ),
        ("block D holds AddMaintainer", h::Op::Add(101), after_add),
    ];
    for (ctx, op, members) in cases {
        let st = fx::state(fx::well_formed(members, D), D);
        let (node, _t) = m2::node_with(&st).await;
        let change = change_tx(&node, D, op);
        m2::put_heights_with(&node, &[D..=m2::TIP], |height, prev| {
            if height == D {
                m2::stub_block_with(height, prev, change.clone())
            } else {
                m2::stub_block(height, prev)
            }
        });
        m2::set_tip_to_store(&node, m2::TIP).await;
        assert!(
            m2::has_block(&node, D + 1) && m2::block_has_maintainer_change(&node, D),
            "{ctx}: fixture: block D holds the change, block D+1 stored"
        );
        m2::assert_start_keeps(&node, ctx).await;
    }
}

// REQ-248-009 (Must) — Decision: a failure means the check ignores the activation gate and resets a pre-gate seed that replay nodes hold as history.
#[tokio::test]
async fn inc_i_248_rel_m25_pre_gate_seed_keeps_root() {
    let (mut node, _t) = m2::node_with(&fx::state(m2::producer_set_at(1), 1)).await;
    m2::put_heights(&node, &[1..=2, G - 1..=G, D..=m2::TIP]);
    m2::set_tip_to_store(&node, m2::TIP).await;
    for (ctx, d) in [
        ("pre-gate seed at D = 1", 1),
        ("pre-gate seed at D = G - 1", G - 1),
    ] {
        m2::install_state(&mut node, &fx::state(m2::producer_set_at(d), d));
        assert!(
            d < G
                && m2::has_block(&node, d)
                && m2::has_block(&node, d + 1)
                && !m2::block_has_maintainer_change(&node, d),
            "{ctx}: fixture: D < G, block D (coinbase only) and block D+1 stored"
        );
        m2::assert_start_keeps(&node, ctx).await;
    }
}

// REQ-248-009 (Must) — Decision: a failure means the check resets a set the node adopted from the quorum (D = adoption tip, last_updated from history).
#[tokio::test]
async fn inc_i_248_rel_m25_adopted_set_keeps_root() {
    let canonical = fx::canonical(h::last_rotation_height());
    let (mut node, _t) = m2::node_with(&fx::state(canonical.clone(), D)).await;
    m2::put_heights(&node, &[D..=m2::TIP]);
    m2::set_tip_to_store(&node, m2::TIP).await;
    for (ctx, t) in [
        ("adopted at T = S+1", D),
        ("adopted at T = TIP - 1", m2::TIP - 1),
    ] {
        m2::install_state(&mut node, &fx::state(canonical.clone(), t));
        assert!(
            canonical.last_updated < t
                && t >= G
                && m2::has_block(&node, t)
                && m2::has_block(&node, t + 1)
                && !m2::block_has_maintainer_change(&node, t),
            "{ctx}: fixture: last_updated < T, block T (coinbase only) and block T+1 stored"
        );
        m2::assert_start_keeps(&node, ctx).await;
    }
}

// REQ-248-009 (Must) — Decision: a failure means the check resets the seed-agreed set that doli snap wrote at the snapshot height.
#[tokio::test]
async fn inc_i_248_rel_m25_doli_snap_write_keeps_root() {
    let canonical = fx::canonical(h::last_rotation_height());
    let (mut node, _t) = m2::node_with(&fx::state(canonical.clone(), m2::S)).await;
    m2::put_heights(&node, &[m2::S..=m2::TIP]);
    m2::set_tip_to_store(&node, m2::TIP).await;
    let cases: [(&str, u64, bool); 2] = [
        ("doli snap at H = S, block H stored", m2::S, true),
        ("doli snap at H = S - 1, block H absent", m2::S - 1, false),
    ];
    for (ctx, snap_h, stored) in cases {
        m2::install_state(&mut node, &fx::state(canonical.clone(), snap_h));
        assert!(
            canonical.last_updated < snap_h
                && snap_h >= G
                && m2::has_block(&node, snap_h) == stored
                && m2::has_block(&node, snap_h + 1),
            "{ctx}: fixture: last_updated < H, block H+1 stored"
        );
        m2::assert_start_keeps(&node, ctx).await;
    }
}

// REQ-248-009 (Must) — Decision: a failure means a missing block D counts as "no maintainer change", so a pruned replay node loses its root.
#[tokio::test]
async fn inc_i_248_rel_m25_block_d_absent_keeps_root() {
    const CUTOFF: u64 = D + 10;
    const PRUNED_TIP: u64 = CUTOFF + 2_000;
    let (mut node, _t) = m2::node_with(&fx::state(m2::producer_set_at(D), D)).await;
    m2::put_heights(&node, &[0..=0, D..=D + 1, CUTOFF..=PRUNED_TIP]);
    let pruned = node
        .block_store
        .prune_blocks_below(CUTOFF, PRUNED_TIP)
        .expect("fixture: prune_blocks_below");
    assert_eq!(pruned, (2, CUTOFF), "fixture: pruned blocks D and D+1");
    m2::set_tip_to_store(&node, PRUNED_TIP).await;
    assert!(
        m2::has_block(&node, 0)
            && !m2::has_block(&node, D)
            && !m2::has_block(&node, D + 1)
            && m2::has_block(&node, CUTOFF),
        "fixture: store = genesis + CUTOFF..=tip"
    );

    let cases: [(&str, u64); 2] = [
        ("D = S+1, blocks D and D+1 pruned", D),
        ("D = cutoff - 1, block D absent, D+1 stored", CUTOFF - 1),
    ];
    for (ctx, d) in cases {
        m2::install_state(&mut node, &fx::state(m2::producer_set_at(d), d));
        assert!(
            d >= G && !m2::has_block(&node, d),
            "{ctx}: fixture: D >= G, block D absent"
        );
        m2::assert_start_keeps(&node, ctx).await;
    }
}

// REQ-248-009 (Must) — Decision: a failure means the check fires on a network without an activation gate (devnet G = 0), where every seed is post-gate.
#[tokio::test]
async fn inc_i_248_rel_m25_devnet_gate_zero_keeps_root() {
    let (mut node, _t) = post_gate_seed_node(D, &[D..=m2::TIP]).await;
    node.config.network = Network::Devnet;
    assert_eq!(
        node.config
            .network
            .params()
            .maintainer_derivation_activation_height,
        0,
        "fixture: devnet G = 0"
    );
    m2::assert_retired_check_off(&node);
    m2::assert_start_keeps(&node, "devnet G = 0, post-gate seed layout").await;
}

// REQ-248-009 (Must) — Decision: a failure means a node restarted before its next block loses a root stamped at its own tip.
#[tokio::test]
async fn inc_i_248_rel_m25_tip_equals_d_keeps_root() {
    let (node, _t) = post_gate_seed_node(m2::TIP, &[D..=m2::TIP + 1]).await;
    assert_eq!(
        node.chain_state.read().await.best_height,
        m2::TIP,
        "fixture: tip == D"
    );
    m2::assert_start_keeps(&node, "tip == D, block D+1 stored").await;
}

fn short(digest: [u8; 32]) -> String {
    hex::encode(digest)[..16].to_string()
}

// REQ-248-009 (Must) — Decision: outcome probe only; a failure is a build or fixture break, never a verdict.
#[tokio::test]
async fn inc_i_248_rel_m25_probe_post_gate_seed_datadir() {
    if std::env::var("INC_I_248_REL_M25_PROBE").ok().as_deref() != Some("1") {
        return;
    }
    let (mut node, _temp) = post_gate_seed_node(D, &[D..=m2::TIP]).await;
    node.sanitize_retired_maintainer_set().await;

    fx::add_sync_peer(&node, m2::TIP).await;
    let t0 = Instant::now();
    node.maintainer_quorum_tick_at(t0 + fx::STARTUP).await;
    let canonical = fx::canonical(h::last_rotation_height());
    let good = fx::frame(&node, &canonical, m2::TIP);
    h::deliver(&mut node, &good, 3).await;
    node.maintainer_quorum_tick_at(t0 + fx::WINDOW_END).await;

    let st = h::state_of(&node).await;
    let digest = if st.set.members.is_empty() {
        "empty".to_string()
    } else {
        short(h::node_digest(&node, &st.set))
    };
    println!(
        "INC-I-248-REL-M25 post-gate-seed-datadir digest={digest} last_change_block={} canonical={}",
        st.set.last_updated,
        short(h::node_digest(&node, &canonical))
    );
}
