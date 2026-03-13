<script>
  /**
   * Send DOLI form -- address, amount, confirmation, result.
   */
  import { sendDoli } from '../../../lib/api/transactions.js';
  import { refreshBalance } from '../../../lib/stores/wallet.js';
  import { validateAddress, validateAmount } from '../../../lib/utils/validation.js';
  import AmountInput from '../../../lib/components/AmountInput.svelte';
  import AddressInput from '../../../lib/components/AddressInput.svelte';
  import TxResult from '../../../lib/components/TxResult.svelte';
  import ConfirmDialog from '../../../lib/components/ConfirmDialog.svelte';
  import LoadingSpinner from '../../../lib/components/LoadingSpinner.svelte';
  import { addNotification } from '../../../lib/stores/notifications.js';

  let address = $state('');
  let amount = $state('');
  let loading = $state(false);
  let result = $state(null);
  let error = $state(null);
  let showConfirm = $state(false);

  let addressValid = $derived(validateAddress(address).valid);
  let amountValid = $derived(validateAmount(amount).valid);
  let canSend = $derived(addressValid && amountValid && !loading);

  function handleSendClick() {
    if (!canSend) return;
    showConfirm = true;
  }

  async function handleConfirmSend() {
    showConfirm = false;
    loading = true;
    result = null;
    error = null;
    try {
      const res = await sendDoli(address.trim(), amount.trim());
      result = res;
      addNotification('success', 'Transaction submitted');
      refreshBalance();
    } catch (err) {
      error = String(err);
      addNotification('error', `Send failed: ${err}`);
    } finally {
      loading = false;
    }
  }

  function handleDismiss() {
    result = null;
    error = null;
    address = '';
    amount = '';
  }
</script>

<div class="send-page">
  <h2>Send DOLI</h2>

  {#if result || error}
    <TxResult {result} {error} onDismiss={handleDismiss} />
  {:else}
    <div class="form">
      <AddressInput
        value={address}
        oninput={(v) => address = v}
      />

      <AmountInput
        value={amount}
        oninput={(v) => amount = v}
      />

      <button class="btn btn-primary" disabled={!canSend} onclick={handleSendClick}>
        {#if loading}
          <LoadingSpinner size="16px" label="" />
        {:else}
          Send
        {/if}
      </button>
    </div>
  {/if}

  <ConfirmDialog
    open={showConfirm}
    title="Confirm Transaction"
    message="Send {amount} DOLI to {address}?"
    confirmLabel="Send"
    onConfirm={handleConfirmSend}
    onCancel={() => showConfirm = false}
  />
</div>

<style>
  .send-page { padding: 24px; max-width: 560px; }
  h2 { margin: 0 0 24px; }
  .form { display: flex; flex-direction: column; gap: 16px; }
  .btn {
    padding: 10px 20px;
    border: none;
    border-radius: 6px;
    font-size: 14px;
    cursor: pointer;
    font-weight: 500;
  }
  .btn-primary { background: var(--color-primary, #6c63ff); color: white; }
  .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
