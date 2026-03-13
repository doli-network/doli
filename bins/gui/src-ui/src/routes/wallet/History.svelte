<script>
  /**
   * Transaction history list.
   */
  import { getHistory } from '../../../lib/api/transactions.js';
  import { formatBalance, formatTimestamp, truncateHash } from '../../../lib/utils/format.js';
  import LoadingSpinner from '../../../lib/components/LoadingSpinner.svelte';

  let history = $state([]);
  let loading = $state(false);
  let error = $state(null);

  async function loadHistory() {
    loading = true;
    error = null;
    try {
      history = await getHistory(50);
    } catch (err) {
      error = String(err);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    loadHistory();
  });
</script>

<div class="history-page">
  <div class="page-header">
    <h2>Transaction History</h2>
    <button class="btn btn-ghost" onclick={loadHistory} disabled={loading}>
      {#if loading}
        <LoadingSpinner size="16px" label="" />
      {:else}
        Refresh
      {/if}
    </button>
  </div>

  {#if error}
    <p class="error-text">{error}</p>
  {/if}

  {#if history.length === 0 && !loading}
    <p class="empty-text">No transactions yet.</p>
  {/if}

  <div class="tx-list">
    {#each history as tx}
      <div class="tx-item">
        <div class="tx-main">
          <span class="tx-type" class:tx-received={tx.direction === 'received'} class:tx-sent={tx.direction === 'sent'}>
            {tx.direction === 'received' ? '+' : '-'}
          </span>
          <div class="tx-details">
            <span class="tx-hash" title={tx.txHash}>{truncateHash(tx.txHash, 8)}</span>
            <span class="tx-time">{formatTimestamp(tx.timestamp)}</span>
          </div>
        </div>
        <span class="tx-amount" class:tx-received={tx.direction === 'received'} class:tx-sent={tx.direction === 'sent'}>
          {tx.direction === 'received' ? '+' : '-'}{formatBalance(tx.amount)}
        </span>
      </div>
    {/each}
  </div>
</div>

<style>
  .history-page { padding: 24px; }

  .page-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 20px;
  }

  h2 { margin: 0; }

  .tx-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .tx-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px;
    background: var(--color-surface, #1a1a2e);
    border: 1px solid var(--color-border, #2d2d4a);
    border-radius: 8px;
  }

  .tx-main {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .tx-type {
    font-size: 18px;
    font-weight: 700;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .tx-received { color: var(--color-success, #4caf50); }
  .tx-sent { color: var(--color-error, #f44336); }

  .tx-details {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .tx-hash {
    font-family: monospace;
    font-size: 13px;
  }

  .tx-time {
    font-size: 12px;
    color: var(--color-text-muted, #8888aa);
  }

  .tx-amount {
    font-family: monospace;
    font-size: 14px;
    font-weight: 600;
  }

  .error-text { color: var(--color-error, #f44336); }
  .empty-text { color: var(--color-text-muted, #8888aa); text-align: center; padding: 40px; }
  .btn { padding: 6px 12px; border: none; border-radius: 6px; font-size: 13px; cursor: pointer; }
  .btn-ghost { background: transparent; color: var(--color-text-muted, #8888aa); }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
