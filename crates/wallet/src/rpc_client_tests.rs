use super::*;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ========================================================================
// Requirement: GUI-FR-070 (Must) -- Chain info display
// Acceptance: Network name, best hash, height, slot, genesis hash
// ========================================================================

#[tokio::test]
async fn test_fr070_get_chain_info() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("getChainInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": {
                "network": "mainnet",
                "bestHash": "abcdef1234567890",
                "bestHeight": 100000,
                "bestSlot": 200000,
                "genesisHash": "genesis0000000000"
            },
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let info = client.get_chain_info().await.unwrap();
    assert_eq!(info.network, "mainnet");
    assert_eq!(info.best_height, 100000);
    assert_eq!(info.best_slot, 200000);
    assert_eq!(info.genesis_hash, "genesis0000000000");
}

// ========================================================================
// Requirement: GUI-FR-010 (Must) -- Balance display
// Acceptance: Shows spendable, bonded, immature, unconfirmed, per-address
// ========================================================================

#[tokio::test]
async fn test_fr010_get_balance() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("getBalance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": {
                "confirmed": 500_000_000_u64,
                "unconfirmed": 100_000_000_u64,
                "immature": 200_000_000_u64,
                "total": 800_000_000_u64
            },
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let balance = client.get_balance("test_address").await.unwrap();
    assert_eq!(balance.confirmed, 500_000_000);
    assert_eq!(balance.unconfirmed, 100_000_000);
    assert_eq!(balance.immature, 200_000_000);
    assert_eq!(balance.total, 800_000_000);
}

#[tokio::test]
async fn test_fr010_balance_zero() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("getBalance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": {
                "confirmed": 0,
                "unconfirmed": 0,
                "immature": 0,
                "total": 0
            },
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let balance = client.get_balance("empty_address").await.unwrap();
    assert_eq!(balance.confirmed, 0);
    assert_eq!(balance.total, 0);
}

// ========================================================================
// Requirement: GUI-FR-014 (Must) -- Transaction history
// Acceptance: Paginated list with hash, type, amount, fee, height, confirmations
// ========================================================================

#[tokio::test]
async fn test_fr014_get_history() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("getHistory"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": [
                {
                    "hash": "tx_hash_1",
                    "txType": "Transfer",
                    "blockHash": "block_1",
                    "height": 1000,
                    "timestamp": 1700000000_u64,
                    "amountReceived": 500_000_000_u64,
                    "amountSent": 0,
                    "fee": 1000,
                    "confirmations": 50
                },
                {
                    "hash": "tx_hash_2",
                    "txType": "Transfer",
                    "blockHash": "block_2",
                    "height": 1001,
                    "timestamp": 1700000010_u64,
                    "amountReceived": 0,
                    "amountSent": 200_000_000_u64,
                    "fee": 1000,
                    "confirmations": 49
                }
            ],
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let history = client.get_history("test_addr", 10).await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].hash, "tx_hash_1");
    assert_eq!(history[0].amount_received, 500_000_000);
    assert_eq!(history[1].amount_sent, 200_000_000);
}

#[tokio::test]
async fn test_fr014_get_history_empty() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("getHistory"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": [],
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let history = client.get_history("new_addr", 10).await.unwrap();
    assert!(history.is_empty());
}

// ========================================================================
// Requirement: GUI-FR-011 (Must) -- Send transaction
// Acceptance: Construction, signing, validation, tx hash returned
// ========================================================================

#[tokio::test]
async fn test_fr011_send_transaction() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("sendTransaction"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": "tx_hash_abcdef1234567890",
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let tx_hash = client.send_transaction("deadbeef").await.unwrap();
    assert_eq!(tx_hash, "tx_hash_abcdef1234567890");
}

// ========================================================================
// Requirement: GUI-FR-030 (Must) -- Rewards list
// Acceptance: Shows epochs with estimated reward amounts, qualification status
// ========================================================================

#[tokio::test]
async fn test_fr030_get_rewards_list() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("getRewardsList"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": [
                {
                    "epoch": 10,
                    "estimatedReward": 100_000_000_u64,
                    "qualified": true,
                    "claimed": false
                },
                {
                    "epoch": 11,
                    "estimatedReward": 95_000_000_u64,
                    "qualified": true,
                    "claimed": true
                }
            ],
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let rewards = client.get_rewards_list("pubkey_hex").await.unwrap();
    assert_eq!(rewards.len(), 2);
    assert_eq!(rewards[0].epoch, 10);
    assert!(rewards[0].qualified);
    assert!(!rewards[0].claimed);
    assert!(rewards[1].claimed);
}

