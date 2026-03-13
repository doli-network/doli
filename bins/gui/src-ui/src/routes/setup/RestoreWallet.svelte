<script>
  /**
   * Wallet restoration from seed phrase.
   */
  import { restoreWallet } from '../../../lib/stores/wallet.js';
  import { validateWalletName, validateSeedPhrase } from '../../../lib/utils/validation.js';
  import LoadingSpinner from '../../../lib/components/LoadingSpinner.svelte';

  let { onNavigate = () => {} } = $props();

  let name = $state('');
  let seedPhrase = $state('');
  let loading = $state(false);
  let error = $state(null);

  let nameValidation = $derived(name ? validateWalletName(name) : { valid: false });
  let phraseValidation = $derived(seedPhrase ? validateSeedPhrase(seedPhrase) : { valid: false });
  let canSubmit = $derived(nameValidation.valid && phraseValidation.valid && !loading);

  async function handleRestore() {
    if (!canSubmit) return;
    loading = true;
    error = null;
    try {
      await restoreWallet(name, seedPhrase, null);
      onNavigate('wallet/overview');
    } catch (err) {
      error = String(err);
    } finally {
      loading = false;
    }
  }
</script>

<div class="restore-wallet">
  <h2>Restore Wallet</h2>
  <p class="description">Enter your seed phrase to restore an existing wallet.</p>

  <div class="form">
    <div class="field">
      <label class="input-label" for="wallet-name">Wallet Name</label>
      <input id="wallet-name" type="text" bind:value={name} placeholder="My Wallet" class="input" />
      {#if nameValidation.error}
        <p class="error-text">{nameValidation.error}</p>
      {/if}
    </div>

    <div class="field">
      <label class="input-label" for="seed-phrase">Seed Phrase (12 or 24 words)</label>
      <textarea
        id="seed-phrase"
        bind:value={seedPhrase}
        placeholder="Enter your seed phrase words separated by spaces..."
        class="textarea"
        rows="4"
        spellcheck="false"
        autocomplete="off"
      ></textarea>
      {#if phraseValidation.error}
        <p class="error-text">{phraseValidation.error}</p>
      {/if}
    </div>

    {#if error}
      <p class="error-text">{error}</p>
    {/if}

    <button class="btn btn-primary" disabled={!canSubmit} onclick={handleRestore}>
      {#if loading}
        <LoadingSpinner size="16px" label="" />
      {:else}
        Restore Wallet
      {/if}
    </button>
  </div>

  <button class="btn btn-ghost" onclick={() => onNavigate('setup/welcome')}>
    Back
  </button>
</div>

<style>
  .restore-wallet {
    padding: 32px;
    max-width: 560px;
    margin: 0 auto;
  }

  h2 { margin: 0 0 8px; }

  .description {
    color: var(--color-text-muted, #8888aa);
    margin: 0 0 24px;
  }

  .form {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .input-label {
    font-size: 13px;
    font-weight: 500;
  }

  .input, .textarea {
    padding: 8px 12px;
    background: var(--color-bg, #0f0f23);
    border: 1px solid var(--color-border, #2d2d4a);
    border-radius: 6px;
    color: var(--color-text, #e0e0e0);
    font-size: 14px;
    font-family: monospace;
  }

  .textarea {
    resize: vertical;
  }

  .error-text {
    color: var(--color-error, #f44336);
    font-size: 13px;
    margin: 0;
  }

  .btn {
    padding: 10px 20px;
    border: none;
    border-radius: 6px;
    font-size: 14px;
    cursor: pointer;
    font-weight: 500;
  }

  .btn-primary {
    background: var(--color-primary, #6c63ff);
    color: white;
  }

  .btn-primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-ghost {
    background: transparent;
    color: var(--color-text-muted, #8888aa);
    margin-top: 12px;
  }
</style>
