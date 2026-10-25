//! INC-I-262 M2 / R-5 — the offline replay tools on a real data directory:
//! `--network` and `--data-dir` on either side of the subcommand, the replay
//! node on the running node's chainspec parameters, and a read-only census of
//! Exited producers that still hold Bond UTXOs.
//!
// covers: cli main init producer_set_check exited_bond_census chain network_params run config chainspec_select cli_release lib
//
// OUTPUT CONTRACT: fn `doli-node [--network N --data-dir D] check-producer-set [--network N --data-dir D]`
//   O1 exit code: 0 EQUAL | 2 DIFFERS | 1 error
//   O2 stdout: one verdict line, the key=value summary on the next line
//   O3 data dir byte-identical; nothing left under the child's TMPDIR
// OUTPUT CONTRACT: fn Node::new_for_replay(PathBuf, Network) -> Result<Node>
//   O1 node.params == ConsensusParams::for_network(n) + apply_chainspec(the chainspec the running node uses)
//   O2 node.producer_set == the set a running node seeds on a fresh start (key, bond count, status)
//   O3 node.epoch_state.producer_list == the O2 keys, sorted
// OUTPUT CONTRACT: fn `doli-node --network devnet --data-dir D exited-bond-census`
//   O1 exit code: 0 | 1 refused (locked store)
//   O2 stdout: one JSON line per Exited producer holding >= 1 Bond UTXO:
//      {"key":<pubkey hex>,"status":"Exited","bond_count":n,"total":t,"bonds":[{"outpoint":"<tx hex>:<index>","amount":a}]}
//      then ONE key=value summary line: height=<h> exited_with_bonds=<n> bond_utxos=<m> total=<t>
//   O3 data dir byte-identical; nothing left under the child's TMPDIR
// PATHS: P-TESTNET store built on the embedded testnet chainspec | P-FLAGS after / before the subcommand
//        | P-PARAMS mainnet, testnet, devnet | P-CENSUS seeded, clean, locked
// INPUT PARTITIONS:
//   census producers: Exited + Bond UTXOs (+ a Normal UTXO) | Exited, no Bond UTXO | Active
//   store: devnet tip 42 (b3_m2 fixture) | testnet tip 3 inside the genesis window
// MATRIX:
//   O1,O2,O3 x P-TESTNET -> inc_i_262_m2_check_replays_a_testnet_store
//   O1,O2 x P-FLAGS x {after, before} -> inc_i_262_m2_check_takes_flags_after_the_subcommand
//   O1 x P-PARAMS x 3 networks -> inc_i_262_m2_replay_params_match_the_running_node
//   O2,O3 x P-PARAMS x 3 networks -> inc_i_262_m2_replay_genesis_producers_match_the_running_node
//   O1,O2,O3 x P-CENSUS seeded -> inc_i_262_m2_census_lists_exited_producers_holding_bonds
//   O1,O2,O3 x P-CENSUS clean -> inc_i_262_m2_census_clean_chain_prints_no_records
//   O1,O2 x P-CENSUS locked -> inc_i_262_m2_census_refuses_a_locked_store

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::block::{Block, BlockHeader};
use doli_core::chainspec::ChainSpec;
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Output, OutputType, Transaction};
use doli_core::validation::{bond_owner, ValidationMode};
use doli_core::Network;
use doli_node::node::Node;
use storage::{ChainState, Outpoint, ProducerSet, ProducerStatus, StateDb, UtxoEntry};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

use super::b3_m2_residue_check::{assert_outcome, build, tree, Run, TIP};
use crate::block_sig::sign_header;
use crate::inc_i_156_m1_harness::coinbase_for;

const TESTNET_SPEC: &str = include_str!("../../../../chainspec.testnet.json");
const MAINNET_SPEC: &str = include_str!("../../../../chainspec.mainnet.json");
const TESTNET_TIP: u64 = 3;
const SUMMARY_KEY: &str = "exited_with_bonds";

/// The chainspec a running node of `net` applies: the embedded one on mainnet
/// and testnet; on devnet only a data-dir `chainspec.json`, absent here.
fn running_spec(net: Network) -> Option<ChainSpec> {
    let json = match net {
        Network::Mainnet => MAINNET_SPEC,
        Network::Testnet => TESTNET_SPEC,
        Network::Devnet => return None,
    };
    Some(serde_json::from_str(json).expect("FIXTURE: embedded chainspec parses"))
}

fn running_params(net: Network) -> ConsensusParams {
    let mut p = ConsensusParams::for_network(net);
    if let Some(spec) = running_spec(net) {
        p.apply_chainspec(&spec);
    }
    p
}

