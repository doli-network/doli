<script>
  /**
   * Bottom status bar showing connection status, chain height, network.
   */
  import { networkState } from '../stores/network.js';
  import { formatNumber } from '../utils/format.js';

  let statusColor = $derived(
    networkState.connected ? 'var(--color-success, #4caf50)' : 'var(--color-error, #f44336)'
  );

  let statusText = $derived(
    networkState.connected ? 'Connected' : 'Disconnected'
  );

  let heightText = $derived(
    networkState.chainInfo
      ? `Height: ${formatNumber(networkState.chainInfo.height)}`
      : 'Height: --'
  );

  let networkLabel = $derived(
    networkState.network.charAt(0).toUpperCase() + networkState.network.slice(1)
  );
</script>

<footer class="status-bar" role="status" aria-live="polite">
  <div class="status-left">
    <span class="status-indicator" style="background-color: {statusColor};" aria-hidden="true"></span>
    <span class="status-text">{statusText}</span>
    <span class="status-separator" aria-hidden="true">|</span>
    <span class="status-height">{heightText}</span>
    {#if networkState.chainInfo?.epoch !== undefined}
      <span class="status-separator" aria-hidden="true">|</span>
      <span class="status-epoch">Epoch: {formatNumber(networkState.chainInfo.epoch)}</span>
    {/if}
  </div>
  <div class="status-right">
    <span class="network-badge">{networkLabel}</span>
  </div>
</footer>

<style>
  .status-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 16px;
    background: var(--color-surface, #1a1a2e);
    border-top: 1px solid var(--color-border, #2d2d4a);
    font-size: 12px;
    color: var(--color-text-muted, #8888aa);
    min-height: 28px;
  }

  .status-left {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .status-indicator {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .status-separator {
    opacity: 0.4;
  }

  .status-right {
    display: flex;
    align-items: center;
  }

  .network-badge {
    padding: 2px 8px;
    border-radius: 4px;
    background: var(--color-primary-dim, rgba(108, 99, 255, 0.15));
    color: var(--color-primary, #6c63ff);
    font-weight: 600;
    font-size: 11px;
  }
</style>
