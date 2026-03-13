/**
 * NFT/Token API -- Tauri invoke wrappers for NFT and token operations.
 */

import { invoke } from '@tauri-apps/api/core';

export async function mintNft(content, value = null) {
  return invoke('mint_nft', { content, value });
}

export async function transferNft(utxoRef, to) {
  return invoke('transfer_nft', { utxoRef, to });
}

export async function nftInfo(utxoRef) {
  return invoke('nft_info', { utxoRef });
}

export async function issueToken(ticker, supply) {
  return invoke('issue_token', { ticker, supply });
}

export async function tokenInfo(utxoRef) {
  return invoke('token_info', { utxoRef });
}
