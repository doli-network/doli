//! INC-I-289 N-3: at/above the INC-I-288 gate the doors weigh a branch by the bond
//! of its DISTINCT supporters (K-1). Three rankings that feed the doors still rank
//! per block: the INC-I-281 hold, the fork-walk seed rule and the cached-branch
//! drain order. Each test races a short branch backed by more distinct bond against
//! a long branch built by ONE producer (more blocks, less bond). Every producer has
//! bond 1 and production weight 1, so the per-block unit is the block count and the
//! bond unit is the count of distinct producers. No attestation reaches the node.
//!
// OUTPUT CONTRACT (N-3a): fn Node::handle_new_block(D, peer) (ExtendsTip ->
//   hold_removed_block -> apply -> drain_cached_children), then run_periodic_tasks()
//   O1: chain_state best_hash / best_height (on D's branch)
//   O2: block_store canonical entry at F+3 (= D, not the rival R1)
//   O3: state_db persisted tip, sync manager tip (assert_on)
//   O4: return values Ok(()) (gossip / tick helpers)
//   PATHS: P1 re-delivery of a rollback-removed D through the hold
//   INPUT PARTITIONS: N3A D,E1 (2 blocks, 2 producers) vs cached rival R1..R3
//     (3 blocks, 1 producer) on the shared parent P -> D re-applies, tip on D's branch
//   MATRIX: N3A O1-O4
// OUTPUT CONTRACT (N-3b): fn Node::try_trigger_fork_recovery() -> bool, then the
//   walk (by-hash fetch, connection, handle_completed_fork_recovery)
//   O1: return value of try_trigger_fork_recovery (a walk started)
//   O2: sync_manager walk target (fork_recovery_current_parent = the seed's parent)
//   O3: chain_state best_hash / best_height after the walk (on S)
//   O4: block_store canonical entry at F+1 (= S1)
//   PATHS: P1 two cached orphan branches, one walk seeded and judged
//   INPUT PARTITIONS: N3B cached S2,S3 (2 blocks, 2 producers) vs cached L2..L5
//     (4 blocks, 1 producer); node on A1 (1 producer) -> the walk seeds S2 and the
//     node ends on S
//   MATRIX: N3B O1-O4
// OUTPUT CONTRACT (N-3c): fn Node::handle_new_block(X, peer) (ExtendsTip -> apply ->
//   drain_cached_children in drain_order), then run_periodic_tasks()
//   O1: chain_state best_hash / best_height right after X and after ticks (= S2)
//   O2: block_store canonical entries at F+3 (= S1, never L1)
//   O3: state_db persisted tip, sync manager tip (assert_on)
//   O4: return values Ok(())
//   PATHS: P1 the tip gains a parent X with two cached child branches
//   INPUT PARTITIONS: N3C S1,S2 (2 blocks, 2 producers) vs L1..L4 (4 blocks,
//     1 producer), both on X -> S drained first, the node ends on S2
//   MATRIX: N3C O1-O4
//   covers: fork_cache fork_walk block_handling fork_recovery branch_support

use crypto::{Hash, KeyPair};
use doli_core::{Block, Network, NetworkParams};
use network::protocols::{SyncRequest, SyncResponse};

use super::inc_i_289_common::*;

/// First slot at or after `from` whose leader is `k`.
fn slot_led_by(keys: &[KeyPair], k: &KeyPair, from: u32) -> u32 {
    (from..from + 30)
        .find(|s| leader(keys, *s).public_key() == k.public_key())
        .expect("fixture: every producer leads one slot in three")
}

/// The two producers that are not the node's own key (`keys[0]`).
fn others(r: &Rig) -> (KeyPair, KeyPair) {
    (r.keys[1].clone(), r.keys[2].clone())
}

/// A chain on `prev` from `height`: block i by `by[i]`, each at that producer's
/// next led slot after the previous block's slot (the first after `after`).
fn chain_by(r: &Rig, prev: Hash, height: u64, by: &[&KeyPair], after: u32) -> Vec<Block> {
    let mut prev = prev;
    let mut slot = after;
    by.iter()
        .enumerate()
        .map(|(i, k)| {
            slot = slot_led_by(&r.keys, k, slot + 1);
            let b = block(height + i as u64, slot, prev, k, &r.node.params);
            prev = b.hash();
            b
        })
        .collect()
}

