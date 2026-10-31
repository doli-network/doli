//! QA-639-003 scenarios: one per mechanism, run at the current tip with the
//! expected side of the gate. The test binary fixes the gate per child process.
//!
//! OUTPUT CONTRACT: N/A — fixture file (each test's contract names the outputs
//! these functions assert). INPUT PARTITIONS: N/A.

use crypto::Hash;
use doli_core::Block;

use super::*;

/// Our A: three blocks by keys[1]; the gossip door decides at this tip.
pub const FC_TIP: u64 = F + 3;
/// Our A1, A2 by keys[1]; the hold decides on a child of this tip.
pub const HOLD_TIP: u64 = F + 2;
/// Our A1 by keys[3] at F+1; X lands at F+2 and the drain ranks there.
pub const DRAIN_TIP: u64 = F + 2;
/// Our A1 by keys[3]; the walk is seeded at this tip.
pub const WALK_TIP: u64 = F + 1;
/// Our A1..A3 by keys[1]; finality of A1 is reached at this applied tip.
pub const FIN_TIP: u64 = F + 3;
/// Recorded weight far above any producer weight.
pub const HEAVY: u64 = 1_000_000;

fn last_slot(blocks: &[Block]) -> u32 {
    blocks.last().expect("fixture: non-empty").header.slot
}

// ---------------------------------------------------------------- fork choice

/// Our A (three blocks, one producer) applied; B (two blocks, two producers) on F.
pub async fn fork_choice_rig() -> (Rig, Vec<Block>) {
    let r = rig(&[1, 1, 1]).await;
    let b = chain_by(&r, r.f_hash, F + 1, &[2, 3], last_slot(&r.ours));
    (r, b)
}

/// Gossips B (two blocks on our block at the fork height) to a node on `r.ours`
/// at tip `t`. On: B2 is the tip (bond 2 > 1). Off: our top is kept and B2 is
/// never applied (our blocks above the fork > 2).
pub async fn fork_choice_decide(r: &mut Rig, b: &[Block], t: u64, gate: Gate, label: &str) {
    let fork_h = (r
        .node
        .block_store
        .get_height_by_hash(&b[0].header.prev_hash))
    .unwrap()
    .expect("fixture: B forks from a canonical block");
    let a = r.ours[(fork_h - F) as usize..].to_vec();
    let top = r.ours.last().unwrap().clone();
    assert_on(r, &top, t, &format!("{label} precondition: on our top")).await;
    assert_rules_disagree(r, &a, b, t, label).await;
    for blk in b {
        gossip(r, blk).await;
    }
    settle(r, &b[1].hash(), 5).await;
    let n = a.len();
    match gate {
        Gate::On => {
            let l = format!("{label}: ON — B (2 producers) must beat our {n} blocks by 1 producer");
            assert_on(r, &b[1], fork_h + 2, &l).await;
        }
        Gate::Off => {
            let l = format!("{label}: OFF — our {n} blocks must beat B's 2 by block count");
            assert_on(r, &top, t, &l).await;
            assert!(!stored(r, &b[1]), "{label}: OFF — B2 was applied");
        }
    }
}

// ------------------------------------------------------------------- finality

pub async fn finalized(r: &Rig) -> Option<u64> {
    r.node.sync_manager.read().await.last_finalized_height()
}

/// (height, hash) of the checkpoint persisted in state_db.
pub fn persisted_cp(r: &Rig) -> Option<(u64, Hash)> {
    (r.node.state_db.get_finality_checkpoint()).map(|c| (c.height, c.block_hash))
}

/// The real finalize path: 67 % weight on our block at `h`, applied tip >= h + 2.
pub async fn finalize(r: &Rig, h: u64) {
    let hash = r.ours[(h - F - 1) as usize].hash();
    (r.node.sync_manager.write().await).add_attestation_weight(
        &hash,
        *r.keys[1].public_key(),
        1_000_000,
    );
    assert_eq!(
        finalized(r).await,
        Some(h),
        "fixture: h={h} must really be finalized, or every finality assertion is vacuous"
    );
}

