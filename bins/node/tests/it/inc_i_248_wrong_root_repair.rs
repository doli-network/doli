// OUTPUT CONTRACT: Node::maintainer_quorum_tick_at(&self, now) + Node::handle_network_event(SyncResponse::MaintainerStateAt)
//   O1: none
//   O2: maintainer_state.set (members, threshold, last_updated), maintainer_state.last_derived_height
//   O3: unit (none)
//   O4: maintainer_state.bin on disk (bytes, mtime; equals memory after Adopt)
//   O5: update recheck Notify (permit stored on Adopt only)
//   Derived: resolve_trust_root(state).keys()/is_usable(); verify_release_with_trust_root(3 rotated)
// OUTPUT CONTRACT: MaintainerQuorumProbe::{new_at, startup_arming_due}
//   O3: startup_arming_due -> bool
// PATHS (REL-M1): P0 restart holding the retired five above the retirement height ->
//   sanitize_retired_maintainer_set() resets it to EMPTY@tip;
//   P1 uptime < 10 s -> not armed, answers dropped;
//   P2 same node, uptime >= 10 s + 1 peer -> armed Startup, 3 answers -> nothing before the
//   window end, Adopt at it, Notify;
//   P3 replay node, usable root, 3 agree + 1 dissent -> never armed, untouched;
//   P4 replay node, usable root, 3 agree, no dissent -> never armed, untouched;
//   P5 replay node, usable root, 3 answers = local digest -> never armed, untouched;
//   P6 0 sync peers at uptime >= 10 s -> not armed.
// INPUT PARTITIONS: canonical = compiled mainnet keys | synthetic rotated keys; local root =
//   retired five (unusable) | producer seed (usable, replay); answer order = agree-first then
//   dissent; peers 0 | 1; uptime 9 s | 10 s | 11 s; probe phase idle | collecting | retry-wait |
//   done | gave-up; first arm Startup | PostSnapInstall.
// MATRIX: P1-P6 x (O2 set, O2 last_derived_height, O4 bytes/mtime, O5 notify) + P2 derived x2
//   + probe 9 cells.
// covers: maintainer_quorum.rs maintainer_sanitize.rs retired.rs quorum.rs params.rs mod.rs init.rs startup.rs periodic.rs run.rs service.rs service_apply.rs service_checks.rs trust_root_wiring.rs

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crate::inc_i_248_harness as h;
use crypto::PublicKey;
use doli_core::maintainer::{KeepReason, QuorumVerdict};
use doli_core::{MaintainerSet, Network};
use doli_node::node::maintainer_quorum::{
    MaintainerQuorumProbe, ProbeReason, ProbeStep, MAINTAINER_QUORUM_MAX_ATTEMPTS,
    MAINTAINER_QUORUM_RETRY_SECS, MAINTAINER_STARTUP_MIN_UPTIME_SECS,
};
use doli_node::node::Node;
use network::protocols::SyncResponse;
use storage::MaintainerState;
use tempfile::TempDir;
use tokio::sync::{Notify, RwLock};

const TIP: u64 = 480_001;
const CANONICAL_LAST_UPDATED: u64 = 331_457;
const MAINNET_RETIREMENT: u64 = 331_457;
const OTHER_LAST_UPDATED: u64 = 20_310;
const STARTUP: Duration = Duration::from_secs(11);
const WINDOW_END: Duration = Duration::from_secs(30);
const QUIET: Duration = Duration::from_millis(300);

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn keys_from_hex(hex: &[&str]) -> Vec<PublicKey> {
    hex.iter()
        .map(|k| PublicKey::from_hex(k).expect("key must parse"))
        .collect()
}

fn retired_five() -> Vec<PublicKey> {
    keys_from_hex(&updater::RETIRED_COMPROMISED_MAINTAINER_KEYS)
}

fn compiled_mainnet_keys() -> Vec<PublicKey> {
    keys_from_hex(updater::bootstrap_maintainer_keys(Network::Mainnet))
}

fn rotated_pubkeys() -> Vec<PublicKey> {
    h::rotated_keys().iter().map(|k| *k.public_key()).collect()
}

fn hex_sorted(keys: &[PublicKey]) -> Vec<String> {
    let mut v: Vec<String> = keys.iter().map(|k| k.to_hex()).collect();
    v.sort();
    v
}