/// (per-block unit: sum of producer effective weights, bond unit: selection
/// weight of the DISTINCT producers) of `blocks` at `height`.
async fn units(r: &Rig, blocks: &[Block], height: u64) -> (u64, u64) {
    let audit = NetworkParams::load(Network::Devnet).security_audit_activation_height;
    let ps = r.node.producer_set.read().await;
    let mut seen = Vec::new();
    let (mut per_block, mut bond) = (0u64, 0u64);
    for b in blocks {
        let pi = ps
            .get_by_pubkey(&b.header.producer)
            .expect("fixture: producer registered");
        per_block += pi.effective_weight(height);
        if !seen.contains(&b.header.producer) {
            seen.push(b.header.producer);
            bond += pi.selection_weight_at(height, audit);
        }
    }
    (per_block, bond)
}

/// No attester other than the node's own key (which the support view drops) is
/// held for any of `blocks`, so their bond is their producers' bond.
async fn assert_no_gossip_support(r: &Rig, blocks: &[Block], label: &str) {
    let own = *r.keys[0].public_key();
    let pending = r.node.sync_manager.read().await.pending_attesters();
    for b in blocks {
        let held: Vec<_> = (pending.iter())
            .filter(|(h, _)| *h == b.hash())
            .flat_map(|(_, v)| v.iter().filter(|pk| **pk != own))
            .collect();
        assert!(
            held.is_empty(),
            "{label}: other attesters held for {:.8}: {held:?}",
            b.hash()
        );
    }
}

/// Drives an active walk up to `ticks` rounds: the by-hash fetch is answered from
/// `serve`, then a connection at a canonical parent completes it. Returns fetches.
async fn drive_walk(r: &mut Rig, serve: &[Block], ticks: usize) -> Vec<Hash> {
    let mut fetched = Vec::new();
    for _ in 0..ticks {
        let req = r.node.sync_manager.write().await.next_request();
        if let Some((q, SyncRequest::GetBlockByHash { hash })) = req {
            fetched.push(hash);
            let blk = serve.iter().find(|b| b.hash() == hash).cloned();
            let out =
                (r.node.sync_manager.write().await).handle_response(q, SyncResponse::Block(blk));
            for b in out {
                r.node
                    .handle_new_block(b, q)
                    .await
                    .expect("handle_new_block returns Ok");
            }
        }
        let parent = (r.node.sync_manager.read().await).fork_recovery_current_parent();
        let connected = parent
            .is_some_and(|p| matches!(r.node.block_store.get_height_by_hash(&p), Ok(Some(_))));
        if connected {
            let done = (r.node.sync_manager.write().await).check_fork_recovery_connection(true);
            if let Some(done) = done {
                let res = r.node.handle_completed_fork_recovery(done).await;
                eprintln!("[N-3b] completed walk -> {res:?}");
            }
        }
        if !r.node.sync_manager.read().await.is_fork_recovery_active() {
            break;
        }
    }
    fetched
}

