/**
 * Bridge API -- Tauri invoke wrappers for HTLC bridge operations.
 */

import { invoke } from '@tauri-apps/api/core';

export async function bridgeLock(params) {
  return invoke('bridge_lock', { params });
}

export async function bridgeClaim(utxoRef, preimage) {
  return invoke('bridge_claim', { utxoRef, preimage });
}

export async function bridgeRefund(utxoRef) {
  return invoke('bridge_refund', { utxoRef });
}
