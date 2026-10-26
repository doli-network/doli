//! B3 M2 — admitting a SlashProducer whose evidence does not verify, or any
//! slash below the gate, does no work that grows with the accused owner's UTXO
//! count; verified evidence at/above the gate still reads the owner's UTXOs.
//!
// covers: slash_admission pool slash_state
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//   O3 return: junk evidence -> Err(InvalidSlash) (unchanged); valid evidence -> Ok (unchanged)
//   O5 RocksDB per-thread perf context (production backend, StateDb): VISITS = point gets + iterator
//      seeks/nexts on the calling thread during the call; read-only for UtxoSet and the live set
// PATHS: P-JUNK at/above gate, accused registered | P-BASE junk naming an unregistered key (no owner lookup today)
//        | P-BELOW child, gate env far above the tip, junk and valid | P-VALID at/above gate (control)
// INPUT PARTITIONS:
//   owner UTXOs of the accused: 1_000 | 50_000; evidence: junk (header 2 signature flipped) | valid
// MATRIX:
//   O5,O3 x {P-JUNK, P-BASE} x {1k, 50k} -> b3_m2_junk_slash_lock_work_is_size_independent
//   O5 x P-BELOW x {junk, valid} x {1k, 50k} -> b3_m2_below_gate_slash_lock_work_is_size_independent
//   O5,O3 x P-VALID x {1k, 50k} -> b3_m2_valid_slash_still_builds_context

use std::sync::Arc;

use crypto::{Hash, KeyPair};
use doli_core::transaction::{SlashData, SlashingEvidence, Transaction};
use mempool::MempoolError;
use rocksdb::perf::{set_perf_stats, PerfContext, PerfMetric, PerfStatsLevel};
use storage::{Outpoint, StateDb, UtxoEntry, UtxoSet};

use super::inc_i_256_m1_fixture::{addr, policy, rig, slash, spawned_child, SLOT, TIP};

const SMALL: u32 = 1_000;
const LARGE: u32 = 50_000;
const BELOW_GATE: u64 = 1_000_000;
const GATE_ENV: &str = "DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "B3_M2_SCAN_BOUND_CHILD";

pub(super) struct Db {
    pub(super) utxo: UtxoSet,
    dir: std::path::PathBuf,
}

