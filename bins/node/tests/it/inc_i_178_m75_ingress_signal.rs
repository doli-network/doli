//! INC-I-178 M7.5 — REQ-BLS-006 AC-2 / precondition P3: make "this ACTIVE
//! producer emitted a verifying 96-byte BLS half" observable from OUTSIDE the
//! node process, so GS-018 can stop SKIPping on a false green.
//!
//! OUTPUT CONTRACT
//!
//! F1: `Node::ingest_attestation`, `BlsAttestVerdict::Valid` arm, reached through
//!   BOTH real ingresses (`on_new_attestation`, `record_direct_attestation`).
//!   Observable outputs added by this milestone:
//!     O1 `doli_attestation_bls_valid_total` (IntCounter) — fleet-wide count of
//!        FIRST-SEEN verifying halves, read out of the RENDERED exposition
//!     O2 `doli_attestation_bls_valid_attester_total{attester="<8 hex>"}`
//!        (IntCounterVec) — the per-producer series AC-2 needs, label = the first
//!        8 hex characters of the attester's Ed25519 pubkey
//!     O3 one `debug!` line carrying the frozen grep literal (M7.6 level)
//!     O4 `self.parent_sig_pool` — unchanged M2 contract, asserted only as the
//!        anti-vacuity witness that the Valid arm actually ran
//!     O5 return value / mutable params / persistent store — unchanged by M7.5
//!   PATHS (the four BLS verdicts x the membership gate, as in M2):
//!     P1 member, on-chain key, VALID, FIRST SEEN  -> O1 +1, O2 +1, O3 one line
//!     P1r the SAME valid half re-delivered        -> O1, O2, O3 all unmoved
//!     P2 member, EMPTY bls_signature              -> O1, O2, O3 unmoved
//!     P3 member, 96 bytes, NO on-chain key        -> O1, O2, O3 unmoved
//!     P4 member, on-chain key, INVALID 96 bytes   -> O1, O2, O3 unmoved
//!     P5 NON-member                               -> never reaches the verdict
//!   INPUT PARTITIONS:
//!     I1 gossip ingress, freshly generated attester key
//!     I2 direct ingress, freshly generated attester key
//!     I3 second delivery of byte-identical valid bytes, different source peer
//!     I4 96 bytes of garbage from a registered member
//!     I5 a key that is in no ProducerSet
//!   MATRIX 5 outputs x 6 paths: O4/O5 are M2's contract, re-asserted only as the
//!     anti-vacuity witness. O2 is asserted on every one of P1-P5 and on P1r.
//!     O3 is asserted as SOURCE TEXT (F2) — this repo has no tracing-capture
//!     harness and adding `tracing-subscriber` as a dev-dependency to observe one
//!     log line is not in scope.
//!
//! F2: `bins/node/src/node/attestation/ingress.rs`, `bins/node/src/metrics.rs`
//!   and `scripts/gauntlet-gs018.sh` as SOURCE TEXT. The probe and the instrument
//!   must name the same two series or GS-018 SKIPs forever on a build that
//!   already emits the signal. INPUT PARTITIONS: N/A — one text per file.
//!
//! WHY THE EXACTNESS LIVES ON O2, NOT O1. Every test in this binary shares one
//! process and one global registry, and `inc_i_178_m2_ingress`, `m4_commit` and
//! `inc_i_204_m41_common` all drive valid ingests concurrently without taking
//! `counter_lock`. A delta of exactly 1 on the FLEET-WIDE series is therefore a
//! race. The per-attester series is keyed by a freshly generated pubkey that no
//! other test can produce, so every exactness assertion lands there; O1 is held
//! by a movement assertion plus the F2 write-site tripwire.

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::attestation::attestation_minute;
use doli_core::Attestation;
use doli_node::node::Node;
use network::PeerId;
use std::fs;
use std::path::{Path, PathBuf};

use crate::inc_i_178_m0_common::{
    assemble, build_via_production, dual, make_node, register_bls, safe_build_height, N_SMALL,
};
use crate::inc_i_178_m5_common::counter_lock;
use crate::inc_i_204_m0_common::{encode_registry, exported_label_values, exported_value};