// N-3a INC-I-289 — Decision: a failure means the hold still weighs blocks, so one producer's longer cached rival holds a rollback-removed block whose branch more distinct bond backs.
#[tokio::test]
async fn n3a_hold_prefers_fewer_blocks_more_bond() {
    let mut r = rig(&[210, 211, 212]).await;
    let (p, d) = (r.ours[1].clone(), r.ours[2].clone());
    rollback_depth_one(&mut r).await;
    assert_on(&r, &p, F + 2, "precondition: tip P = A2 after the rollback").await;
    assert!(removed(&r, &d.hash()).await, "precondition: D is removed");
    let (o1, o2) = others(&r);
    let e_by = if *o2.public_key() != d.header.producer {
        o2.clone()
    } else {
        o1.clone()
    };
    let e1 = chain_by(&r, d.hash(), F + 4, &[&e_by], d.header.slot).remove(0);
    let rival = chain_by(&r, p.hash(), F + 3, &[&o1, &o1, &o1], e1.header.slot);
    for (i, b) in rival.iter().enumerate() {
        cache_at_production_weight(&r, b, F + 3 + i as u64).await;
    }
    gossip(&mut r, &e1).await;
    assert!(
        cached(&r, &e1.hash()).await && !removed(&r, &e1.hash()).await,
        "precondition: E1 is a cached child of D"
    );
    let ours = [d.clone(), e1.clone()];
    let (ours_blocks, ours_bond) = units(&r, &ours, F + 3).await;
    let (rival_blocks, rival_bond) = units(&r, &rival, F + 3).await;
    assert!(
        rival_blocks > ours_blocks && ours_bond > rival_bond,
        "precondition: rival has more blocks ({rival_blocks} > {ours_blocks}) and less bond \
         ({rival_bond} < {ours_bond})"
    );
    let all: Vec<Block> = ours.iter().chain(&rival).cloned().collect();
    assert_no_gossip_support(&r, &all, "precondition").await;

    gossip(&mut r, &d).await;
    let after_d = tip_hash(&r).await;
    eprintln!(
        "[N-3a] tip after D: {after_d:.8} (D {:.8}, E1 {:.8}, P {:.8})",
        d.hash(),
        e1.hash(),
        p.hash()
    );
    assert_eq!(
        r.node.block_store.get_hash_by_height(F + 3).unwrap(),
        Some(d.hash()),
        "N-3a: D held — the hold weighed blocks (rival R1..R3 = {rival_blocks}) instead of \
         bond (D,E1 = {ours_bond} > rival {rival_bond})"
    );
    for _ in 0..3 {
        tick(&mut r).await;
    }
    assert_on(
        &r,
        &e1,
        F + 4,
        "N-3a: the node ends on D,E1 (the bond-heavier branch)",
    )
    .await;
    assert_ne!(
        r.node.block_store.get_hash_by_height(F + 3).unwrap(),
        Some(rival[0].hash()),
        "N-3a: the rival R1 is canonical"
    );
}

// N-3b INC-I-289 — Decision: a failure means the walk seed still ranks by blocks, so one producer's longer cached branch is walked before the branch more distinct bond backs.
#[tokio::test]
async fn n3b_walk_seed_prefers_fewer_blocks_more_bond() {
    let mut r = rig(&[210]).await;
    let a1 = r.ours[0].clone();
    let (o1, o2) = others(&r);
    let s = chain_by(&r, r.f_hash, F + 1, &[&o1, &o2, &o1], a1.header.slot);
    let l = chain_by(&r, r.f_hash, F + 1, &[&o2; 5], s[2].header.slot);
    for (i, b) in s.iter().enumerate().skip(1) {
        cache_at_production_weight(&r, b, F + 1 + i as u64).await;
    }
    for (i, b) in l.iter().enumerate().skip(1) {
        cache_at_production_weight(&r, b, F + 1 + i as u64).await;
    }
    let _peer = peer_at(&r, F + 3, s[2].hash(), s[2].header.slot).await;
    for b in [&s[0], &l[0]] {
        assert!(
            !stored(&r, b) && !cached(&r, &b.hash()).await,
            "precondition: S1 and L1 are unknown (S2 and L2 are walk seeds)"
        );
    }
    let (l_blocks, l_bond) = units(&r, &l[1..], F + 2).await;
    let (s_blocks, s_bond) = units(&r, &s[1..], F + 2).await;
    assert!(
        l_blocks > s_blocks && s_bond > l_bond,
        "precondition: cached L2..L5 has more blocks ({l_blocks} > {s_blocks}) and less bond \
         ({l_bond} < {s_bond}) than cached S2,S3"
    );
    let (_, walked_s) = units(&r, &s[..2], F + 1).await;
    let (_, ours) = units(&r, std::slice::from_ref(&a1), F + 1).await;
    assert!(
        walked_s > ours,
        "precondition: S1,S2 ({walked_s}) out-weighs A1 ({ours}) at the walk's door"
    );
    let all: Vec<Block> = s.iter().chain(&l).cloned().collect();
    assert_no_gossip_support(&r, &all, "precondition").await;
    assert!(
        !r.node.sync_manager.read().await.is_fork_recovery_active(),
        "precondition: no walk is active"
    );

    let started = r.node.try_trigger_fork_recovery().await;
    let target = (r.node.sync_manager.read().await).fork_recovery_current_parent();
    assert!(started, "N-3b: no walk started");
    assert_eq!(
        target,
        Some(s[0].hash()),
        "N-3b: the walk seeded L2 (fetch L1 {:.8}) by blocks ({l_blocks} > {s_blocks}), not S2 \
         (fetch S1) by bond ({s_bond} > {l_bond})",
        l[0].hash()
    );
    let fetched = drive_walk(&mut r, &all, 8).await;
    eprintln!("[N-3b] fetched: {fetched:?}");

    assert_eq!(
        r.node.block_store.get_hash_by_height(F + 1).unwrap(),
        Some(s[0].hash()),
        "N-3b: S1 is not canonical at F+1 after the walk"
    );
    let t = tip_hash(&r).await;
    let at = s.iter().position(|b| b.hash() == t);
    let at = at.unwrap_or_else(|| panic!("N-3b: tip {t:.8} is not on S"));
    assert_on(&r, &s[at], F + 1 + at as u64, "N-3b: the node ends on S").await;
}

