//! SlashProducer UTXO effect: every UTXO owned by the accused that is
//! slash-burnable at the block height is spent and nothing is created.

use super::*;
use doli_core::validation::is_slash_burnable;
use storage::{BlockBatch, Outpoint, UtxoEntry};

impl Node {
    /// Owner hash of the accused when `tx` is a SlashProducer mined at or
    /// above the slashing activation height.
    fn slash_burn_owner(&self, tx: &Transaction, height: u64) -> Option<Hash> {
        let gate = self
            .config
            .network
            .params()
            .effective_slashing_activation_height();
        if tx.tx_type != TxType::SlashProducer || height < gate {
            return None;
        }
        let data = tx.slash_data()?;
        Some(hash_with_domain(
            ADDRESS_DOMAIN,
            data.producer_pubkey.as_bytes(),
        ))
    }

    /// Apply path. A burned outpoint created earlier in the same block is left
    /// out of `undo_spent`, so rollback does not re-insert it.
    pub(super) fn slash_burn_in_batch(
        &self,
        tx: &Transaction,
        height: u64,
        batch: &mut BlockBatch<'_>,
        undo_spent: &mut Vec<(Outpoint, UtxoEntry)>,
        created_in_block: &[Outpoint],
    ) -> Result<()> {
        let Some(owner) = self.slash_burn_owner(tx, height) else {
            return Ok(());
        };
        let mut burned = 0u64;
        for (outpoint, entry) in batch.get_utxos_by_pubkey(&owner) {
            if !is_slash_burnable(&entry.output, height) {
                continue;
            }
            batch
                .spend_utxo(&outpoint)
                .map_err(|e| anyhow::anyhow!("slash burn failed at h={}: {}", height, e))?;
            burned = burned.saturating_add(entry.output.amount);
            if !created_in_block.contains(&outpoint) {
                undo_spent.push((outpoint, entry));
            }
        }
        warn!(
            "[SLASH] burned {} units of collateral owned by {} at h={}",
            burned, owner, height
        );
        Ok(())
    }

    /// Rebuild-from-blocks path: same selection, applied to a `UtxoSet`.
    pub(crate) fn slash_burn_in_utxo_set(
        &self,
        tx: &Transaction,
        height: u64,
        utxo: &mut UtxoSet,
    ) -> Result<()> {
        let Some(owner) = self.slash_burn_owner(tx, height) else {
            return Ok(());
        };
        for (outpoint, entry) in utxo.get_by_pubkey_hash(&owner) {
            if is_slash_burnable(&entry.output, height) {
                utxo.remove(&outpoint)?;
            }
        }
        Ok(())
    }
}
