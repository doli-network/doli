<script>
  /**
   * Network settings -- network selector + RPC endpoint configuration.
   */
  import { networkState, setNetwork, setRpcEndpoint } from '../../../lib/stores/network.js';
  import { testConnection } from '../../../lib/api/network.js';
  import { validateUrl } from '../../../lib/utils/validation.js';
  import { addNotification } from '../../../lib/stores/notifications.js';
  import LoadingSpinner from '../../../lib/components/LoadingSpinner.svelte';

  let rpcUrl = $state(networkState.rpcUrl || '');
  let testing = $state(false);
  let testResult = $state(null);

  let urlValid = $derived(validateUrl(rpcUrl).valid);

  const networks = ['mainnet', 'testnet', 'devnet'];

  async function handleTestConnection() {
    if (!urlValid) return;
    testing = true;
    testResult = null;
    try {
      testResult = await testConnection(rpcUrl.trim());
      if (testResult.success) {
        addNotification('success', `Connected (${testResult.latencyMs}ms)`);
      } else {
        addNotification('warning', `Connection failed: ${testResult.error}`);
      }
    } catch (err) {
      testResult = { success: false, error: String(err) };
    } finally { testing = false; }
  }

  async function handleSaveRpc() {
    if (!urlValid) return;
    await setRpcEndpoint(rpcUrl.trim());
  }
</script>

<div class="network-settings">
  <h2>Network Settings</h2>

  <div class="section">
    <h3>Network</h3>
    <div class="network-selector">
      {#each networks as net}
        <button
          class="network-btn"
          class:active={networkState.network === net}
          onclick={() => setNetwork(net)}
        >
          {net.charAt(0).toUpperCase() + net.slice(1)}
        </button>
      {/each}
    </div>
  </div>

  <div class="section">
    <h3>RPC Endpoint</h3>
    <div class="rpc-form">
      <input
        type="text"
        bind:value={rpcUrl}
        placeholder="http://localhost:8332"
        class="input"
      />
      <div class="rpc-actions">
        <button class="btn btn-secondary" disabled={!urlValid || testing} onclick={handleTestConnection}>
          {#if testing}<LoadingSpinner size="16px" label="" />{:else}Test{/if}
        </button>
        <button class="btn btn-primary" disabled={!urlValid} onclick={handleSaveRpc}>
          Save
        </button>
      </div>
    </div>

    {#if testResult}
      <div class="test-result" class:success={testResult.success} class:failure={!testResult.success}>
        {testResult.success ? `Connected (${testResult.latencyMs}ms)` : `Failed: ${testResult.error}`}
      </div>
    {/if}
  </div>

  <div class="section">
    <h3>Connection Status</h3>
    <div class="status-info">
      <div class="info-row"><span>Status:</span><span>{networkState.connected ? 'Connected' : 'Disconnected'}</span></div>
      <div class="info-row"><span>Network:</span><span>{networkState.network}</span></div>
      {#if networkState.chainInfo}
        <div class="info-row"><span>Height:</span><span>{networkState.chainInfo.height}</span></div>
      {/if}
    </div>
  </div>
</div>

<style>
  .network-settings { padding: 24px; max-width: 560px; }
  h2 { margin: 0 0 24px; }
  .section { margin-bottom: 24px; }
  h3 { margin: 0 0 12px; font-size: 15px; }

  .network-selector { display: flex; gap: 8px; }
  .network-btn { padding: 8px 16px; background: var(--color-surface-alt, #2d2d4a); border: 1px solid var(--color-border, #2d2d4a); border-radius: 6px; color: var(--color-text, #e0e0e0); cursor: pointer; font-size: 13px; }
  .network-btn.active { background: var(--color-primary, #6c63ff); border-color: var(--color-primary, #6c63ff); color: white; }

  .rpc-form { display: flex; flex-direction: column; gap: 8px; }
  .input { padding: 8px 12px; background: var(--color-bg, #0f0f23); border: 1px solid var(--color-border, #2d2d4a); border-radius: 6px; color: var(--color-text, #e0e0e0); font-size: 14px; font-family: monospace; }
  .rpc-actions { display: flex; gap: 8px; }

  .test-result { padding: 8px 12px; border-radius: 6px; font-size: 13px; margin-top: 8px; }
  .test-result.success { background: rgba(76, 175, 80, 0.1); color: var(--color-success, #4caf50); }
  .test-result.failure { background: rgba(244, 67, 54, 0.1); color: var(--color-error, #f44336); }

  .status-info { padding: 12px; background: var(--color-surface, #1a1a2e); border: 1px solid var(--color-border, #2d2d4a); border-radius: 8px; }
  .info-row { display: flex; justify-content: space-between; padding: 4px 0; font-size: 13px; }

  .btn { padding: 8px 16px; border: none; border-radius: 6px; font-size: 14px; cursor: pointer; font-weight: 500; }
  .btn-primary { background: var(--color-primary, #6c63ff); color: white; }
  .btn-secondary { background: var(--color-surface-alt, #2d2d4a); color: var(--color-text, #e0e0e0); }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
