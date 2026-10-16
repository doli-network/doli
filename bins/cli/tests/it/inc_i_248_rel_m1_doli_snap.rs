// covers: bins/cli/src/cmd_snap.rs, bins/cli/src/cmd_snap_maintainer.rs, bins/cli/src/main.rs, bins/cli/src/lib.rs, crates/core/src/maintainer/retired.rs, crates/core/src/maintainer/mod.rs, crates/core/src/network_params/mod.rs, crates/core/src/network_params/defaults.rs
//! INC-I-248 REL-M1 — `doli snap` maintainer digest agreement mirrors the state-root rule,
//! and a set holding a key retired at the snapshot height is written EMPTY.
//! REQ-248-007 (Must) — Decision: a red here means `doli snap` installs the leaked genesis keys past 331_457, or trusts one seed while the other configured seeds give no maintainer answer (SEC-INJECTION-001/002, SEC-CONFIG-002).
//
// OUTPUT CONTRACT: `doli --network N snap --data-dir D --seed S.. --no-restart`
//   O1: exit status 0; O2: D/maintainer_state.bin exists; O3: set EMPTY | canonical; O4: ldh == H
// PATHS: B1 3 seeds, 2 reachable without a maintainer answer + 1 canonical -> EMPTY@H;
//   B2 mainnet, snapshot set = retired five at H = 480_001, 3 seeds agree -> EMPTY@H
// OUTPUT CONTRACT: fn agree_maintainer_digest(answers, snapshot_digest, seed_count, trust) -> Option<String>
//   O3: Some(snapshot_digest) iff (trust || count >= 1) && !(!trust && seed_count >= 2 && count < 2),
//   count = answers equal to snapshot_digest; pure
// PATHS: G1 accepted; G2 refused
// INPUT PARTITIONS: seed_count 1 | 2 | 3; count 0 | 1 | 2 | 3; trust false | true;
//   answers fewer than seeds (non-answering seeds)
// OUTPUT CONTRACT: fn write_snap_maintainer_state(data_dir, field, height, agreed, genesis, retired_after) -> Result<SnapMaintainerOutcome>
//   O3: Canonical | FailClosed; O4: maintainer_state.bin == {set | EMPTY, height}
// PATHS: W1 retired key and height > retired_after -> FailClosed EMPTY@H;
//   W2 height == retired_after | retired_after MAX | no retired key -> Canonical
// MATRIX: B1/B2 x O1-O4 = 8; agree 13 rows; writer 5 rows x (O3, O4) = 10 cells

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output};

use crypto::{Hash, PublicKey};
use doli_cli::cmd_snap_maintainer::{
    agree_maintainer_digest, write_snap_maintainer_state, SnapMaintainerOutcome,
};
use doli_core::consensus::ConsensusParams;
use doli_core::genesis::MAINNET_GENESIS_PRODUCERS;
use doli_core::maintainer::{maintainer_set_digest, MaintainerSet};
use doli_core::Network;
use serde_json::{json, Value};
use storage::{ChainState, MaintainerState, ProducerSet, StateSnapshot, UtxoSet};
use tempfile::TempDir;

const TESTNET_H: u64 = 25_000;
const MAINNET_H: u64 = 480_001;
const RETIREMENT: u64 = 331_457;

fn genesis(network: Network) -> Hash {
    ConsensusParams::for_network(network).genesis_hash
}

fn retired_five() -> Vec<PublicKey> {
    MAINNET_GENESIS_PRODUCERS
        .iter()
        .map(|(hex, _)| PublicKey::from_hex(hex).expect("retired key hex"))
        .collect()
}

fn canonical_set() -> MaintainerSet {
    let members = (0..5u8)
        .map(|i| *crypto::KeyPair::from_seed([0xA0 + i; 32]).public_key())
        .collect();
    MaintainerSet::with_members(members, 20_000)
}

fn digest_hex(set: &MaintainerSet, network: Network) -> String {
    hex::encode(maintainer_set_digest(set, genesis(network).as_bytes()))
}

fn field_for(set: &MaintainerSet, height: u64, network: Network) -> Value {
    json!({
        "members": set.members.iter().map(|k| k.to_hex()).collect::<Vec<_>>(),
        "threshold": set.threshold,
        "lastChangeBlock": set.last_updated,
        "lastDerivedHeight": height,
        "digest": digest_hex(set, network),
        "genesisHash": genesis(network).to_hex(),
    })
}

