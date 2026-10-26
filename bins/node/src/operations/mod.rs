mod chain;
mod exited_bond_census;
mod init;
mod producer_set_check;
mod restore;

pub(crate) use chain::{recover_chain_state, reindex_canonical_chain, truncate_chain};
pub(crate) use exited_bond_census::exited_bond_census;
pub(crate) use init::{export_blocks, import_blocks, init_data_dir, show_status};
pub(crate) use producer_set_check::check_producer_set;
pub(crate) use restore::{backfill_from_archive, restore_from_archive, restore_from_rpc};

// Infrastructure for Option G (startup auto-detection bridge). Currently the
// bridgeFromArchive RPC handler composes the same primitives inline in
// crates/rpc/src/methods/guardian.rs.
#[allow(unused_imports)]
pub(crate) use restore::{bridge_checkpoint_to_archive, BridgeReport};