/// Appends `n` blocks by keys[1] to our chain (raw apply).
pub async fn extend(r: &mut Rig, n: usize) {
    let top = r.ours.last().unwrap().clone();
    let h = F + 1 + r.ours.len() as u64;
    let more = chain_by(r, top.hash(), h, &vec![1; n], top.header.slot);
    for b in &more {
        apply(r, b).await;
    }
    r.ours.extend(more);
}

/// A1 finalized at applied tip FIN_TIP, then a restart on the same data dir.
pub async fn finality(gate: Gate) {
    let r = rig(&[1, 1, 1]).await;
    assert_decision_tip(&r, FIN_TIP, "FIN").await;
    assert_eq!(finalized(&r).await, None, "fixture: nothing finalized yet");
    finalize(&r, F + 1).await;
    let written = persisted_cp(&r);
    let want = (F + 1, r.ours[0].hash());
    let r = reopen(r).await;
    assert_eq!(tip(&r).await.1, FIN_TIP, "reopen restores the tip");
    match gate {
        Gate::On => {
            assert_eq!(
                written,
                Some(want),
                "FIN: ON — the checkpoint must be written"
            );
            assert_eq!(finalized(&r).await, Some(F + 1), "FIN: ON — restored");
        }
        Gate::Off => {
            assert_eq!(
                written, None,
                "FIN: OFF — a checkpoint was written below the gate"
            );
            assert_eq!(
                finalized(&r).await,
                None,
                "FIN: OFF — finality survived a restart"
            );
        }
    }
}

// ----------------------------------------------------------------------- hold

/// On the tip T (height t): D = child by keys[1], applied then rolled back
/// (removed); E1 on D by keys[2] (cached); S1 on T by keys[3], cached with
/// weight HEAVY. Gossips D. On: released (bond D,E1 = 2 > S1 = 1), D canonical
/// at t+1. Off: held (recorded HEAVY > D's 1), tip stays T. Returns (D, S1).
pub async fn hold_at(r: &mut Rig, gate: Gate, label: &str) -> (Block, Block) {
    let (t_hash, t, t_slot) = tip(r).await;
    let d = chain_by(r, t_hash, t + 1, &[1], t_slot).remove(0);
    apply(r, &d).await;
    rollback_depth_one(r).await;
    assert_decision_tip(r, t, label).await;
    assert!(
        removed(r, &d.hash()).await,
        "{label} precondition: D removed"
    );
    let e1 = chain_by(r, d.hash(), t + 2, &[2], d.header.slot).remove(0);
    let s1 = chain_by(r, t_hash, t + 1, &[3], e1.header.slot).remove(0);
    cache_weighed(r, &s1, t + 1, HEAVY).await;
    gossip(r, &e1).await;
    assert!(
        cached(r, &e1.hash()).await && !removed(r, &e1.hash()).await,
        "{label} precondition: E1 is a cached child of D"
    );
    let ours = [d.clone(), e1.clone()];
    let (_, our_bond) = units(r, &ours, t + 1).await;
    let (s_w, s_bond) = units(r, std::slice::from_ref(&s1), t + 1).await;
    assert!(
        our_bond > s_bond && HEAVY > s_w,
        "{label} precondition: bond {our_bond} > {s_bond}; recorded {HEAVY} > {s_w}"
    );
    gossip(r, &d).await;
    match gate {
        Gate::On => assert_eq!(
            canonical(r, t + 1),
            Some(d.hash()),
            "{label}: ON — the hold must release D (bond {our_bond} > S1 {s_bond}), \
             not keep it by S1's recorded weight"
        ),
        Gate::Off => {
            assert_eq!(
                canonical(r, t + 1),
                None,
                "{label}: OFF — the hold must keep D out (S1 recorded {HEAVY} > 1)"
            );
            assert_eq!(tip(r).await.0, t_hash, "{label}: OFF — tip stays T");
        }
    }
    (d, s1)
}

// ---------------------------------------------------------------------- drain

