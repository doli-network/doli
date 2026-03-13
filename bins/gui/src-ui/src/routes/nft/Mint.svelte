<script>
  /**
   * NFT minting form.
   */
  import { mintNft } from '../../../lib/api/nft.js';
  import TxResult from '../../../lib/components/TxResult.svelte';
  import AmountInput from '../../../lib/components/AmountInput.svelte';
  import { addNotification } from '../../../lib/stores/notifications.js';

  let content = $state('');
  let value = $state('');
  let loading = $state(false);
  let result = $state(null);
  let error = $state(null);

  async function handleMint() {
    loading = true;
    result = null; error = null;
    try {
      const res = await mintNft(content, value || null);
      result = res;
      addNotification('success', 'NFT minted');
    } catch (err) {
      error = String(err);
    } finally { loading = false; }
  }

  function handleDismiss() { result = null; error = null; content = ''; value = ''; }
</script>

<div class="mint-page">
  <h2>Mint NFT</h2>

  {#if result || error}
    <TxResult {result} {error} onDismiss={handleDismiss} />
  {:else}
    <div class="form">
      <div class="field">
        <label class="input-label" for="nft-content">Content</label>
        <textarea
          id="nft-content"
          bind:value={content}
          placeholder="NFT content (text, JSON, or hex-encoded data)"
          class="textarea"
          rows="4"
        ></textarea>
      </div>

      <AmountInput
        label="Value (DOLI, optional)"
        value={value}
        oninput={(v) => value = v}
        placeholder="0.00000000"
      />

      <button class="btn btn-primary" disabled={!content.trim() || loading} onclick={handleMint}>
        Mint NFT
      </button>
    </div>
  {/if}
</div>

<style>
  .mint-page { padding: 24px; max-width: 560px; }
  h2 { margin: 0 0 24px; }
  .form { display: flex; flex-direction: column; gap: 16px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .input-label { font-size: 13px; font-weight: 500; }
  .textarea { padding: 8px 12px; background: var(--color-bg, #0f0f23); border: 1px solid var(--color-border, #2d2d4a); border-radius: 6px; color: var(--color-text, #e0e0e0); font-size: 14px; resize: vertical; }
  .btn { padding: 10px 20px; border: none; border-radius: 6px; font-size: 14px; cursor: pointer; font-weight: 500; }
  .btn-primary { background: var(--color-primary, #6c63ff); color: white; }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
