//! INC-I-248 M1.5 — unit contract of `doli_cli::cmd_snap_maintainer`.
//! REQ-248-002 — Decision: a red here means `doli snap` installs an unverified set or no fail-closed root.
//
// OUTPUT CONTRACT (REL-M1): fn agree_maintainer_digest(answers: &[(String, String)],
//   snapshot_digest: &str, seed_count: usize, trust: bool) -> Option<String>
//   O1: return — Some(snapshot_digest) iff (trust || count >= 1) && !(!trust && seed_count >= 2
//       && count < 2), count = answers equal to snapshot_digest (the cmd_snap.rs state-root rule)
// PATHS: A1 accepted ; A2 refused
// INPUT PARTITIONS:
//   A1a 3 agree -> Some ; A1b 2-of-3 (dissent first or last) -> Some ; A1b' 2-of-3 for another
//   digest -> None ; A1c all differ -> None ; A1d 2 seeds differ -> None ; A1e 2 agree -> Some
//   A1f tie 2-2 -> Some(snapshot digest) ; A2a 1 seed 1 answer -> Some ; A2a' 1 seed other -> None
//   A2b trust, 3 differ -> Some ; A2c trust, none matches -> Some ; A3a 0 -> None ; A3b 0 + trust -> Some
// MATRIX: 1 output x 13 partitions = 13 cells
//
// OUTPUT CONTRACT: fn write_snap_maintainer_state(data_dir, field, height, agreed_digest, genesis_hash,
//                      retired_after: u64) -> anyhow::Result<SnapMaintainerOutcome>
//   O1: return — Ok(Canonical) | Ok(FailClosed(reason))
//   O2: data_dir/maintainer_state.bin — ALWAYS rewritten; bytes == MaintainerState::save of
//       {set, last_derived_height: height} (Canonical) or {MaintainerSet::new(), height} (FailClosed)
//   O3: the pre-existing different file is replaced on every path
// PATHS: W1 canonical ; W2 fail closed
// INPUT PARTITIONS:
//   W1a 5 canonical keys, lastChangeBlock < height
//   W1b lastChangeBlock == height (boundary)
//   W1c agreed set holds one retired mainnet genesis key, retired_after = u64::MAX (testnet)
//       -> written as received (the mainnet case is in inc_i_248_rel_m1_doli_snap.rs)
//   W2a field null/absent ; W2b field not an object ; W2c field digest != recomputed
//   W2d agreed None ; W2e agreed != field digest ; W2f lastChangeBlock > height
//   W2g member not hex ; W2h member wrong length ; W2i duplicate member
//   W2j threshold not reconciled ; W2k digest bound to another genesis
// MATRIX: 3 outputs x 14 partitions = 42 cells
//
// OUTPUT CONTRACT: async fn query_maintainer_digest(rpc_url) -> anyhow::Result<String>
//   O1: return — Ok(digest) only for source "on-chain", member_count > 0, digest string
// PATHS: Q1 counted ; Q2 not counted (Err)
// INPUT PARTITIONS:
//   Q1a on-chain 5 members ; Q2a source "derived" ; Q2b member_count 0
//   Q2c digest missing ; Q2d JSON-RPC error ; Q2e unreachable
// MATRIX: 1 output x 6 partitions = 6 cells
// covers: bins/cli/src/cmd_snap.rs, bins/cli/src/cmd_snap_maintainer.rs

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;

use crypto::{Hash, PublicKey};
use doli_cli::cmd_snap_maintainer::{
    agree_maintainer_digest, query_maintainer_digest, write_snap_maintainer_state,
    SnapMaintainerOutcome,
};
use doli_core::consensus::ConsensusParams;
use doli_core::genesis::MAINNET_GENESIS_PRODUCERS;
use doli_core::maintainer::{maintainer_set_digest, MaintainerSet};
use doli_core::Network;
use serde_json::{json, Value};
use storage::{MaintainerState, MAINTAINER_STATE_VERSION};
use tempfile::TempDir;

const H: u64 = 25_000;
const LAST_CHANGE: u64 = 20_000;
const D_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const D_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const D_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn genesis() -> Hash {
    ConsensusParams::for_network(Network::Testnet).genesis_hash
}

fn key(i: u8) -> PublicKey {
    *crypto::KeyPair::from_seed([0xA0 + i; 32]).public_key()
}

fn retired_keys() -> Vec<PublicKey> {
    MAINNET_GENESIS_PRODUCERS
        .iter()
        .map(|(h, _)| PublicKey::from_hex(h).expect("retired key hex"))
        .collect()
}

fn canonical_set() -> MaintainerSet {
    MaintainerSet::with_members((0..5).map(key).collect(), LAST_CHANGE)
}

