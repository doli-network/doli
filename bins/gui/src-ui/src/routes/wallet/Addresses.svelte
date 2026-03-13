<script>
  /**
   * Address list management.
   */
  import { walletState, generateAddress, refreshAddresses } from '../../../lib/stores/wallet.js';
  import { truncateHash } from '../../../lib/utils/format.js';
  import { addNotification } from '../../../lib/stores/notifications.js';

  let newLabel = $state('');

  async function handleGenerate() {
    try {
      await generateAddress(newLabel || null);
      newLabel = '';
    } catch (err) {
      // notification handled in store
    }
  }

  async function handleCopy(address) {
    try {
      await navigator.clipboard.writeText(address);
      addNotification('success', 'Address copied');
    } catch {
      addNotification('error', 'Failed to copy');
    }
  }
</script>

<div class="addresses-page">
  <h2>Addresses</h2>

  <div class="generate-form">
    <input
      type="text"
      bind:value={newLabel}
      placeholder="Label (optional)"
      class="input"
    />
    <button class="btn btn-primary" onclick={handleGenerate}>Generate New</button>
  </div>

  <div class="address-list">
    {#each walletState.addresses as addr, i}
      <div class="address-item">
        <div class="addr-info">
          <span class="addr-index">#{i + 1}</span>
          <div class="addr-details">
            <code class="addr-value">{addr.address}</code>
            {#if addr.label}
              <span class="addr-label">{addr.label}</span>
            {/if}
            {#if addr.pubkeyHash}
              <span class="addr-hash">Hash: {truncateHash(addr.pubkeyHash, 8)}</span>
            {/if}
          </div>
        </div>
        <button class="btn btn-ghost" onclick={() => handleCopy(addr.address)}>Copy</button>
      </div>
    {/each}
  </div>

  {#if walletState.addresses.length === 0}
    <p class="empty-text">No addresses yet. Generate one to get started.</p>
  {/if}
</div>

<style>
  .addresses-page { padding: 24px; }
  h2 { margin: 0 0 20px; }

  .generate-form {
    display: flex;
    gap: 8px;
    margin-bottom: 20px;
  }

  .input {
    flex: 1;
    padding: 8px 12px;
    background: var(--color-bg, #0f0f23);
    border: 1px solid var(--color-border, #2d2d4a);
    border-radius: 6px;
    color: var(--color-text, #e0e0e0);
    font-size: 14px;
  }

  .address-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .address-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px;
    background: var(--color-surface, #1a1a2e);
    border: 1px solid var(--color-border, #2d2d4a);
    border-radius: 8px;
  }

  .addr-info { display: flex; align-items: flex-start; gap: 12px; flex: 1; min-width: 0; }
  .addr-index { color: var(--color-text-muted, #8888aa); font-size: 13px; min-width: 24px; }
  .addr-details { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .addr-value { font-size: 13px; word-break: break-all; }
  .addr-label { font-size: 12px; color: var(--color-primary, #6c63ff); }
  .addr-hash { font-size: 11px; color: var(--color-text-muted, #8888aa); font-family: monospace; }
  .empty-text { color: var(--color-text-muted, #8888aa); text-align: center; padding: 40px; }
  .btn { padding: 6px 12px; border: none; border-radius: 6px; font-size: 13px; cursor: pointer; font-weight: 500; }
  .btn-primary { background: var(--color-primary, #6c63ff); color: white; }
  .btn-ghost { background: transparent; color: var(--color-text-muted, #8888aa); }
</style>
