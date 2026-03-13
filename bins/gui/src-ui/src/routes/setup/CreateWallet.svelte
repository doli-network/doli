<script>
  /**
   * Wallet creation -- displays seed phrase for backup.
   */
  import { createWallet } from '../../../lib/stores/wallet.js';
  import { validateWalletName } from '../../../lib/utils/validation.js';
  import LoadingSpinner from '../../../lib/components/LoadingSpinner.svelte';

  let { onNavigate = () => {} } = $props();

  let name = $state('');
  let seedPhrase = $state(null);
  let confirmed = $state(false);
  let loading = $state(false);
  let error = $state(null);

  let nameValidation = $derived(name ? validateWalletName(name) : { valid: false });

  async function handleCreate() {
    if (!nameValidation.valid) return;
    loading = true;
    error = null;
    try {
      const result = await createWallet(name, null);
      seedPhrase = result.seedPhrase;
    } catch (err) {
      error = String(err);
    } finally {
      loading = false;
    }
  }

  function handleConfirmBackup() {
    confirmed = true;
    onNavigate('wallet/overview');
  }
</script>

<div class="create-wallet">
  <h2>Create New Wallet</h2>

  {#if !seedPhrase}
    <div class="form">
      <label class="input-label" for="wallet-name">Wallet Name</label>
      <input
        id="wallet-name"
        type="text"
        bind:value={name}
        placeholder="My Wallet"
        class="input"
      />
      {#if nameValidation.error}
        <p class="error-text">{nameValidation.error}</p>
      {/if}

      {#if error}
        <p class="error-text">{error}</p>
      {/if}

      <button
        class="btn btn-primary"
        disabled={!nameValidation.valid || loading}
        onclick={handleCreate}
      >
        {#if loading}
          <LoadingSpinner size="16px" label="" />
        {:else}
          Create Wallet
        {/if}
      </button>
    </div>
  {:else}
    <div class="seed-display">
      <p class="warning-text">
        Write down these words in order and store them securely.
        This is the ONLY way to recover your wallet.
      </p>
      <div class="seed-words" aria-label="Seed phrase words">
        {#each seedPhrase.split(' ') as word, i}
          <div class="seed-word">
            <span class="word-index">{i + 1}.</span>
            <span class="word-text">{word}</span>
          </div>
        {/each}
      </div>
      <label class="confirm-label">
        <input type="checkbox" bind:checked={confirmed} />
        I have securely backed up my seed phrase
      </label>
      <button
        class="btn btn-primary"
        disabled={!confirmed}
        onclick={handleConfirmBackup}
      >
        Continue to Wallet
      </button>
    </div>
  {/if}

  <button class="btn btn-ghost" onclick={() => onNavigate('setup/welcome')}>
    Back
  </button>
</div>

<style>
  .create-wallet {
    padding: 32px;
    max-width: 560px;
    margin: 0 auto;
  }

  h2 { margin: 0 0 24px; }

  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .input-label {
    font-size: 13px;
    font-weight: 500;
  }

  .input {
    padding: 8px 12px;
    background: var(--color-bg, #0f0f23);
    border: 1px solid var(--color-border, #2d2d4a);
    border-radius: 6px;
    color: var(--color-text, #e0e0e0);
    font-size: 14px;
  }

  .error-text {
    color: var(--color-error, #f44336);
    font-size: 13px;
    margin: 0;
  }

  .warning-text {
    color: var(--color-warning, #ff9800);
    font-size: 14px;
    line-height: 1.5;
    padding: 12px;
    background: rgba(255, 152, 0, 0.1);
    border: 1px solid rgba(255, 152, 0, 0.3);
    border-radius: 6px;
  }

  .seed-display {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .seed-words {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
    padding: 16px;
    background: var(--color-surface, #1a1a2e);
    border-radius: 8px;
  }

  .seed-word {
    display: flex;
    gap: 4px;
    padding: 6px 8px;
    background: var(--color-bg, #0f0f23);
    border-radius: 4px;
    font-family: monospace;
    font-size: 13px;
  }

  .word-index {
    color: var(--color-text-muted, #8888aa);
    min-width: 20px;
  }

  .confirm-label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
    cursor: pointer;
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
