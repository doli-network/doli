/**
 * Governance API -- Tauri invoke wrappers for governance and signing.
 */

import { invoke } from '@tauri-apps/api/core';

export async function checkUpdates() {
  return invoke('check_updates');
}

export async function updateStatus() {
  return invoke('update_status');
}

export async function voteUpdate(version, approve) {
  return invoke('vote_update', { version, approve });
}

export async function signMessage(message, address = null) {
  return invoke('sign_message', { message, address });
}

export async function verifySignature(message, signature, pubkey) {
  return invoke('verify_signature', { message, signature, pubkey });
}
