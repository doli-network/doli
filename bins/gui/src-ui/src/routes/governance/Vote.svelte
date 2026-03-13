<script>
  /**
   * Vote on governance updates.
   */
  import { voteUpdate } from '../../../lib/api/governance.js';
  import TxResult from '../../../lib/components/TxResult.svelte';
  import ConfirmDialog from '../../../lib/components/ConfirmDialog.svelte';
  import { addNotification } from '../../../lib/stores/notifications.js';

  let version = $state('');
  let approve = $state(true);
  let loading = $state(false);
  let result = $state(null);
  let error = $state(null);
  let showConfirm = $state(false);

  async function handleVote() {
    showConfirm = false;
    loading = true;
    result = null; error = null;
    try {
      const res = await voteUpdate(version.trim(), approve);
      result = res;
      addNotification('success', `Vote submitted: ${approve ? 'Approve' : 'Reject'}`);
    } catch (err) {
      error = String(err);
    } finally { loading = false; }
  }

  function handleDismiss() { result = null; error = null; version = ''; }
</script>

<div class="vote-page">
  <h2>Vote on Update</h2>

  {#if result || error}
    <TxResult {result} {error} onDismiss={handleDismiss} />
  {:else}
    <div class="form">
      <div class="field">
        <label class="input-label" for="version">Version</label>
        <input id="version" type="text" bind:value={version} placeholder="e.g. 3.5.0" class="input" />
      </div>

      <div class="vote-options">
        <label class="vote-option">
          <input type="radio" bind:group={approve} value={true} /> Approve
        </label>
        <label class="vote-option">
          <input type="radio" bind:group={approve} value={false} /> Reject
        </label>
      </div>

      <button class="btn btn-primary" disabled={!version.trim() || loading} onclick={() => showConfirm = true}>
        Submit Vote
      </button>
    </div>
  {/if}

  <ConfirmDialog
    open={showConfirm}
    title="Confirm Vote"
    message="Vote to {approve ? 'approve' : 'reject'} version {version}?"
    confirmLabel="Submit"
    onConfirm={handleVote}
    onCancel={() => showConfirm = false}
  />
</div>

<style>
  .vote-page { padding: 24px; max-width: 560px; }
  h2 { margin: 0 0 24px; }
  .form { display: flex; flex-direction: column; gap: 16px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .input-label { font-size: 13px; font-weight: 500; }
  .input { padding: 8px 12px; background: var(--color-bg, #0f0f23); border: 1px solid var(--color-border, #2d2d4a); border-radius: 6px; color: var(--color-text, #e0e0e0); font-size: 14px; }
  .vote-options { display: flex; gap: 20px; }
  .vote-option { display: flex; align-items: center; gap: 6px; cursor: pointer; font-size: 14px; }
  .btn { padding: 10px 20px; border: none; border-radius: 6px; font-size: 14px; cursor: pointer; font-weight: 500; }
  .btn-primary { background: var(--color-primary, #6c63ff); color: white; }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
