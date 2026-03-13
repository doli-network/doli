/**
 * Transaction API -- Tauri invoke wrappers for balance, send, and history.
 */

import { invoke } from '@tauri-apps/api/core';

export async function getBalance(address = null) {
  return invoke('get_balance', { address });
}

export async function sendDoli(to, amount, fee = null) {
  return invoke('send_doli', { to, amount, fee });
}

export async function getHistory(limit = 50) {
  return invoke('get_history', { limit });
}
