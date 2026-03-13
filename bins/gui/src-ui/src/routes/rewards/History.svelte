<script>
  /**
   * Rewards claim history.
   */
  import { listRewards } from '../../../lib/api/rewards.js';
  import { formatBalance, formatNumber, formatTimestamp } from '../../../lib/utils/format.js';
  import LoadingSpinner from '../../../lib/components/LoadingSpinner.svelte';

  let history = $state([]);
  let loading = $state(false);
  let error = $state(null);

  async function loadHistory() {
    loading = true;
    error = null;
    try {
      const all = await listRewards();
      history = all.filter((r) => r.claimed);
    } catch (err) {
      error = String(err);
    } finally { loading = false; }
  }

  $effect(() => { loadHistory(); });
</script>

<div class="history-page">
  <div class="page-header">
    <h2>Reward History</h2>
    <button class="btn btn-ghost" onclick={loadHistory} disabled={loading}>
      {#if loading}<LoadingSpinner size="16px" label="" />{:else}Refresh{/if}
    </button>
  </div>

  {#if error}
    <p class="error-text">{error}</p>
  {/if}

  {#if history.length === 0 && !loading}
    <p class="empty-text">No reward claims yet.</p>
  {/if}

  <div class="history-list">
    {#each history as entry}
      <div class="history-item">
        <span class="epoch">Epoch {formatNumber(entry.epoch)}</span>
        <span class="amount">{formatBalance(entry.amount)}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  .history-page { padding: 24px; }
  .page-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px; }
  h2 { margin: 0; }
  .history-list { display: flex; flex-direction: column; gap: 4px; }
  .history-item { display: flex; justify-content: space-between; padding: 10px 12px; background: var(--color-surface, #1a1a2e); border: 1px solid var(--color-border, #2d2d4a); border-radius: 6px; font-size: 13px; }
  .epoch { color: var(--color-text-muted, #8888aa); }
  .amount { font-family: monospace; font-weight: 600; }
  .error-text { color: var(--color-error, #f44336); }
  .empty-text { color: var(--color-text-muted, #8888aa); text-align: center; padding: 40px; }
  .btn { padding: 6px 12px; border: none; border-radius: 6px; font-size: 13px; cursor: pointer; }
  .btn-ghost { background: transparent; color: var(--color-text-muted, #8888aa); }
  .btn:disabled { opacity: 0.5; }
</style>
