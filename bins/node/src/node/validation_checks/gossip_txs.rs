use super::*;

impl Node {
    /// Relay verdict on one gossip tx message (INC-I-237 D5): every tx is
    /// admitted on its own; Accept iff the message is non-empty and EVERY tx
    /// was newly admitted by this call. Residency never authorizes a forward.
    pub async fn handle_gossip_txs(&self, txs: Vec<Transaction>) -> network::Verdict {
        let mut all_new = !txs.is_empty();
        for tx in txs {
            all_new &= self.handle_new_transaction(tx).await;
        }
        if all_new {
            network::Verdict::Accept
        } else {
            network::Verdict::Ignore
        }
    }
}
