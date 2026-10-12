use super::*;

impl Node {
    /// Relay verdict on one gossip tx message (INC-I-237 D5): every tx is
    /// admitted on its own; Accept iff the message is non-empty and EVERY tx
    /// was newly admitted by this call. Residency never authorizes a forward;
    /// a zero-flow tx does only if admission authenticated its type (M8),
    /// so e.g. a zero-flow Registration is admitted but never relayed (AUDIT-P1-505).
    pub async fn handle_gossip_txs(&self, txs: Vec<Transaction>) -> network::Verdict {
        let mut all_new = !txs.is_empty();
        for tx in txs {
            let unauthenticated =
                tx.is_zero_flow() && !mempool::zero_flow_auth::is_authenticated(tx.tx_type);
            all_new &= self.handle_new_transaction(tx).await && !unauthenticated;
        }
        if all_new {
            network::Verdict::Accept
        } else {
            network::Verdict::Ignore
        }
    }
}
