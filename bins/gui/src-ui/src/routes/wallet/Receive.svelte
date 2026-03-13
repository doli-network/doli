<script>
  /**
   * Receive address display with copy support.
   * GUI-FR-015 (clipboard copy), GUI-FR-016 (QR code -- deferred).
   */
  import { walletState, generateAddress } from '../../../lib/stores/wallet.js';
  import { truncateHash } from '../../../lib/utils/format.js';
  import { addNotification } from '../../../lib/stores/notifications.js';

  let selectedAddress = $state(null);

  let primaryAddress = $derived(
    walletState.addresses.length > 0 ? walletState.addresses[0] : null
  );

  let displayAddress = $derived(selectedAddress || primaryAddress);

  async function handleCopy() {
    if (!displayAddress) return;
    try {
      // Use clipboard API (Tauri plugin or browser fallback)
      await navigator.clipboard.writeText(displayAddress.address);
      addNotification('success', 'Address copied to clipboard');
    } catch {
      addNotification('error', 'Failed to copy address');
    }
  }

  async function handleGenerateNew() {
    try {
      await generateAddress(null);
      addNotification('success', 'New address generated');
    } catch (err) {
      addNotification('error', `Failed to generate address: ${err}`);
    }
  }
</script>

<div class="receive-page">
  <h2>Receive DOLI</h2>

  {#if displayAddress}
    <div class="address-card">
      <span class="address-label">Your Address</span>
      <code class="address-value">{displayAddress.address}</code>
      <div class="address-actions">
        <button class="btn btn-primary" onclick={handleCopy}>Copy Address</button>
      </div>
    </div>
  {:else}
    <p class="no-address">No addresses available.</p>
  {/if}

  {#if walletState.addresses.length > 1}
    <div class="address-list">
      <h3>All Addresses</h3>
      {#each walletState.addresses as addr}
        <button
          class="address-item"
          class:selected={displayAddress?.address === addr.address}
          onclick={() => selectedAddress = addr}
        >
          <span class="addr-display">{truncateHash(addr.address, 12)}</span>
          {#if addr.label}
            <span class="addr-label">{addr.label}</span>
          {/if}
        </button>
      {/each}
    </div>
  {/if}

  <button class="btn btn-secondary" onclick={handleGenerateNew}>
    Generate New Address
  </button>
</div>

<style>
  .receive-page { padding: 24px; max-width: 560px; }
  h2 { margin: 0 0 24px; }

  .address-card {
    display: flex;
    flex-direction: column;
    padding: 20px;
    background: var(--color-surface, #1a1a2e);
    border: 1px solid var(--color-border, #2d2d4a);
    border-radius: 12px;
    margin-bottom: 20px;
  }

  .address-label {
    font-size: 13px;
    color: var(--color-text-muted, #8888aa);
    margin-bottom: 8px;
  }

  .address-value {
    font-size: 14px;
    word-break: break-all;
    background: var(--color-bg, #0f0f23);
    padding: 12px;
    border-radius: 6px;
    margin-bottom: 12px;
  }

  .address-actions { display: flex; gap: 8px; }

  .address-list {
    margin-bottom: 16px;
  }

  .address-list h3 {
    font-size: 14px;
    margin: 0 0 8px;
  }

  .address-item {
    display: flex;
    justify-content: space-between;
    width: 100%;
    padding: 8px 12px;
    background: var(--color-surface, #1a1a2e);
    border: 1px solid var(--color-border, #2d2d4a);
    border-radius: 6px;
    color: var(--color-text, #e0e0e0);
    cursor: pointer;
    margin-bottom: 4px;
    font-size: 13px;
    font-family: monospace;
  }

  .address-item.selected {
    border-color: var(--color-primary, #6c63ff);
  }

  .addr-label {
    color: var(--color-text-muted, #8888aa);
    font-family: sans-serif;
  }

  .no-address {
    color: var(--color-text-muted, #8888aa);
  }

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
</style>
