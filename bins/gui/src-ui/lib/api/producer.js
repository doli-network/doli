/**
 * Producer API -- Tauri invoke wrappers for producer management.
 */

import { invoke } from '@tauri-apps/api/core';

export async function producerStatus() {
  return invoke('producer_status');
}

export async function registerProducer(bondCount) {
  return invoke('register_producer', { bondCount });
}

export async function addBonds(count) {
  return invoke('add_bonds', { count });
}

export async function requestWithdrawal(bondCount, dest = null) {
  return invoke('request_withdrawal', { bondCount, dest });
}

export async function simulateWithdrawal(bondCount) {
  return invoke('simulate_withdrawal', { bondCount });
}

export async function exitProducer(force = false) {
  return invoke('exit_producer', { force });
}
