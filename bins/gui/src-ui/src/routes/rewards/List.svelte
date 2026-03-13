<script>
  /**
   * Claimable rewards list.
   */
  import { listRewards, claimReward, claimAllRewards } from '../../../lib/api/rewards.js';
  import { formatBalance, formatNumber } from '../../../lib/utils/format.js';
  import LoadingSpinner from '../../../lib/components/LoadingSpinner.svelte';
  import TxResult from '../../../lib/components/TxResult.svelte';
  import { addNotification } from '../../../lib/stores/notifications.js';

  let rewards = $state([]);
  let loading = $state(false);
  let claiming = $state(false);
  let error = $state(null);
  let result = $state(null);
  let claimError = $state(null);

  async function loadRewards() {
    loading = true;
    error = null;
    try {
      rewards = await listRewards();
    } catch (err) {
      error = String(err);
    } finally { loading = false; }
  }

  async function handleClaim(epoch) {
    claiming = true;
    result = null; claimError = null;
    try {
      const res = await claimReward(epoch);
      result = res;
      addNotification('success', `Reward claimed for epoch ${epoch}`);
      await loadRewards();
    } catch (err) {
      claimError = String(err);
    } finally { claiming = false; }
  }

  async function handleClaimAll() {
    claiming = true;
    result = null; claimError = null;
    try {
      const res = await claimAllRewards();
      result = res;
      addNotification('success', 'All rewards claimed');
      await loadRewards();
    } catch (err) {
      claimError = String(err);
    } finally { claiming = false; }
  }

  function handleDismiss() { result = null; claimError = null; }

  $effect(() => { loadRewards(); });
</script>

<div class="rewards-page">
  <div class="page-header">
    <h2>Claimable Rewards</h2>
    <button class="btn btn-ghost" onclick={loadRewards} disabled={loading}>
      {#if loading}<LoadingSpinner size="16px" label="" />{:else}Refresh{/if}
    </button>
  </div>

  {#if result || claimError}
    <TxResult result={result} error={claimError} onDismiss={handleDismiss} />
  {/if}

  {#if error}
    <p class="error-text">{error}</p>
  {/if}

  {#if rewards.length > 0}
    <button class="btn btn-primary" onclick={handleClaimAll} disabled={claiming}>Claim All</button>

    <div class="rewards-list">
      {#each rewards as reward}
        <div class="reward-item">
          <div class="reward-info">
            <span class="reward-epoch">Epoch {formatNumber(reward.epoch)}</span>
            <span class="reward-amount">{formatBalance(reward.amount)}</span>
          </div>
          <button class="btn btn-secondary btn-sm" onclick={() => handleClaim(reward.epoch)} disabled={claiming}>
            Claim
          </button>
        </div>
      {/each}
    </div>
  {:else if !loading}
    <p class="empty-text">No claimable rewards.</p>
  {/if}
</div>

<style>
  .rewards-page { padding: 24px; }
  .page-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px; }
  h2 { margin: 0; }
  .rewards-list { display: flex; flex-direction: column; gap: 6px; margin-top: 12px; }
  .reward-item { display: flex; justify-content: space-between; align-items: center; padding: 12px; background: var(--color-surface, #1a1a2e); border: 1px solid var(--color-border, #2d2d4a); border-radius: 8px; }
  .reward-info { display: flex; flex-direction: column; gap: 2px; }
  .reward-epoch { font-size: 13px; color: var(--color-text-muted, #8888aa); }
  .reward-amount { font-family: monospace; font-size: 14px; font-weight: 600; }
  .error-text { color: var(--color-error, #f44336); }
  .empty-text { color: var(--color-text-muted, #8888aa); text-align: center; padding: 40px; }
  .btn { padding: 8px 16px; border: none; border-radius: 6px; font-size: 14px; cursor: pointer; font-weight: 500; }
  .btn-primary { background: var(--color-primary, #6c63ff); color: white; }
  .btn-secondary { background: var(--color-surface-alt, #2d2d4a); color: var(--color-text, #e0e0e0); }
  .btn-ghost { background: transparent; color: var(--color-text-muted, #8888aa); }
  .btn-sm { padding: 4px 10px; font-size: 12px; }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
