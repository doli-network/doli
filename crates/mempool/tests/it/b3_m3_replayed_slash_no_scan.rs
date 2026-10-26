//! B3 M3 — a replayed copy of a mined SlashProducer (valid evidence, accused
//! already slash-queued or Slashed) is refused, at admission and at
//! revalidation, without the owner-UTXO lookup of the accused.
//!
// covers: slash_admission pool
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//                  fn Mempool::revalidate(&mut self, &UtxoSet, u64, u32)
//   O1 return: replay, accused queued -> Err(InvalidSlash "already has a pending slash") (unchanged)
//      | replay, accused Slashed -> Err(InvalidSlash "already slashed") (unchanged)
//      | resident same id -> Err(AlreadyExists) | accused Active (first, or after the queued Slash is gone) -> Ok
//   O2 pool entries: revalidate evicts a resident slash whose accused became queued / Slashed (unchanged)
//   O3 RocksDB per-thread perf context (production backend, StateDb): VISITS = point gets + iterator
//      seeks/nexts on the calling thread during the call; read-only for UtxoSet and the live set
// PATHS: P-QUEUED replay, accused slash-queued | P-SLASHED replay, accused Slashed
//        | P-BASE valid slash naming an unregistered key (no owner lookup today)
//        | P-RESIDENT same tx id resident | P-ACTIVE first valid slash (control)
//        | P-ROLLBACK queued Slash removed, same bytes again | P-REVAL resident slash, accused settled
// INPUT PARTITIONS:
//   owner UTXOs of the accused: 1_000 | 50_000; settled state: queued | Slashed
// MATRIX:
//   O1,O3 x {P-QUEUED, P-BASE} x {1k, 50k}  -> b3_m3_replayed_slash_queued_accused_no_owner_scan
//   O1,O3 x {P-SLASHED, P-BASE} x {1k, 50k} -> b3_m3_replayed_slash_slashed_accused_no_owner_scan
//   O1,O3 x P-RESIDENT x 1k                 -> b3_m3_resident_replay_same_id_no_owner_scan
//   O1,O3 x P-ACTIVE x 1k                   -> b3_m3_first_valid_slash_active_accused_still_scans
//   O1,O3 x P-ROLLBACK x 1k                 -> b3_m3_readmitted_after_rollback_when_no_longer_queued
//   O2,O3 x P-REVAL x {queued, Slashed} x {1k, 50k} -> b3_m3_revalidate_settled_accused_no_owner_scan

use crypto::{KeyPair, PublicKey};
use mempool::MempoolError;
use rocksdb::perf::{set_perf_stats, PerfContext, PerfMetric, PerfStatsLevel};
use storage::PendingProducerUpdate;

use super::b3_m2_slash_scan_bound::db_with;
use super::inc_i_256_m1_fixture::{addr, policy, rig, slash, Rig, SLOT, TIP};

const SMALL: u32 = 1_000;
const LARGE: u32 = 50_000;
const PAIR: (u8, u8) = (0x11, 0x22);

#[derive(Clone, Copy, Debug)]
enum Settled {
    Queued,
    Slashed,
}

impl Settled {
    fn refusal(self) -> &'static str {
        match self {
            Settled::Queued => "slash target already has a pending slash",
            Settled::Slashed => "slash target is already slashed",
        }
    }
}

/// VISITS of `f` on this thread.
fn counted<R>(f: impl FnOnce() -> R) -> (u64, R) {
    set_perf_stats(PerfStatsLevel::EnableCount);
    let mut perf = PerfContext::default();
    perf.reset();
    let out = f();
    let visits = [
        PerfMetric::GetFromMemtableCount,
        PerfMetric::SeekOnMemtableCount,
        PerfMetric::NextOnMemtableCount,
    ]
    .iter()
    .map(|m| perf.metric(*m))
    .sum();
    set_perf_stats(PerfStatsLevel::Disable);
    (visits, out)
}

/// The live set in the state the including block left: Slash queued, or Slashed after the boundary.
fn settle(r: &Rig, pk: &PublicKey, s: Settled) {
    let mut ps = r.producers.try_write().expect("FIXTURE: live set unlocked");
    match s {
        Settled::Queued => ps.queue_update(PendingProducerUpdate::Slash {
            pubkey: *pk,
            height: TIP,
        }),
        Settled::Slashed => {
            ps.slash_producer(pk, TIP).expect("FIXTURE: slash producer");
        }
    }
}

