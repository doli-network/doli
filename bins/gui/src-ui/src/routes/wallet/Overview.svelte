<script>
  /**
   * Balance overview -- displays total balance, recent activity.
   */
  import { walletState, refreshBalance } from '../../../lib/stores/wallet.js';
  import { formatBalance, formatBalanceShort } from '../../../lib/utils/format.js';
  import LoadingSpinner from '../../../lib/components/LoadingSpinner.svelte';

  let { onNavigate = () => {} } = $props();

  let loading = $state(false);

  async function handleRefresh() {
    loading = true;
    await refreshBalance();
    loading = false;
  }

  // Refresh on mount
  $effect(() => {
    handleRefresh();
  });
</script>

<div class="overview">
  <div class="overview-header">
    <h2>Wallet Overview</h2>
    <button class="btn btn-ghost" onclick={handleRefresh} disabled={loading} aria-label="Refresh balance">
      {#if loading}
        <LoadingSpinner size="16px" label="" />
      {:else}
        Refresh
      {/if}
    </button>
  </div>

  <div class="balance-card">
    <span class="balance-label">Total Balance</span>
    <span class="balance-value">
      {#if walletState.balance}
        {formatBalance(walletState.balance.confirmed)}
      {:else}
        -- DOLI
      {/if}
    </span>
    {#if walletState.balance?.unconfirmed}
      <span class="balance-pending">
        + {formatBalanceShort(walletState.balance.unconfirmed)} pending
      </span>
    {/if}
  </div>

  <div class="actions-row">
    <button class="btn btn-primary" onclick={() => onNavigate('wallet/send')}>Send</button>
    <button class="btn btn-secondary" onclick={() => onNavigate('wallet/receive')}>Receive</button>
    <button class="btn btn-secondary" onclick={() => onNavigate('wallet/history')}>History</button>
  </div>

  {#if walletState.info}
    <div class="info-section">
      <h3>Wallet Info</h3>
      <div class="info-row">
        <span class="info-label">Name:</span>
        <span class="info-value">{walletState.info.name}</span>
      </div>
      <div class="info-row">
        <span class="info-label">Addresses:</span>
        <span class="info-value">{walletState.addresses.length}</span>
      </div>
    </div>
  {/if}
</div>

<style>
  .overview {
    padding: 24px;
  }

  .overview-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 24px;
  }

  h2 { margin: 0; }

  .balance-card {
    display: flex;
    flex-direction: column;
    padding: 24px;
    background: var(--color-surface, #1a1a2e);
    border: 1px solid var(--color-border, #2d2d4a);
    border-radius: 12px;
    margin-bottom: 24px;
  }

  .balance-label {
    font-size: 13px;
    color: var(--color-text-muted, #8888aa);
    margin-bottom: 4px;
  }

  .balance-value {
    font-size: 28px;
    font-weight: 700;
    font-family: monospace;
  }

  .balance-pending {
    font-size: 14px;
    color: var(--color-text-muted, #8888aa);
    margin-top: 4px;
  }

  .actions-row {
    display: flex;
    gap: 8px;
    margin-bottom: 24px;
  }

  .info-section {
    background: var(--color-surface, #1a1a2e);
    border: 1px solid var(--color-border, #2d2d4a);
    border-radius: 8px;
    padding: 16px;
  }

  .info-section h3 {
    margin: 0 0 12px;
    font-size: 15px;
  }

  .info-row {
    display: flex;
    justify-content: space-between;
    padding: 6px 0;
    font-size: 13px;
  }

  .info-label { color: var(--color-text-muted, #8888aa); }

  .btn {
    padding: 8px 16px;
    border: none;
    border-radius: 6px;
    font-size: 14px;
    cursor: pointer;
    font-weight: 500;
  }

  .btn-primary { background: var(--color-primary, #6c63ff); color: white; }
  .btn-secondary { background: var(--color-surface-alt, #2d2d4a); color: var(--color-text, #e0e0e0); }
  .btn-ghost { background: transparent; color: var(--color-text-muted, #8888aa); }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