/// Fleet-wide first-seen verifying halves. Its ABSENCE is GS-018's capability
/// marker for "this node predates M7.5".
const FAMILY_TOTAL: &str = "doli_attestation_bls_valid_total";
/// Per-attester series — the only thing that answers "which producer dual-signs".
const FAMILY_BY_ATTESTER: &str = "doli_attestation_bls_valid_attester_total";
const LABEL: &str = "attester";

/// The literal an operator and GS-018 grep for. Frozen: a reworded line is a
/// silent probe break. Plain `{}` because `{:.8}` truncates nothing here (see
/// the sliced-label test) — both fields are sliced to 8 hex at the call site, so
/// the log tag and the metric label are the same string.
const VALID_LOG_LITERAL: &str = "[ATTEST_INGEST] valid bls attester={} parent={} sig_len={}";

const INGRESS_RS: &str = "bins/node/src/node/attestation/ingress.rs";
const METRICS_RS: &str = "bins/node/src/metrics.rs";
const GS018_SH: &str = "scripts/gauntlet-gs018.sh";

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve")
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// Rust source with every whole-line `//` comment removed.
fn code_only(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Shell source with every whole-line `#` comment removed, so the GS-018 header
/// block cannot satisfy a "the probe reads this series" assertion by prose.
fn sh_code_only(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build one real block, make it canonical, hand back its hash, slot, height.
async fn canonical_block(node: &mut Node) -> (Hash, u32, u64) {
    let h = safe_build_height(node);
    let (header, txs, bf) = build_via_production(node, h).await;
    let block = assemble(header, txs, bf);
    let hash = block.hash();
    let slot = block.header.slot;
    node.block_store
        .put_block_canonical(&block, h)
        .expect("put_block_canonical failed");
    (hash, slot, h)
}

fn with_bls(kp: &KeyPair, hash: Hash, slot: u32, height: u64, blob: Vec<u8>) -> Attestation {
    let mut a = Attestation::new(hash, slot, height, 1, kp.private_key(), *kp.public_key());
    a.bls_signature = blob;
    a
}

fn attended(node: &Node, slot: u32, pk: &PublicKey) -> bool {
    node.minute_tracker
        .attested_in_minute(attestation_minute(slot))
        .contains(&pk)
}

/// The direct ingress derives authority at the CURRENT tip.
async fn tip_height(node: &Node) -> u64 {
    node.best_height().await
}

/// The label value the contract pins: the first 8 hex characters of the pubkey.
fn label_of(pk: &PublicKey) -> String {
    pk.to_hex()[..8].to_string()
}

/// The per-attester series scalar, read out of the RENDERED exposition text.
/// `None` = the series does not exist, which is what a never-seen attester and a
/// never-written family look like alike — every caller pins which it expects.
fn attester_series(pk: &PublicKey) -> Option<f64> {
    exported_value(FAMILY_BY_ATTESTER, &[(LABEL, &label_of(pk))])
}

/// The fleet-wide scalar. `None` = the family is not registered at all.
fn fleet_total() -> Option<f64> {
    exported_value(FAMILY_TOTAL, &[])
}

/// Every test starts from "this attester has no series", so a later `Some` is
/// unambiguously this test's write and not another test's leftover.
fn assert_unseen(pk: &PublicKey, ctx: &str) {
    assert_eq!(
        attester_series(pk),
        None,
        "{ctx}: precondition — a freshly generated attester must have no series yet"
    );
}

// ===========================================================================
// P1 — the signal REQ-BLS-006 AC-2 needs
// ===========================================================================

/// REQ-BLS-006 AC-2 — Decision: a failure means the fleet-wide "is every ACTIVE
/// producer dual-signing" question stays unanswerable from outside the process,
/// which is exactly the state GS-018 refuses to green today; the AH would then be
/// pinned on "0 unverifiable-BLS warnings", which is also what a fleet that emits
/// no BLS halves at all looks like.
#[tokio::test]
async fn req_bls_006_a_valid_first_seen_gossip_ingest_publishes_a_per_attester_series() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[3];
    register_bls(&node, a.public_key(), &bls).await;
    let att = dual(a, &bls, hash, slot, height);

    let _guard = counter_lock().await;
    assert_unseen(a.public_key(), "P1 gossip");
    let before_total = fleet_total().expect(
        "O1: doli_attestation_bls_valid_total must render a series from process start; \
         its absence is GS-018's 'old build' marker and cannot double as 'no data yet'",
    );

    node.on_new_attestation(att.to_bytes(), PeerId::random())
        .await;

    assert!(
        node.parent_sig_pool.get(&hash, a.public_key()).is_some(),
        "anti-vacuity: the Valid arm must actually have run (M2 contract)"
    );
    assert_eq!(
        attester_series(a.public_key()),
        Some(1.0),
        "O2: one verifying half from {} must publish \
         {FAMILY_BY_ATTESTER}{{{LABEL}=\"{}\"}} = 1. A missing series means the label was \
         built from something other than the first 8 hex characters of the pubkey — \
         GS-018 matches this label against getProducers rows, so any other shape is \
         unmatchable.\n--- exported ---\n{}",
        a.public_key(),
        label_of(a.public_key()),
        encode_registry()
    );
    let after_total = fleet_total().expect("O1: the fleet-wide family must still render");
    assert!(
        after_total > before_total,
        "O1: the fleet-wide series must MOVE in the rendered exposition \
         ({before_total} -> {after_total}); registered-and-never-written is the INC-I-187 \
         shape and reads on a dashboard exactly like 'nobody is dual-signing'"
    );
}

/// REQ-BLS-006 AC-2 — Decision: a failure means the signal is gossip-only, so any
/// producer whose halves reach this node through a sync response is invisible to the
/// probe and would be counted as NOT dual-signing — the AC-2 denominator would be
/// wrong in the direction that blocks the AH forever.
#[tokio::test]
async fn req_bls_006_a_valid_first_seen_direct_ingest_publishes_a_per_attester_series() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, _h) = canonical_block(&mut node).await;
    let height = tip_height(&node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[4];
    register_bls(&node, a.public_key(), &bls).await;
    let att = dual(a, &bls, hash, slot, height);

    let _guard = counter_lock().await;
    assert_unseen(a.public_key(), "P1 direct");

    node.record_direct_attestation(att, PeerId::random()).await;

    assert!(
        node.parent_sig_pool.get(&hash, a.public_key()).is_some(),
        "anti-vacuity: the Valid arm must actually have run"
    );
    assert_eq!(
        attester_series(a.public_key()),
        Some(1.0),
        "O2: the direct ingress runs the SAME shared body, so it must publish the same \
         per-attester series"
    );
}