// N-3c INC-I-289 — Decision: a failure means the cached-child drain still applies the deepest branch first, so one producer's longer branch is applied without fork choice over the branch more distinct bond backs.
#[tokio::test]
async fn n3c_cached_branch_tip_prefers_fewer_blocks_more_bond() {
    let mut r = rig(&[210]).await;
    let a1 = r.ours[0].clone();
    let (o1, o2) = others(&r);
    let x = chain_by(&r, a1.hash(), F + 2, &[&o1], a1.header.slot).remove(0);
    let s = chain_by(&r, x.hash(), F + 3, &[&o2, &o1], x.header.slot);
    let l = chain_by(&r, x.hash(), F + 3, &[&o1, &o1, &o1, &o1], s[1].header.slot);
    for b in l.iter().chain(&s) {
        gossip(&mut r, b).await;
        assert!(
            cached(&r, &b.hash()).await && !stored(&r, b),
            "precondition: {:.8} is a cached orphan (parent X unknown)",
            b.hash()
        );
    }
    assert_on(&r, &a1, F + 1, "precondition: tip A1").await;
    let (l_blocks, l_bond) = units(&r, &l, F + 3).await;
    let (s_blocks, s_bond) = units(&r, &s, F + 3).await;
    assert!(
        l_blocks > s_blocks && s_bond > l_bond,
        "precondition: L1..L4 has more blocks ({l_blocks} > {s_blocks}) and less bond \
         ({l_bond} < {s_bond}) than S1,S2"
    );
    let all: Vec<Block> = s.iter().chain(&l).cloned().collect();
    assert_no_gossip_support(&r, &all, "precondition").await;

    gossip(&mut r, &x).await;
    let after_x = tip_hash(&r).await;
    eprintln!(
        "[N-3c] tip after X: {after_x:.8} (S2 {:.8}, L4 {:.8})",
        s[1].hash(),
        l[3].hash()
    );
    assert_eq!(
        r.node.block_store.get_hash_by_height(F + 3).unwrap(),
        Some(s[0].hash()),
        "N-3c: the drain applied L1 first by depth ({l_blocks} > {s_blocks}), not S1 by bond \
         ({s_bond} > {l_bond})"
    );
    assert_on(&r, &s[1], F + 4, "N-3c: S drained first, tip S2").await;
    for _ in 0..3 {
        tick(&mut r).await;
    }
    assert_on(
        &r,
        &s[1],
        F + 4,
        "N-3c: the node ends on S2 (the bond-heavier branch)",
    )
    .await;
}
