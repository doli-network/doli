//! Persisted keys `truncate_chain` restores after its undo loop, mirroring `rollback_one_block`.

use std::collections::BTreeSet;

use doli_core::{EpochSnapshot, EpochState, Network};
use storage::StateDb;

/// Puts back the epoch state that `undo(new_tip + 1)` saved before that block applied.
pub(super) fn restore_epoch_state(state_db: &StateDb, new_tip: u64, snapshot: Option<Vec<u8>>) {
    let at = new_tip + 1;
    match snapshot.map(|bytes| EpochState::deserialize(&bytes).map(|_| bytes)) {
        Some(Ok(bytes)) => {
            state_db.put_epoch_state(&bytes);
            println!("epoch state restored for tip {new_tip} from undo at height {at}");
        }
        Some(Err(e)) => {
            println!("Epoch state snapshot in undo at height {at} does not decode: {e}");
            println!(
                "Stored epoch state kept. WARNING: no epoch state snapshot in undo at height {at}"
            );
        }
        None => {
            println!(
                "Stored epoch state kept. WARNING: no epoch state snapshot in undo at height {at}"
            )
        }
    }
}

/// Deletes the epoch table of every dropped boundary (and of epoch 0), as the live rollback does.
pub(super) fn drop_epoch_tables(state_db: &StateDb, network: Network, new_tip: u64, old_tip: u64) {
    let bpe = network.blocks_per_reward_epoch();
    let starts: BTreeSet<u64> = (new_tip + 1..=old_tip)
        .filter(|&h| EpochSnapshot::is_epoch_boundary_with(h, bpe) || h < bpe)
        .map(|h| h - h % bpe)
        .collect();
    let mut dropped = 0u64;
    for start in starts {
        dropped += u64::from(state_db.get_epoch_table(start).is_some());
        state_db.delete_epoch_table(start);
    }
    println!("epoch tables dropped: {dropped}");
}

/// The node recomputes the commitment on its next integrity scan.
pub(super) fn clear_chain_commitment(state_db: &StateDb) {
    state_db.delete_chain_commitment();
    println!("Chain commitment cleared.");
}
