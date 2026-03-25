//! JSON-RPC request and response types
//!
//! Organized into domain modules:
//! - [`protocol`] — JSON-RPC 2.0 wire format (request/response envelopes)
//! - [`block`] — Block and transaction responses, covenant condition rendering
//! - [`chain`] — Chain info, network, balance, UTXO, mempool, stats, backfill
//! - [`producer`] — Producer, bond, schedule, epoch, governance

mod block;
mod chain;
mod producer;
mod protocol;

pub use block::*;
pub use chain::*;
pub use producer::*;
pub use protocol::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_epoch_info_response_serialization() {
        let response = EpochInfoResponse {
            current_height: 1500,
            current_epoch: 4,
            last_complete_epoch: Some(3),
            blocks_per_epoch: 360,
            blocks_remaining: 60,
            epoch_start_height: 1440,
            epoch_end_height: 1800,
            block_reward: 5_000_000_000,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"blocksPerEpoch\":360"));
        assert!(json.contains("\"lastCompleteEpoch\":3"));

        let parsed: EpochInfoResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.current_epoch, 4);
        assert_eq!(parsed.blocks_remaining, 60);
    }

    #[test]
    fn test_chain_info_response_serialization() {
        let response = ChainInfoResponse {
            network: "mainnet".to_string(),
            version: "1.1.11".to_string(),
            best_hash: "abc123".to_string(),
            best_height: 50_000,
            best_slot: 50_100,
            genesis_hash: "genesis".to_string(),
            reward_pool_balance: 500_000_000,
        };

        let json = serde_json::to_string(&response).unwrap();
        let parsed: ChainInfoResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.network, "mainnet");
        assert_eq!(parsed.best_height, 50_000);
        assert_eq!(parsed.best_slot, 50_100);
    }

    #[test]
    fn test_balance_response_serialization() {
        let response = BalanceResponse {
            confirmed: 1_000_000,
            unconfirmed: 500,
            immature: 0,
            bonded: 0,
            total: 1_000_500,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"confirmed\":1000000"));

        let parsed: BalanceResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.total, 1_000_500);
    }

    #[test]
    fn test_producer_response_with_withdrawals() {
        let response = ProducerResponse {
            public_key: "aabbcc".to_string(),
            address_hash: "ddeeff".to_string(),
            registration_height: 100,
            bond_amount: 1_000_000_000,
            bond_count: 1,
            status: "unbonding".to_string(),
            era: 0,
            pending_withdrawals: vec![PendingWithdrawalResponse {
                bond_count: 1,
                request_slot: 5000,
                net_amount: 1_000_000_000,
                claimable: false,
            }],
            pending_updates: Vec::new(),
            bls_pubkey: String::new(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"pendingWithdrawals\""));
        assert!(json.contains("\"claimable\":false"));

        let parsed: ProducerResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.pending_withdrawals.len(), 1);
        assert!(!parsed.pending_withdrawals[0].claimable);
    }

    #[test]
    fn test_producer_response_empty_withdrawals() {
        let response = ProducerResponse {
            public_key: "aabbcc".to_string(),
            address_hash: "ddeeff".to_string(),
            registration_height: 100,
            bond_amount: 1_000_000_000,
            bond_count: 1,
            status: "active".to_string(),
            era: 0,
            pending_withdrawals: Vec::new(),
            pending_updates: Vec::new(),
            bls_pubkey: String::new(),
        };

        let json = serde_json::to_string(&response).unwrap();
        let parsed: ProducerResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.pending_withdrawals.is_empty());
    }

    #[test]
    fn test_mempool_info_response() {
        let response = MempoolInfoResponse {
            tx_count: 42,
            total_size: 1024,
            min_fee_rate: 1,
            max_size: 10_485_760,
            max_count: 5000,
        };

        let json = serde_json::to_string(&response).unwrap();
        let parsed: MempoolInfoResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tx_count, 42);
        assert_eq!(parsed.max_count, 5000);
    }

    #[test]
    fn test_json_rpc_response_success() {
        let resp =
            JsonRpcResponse::success(serde_json::json!(1), serde_json::json!({"height": 100}));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"height\":100"));
        assert!(!json.contains("error"));
    }

    #[test]
    fn test_json_rpc_response_error() {
        let err = crate::error::RpcError::invalid_params("bad param");
        let resp = JsonRpcResponse::error(serde_json::json!(1), err);
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"error\""));
        assert!(json.contains("bad param"));
    }

    #[test]
    fn test_history_entry_response() {
        let entry = HistoryEntryResponse {
            hash: "txhash123".to_string(),
            tx_type: "transfer".to_string(),
            block_hash: "blockhash".to_string(),
            height: 1000,
            timestamp: 1700000000,
            amount_received: 5000,
            amount_sent: 0,
            fee: 100,
            confirmations: 50,
        };

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: HistoryEntryResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.amount_received, 5000);
        assert_eq!(parsed.confirmations, 50);
    }
}
