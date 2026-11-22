//! AUDIT-RPC-ACCESS (2026-11-21) — fixture of `audit_rpc_access.rs`: the audit
//! table (every dispatched method and its access class), a throwaway `RpcServer` on
//! a free 127.0.0.1 port, and the probes that call it.
//!
//! OUTPUT CONTRACT: N/A — fixture file (no test logic; see `audit_rpc_access.rs`).
//! INPUT PARTITIONS: N/A — fixture file.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crypto::Hash;
use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::{signing_message_at, MAINTAINER_AUTH_VALID_BEFORE_UNSET};
use doli_core::network::Network;
use doli_core::{MaintainerSet, Transaction};
use mempool::{Mempool, MempoolPolicy};
use network::{SyncConfig, SyncManager};
use rpc::error::codes;
use rpc::{RpcContext, RpcServer, RpcServerConfig, TxBroadcastPermit};
use serde_json::{json, Value};
use storage::{BlockStore, ChainState, MaintainerState, ProducerSet, StateDb, UtxoSet};
use tempfile::TempDir;
use tokio::sync::RwLock;

use Access::{AdminMutating, AdminReadOnly, OpenMutating, OpenReadOnly, OperatorToken};

pub(super) const TOKEN: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
pub(super) const PUBLIC: &str = "203.0.113.9";
pub(super) const DISPATCH_SRC: &str = include_str!("../../src/methods/dispatch.rs");

/// Access class of one RPC method, from reading its handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Access {
    /// `ADMIN_METHODS` + `OPERATOR_TOKEN_METHODS`: bearer token from every address.
    OperatorToken,
    /// `ADMIN_METHODS`; changes node state, writes or deletes disk, or makes an
    /// outbound request. Token asked only from a non-trusted address.
    AdminMutating,
    /// `ADMIN_METHODS`; read-only (full-state disclosure or an expensive scan).
    AdminReadOnly,
    /// NOT in `ADMIN_METHODS`; changes the mempool or forwards to gossip.
    OpenMutating,
    /// NOT in `ADMIN_METHODS`; read-only.
    OpenReadOnly,
}

