// covers: slash_state slash_window slash_evidence slash_admission pool unbonding entry error assembly tx_processing validation_checks mod lib block transaction types set_registration set_lifecycle defaults network_params
//! INC-I-256 M2 fixtures: a double-production pair at a chosen slot, chains
//! with chosen slots, the verdict of one slash on the tip from mempool
//! admission, builder selection, apply and replay, and a child-process runner.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.

#![allow(dead_code)]

use crypto::KeyPair;
use doli_core::transaction::{OutputType, SlashingEvidence, Transaction};
use doli_core::validation::{bond_owner, ValidationMode};
use doli_core::Block;
use storage::UtxoSet;

use super::inc_i_255_m1_common::{
    apply_slash_msg, canonical, chain_genesis, evidence_header, full_block_with, mempool_slash_msg,
};
use super::inc_i_255_m2_common::Rig;
use super::inc_i_255_m4_common::admit_at;

pub const EXPIRED: &str = "slash evidence expired";
pub const PRE_TENURE: &str = "predates the accused's current registration";
pub const ALREADY_SLASHED: &str = "slash target is already slashed";
pub const PENDING: &str = "slash target already has a pending slash";
pub const PERIOD_ENV: &str = "DOLI_UNBONDING_PERIOD";
pub const GATE_ENV: &str = "DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT";
pub const GENESIS_ENV: &str = "DOLI_GENESIS_BLOCKS";
pub const PATHS: [&str; 4] = ["mempool", "builder", "apply", "replay"];
const CHILD_ENV: &str = "INC_I_256_M2_CHILD";
const FORCE_HEIGHT_SKEW: u64 = 1_000;
const CLEARED_ENV: [&str; 5] = [
    PERIOD_ENV,
    GATE_ENV,
    GENESIS_ENV,
    "DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT",
    "DOLI_INC_I_182_PRODUCER_AUTH_ACTIVATION_HEIGHT",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Accepted,
    Refused(String),
}

impl Verdict {
    pub fn accepted(&self) -> bool {
        matches!(self, Verdict::Accepted)
    }

    pub fn names(&self, rule: &str) -> bool {
        matches!(self, Verdict::Refused(m) if m.contains(rule))
    }
}

/// Verdicts in `PATHS` order; `resident` = the slash was in the pool when the
/// builder ran; `builder_slot` = slot of the block the builder assembled.
#[derive(Debug)]
pub struct Probe {
    pub v: [Verdict; 4],
    pub resident: bool,
    pub builder_slot: u32,
    pub parent: (u64, u32),
}

/// Two conflicting signed headers by `kp` at `slot`, as one canonical slash.
pub fn slash_at(kp: &KeyPair, slot: u32) -> Transaction {
    let g = chain_genesis();
    let a = evidence_header(kp, g, slot, 0xA1, 0);
    let b = evidence_header(kp, g, slot, 0xB2, 0);
    canonical(*kp.public_key(), a, b)
}

pub fn offence_slot(tx: &Transaction) -> u32 {
    match &tx.slash_data().expect("FIXTURE: a slash").evidence {
        SlashingEvidence::DoubleProduction { block_header_1, .. } => block_header_1.slot,
        other => panic!("FIXTURE: double production expected, got {other:?}"),
    }
}

/// (height the next block takes, judged slot = tip slot + 1).
pub async fn judged(r: &Rig) -> (u64, u32) {
    let (h, s, _) = r.best().await;
    (h + 1, s.saturating_add(1))
}

/// Slots from the offence to the judged slot of the next block.
pub async fn age_at_next(r: &Rig, tx: &Transaction) -> u64 {
    let (_, j) = judged(r).await;
    u64::from(j.saturating_sub(offence_slot(tx)))
}

/// One empty block on the tip at `slot`.
pub async fn block_at_slot(r: &mut Rig, slot: u32) {
    let (h, s, _) = r.best().await;
    assert!(
        slot > s,
        "FIXTURE: slot {slot} must exceed the tip slot {s}"
    );
    let block = r.next_block(Vec::new(), slot - s - 1).await;
    r.apply(block)
        .await
        .unwrap_or_else(|e| panic!("FIXTURE: block {} at slot {slot}: {e:#}", h + 1));
}

/// One block per slot up to `height` (slot == height).
pub async fn dense_to(r: &mut Rig, height: u64) {
    r.advance_to(height).await;
    let (h, s, _) = r.best().await;
    assert_eq!((h, u64::from(s)), (height, height), "FIXTURE: dense chain");
}

async fn on_tip(r: &mut Rig, block: Block, mode: ValidationMode) -> Verdict {
    let before = r.best().await;
    let res = r.node.apply_block(block, mode).await;
    match &res {
        Ok(()) => {
            r.rollback().await;
            Verdict::Accepted
        }
        Err(e) => {
            assert_eq!(
                r.best().await,
                before,
                "{mode:?}: a refused block moved the tip"
            );
            Verdict::Refused(apply_slash_msg(&res).unwrap_or_else(|| format!("{e:#}")))
        }
    }
}

