//! INC-I-182 M1 fixtures: a devnet world where producer P is listed with bonds
//! but owns no Bond UTXO, and a third party T whose own coin pays for a
//! RequestWithdrawal that declares P's full allowance. Each path runs in a
//! child process: holdings gate at 0, producer-auth gate at `PAUTH_GATE`.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use crypto::{KeyPair, PublicKey};
use doli_core::transaction::{Input, OutputType, Transaction};
use doli_core::validation::ValidationMode;
use doli_node::node::Node;
use storage::{Outpoint, ProducerSet};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

use super::inc_i_255_m1_common::{
    admit, full_block_with, genesis_bond_outpoint, tip, world, World,
};
use super::inc_i_255_m2_common::addr;
use super::inc_i_255_m4_common::{admit_at, err_text, resident};
use super::inc_i_255_m6_common::{block_with, seed_coin};

pub const CODE: &str = "ERRTX-PAUTH001";
pub const METRIC: &str = "PAUTH_FORCED_EXIT_ACCEPTED_PATHS=";
pub const PAUTH_GATE: u64 = 2;
pub const PAUTH_GATE_ENV: &str = "DOLI_INC_I_182_PRODUCER_AUTH_ACTIVATION_HEIGHT";
pub const HOLDINGS_GATE_ENV: &str = "DOLI_WITHDRAWAL_HOLDINGS_GATE_ACTIVATION_HEIGHT";
pub const P_LEDGER_BONDS: u32 = 2;
pub const COIN: u64 = 500_000;
pub const FEE: u64 = 1;
pub const LOCK: u64 = 10_000;
const T_COIN_TAG: u8 = 0x82;
const CHILD_ENV: &str = "INC_I_182_M1_CHILD";

/// Node key = `w.reporter`; P = `w.accused`; T = `third`.
pub struct Fx {
    pub w: World,
    pub third: KeyPair,
    pub tx: Transaction,
}

#[derive(Debug)]
pub struct Verdict {
    pub accepted: bool,
    pub text: String,
}

impl Verdict {
    pub fn coded(&self) -> bool {
        self.text.contains(CODE)
    }
}

/// T spends only its own Normal coin; the payout is the gated Unbonding output to P.
pub fn forced_exit(third: &KeyPair, coin: Outpoint, p: &PublicKey, bonds: u32) -> Transaction {
    let mut input = Input::new(coin.tx_hash, coin.index);
    input.public_key = Some(*third.public_key());
    let mut tx = Transaction::new_request_withdrawal(vec![input], *p, bonds, addr(p), COIN - FEE);
    tx.outputs[0].output_type = OutputType::Unbonding;
    tx.outputs[0].lock_until = LOCK;
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, third.private_key());
    tx
}

pub async fn fixture() -> Fx {
    let w = world().await;
    let unit = w.node.config.network.bond_unit();
    let p = w.accused_pk();
    let mut ps = ProducerSet::new();
    ps.register_genesis_producer(*w.reporter.public_key(), 1, unit)
        .expect("FIXTURE: node producer ledger");
    ps.register_genesis_producer(p, P_LEDGER_BONDS, unit)
        .expect("FIXTURE: P ledger");
    w.node.state_db.write_producer_set(&ps).expect("persist ps");
    *w.node.producer_set.write().await = ps;
    let third = KeyPair::generate();
    let coin = seed_coin(&w.node, third.public_key(), T_COIN_TAG, COIN).await;
    w.node
        .utxo_set
        .write()
        .await
        .remove(&genesis_bond_outpoint(&p))
        .expect("FIXTURE: drop P's genesis bond");
    let owned = w
        .node
        .utxo_set
        .read()
        .await
        .get_bond_entries(&addr(&p))
        .len();
    assert_eq!(owned, 0, "FIXTURE: P owns no Bond UTXO at its own address");
    let tx = forced_exit(&third, coin, &p, P_LEDGER_BONDS);
    Fx { w, third, tx }
}

/// Real builder blocks, applied in Full mode, until the tip is `height`.
pub async fn advance_to(node: &mut Node, height: u64) {
    while tip(node).await.0 < height {
        let (b, _) = full_block_with(node, &[]).await;
        let r = node.apply_block(b, ValidationMode::Full).await;
        assert!(r.is_ok(), "FIXTURE: advance block; got {}", err_text(&r));
    }
}

