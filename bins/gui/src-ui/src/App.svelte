<script>
  import { invoke } from '@tauri-apps/api/core';

  // Simple client-side routing state
  let currentRoute = $state('dashboard');
  let walletLoaded = $state(false);
  let walletInfo = $state(null);
  let connectionStatus = $state({ connected: false, status: 'disconnected' });
  let chainHeight = $state(null);
  let error = $state(null);

  // Check for existing wallet on mount
  async function checkWallet() {
    try {
      const info = await invoke('wallet_info');
      walletInfo = info;
      walletLoaded = true;
    } catch {
      walletLoaded = false;
      currentRoute = 'setup';
    }
  }

  // Poll connection status
  async function pollStatus() {
    try {
      const status = await invoke('get_connection_status');
      connectionStatus = status;
      chainHeight = status.chainHeight;
    } catch {
      connectionStatus = { connected: false, status: 'disconnected' };
    }
  }

  function navigate(route) {
    currentRoute = route;
    error = null;
  }

  // Navigation sections
  const navSections = [
    {
      title: 'Wallet',
      items: [
        { id: 'dashboard', label: 'Overview' },
        { id: 'send', label: 'Send' },
        { id: 'receive', label: 'Receive' },
        { id: 'history', label: 'History' },
        { id: 'addresses', label: 'Addresses' },
      ]
    },
    {
      title: 'Producer',
      items: [
        { id: 'producer', label: 'Dashboard' },
        { id: 'bonds', label: 'Bonds' },
        { id: 'producers', label: 'Network' },
      ]
    },
    {
      title: 'Rewards',
      items: [
        { id: 'rewards', label: 'Claimable' },
      ]
    },
    {
      title: 'Assets',
      items: [
        { id: 'nft', label: 'NFTs' },
        { id: 'bridge', label: 'Bridge' },
      ]
    },
    {
      title: 'System',
      items: [
        { id: 'governance', label: 'Governance' },
        { id: 'settings', label: 'Settings' },
      ]
    }
  ];
</script>

<div id="app-root">
  {#if !walletLoaded}
    <!-- Setup flow: create or restore wallet -->
    <div class="setup-container">
      <h1>DOLI Wallet</h1>
      <p>Create a new wallet or restore from seed phrase.</p>
      <div class="setup-actions">
        <button class="btn btn-primary" onclick={() => navigate('create')}>
          Create New Wallet
        </button>
        <button class="btn" onclick={() => navigate('restore')}>
          Restore from Seed Phrase
        </button>
        <button class="btn" onclick={() => navigate('import')}>
          Import Wallet File
        </button>
      </div>
    </div>
  {:else}
    <!-- Main app layout -->
    <div class="layout">
      <nav class="sidebar">
        <div style="padding: 16px; font-weight: 700; font-size: 16px;">
          DOLI Wallet
        </div>
        {#each navSections as section}
          <div class="nav-section">
            <div class="nav-section-title">{section.title}</div>
            {#each section.items as item}
              <button
                class="nav-item"
                class:active={currentRoute === item.id}
                onclick={() => navigate(item.id)}
              >
                {item.label}
              </button>
            {/each}
          </div>
        {/each}
      </nav>

      <main class="content">
        <!-- Route content rendered here -->
        <div class="route-content">
          {#if currentRoute === 'dashboard'}
            <h2>Wallet Overview</h2>
            {#if walletInfo}
              <div class="card">
                <div class="balance-label">Total Balance</div>
                <div class="balance-amount">Loading...</div>
              </div>
              <div class="card">
                <div class="card-title">Wallet Info</div>
                <p>Name: {walletInfo.name}</p>
                <p class="mono">Address: {walletInfo.bech32Address}</p>
              </div>
            {/if}
          {:else if currentRoute === 'settings'}
            <h2>Settings</h2>
            <div class="card">
              <div class="card-title">Network</div>
              <p>Connected to: {connectionStatus.network || 'unknown'}</p>
              <p>Endpoint: {connectionStatus.endpoint || 'none'}</p>
            </div>
          {:else}
            <h2>{currentRoute}</h2>
            <p>View coming soon.</p>
          {/if}
        </div>
      </main>
    </div>

    <footer class="statusbar">
      <span>
        <span class="status-dot" class:status-connected={connectionStatus.connected}
          class:status-disconnected={!connectionStatus.connected}></span>
        {connectionStatus.connected ? 'Connected' : 'Disconnected'}
      </span>
      {#if chainHeight}
        <span>Height: {chainHeight.toLocaleString()}</span>
      {/if}
      <span>{connectionStatus.network || 'mainnet'}</span>
    </footer>
  {/if}
</div>

<style>
  .setup-container {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100vh;
    gap: 16px;
  }
  .setup-actions {
    display: flex;
    gap: 12px;
    margin-top: 16px;
  }
  #app-root {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
</style>
