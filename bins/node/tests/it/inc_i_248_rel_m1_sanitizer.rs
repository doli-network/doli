//! INC-I-248 REL-M1 — the retired-set sanitizer and the retired-key vote filter.
//!
// covers: crates/core/src/maintainer/quorum.rs, crates/core/src/maintainer/retired.rs, crates/core/src/maintainer/mod.rs, crates/core/src/consensus/params.rs, crates/core/src/consensus/stress.rs, crates/core/src/network_params/mod.rs, crates/core/src/network_params/defaults.rs, bins/node/src/node/maintainer_quorum.rs, bins/node/src/node/maintainer_sanitize.rs, bins/node/src/node/mod.rs, bins/node/src/node/init.rs, bins/node/src/node/startup.rs, bins/node/src/node/periodic.rs, bins/node/src/node/fork_recovery.rs, bins/node/src/updater/mod.rs, bins/node/src/run.rs
//!
//! REQ-248-007 (Must) — Decision: a failure means a node keeps or adopts the 5 leaked genesis keys past their retirement, governance signed by members of such a set is still applied, or a replay node's history set is reset.
//!
// OUTPUT CONTRACT: Node::sanitize_retired_maintainer_set(&self) -> bool (async)
//   O2: maintainer_state.set / last_derived_height; O3: true iff it reset; O4: maintainer_state.bin
// OUTPUT CONTRACT: Node::maybe_bootstrap_maintainer_set(height) (sanitizer right after the seed)
// OUTPUT CONTRACT: vote filter in Node::maintainer_quorum_tick_at / apply_maintainer_quorum_answers
//   O2: set / ldh / maintainer_probe_phase(); O4: file; O6: recheck Notify on Adopt
// PATHS: Z1 retired five above retirement -> EMPTY@tip, then quorum Adopt(canonical);
//   Z2 sanitized EMPTY root rejects AddMaintainer by old members (control: unsanitized accepts);
//   Z3 producer seed of the retired five above retirement -> EMPTY@tip, probe arms;
//   Z4 retired answers dropped on mainnet params, adopted on testnet params;
//   Z5 not sanitized: tip == retirement, tip below, canonical set, EMPTY, testnet params.
// INPUT PARTITIONS: retirement = 331_457 | u64::MAX; tip = 200_000 | 331_457 | 331_458 | 480_001;
//   set = retired five | 1 retired + 4 | canonical rotated | EMPTY;
//   answers = 3 retired | 3 retired + 3 canonical.
// MATRIX: Z1-Z5 x (O2 set, O2 ldh, O3, O4) + Z1/Z4 phase+notify = 34 cells.

use std::time::{Duration, Instant};

use crate::inc_i_248_harness as h;
use crate::inc_i_248_rel_m1_fixture as fx;
use doli_core::MaintainerSet;
use doli_node::node::maintainer_quorum::ProbeReason;

fn mainnet_like(node: &mut doli_node::node::Node) {
    node.params.retired_genesis_maintainer_keys_height = fx::MAINNET_RETIREMENT;
}

// REQ-248-007 — Decision: a failure means a node that restarts holding the leaked five past 331_457 keeps them instead of failing closed and repairing from the quorum (SEC-AUTH-001).
#[tokio::test]
async fn inc_i_248_rel_m1_sanitizer_resets_retired_five_then_quorum_repairs() {
    let tip = fx::MAINNET_TIP;
    let (mut node, _t) = fx::node_with_state(fx::state(fx::retired_set(0), tip), tip).await;
    mainnet_like(&mut node);
    let notify = fx::attach_notify(&mut node);

    assert!(node.sanitize_retired_maintainer_set().await, "must reset");
    fx::assert_empty_at(&node, tip, "after the sanitizer").await;
    assert!(!h::resolved_root(&node).await.is_usable());
    assert!(
        !node.sanitize_retired_maintainer_set().await,
        "second call is a no-op"
    );
    fx::assert_empty_at(&node, tip, "after the second call").await;

    fx::add_sync_peer(&node, tip).await;
    let t0 = Instant::now();
    node.maintainer_quorum_tick_at(t0 + fx::STARTUP).await;
    assert_eq!(
        node.maintainer_probe_phase(),
        "collecting",
        "the startup probe arms"
    );
    let canonical = fx::canonical(fx::MAINNET_RETIREMENT);
    let good = fx::frame(&node, &canonical, tip);
    h::deliver(&mut node, &good, 3).await;
    fx::assert_empty_at(&node, tip, "before the window end").await;
    node.maintainer_quorum_tick_at(t0 + fx::WINDOW_END).await;

    fx::assert_state(&node, &canonical, tip, "window end").await;
    let root = h::resolved_root(&node).await;
    let mut keys = root.keys().to_vec();
    keys.sort();
    assert_eq!(
        keys,
        fx::hex_sorted(&fx::rotated_pubkeys()),
        "resolved root == canonical"
    );
    assert_eq!(
        h::verdict_name(&h::verify_rotated_release(&node).await),
        "Ok(3)"
    );
    assert!(
        fx::fired(&notify, Duration::from_secs(1)).await,
        "Adopt must fire the recheck"
    );
}