impl Drop for Db {
    fn drop(&mut self) {
        self.utxo = UtxoSet::new();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// RocksDB-backed set where `owner_of` holds its genesis Bond UTXO (slot 0)
/// and `n - 1` Normal UTXOs.
pub(super) fn db_with(owner_of: &KeyPair, n: u32, tag: &str) -> Db {
    let dir = std::env::temp_dir().join(format!("b3-m2-scan-{tag}-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let sdb = StateDb::open(&dir).expect("FIXTURE: open StateDb");
    let mut utxo = UtxoSet::from_state_db(Arc::new(sdb));
    let owner = addr(owner_of.public_key());
    for i in 0..n {
        let mut h = [0u8; 32];
        h[..4].copy_from_slice(&i.to_le_bytes());
        let output = match i {
            0 => doli_core::transaction::Output::bond(1, owner, u64::MAX, 0),
            _ => doli_core::transaction::Output::normal(1, owner),
        };
        let entry = UtxoEntry {
            output,
            height: 0,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo.insert(Outpoint::new(Hash::from_bytes(h), 0), entry)
            .expect("FIXTURE: seed owner UTXO");
    }
    assert_eq!(
        utxo.get_by_pubkey_hash(&owner).len(),
        n as usize,
        "FIXTURE: owner index holds n entries"
    );
    Db { utxo, dir }
}

/// Valid evidence with header 2's producer signature flipped.
fn junk(kp: &KeyPair) -> Transaction {
    let data = slash(kp, (0x11, 0x22))
        .slash_data()
        .expect("FIXTURE: slash data");
    let SlashingEvidence::DoubleProduction {
        block_header_1,
        mut block_header_2,
    } = data.evidence
    else {
        panic!("FIXTURE: double production evidence");
    };
    block_header_2.vdf_proof.pi[0] ^= 0xFF;
    Transaction::new_slash_producer(SlashData::double_production(
        data.producer_pubkey,
        block_header_1,
        block_header_2,
    ))
}

/// Admit `tx` into a fresh pool whose live set registers `accused`; VISITS counted on this thread.
fn admit_counted(accused: &KeyPair, tx: &Transaction, db: &Db) -> (u64, Result<(), MempoolError>) {
    let mut r = rig(policy(100, 1_000), &[accused]);
    set_perf_stats(PerfStatsLevel::EnableCount);
    let mut perf = PerfContext::default();
    perf.reset();
    let res = r
        .pool
        .add_transaction(tx.clone(), &db.utxo, TIP, SLOT)
        .map(|_| ());
    let visits = [
        PerfMetric::GetFromMemtableCount,
        PerfMetric::SeekOnMemtableCount,
        PerfMetric::NextOnMemtableCount,
    ]
    .iter()
    .map(|m| perf.metric(*m))
    .sum();
    set_perf_stats(PerfStatsLevel::Disable);
    (visits, res)
}

fn is_invalid_slash(r: &Result<(), MempoolError>) -> bool {
    matches!(r, Err(e) if format!("{e:?}").contains("InvalidSlash"))
}

/// VISITS of junk against the registered accused and of junk naming a stranger, per size.
fn junk_visits(sizes: [u32; 2]) -> Vec<(u32, u64, u64)> {
    let accused = KeyPair::from_seed([0xA7; 32]);
    let stranger = KeyPair::from_seed([0xA8; 32]);
    sizes
        .into_iter()
        .map(|n| {
            let db = db_with(&accused, n, "junk");
            let (named, r1) = admit_counted(&accused, &junk(&accused), &db);
            let (base, r2) = admit_counted(&accused, &junk(&stranger), &db);
            assert!(
                r1.is_err() && r2.is_err(),
                "junk evidence is refused at admission: {r1:?} / {r2:?}"
            );
            (n, named, base)
        })
        .collect()
}

// REQ-B3-004 — Decision: a failure means a peer can make every node walk the accused's whole owner index under the mempool write lock with evidence that never verifies, so admission cost grows with the victim's UTXO count.
#[test]
fn b3_m2_junk_slash_lock_work_is_size_independent() {
    let seen = junk_visits([SMALL, LARGE]);
    for &(n, named, base) in &seen {
        assert!(
            named == base,
            "junk evidence naming a registered accused with {n} owner UTXOs must cost what junk naming an unregistered key costs (no owner lookup): visits {named} vs baseline {base}; all {seen:?}"
        );
    }
    assert_eq!(
        seen[0].1, seen[1].1,
        "junk-evidence admission work must not grow from {SMALL} to {LARGE} owner UTXOs: {seen:?}"
    );
}

// REQ-B3-004 — Decision: a failure means a slash below the slashing gate still pays the owner lookup under the mempool lock, although no state verdict depends on it there.
#[test]
fn b3_m2_below_gate_slash_lock_work_is_size_independent() {
    if spawned_child(
        "b3_m2_slash_scan_bound::b3_m2_below_gate_slash_lock_work_is_size_independent",
        CHILD_ENV,
        &[(GATE_ENV, BELOW_GATE)],
    ) {
        return;
    }
    assert_eq!(
        doli_core::network::Network::Devnet
            .params()
            .effective_slashing_activation_height(),
        BELOW_GATE,
        "PRECONDITION: {GATE_ENV} moves the devnet slashing gate"
    );
    let accused = KeyPair::from_seed([0xA9; 32]);
    let stranger = KeyPair::from_seed([0xAA; 32]);
    let mut seen = Vec::new();
    for n in [SMALL, LARGE] {
        let db = db_with(&accused, n, "below");
        let (base, _) = admit_counted(&accused, &junk(&stranger), &db);
        for (form, tx) in [
            ("junk", junk(&accused)),
            ("valid", slash(&accused, (0x11, 0x22))),
        ] {
            let (named, res) = admit_counted(&accused, &tx, &db);
            seen.push((n, form, named, base, format!("{res:?}")));
            assert!(
                named == base,
                "below the gate a {form} slash naming an accused with {n} owner UTXOs must do no owner lookup: visits {named} vs baseline {base}; {seen:?}"
            );
        }
    }
}

// REQ-B3-004 — Decision: a failure means the reorder dropped the state context for verified evidence, so a valid slash loses its locked-collateral fact or its admission verdict changes.
#[test]
fn b3_m2_valid_slash_still_builds_context() {
    let accused = KeyPair::from_seed([0xAB; 32]);
    let stranger = KeyPair::from_seed([0xAC; 32]);
    let mut seen = Vec::new();
    for n in [SMALL, LARGE] {
        let db = db_with(&accused, n, "valid");
        let (named, res) = admit_counted(&accused, &slash(&accused, (0x11, 0x22)), &db);
        let (base, junk_res) = admit_counted(&accused, &junk(&stranger), &db);
        assert!(
            res.is_ok(),
            "valid evidence stays admissible with {n} owner UTXOs: {res:?}"
        );
        assert!(
            is_invalid_slash(&junk_res),
            "CONTROL: junk is an InvalidSlash: {junk_res:?}"
        );
        seen.push((n, named, base));
        assert!(
            named >= base + u64::from(n),
            "valid evidence must still read the accused's {n} owner UTXOs (context built): visits {named}, baseline {base}"
        );
    }
    assert!(
        seen[1].1 > seen[0].1,
        "valid-evidence lookup scales with the owner index: {seen:?}"
    );
}
