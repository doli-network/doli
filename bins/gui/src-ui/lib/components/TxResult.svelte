<script>
  /**
   * Transaction result display (success/error after submission).
   * Props: result (object|null), error (string|null), onDismiss (function)
   */
  import { truncateHash } from '../utils/format.js';

  let {
    result = null,
    error = null,
    onDismiss = () => {},
  } = $props();
</script>

{#if result}
  <div class="tx-result success" role="alert">
    <h3 class="result-title">Transaction Submitted</h3>
    {#if result.txHash}
      <div class="result-field">
        <span class="result-label">Transaction Hash:</span>
        <code class="result-value" title={result.txHash}>{truncateHash(result.txHash, 12)}</code>
      </div>
    {/if}
    {#if result.message}
      <p class="result-message">{result.message}</p>
    {/if}
    <button class="btn btn-secondary" onclick={onDismiss}>Dismiss</button>
  </div>
{/if}

{#if error}
  <div class="tx-result error" role="alert">
    <h3 class="result-title">Transaction Failed</h3>
    <p class="result-message">{error}</p>
    <button class="btn btn-secondary" onclick={onDismiss}>Dismiss</button>
  </div>
{/if}

<style>
  .tx-result {
    padding: 16px;
    border-radius: 8px;
    margin: 12px 0;
  }

  .tx-result.success {
    background: rgba(76, 175, 80, 0.1);
    border: 1px solid rgba(76, 175, 80, 0.3);
  }

  .tx-result.error {
    background: rgba(244, 67, 54, 0.1);
    border: 1px solid rgba(244, 67, 54, 0.3);
  }

  .result-title {
    margin: 0 0 8px;
    font-size: 16px;
  }

  .success .result-title {
    color: var(--color-success, #4caf50);
  }

  .error .result-title {
    color: var(--color-error, #f44336);
  }

  .result-field {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
  }

  .result-label {
    font-size: 13px;
    color: var(--color-text-muted, #8888aa);
  }

  .result-value {
    font-size: 13px;
    color: var(--color-text, #e0e0e0);
    background: var(--color-bg, #0f0f23);
    padding: 2px 6px;
    border-radius: 4px;
  }

  .result-message {
    margin: 0 0 12px;
    font-size: 13px;
    color: var(--color-text-muted, #8888aa);
    line-height: 1.5;
  }

  .btn {
    padding: 6px 12px;
    border: none;
    border-radius: 6px;
    font-size: 13px;
    cursor: pointer;
  }

  .btn-secondary {
    background: var(--color-surface-alt, #2d2d4a);
    color: var(--color-text, #e0e0e0);
  }
</style>
