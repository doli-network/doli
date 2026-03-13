<script>
  /**
   * Bridge lock -- initiate HTLC bridge lock.
   */
  import { bridgeLock } from '../../../lib/api/bridge.js';
  import AmountInput from '../../../lib/components/AmountInput.svelte';
  import AddressInput from '../../../lib/components/AddressInput.svelte';
  import TxResult from '../../../lib/components/TxResult.svelte';
  import ConfirmDialog from '../../../lib/components/ConfirmDialog.svelte';
  import { validateAmount, validateAddress } from '../../../lib/utils/validation.js';
  import { addNotification } from '../../../lib/stores/notifications.js';

  let recipient = $state('');
  let amount = $state('');
  let hashlock = $state('');
  let timeout = $state('3600');
  let loading = $state(false);
  let result = $state(null);
  let error = $state(null);
  let showConfirm = $state(false);

  let addressValid = $derived(validateAddress(recipient).valid);
  let amountValid = $derived(validateAmount(amount).valid);

  async function handleLock() {
    showConfirm = false;
    loading = true;
    result = null; error = null;
    try {
      const res = await bridgeLock({
        recipient: recipient.trim(),
        amount: amount.trim(),
        hashlock: hashlock.trim(),
        timeoutSeconds: parseInt(timeout, 10),
      });
      result = res;
      addNotification('success', 'Bridge lock created');
    } catch (err) {
      error = String(err);
    } finally { loading = false; }
  }

  function handleDismiss() { result = null; error = null; }
</script>

<div class="lock-page">
  <h2>Bridge Lock (HTLC)</h2>

  {#if result || error}
    <TxResult {result} {error} onDismiss={handleDismiss} />
  {:else}
    <div class="form">
      <AddressInput value={recipient} oninput={(v) => recipient = v} label="Recipient" />
      <AmountInput value={amount} oninput={(v) => amount = v} />

      <div class="field">
        <label class="input-label" for="hashlock">Hashlock (hex)</label>
        <input id="hashlock" type="text" bind:value={hashlock} placeholder="BLAKE3 hash hex" class="input" spellcheck="false" />
      </div>

      <div class="field">
        <label class="input-label" for="timeout">Timeout (seconds)</label>
        <input id="timeout" type="number" bind:value={timeout} class="input" min="60" />
      </div>

      <button class="btn btn-primary" disabled={!addressValid || !amountValid || !hashlock.trim() || loading} onclick={() => showConfirm = true}>
        Create Lock
      </button>
    </div>
  {/if}

  <ConfirmDialog
    open={showConfirm}
    title="Confirm Bridge Lock"
    message="Lock {amount} DOLI in HTLC bridge contract?"
    confirmLabel="Lock"
    onConfirm={handleLock}
    onCancel={() => showConfirm = false}
  />
</div>

<style>
  .lock-page { padding: 24px; max-width: 560px; }
  h2 { margin: 0 0 24px; }
  .form { display: flex; flex-direction: column; gap: 16px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .input-label { font-size: 13px; font-weight: 500; }
  .input { padding: 8px 12px; background: var(--color-bg, #0f0f23); border: 1px solid var(--color-border, #2d2d4a); border-radius: 6px; color: var(--color-text, #e0e0e0); font-size: 14px; font-family: monospace; }
  .btn { padding: 10px 20px; border: none; border-radius: 6px; font-size: 14px; cursor: pointer; font-weight: 500; }
  .btn-primary { background: var(--color-primary, #6c63ff); color: white; }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
