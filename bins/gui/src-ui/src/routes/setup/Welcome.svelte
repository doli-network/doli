<script>
  /**
   * Welcome screen -- first-run: create or restore wallet.
   */
  let { onNavigate = () => {} } = $props();
</script>

<div class="welcome">
  <div class="welcome-content">
    <h1 class="welcome-title">Welcome to DOLI</h1>
    <p class="welcome-subtitle">Secure blockchain wallet for the DOLI network</p>

    <div class="welcome-actions">
      <button class="btn btn-primary btn-large" onclick={() => onNavigate('setup/create')}>
        Create New Wallet
      </button>
      <button class="btn btn-secondary btn-large" onclick={() => onNavigate('setup/restore')}>
        Restore from Seed Phrase
      </button>
      <button class="btn btn-ghost btn-large" onclick={() => onNavigate('setup/load')}>
        Load Existing Wallet
      </button>
    </div>
  </div>
</div>

<style>
  .welcome {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    padding: 32px;
  }

  .welcome-content {
    text-align: center;
    max-width: 420px;
  }

  .welcome-title {
    font-size: 32px;
    font-weight: 700;
    margin: 0 0 8px;
    color: var(--color-primary, #6c63ff);
  }

  .welcome-subtitle {
    font-size: 16px;
    color: var(--color-text-muted, #8888aa);
    margin: 0 0 40px;
  }

  .welcome-actions {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .btn {
    padding: 12px 24px;
    border: none;
    border-radius: 8px;
    font-size: 16px;
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

  .btn-ghost {
    background: transparent;
    color: var(--color-text-muted, #8888aa);
    border: 1px solid var(--color-border, #2d2d4a);
  }
</style>
