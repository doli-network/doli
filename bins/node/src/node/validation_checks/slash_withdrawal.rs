use super::*;

impl Node {
    /// From the slashing activation height, a RequestWithdrawal naming a
    /// Slashed or slash-queued producer is refused in every mode.
    pub(super) async fn check_slashed_withdrawals(&self, block: &Block, height: u64) -> Result<()> {
        if height
            < self
                .config
                .network
                .params()
                .effective_slashing_activation_height()
        {
            return Ok(());
        }
        let producers = self.producer_set.read().await;
        for tx in &block.transactions {
            if tx.tx_type != TxType::RequestWithdrawal {
                continue;
            }
            let Some(wd) = tx.withdrawal_request_data() else {
                continue;
            };
            if producers.is_slashed_or_slash_queued(&wd.producer_pubkey) {
                anyhow::bail!(
                    "[ECON_WITHDRAWAL_SLASHED] RequestWithdrawal at height={} for slashed \
                     or slash-queued producer={}",
                    height,
                    crypto_hash(wd.producer_pubkey.as_bytes())
                );
            }
        }
        Ok(())
    }
}