// ========================================================================
// Requirement: GUI-FR-020, GUI-FR-021 (Must) -- Producer registration, status
// Acceptance: Shows producer info from getProducers RPC
// ========================================================================

#[tokio::test]
async fn test_fr021_get_producers() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("getProducers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": [
                {
                    "publicKey": "abcd1234",
                    "registrationHeight": 500,
                    "bondAmount": 10_000_000_000_u64,
                    "bondCount": 10,
                    "status": "active",
                    "era": 3
                }
            ],
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let producers = client.get_producers().await.unwrap();
    assert_eq!(producers.len(), 1);
    assert_eq!(producers[0].public_key, "abcd1234");
    assert_eq!(producers[0].bond_count, 10);
    assert_eq!(producers[0].status, "active");
}

// ========================================================================
// Requirement: GUI-FR-025 (Must) -- Request withdrawal (simulation)
// ========================================================================

#[tokio::test]
async fn test_fr025_simulate_withdrawal() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("simulateWithdrawal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": {
                "bondCount": 5,
                "totalStaked": 5_000_000_000_u64,
                "totalPenalty": 1_250_000_000_u64,
                "netAmount": 3_750_000_000_u64,
                "bonds": [
                    {
                        "creationSlot": 1000,
                        "amount": 1_000_000_000_u64,
                        "penaltyPct": 50,
                        "penaltyAmount": 500_000_000_u64,
                        "netAmount": 500_000_000_u64
                    }
                ]
            },
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let sim = client.simulate_withdrawal("pubkey", 5).await.unwrap();
    assert_eq!(sim.bond_count, 5);
    assert_eq!(sim.total_penalty, 1_250_000_000);
    assert_eq!(sim.net_amount, 3_750_000_000);
}

// ========================================================================
// Requirement: GUI-FR-080 (Must) -- Public RPC endpoints
// Acceptance: Pre-configured endpoints, fallback on failure
// ========================================================================

#[test]
fn test_fr080_default_mainnet_endpoints() {
    let endpoints = default_endpoints("mainnet");
    assert!(
        endpoints.len() >= 2,
        "Mainnet must have at least 2 fallback endpoints"
    );
    for ep in &endpoints {
        assert!(
            ep.starts_with("https://"),
            "Mainnet endpoints must use HTTPS: {}",
            ep
        );
    }
}

#[test]
fn test_fr080_default_testnet_endpoints() {
    let endpoints = default_endpoints("testnet");
    assert!(!endpoints.is_empty());
}

#[test]
fn test_fr080_default_devnet_endpoints() {
    let endpoints = default_endpoints("devnet");
    assert!(!endpoints.is_empty());
    // Devnet defaults to localhost
    assert!(endpoints[0].contains("127.0.0.1"));
}

#[test]
fn test_fr080_unknown_network_empty() {
    let endpoints = default_endpoints("unknown");
    assert!(endpoints.is_empty());
}

// ========================================================================
// Requirement: GUI-FR-082 (Must) -- Network selector
// Acceptance: Correct address prefix per network
// ========================================================================

#[test]
fn test_fr082_network_prefix_mainnet() {
    assert_eq!(network_prefix("mainnet"), "doli");
}

#[test]
fn test_fr082_network_prefix_testnet() {
    assert_eq!(network_prefix("testnet"), "tdoli");
}

#[test]
fn test_fr082_network_prefix_devnet() {
    assert_eq!(network_prefix("devnet"), "ddoli");
}

#[test]
fn test_fr082_network_prefix_unknown_defaults_mainnet() {
    assert_eq!(network_prefix("unknown"), "doli");
}

// ========================================================================
// Requirement: GUI-FR-081 (Must) -- Custom RPC endpoint
// Acceptance: Connection test, saves to settings
// ========================================================================

