mod chain;
mod init;
mod restore;

pub(crate) use chain::{recover_chain_state, reindex_canonical_chain, truncate_chain};
pub(crate) use init::{export_blocks, import_blocks, init_data_dir, show_status};
pub(crate) use restore::{backfill_from_archive, restore_from_archive, restore_from_rpc};

// Infrastructure for Option G (startup auto-detection bridge). Currently the
// bridgeFromArchive RPC handler composes the same primitives inline in
// crates/rpc/src/methods/guardian.rs.
#[allow(unused_imports)]
pub(crate) use restore::{bridge_checkpoint_to_archive, BridgeReport};
