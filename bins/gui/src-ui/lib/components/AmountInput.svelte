<script>
  /**
   * DOLI amount input with validation feedback.
   * Props: value (string), label (string), placeholder (string),
   *        error (string|null), disabled (boolean), oninput (function)
   */
  import { validateAmount } from '../utils/validation.js';

  let {
    value = '',
    label = 'Amount (DOLI)',
    placeholder = '0.00000000',
    error = null,
    disabled = false,
    oninput = () => {},
  } = $props();

  let validationError = $derived(
    value ? validateAmount(value).error || null : null
  );

  let displayError = $derived(error || validationError);
</script>

<div class="amount-input-group">
  <label class="input-label" for="amount-input">{label}</label>
  <div class="input-wrapper" class:has-error={displayError}>
    <input
      id="amount-input"
      type="text"
      inputmode="decimal"
      {placeholder}
      {disabled}
      {value}
      oninput={(e) => oninput(e.target.value)}
      class="input"
      aria-invalid={displayError ? 'true' : undefined}
      aria-describedby={displayError ? 'amount-error' : undefined}
    />
    <span class="input-suffix">DOLI</span>
  </div>
  {#if displayError}
    <p id="amount-error" class="input-error" role="alert">{displayError}</p>
  {/if}
</div>

<style>
  .amount-input-group {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .input-label {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text, #e0e0e0);
  }

  .input-wrapper {
    display: flex;
    align-items: center;
    border: 1px solid var(--color-border, #2d2d4a);
    border-radius: 6px;
    background: var(--color-bg, #0f0f23);
    overflow: hidden;
  }

  .input-wrapper:focus-within {
    border-color: var(--color-primary, #6c63ff);
  }

  .input-wrapper.has-error {
    border-color: var(--color-error, #f44336);
  }

  .input {
    flex: 1;
    padding: 8px 12px;
    background: none;
    border: none;
    color: var(--color-text, #e0e0e0);
    font-size: 14px;
    font-family: monospace;
    outline: none;
  }

  .input-suffix {
    padding: 8px 12px;
    font-size: 13px;
    color: var(--color-text-muted, #8888aa);
    background: var(--color-surface-alt, #2d2d4a);
  }

  .input-error {
    margin: 0;
    font-size: 12px;
    color: var(--color-error, #f44336);
  }
</style>