fn digest_of(set: &MaintainerSet) -> String {
    hex::encode(maintainer_set_digest(set, genesis().as_bytes()))
}

fn field_for(set: &MaintainerSet, digest: &str) -> Value {
    json!({
        "members": set.members.iter().map(|k| k.to_hex()).collect::<Vec<_>>(),
        "threshold": set.threshold,
        "lastChangeBlock": set.last_updated,
        "lastDerivedHeight": H,
        "digest": digest,
        "genesisHash": genesis().to_hex(),
    })
}

fn answers(ds: &[&str]) -> Vec<(String, String)> {
    ds.iter()
        .enumerate()
        .map(|(i, d)| (format!("http://seed-{i}:8500"), d.to_string()))
        .collect()
}

fn saved_bytes(state: &MaintainerState) -> Vec<u8> {
    let tmp = TempDir::new().unwrap();
    state.save(tmp.path()).expect("reference save");
    std::fs::read(tmp.path().join("maintainer_state.bin")).unwrap()
}

fn state_at_h(set: MaintainerSet) -> MaintainerState {
    MaintainerState {
        version: MAINTAINER_STATE_VERSION,
        set,
        last_derived_height: H,
    }
}

/// Writes a pre-existing DIFFERENT file (the 5 retired keys at height 7), then runs the writer.
fn run_writer(field: &Value, agreed: Option<&str>) -> (TempDir, SnapMaintainerOutcome) {
    let tmp = TempDir::new().unwrap();
    MaintainerState {
        version: MAINTAINER_STATE_VERSION,
        set: MaintainerSet::with_members(retired_keys(), 1),
        last_derived_height: 7,
    }
    .save(tmp.path())
    .expect("fixture: pre-existing file");
    let outcome = write_snap_maintainer_state(tmp.path(), field, H, agreed, &genesis(), u64::MAX)
        .expect("O1: the writer must not error on a writable data dir");
    (tmp, outcome)
}

fn file_bytes(dir: &Path) -> Vec<u8> {
    std::fs::read(dir.join("maintainer_state.bin"))
        .expect("O2: maintainer_state.bin must exist after the writer")
}

fn assert_canonical(field: &Value, agreed: Option<&str>, want: MaintainerSet, case: &str) {
    let (tmp, outcome) = run_writer(field, agreed);
    assert_eq!(outcome, SnapMaintainerOutcome::Canonical, "O1 [{case}]");
    assert_eq!(
        file_bytes(tmp.path()),
        saved_bytes(&state_at_h(want.clone())),
        "O2/O3 [{case}]: file must hold the received set at height {H}"
    );
    let st = MaintainerState::load(tmp.path()).expect("O2: loads");
    assert_eq!(st.set, want, "O2 [{case}]");
    assert_eq!(st.last_derived_height, H, "O2 [{case}]");
}

fn assert_fail_closed(field: &Value, agreed: Option<&str>, case: &str) {
    let (tmp, outcome) = run_writer(field, agreed);
    assert!(
        matches!(outcome, SnapMaintainerOutcome::FailClosed(_)),
        "O1 [{case}]: expected FailClosed, got {outcome:?}"
    );
    assert_eq!(
        file_bytes(tmp.path()),
        saved_bytes(&state_at_h(MaintainerSet::new())),
        "O2/O3 [{case}]: file must be EMPTY@{H}, replacing the pre-existing file"
    );
    let st = MaintainerState::load(tmp.path()).expect("O2: loads");
    assert!(st.set.members.is_empty(), "O2 [{case}]: EMPTY");
    assert_eq!(st.last_derived_height, H, "O2 [{case}]");
}

// ---- agree_maintainer_digest ------------------------------------------------

// INC-I-248 / REQ-248-002 — Decision: red means the maintainer digest vote differs from the state-root vote.
#[test]
fn inc_i_248_agree_three_agree() {
    assert_eq!(
        agree_maintainer_digest(&answers(&[D_A, D_A, D_A]), D_A, 3, false),
        Some(D_A.into())
    );
}

#[test]
fn inc_i_248_agree_two_of_three() {
    assert_eq!(
        agree_maintainer_digest(&answers(&[D_A, D_A, D_B]), D_A, 3, false),
        Some(D_A.into())
    );
    assert_eq!(
        agree_maintainer_digest(&answers(&[D_B, D_A, D_A]), D_A, 3, false),
        Some(D_A.into())
    );
    assert_eq!(
        agree_maintainer_digest(&answers(&[D_A, D_A, D_B]), D_B, 3, false),
        None,
        "the count is of the snapshot digest, not of the largest group"
    );
}

