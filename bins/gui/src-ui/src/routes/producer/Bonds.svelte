<script>
  /**
   * Bond management -- add bonds, request withdrawal, simulate withdrawal.
   */
  import { addBonds, requestWithdrawal, simulateWithdrawal } from '../../../lib/api/producer.js';
  import { validateAmount } from '../../../lib/utils/validation.js';
  import AmountInput from '../../../lib/components/AmountInput.svelte';
  import TxResult from '../../../lib/components/TxResult.svelte';
  import ConfirmDialog from '../../../lib/components/ConfirmDialog.svelte';
  import { addNotification } from '../../../lib/stores/notifications.js';
  import { formatBalance } from '../../../lib/utils/format.js';

  let tab = $state('add');
  let amount = $state('');
  let loading = $state(false);
  let result = $state(null);
  let error = $state(null);
  let showConfirm = $state(false);
  let simulation = $state(null);

  let amountValid = $derived(validateAmount(amount).valid);

  async function handleAddBonds() {
    showConfirm = false;
    loading = true;
    result = null; error = null;
    try {
      const res = await addBonds(amount.trim());
      result = res;
      addNotification('success', 'Bonds added');
    } catch (err) {
      error = String(err);
    } finally { loading = false; }
  }

  async function handleWithdraw() {
    showConfirm = false;
    loading = true;
    result = null; error = null;
    try {
      const res = await requestWithdrawal(amount.trim());
      result = res;
      addNotification('success', 'Withdrawal requested');
    } catch (err) {
      error = String(err);
    } finally { loading = false; }
  }

  async function handleSimulate() {
    loading = true;
    simulation = null;
    try {
      simulation = await simulateWithdrawal(amount.trim());
    } catch (err) {
      addNotification('error', `Simulation failed: ${err}`);
    } finally { loading = false; }
  }

  function handleDismiss() { result = null; error = null; amount = ''; simulation = null; }
</script>

<div class="bonds-page">
  <h2>Bond Management</h2>

  <div class="tabs">
    <button class="tab" class:active={tab === 'add'} onclick={() => { tab = 'add'; handleDismiss(); }}>Add Bonds</button>
    <button class="tab" class:active={tab === 'withdraw'} onclick={() => { tab = 'withdraw'; handleDismiss(); }}>Withdraw</button>
  </div>

  {#if result || error}
    <TxResult {result} {error} onDismiss={handleDismiss} />
  {:else}
    <div class="form">
      <AmountInput value={amount} oninput={(v) => amount = v} label="{tab === 'add' ? 'Bond' : 'Withdraw'} Amount (DOLI)" />

      {#if tab === 'add'}
        <button class="btn btn-primary" disabled={!amountValid || loading} onclick={() => showConfirm = true}>Add Bonds</button>
      {:else}
        <div class="action-row">
          <button class="btn btn-secondary" disabled={!amountValid || loading} onclick={handleSimulate}>Simulate</button>
          <button class="btn btn-primary" disabled={!amountValid || loading} onclick={() => showConfirm = true}>Request Withdrawal</button>
        </div>
      {/if}

      {#if simulation}
        <div class="simulation-result">
          <h4>Simulation Result</h4>
          <div class="sim-row"><span>Gross:</span><span>{formatBalance(simulation.grossAmount || 0)}</span></div>
          <div class="sim-row"><span>Penalty:</span><span>{formatBalance(simulation.penaltyAmount || 0)}</span></div>
          <div class="sim-row"><span>Net:</span><span>{formatBalance(simulation.netAmount || 0)}</span></div>
        </div>
      {/if}
    </div>
  {/if}

  <ConfirmDialog
    open={showConfirm}
    title={tab === 'add' ? 'Confirm Add Bonds' : 'Confirm Withdrawal'}
    message={tab === 'add' ? `Add ${amount} DOLI in bonds?` : `Request withdrawal of ${amount} DOLI?`}
    confirmLabel={tab === 'add' ? 'Add' : 'Withdraw'}
    danger={tab === 'withdraw'}
    onConfirm={tab === 'add' ? handleAddBonds : handleWithdraw}
    onCancel={() => showConfirm = false}
  />
</div>

<style>
  .bonds-page { padding: 24px; max-width: 560px; }
  h2 { margin: 0 0 16px; }
  .tabs { display: flex; gap: 4px; margin-bottom: 20px; }
  .tab { padding: 8px 16px; background: var(--color-surface-alt, #2d2d4a); border: none; border-radius: 6px; color: var(--color-text, #e0e0e0); cursor: pointer; font-size: 13px; }
  .tab.active { background: var(--color-primary, #6c63ff); color: white; }
  .form { display: flex; flex-direction: column; gap: 16px; }
  .action-row { display: flex; gap: 8px; }
  .simulation-result { padding: 12px; background: var(--color-surface, #1a1a2e); border: 1px solid var(--color-border, #2d2d4a); border-radius: 8px; }
  .simulation-result h4 { margin: 0 0 8px; font-size: 14px; }
  .sim-row { display: flex; justify-content: space-between; font-size: 13px; padding: 4px 0; font-family: monospace; }
  .btn { padding: 10px 20px; border: none; border-radius: 6px; font-size: 14px; cursor: pointer; font-weight: 500; }
  .btn-primary { background: var(--color-primary, #6c63ff); color: white; }
  .btn-secondary { background: var(--color-surface-alt, #2d2d4a); color: var(--color-text, #e0e0e0); }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