/// Normal-lane admission judged as at the offence slot, far above the tip, on
/// a copy of the accused's owner entries with every Bond re-stamped at slot 0,
/// so the builder judges a slash the tip-level admission refused.
async fn force_resident(r: &Rig, tx: &Transaction, tip: u64) -> bool {
    let accused = tx.slash_data().expect("FIXTURE: a slash").producer_pubkey;
    let mut scratch = UtxoSet::new();
    for (at, mut e) in r
        .node
        .utxo_set
        .read()
        .await
        .get_by_pubkey_hash(&bond_owner(&accused))
    {
        if e.output.output_type == OutputType::Bond {
            e.output.extra_data = 0u32.to_le_bytes().to_vec();
        }
        scratch.insert(at, e).expect("FIXTURE: scratch owner entry");
    }
    let mut pool = r.node.mempool.write().await;
    pool.add_transaction(
        tx.clone(),
        &scratch,
        tip + FORCE_HEIGHT_SKEW,
        offence_slot(tx),
    )
    .is_ok()
}

/// The same `tx` on the same parent: mempool admission (node arguments),
/// builder selection (real builder at the wall slot), apply (Light) and replay
/// (Replay) of one block at parent slot + 1 + `bump`. The rig ends on its tip.
pub async fn four_paths(r: &mut Rig, tx: &Transaction, bump: u32) -> Probe {
    let before = r.best().await;
    let res = admit_at(&r.node, tx, before.0).await;
    let mempool = match &res {
        Ok(()) => Verdict::Accepted,
        Err(_) => Verdict::Refused(mempool_slash_msg(&res).unwrap_or_else(|| format!("{res:?}"))),
    };
    let resident = res.is_ok() || force_resident(r, tx, before.0).await;
    let (built, selected) = full_block_with(&mut r.node, &[]).await;
    r.node.mempool.write().await.remove_transaction(&tx.hash());
    let builder = if selected.contains(&tx.hash()) {
        Verdict::Accepted
    } else {
        Verdict::Refused(format!("builder did not select it (resident: {resident})"))
    };
    let block = r.next_block(vec![tx.clone()], bump).await;
    let apply = on_tip(r, block.clone(), ValidationMode::Light).await;
    let replay = on_tip(r, block, ValidationMode::Replay).await;
    assert_eq!(
        r.best().await,
        before,
        "FIXTURE: the probe leaves the rig on its tip"
    );
    Probe {
        v: [mempool, builder, apply, replay],
        resident,
        builder_slot: built.header.slot,
        parent: (before.0, before.1),
    }
}

pub fn print_invalid_accepted(case: &str, p: &Probe) {
    for (path, v) in PATHS.iter().zip(&p.v) {
        println!(
            "B2M2_METRIC INVALID_EVIDENCE_ACCEPTED={} path={path} case={case}",
            u8::from(v.accepted())
        );
    }
}

/// Paths whose verdict differs from apply's (the builder by accept/refuse only).
pub fn splits(p: &Probe) -> usize {
    let apply = &p.v[2];
    p.v.iter()
        .enumerate()
        .filter(|(i, v)| match i {
            2 => false,
            1 => v.accepted() != apply.accepted(),
            _ => *v != apply,
        })
        .count()
}

pub fn assert_accepted_everywhere(case: &str, p: &Probe) {
    for (path, v) in PATHS.iter().zip(&p.v) {
        assert!(
            v.accepted(),
            "{case}: {path} must accept; got {v:?} (parent {:?})",
            p.parent
        );
    }
}

/// Mempool, apply and replay refuse with one identical message naming `rule`;
/// the builder does not select the slash.
pub fn assert_refused_everywhere(case: &str, p: &Probe, rule: &str) {
    for (path, v) in PATHS.iter().zip(&p.v) {
        if *path == "builder" {
            assert!(
                !v.accepted(),
                "{case}: the builder selected the slash (parent {:?})",
                p.parent
            );
            continue;
        }
        assert!(
            v.names(rule),
            "{case}: {path} must refuse with {rule:?}; got {v:?} (parent {:?})",
            p.parent
        );
    }
    assert!(
        p.v[0] == p.v[2] && p.v[2] == p.v[3],
        "{case}: mempool, apply and replay must give one message; got {:?}",
        p.v
    );
}

pub fn in_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}

/// Re-exec `test` alone with `env`; forwards metric lines; re-raises the child's panic.
pub fn child(test: &str, env: &[(&str, String)]) {
    let mut cmd = std::process::Command::new(std::env::current_exe().expect("current exe"));
    cmd.args([test, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1");
    for k in CLEARED_ENV {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    for line in stdout
        .lines()
        .filter_map(|l| l.find("B2M2_METRIC").map(|i| &l[i..]))
    {
        println!("{line}");
    }
    if out.status.success() && stdout.contains("1 passed") {
        return;
    }
    let why: Vec<&str> = stderr
        .lines()
        .chain(stdout.lines())
        .skip_while(|l| !l.contains("panicked at"))
        .skip(1)
        .take_while(|l| !l.starts_with("note: run with"))
        .collect();
    if why.is_empty() {
        panic!("child {test} failed or did not run:\n{stdout}\n{stderr}");
    }
    panic!("{}", why.join("\n"));
}