/// REQ-BLS-006 AC-2 — Decision: THE rate bound. `bls_verdict` returns `Valid` again
/// for a byte-identical re-delivery (the pooled fast path at ingress.rs:124-130), so
/// an unguarded `.inc()` in the Valid arm counts once per RELAY, not once per
/// attester per block. At N=45 with a full mesh that is ~2 orders of magnitude of
/// inflation on both series and one log line per relayed copy — the counter would
/// then measure gossip fan-out, and any AC-2 threshold read off it is meaningless.
#[tokio::test]
async fn req_bls_006_a_redelivered_identical_valid_half_is_counted_once() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[5];
    register_bls(&node, a.public_key(), &bls).await;
    let att = dual(a, &bls, hash, slot, height);
    let bytes = att.to_bytes();

    let _guard = counter_lock().await;
    assert_unseen(a.public_key(), "P1r");

    node.on_new_attestation(bytes.clone(), PeerId::random())
        .await;
    assert_eq!(
        attester_series(a.public_key()),
        Some(1.0),
        "precondition: the first delivery counts"
    );

    node.on_new_attestation(bytes.clone(), PeerId::random())
        .await;
    node.on_new_attestation(bytes, PeerId::random()).await;

    assert_eq!(
        attester_series(a.public_key()),
        Some(1.0),
        "O2: first-seen only. Two further relays of the SAME (block_hash, attester) must \
         not move the series; determine first-seen from `parent_sig_pool` BEFORE the insert"
    );
    assert_eq!(
        node.parent_sig_pool.total_signatures(),
        1,
        "anti-vacuity: all three deliveries did reach the Valid arm"
    );
}