/// The audit table: every name in the dispatch `match` (dispatch.rs:15-82), its
/// class, and what the handler does. `audit_rpc_access_table_classifies_every_method`
/// fails when a method is added to dispatch or `ADMIN_METHODS` and not classified.
pub(super) const AUDIT_TABLE: &[(&str, Access, &str)] = &[
    (
        "forceReorgTo",
        OperatorToken,
        "arms a reorg across the finality marker (fork_escape.rs:23)",
    ),
    (
        "pauseProduction",
        AdminMutating,
        "halts block production through SyncManager::block_production (guardian.rs:19)",
    ),
    (
        "resumeProduction",
        AdminMutating,
        "clears an operator production halt (guardian.rs:39)",
    ),
    (
        "createCheckpoint",
        AdminMutating,
        "writes a RocksDB checkpoint of state_db + blocks under data_dir (guardian.rs:62)",
    ),
    (
        "pruneBlocks",
        AdminMutating,
        "deletes block bodies below tip - keep_last_n (pruning.rs:19)",
    ),
    (
        "backfillFromPeer",
        AdminMutating,
        "POSTs to a caller-supplied URL, writes the fetched blocks to the block store (backfill.rs:77)",
    ),
    (
        "enterRecoveryMode",
        AdminMutating,
        "sets recovery_mode: the node drops every inbound block and snap sync (guardian.rs:242)",
    ),
    (
        "exitRecoveryMode",
        AdminMutating,
        "clears recovery_mode in the middle of an operator recovery (guardian.rs:257)",
    ),
    (
        "bridgeFromArchive",
        AdminMutating,
        "deletes the chain commitment, imports archive blocks, force=true replaces blocks (guardian.rs:280)",
    ),
    (
        "repairArchiveFromPeer",
        AdminMutating,
        "POSTs to a caller-supplied URL, writes .block files to the archive dir (guardian.rs:414)",
    ),
    (
        "getUtxoDiff",
        AdminReadOnly,
        "reads and compares the UTXO set (stats.rs:115)",
    ),
    (
        "getStateSnapshot",
        AdminReadOnly,
        "returns the full serialized node state (snapshot.rs:25)",
    ),
    (
        "getStateRootDebug",
        AdminReadOnly,
        "returns the state root components (stats.rs:66)",
    ),
    (
        "verifyChainIntegrity",
        AdminReadOnly,
        "full block-store scan; stores the recomputed chain commitment only when none is stored (backfill.rs:501, 702-706)",
    ),
    (
        "sendTransaction",
        OpenMutating,
        "admits a transaction to the mempool and relays it to gossip (transaction.rs:175)",
    ),
    (
        "submitVote",
        OpenMutating,
        "forwards a producer veto vote to gossip and the updater (governance.rs:16)",
    ),
    (
        "submitMaintainerChange",
        OpenMutating,
        "builds a maintainer add/remove tx, admits it to the mempool, relays it (governance.rs:220)",
    ),
    ("getBlockByHash", OpenReadOnly, "reads the block store"),
    ("getBlockByHeight", OpenReadOnly, "reads the block store"),
    ("getBlockRaw", OpenReadOnly, "reads the block store"),
    ("getBlockData", OpenReadOnly, "reads the block store"),
    ("getTransaction", OpenReadOnly, "reads mempool + block store"),
    ("getNftByTokenId", OpenReadOnly, "reads the UTXO set"),
    ("getBalance", OpenReadOnly, "reads the UTXO set + mempool"),
    ("getUtxos", OpenReadOnly, "reads the UTXO set"),
    ("getMempoolInfo", OpenReadOnly, "reads the mempool"),
    ("getNetworkInfo", OpenReadOnly, "reads peer count + sync status"),
    ("getPeerInfo", OpenReadOnly, "reads the peer list (peer addresses)"),
    ("getChainInfo", OpenReadOnly, "reads chain state"),
    ("getProducer", OpenReadOnly, "reads the producer set"),
    ("getProducers", OpenReadOnly, "reads the producer set"),
    ("getHistory", OpenReadOnly, "scans the block store for an address"),
    ("getUpdateStatus", OpenReadOnly, "reads the update service status"),
    ("getMaintainerSet", OpenReadOnly, "reads the maintainer set"),
    ("getNodeInfo", OpenReadOnly, "reads version + network"),
    ("getEpochInfo", OpenReadOnly, "reads chain state"),
    ("getNetworkParams", OpenReadOnly, "reads network parameters"),
    ("getBondDetails", OpenReadOnly, "reads the producer set + UTXO set"),
    ("backfillStatus", OpenReadOnly, "reads backfill counters"),
    ("getChainStats", OpenReadOnly, "reads chain state + UTXO set"),
    ("getMempoolTransactions", OpenReadOnly, "reads the mempool"),
    ("getSlotSchedule", OpenReadOnly, "computes the slot schedule"),
    ("getProducerSchedule", OpenReadOnly, "computes the producer schedule"),
    ("getAttestationStats", OpenReadOnly, "reads the block store"),
    ("getPoolInfo", OpenReadOnly, "reads the UTXO set"),
    ("getPoolList", OpenReadOnly, "reads the UTXO set"),
    ("getPoolPrice", OpenReadOnly, "reads the UTXO set"),
    ("getSwapQuote", OpenReadOnly, "reads the UTXO set"),
    ("getDefiHealthMetric", OpenReadOnly, "reads the UTXO set + producer set"),
    ("getOraclePrice", OpenReadOnly, "reads the UTXO set"),
    ("getOracleAttestations", OpenReadOnly, "reads the block store"),
    ("getOracleStatus", OpenReadOnly, "reads the UTXO set + producer set"),
    ("getStorageInfo", OpenReadOnly, "reads block-store bounds"),
    ("getGuardianStatus", OpenReadOnly, "reads halt + recovery flags, lists checkpoint names"),
    ("getForkChoiceVersion", OpenReadOnly, "reads a network parameter"),
];

/// One throwaway node: a real `RpcServer` on a free 127.0.0.1 port, with the state
/// handles the tests read back.
pub(super) struct Node {
    pub(super) url: String,
    pub(super) client: reqwest::Client,
    pub(super) sync: Arc<RwLock<SyncManager>>,
    pub(super) recovery: Arc<AtomicBool>,
    pub(super) mempool: Arc<RwLock<Mempool>>,
    pub(super) tx_relays: Arc<AtomicUsize>,
    pub(super) vote_relays: Arc<AtomicUsize>,
    pub(super) checkpoints: PathBuf,
    pub(super) server: tokio::task::JoinHandle<()>,
    pub(super) _tempdir: TempDir,
}

impl Drop for Node {
    fn drop(&mut self) {
        self.server.abort();
    }
}

pub(super) fn keypair(seed: u8) -> crypto::KeyPair {
    crypto::KeyPair::from_seed([seed; 32])
}

