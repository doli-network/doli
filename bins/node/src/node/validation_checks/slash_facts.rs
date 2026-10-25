//! Slash target facts for block validation and the block builder.

use crypto::PublicKey;
use doli_core::transaction::Transaction;
use doli_core::validation::SlashTargetState;
use mempool::slash_admission::slash_targets;
use storage::ProducerSet;

use crate::node::Node;

impl Node {
    /// `slash_targets` over `txs` at `height`, or empty below the effective
    /// slashing height (no producer-set or UTXO read). `held` is a producer-set
    /// guard the caller already holds; `None` takes the read lock here.
    pub(in crate::node) async fn slash_facts(
        &self,
        held: Option<&ProducerSet>,
        height: u64,
        txs: &[Transaction],
    ) -> Vec<(PublicKey, SlashTargetState)> {
        let gate = self
            .config
            .network
            .params()
            .effective_slashing_activation_height();
        if height < gate {
            return Vec::new();
        }
        match held {
            Some(producers) => slash_targets(producers, &*self.utxo_set.read().await, height, txs),
            None => slash_targets(
                &*self.producer_set.read().await,
                &*self.utxo_set.read().await,
                height,
                txs,
            ),
        }
    }
}
