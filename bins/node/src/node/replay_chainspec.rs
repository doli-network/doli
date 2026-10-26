//! The replay node runs on the chainspec a running node of its network selects.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use crypto::Hash;
use doli_core::chainspec::ChainSpec;
use doli_core::consensus::ConsensusParams;
use doli_core::Network;

use super::Node;

impl Node {
    /// [`Node::new_for_replay_with_chainspec`] on the chainspec a running node
    /// of `network` selects for `data_dir`.
    pub async fn new_for_replay(data_dir: PathBuf, network: Network) -> Result<Self> {
        let spec = crate::chainspec_select::network_chainspec(network, None, &data_dir)
            .map_err(|e| anyhow!(e))?;
        Self::new_for_replay_with_chainspec(data_dir, network, spec).await
    }
}

/// Genesis hash and params as `Node::new` derives them from `chainspec`.
pub(super) fn replay_params(
    network: Network,
    chainspec: Option<&ChainSpec>,
) -> (Hash, ConsensusParams) {
    let mut params = ConsensusParams::for_network(network);
    let Some(spec) = chainspec else {
        return (params.genesis_hash, params);
    };
    params.apply_chainspec(spec);
    (spec.genesis_hash(), params)
}