#[test]
fn inc_i_248_agree_all_differ_is_none() {
    assert_eq!(
        agree_maintainer_digest(&answers(&[D_A, D_B, D_C]), D_A, 3, false),
        None
    );
}

#[test]
fn inc_i_248_agree_two_answers() {
    assert_eq!(
        agree_maintainer_digest(&answers(&[D_A, D_B]), D_A, 2, false),
        None
    );
    assert_eq!(
        agree_maintainer_digest(&answers(&[D_A, D_A]), D_A, 2, false),
        Some(D_A.into())
    );
}

#[test]
fn inc_i_248_agree_tie_two_two_follows_snapshot_digest() {
    assert_eq!(
        agree_maintainer_digest(&answers(&[D_A, D_A, D_B, D_B]), D_A, 4, false),
        Some(D_A.into())
    );
    assert_eq!(
        agree_maintainer_digest(&answers(&[D_A, D_B, D_B, D_A]), D_B, 4, false),
        Some(D_B.into())
    );
    assert_eq!(
        agree_maintainer_digest(&answers(&[D_A, D_B, D_B, D_A]), D_C, 4, false),
        None
    );
}

#[test]
fn inc_i_248_agree_single_seed_counts_one() {
    assert_eq!(
        agree_maintainer_digest(&answers(&[D_B]), D_B, 1, false),
        Some(D_B.into())
    );
    assert_eq!(
        agree_maintainer_digest(&answers(&[D_B]), D_A, 1, false),
        None
    );
    assert_eq!(
        agree_maintainer_digest(&answers(&[D_B]), D_B, 3, false),
        None,
        "two configured seeds gave no answer"
    );
}

#[test]
fn inc_i_248_agree_trust_accepts_snapshot_digest() {
    assert_eq!(
        agree_maintainer_digest(&answers(&[D_C, D_A, D_B]), D_C, 3, true),
        Some(D_C.into())
    );
    assert_eq!(
        agree_maintainer_digest(&answers(&[D_A, D_A, D_A]), D_C, 3, true),
        Some(D_C.into()),
        "--trust verifies like the state root: no count needed"
    );
}

#[test]
fn inc_i_248_agree_no_answers() {
    assert_eq!(agree_maintainer_digest(&[], D_A, 0, false), None);
    assert_eq!(agree_maintainer_digest(&[], D_A, 3, false), None);
    assert_eq!(agree_maintainer_digest(&[], D_A, 1, true), Some(D_A.into()));
}

// ---- write_snap_maintainer_state: canonical ---------------------------------

// INC-I-248 / REQ-248-002 — Decision: red means a quorum-agreed snapshot set is not installed verbatim.
#[test]
fn inc_i_248_writer_canonical() {
    let set = canonical_set();
    let d = digest_of(&set);
    assert_canonical(&field_for(&set, &d), Some(&d), set, "W1a");
}

#[test]
fn inc_i_248_writer_last_change_equal_height_is_canonical() {
    let set = MaintainerSet::with_members((0..5).map(key).collect(), H);
    let d = digest_of(&set);
    assert_canonical(&field_for(&set, &d), Some(&d), set, "W1b");
}

#[test]
fn inc_i_248_writer_retired_key_set_written_as_received() {
    let mut members: Vec<PublicKey> = (0..4).map(key).collect();
    members.push(retired_keys()[0]);
    let set = MaintainerSet::with_members(members, LAST_CHANGE);
    let d = digest_of(&set);
    assert_canonical(&field_for(&set, &d), Some(&d), set, "W1c");
}

// ---- write_snap_maintainer_state: fail closed -------------------------------

// INC-I-248 / REQ-248-002 — Decision: red means an unverifiable maintainer field becomes a trust root.
#[test]
fn inc_i_248_writer_field_null_or_absent_fails_closed() {
    let d = digest_of(&canonical_set());
    assert_fail_closed(&Value::Null, Some(&d), "W2a null");
    let absent = json!({ "height": H });
    assert_fail_closed(&absent["maintainerState"], Some(&d), "W2a absent");
}

#[test]
fn inc_i_248_writer_field_not_object_fails_closed() {
    let d = digest_of(&canonical_set());
    assert_fail_closed(&json!("not-an-object"), Some(&d), "W2b string");
    assert_fail_closed(&json!([1, 2, 3]), Some(&d), "W2b array");
}

#[test]
fn inc_i_248_writer_field_digest_mismatch_fails_closed() {
    let field = field_for(&canonical_set(), D_A);
    assert_fail_closed(&field, Some(D_A), "W2c");
}

#[test]
fn inc_i_248_writer_agreed_none_fails_closed() {
    let set = canonical_set();
    assert_fail_closed(&field_for(&set, &digest_of(&set)), None, "W2d");
}

