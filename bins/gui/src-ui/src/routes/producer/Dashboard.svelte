<script>
  /**
   * Producer status dashboard.
   */
  import { producerStatus } from '../../../lib/api/producer.js';
  import { formatBalance, formatNumber } from '../../../lib/utils/format.js';
  import LoadingSpinner from '../../../lib/components/LoadingSpinner.svelte';

  let status = $state(null);
  let loading = $state(false);
  let error = $state(null);

  async function loadStatus() {
    loading = true;
    error = null;
    try {
      status = await producerStatus();
    } catch (err) {
      error = String(err);
    } finally {
      loading = false;
    }
  }

  $effect(() => { loadStatus(); });
</script>

<div class="dashboard">
  <div class="page-header">
    <h2>Producer Dashboard</h2>
    <button class="btn btn-ghost" onclick={loadStatus} disabled={loading}>
      {#if loading}<LoadingSpinner size="16px" label="" />{:else}Refresh{/if}
    </button>
  </div>

  {#if error}
    <p class="error-text">{error}</p>
  {/if}

  {#if status}
    <div class="status-grid">
      <div class="status-card">
        <span class="card-label">Status</span>
        <span class="card-value">{status.registered ? 'Registered' : 'Not Registered'}</span>
      </div>
      {#if status.registered}
        <div class="status-card">
          <span class="card-label">Total Bonded</span>
          <span class="card-value">{formatBalance(status.totalBonded || 0)}</span>
        </div>
        <div class="status-card">
          <span class="card-label">Blocks Produced</span>
          <span class="card-value">{formatNumber(status.blocksProduced || 0)}</span>
        </div>
        <div class="status-card">
          <span class="card-label">Active</span>
          <span class="card-value">{status.active ? 'Yes' : 'No'}</span>
        </div>
      {/if}
    </div>
  {:else if !loading}
    <p class="info-text">Unable to load producer status. Ensure you are connected to a node.</p>
  {/if}
</div>

<style>
  .dashboard { padding: 24px; }
  .page-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px; }
  h2 { margin: 0; }

  .status-grid { display: grid; grid-template-columns: repeat(2, 1fr); gap: 12px; }
  .status-card {
    padding: 16px;
    background: var(--color-surface, #1a1a2e);
    border: 1px solid var(--color-border, #2d2d4a);
    border-radius: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .card-label { font-size: 12px; color: var(--color-text-muted, #8888aa); }
  .card-value { font-size: 18px; font-weight: 600; }
  .error-text { color: var(--color-error, #f44336); }
  .info-text { color: var(--color-text-muted, #8888aa); }
  .btn { padding: 6px 12px; border: none; border-radius: 6px; font-size: 13px; cursor: pointer; }
  .btn-ghost { background: transparent; color: var(--color-text-muted, #8888aa); }
  .btn:disabled { opacity: 0.5; }
</style>