/// Admission with the node's own tip, at or above the producer-auth gate.
pub async fn mempool_path() -> Verdict {
    let mut f = fixture().await;
    advance_to(&mut f.w.node, PAUTH_GATE).await;
    let r = admit(&f.w.node, &f.tx).await;
    Verdict {
        accepted: r.is_ok(),
        text: format!("admission at tip {PAUTH_GATE}: {r:?}"),
    }
}

pub struct BuilderSeen {
    pub resident: bool,
    pub admission: String,
    pub v: Verdict,
}

/// Admitted with the tip judged below the gate, then the builder runs at the gate.
pub async fn builder_path() -> BuilderSeen {
    let logs = Logs::default();
    let _g = tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let mut f = fixture().await;
    advance_to(&mut f.w.node, PAUTH_GATE - 1).await;
    let h = tip(&f.w.node).await.0;
    let admission = format!("{:?}", admit_at(&f.w.node, &f.tx, h - 1).await);
    let resident = resident(&f.w.node, &f.tx).await;
    let selected = full_block_with(&mut f.w.node, &[]).await.1;
    let accepted = selected.contains(&f.tx.hash());
    let said = logs.mentioning(&f.tx.hash().to_string());
    BuilderSeen {
        resident,
        admission,
        v: Verdict {
            accepted,
            text: format!("selected {accepted} at height {PAUTH_GATE}; builder log {said:?}"),
        },
    }
}

/// A builder block for height `PAUTH_GATE` carrying the tx, applied in Full mode.
pub async fn apply_path() -> Verdict {
    let mut f = fixture().await;
    advance_to(&mut f.w.node, PAUTH_GATE - 1).await;
    let block = block_with(&mut f.w.node, std::slice::from_ref(&f.tx)).await;
    let r = f.w.node.apply_block(block, ValidationMode::Full).await;
    let p = f.w.accused_pk();
    let ps = f.w.node.producer_set.read().await;
    let after = ps
        .get_by_pubkey(&p)
        .map(|i| (i.bond_count, i.withdrawal_pending_count, i.status));
    let queued = ps.pending_updates_for(&p).len();
    Verdict {
        accepted: r.is_ok(),
        text: format!(
            "block at height {PAUTH_GATE}: {}; P (bonds, withdrawal_pending, status) {after:?}, queued updates {queued}",
            err_text(&r)
        ),
    }
}

pub fn in_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}

/// Re-exec `test` alone with both gate overrides; forward its metric line and
/// re-raise the first line of its panic message.
pub fn run_in_child(test: &str) {
    let out = std::process::Command::new(std::env::current_exe().expect("current exe"))
        .args([test, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(HOLDINGS_GATE_ENV, "0")
        .env(PAUTH_GATE_ENV, PAUTH_GATE.to_string())
        .output()
        .expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    for line in stdout
        .lines()
        .filter_map(|l| l.find(METRIC).map(|i| &l[i..]))
    {
        println!("{line}");
    }
    if out.status.success() && stdout.contains("1 passed") {
        return;
    }
    let why = stderr
        .lines()
        .skip_while(|l| !l.contains("panicked at"))
        .nth(1)
        .map(str::to_string)
        .unwrap_or_else(|| format!("child {test} failed or did not run:\n{stdout}\n{stderr}"));
    panic!("{why}");
}

#[derive(Clone, Default)]
pub struct Logs(Arc<Mutex<Vec<String>>>);

impl Logs {
    pub fn mentioning(&self, needle: &str) -> Vec<String> {
        let all = self.0.lock().unwrap();
        all.iter().filter(|l| l.contains(needle)).cloned().collect()
    }
}

struct Fields(String);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={v:?} ", f.name()));
    }
}

impl<S: tracing::Subscriber> Layer<S> for Logs {
    fn on_event(&self, e: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut f = Fields(String::new());
        e.record(&mut f);
        let line = format!("{} {}", e.metadata().level(), f.0);
        self.0.lock().unwrap().push(line);
    }
}
