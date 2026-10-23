//! INC-I-256 M1 / R-2 — revalidation evicts a resident slash on the full
//! slash-target verdict judged at best + 1, not only on Slashed or queued.
//!
// covers: pool slash_admission entry lib slash_state
//
// OUTPUT CONTRACT: fn Mempool::revalidate(&mut self, &UtxoSet, u64 /*best*/, u32)
//   O2 receiver residency of the resident slash (contains); no return value; the shared
//   producer set and the UtxoSet are read-only
// PATHS: P-KEEP accused still slashable | P-BUSY producer-set write lock held (fail open, keep)
//        | P-STATE accused state fails the target check | P-LOCK collateral unlock judged at best + 1
// INPUT PARTITIONS:
//   target-check failure: Slashed (already evicted today) | Exited with no collateral (not evicted today)
//   collateral lock L: best = L - 2 (judged L - 1, locked, keep) | best = L - 1 (judged L, unlocked, evict)
// MATRIX:
//   O2 x {P-KEEP, P-BUSY, P-STATE} x {Slashed, Exited} -> inc_i_256_m1_revalidate_evicts_slash_whose_target_check_fails
//   O2 x P-LOCK x {L - 2, L - 1} -> inc_i_256_m1_revalidate_evicts_slash_when_collateral_unlocks_at_next_height

use crypto::{Hash, KeyPair};
use doli_core::transaction::{Output, OutputType};
use mempool::MempoolPolicy;
use storage::{Outpoint, ProducerStatus};

use crate::inc_i_256_m1_fixture::{addr, policy, rig, seed, slash, TIP};

const L: u64 = 100;

#[derive(Clone, Copy, Debug)]
enum Reason {
    Slashed,
    Exited,
}

// R-2 — Decision: a failure means a slash whose accused can no longer be slashed stays resident and poisons every block the builder assembles from it.
#[test]
fn inc_i_256_m1_revalidate_evicts_slash_whose_target_check_fails() {
    for reason in [Reason::Slashed, Reason::Exited] {
        let x = KeyPair::generate();
        let mut r = rig(policy(10, MempoolPolicy::testnet().max_age), &[&x]);
        let s = slash(&x, (0xA1, 0xB2));
        r.pool
            .add_transaction(s.clone(), &r.utxo, TIP, 0)
            .expect("PRECONDITION: the slash is admitted");
        r.pool.revalidate(&r.utxo, TIP, 0);
        assert!(
            r.pool.contains(&s.hash()),
            "{reason:?}: NON-VACUITY: a still-slashable accused keeps its slash"
        );

        {
            let mut ps = r.producers.try_write().expect("FIXTURE: lock");
            match reason {
                Reason::Slashed => {
                    ps.slash_producer(x.public_key(), 0)
                        .expect("FIXTURE: slash");
                }
                Reason::Exited => {
                    ps.get_by_pubkey_mut(x.public_key())
                        .expect("FIXTURE: registered")
                        .status = ProducerStatus::Exited;
                }
            }
        }
        let producers = r.producers.clone();
        let held = producers.try_write().expect("FIXTURE: hold the write lock");
        r.pool.revalidate(&r.utxo, TIP, 0);
        assert!(
            r.pool.contains(&s.hash()),
            "{reason:?}: a busy producer set must not evict (fail open)"
        );
        drop(held);

        r.pool.revalidate(&r.utxo, TIP, 0);
        assert!(
            !r.pool.contains(&s.hash()),
            "{reason:?}: revalidate must evict a slash whose accused fails the target check"
        );
        assert_eq!(r.pool.len(), 0);
    }
}

// R-2 — Decision: a failure means revalidation keeps a slash whose only burnable collateral unlocks at the next block, or judges the lock at the tip instead of best + 1.
#[test]
fn inc_i_256_m1_revalidate_evicts_slash_when_collateral_unlocks_at_next_height() {
    let x = KeyPair::generate();
    let mut r = rig(policy(10, MempoolPolicy::testnet().max_age), &[&x]);
    r.producers
        .try_write()
        .expect("FIXTURE: lock")
        .get_by_pubkey_mut(x.public_key())
        .expect("FIXTURE: registered")
        .status = ProducerStatus::Exited;
    seed(
        &mut r.utxo,
        Outpoint::new(Hash::from_bytes([0x01; 32]), 0),
        Output {
            output_type: OutputType::Unbonding,
            amount: doli_core::network::Network::Devnet.bond_unit(),
            pubkey_hash: addr(x.public_key()),
            lock_until: L,
            extra_data: Vec::new(),
        },
    );
    let s = slash(&x, (0xA1, 0xB2));
    r.pool
        .add_transaction(s.clone(), &r.utxo, L - 3, 0)
        .expect("PRECONDITION: judged at L - 2 the Exited accused owns locked collateral");

    r.pool.revalidate(&r.utxo, L - 2, 0);
    assert!(
        r.pool.contains(&s.hash()),
        "best L - 2: judged at L - 1 the collateral is still locked, keep"
    );
    r.pool.revalidate(&r.utxo, L - 1, 0);
    assert!(
        !r.pool.contains(&s.hash()),
        "best L - 1: judged at L nothing is burnable, revalidate must evict"
    );
}
