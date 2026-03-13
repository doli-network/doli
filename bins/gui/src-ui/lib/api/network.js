/**
 * Network API -- Tauri invoke wrappers for chain info, RPC management,
 * and embedded node control.
 */

import { invoke } from '@tauri-apps/api/core';

// ---------- Chain / RPC ----------

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

// ---------- Embedded Node ----------

export async function startNode() {
  return invoke('start_node');
}

export async function stopNode() {
  return invoke('stop_node');
}

export async function nodeStatus() {
  return invoke('node_status');
}

export async function restartNode(network) {
  return invoke('restart_node', { network: network || null });
}

export async function getNodeLogs(lines = 100) {
  return invoke('get_node_logs', { lines });
}
