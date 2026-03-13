<script>
  /**
   * Bridge claim -- claim HTLC with preimage.
   */
  import { bridgeClaim } from '../../../lib/api/bridge.js';
  import TxResult from '../../../lib/components/TxResult.svelte';
  import { addNotification } from '../../../lib/stores/notifications.js';

  let utxoRef = $state('');
  let preimage = $state('');
  let loading = $state(false);
  let result = $state(null);
  let error = $state(null);

  async function handleClaim() {
    loading = true;
    result = null; error = null;
    try {
      const res = await bridgeClaim(utxoRef.trim(), preimage.trim());
      result = res;
      addNotification('success', 'Bridge claim submitted');
    } catch (err) {
      error = String(err);
    } finally { loading = false; }
  }

  function handleDismiss() { result = null; error = null; utxoRef = ''; preimage = ''; }
</script>

<div class="claim-page">
  <h2>Bridge Claim</h2>

  {#if result || error}
    <TxResult {result} {error} onDismiss={handleDismiss} />
  {:else}
    <div class="form">
      <div class="field">
        <label class="input-label" for="utxo-ref">HTLC UTXO Reference</label>
        <input id="utxo-ref" type="text" bind:value={utxoRef} placeholder="txhash:index" class="input" spellcheck="false" />
      </div>
      <div class="field">
        <label class="input-label" for="preimage">Preimage (hex)</label>
        <input id="preimage" type="text" bind:value={preimage} placeholder="BLAKE3 preimage hex" class="input" spellcheck="false" />
      </div>
      <button class="btn btn-primary" disabled={!utxoRef.trim() || !preimage.trim() || loading} onclick={handleClaim}>
        Claim
      </button>
    </div>
  {/if}
</div>

<style>
  .claim-page { padding: 24px; max-width: 560px; }
  h2 { margin: 0 0 24px; }
  .form { display: flex; flex-direction: column; gap: 16px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .input-label { font-size: 13px; font-weight: 500; }
  .input { padding: 8px 12px; background: var(--color-bg, #0f0f23); border: 1px solid var(--color-border, #2d2d4a); border-radius: 6px; color: var(--color-text, #e0e0e0); font-size: 14px; font-family: monospace; }
  .btn { padding: 10px 20px; border: none; border-radius: 6px; font-size: 14px; cursor: pointer; font-weight: 500; }
  .btn-primary { background: var(--color-primary, #6c63ff); color: white; }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
