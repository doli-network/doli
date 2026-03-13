<script>
  /**
   * Wallet settings -- wallet info, export, import, BLS key management.
   */
  import { walletState, loadWallet, resetWallet } from '../../../lib/stores/wallet.js';
  import { walletInfo, exportWallet, importWallet, addBlsKey } from '../../../lib/api/wallet.js';
  import { addNotification } from '../../../lib/stores/notifications.js';
  import { truncateHash } from '../../../lib/utils/format.js';

  let { onNavigate = () => {} } = $props();

  let loading = $state(false);

  async function handleExport() {
    loading = true;
    try {
      const result = await exportWallet(null);
      addNotification('success', `Wallet exported: ${result}`);
    } catch (err) {
      addNotification('error', `Export failed: ${err}`);
    } finally { loading = false; }
  }

  async function handleImport() {
    loading = true;
    try {
      const info = await importWallet(null);
      addNotification('success', 'Wallet imported');
    } catch (err) {
      addNotification('error', `Import failed: ${err}`);
    } finally { loading = false; }
  }

  async function handleAddBls() {
    loading = true;
    try {
      const pubkey = await addBlsKey();
      addNotification('success', 'BLS key added');
    } catch (err) {
      addNotification('error', `BLS key failed: ${err}`);
    } finally { loading = false; }
  }
</script>

<div class="wallet-settings">
  <h2>Wallet Settings</h2>

  {#if walletState.info}
    <div class="section">
      <h3>Wallet Info</h3>
      <div class="info-card">
        <div class="info-row"><span>Name:</span><span>{walletState.info.name}</span></div>
        <div class="info-row"><span>Version:</span><span>{walletState.info.version}</span></div>
        <div class="info-row"><span>Addresses:</span><span>{walletState.addresses.length}</span></div>
        {#if walletState.info.primaryAddress}
          <div class="info-row">
            <span>Primary Address:</span>
            <code>{truncateHash(walletState.info.primaryAddress, 12)}</code>
          </div>
        {/if}
        <div class="info-row"><span>BLS Key:</span><span>{walletState.info.hasBlsKey ? 'Yes' : 'No'}</span></div>
      </div>
    </div>
  {/if}

  <div class="section">
    <h3>Actions</h3>
    <div class="actions">
      <button class="btn btn-secondary" disabled={loading} onclick={handleExport}>Export Wallet</button>
      <button class="btn btn-secondary" disabled={loading} onclick={handleImport}>Import Wallet</button>
      {#if walletState.info && !walletState.info.hasBlsKey}
        <button class="btn btn-secondary" disabled={loading} onclick={handleAddBls}>Add BLS Key</button>
      {/if}
      <button class="btn btn-ghost" onclick={() => { resetWallet(); onNavigate('setup/welcome'); }}>
        Close Wallet
      </button>
    </div>
  </div>
</div>

<style>
  .wallet-settings { padding: 24px; max-width: 560px; }
  h2 { margin: 0 0 24px; }
  .section { margin-bottom: 24px; }
  h3 { margin: 0 0 12px; font-size: 15px; }

  .info-card { padding: 12px; background: var(--color-surface, #1a1a2e); border: 1px solid var(--color-border, #2d2d4a); border-radius: 8px; }
  .info-row { display: flex; justify-content: space-between; padding: 6px 0; font-size: 13px; }
  .info-row span:first-child { color: var(--color-text-muted, #8888aa); }

  .actions { display: flex; flex-direction: column; gap: 8px; }
  .btn { padding: 8px 16px; border: none; border-radius: 6px; font-size: 14px; cursor: pointer; font-weight: 500; }
  .btn-secondary { background: var(--color-surface-alt, #2d2d4a); color: var(--color-text, #e0e0e0); }
  .btn-ghost { background: transparent; color: var(--color-text-muted, #8888aa); }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
