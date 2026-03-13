<script>
  /**
   * NFT/Token info lookup.
   */
  import { nftInfo, tokenInfo } from '../../../lib/api/nft.js';
  import { formatBalance } from '../../../lib/utils/format.js';
  import LoadingSpinner from '../../../lib/components/LoadingSpinner.svelte';

  let utxoRef = $state('');
  let lookupType = $state('nft');
  let info = $state(null);
  let loading = $state(false);
  let error = $state(null);

  async function handleLookup() {
    if (!utxoRef.trim()) return;
    loading = true;
    info = null; error = null;
    try {
      if (lookupType === 'nft') {
        info = await nftInfo(utxoRef.trim());
      } else {
        info = await tokenInfo(utxoRef.trim());
      }
    } catch (err) {
      error = String(err);
    } finally { loading = false; }
  }
</script>

<div class="info-page">
  <h2>{lookupType === 'nft' ? 'NFT' : 'Token'} Info</h2>

  <div class="tabs">
    <button class="tab" class:active={lookupType === 'nft'} onclick={() => { lookupType = 'nft'; info = null; }}>NFT</button>
    <button class="tab" class:active={lookupType === 'token'} onclick={() => { lookupType = 'token'; info = null; }}>Token</button>
  </div>

  <div class="form">
    <div class="field">
      <label class="input-label" for="utxo-ref">UTXO Reference</label>
      <input id="utxo-ref" type="text" bind:value={utxoRef} placeholder="txhash:index" class="input" spellcheck="false" />
    </div>
    <button class="btn btn-primary" disabled={!utxoRef.trim() || loading} onclick={handleLookup}>
      {#if loading}<LoadingSpinner size="16px" label="" />{:else}Lookup{/if}
    </button>
  </div>

  {#if error}
    <p class="error-text">{error}</p>
  {/if}

  {#if info}
    <div class="info-card">
      <h3>Details</h3>
      {#each Object.entries(info) as [key, val]}
        <div class="info-row">
          <span class="info-label">{key}:</span>
          <span class="info-value">{typeof val === 'object' ? JSON.stringify(val) : val}</span>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .info-page { padding: 24px; max-width: 560px; }
  h2 { margin: 0 0 16px; }
  .tabs { display: flex; gap: 4px; margin-bottom: 20px; }
  .tab { padding: 8px 16px; background: var(--color-surface-alt, #2d2d4a); border: none; border-radius: 6px; color: var(--color-text, #e0e0e0); cursor: pointer; font-size: 13px; }
  .tab.active { background: var(--color-primary, #6c63ff); color: white; }
  .form { display: flex; flex-direction: column; gap: 12px; margin-bottom: 20px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .input-label { font-size: 13px; font-weight: 500; }
  .input { padding: 8px 12px; background: var(--color-bg, #0f0f23); border: 1px solid var(--color-border, #2d2d4a); border-radius: 6px; color: var(--color-text, #e0e0e0); font-size: 14px; font-family: monospace; }
  .info-card { padding: 16px; background: var(--color-surface, #1a1a2e); border: 1px solid var(--color-border, #2d2d4a); border-radius: 8px; }
  .info-card h3 { margin: 0 0 12px; font-size: 15px; }
  .info-row { display: flex; justify-content: space-between; padding: 4px 0; font-size: 13px; }
  .info-label { color: var(--color-text-muted, #8888aa); }
  .info-value { font-family: monospace; word-break: break-all; }
  .error-text { color: var(--color-error, #f44336); }
  .btn { padding: 10px 20px; border: none; border-radius: 6px; font-size: 14px; cursor: pointer; font-weight: 500; }
  .btn-primary { background: var(--color-primary, #6c63ff); color: white; }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