fn root_keys_sorted(root: &updater::TrustRoot) -> Vec<String> {
    let mut v = root.keys().to_vec();
    v.sort();
    v
}

fn well_formed(members: Vec<PublicKey>, last_updated: u64) -> MaintainerSet {
    let set = MaintainerSet::with_members(members, last_updated);
    storage::validate_persisted_set(Path::new("test:answer"), &set)
        .expect("fixture: answer set must be well formed");
    assert_eq!(set.threshold, 3, "fixture: 5 members must give threshold 3");
    set
}

fn frame(node: &Node, set: &MaintainerSet, tip: u64) -> SyncResponse {
    SyncResponse::MaintainerStateAt {
        block_hash: h::tip_hash(),
        block_height: tip,
        digest: crypto::Hash::from_bytes(h::node_digest(node, set)),
        set_bytes: bincode::serialize(set).expect("set must serialize"),
        last_derived_height: tip,
    }
}

fn file_path(node: &Node) -> PathBuf {
    node.config.data_dir.join("maintainer_state.bin")
}

fn file_bytes(node: &Node) -> Vec<u8> {
    std::fs::read(file_path(node)).expect("maintainer_state.bin must exist")
}

fn file_mtime(node: &Node) -> SystemTime {
    std::fs::metadata(file_path(node))
        .and_then(|m| m.modified())
        .expect("maintainer_state.bin mtime must read")
}

async fn add_sync_peer(node: &Node, tip: u64) {
    node.sync_manager.write().await.add_peer(
        network::PeerId::random(),
        tip,
        h::tip_hash(),
        tip as u32,
    );
}

async fn fired(notify: &Notify, within: Duration) -> bool {
    tokio::time::timeout(within, notify.notified())
        .await
        .is_ok()
}

struct Restarted {
    node: Node,
    _temp: TempDir,
    notify: Arc<Notify>,
    retired: MaintainerSet,
}

/// Node restarted from a file holding the retired five at threshold 3, tip TIP, retirement
/// height below the tip; the start-up sanitizer has run (EMPTY@TIP), never armed.
async fn restarted_with_retired_five() -> Restarted {
    let (mut node, temp) = h::testnet_node(&h::producer_keys()).await;
    let retired = MaintainerSet::with_members(retired_five(), 0);
    assert_eq!(retired.threshold, 3, "fixture: retired five at threshold 3");
    MaintainerState {
        set: retired.clone(),
        last_derived_height: TIP,
        ..MaintainerState::default()
    }
    .save(&node.config.data_dir)
    .expect("fixture: retired-five state must persist");
    let loaded =
        MaintainerState::load(&node.config.data_dir).expect("fixture: retired five must load");
    node.set_maintainer_state(Arc::new(RwLock::new(loaded)));
    {
        let mut cs = node.chain_state.write().await;
        cs.best_height = TIP;
        cs.best_hash = h::tip_hash();
    }
    assert!(
        !h::resolved_root(&node).await.is_usable(),
        "fixture: a root holding the retired five must be unusable"
    );
    node.params.retired_genesis_maintainer_keys_height = MAINNET_RETIREMENT;
    assert!(
        node.sanitize_retired_maintainer_set().await,
        "the start-up sanitizer must reset the retired five"
    );
    let notify = Arc::new(Notify::new());
    node.set_update_recheck(notify.clone());
    let r = Restarted {
        node,
        _temp: temp,
        notify,
        retired,
    };
    assert_still_empty(&r, "after the start-up sanitizer").await;
    r
}

async fn assert_still_empty(r: &Restarted, ctx: &str) {
    let state = h::state_of(&r.node).await;
    assert!(
        state.set.members.is_empty(),
        "{ctx}: in-memory set not EMPTY"
    );
    assert_ne!(state.set, r.retired, "{ctx}: the retired five came back");
    assert_eq!(state.last_derived_height, TIP, "{ctx}: height changed");
    let disk = h::disk_state_of(&r.node);
    assert!(disk.set.members.is_empty(), "{ctx}: disk set not EMPTY");
    assert_eq!(disk.last_derived_height, TIP, "{ctx}: disk height changed");
    assert!(!fired(&r.notify, QUIET).await, "{ctx}: recheck fired");
}