fn snapshot_result(network: Network, height: u64, set: &MaintainerSet) -> Value {
    let mut cs = ChainState::new(genesis(network));
    cs.best_height = height;
    cs.best_hash = Hash::from_bytes([0x5A; 32]);
    cs.best_slot = height as u32;
    let ah = network.params().inc_i_181_pending_root_activation_height;
    let snap = StateSnapshot::create_gated(&cs, &UtxoSet::new(), &ProducerSet::new(), ah)
        .expect("fixture: snapshot");
    json!({
        "height": snap.block_height,
        "blockHash": snap.block_hash.to_hex(),
        "stateRoot": snap.state_root.to_hex(),
        "chainState": hex::encode(&snap.chain_state_bytes),
        "utxoSet": hex::encode(&snap.utxo_set_bytes),
        "producerSet": hex::encode(&snap.producer_set_bytes),
        "epochBondSnapshot": null,
        "epochAccumulators": null,
        "totalBytes": snap.total_bytes(),
        "maintainerState": field_for(set, height, network),
    })
}

fn read_body(stream: &mut TcpStream) -> Option<String> {
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
                return Some(String::from_utf8_lossy(&buf[end + 4..end + 4 + len]).to_string());
            }
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// Loopback JSON-RPC seed; `maintainer_digest = None` answers getMaintainerSet with an error.
fn start_seed(snapshot: Value, maintainer_digest: Option<String>, member_count: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("seed: bind");
    let url = format!("http://{}", listener.local_addr().expect("seed: addr"));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let Some(body) = read_body(&mut stream) else {
                continue;
            };
            let req: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let ok = |r: Value| json!({ "jsonrpc": "2.0", "id": 1, "result": r });
            let err = json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32601, "message": "Method not found" } });
            let payload = match (req["method"].as_str().unwrap_or(""), &maintainer_digest) {
                ("getStateRootDebug", _) => ok(json!({
                    "height": snapshot["height"], "stateRoot": snapshot["stateRoot"],
                })),
                ("getStateSnapshot", _) => ok(snapshot.clone()),
                ("getMaintainerSet", Some(d)) => ok(json!({
                    "member_count": member_count, "source": "on-chain", "maintainer_set_digest": d,
                })),
                _ => err,
            };
            let text = payload.to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                text.len(),
                text
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.shutdown(Shutdown::Both);
        }
    });
    url
}

fn run_snap(network: &str, dir: &Path, seeds: &[String]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_doli"));
    cmd.args(["--network", network, "snap", "--data-dir"])
        .arg(dir)
        .arg("--no-restart");
    for s in seeds {
        cmd.args(["--seed", s]);
    }
    for v in [
        "DOLI_NETWORK",
        "DOLI_RPC_URL",
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        cmd.env_remove(v);
    }
    cmd.env("NO_PROXY", "127.0.0.1");
    cmd.output().expect("spawn doli")
}

fn assert_empty_written(dir: &Path, out: &Output, height: u64, ctx: &str) {
    let diag = format!(
        "exit={:?}\nstdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{ctx}: O1 exit 0\n{diag}");
    let st = MaintainerState::load(dir).unwrap_or_else(|e| panic!("{ctx}: O2 load: {e}\n{diag}"));
    assert!(
        st.set.members.is_empty(),
        "{ctx}: O3 expected EMPTY, got {} members\n{diag}",
        st.set.members.len()
    );
    assert_eq!(st.last_derived_height, height, "{ctx}: O4\n{diag}");
}

// REQ-248-007 — Decision: a red means one canonical seed is trusted while two reachable seeds give no maintainer answer.
#[test]
fn inc_i_248_rel_m1_doli_snap_non_answering_seeds_fail_closed() {
    let tmp = TempDir::new().unwrap();
    let set = canonical_set();
    let snap = snapshot_result(Network::Testnet, TESTNET_H, &set);
    let d = digest_hex(&set, Network::Testnet);
    let seeds = vec![
        start_seed(snap.clone(), Some(d), 5),
        start_seed(snap.clone(), None, 5),
        start_seed(snap, None, 5),
    ];
    let out = run_snap("testnet", tmp.path(), &seeds);
    assert_empty_written(tmp.path(), &out, TESTNET_H, "B1 1 of 3 seeds answers");
}

