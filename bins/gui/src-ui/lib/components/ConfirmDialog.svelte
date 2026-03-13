<script>
  /**
   * Reusable confirmation dialog.
   * Props: open (boolean), title (string), message (string), confirmLabel (string),
   *        onConfirm (function), onCancel (function)
   */
  let {
    open = false,
    title = 'Confirm',
    message = 'Are you sure?',
    confirmLabel = 'Confirm',
    cancelLabel = 'Cancel',
    danger = false,
    onConfirm = () => {},
    onCancel = () => {},
  } = $props();

  function handleKeydown(event) {
    if (event.key === 'Escape') {
      onCancel();
    }
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="dialog-backdrop" role="dialog" aria-modal="true" aria-labelledby="dialog-title" onkeydown={handleKeydown}>
    <div class="dialog-content">
      <h2 id="dialog-title" class="dialog-title">{title}</h2>
      <p class="dialog-message">{message}</p>
      <div class="dialog-actions">
        <button class="btn btn-secondary" onclick={onCancel}>
          {cancelLabel}
        </button>
        <button
          class="btn"
          class:btn-danger={danger}
          class:btn-primary={!danger}
          onclick={onConfirm}
        >
          {confirmLabel}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .dialog-content {
    background: var(--color-surface, #1a1a2e);
    border: 1px solid var(--color-border, #2d2d4a);
    border-radius: 8px;
    padding: 24px;
    min-width: 320px;
    max-width: 480px;
  }

  .dialog-title {
    margin: 0 0 12px;
    font-size: 18px;
  }

  .dialog-message {
    margin: 0 0 20px;
    color: var(--color-text-muted, #8888aa);
    line-height: 1.5;
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .btn {
    padding: 8px 16px;
    border: none;
    border-radius: 6px;
    font-size: 14px;
    cursor: pointer;
    font-weight: 500;
  }

  .btn-primary {
    background: var(--color-primary, #6c63ff);
    color: white;
  }

  .btn-secondary {
    background: var(--color-surface-alt, #2d2d4a);
    color: var(--color-text, #e0e0e0);
  }

  .btn-danger {
    background: var(--color-error, #f44336);
    color: white;
  }
</style>