// REQ-248-007 — Decision: a failure means governance signed by members of a set that holds a leaked key can still rotate the sanitized root (laundering, SEC-AUTH-002).
#[tokio::test]
async fn inc_i_248_rel_m1_sanitized_root_rejects_governance_by_old_members() {
    let tip = fx::MAINNET_TIP;
    let mixed = fx::well_formed(
        vec![fx::retired_five()[0], h::pk(101), h::pk(102), h::pk(103)],
        20_000,
    );
    assert_eq!(mixed.threshold, 3, "fixture: 4 members at threshold 3");
    let step = h::Step {
        label: "add X1 by old members",
        height: tip + 1,
        op: h::Op::Add(201),
        signers: vec![101, 102, 103],
        accept: false,
    };

    let (control, _tc) = fx::node_with_state(fx::state(mixed.clone(), tip), tip).await;
    assert!(
        !control.sanitize_retired_maintainer_set().await,
        "testnet params: no reset"
    );
    h::apply_step(&control, &step).await;
    assert!(
        h::state_of(&control)
            .await
            .set
            .members
            .contains(&h::pk(201)),
        "control: the unsanitized set accepts the change"
    );

    let (mut node, _t) = fx::node_with_state(fx::state(mixed, tip), tip).await;
    mainnet_like(&mut node);
    assert!(
        node.sanitize_retired_maintainer_set().await,
        "one retired key resets the set"
    );
    h::apply_step(&node, &step).await;
    fx::assert_empty_at(&node, tip, "the sanitized root accepts no governance").await;
}

// REQ-248-007 — Decision: a failure means a producer seed above the retirement height installs the leaked keys as the trust root.
#[tokio::test]
async fn inc_i_248_rel_m1_sanitizer_after_producer_seed() {
    let height = fx::MAINNET_RETIREMENT + 1;
    let (mut node, _t) = h::testnet_node(&[]).await;
    mainnet_like(&mut node);
    {
        let mut ps = node.producer_set.write().await;
        for k in fx::retired_five() {
            ps.register_genesis_producer(k, 1, doli_core::Network::Testnet.bond_unit())
                .expect("fixture: register retired producer");
        }
    }
    fx::set_tip(&node, height).await;

    node.maybe_bootstrap_maintainer_set(height).await;
    fx::assert_empty_at(&node, height, "after the producer seed").await;

    fx::add_sync_peer(&node, height).await;
    node.maintainer_quorum_tick_at(Instant::now() + fx::STARTUP)
        .await;
    assert_eq!(
        node.maintainer_probe_phase(),
        "collecting",
        "the sanitized root is eligible"
    );
}

