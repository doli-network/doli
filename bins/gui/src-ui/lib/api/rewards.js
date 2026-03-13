/**
 * Rewards API -- Tauri invoke wrappers for reward listing and claiming.
 */

import { invoke } from '@tauri-apps/api/core';

export async function listRewards() {
  return invoke('list_rewards');
}

export async function claimReward(epoch, recipient = null) {
  return invoke('claim_reward', { epoch, recipient });
}

export async function claimAllRewards() {
  return invoke('claim_all_rewards');
}