/// The producer set a running node of `net` seeds on a fresh start: the
/// chainspec's genesis producers, else the network constants, else none.
fn running_genesis(net: Network) -> ProducerSet {
    let from_spec = running_spec(net).map_or_else(Vec::new, |s| s.get_genesis_producers());
    let producers = if !from_spec.is_empty() {
        from_spec
    } else {
        match net {
            Network::Mainnet => doli_core::genesis::mainnet_genesis_producers(),
            Network::Testnet => doli_core::genesis::testnet_genesis_producers(),
            Network::Devnet => Vec::new(),
        }
    };
    ProducerSet::with_genesis_producers(producers, net.bond_unit())
}

/// (key hex, bond count, status) per producer, in key order.
fn roster(ps: &ProducerSet) -> Vec<(String, u32, String)> {
    let mut v: Vec<_> = ps
        .all_producers()
        .iter()
        .map(|p| (p.public_key.to_hex(), p.bonds(), format!("{:?}", p.status)))
        .collect();
    v.sort();
    v
}

struct Store {
    _tmp: TempDir,
    data: PathBuf,
    scratch_root: PathBuf,
}

fn testnet_block(
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
    fork_id: Hash,
    txs: Vec<Transaction>,
) -> Block {
    let mut header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root: doli_core::block::compute_merkle_root(&txs),
        presence_root: Hash::ZERO,
        genesis_hash: params.genesis_hash,
        timestamp: params.genesis_time + u64::from(slot) * params.slot_duration,
        slot,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id,
    };
    sign_header(&mut header, producer);
    Block::new(header, txs)
}

/// A stopped testnet node: blocks 1..=TESTNET_TIP applied through
/// apply_block(Replay) on a node holding the running testnet node's parameters.
async fn testnet_store() -> Store {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let scratch_root = tmp.path().join("scratch-root");
    std::fs::create_dir_all(&scratch_root).unwrap();
    let spec = running_spec(Network::Testnet).expect("FIXTURE: testnet chainspec");
    assert_ne!(
        spec.genesis.timestamp, 0,
        "PRECONDITION: testnet chainspec genesis time"
    );
    let producer = KeyPair::generate();
    {
        let mut node = Node::new_for_replay(data.clone(), Network::Testnet)
            .await
            .expect("FIXTURE: Node::new_for_replay");
        node.params = running_params(Network::Testnet);
        node.config.genesis_time_override = Some(spec.genesis.timestamp);
        node.config.chainspec = Some(spec);
        let mut prev = node.chain_state.read().await.best_hash;
        for h in 1..=TESTNET_TIP {
            let slot = h as u32;
            let txs = vec![coinbase_for(h, slot, &node.params)];
            let block = testnet_block(
                slot,
                prev,
                &producer,
                &node.params,
                node.current_fork_id(),
                txs,
            );
            prev = block.hash();
            node.apply_block(block, ValidationMode::Replay)
                .await
                .unwrap_or_else(|e| panic!("FIXTURE: apply testnet h={h}: {e:#}"));
        }
    }
    let (_, cs) = persisted(&data);
    assert_eq!(
        cs.best_height, TESTNET_TIP,
        "PRECONDITION: persisted testnet tip"
    );
    Store {
        _tmp: tmp,
        data,
        scratch_root,
    }
}

