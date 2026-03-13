<script>
  /**
   * Network producer list.
   */
  import { producerStatus } from '../../../lib/api/producer.js';
  import { formatBalance, formatNumber } from '../../../lib/utils/format.js';
  import LoadingSpinner from '../../../lib/components/LoadingSpinner.svelte';

  let producers = $state([]);
  let loading = $state(false);
  let error = $state(null);

  async function loadProducers() {
    loading = true;
    error = null;
    try {
      // producerStatus returns status which may include producers list
      const status = await producerStatus();
      producers = status.producers || [];
    } catch (err) {
      error = String(err);
    } finally { loading = false; }
  }

  $effect(() => { loadProducers(); });
</script>

<div class="producers-page">
  <div class="page-header">
    <h2>Network Producers</h2>
    <button class="btn btn-ghost" onclick={loadProducers} disabled={loading}>
      {#if loading}<LoadingSpinner size="16px" label="" />{:else}Refresh{/if}
    </button>
  </div>

  {#if error}
    <p class="error-text">{error}</p>
  {/if}

  {#if producers.length === 0 && !loading}
    <p class="empty-text">No producers found.</p>
  {/if}

  <div class="producer-list">
    {#each producers as producer}
      <div class="producer-item">
        <div class="producer-info">
          <code class="producer-pubkey">{producer.pubkey || '--'}</code>
          <span class="producer-status" class:active={producer.active}>{producer.active ? 'Active' : 'Inactive'}</span>
        </div>
        <span class="producer-bond">{formatBalance(producer.totalBond || 0)}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  .producers-page { padding: 24px; }
  .page-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px; }
  h2 { margin: 0; }
  .producer-list { display: flex; flex-direction: column; gap: 6px; }
  .producer-item { display: flex; justify-content: space-between; align-items: center; padding: 12px; background: var(--color-surface, #1a1a2e); border: 1px solid var(--color-border, #2d2d4a); border-radius: 8px; }
  .producer-info { display: flex; flex-direction: column; gap: 4px; }
  .producer-pubkey { font-size: 12px; word-break: break-all; }
  .producer-status { font-size: 12px; }
  .producer-status.active { color: var(--color-success, #4caf50); }
  .producer-bond { font-family: monospace; font-size: 13px; }
  .error-text { color: var(--color-error, #f44336); }
  .empty-text { color: var(--color-text-muted, #8888aa); text-align: center; padding: 40px; }
  .btn { padding: 6px 12px; border: none; border-radius: 6px; font-size: 13px; cursor: pointer; }
  .btn-ghost { background: transparent; color: var(--color-text-muted, #8888aa); }
  .btn:disabled { opacity: 0.5; }
</style>
