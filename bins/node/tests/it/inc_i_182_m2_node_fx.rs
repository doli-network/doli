//! INC-I-182 M2 node fixtures: a Run B rig whose producer P is listed with
//! bonds (no delegations), owns `bond_utxos` Bond UTXOs and one Normal coin.
//! P's withdrawal in each authorization form, path drivers, child runner.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
// covers: exit unbonding_tx producer_tx producer_ledger withdrawal_economics withdrawal_holdings slash_withdrawal slash_admission tx_processing pool assembly validation_checks producer_auth unbonding transaction

#![allow(dead_code)]

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::{Input, OutputType, Transaction};
use storage::{Outpoint, ProducerSet, ProducerStatus};
use tracing_subscriber::layer::SubscriberExt;

use super::inc_i_182_m1_common::{Logs, Verdict};
use super::inc_i_182_m1_node_fx::reference_sign;
use super::inc_i_255_m1_common::{admit, full_block_with};
use super::inc_i_255_m2_common::{
    addr, outpoints, rig_with, schedulable, Keys, Rig, ACCUSED_BONDS, ACCUSED_TAG, COIN, COIN_TAG,
    DELEGATOR_BONDS, HONEST_BONDS,
};
use super::inc_i_255_m4_common::{err_text, resident};

pub const CODE: &str = "ERRTX-PAUTH001";
pub const METRIC: &str = "PAUTH_SELF_EXIT_NODE_PATHS=";
pub const GATE: u64 = 2;
pub const FEE: u64 = 1;
pub const LOCK: u64 = 10_000;
pub const P_BONDS: u32 = ACCUSED_BONDS;
const CHILD: &str = "INC_I_182_M2_CHILD";
const CHILD_ENV: [(&str, &str); 3] = [
    ("DOLI_INC_I_182_PRODUCER_AUTH_ACTIVATION_HEIGHT", "2"),
    ("DOLI_WITHDRAWAL_HOLDINGS_GATE_ACTIVATION_HEIGHT", "0"),
    ("DOLI_GENESIS_BLOCKS", "0"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Signed,
    Unsigned,
    WrongKey,
}

/// (bond_count, withdrawal_pending_count, status) of P.
pub type Ledger = Option<(u32, u32, ProducerStatus)>;

pub struct Sx {
    pub r: Rig,
}

/// P = the rig's accused key; ledger honest 2, P 3, delegator 2.
pub async fn sx(bond_utxos: u32) -> Sx {
    let r = rig_with(&Keys::new(), bond_utxos).await;
    let unit = r.unit();
    let mut ps = ProducerSet::new();
    for (k, n) in [
        (&r.keys.honest, HONEST_BONDS),
        (&r.keys.accused, P_BONDS),
        (&r.keys.delegator, DELEGATOR_BONDS),
    ] {
        ps.register_genesis_producer(*k.public_key(), n, unit)
            .expect("FIXTURE: ledger");
    }
    r.node.state_db.write_producer_set(&ps).expect("persist ps");
    *r.node.producer_set.write().await = ps;
    r.node.refresh_mempool_producer_snapshot(0).await;
    let s = Sx { r };
    let owned =
        s.r.node
            .utxo_set
            .read()
            .await
            .get_bond_entries(&addr(&s.pk()))
            .len();
    assert_eq!(owned as u32, bond_utxos, "FIXTURE: P's live Bond UTXOs");
    s
}

impl Sx {
    pub fn p(&self) -> &KeyPair {
        &self.r.keys.accused
    }

    pub fn pk(&self) -> PublicKey {
        *self.p().public_key()
    }

    pub fn coin(&self) -> Outpoint {
        outpoints(COIN_TAG, 1)[0]
    }

    pub fn genesis(&self) -> Hash {
        self.r.node.params.genesis_hash
    }

    /// RequestWithdrawal by P spending `inputs` (P signs every input); one
    /// Unbonding payout to P's own address; auth tail per `form`.
    pub fn withdrawal(
        &self,
        inputs: &[Outpoint],
        bonds: u32,
        payout: u64,
        form: Form,
    ) -> Transaction {
        let ins = inputs
            .iter()
            .map(|o| Input::new(o.tx_hash, o.index))
            .collect();
        let pk = self.pk();
        let mut tx = Transaction::new_request_withdrawal(ins, pk, bonds, addr(&pk), payout);
        tx.outputs[0].output_type = OutputType::Unbonding;
        tx.outputs[0].lock_until = LOCK;
        for i in 0..tx.inputs.len() {
            tx.inputs[i].public_key = Some(pk);
            let msg = tx.signing_message_for_input(i);
            tx.inputs[i].signature = crypto::signature::sign_hash(&msg, self.p().private_key());
        }
        match form {
            Form::Signed => reference_sign(&mut tx, &self.genesis(), self.p()),
            Form::Unsigned => {}
            Form::WrongKey => reference_sign(&mut tx, &self.genesis(), &self.r.keys.delegator),
        }
        tx
    }

    /// Zero Bond inputs, full allowance declared; P's coin pays fee and payout.
    pub fn self_exit(&self, form: Form) -> Transaction {
        self.withdrawal(&[self.coin()], P_BONDS, COIN - FEE, form)
    }

    /// Every Bond UTXO P owns, full allowance declared, signed by P.
    pub fn full_withdrawal(&self) -> Transaction {
        let gross = self.r.unit() * u64::from(P_BONDS);
        self.withdrawal(
            &outpoints(ACCUSED_TAG, P_BONDS),
            P_BONDS,
            gross - FEE,
            Form::Signed,
        )
    }

    pub async fn ledger(&self) -> Ledger {
        let ps = self.r.node.producer_set.read().await;
        ps.get_by_pubkey(&self.pk())
            .map(|i| (i.bond_count, i.withdrawal_pending_count, i.status))
    }

    pub async fn queued(&self) -> usize {
        self.r
            .node
            .producer_set
            .read()
            .await
            .pending_updates_for(&self.pk())
            .len()
    }

    pub async fn schedulable_next(&self) -> bool {
        let h = self.r.best().await.0 + 1;
        schedulable(&self.r.node, &self.pk(), h).await
    }

    /// Light-mode apply of a block at the next height holding `tx`.
    pub async fn apply_with(&mut self, tx: &Transaction) -> Verdict {
        let h = self.r.best().await.0 + 1;
        let block = self.r.next_block(vec![tx.clone()], 0).await;
        let res = self.r.apply(block).await;
        Verdict {
            accepted: res.is_ok(),
            text: format!(
                "apply at {h}: {}; P (bonds, pending, status) {:?}, queued {}",
                err_text(&res),
                self.ledger().await,
                self.queued().await
            ),
        }
    }

    /// Admission with the node's own tip.
    pub async fn admit(&self, tx: &Transaction) -> Verdict {
        let h = self.r.best().await.0;
        let r = admit(&self.r.node, tx).await;
        Verdict {
            accepted: r.is_ok(),
            text: format!("mempool at tip {h}: {r:?}"),
        }
    }

    /// The builder for the next height; reports selection and its log lines for `tx`.
    pub async fn build_next(&mut self, tx: &Transaction) -> Verdict {
        let h = self.r.best().await.0 + 1;
        let was_resident = resident(&self.r.node, tx).await;
        let logs = Logs::default();
        let _g =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
        let selected = full_block_with(&mut self.r.node, &[]).await.1;
        let accepted = selected.contains(&tx.hash());
        Verdict {
            accepted,
            text: format!(
                "builder at {h}: resident {was_resident}, selected {accepted}; log {:?}",
                logs.mentioning(&tx.hash().to_string())
            ),
        }
    }
}

pub fn in_child() -> bool {
    std::env::var_os(CHILD).is_some()
}

/// Re-exec `test` alone with the gate overrides; forward its metric line and
/// re-raise its panic message.
pub fn run_child(test: &str) {
    let mut cmd = std::process::Command::new(std::env::current_exe().expect("current exe"));
    cmd.args([test, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD, "1");
    for (k, v) in CHILD_ENV {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn child test process");
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
    let why: Vec<&str> = stderr
        .lines()
        .skip_while(|l| !l.contains("panicked at"))
        .skip(1)
        .take_while(|l| !l.starts_with("note: run with"))
        .collect();
    if why.is_empty() {
        panic!("child {test} failed or did not run:\n{stdout}\n{stderr}");
    }
    panic!("{}", why.join("\n"));
}