fn run_node(scratch_root: &Path, args: Vec<OsString>) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_doli-node"));
    for (k, _) in std::env::vars() {
        if k.starts_with("DOLI_") {
            cmd.env_remove(k);
        }
    }
    let out = cmd
        .env("TMPDIR", scratch_root)
        .current_dir(scratch_root)
        .args(args)
        .output()
        .expect("spawn doli-node");
    Run {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// `pre` + `--data-dir <data>` + `post`.
fn argv(pre: &[&str], data: &Path, post: &[&str]) -> Vec<OsString> {
    let mut v: Vec<OsString> = pre.iter().map(OsString::from).collect();
    v.push("--data-dir".into());
    v.push(data.as_os_str().to_owned());
    v.extend(post.iter().map(OsString::from));
    v
}

fn census_args(data: &Path) -> Vec<OsString> {
    argv(
        &["--network", "devnet", "--log-level", "error"],
        data,
        &["exited-bond-census"],
    )
}

fn assert_clean(data: &Path, scratch_root: &Path, before: &BTreeMap<String, String>, what: &str) {
    assert_eq!(
        &tree(data),
        before,
        "{what}: the tool changed the data directory"
    );
    let left: Vec<String> = std::fs::read_dir(scratch_root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        left.is_empty(),
        "{what}: scratch left under TMPDIR: {left:?}"
    );
}

/// Census output: the JSON records, and the one summary line's key=value map.
fn census_output(r: &Run) -> (Vec<serde_json::Value>, BTreeMap<String, String>) {
    let lines: Vec<&str> = r
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let at: Vec<usize> = (0..lines.len())
        .filter(|&i| {
            lines[i]
                .split_whitespace()
                .any(|t| t.starts_with(&format!("{SUMMARY_KEY}=")))
        })
        .collect();
    assert_eq!(
        at.len(),
        1,
        "expected exactly one census summary line; exit={:?}\nstdout:\n{}\nstderr:\n{}",
        r.code,
        r.stdout,
        r.stderr
    );
    let records: Vec<serde_json::Value> = lines[..at[0]]
        .iter()
        .filter(|l| l.starts_with('{'))
        .map(|l| {
            serde_json::from_str(l).unwrap_or_else(|e| panic!("record is not JSON ({e}): {l}"))
        })
        .collect();
    assert!(
        lines[at[0] + 1..].iter().all(|l| !l.starts_with('{')),
        "the summary line comes after every record:\n{}",
        r.stdout
    );
    let summary = lines[at[0]]
        .split_whitespace()
        .filter_map(|t| t.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    (records, summary)
}

/// Sorted (outpoint, amount) of every Bond UTXO owned by `bond_owner(pk)`.
fn bonds_of(data: &Path, pk: &PublicKey) -> Vec<(String, u64)> {
    let db = StateDb::open(&data.join("state_db")).expect("FIXTURE: open state_db");
    let mut v: Vec<(String, u64)> = db
        .get_utxos_by_pubkey(&bond_owner(pk))
        .into_iter()
        .filter(|(_, e)| e.output.output_type == OutputType::Bond)
        .map(|(op, e)| {
            (
                format!("{}:{}", op.tx_hash.to_hex(), op.index),
                e.output.amount,
            )
        })
        .collect();
    v.sort();
    v
}

fn entry(output: Output) -> UtxoEntry {
    UtxoEntry {
        output,
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    }
}

/// The fixture's producer made Exited with two seeded Bond UTXOs and one
/// Normal UTXO; a second registered key made Exited with no UTXO.
/// Returns (holder, bare).
fn seed_exited(data: &Path, unit: u64) -> (PublicKey, PublicKey) {
    let db = StateDb::open(&data.join("state_db")).expect("FIXTURE: open state_db");
    let mut ps = db.load_producer_set();
    let holder = ps.all_producers()[0].public_key;
    let bare = *KeyPair::generate().public_key();
    ps.register_genesis_producer(bare, 1, unit)
        .expect("FIXTURE: register a bare key");
    for pk in [holder, bare] {
        ps.get_by_pubkey_mut(&pk).expect("FIXTURE: producer").status = ProducerStatus::Exited;
    }
    db.write_producer_set(&ps)
        .expect("FIXTURE: write producer set");
    let owner = bond_owner(&holder);
    for (i, amount) in [(0u8, unit), (1, 2 * unit)] {
        let op = Outpoint::new(Hash::from_bytes([0xC0 + i; 32]), u32::from(i));
        db.insert_utxo(&op, &entry(Output::bond(amount, owner, u64::MAX, 7)))
            .expect("FIXTURE: seed Bond UTXO");
    }
    let normal = Outpoint::new(Hash::from_bytes([0xC9; 32]), 0);
    db.insert_utxo(&normal, &entry(Output::normal(5, owner)))
        .expect("FIXTURE: seed Normal UTXO");
    (holder, bare)
}

fn persisted(data: &Path) -> (ProducerSet, ChainState) {
    let db = StateDb::open(&data.join("state_db")).expect("FIXTURE: reopen state_db");
    let cs = db
        .get_chain_state()
        .expect("FIXTURE: persisted chain state");
    (db.load_producer_set(), cs)
}

// REQ-DEF-005 — Decision: a failure means the check on a stopped test-network data dir still replays with genesis time 0 and dies at height 1 instead of giving a verdict.
#[tokio::test]
async fn inc_i_262_m2_check_replays_a_testnet_store() {
    let st = testnet_store().await;
    let before = tree(&st.data);
    let r = run_node(
        &st.scratch_root,
        argv(
            &["--network", "testnet"],
            &st.data,
            &["--log-level", "error", "check-producer-set"],
        ),
    );
    assert_outcome(
        &r,
        "EQUAL",
        0,
        &[
            ("height", TESTNET_TIP.to_string()),
            (
                "producers",
                running_genesis(Network::Testnet)
                    .all_producers()
                    .len()
                    .to_string(),
            ),
            ("pending", "0".to_string()),
        ],
    );
    assert_clean(&st.data, &st.scratch_root, &before, "testnet check");
}

// REQ-DEF-005 — Decision: a failure means an operator who writes the flags after the subcommand gets a parser refusal (or the default data dir) instead of the check of the directory they named.
#[tokio::test]
async fn inc_i_262_m2_check_takes_flags_after_the_subcommand() {
    let fx = build().await;
    let before = tree(&fx.data);
    let want = [
        ("height", TIP.to_string()),
        ("producers", fx.producers.to_string()),
        ("pending", fx.pending.to_string()),
    ];
    let after = run_node(
        &fx.scratch_root,
        argv(
            &[
                "--log-level",
                "error",
                "check-producer-set",
                "--network",
                "devnet",
            ],
            &fx.data,
            &[],
        ),
    );
    assert_outcome(&after, "EQUAL", 0, &want);
    let ahead = run_node(
        &fx.scratch_root,
        argv(
            &["--network", "devnet"],
            &fx.data,
            &["--log-level", "error", "check-producer-set"],
        ),
    );
    assert_outcome(&ahead, "EQUAL", 0, &want);
    assert_clean(&fx.data, &fx.scratch_root, &before, "flags on both sides");
}

// REQ-DEF-005 — Decision: a failure means the replay node derives slots or rewards from parameters the running node of that network never uses, so check and recover rebuild a different chain.
#[tokio::test]
async fn inc_i_262_m2_replay_params_match_the_running_node() {
    let mut wrong = Vec::new();
    for net in [Network::Mainnet, Network::Testnet, Network::Devnet] {
        let tmp = TempDir::new().unwrap();
        let node = Node::new_for_replay(tmp.path().join("replay"), net)
            .await
            .expect("FIXTURE: Node::new_for_replay");
        let (got, want) = (
            format!("{:#?}", node.params),
            format!("{:#?}", running_params(net)),
        );
        let differs: Vec<String> = got
            .lines()
            .zip(want.lines())
            .filter(|(g, w)| g != w)
            .map(|(g, w)| format!("{} (running node: {})", g.trim(), w.trim()))
            .collect();
        let genesis = (
            node.params.genesis_time,
            running_spec(net).map_or(0, |spec| spec.genesis.timestamp),
        );
        if genesis.0 != genesis.1
            || !differs.is_empty()
            || got.lines().count() != want.lines().count()
        {
            wrong.push(format!(
                "{net:?}: genesis_time {} vs chainspec {}; differs {differs:?}",
                genesis.0, genesis.1
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "the replay node's params must equal the running node's (for_network + apply_chainspec of the chainspec it runs on; devnet without a data-dir chainspec keeps genesis time 0):\n{}",
        wrong.join("\n")
    );
}

// REQ-DEF-005 — Decision: a failure means the replay starts from a producer set the running node never had, so check and recover reject a real chain's attestation bitfields or schedule.
#[tokio::test]
async fn inc_i_262_m2_replay_genesis_producers_match_the_running_node() {
    let mut wrong = Vec::new();
    for net in [Network::Mainnet, Network::Testnet, Network::Devnet] {
        let tmp = TempDir::new().unwrap();
        let node = Node::new_for_replay(tmp.path().join("replay"), net)
            .await
            .expect("FIXTURE: Node::new_for_replay");
        let want = roster(&running_genesis(net));
        let got = roster(&*node.producer_set.read().await);
        if got != want {
            wrong.push(format!(
                "{net:?}: producer_set has {} producers, the running node seeds {}\n  replay: {got:?}\n  running: {want:?}",
                got.len(),
                want.len()
            ));
        }
        // Genesis producers are active at height 0 with weight >= 1, so the
        // running node's first scheduler list is every seeded key, sorted.
        let want_list: Vec<String> = want.iter().map(|(k, _, _)| k.clone()).collect();
        let got_list: Vec<String> = node
            .epoch_state
            .producer_list
            .iter()
            .map(|pk| pk.to_hex())
            .collect();
        if got_list != want_list {
            wrong.push(format!(
                "{net:?}: epoch_state.producer_list {got_list:?}, running node {want_list:?}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "the replay node must start from the genesis producer set a running node of its network seeds (chainspec genesis producers, else the network constants; devnet without a data-dir chainspec: none):\n{}",
        wrong.join("\n")
    );
}

// REQ-DEF-005 — Decision: a failure means an Exited producer that still holds bond collateral stays invisible to the operator, or the census reports a non-bond UTXO or an Exited key with no bond.
#[tokio::test]
async fn inc_i_262_m2_census_lists_exited_producers_holding_bonds() {
    let fx = build().await;
    let unit = Network::Devnet.bond_unit();
    let (holder, bare) = seed_exited(&fx.data, unit);
    let bonds = bonds_of(&fx.data, &holder);
    assert!(
        bonds.len() >= 2,
        "PRECONDITION: the holder owns the seeded bonds: {bonds:?}"
    );
    assert!(
        bonds_of(&fx.data, &bare).is_empty(),
        "PRECONDITION: the bare key owns no bond"
    );
    let total: u64 = bonds.iter().map(|(_, a)| a).sum();
    let before = tree(&fx.data);
    let r = run_node(&fx.scratch_root, census_args(&fx.data));
    let (records, summary) = census_output(&r);
    assert_eq!(r.code, Some(0), "census exit code; stderr:\n{}", r.stderr);
    assert_eq!(
        records.len(),
        1,
        "one record (the holder, not the bare key):\n{}",
        r.stdout
    );
    let rec = &records[0];
    assert_eq!(rec["key"], holder.to_hex().as_str(), "record key: {rec}");
    assert_eq!(rec["status"], "Exited", "record status: {rec}");
    assert_eq!(
        rec["bond_count"],
        bonds.len() as u64,
        "record bond_count: {rec}"
    );
    assert_eq!(rec["total"], total, "record total: {rec}");
    let mut listed: Vec<(String, u64)> = rec["bonds"]
        .as_array()
        .unwrap_or_else(|| panic!("record bonds is an array: {rec}"))
        .iter()
        .map(|b| {
            let op = b["outpoint"]
                .as_str()
                .unwrap_or_else(|| panic!("bond outpoint: {b}"));
            (
                op.to_string(),
                b["amount"]
                    .as_u64()
                    .unwrap_or_else(|| panic!("bond amount: {b}")),
            )
        })
        .collect();
    listed.sort();
    assert_eq!(
        listed, bonds,
        "record bonds are the holder's Bond UTXOs only: {rec}"
    );
    let want = [
        ("height", TIP.to_string()),
        (SUMMARY_KEY, "1".to_string()),
        ("bond_utxos", bonds.len().to_string()),
        ("total", total.to_string()),
    ];
    for (k, v) in want {
        assert_eq!(
            summary.get(k),
            Some(&v),
            "summary key {k}; stdout:\n{}",
            r.stdout
        );
    }
    assert_clean(&fx.data, &fx.scratch_root, &before, "census with records");
}

// REQ-DEF-005 — Decision: a failure means a clean chain reads as carrying stranded bonds, or the census writes into the data dir.
#[tokio::test]
async fn inc_i_262_m2_census_clean_chain_prints_no_records() {
    let fx = build().await;
    let before = tree(&fx.data);
    let r = run_node(&fx.scratch_root, census_args(&fx.data));
    let (records, summary) = census_output(&r);
    assert_eq!(r.code, Some(0), "census exit code; stderr:\n{}", r.stderr);
    assert!(
        records.is_empty(),
        "a clean chain prints no record:\n{}",
        r.stdout
    );
    for (k, v) in [
        ("height", TIP.to_string()),
        (SUMMARY_KEY, "0".into()),
        ("bond_utxos", "0".into()),
        ("total", "0".into()),
    ] {
        assert_eq!(
            summary.get(k),
            Some(&v),
            "summary key {k}; stdout:\n{}",
            r.stdout
        );
    }
    assert_clean(&fx.data, &fx.scratch_root, &before, "clean census");
}

// REQ-DEF-005 — Decision: a failure means the census reads a live node's open databases and reports records from a moving state.
#[tokio::test]
async fn inc_i_262_m2_census_refuses_a_locked_store() {
    let fx = build().await;
    seed_exited(&fx.data, Network::Devnet.bond_unit());
    let db = StateDb::open(&fx.data.join("state_db")).expect("FIXTURE: hold state_db");
    let bs = storage::BlockStore::open(&fx.data.join("blocks")).expect("FIXTURE: hold blocks");
    let r = run_node(&fx.scratch_root, census_args(&fx.data));
    drop((db, bs));
    assert_eq!(
        r.code,
        Some(1),
        "a locked store is refused with exit 1\nstdout:\n{}\nstderr:\n{}",
        r.stdout,
        r.stderr
    );
    assert!(
        r.stdout.lines().all(|l| !l.trim_start().starts_with('{')),
        "a refused census prints no record:\n{}",
        r.stdout
    );
    let said = format!("{}{}", r.stdout, r.stderr).to_lowercase();
    assert!(said.contains("lock"), "the refusal names the lock:\n{said}");
}