#[test]
fn inc_i_248_writer_agreed_differs_from_field_fails_closed() {
    let set = canonical_set();
    assert_fail_closed(&field_for(&set, &digest_of(&set)), Some(D_B), "W2e");
}

#[test]
fn inc_i_248_writer_last_change_above_height_fails_closed() {
    let set = MaintainerSet::with_members((0..5).map(key).collect(), H + 1);
    let d = digest_of(&set);
    assert_fail_closed(&field_for(&set, &d), Some(&d), "W2f");
}

#[test]
fn inc_i_248_writer_bad_member_hex_fails_closed() {
    let set = canonical_set();
    let d = digest_of(&set);
    let mut field = field_for(&set, &d);
    field["members"][2] = json!("zz".repeat(32));
    assert_fail_closed(&field, Some(&d), "W2g not hex");
    let mut field = field_for(&set, &d);
    field["members"][2] = json!("abcd");
    assert_fail_closed(&field, Some(&d), "W2h wrong length");
}

#[test]
fn inc_i_248_writer_duplicate_member_fails_closed() {
    let set =
        MaintainerSet::with_members(vec![key(0), key(0), key(2), key(3), key(4)], LAST_CHANGE);
    let d = digest_of(&set);
    assert_fail_closed(&field_for(&set, &d), Some(&d), "W2i");
}

#[test]
fn inc_i_248_writer_unreconciled_threshold_fails_closed() {
    let mut set = canonical_set();
    set.threshold = 1;
    let d = digest_of(&set);
    assert_fail_closed(&field_for(&set, &d), Some(&d), "W2j");
}

#[test]
fn inc_i_248_writer_foreign_genesis_digest_fails_closed() {
    let set = canonical_set();
    let foreign = hex::encode(maintainer_set_digest(
        &set,
        Hash::from_bytes([0x77; 32]).as_bytes(),
    ));
    assert_fail_closed(&field_for(&set, &foreign), Some(&foreign), "W2k");
}

// ---- query_maintainer_digest -----------------------------------------------

/// Reads one HTTP request through its `Content-Length` body.
fn drain_request(s: &mut TcpStream) {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let len = String::from_utf8_lossy(&buf[..end])
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                .and_then(|l| l.split(':').nth(1))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buf.len() >= end + 4 + len {
                return;
            }
        }
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

fn one_shot_rpc(body: Value) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            drain_request(&mut s);
            let text = body.to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                text.len(),
                text
            );
            let _ = s.write_all(resp.as_bytes());
            let _ = s.shutdown(Shutdown::Both);
        }
    });
    url
}

fn on_chain(source: &str, member_count: usize, digest: Option<&str>) -> Value {
    let mut r = json!({
        "maintainers": (0..member_count as u8).map(|i| json!({ "pubkey": key(i).to_hex() })).collect::<Vec<_>>(),
        "threshold": MaintainerSet::calculate_threshold(member_count),
        "member_count": member_count,
        "last_change_block": LAST_CHANGE,
        "source": source,
        "enforced": source == "on-chain",
        "genesis_hash": genesis().to_hex(),
    });
    if let Some(d) = digest {
        r["maintainer_set_digest"] = json!(d);
    }
    json!({ "jsonrpc": "2.0", "id": 1, "result": r })
}

// INC-I-248 / REQ-248-002 — Decision: red means an advisory or empty seed answer is counted in the digest vote.
#[tokio::test]
async fn inc_i_248_query_counts_only_on_chain_non_empty() {
    let ok = query_maintainer_digest(&one_shot_rpc(on_chain("on-chain", 5, Some(D_A)))).await;
    assert_eq!(ok.expect("Q1a: on-chain answer counts"), D_A);

    let derived = query_maintainer_digest(&one_shot_rpc(on_chain("derived", 5, Some(D_A)))).await;
    assert!(derived.is_err(), "Q2a: source derived must not count");

    let empty = query_maintainer_digest(&one_shot_rpc(on_chain("on-chain", 0, Some(D_A)))).await;
    assert!(empty.is_err(), "Q2b: member_count 0 must not count");

    let no_digest = query_maintainer_digest(&one_shot_rpc(on_chain("on-chain", 5, None))).await;
    assert!(no_digest.is_err(), "Q2c: missing digest must not count");

    let err = json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32601, "message": "Method not found" } });
    assert!(
        query_maintainer_digest(&one_shot_rpc(err)).await.is_err(),
        "Q2d: JSON-RPC error must not count"
    );

    let dead = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    assert!(
        query_maintainer_digest(&dead).await.is_err(),
        "Q2e: unreachable must not count"
    );
}