#[tokio::test]
async fn test_fr081_test_connection_success() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("getChainInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": {
                "network": "mainnet",
                "bestHash": "abc",
                "bestHeight": 1,
                "bestSlot": 1,
                "genesisHash": "gen"
            },
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let connected = client.test_connection().await.unwrap();
    assert!(connected);
}

#[tokio::test]
async fn test_fr081_test_connection_failure() {
    let client = RpcClient::new("http://127.0.0.1:1"); // port 1 should fail
    let result = client.test_connection().await;
    assert!(result.is_err(), "Connection to invalid endpoint must fail");
}

// ========================================================================
// Requirement: GUI-FR-083 (Must) -- Connection status
// Acceptance: Connected, syncing, or disconnected
// ========================================================================

#[test]
fn test_fr083_rpc_client_stores_url() {
    let client = RpcClient::new("http://example.com:8500");
    assert_eq!(client.url(), "http://example.com:8500");
}

// ========================================================================
// Failure mode tests (from Architecture: RPC endpoint unreachable)
// ========================================================================

#[tokio::test]
async fn test_failure_rpc_unreachable() {
    let client = RpcClient::new("http://127.0.0.1:1");
    let result = client.get_balance("addr").await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Failed to connect") || err.contains("Connection refused"),
        "Error should mention connection failure: {}",
        err
    );
}

#[tokio::test]
async fn test_failure_rpc_malformed_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let result = client.get_chain_info().await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Failed to parse"));
}

#[tokio::test]
async fn test_failure_rpc_error_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "error": {
                "code": -32600,
                "message": "Invalid request"
            },
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let result = client.get_chain_info().await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("RPC error"));
}

#[tokio::test]
async fn test_failure_rpc_http_500() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let result = client.get_chain_info().await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("status"));
}

#[tokio::test]
async fn test_failure_rpc_null_result() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": null,
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let result = client.get_chain_info().await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No result"));
}

// ========================================================================
// Requirement: GUI-FR-034 (Should) -- Epoch info
// ========================================================================

#[tokio::test]
async fn test_fr034_get_epoch_info() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("getEpochInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": {
                "currentEpoch": 100,
                "lastCompleteEpoch": 99,
                "blocksPerEpoch": 360,
                "blocksRemaining": 150,
                "epochStartHeight": 35640,
                "epochEndHeight": 36000,
                "blockReward": 100_000_000_u64
            },
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let epoch = client.get_epoch_info().await.unwrap();
    assert_eq!(epoch.current_epoch, 100);
    assert_eq!(epoch.blocks_per_epoch, 360);
    assert_eq!(epoch.blocks_remaining, 150);
}

// ========================================================================
// Requirement: GUI-FR-022 (Should) -- Bond details
// ========================================================================

#[tokio::test]
async fn test_fr022_get_bond_details() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("getBondDetails"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": {
                "bondCount": 10,
                "totalStaked": 10_000_000_000_u64,
                "summary": {
                    "q1": 2,
                    "q2": 3,
                    "q3": 2,
                    "vested": 3
                },
                "bonds": [
                    {
                        "creationSlot": 100,
                        "amount": 1_000_000_000_u64,
                        "ageSlots": 50000,
                        "penaltyPct": 0,
                        "vested": true,
                        "maturationSlot": 10000
                    }
                ],
                "withdrawalPendingCount": 0,
                "vestingQuarterSlots": 3153600,
                "vestingPeriodSlots": 12614400
            },
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let details = client.get_bond_details("pubkey").await.unwrap();
    assert_eq!(details.bond_count, 10);
    assert_eq!(details.summary.vested, 3);
    assert_eq!(details.bonds.len(), 1);
    assert!(details.bonds[0].vested);
}

// ========================================================================
// UTXO retrieval tests
// ========================================================================

#[tokio::test]
async fn test_get_utxos() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("getUtxos"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": [
                {
                    "txHash": "txhash001",
                    "outputIndex": 0,
                    "amount": 500_000_000_u64,
                    "outputType": "normal",
                    "lockUntil": 0,
                    "height": 1000,
                    "spendable": true
                }
            ],
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = RpcClient::new(&server.uri());
    let utxos = client.get_utxos("addr", true).await.unwrap();
    assert_eq!(utxos.len(), 1);
    assert_eq!(utxos[0].amount, 500_000_000);
    assert!(utxos[0].spendable);
}
