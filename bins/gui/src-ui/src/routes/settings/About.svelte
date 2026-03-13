<script>
  /**
   * About page -- version info, links.
   */
</script>

<div class="about-page">
  <h2>About DOLI Wallet</h2>

  <div class="info-card">
    <div class="info-row"><span>Application:</span><span>DOLI Desktop Wallet</span></div>
    <div class="info-row"><span>Version:</span><span>3.4.4</span></div>
    <div class="info-row"><span>Framework:</span><span>Tauri 2.x + Svelte 5</span></div>
    <div class="info-row"><span>License:</span><span>MIT</span></div>
  </div>

  <div class="links">
    <h3>Links</h3>
    <ul>
      <li>Website: doli.network</li>
      <li>Source: github.com/doli-network</li>
      <li>Documentation: docs.doli.network</li>
    </ul>
  </div>
</div>

<style>
  .about-page { padding: 24px; max-width: 560px; }
  h2 { margin: 0 0 24px; }

  .info-card { padding: 16px; background: var(--color-surface, #1a1a2e); border: 1px solid var(--color-border, #2d2d4a); border-radius: 8px; margin-bottom: 24px; }
  .info-row { display: flex; justify-content: space-between; padding: 6px 0; font-size: 14px; }
  .info-row span:first-child { color: var(--color-text-muted, #8888aa); }

  .links { margin-bottom: 24px; }
  h3 { margin: 0 0 8px; font-size: 15px; }
  ul { padding-left: 20px; }
  li { padding: 4px 0; font-size: 14px; color: var(--color-text-muted, #8888aa); }
</style>