/// `RpcServer::run` binds `config.listen_addr` itself and does not report the bound
/// port, so port 0 cannot be passed straight in. The port is taken from a
/// `127.0.0.1:0` probe socket and released for the server; a lost race is retried.
pub(super) async fn start(admin_token: Option<&str>, trusted_proxies: &[&str]) -> Node {
    for _attempt in 0..5 {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .expect("probe socket")
            .port();
        if (8500..=8517).contains(&port) || (28500..=28550).contains(&port) {
            continue;
        }

        let tempdir = TempDir::new().expect("tempdir");
        let data_dir = tempdir.path().join("data");
        let blocks_dir = tempdir.path().join("blocks");
        let state_dir = tempdir.path().join("state_db");
        for dir in [&data_dir, &blocks_dir, &state_dir] {
            std::fs::create_dir_all(dir).expect("fixture dir");
        }

        let network = Network::Devnet;
        let params = ConsensusParams::for_network(network);
        let mempool = Arc::new(RwLock::new(Mempool::new(
            MempoolPolicy::default(),
            params.clone(),
            network,
        )));
        let sync = Arc::new(RwLock::new(SyncManager::new(
            SyncConfig::default(),
            Hash::ZERO,
        )));
        let recovery = Arc::new(AtomicBool::new(false));
        let tx_relays = Arc::new(AtomicUsize::new(0));
        let vote_relays = Arc::new(AtomicUsize::new(0));
        let (tx_sink, vote_sink) = (tx_relays.clone(), vote_relays.clone());

        let mut ctx = RpcContext::new_for_network(
            Arc::new(RwLock::new(ChainState::new(Hash::ZERO))),
            Arc::new(BlockStore::open(&blocks_dir).expect("blockstore")),
            Arc::new(RwLock::new(UtxoSet::new())),
            mempool.clone(),
            params,
            network,
        )
        .with_sync_manager(sync.clone())
        .with_recovery_mode(recovery.clone())
        .with_state_db(Arc::new(StateDb::open(&state_dir).expect("state_db")))
        .with_data_dir(data_dir.clone())
        .with_producer_set(Arc::new(RwLock::new(ProducerSet::new())))
        .with_broadcast(move || {
            let sink = tx_sink.clone();
            Some(Box::new(move |_tx: Transaction| {
                sink.fetch_add(1, Ordering::SeqCst);
            }) as TxBroadcastPermit)
        })
        .with_broadcast_vote(move |_vote| {
            vote_sink.fetch_add(1, Ordering::SeqCst);
            true
        });
        // Maintainer set = fixture keys 1..=4 (threshold 3).
        let members = (1..=4).map(|i| *keypair(i).public_key()).collect();
        ctx.maintainer_state = Some(Arc::new(RwLock::new(MaintainerState {
            set: MaintainerSet::with_members(members, 0),
            ..Default::default()
        })));

        let config = RpcServerConfig {
            listen_addr: SocketAddr::from(([127, 0, 0, 1], port)),
            enable_cors: false,
            allowed_origins: vec![],
            admin_token: admin_token.map(str::to_string),
            trusted_proxies: trusted_proxies
                .iter()
                .map(|p| p.parse().expect("proxy ip"))
                .collect(),
        };
        let (ws_tx, _ws_rx) = rpc::ws::broadcast_channel();
        let server = RpcServer::new(config, ctx, ws_tx).spawn();

        let mut up = false;
        for _ in 0..300 {
            if server.is_finished() {
                break;
            }
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                up = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // A failed bind ends the server task: then the port answered for another
        // process, and this attempt is dropped.
        tokio::time::sleep(Duration::from_millis(30)).await;
        if !up || server.is_finished() {
            server.abort();
            continue;
        }

        return Node {
            url: format!("http://127.0.0.1:{port}"),
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("http client"),
            sync,
            recovery,
            mempool,
            tx_relays,
            vote_relays,
            checkpoints: data_dir.join("checkpoints"),
            server,
            _tempdir: tempdir,
        };
    }
    panic!("no RPC server came up on a free 127.0.0.1 port in 5 attempts");
}

pub(super) async fn post_raw(
    node: &Node,
    path: &str,
    body: String,
    headers: &[(&str, &str)],
) -> Value {
    let mut req = node.client.post(format!("{}{path}", node.url)).body(body);
    if !headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        req = req.header("content-type", "application/json");
    }
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let resp = req.send().await.expect("the test RPC server answers");
    let status = resp.status().as_u16();
    let text = resp.text().await.expect("response body");
    serde_json::from_str(&text).unwrap_or_else(|_| json!({ "http_status": status, "body": text }))
}

pub(super) async fn call(
    node: &Node,
    method: &str,
    params: Value,
    headers: &[(&str, &str)],
) -> Value {
    let body = json!({ "jsonrpc": "2.0", "method": method, "params": params, "id": 1 });
    post_raw(node, "/", body.to_string(), headers).await
}

pub(super) fn error_code(resp: &Value) -> Option<i64> {
    resp.pointer("/error/code").and_then(Value::as_i64)
}

/// Refused by the admin gate (`RpcError::unauthorized`, -32008).
pub(super) fn refused(resp: &Value) -> bool {
    error_code(resp) == Some(i64::from(codes::UNAUTHORIZED))
}

