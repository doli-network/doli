//! The genesis maintainer keys retired on mainnet (INC-I-248).

use crypto::PublicKey;

use super::MaintainerSet;
use crate::genesis::mainnet_genesis_producers;

/// The keys of `MAINNET_GENESIS_PRODUCERS`.
pub fn retired_genesis_maintainer_keys() -> Vec<PublicKey> {
    mainnet_genesis_producers()
        .into_iter()
        .map(|(key, _)| key)
        .collect()
}

/// True when `height` is above `retired_after` and the set holds a retired key.
pub fn holds_retired_key_at(set: &MaintainerSet, height: u64, retired_after: u64) -> bool {
    height > retired_after
        && retired_genesis_maintainer_keys()
            .iter()
            .any(|k| set.is_maintainer(k))
}
