/**
 * Network store -- reactive state for connection, chain info, RPC endpoint,
 * and embedded node status.
 * Uses Svelte 5 runes ($state).
 */

import * as networkApi from '../api/network.js';
import { addNotification } from './notifications.js';

export const networkState = $state({
  connected: false,
  network: 'mainnet',
  chainInfo: null,
  rpcUrl: '',
  status: 'disconnected',
  pollInterval: null,
  // Embedded node status
  nodeRunning: false,
  nodeRpcUrl: '',
  nodeLogPath: '',
});

export async function refreshChainInfo() {
  try {
    const info = await networkApi.getChainInfo();
    networkState.chainInfo = info;
    networkState.connected = true;
    networkState.status = 'connected';
  } catch (err) {
    networkState.connected = false;
    networkState.status = 'disconnected';
  }
}

export async function refreshConnectionStatus() {
  try {
    const status = await networkApi.getConnectionStatus();
    networkState.connected = status.connected;
    networkState.status = status.connected ? 'connected' : 'disconnected';
    networkState.rpcUrl = status.rpcUrl || networkState.rpcUrl;
    networkState.network = status.network || networkState.network;
  } catch (err) {
    networkState.connected = false;
    networkState.status = 'disconnected';
  }
}

export async function refreshNodeStatus() {
  try {
    const status = await networkApi.nodeStatus();
    networkState.nodeRunning = status.running;
    networkState.nodeRpcUrl = status.rpcUrl;
    networkState.nodeLogPath = status.logPath;
    networkState.network = status.network || networkState.network;
  } catch (err) {
    networkState.nodeRunning = false;
  }
}

export async function setNetwork(network) {
  try {
    await networkApi.setNetwork(network);
    networkState.network = network;
    addNotification('success', `Switched to ${network}`);
    await refreshNodeStatus();
    await refreshChainInfo();
  } catch (err) {
    addNotification('error', `Failed to switch network: ${err}`);
  }
}

export async function setRpcEndpoint(url) {
  try {
    await networkApi.setRpcEndpoint(url);
    networkState.rpcUrl = url;
    addNotification('success', 'RPC endpoint updated');
    await refreshChainInfo();
  } catch (err) {
    addNotification('error', `Failed to set RPC endpoint: ${err}`);
  }
}

export async function startNode() {
  try {
    await networkApi.startNode();
    networkState.nodeRunning = true;
    addNotification('success', 'Node started');
    await refreshNodeStatus();
  } catch (err) {
    addNotification('error', `Failed to start node: ${err}`);
  }
}

export async function stopNode() {
  try {
    await networkApi.stopNode();
    networkState.nodeRunning = false;
    addNotification('success', 'Node stopped');
  } catch (err) {
    addNotification('error', `Failed to stop node: ${err}`);
  }
}

export function startPolling(intervalMs = 10000) {
  stopPolling();
  refreshChainInfo();
  refreshNodeStatus();
  networkState.pollInterval = setInterval(() => {
    refreshChainInfo();
    refreshNodeStatus();
  }, intervalMs);
}

export function stopPolling() {
  if (networkState.pollInterval) {
    clearInterval(networkState.pollInterval);
    networkState.pollInterval = null;
  }
}