pub(super) fn summary(resp: &Value) -> String {
    match error_code(resp) {
        Some(code) => format!(
            "error {code} \"{}\"",
            resp.pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
        None => {
            let mut text = resp.get("result").unwrap_or(resp).to_string();
            if text.len() > 110 {
                text = format!("{}...", text.chars().take(110).collect::<String>());
            }
            format!("result {text}")
        }
    }
}

/// Params that reach the handler and stop before any outbound request: `rpc_url`
/// is omitted on purpose, so `backfillFromPeer` (backfill.rs:78-79) and
/// `repairArchiveFromPeer` (guardian.rs:416-420) return INVALID_PARAMS first.
pub(super) fn harmless_params(method: &str) -> Value {
    match method {
        "backfillFromPeer" | "repairArchiveFromPeer" => json!({}),
        _ => Value::Null,
    }
}

/// Puts the node in the state the method is about to change.
pub(super) async fn arm(node: &Node, method: &str) {
    match method {
        "pauseProduction" => node.sync.write().await.unblock_production(),
        "resumeProduction" => node.sync.write().await.block_production("operator halt"),
        "enterRecoveryMode" => node.recovery.store(false, Ordering::SeqCst),
        "exitRecoveryMode" => node.recovery.store(true, Ordering::SeqCst),
        "createCheckpoint" => {
            let _ = std::fs::remove_dir_all(&node.checkpoints);
        }
        _ => {}
    }
}

/// The node-state change read back after the call. `None`: no change is visible
/// (the gate refused, or the handler stopped at a precondition the empty fixture
/// chain does not meet — no blocks, no archive dir, no `rpc_url`).
pub(super) async fn observed_effect(node: &Node, method: &str) -> Option<&'static str> {
    let halted = node.sync.read().await.production_block_reason().is_some();
    let recovering = node.recovery.load(Ordering::SeqCst);
    match method {
        "pauseProduction" => halted.then_some("STATE CHANGED: block production is halted"),
        "resumeProduction" => {
            (!halted).then_some("STATE CHANGED: the operator's production halt is cleared")
        }
        "enterRecoveryMode" => {
            recovering.then_some("STATE CHANGED: recovery mode on, inbound blocks are dropped")
        }
        "exitRecoveryMode" => (!recovering).then_some("STATE CHANGED: recovery mode cleared"),
        "createCheckpoint" => std::fs::read_dir(&node.checkpoints)
            .is_ok_and(|mut entries| entries.next().is_some())
            .then_some("DISK WRITTEN: a checkpoint directory exists"),
        _ => None,
    }
}

/// Calls every `AdminMutating` method as `caller`. Returns one line for each call
/// that the gate did NOT refuse, or that changed node state.
pub(super) async fn exposed_admin_methods(
    node: &Node,
    caller: &str,
    headers: &[(&str, &str)],
) -> Vec<String> {
    let mut exposed = Vec::new();
    for &(method, class, what) in AUDIT_TABLE {
        if class != AdminMutating {
            continue;
        }
        arm(node, method).await;
        let resp = call(node, method, harmless_params(method), headers).await;
        let effect = observed_effect(node, method).await;
        if refused(&resp) && effect.is_none() {
            continue;
        }
        exposed.push(format!(
            "  {method} <- {caller}: {} | {} | handler: {what}",
            summary(&resp),
            effect.unwrap_or("handler reached, stopped at an empty-fixture precondition"),
        ));
    }
    exposed
}

pub(super) fn dispatch_methods() -> Vec<String> {
    let mut names = Vec::new();
    for line in DISPATCH_SRC.lines().map(str::trim_start) {
        if !line.starts_with('"') {
            continue;
        }
        if let Some((pattern, _)) = line.split_once("=>") {
            names.extend(
                pattern
                    .split('|')
                    .map(|alt| alt.trim().trim_matches('"').to_string()),
            );
        }
    }
    names
}

/// `n` signatures by the fixture keys `first..first+n` over the bytes that apply
/// verifies at the next height (tip 0 -> 1) for a maintainer add of `target`.
pub(super) fn maintainer_sigs(first: u8, n: u8, target: u8) -> Vec<Value> {
    let network = Network::Devnet;
    let genesis = ConsensusParams::for_network(network).genesis_hash;
    let msg = signing_message_at(
        genesis.as_bytes(),
        true,
        keypair(target).public_key(),
        MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        1,
        network.params().inc_i_176_auth_binding_activation_height,
    );
    (first..first + n)
        .map(|seed| {
            let kp = keypair(seed);
            json!({
                "pubkey": kp.public_key().to_hex(),
                "signature": crypto::signature::sign(&msg, kp.private_key()).to_hex(),
            })
        })
        .collect()
}
