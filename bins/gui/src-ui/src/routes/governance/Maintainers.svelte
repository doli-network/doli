<script>
  /**
   * Maintainer list display.
   */
  import LoadingSpinner from '../../../lib/components/LoadingSpinner.svelte';
  import { truncateHash } from '../../../lib/utils/format.js';

  let maintainers = $state([]);
  let loading = $state(false);
  let error = $state(null);

  // Maintainer list would be fetched from chain info
  // Stubbed for now as it requires specific RPC methods
</script>

<div class="maintainers-page">
  <h2>Maintainers</h2>
  <p class="description">Network maintainers who manage protocol updates and governance.</p>

  {#if error}
    <p class="error-text">{error}</p>
  {/if}

  {#if maintainers.length === 0 && !loading}
    <p class="info-text">Maintainer information is available from the chain info endpoint.</p>
  {/if}

  <div class="maintainer-list">
    {#each maintainers as m}
      <div class="maintainer-item">
        <code>{truncateHash(m.pubkey || '', 12)}</code>
      </div>
    {/each}
  </div>
</div>

<style>
  .maintainers-page { padding: 24px; }
  h2 { margin: 0 0 8px; }
  .description { color: var(--color-text-muted, #8888aa); margin: 0 0 24px; }
  .maintainer-list { display: flex; flex-direction: column; gap: 6px; }
  .maintainer-item { padding: 10px 12px; background: var(--color-surface, #1a1a2e); border: 1px solid var(--color-border, #2d2d4a); border-radius: 6px; font-size: 13px; }
  .error-text { color: var(--color-error, #f44336); }
  .info-text { color: var(--color-text-muted, #8888aa); }
</style>
