<script>
  /**
   * Claim reward interface -- claim individual or all rewards.
   */
  let { onNavigate = () => {} } = $props();
</script>

<div class="claim-page">
  <h2>Claim Rewards</h2>
  <p class="description">
    Use the <button class="link" onclick={() => onNavigate('rewards/list')}>Claimable Rewards</button> page to claim individual or all rewards.
  </p>
</div>

<style>
  .claim-page { padding: 24px; }
  h2 { margin: 0 0 12px; }
  .description { color: var(--color-text-muted, #8888aa); }
  .link { background: none; border: none; color: var(--color-primary, #6c63ff); cursor: pointer; text-decoration: underline; font-size: inherit; padding: 0; }
</style>