// REQ-248-007 — Decision: a failure means the vote counts answers that hold a key retired at S on mainnet, or drops them on testnet where the keys are live (SEC-AUTH-004).
#[tokio::test]
async fn inc_i_248_rel_m1_retired_answers_dropped_by_network() {
    let tip = fx::MAINNET_TIP;
    let retired = fx::retired_set(0);
    let canonical = fx::canonical(fx::MAINNET_RETIREMENT);

    let (mut only, _t1) = fx::node_with_state(fx::empty_state(tip), tip).await;
    mainnet_like(&mut only);
    only.arm_maintainer_quorum(ProbeReason::PostSnapInstall)
        .await;
    let old = fx::frame(&only, &retired, tip);
    h::deliver(&mut only, &old, 3).await;
    only.maintainer_quorum_tick_at(Instant::now() + fx::WINDOW_END)
        .await;
    fx::assert_empty_at(&only, tip, "mainnet: 3 retired answers").await;
    assert_eq!(only.maintainer_probe_phase(), "retry-wait");

    let (mut mixed, _t2) = fx::node_with_state(fx::empty_state(tip), tip).await;
    mainnet_like(&mut mixed);
    mixed
        .arm_maintainer_quorum(ProbeReason::PostSnapInstall)
        .await;
    let (old, good) = (
        fx::frame(&mixed, &retired, tip),
        fx::frame(&mixed, &canonical, tip),
    );
    h::deliver(&mut mixed, &old, 3).await;
    h::deliver(&mut mixed, &good, 3).await;
    mixed
        .maintainer_quorum_tick_at(Instant::now() + fx::WINDOW_END)
        .await;
    fx::assert_state(
        &mixed,
        &canonical,
        tip,
        "mainnet: retired dropped, not a tie",
    )
    .await;

    let (mut testnet, _t3) = fx::node_with_state(fx::empty_state(fx::S), fx::S).await;
    testnet
        .arm_maintainer_quorum(ProbeReason::PostSnapInstall)
        .await;
    let old = fx::frame(&testnet, &retired, fx::S);
    h::deliver(&mut testnet, &old, 3).await;
    testnet
        .maintainer_quorum_tick_at(Instant::now() + fx::WINDOW_END)
        .await;
    fx::assert_state(&testnet, &retired, fx::S, "testnet: genesis five adopted").await;
}

struct Case {
    name: &'static str,
    set: MaintainerSet,
    tip: u64,
    retirement: u64,
    resets: bool,
}

// REQ-248-004 (Must) — Decision: a failure means the sanitizer resets a replay node's history set (below the retirement height, the canonical set, testnet) or misses a retired key above it.
#[tokio::test]
async fn inc_i_248_rel_m1_sanitizer_boundary_and_replay_non_interference() {
    let r = fx::MAINNET_RETIREMENT;
    let mut one_retired = fx::rotated_pubkeys();
    one_retired[4] = fx::retired_five()[2];
    let cases = [
        Case {
            name: "retired five below",
            set: fx::retired_set(0),
            tip: 200_000,
            retirement: r,
            resets: false,
        },
        Case {
            name: "retired five at retirement",
            set: fx::retired_set(0),
            tip: r,
            retirement: r,
            resets: false,
        },
        Case {
            name: "retired five above",
            set: fx::retired_set(0),
            tip: r + 1,
            retirement: r,
            resets: true,
        },
        Case {
            name: "one retired key above",
            set: fx::well_formed(one_retired, r),
            tip: fx::MAINNET_TIP,
            retirement: r,
            resets: true,
        },
        Case {
            name: "canonical above",
            set: fx::canonical(r),
            tip: fx::MAINNET_TIP,
            retirement: r,
            resets: false,
        },
        Case {
            name: "EMPTY above",
            set: MaintainerSet::new(),
            tip: fx::MAINNET_TIP,
            retirement: r,
            resets: false,
        },
        Case {
            name: "testnet retired five",
            set: fx::retired_set(0),
            tip: fx::MAINNET_TIP,
            retirement: u64::MAX,
            resets: false,
        },
    ];
    for c in cases {
        let (mut node, _t) = fx::node_with_state(fx::state(c.set.clone(), c.tip), c.tip).await;
        node.params.retired_genesis_maintainer_keys_height = c.retirement;
        let before = fx::file_bytes(&node);
        assert_eq!(
            node.sanitize_retired_maintainer_set().await,
            c.resets,
            "{}: return",
            c.name
        );
        if c.resets {
            fx::assert_empty_at(&node, c.tip, c.name).await;
        } else {
            fx::assert_state(&node, &c.set, c.tip, c.name).await;
            assert_eq!(fx::file_bytes(&node), before, "{}: file rewritten", c.name);
        }
    }
}