// ===========================================================================
// P2 / P3 / P4 / P5 — nothing but a verifying half may publish the series
// ===========================================================================

/// REQ-BLS-006 AC-2 — Decision: a failure makes the series say "this producer
/// dual-signs" for a producer whose BLS key does not match its on-chain key. AC-2
/// would read 100 % and the AH would be pinned; at the AH those same producers stop
/// contributing a verifying aggregate and the network fails blocks it must accept.
#[tokio::test]
async fn req_bls_006_an_invalid_bls_half_publishes_no_attester_series() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[6];
    register_bls(&node, a.public_key(), &bls).await;

    let _guard = counter_lock().await;
    assert_unseen(a.public_key(), "P4");

    node.on_new_attestation(
        with_bls(a, hash, slot, height, vec![0xC3u8; 96]).to_bytes(),
        PeerId::random(),
    )
    .await;

    assert!(
        attended(&node, slot, a.public_key()),
        "anti-vacuity: the ingest reached the verdict (attendance is Ed25519-authenticated)"
    );
    assert_eq!(
        attester_series(a.public_key()),
        None,
        "O2: the Invalid arm must publish nothing. A series here is a false green on AC-2"
    );
}

/// REQ-BLS-006 AC-2 — Decision: an empty BLS half is a Release N-1 peer, which is
/// the exact population AC-2 is counting DOWN to zero. Counting it as dual-signing
/// inverts the measurement and would declare the rollout complete while un-upgraded
/// producers are still the majority.
#[tokio::test]
async fn req_bls_006_an_empty_bls_half_publishes_no_attester_series() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let old = &producers[7];

    let _guard = counter_lock().await;
    assert_unseen(old.public_key(), "P2");

    node.on_new_attestation(
        with_bls(old, hash, slot, height, Vec::new()).to_bytes(),
        PeerId::random(),
    )
    .await;

    assert!(
        attended(&node, slot, old.public_key()),
        "anti-vacuity: the bridge arm still attends (REQ-BLS-010)"
    );
    assert_eq!(
        attester_series(old.public_key()),
        None,
        "O2: the Empty arm must publish nothing"
    );
}

/// REQ-BLS-006 AC-2 — Decision: a producer with no on-chain `bls_pubkey` cannot
/// contribute to a post-AH aggregate at all. If NoKey published the series, AC-2
/// would report a producer as ready whose halves are structurally unusable, and the
/// AH would be pinned on a fleet that cannot form a verifying aggregate.
#[tokio::test]
async fn req_bls_006_an_attester_with_no_onchain_key_publishes_no_attester_series() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let bls = BlsKeyPair::generate();
    let legacy = &producers[8];

    let _guard = counter_lock().await;
    assert_unseen(legacy.public_key(), "P3");

    node.on_new_attestation(
        dual(legacy, &bls, hash, slot, height).to_bytes(),
        PeerId::random(),
    )
    .await;

    assert!(
        attended(&node, slot, legacy.public_key()),
        "anti-vacuity: the NoKey arm still attends"
    );
    assert_eq!(
        attester_series(legacy.public_key()),
        None,
        "O2: the NoKey arm must publish nothing"
    );
}

