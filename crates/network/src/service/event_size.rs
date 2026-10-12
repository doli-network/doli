//! Heap estimates of event payloads, for [`event_bytes`](super::event_sink::event_bytes).

use doli_core::{Block, Transaction};

use crate::protocols::SyncResponse;

const INPUT_BYTES: usize = 112;
const OUTPUT_BYTES: usize = 56;

pub(super) fn tx_bytes(tx: &Transaction) -> usize {
    let outputs: usize = tx
        .outputs
        .iter()
        .map(|o| OUTPUT_BYTES + o.extra_data.len())
        .sum();
    16 + tx.inputs.len() * INPUT_BYTES + outputs + tx.extra_data.len()
}

pub(super) fn block_bytes(b: &Block) -> usize {
    b.header.size() + b.transactions.iter().map(tx_bytes).sum::<usize>()
}

fn opt_len(v: &Option<Vec<u8>>) -> usize {
    v.as_ref().map_or(0, Vec::len)
}

pub(super) fn sync_response_bytes(r: &SyncResponse) -> usize {
    match r {
        SyncResponse::Headers(h) => h.iter().map(|h| h.size()).sum(),
        SyncResponse::Bodies(b) => b.iter().map(block_bytes).sum(),
        SyncResponse::Block(b) => b.as_ref().map_or(0, block_bytes),
        SyncResponse::StateSnapshot {
            chain_state,
            utxo_set,
            producer_set,
            block_header_bytes,
            epoch_bond_snapshot_bytes,
            epoch_accumulators_bytes,
            epoch_state_bytes,
            ..
        } => {
            chain_state.len()
                + utxo_set.len()
                + producer_set.len()
                + opt_len(block_header_bytes)
                + opt_len(epoch_bond_snapshot_bytes)
                + opt_len(epoch_accumulators_bytes)
                + opt_len(epoch_state_bytes)
        }
        SyncResponse::StateManifest {
            chain_state,
            producer_set,
            block_header_bytes,
            epoch_bond_snapshot_bytes,
            epoch_accumulators_bytes,
            epoch_state_bytes,
            ..
        } => {
            chain_state.len()
                + producer_set.len()
                + opt_len(block_header_bytes)
                + opt_len(epoch_bond_snapshot_bytes)
                + opt_len(epoch_accumulators_bytes)
                + opt_len(epoch_state_bytes)
        }
        SyncResponse::StateChunk { body, next_key, .. } => body.len() + opt_len(next_key),
        SyncResponse::Error(s) => s.len(),
        SyncResponse::StateRoot { .. } | SyncResponse::StateSessionUnavailable { .. } => 0,
    }
}
