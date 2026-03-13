<script>
  /**
   * Loading spinner indicator.
   * Props: size (string), label (string)
   */
  let { size = '24px', label = 'Loading...' } = $props();
</script>

<div class="spinner-container" role="status" aria-label={label}>
  <div class="spinner" style="width: {size}; height: {size};" aria-hidden="true"></div>
  {#if label}
    <span class="spinner-label">{label}</span>
  {/if}
</div>

<style>
  .spinner-container {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .spinner {
    border: 2px solid var(--color-border, #2d2d4a);
    border-top-color: var(--color-primary, #6c63ff);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .spinner-label {
    font-size: 13px;
    color: var(--color-text-muted, #8888aa);
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