/// Full startup path; returns the node after the canonical set was adopted.
async fn repair_on_start(canonical: &MaintainerSet) -> Restarted {
    let mut r = restarted_with_retired_five().await;
    add_sync_peer(&r.node, TIP).await;
    let answer = frame(&r.node, canonical, TIP);

    r.node.maintainer_quorum_tick_at(Instant::now()).await;
    h::deliver(&mut r.node, &answer, 3).await;
    assert_still_empty(&r, "uptime < 10 s: the startup probe must not be armed").await;

    r.node
        .maintainer_quorum_tick_at(Instant::now() + STARTUP)
        .await;
    h::deliver(&mut r.node, &answer, 3).await;
    assert_still_empty(&r, "before the window end: no early adoption").await;
    r.node
        .maintainer_quorum_tick_at(Instant::now() + WINDOW_END)
        .await;

    let state = h::state_of(&r.node).await;
    assert_eq!(
        hex_sorted(&state.set.members),
        hex_sorted(&canonical.members),
        "the retired five must be replaced by the quorum set at start"
    );
    assert_eq!(state.set.threshold, 3);
    assert_eq!(state.set.last_updated, CANONICAL_LAST_UPDATED);
    assert_eq!(state.last_derived_height, TIP);
    let disk = h::disk_state_of(&r.node);
    assert_eq!(disk.set, state.set, "disk set != memory set after Adopt");
    assert_eq!(disk.last_derived_height, state.last_derived_height);
    assert!(
        fired(&r.notify, secs(1)).await,
        "Adopt must fire the updater recheck"
    );
    r
}

/// REQ-248-003 (Must) — Decision: a failure means a node restarting with the retired five keeps them (the sanitizer or the quorum repair is missing), so it can never install a release signed by the rotated maintainers, or the updater waits for its next interval.
#[tokio::test]
async fn inc_i_248_wrong_root_repaired_on_start() {
    let compiled = well_formed(compiled_mainnet_keys(), CANONICAL_LAST_UPDATED);
    let r = repair_on_start(&compiled).await;
    let root = h::resolved_root(&r.node).await;
    assert_eq!(
        root_keys_sorted(&root),
        hex_sorted(&compiled.members),
        "resolved root must be the compiled mainnet maintainer keys"
    );
    assert_eq!(root.threshold(), 3);
    assert!(root.is_usable(), "the repaired root must be usable");

    let rotated = well_formed(rotated_pubkeys(), CANONICAL_LAST_UPDATED);
    let r = repair_on_start(&rotated).await;
    let root = h::resolved_root(&r.node).await;
    assert_eq!(root_keys_sorted(&root), hex_sorted(&rotated.members));
    assert_eq!(
        h::verdict_name(&h::verify_rotated_release(&r.node).await),
        "Ok(3)",
        "a release signed by 3 rotated maintainers must verify against the repaired root"
    );
}

struct ReplayCase {
    node: Node,
    _temp: TempDir,
    notify: Arc<Notify>,
    before: MaintainerState,
    before_bytes: Vec<u8>,
    before_mtime: SystemTime,
}

/// Replay node with a usable producer-seeded root at SNAP_HEIGHT after its startup tick (never armed).
async fn armed_replay_node() -> ReplayCase {
    let (mut node, temp) = h::replay_node_at_snap_height().await;
    assert!(
        !node.chain_state.read().await.is_snap_synced(),
        "fixture: replay node must not be snap-synced"
    );
    assert!(
        h::resolved_root(&node).await.is_usable(),
        "fixture: replay node root must be usable"
    );
    let notify = Arc::new(Notify::new());
    node.set_update_recheck(notify.clone());
    add_sync_peer(&node, h::SNAP_HEIGHT).await;
    node.maintainer_quorum_tick_at(Instant::now() + STARTUP)
        .await;
    assert_eq!(
        node.maintainer_probe_phase(),
        "idle",
        "a usable replay root never arms the probe"
    );
    let before = h::state_of(&node).await;
    let before_bytes = file_bytes(&node);
    let before_mtime = file_mtime(&node);
    ReplayCase {
        node,
        _temp: temp,
        notify,
        before,
        before_bytes,
        before_mtime,
    }
}