// REQ-248-007 — Decision: a red means `doli snap` on mainnet installs the 5 retired genesis keys above 331_457.
#[test]
fn inc_i_248_rel_m1_doli_snap_mainnet_retired_set_fails_closed() {
    let tmp = TempDir::new().unwrap();
    let set = MaintainerSet::with_members(retired_five(), 0);
    let snap = snapshot_result(Network::Mainnet, MAINNET_H, &set);
    let d = digest_hex(&set, Network::Mainnet);
    let seeds: Vec<String> = (0..3)
        .map(|_| start_seed(snap.clone(), Some(d.clone()), 5))
        .collect();
    let out = run_snap("mainnet", tmp.path(), &seeds);
    assert_empty_written(tmp.path(), &out, MAINNET_H, "B2 retired five at 480_001");
}

const D_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const D_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn answers(ds: &[&str]) -> Vec<(String, String)> {
    ds.iter()
        .enumerate()
        .map(|(i, d)| (format!("http://seed-{i}:8500"), d.to_string()))
        .collect()
}

// REQ-248-007 — Decision: a red means the maintainer digest vote differs from the state-root vote in cmd_snap.rs (non-answering seeds shrink the seed count, or a non-snapshot digest wins).
#[test]
fn inc_i_248_rel_m1_agree_mirrors_state_root_rule() {
    let some = Some(D_A.to_string());
    type Row<'a> = (&'a [&'a str], usize, bool, Option<String>, &'a str);
    let rows: [Row; 13] = [
        (&[D_A], 3, false, None, "1 of 3 answers (2 seeds silent)"),
        (
            &[D_A, D_A],
            3,
            false,
            some.clone(),
            "2 of 3 agree, 1 silent",
        ),
        (
            &[D_A, D_A, D_B],
            3,
            false,
            some.clone(),
            "2 of 3 agree, 1 dissents",
        ),
        (
            &[D_B, D_B, D_A],
            3,
            false,
            None,
            "the majority is not the snapshot digest",
        ),
        (&[D_A, D_A, D_A], 3, false, some.clone(), "3 of 3"),
        (&[D_A], 2, false, None, "1 of 2"),
        (
            &[D_A, D_A, D_B, D_B],
            4,
            false,
            some.clone(),
            "2-2 tie: count >= 2 is accepted",
        ),
        (&[D_A], 1, false, some.clone(), "single configured seed"),
        (&[D_B], 1, false, None, "single seed, other digest"),
        (&[], 1, false, None, "no answer"),
        (&[], 3, false, None, "no answers"),
        (
            &[D_B, D_B, D_B],
            3,
            true,
            some.clone(),
            "--trust accepts like the state root",
        ),
        (&[], 1, true, some, "--trust, no answer"),
    ];
    for (ds, seeds, trust, want, case) in rows {
        assert_eq!(
            agree_maintainer_digest(&answers(ds), D_A, seeds, trust),
            want,
            "{case}"
        );
    }
}

// REQ-248-007 — Decision: a red means the writer installs a set holding a key retired at the snapshot height, or refuses one at or below it.
#[test]
fn inc_i_248_rel_m1_writer_retired_key_at_height() {
    let g = genesis(Network::Mainnet);
    let retired = MaintainerSet::with_members(retired_five(), 0);
    let mut members: Vec<PublicKey> = canonical_set().members[..4].to_vec();
    members.push(retired_five()[1]);
    let one = MaintainerSet::with_members(members, 20_000);
    let rows = [
        (&retired, MAINNET_H, RETIREMENT, false, "retired five above"),
        (&one, MAINNET_H, RETIREMENT, false, "one retired key above"),
        (
            &retired,
            RETIREMENT,
            RETIREMENT,
            true,
            "at the retirement height",
        ),
        (
            &retired,
            MAINNET_H,
            u64::MAX,
            true,
            "never retired (testnet/devnet)",
        ),
        (
            &canonical_set(),
            MAINNET_H,
            RETIREMENT,
            true,
            "no retired key",
        ),
    ];
    for (set, h, retired_after, canonical, case) in rows {
        let tmp = TempDir::new().unwrap();
        let field = field_for(set, h, Network::Mainnet);
        let d = digest_hex(set, Network::Mainnet);
        let outcome =
            write_snap_maintainer_state(tmp.path(), &field, h, Some(&d), &g, retired_after)
                .expect("writer must not error");
        let st = MaintainerState::load(tmp.path()).expect("written file loads");
        assert_eq!(st.last_derived_height, h, "{case}: height");
        if canonical {
            assert_eq!(outcome, SnapMaintainerOutcome::Canonical, "{case}");
            assert_eq!(st.set, *set, "{case}: set");
        } else {
            assert!(
                matches!(outcome, SnapMaintainerOutcome::FailClosed(_)),
                "{case}: {outcome:?}"
            );
            assert!(st.set.members.is_empty(), "{case}: EMPTY");
        }
    }
}
