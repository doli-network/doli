<script>
  /**
   * Producer registration form.
   */
  import { registerProducer } from '../../../lib/api/producer.js';
  import { validateAmount } from '../../../lib/utils/validation.js';
  import AmountInput from '../../../lib/components/AmountInput.svelte';
  import TxResult from '../../../lib/components/TxResult.svelte';
  import ConfirmDialog from '../../../lib/components/ConfirmDialog.svelte';
  import LoadingSpinner from '../../../lib/components/LoadingSpinner.svelte';
  import { addNotification } from '../../../lib/stores/notifications.js';

  let bondAmount = $state('');
  let loading = $state(false);
  let result = $state(null);
  let error = $state(null);
  let showConfirm = $state(false);

  let amountValid = $derived(validateAmount(bondAmount).valid);

  async function handleConfirm() {
    showConfirm = false;
    loading = true;
    result = null;
    error = null;
    try {
      const res = await registerProducer(bondAmount.trim());
      result = res;
      addNotification('success', 'Registration submitted');
    } catch (err) {
      error = String(err);
      addNotification('error', `Registration failed: ${err}`);
    } finally {
      loading = false;
    }
  }

  function handleDismiss() { result = null; error = null; bondAmount = ''; }
</script>

<div class="register-page">
  <h2>Register as Producer</h2>
  <p class="description">
    Bond DOLI to register as a block producer. Registration activates after a delay period.
  </p>

  {#if result || error}
    <TxResult {result} {error} onDismiss={handleDismiss} />
  {:else}
    <div class="form">
      <AmountInput
        label="Bond Amount (DOLI)"
        value={bondAmount}
        oninput={(v) => bondAmount = v}
      />
      <button class="btn btn-primary" disabled={!amountValid || loading} onclick={() => showConfirm = true}>
        {#if loading}<LoadingSpinner size="16px" label="" />{:else}Register{/if}
      </button>
    </div>
  {/if}

  <ConfirmDialog
    open={showConfirm}
    title="Confirm Registration"
    message="Bond {bondAmount} DOLI to register as a producer?"
    confirmLabel="Register"
    onConfirm={handleConfirm}
    onCancel={() => showConfirm = false}
  />
</div>

<style>
  .register-page { padding: 24px; max-width: 560px; }
  h2 { margin: 0 0 8px; }
  .description { color: var(--color-text-muted, #8888aa); margin: 0 0 24px; }
  .form { display: flex; flex-direction: column; gap: 16px; }
  .btn { padding: 10px 20px; border: none; border-radius: 6px; font-size: 14px; cursor: pointer; font-weight: 500; }
  .btn-primary { background: var(--color-primary, #6c63ff); color: white; }
  .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