async fn assert_untouched(c: &ReplayCase, ctx: &str) {
    let state = h::state_of(&c.node).await;
    assert_eq!(state.set, c.before.set, "{ctx}: in-memory set changed");
    assert_eq!(
        state.last_derived_height, c.before.last_derived_height,
        "{ctx}: last_derived_height changed"
    );
    assert_eq!(
        file_bytes(&c.node),
        c.before_bytes,
        "{ctx}: file on disk changed"
    );
    assert_eq!(
        file_mtime(&c.node),
        c.before_mtime,
        "{ctx}: file was rewritten"
    );
    assert_eq!(
        c.node.maintainer_probe_phase(),
        "idle",
        "{ctx}: probe armed"
    );
}

fn other_set() -> MaintainerSet {
    well_formed(rotated_pubkeys(), OTHER_LAST_UPDATED)
}

/// REQ-248-004 (Must) — Decision: a failure means a colluding quorum replaces a correct replay node's root, with or without an honest dissent (REL-M1: a non-empty root is never eligible).
#[tokio::test]
async fn inc_i_248_correct_replay_root_keeps_on_dissent() {
    let mut c = armed_replay_node().await;
    let agree = frame(&c.node, &other_set(), h::SNAP_HEIGHT);
    let dissent = frame(&c.node, &c.before.set, h::SNAP_HEIGHT);
    h::deliver(&mut c.node, &agree, 3).await;
    h::deliver(&mut c.node, &dissent, 1).await;
    c.node
        .maintainer_quorum_tick_at(Instant::now() + WINDOW_END)
        .await;
    assert_untouched(&c, "3 agree + 1 dissent at the window end").await;
    assert!(
        !fired(&c.notify, QUIET).await,
        "a non-eligible root must not fire the recheck"
    );

    let mut ctl = armed_replay_node().await;
    let agree = frame(&ctl.node, &other_set(), h::SNAP_HEIGHT);
    h::deliver(&mut ctl.node, &agree, 3).await;
    ctl.node
        .maintainer_quorum_tick_at(Instant::now() + WINDOW_END)
        .await;
    assert_untouched(&ctl, "3 agree, no dissent: still not eligible").await;
}

/// REQ-248-004 (Must) — Decision: a failure means a unanimous peer quorum replaces a replay node's history-derived root, before or at the window end.
#[tokio::test]
async fn inc_i_248_replay_root_never_adopts_unanimous() {
    let mut c = armed_replay_node().await;
    let other = other_set();
    let agree = frame(&c.node, &other, h::SNAP_HEIGHT);
    h::deliver(&mut c.node, &agree, 3).await;
    assert_untouched(&c, "before the window end").await;
    assert!(
        !fired(&c.notify, QUIET).await,
        "no recheck before the window end"
    );

    c.node
        .maintainer_quorum_tick_at(Instant::now() + WINDOW_END)
        .await;
    assert_untouched(&c, "at the window end").await;
    assert_ne!(h::state_of(&c.node).await.set, other);
    assert!(
        !fired(&c.notify, QUIET).await,
        "no recheck at the window end"
    );
}

/// REQ-248-004 (Must) — Decision: a failure means a restart of a correct replay node rewrites its trust root or triggers a recheck although every peer agrees with it.
#[tokio::test]
async fn inc_i_248_correct_replay_root_unanimous_is_already_canonical() {
    let mut c = armed_replay_node().await;
    let same = frame(&c.node, &c.before.set, h::SNAP_HEIGHT);
    h::deliver(&mut c.node, &same, 3).await;
    c.node
        .maintainer_quorum_tick_at(Instant::now() + WINDOW_END)
        .await;
    assert_untouched(&c, "peers agree with the local root").await;
    assert!(
        !fired(&c.notify, QUIET).await,
        "a non-eligible root must not fire the recheck"
    );
}

fn transient() -> QuorumVerdict {
    QuorumVerdict::Keep(KeepReason::NoQuorum {
        largest: 1,
        quorum: 3,
    })
}

