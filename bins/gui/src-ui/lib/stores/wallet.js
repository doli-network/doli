/**
 * Wallet store -- reactive state for wallet info, addresses, balance.
 * Uses Svelte 5 runes ($state).
 */

import * as walletApi from '../api/wallet.js';
import * as txApi from '../api/transactions.js';
import { addNotification } from './notifications.js';

export const walletState = $state({
  loaded: false,
  info: null,
  addresses: [],
  balance: null,
  loading: false,
  error: null,
});

export async function loadWallet(path) {
  walletState.loading = true;
  walletState.error = null;
  try {
    const info = await walletApi.loadWallet(path);
    walletState.info = info;
    walletState.loaded = true;
    const addrs = await walletApi.listAddresses();
    walletState.addresses = addrs;
  } catch (err) {
    walletState.error = String(err);
    addNotification('error', `Failed to load wallet: ${err}`);
  } finally {
    walletState.loading = false;
  }
}

export async function createWallet(name, path) {
  walletState.loading = true;
  walletState.error = null;
  try {
    const result = await walletApi.createWallet(name, path);
    walletState.info = result.wallet;
    walletState.loaded = true;
    return result;
  } catch (err) {
    walletState.error = String(err);
    addNotification('error', `Failed to create wallet: ${err}`);
    throw err;
  } finally {
    walletState.loading = false;
  }
}

export async function restoreWallet(name, seedPhrase, path) {
  walletState.loading = true;
  walletState.error = null;
  try {
    const info = await walletApi.restoreWallet(name, seedPhrase, path);
    walletState.info = info;
    walletState.loaded = true;
    return info;
  } catch (err) {
    walletState.error = String(err);
    addNotification('error', `Failed to restore wallet: ${err}`);
    throw err;
  } finally {
    walletState.loading = false;
  }
}

export async function refreshAddresses() {
  try {
    const addrs = await walletApi.listAddresses();
    walletState.addresses = addrs;
  } catch (err) {
    addNotification('error', `Failed to refresh addresses: ${err}`);
  }
}

export async function refreshBalance() {
  try {
    const balance = await txApi.getBalance();
    walletState.balance = balance;
  } catch (err) {
    // Silently fail for polling -- user sees stale data
  }
}

export async function generateAddress(label) {
  try {
    const address = await walletApi.generateAddress(label || null);
    await refreshAddresses();
    addNotification('success', 'New address generated');
    return address;
  } catch (err) {
    addNotification('error', `Failed to generate address: ${err}`);
    throw err;
  }
}

export function resetWallet() {
  walletState.loaded = false;
  walletState.info = null;
  walletState.addresses = [];
  walletState.balance = null;
  walletState.loading = false;
  walletState.error = null;
}