/// REQ-BLS-006 AC-2 — Decision: membership is what bounds this label's cardinality
/// to the ProducerSet. If a non-member could publish a series, any peer could mint
/// unbounded keys and grow the exposition without limit — a metrics-side DoS on
/// every scraper in the fleet, and the label would stop meaning "a chain-registered
/// producer", which is the only reason GS-018 can join it to getProducers.
#[tokio::test]
async fn req_bls_006_a_non_member_publishes_no_attester_series() {
    let (mut node, _producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let stranger = KeyPair::generate();
    let bls = BlsKeyPair::generate();

    let _guard = counter_lock().await;
    assert_unseen(stranger.public_key(), "P5");

    node.on_new_attestation(
        dual(&stranger, &bls, hash, slot, height).to_bytes(),
        PeerId::random(),
    )
    .await;

    assert_eq!(
        node.minute_tracker.total_entries(),
        0,
        "anti-vacuity: the non-member was dropped before the verdict (INC-I-192)"
    );
    assert_eq!(
        attester_series(stranger.public_key()),
        None,
        "O2: cardinality is bounded by the ProducerSet, not by what a peer sends"
    );
}

// ===========================================================================
// REQ-BLS-007 — the capability marker and the label shape
// ===========================================================================

/// REQ-BLS-007 — Decision: GS-018 keys its SKIP on the ABSENCE of this family
/// ("this node runs a build older than M7.5"). If the family only appears after the
/// first verifying half, absence is ambiguous — an M7.5 node on a fleet where nobody
/// dual-signs is indistinguishable from a pre-M7.5 node, and the probe SKIPs on
/// exactly the fleet state it exists to detect. Asserted as PRESENCE, not as the
/// literal 0: the `it` binary is one process and sibling tests write this series
/// concurrently, so a zero-value assertion would be a race, not a contract.
#[tokio::test]
async fn req_bls_007_the_fleet_wide_valid_counter_is_registered_from_process_start() {
    let _guard = counter_lock().await;
    let text = encode_registry();
    assert!(
        text.contains(FAMILY_TOTAL),
        "{FAMILY_TOTAL} is not published by register_metrics(). Zero-initialise it there \
         (the ATTESTATION_VERIFY_TOTAL pattern), or GS-018 cannot tell an old build from a \
         silent fleet.\n--- exported ---\n{text}"
    );
    assert!(
        fleet_total().is_some(),
        "{FAMILY_TOTAL} renders no series. `REGISTRY.register` alone is the contract: an \
         IntCounter that is registered exports 0 immediately"
    );
}

/// REQ-BLS-007 — Decision: the labelled family is NOT zero-initialisable (label
/// values are dynamic), so its absence at process start is expected and must never
/// be treated as a build marker. A failure here means someone zero-initialised it
/// with a placeholder label, which would make GS-018 count a fictional producer.
#[tokio::test]
async fn req_bls_007_the_attester_family_carries_only_real_eight_hex_labels() {
    let _guard = counter_lock().await;
    for value in exported_label_values(FAMILY_BY_ATTESTER, LABEL) {
        assert_eq!(
            value.len(),
            8,
            "{FAMILY_BY_ATTESTER}{{{LABEL}=\"{value}\"}} is not an 8-hex-character pubkey \
             prefix. GS-018 joins this label to getProducers by prefix; any other shape \
             (a full 64-hex key, a placeholder, a peer id) is unjoinable"
        );
        assert!(
            value.chars().all(|c| c.is_ascii_hexdigit()),
            "{FAMILY_BY_ATTESTER}{{{LABEL}=\"{value}\"}} is not hexadecimal"
        );
    }
}

/// REQ-BLS-006 AC-2 — Decision: `PublicKey`'s Display is `write!(f, "{}", to_hex())`,
/// which DROPS the outer formatter's precision, so `format!("{:.8}", pk)` yields the
/// full 64 hex characters — the `{:.8}` in the existing `[ATTEST_INGEST]` lines
/// truncates nothing. The label must therefore be SLICED. A failure here means
/// crypto switched to `f.pad`, at which point the label and the log line may share
/// one expression again; while it passes, any label built with `{:.8}` is a 64-hex
/// string and every GS-018 join silently misses.
#[test]
fn req_bls_006_the_attester_label_must_be_sliced_not_precision_formatted() {
    let kp = KeyPair::generate();
    let pk = kp.public_key();
    let full = pk.to_hex();
    let eight = label_of(pk);

    assert_eq!(
        full.len(),
        64,
        "premise: an Ed25519 pubkey is 64 hex characters"
    );
    assert_eq!(eight.len(), 8);
    assert!(full.starts_with(&eight));
    assert_eq!(
        format!("{pk:.8}"),
        full,
        "if this now truncates, PublicKey::fmt started honouring precision and the label \
         may be built with {{:.8}}; until then it must be sliced from to_hex()"
    );
}

// ===========================================================================
// F2 — the instrument and the probe cannot drift apart
// ===========================================================================

/// REQ-BLS-007 — Decision: the fleet runs `--log-level info`, so `debug!` takes this
/// line's production rate to zero — that is the point of M7.6; a louder macro puts the
/// per-node log volume back on ~30 rolling-deployed mainnet nodes. The literal stays
/// frozen for an operator who raises the level, and the PRODUCTION signal is the
/// counter pair the two tests below assert.
#[test]
fn req_bls_007_the_valid_arm_emits_the_grep_literal_at_debug_level() {
    let src = code_only(&read(INGRESS_RS));
    let at = src.find(VALID_LOG_LITERAL).unwrap_or_else(|| {
        panic!("{INGRESS_RS} does not contain the frozen literal `{VALID_LOG_LITERAL}`")
    });
    let head = &src[..at];
    let debug = head
        .rfind("debug!(")
        .expect("the literal must be emitted by a `debug!` invocation");
    for other in ["info!(", "warn!(", "error!(", "trace!("] {
        if let Some(at_other) = head.rfind(other) {
            assert!(
                debug > at_other,
                "the valid-bls line is emitted by `{other}`, not `debug!`. A louder macro \
                 restores the per-node log volume M7.6 removes; a quieter one hides the frozen \
                 literal from an operator who raises the level to debug"
            );
        }
    }
}

/// REQ-BLS-007 — Decision: the exactness assertions above land on the per-attester
/// series, because sibling test modules write the fleet-wide series concurrently and
/// a delta of exactly 1 on it would be a race. This tripwire is what stops the
/// fleet-wide counter from being forgotten entirely — without it, another test's
/// concurrent increment could satisfy the "it moved" assertion.
#[test]
fn req_bls_007_the_valid_arm_writes_both_counter_families() {
    let src = code_only(&read(INGRESS_RS));
    for symbol in [
        "ATTESTATION_BLS_VALID_TOTAL",
        "ATTESTATION_BLS_VALID_BY_ATTESTER",
    ] {
        assert!(
            src.contains(symbol),
            "{INGRESS_RS} never writes `{symbol}`; the Valid arm must increment BOTH \
             families at the same site, or the fleet-wide total and the per-attester \
             series disagree and neither can be trusted"
        );
    }

    let metrics = code_only(&read(METRICS_RS));
    for name in [FAMILY_TOTAL, FAMILY_BY_ATTESTER] {
        assert!(
            metrics.contains(name),
            "{METRICS_RS} does not define `{name}`"
        );
    }
    assert!(
        metrics.matches("ATTESTATION_BLS_VALID_TOTAL").count() >= 2,
        "`ATTESTATION_BLS_VALID_TOTAL` must be DEFINED and REGISTERED in {METRICS_RS}; a \
         defined-but-unregistered counter renders nothing and GS-018 reads its absence as \
         'old build' on every node in the fleet"
    );
    assert!(
        metrics.contains("\"attester\""),
        "`{FAMILY_BY_ATTESTER}` must carry the `attester` label; an unlabelled duplicate of \
         the total answers nothing that REQ-BLS-006 AC-2 asks"
    );
}

/// REQ-BLS-007 — Decision: the whole milestone exists to unblock GS-018's
/// `_gs018_dual_check`, which SKIPs today because no series carries a producer
/// label. If the probe is not rewired to these two names, M7.5 ships an instrument
/// nothing reads and AC-2 stays unobservable — the milestone's outcome would be
/// entirely internal.
#[test]
fn req_bls_007_gs018_reads_both_metric_names() {
    let script = sh_code_only(&read(GS018_SH));
    for name in [FAMILY_TOTAL, FAMILY_BY_ATTESTER] {
        assert!(
            script.contains(name),
            "{GS018_SH} does not read `{name}` in executable code. The probe and the \
             instrument must name the same series or they drift apart silently"
        );
    }
    assert!(
        script.contains("_gs018_dual_check"),
        "{GS018_SH} must still define the dual-sign check it is being rewired to"
    );
}