/// X = child of the tip by keys[1]; S = [k2, k1] on X; L = [k1 x4] on X.
pub async fn drain_set(r: &Rig) -> (Block, Vec<Block>, Vec<Block>) {
    let (t_hash, t, t_slot) = tip(r).await;
    let x = chain_by(r, t_hash, t + 1, &[1], t_slot).remove(0);
    let s = chain_by(r, x.hash(), t + 2, &[2, 1], x.header.slot);
    let l = chain_by(r, x.hash(), t + 2, &[1, 1, 1, 1], last_slot(&s));
    (x, s, l)
}

/// Gossips L and S (cached orphans).
pub async fn cache_orphans(r: &mut Rig, s: &[Block], l: &[Block], label: &str) {
    for b in l.iter().chain(s) {
        gossip(r, b).await;
        assert!(
            cached(r, &b.hash()).await && !stored(r, b),
            "{label} precondition: {:.8} is a cached orphan",
            b.hash()
        );
    }
}

/// Gossips L, S, then X: the drain ranks two cached children at X's height
/// t+1. On: S1 first, tip S2 at t+3. Off: L1 first, tip L4 at t+5.
pub async fn drain_at(r: &mut Rig, gate: Gate, label: &str) {
    let t = tip(r).await.1;
    let (x, s, l) = drain_set(r).await;
    cache_orphans(r, &s, &l, label).await;
    assert_rules_disagree(r, &l, &s, t + 2, label).await;
    gossip(r, &x).await;
    match gate {
        Gate::On => {
            let m = format!("{label}: ON — the drain must apply S1 first (bond 2 > 1)");
            assert_eq!(canonical(r, t + 2), Some(s[0].hash()), "{m}");
            assert_on(r, &s[1], t + 3, &m).await;
        }
        Gate::Off => {
            let m = format!("{label}: OFF — the drain must apply L1 first (depth 4 > 2)");
            assert_eq!(canonical(r, t + 2), Some(l[0].hash()), "{m}");
            assert_on(r, &l[3], t + 5, &m).await;
        }
    }
}

// ----------------------------------------------------------------------- walk

/// S = [k1, k2, k1] on F; L = [k2 x5] on F (S1 and L1 stay unknown to the node).
pub fn walk_set(r: &Rig) -> (Vec<Block>, Vec<Block>) {
    let s = chain_by(r, r.f_hash, F + 1, &[1, 2, 1], last_slot(&r.ours));
    let l = chain_by(r, r.f_hash, F + 1, &[2; 5], last_slot(&s));
    (s, l)
}

/// Caches S2,S3 and L2..L5 at production weight, a peer names S3, then a walk is
/// triggered at the current tip. On: seeds S2 (fetches S1). Off: seeds L2 (L1).
pub async fn walk_at(r: &mut Rig, s: &[Block], l: &[Block], gate: Gate, label: &str) {
    for (i, b) in s.iter().enumerate().skip(1) {
        cache_weighed(r, b, F + 1 + i as u64, 1).await;
    }
    for (i, b) in l.iter().enumerate().skip(1) {
        cache_weighed(r, b, F + 1 + i as u64, 1).await;
    }
    peer_at(r, F + 3, s[2].hash(), s[2].header.slot).await;
    for b in [&s[0], &l[0]] {
        assert!(
            !stored(r, b) && !cached(r, &b.hash()).await,
            "{label} precondition: S1 and L1 are unknown"
        );
    }
    assert_rules_disagree(r, &l[1..], &s[1..], F + 2, label).await;
    assert!(
        !r.node.sync_manager.read().await.is_fork_recovery_active(),
        "{label} precondition: no walk is active"
    );
    let started = r.node.try_trigger_fork_recovery().await;
    let target = (r.node.sync_manager.read().await).fork_recovery_current_parent();
    assert!(started, "{label}: no walk started");
    let (want, name) = match gate {
        Gate::On => (&s[0], "ON — seed S2 (bond 2 > 1), fetch S1"),
        Gate::Off => (&l[0], "OFF — seed L2 (blocks 4 > 2), fetch L1"),
    };
    assert_eq!(target, Some(want.hash()), "{label}: the walk must {name}");
}
