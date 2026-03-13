/**
 * Network API -- Tauri invoke wrappers for chain info and RPC management.
 */

import { invoke } from '@tauri-apps/api/core';

export async function getChainInfo() {
  return invoke('get_chain_info');
}

export async function setRpcEndpoint(url) {
  return invoke('set_rpc_endpoint', { url });
}

export async function setNetwork(network) {
  return invoke('set_network', { network });
}

export async function testConnection(url) {
  return invoke('test_connection', { url });
}

export async function getConnectionStatus() {
  return invoke('get_connection_status');
}