fn shows(r: &Result<impl std::fmt::Debug, MempoolError>, needle: &str) -> bool {
    matches!(r, Err(e) if format!("{e:?}").contains("InvalidSlash") && format!("{e:?}").contains(needle))
}

/// Per size: (n, VISITS of the replay against the settled accused, VISITS of the unregistered-key baseline).
fn replay_visits(s: Settled, seed: u8, tag: &str) -> Vec<(u32, u64, u64)> {
    let accused = KeyPair::from_seed([seed; 32]);
    let stranger = KeyPair::from_seed([seed.wrapping_add(1); 32]);
    [SMALL, LARGE]
        .into_iter()
        .map(|n| {
            let db = db_with(&accused, n, tag);
            let mut r = rig(policy(100, 1_000), &[&accused]);
            let (base, b) = counted(|| {
                r.pool
                    .add_transaction(slash(&stranger, PAIR), &db.utxo, TIP, SLOT)
            });
            assert!(
                shows(&b, "not a registered producer"),
                "CONTROL: a slash naming an unregistered key is refused: {b:?}"
            );
            settle(&r, accused.public_key(), s);
            let (named, res) = counted(|| {
                r.pool
                    .add_transaction(slash(&accused, PAIR), &db.utxo, TIP, SLOT)
            });
            assert!(
                shows(&res, s.refusal()),
                "the replay against a {s:?} accused keeps today's refusal `{}`: {res:?}",
                s.refusal()
            );
            (n, named, base)
        })
        .collect()
}

fn assert_flat_at_baseline(seen: &[(u32, u64, u64)], what: &str) {
    for &(n, named, base) in seen {
        assert!(
            named == base,
            "{what} with {n} owner UTXOs must cost what an unregistered-key slash costs (no owner lookup): visits {named} vs baseline {base}; all {seen:?}"
        );
    }
    assert_eq!(
        seen[0].1, seen[1].1,
        "{what} work must not grow from {SMALL} to {LARGE} owner UTXOs: {seen:?}"
    );
}

// REQ-B3-004 — Decision: a failure means any peer can replay a mined slash and make every node walk the accused's whole owner index under the mempool write lock until the boundary.
#[test]
fn b3_m3_replayed_slash_queued_accused_no_owner_scan() {
    let seen = replay_visits(Settled::Queued, 0xB1, "m3-queued");
    eprintln!("METRIC replayed_slash_visits_1k={}", seen[0].1);
    eprintln!("METRIC replayed_slash_visits_50k={}", seen[1].1);
    eprintln!("METRIC replayed_slash_baseline={}", seen[0].2);
    assert_flat_at_baseline(&seen, "a replayed slash against a slash-queued accused");
}

// REQ-B3-004 — Decision: a failure means a replayed slash keeps costing an owner-index walk under the lock after the boundary made the accused Slashed.
#[test]
fn b3_m3_replayed_slash_slashed_accused_no_owner_scan() {
    let seen = replay_visits(Settled::Slashed, 0xB3, "m3-slashed");
    eprintln!("METRIC replayed_slashed_visits_1k={}", seen[0].1);
    eprintln!("METRIC replayed_slashed_visits_50k={}", seen[1].1);
    assert_flat_at_baseline(&seen, "a replayed slash against a Slashed accused");
}

// REQ-B3-004 — Decision: a failure means a resident same-id replay reaches the owner lookup instead of the AlreadyExists refusal.
#[test]
fn b3_m3_resident_replay_same_id_no_owner_scan() {
    let accused = KeyPair::from_seed([0xB5; 32]);
    let db = db_with(&accused, SMALL, "m3-resident");
    let mut r = rig(policy(100, 1_000), &[&accused]);
    let tx = slash(&accused, PAIR);
    let first = r.pool.add_transaction(tx.clone(), &db.utxo, TIP, SLOT);
    assert!(
        first.is_ok(),
        "PRECONDITION: first slash admitted: {first:?}"
    );
    let (visits, res) = counted(|| r.pool.add_transaction(tx, &db.utxo, TIP, SLOT));
    assert!(
        matches!(res, Err(MempoolError::AlreadyExists)),
        "a resident same-id replay is AlreadyExists: {res:?}"
    );
    assert_eq!(visits, 0, "the AlreadyExists replay reads no UTXO state");
}