/// REQ-248-003 (Must) — Decision: a failure means the startup probe never arms, arms before peers exist, or re-arms after it finished and loops forever.
#[tokio::test]
async fn inc_i_248_startup_probe_arms_once() {
    assert_eq!(MAINTAINER_STARTUP_MIN_UPTIME_SECS, 10);
    let t0 = Instant::now();
    let p = MaintainerQuorumProbe::new_at(t0);
    assert!(!p.startup_arming_due(0, t0 + secs(20)), "0 peers");
    assert!(!p.startup_arming_due(1, t0 + secs(9)), "uptime 9 s");
    assert!(
        p.startup_arming_due(1, t0 + secs(10)),
        "uptime 10 s, 1 peer"
    );
    assert!(p.startup_arming_due(5, t0 + secs(3_600)), "late start");

    let fresh = MaintainerQuorumProbe::new();
    let now = Instant::now();
    assert!(!fresh.startup_arming_due(1, now), "new() uptime 0");
    assert!(
        fresh.startup_arming_due(1, now + secs(10)),
        "new() == new_at(now)"
    );

    let mut p = MaintainerQuorumProbe::new_at(t0);
    p.arm(ProbeReason::Startup, t0 + secs(10));
    assert!(!p.startup_arming_due(1, t0 + secs(11)), "collecting");

    let finals = [
        QuorumVerdict::Adopt {
            set: other_set(),
            agreeing: 3,
        },
        QuorumVerdict::Keep(KeepReason::NotEligible),
    ];
    for verdict in finals {
        let mut p = MaintainerQuorumProbe::new_at(t0);
        p.arm(ProbeReason::Startup, t0 + secs(10));
        p.on_verdict(&verdict, t0 + secs(25));
        assert_eq!(p.phase_name(), "done");
        assert!(
            !p.startup_arming_due(1, t0 + secs(3_600)),
            "{verdict:?}: done"
        );
    }

    let mut p = MaintainerQuorumProbe::new_at(t0);
    let mut now = t0 + secs(10);
    for _ in 0..MAINTAINER_QUORUM_MAX_ATTEMPTS {
        p.arm(ProbeReason::Startup, now);
        now += secs(15);
        p.on_verdict(&transient(), now);
        now += secs(MAINTAINER_QUORUM_RETRY_SECS);
    }
    assert_eq!(p.phase_name(), "gave-up");
    assert!(!p.startup_arming_due(1, now + secs(3_600)), "gave-up");

    let mut p = MaintainerQuorumProbe::new_at(t0);
    p.arm(ProbeReason::Startup, t0 + secs(10));
    p.on_verdict(&transient(), t0 + secs(25));
    assert_eq!(p.phase_name(), "retry-wait");
    assert!(!p.startup_arming_due(1, t0 + secs(26)), "retry-wait");
    let rearm_at = t0 + secs(25 + MAINTAINER_QUORUM_RETRY_SECS);
    assert_eq!(p.on_tick(rearm_at), ProbeStep::Rearm);
    p.arm(ProbeReason::Startup, rearm_at);
    assert!(!p.startup_arming_due(1, rearm_at + secs(1)), "re-armed");

    let mut p = MaintainerQuorumProbe::new_at(t0);
    p.arm(ProbeReason::PostSnapInstall, t0 + secs(1));
    assert!(
        !p.startup_arming_due(1, t0 + secs(20)),
        "post-snap probe collecting"
    );

    let mut r = restarted_with_retired_five().await;
    assert_eq!(r.node.sync_manager.read().await.peer_count(), 0);
    let canonical = well_formed(rotated_pubkeys(), CANONICAL_LAST_UPDATED);
    let answer = frame(&r.node, &canonical, TIP);
    r.node
        .maintainer_quorum_tick_at(Instant::now() + STARTUP)
        .await;
    h::deliver(&mut r.node, &answer, 3).await;
    assert_still_empty(&r, "0 sync peers: the startup probe must not be armed").await;

    add_sync_peer(&r.node, TIP).await;
    r.node
        .maintainer_quorum_tick_at(Instant::now() + STARTUP)
        .await;
    h::deliver(&mut r.node, &answer, 3).await;
    r.node
        .maintainer_quorum_tick_at(Instant::now() + WINDOW_END)
        .await;
    assert_eq!(
        h::state_of(&r.node).await.set,
        canonical,
        "control: with 1 peer the startup probe arms and the quorum set is adopted"
    );
}
