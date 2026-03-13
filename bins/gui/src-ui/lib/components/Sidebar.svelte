<script>
  /**
   * Navigation sidebar with collapsible sections.
   * Props: currentView (string), onNavigate (function)
   */
  let { currentView = 'wallet/overview', onNavigate = () => {} } = $props();

  const sections = [
    {
      label: 'Wallet',
      items: [
        { id: 'wallet/overview', label: 'Overview' },
        { id: 'wallet/send', label: 'Send' },
        { id: 'wallet/receive', label: 'Receive' },
        { id: 'wallet/history', label: 'History' },
        { id: 'wallet/addresses', label: 'Addresses' },
      ],
    },
    {
      label: 'Producer',
      items: [
        { id: 'producer/dashboard', label: 'Dashboard' },
        { id: 'producer/register', label: 'Register' },
        { id: 'producer/bonds', label: 'Bonds' },
        { id: 'producer/producers', label: 'Network' },
        { id: 'producer/exit', label: 'Exit' },
      ],
    },
    {
      label: 'Rewards',
      items: [
        { id: 'rewards/list', label: 'Claimable' },
        { id: 'rewards/history', label: 'History' },
      ],
    },
    {
      label: 'Assets',
      items: [
        { id: 'nft/mint', label: 'NFTs' },
        { id: 'nft/info', label: 'Tokens' },
        { id: 'bridge/lock', label: 'Bridge' },
      ],
    },
    {
      label: 'Governance',
      items: [
        { id: 'governance/updates', label: 'Updates' },
        { id: 'governance/vote', label: 'Vote' },
      ],
    },
    {
      label: 'Settings',
      items: [
        { id: 'settings/network', label: 'Network' },
        { id: 'settings/wallet', label: 'Wallet' },
        { id: 'settings/about', label: 'About' },
      ],
    },
  ];
</script>

<nav class="sidebar" aria-label="Main navigation">
  <div class="sidebar-header">
    <h1 class="app-title">DOLI Wallet</h1>
  </div>
  <div class="sidebar-content">
    {#each sections as section}
      <div class="nav-section">
        <h2 class="nav-section-label">{section.label}</h2>
        <ul class="nav-list">
          {#each section.items as item}
            <li>
              <button
                class="nav-item"
                class:active={currentView === item.id}
                onclick={() => onNavigate(item.id)}
                aria-current={currentView === item.id ? 'page' : undefined}
              >
                {item.label}
              </button>
            </li>
          {/each}
        </ul>
      </div>
    {/each}
  </div>
</nav>

<style>
  .sidebar {
    width: 200px;
    background: var(--color-surface, #1a1a2e);
    border-right: 1px solid var(--color-border, #2d2d4a);
    display: flex;
    flex-direction: column;
    overflow-y: auto;
  }

  .sidebar-header {
    padding: 16px;
    border-bottom: 1px solid var(--color-border, #2d2d4a);
  }

  .app-title {
    font-size: 18px;
    font-weight: 700;
    margin: 0;
    color: var(--color-primary, #6c63ff);
  }

  .sidebar-content {
    flex: 1;
    padding: 8px 0;
  }

  .nav-section {
    margin-bottom: 8px;
  }

  .nav-section-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--color-text-muted, #8888aa);
    padding: 8px 16px 4px;
    margin: 0;
  }

  .nav-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .nav-item {
    display: block;
    width: 100%;
    padding: 6px 16px 6px 24px;
    background: none;
    border: none;
    color: var(--color-text, #e0e0e0);
    font-size: 13px;
    text-align: left;
    cursor: pointer;
    border-radius: 0;
    transition: background-color 0.15s;
  }

  .nav-item:hover {
    background: var(--color-hover, rgba(108, 99, 255, 0.1));
  }

  .nav-item.active {
    background: var(--color-active, rgba(108, 99, 255, 0.2));
    color: var(--color-primary, #6c63ff);
    font-weight: 600;
  }
</style>
