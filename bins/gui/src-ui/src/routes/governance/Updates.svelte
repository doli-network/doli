<script>
  /**
   * Governance updates -- check for updates and view status.
   */
  import { checkUpdates, updateStatus } from '../../../lib/api/governance.js';
  import LoadingSpinner from '../../../lib/components/LoadingSpinner.svelte';

  let updates = $state(null);
  let status = $state(null);
  let loading = $state(false);
  let error = $state(null);

  async function handleCheck() {
    loading = true;
    error = null;
    try {
      updates = await checkUpdates();
      status = await updateStatus();
    } catch (err) {
      error = String(err);
    } finally { loading = false; }
  }

  $effect(() => { handleCheck(); });
</script>

<div class="updates-page">
  <div class="page-header">
    <h2>Governance Updates</h2>
    <button class="btn btn-ghost" onclick={handleCheck} disabled={loading}>
      {#if loading}<LoadingSpinner size="16px" label="" />{:else}Check{/if}
    </button>
  </div>

  {#if error}
    <p class="error-text">{error}</p>
  {/if}

  {#if updates}
    <div class="update-card">
      <h3>Available Update</h3>
      <div class="info-row"><span>Version:</span><span>{updates.version || '--'}</span></div>
      <div class="info-row"><span>Description:</span><span>{updates.description || '--'}</span></div>
    </div>
  {:else if !loading}
    <p class="info-text">No updates available. Your software is up to date.</p>
  {/if}

  {#if status}
    <div class="status-card">
      <h3>Update Status</h3>
      <div class="info-row"><span>Current Version:</span><span>{status.currentVersion || '--'}</span></div>
      <div class="info-row"><span>Status:</span><span>{status.status || '--'}</span></div>
    </div>
  {/if}
</div>

<style>
  .updates-page { padding: 24px; }
  .page-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px; }
  h2 { margin: 0; }
  .update-card, .status-card { padding: 16px; background: var(--color-surface, #1a1a2e); border: 1px solid var(--color-border, #2d2d4a); border-radius: 8px; margin-bottom: 12px; }
  h3 { margin: 0 0 12px; font-size: 15px; }
  .info-row { display: flex; justify-content: space-between; padding: 4px 0; font-size: 13px; }
  .error-text { color: var(--color-error, #f44336); }
  .info-text { color: var(--color-text-muted, #8888aa); }
  .btn { padding: 6px 12px; border: none; border-radius: 6px; font-size: 13px; cursor: pointer; }
  .btn-ghost { background: transparent; color: var(--color-text-muted, #8888aa); }
  .btn:disabled { opacity: 0.5; }
</style>
