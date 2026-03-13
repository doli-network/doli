/**
 * Wallet API -- Tauri invoke wrappers for wallet commands.
 * All wallet operations happen in the Rust backend.
 * Private keys never cross this boundary (GUI-NF-004).
 */

import { invoke } from '@tauri-apps/api/core';

export async function createWallet(name, walletPath) {
  return invoke('create_wallet', { name, walletPath });
}

export async function restoreWallet(name, seedPhrase, walletPath) {
  return invoke('restore_wallet', { name, seedPhrase, walletPath });
}

export async function loadWallet(walletPath) {
  return invoke('load_wallet', { walletPath });
}

export async function generateAddress(label = null) {
  return invoke('generate_address', { label });
}

export async function listAddresses() {
  return invoke('list_addresses');
}

export async function exportWallet(destination) {
  return invoke('export_wallet', { destination });
}

export async function importWallet(source, destination) {
  return invoke('import_wallet', { source, destination });
}

export async function walletInfo() {
  return invoke('wallet_info');
}

export async function addBlsKey() {
  return invoke('add_bls_key');
}