// REQ-B3-004 — Decision: a failure means the early refusal also cut the owner lookup of a first valid slash, so its locked-collateral fact is lost.
#[test]
fn b3_m3_first_valid_slash_active_accused_still_scans() {
    let accused = KeyPair::from_seed([0xB6; 32]);
    let stranger = KeyPair::from_seed([0xB7; 32]);
    let db = db_with(&accused, SMALL, "m3-active");
    let (one_lookup, held) = counted(|| {
        db.utxo
            .get_by_pubkey_hash(&addr(accused.public_key()))
            .len()
    });
    assert_eq!(held, SMALL as usize, "FIXTURE: owner index size");
    let mut r = rig(policy(100, 1_000), &[&accused]);
    let (base, _) = counted(|| {
        r.pool
            .add_transaction(slash(&stranger, PAIR), &db.utxo, TIP, SLOT)
    });
    let (named, res) = counted(|| {
        r.pool
            .add_transaction(slash(&accused, PAIR), &db.utxo, TIP, SLOT)
    });
    assert!(
        res.is_ok(),
        "a first valid slash for an Active accused is admitted: {res:?}"
    );
    assert!(named > 0, "the first valid slash reads the owner index");
    assert_eq!(
        named,
        base + one_lookup,
        "the first valid slash does exactly one owner lookup ({one_lookup} visits) over the baseline {base}"
    );
}

// REQ-B3-004 — Decision: a failure means the early refusal outlives the queued Slash, so a slash whose including block was rolled back can never be admitted again.
#[test]
fn b3_m3_readmitted_after_rollback_when_no_longer_queued() {
    let accused = KeyPair::from_seed([0xB8; 32]);
    let db = db_with(&accused, SMALL, "m3-rollback");
    let mut r = rig(policy(100, 1_000), &[&accused]);
    let before = r.producers.try_read().expect("FIXTURE: live set").clone();
    settle(&r, accused.public_key(), Settled::Queued);
    let tx = slash(&accused, PAIR);
    let refused = r.pool.add_transaction(tx.clone(), &db.utxo, TIP, SLOT);
    assert!(
        shows(&refused, Settled::Queued.refusal()),
        "PRECONDITION: the replay is refused while the Slash is queued: {refused:?}"
    );
    *r.producers.try_write().expect("FIXTURE: live set") = before;
    let (visits, res) = counted(|| r.pool.add_transaction(tx, &db.utxo, TIP, SLOT));
    assert!(
        res.is_ok(),
        "the same slash bytes are admitted once the queued Slash is gone: {res:?}"
    );
    assert!(visits > 0, "the readmitted slash builds its owner context");
}

// REQ-B3-004 — Decision: a failure means every revalidate after a slash is mined walks the accused's owner index under the mempool write lock for a verdict already fixed by the producer-set state.
#[test]
fn b3_m3_revalidate_settled_accused_no_owner_scan() {
    let mut fails = Vec::new();
    for (s, seed, tag) in [
        (Settled::Queued, 0xC1, "m3-reval-queued"),
        (Settled::Slashed, 0xC3, "m3-reval-slashed"),
    ] {
        let accused = KeyPair::from_seed([seed; 32]);
        let mut seen = Vec::new();
        for n in [SMALL, LARGE] {
            let db = db_with(&accused, n, tag);
            let mut r = rig(policy(100, 1_000), &[&accused]);
            let tx = slash(&accused, PAIR);
            let hash = tx.hash();
            let first = r.pool.add_transaction(tx, &db.utxo, TIP, SLOT);
            assert!(first.is_ok(), "PRECONDITION: slash resident: {first:?}");
            settle(&r, accused.public_key(), s);
            let (visits, ()) = counted(|| r.pool.revalidate(&db.utxo, TIP, SLOT));
            assert!(
                !r.pool.contains(&hash),
                "revalidate evicts the resident slash of a {s:?} accused"
            );
            seen.push((n, visits));
        }
        let label = format!("{s:?}").to_lowercase();
        eprintln!("METRIC revalidate_{label}_visits_1k={}", seen[0].1);
        eprintln!("METRIC revalidate_{label}_visits_50k={}", seen[1].1);
        if seen[0].1 != seen[1].1 {
            fails.push((s, seen));
        }
    }
    assert!(
        fails.is_empty(),
        "revalidate work for a settled accused must not grow from {SMALL} to {LARGE} owner UTXOs: {fails:?}"
    );
}
