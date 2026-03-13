<script>
  /**
   * Exit producer -- remove producer registration.
   */
  import { exitProducer } from '../../../lib/api/producer.js';
  import TxResult from '../../../lib/components/TxResult.svelte';
  import ConfirmDialog from '../../../lib/components/ConfirmDialog.svelte';
  import { addNotification } from '../../../lib/stores/notifications.js';

  let loading = $state(false);
  let result = $state(null);
  let error = $state(null);
  let showConfirm = $state(false);

  async function handleExit() {
    showConfirm = false;
    loading = true;
    result = null; error = null;
    try {
      const res = await exitProducer();
      result = res;
      addNotification('success', 'Exit submitted');
    } catch (err) {
      error = String(err);
    } finally { loading = false; }
  }

  function handleDismiss() { result = null; error = null; }
</script>

<div class="exit-page">
  <h2>Exit Producer</h2>
  <p class="warning-text">
    Exiting will remove your producer registration. Bonded funds will be returned
    after the vesting delay, subject to any applicable penalty.
  </p>

  {#if result || error}
    <TxResult {result} {error} onDismiss={handleDismiss} />
  {:else}
    <button class="btn btn-danger" disabled={loading} onclick={() => showConfirm = true}>
      Exit Producer
    </button>
  {/if}

  <ConfirmDialog
    open={showConfirm}
    title="Confirm Exit"
    message="Are you sure you want to exit as a producer? This cannot be undone easily."
    confirmLabel="Exit"
    danger={true}
    onConfirm={handleExit}
    onCancel={() => showConfirm = false}
  />
</div>

<style>
  .exit-page { padding: 24px; max-width: 560px; }
  h2 { margin: 0 0 12px; }
  .warning-text { color: var(--color-warning, #ff9800); line-height: 1.5; padding: 12px; background: rgba(255, 152, 0, 0.1); border: 1px solid rgba(255, 152, 0, 0.3); border-radius: 6px; margin-bottom: 20px; }
  .btn { padding: 10px 20px; border: none; border-radius: 6px; font-size: 14px; cursor: pointer; font-weight: 500; }
  .btn-danger { background: var(--color-error, #f44336); color: white; }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
