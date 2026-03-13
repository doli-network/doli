<script>
  /**
   * Address input with bech32m/hex validation feedback.
   * Props: value (string), label (string), placeholder (string),
   *        error (string|null), disabled (boolean), oninput (function)
   */
  import { validateAddress } from '../utils/validation.js';

  let {
    value = '',
    label = 'Recipient Address',
    placeholder = 'doli1... or hex pubkey hash',
    error = null,
    disabled = false,
    oninput = () => {},
  } = $props();

  let validationError = $derived(
    value ? validateAddress(value).error || null : null
  );

  let displayError = $derived(error || validationError);
</script>

<div class="address-input-group">
  <label class="input-label" for="address-input">{label}</label>
  <div class="input-wrapper" class:has-error={displayError}>
    <input
      id="address-input"
      type="text"
      {placeholder}
      {disabled}
      {value}
      oninput={(e) => oninput(e.target.value)}
      class="input"
      spellcheck="false"
      autocomplete="off"
      aria-invalid={displayError ? 'true' : undefined}
      aria-describedby={displayError ? 'address-error' : undefined}
    />
  </div>
  {#if displayError}
    <p id="address-error" class="input-error" role="alert">{displayError}</p>
  {/if}
</div>

<style>
  .address-input-group {
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
    width: 100%;
    padding: 8px 12px;
    background: none;
    border: none;
    color: var(--color-text, #e0e0e0);
    font-size: 14px;
    font-family: monospace;
    outline: none;
  }

  .input-error {
    margin: 0;
    font-size: 12px;
    color: var(--color-error, #f44336);
  }
</style>
