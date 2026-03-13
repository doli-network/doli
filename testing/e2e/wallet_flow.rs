//! Wallet Flow End-to-End Test
//!
//! Tests complete wallet operations: create, send, receive

#[path = "../common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::sync::Arc;

use common::{
    create_coinbase, create_test_block, create_transfer, generate_test_chain,
    init_test_logging, TestNode, TestNodeConfig,
};
use doli_core::{
    Block, Transaction, Output,
    consensus::ConsensusParams,
    types::{coins_to_units, units_to_coins},
};
use crypto::{Hash, KeyPair, signature};
use storage::{Outpoint, UtxoEntry};
use tempfile::TempDir;

/// Simulated wallet for testing
struct TestWallet {
    keypair: KeyPair,
    pubkey_hash: Hash,
    utxos: HashMap<Outpoint, UtxoEntry>,
}

impl TestWallet {
    fn new() -> Self {
        let keypair = KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(keypair.public_key().as_bytes());

        Self {
            keypair,
            pubkey_hash,
            utxos: HashMap::new(),
        }
    }

    fn address(&self) -> Hash {
        self.pubkey_hash
    }

    fn balance(&self) -> u64 {
        self.utxos.values().map(|u| u.output.amount).sum()
    }

    fn add_utxo(&mut self, outpoint: Outpoint, entry: UtxoEntry) {
        self.utxos.insert(outpoint, entry);
    }

    fn remove_utxo(&mut self, outpoint: &Outpoint) {
        self.utxos.remove(outpoint);
    }

    fn select_utxos_for_amount(&self, target: u64) -> Option<Vec<(Outpoint, u64)>> {
        let mut selected = Vec::new();
        let mut total = 0u64;

        for (outpoint, entry) in &self.utxos {
            if total >= target {
                break;
            }
            selected.push((outpoint.clone(), entry.output.amount));
            total += entry.output.amount;
        }

        if total >= target {
            Some(selected)
        } else {
            None
        }
    }

    fn create_send_transaction(
        &mut self,
        recipient: &Hash,
        amount: u64,
        fee: u64,
    ) -> Option<Transaction> {
        let total_needed = amount + fee;
        let selected = self.select_utxos_for_amount(total_needed)?;

        let total_input: u64 = selected.iter().map(|(_, a)| *a).sum();
        let change = total_input - amount - fee;

        let inputs: Vec<(Hash, u32)> = selected
            .iter()
            .map(|(op, _)| (op.tx_hash, op.index))
            .collect();

        let mut outputs = vec![(amount, *recipient)];
        if change > 0 {
            outputs.push((change, self.pubkey_hash));
        }

        let tx = create_transfer(inputs, outputs, &self.keypair);

        // Remove spent UTXOs from wallet
        for (outpoint, _) in &selected {
            self.remove_utxo(outpoint);
        }

        Some(tx)
    }
}

/// Basic wallet creation and usage
#[tokio::test]
async fn test_wallet_basic_flow() {
    init_test_logging();

    // Create wallet
    let wallet = TestWallet::new();

    assert_eq!(wallet.balance(), 0);
    assert!(!wallet.address().is_zero());

    println!("Wallet address: {:?}", wallet.address());
}

/// Test receiving coins from coinbase
#[tokio::test]
async fn test_wallet_receive_coinbase() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 0);
    let node = Arc::new(TestNode::new(config));

    let mut wallet = TestWallet::new();
    let producer = KeyPair::generate();
    let params = ConsensusParams::mainnet();

    // Wallet is the recipient of block rewards
    let reward = params.block_reward(0);
    let coinbase = Transaction::new_coinbase(reward, wallet.address(), 0);
    let coinbase_hash = coinbase.hash();

    let block = create_test_block(0, Hash::ZERO, producer.public_key(), vec![coinbase]);
    node.add_block(block).await.unwrap();

    // Update wallet UTXO set
    let outpoint = Outpoint {
        tx_hash: coinbase_hash,
        index: 0,
    };
    let entry = UtxoEntry {
        output: Output::normal(reward, wallet.address()),
        height: 0,
        is_coinbase: true,
    };
    wallet.add_utxo(outpoint, entry);

    assert_eq!(wallet.balance(), reward);
    println!("Wallet balance: {} DOLI", units_to_coins(wallet.balance()));
}

/// Test sending coins to another wallet
#[tokio::test]
async fn test_wallet_send_coins() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 10);
    let node = Arc::new(TestNode::new(config));

    let mut sender_wallet = TestWallet::new();
    let mut recipient_wallet = TestWallet::new();
    let producer = KeyPair::generate();
    let params = ConsensusParams::mainnet();

    // Give sender some coins via coinbase
    let initial_amount = coins_to_units(100);
    let coinbase = Transaction::new_coinbase(initial_amount, sender_wallet.address(), 0);
    let coinbase_hash = coinbase.hash();

    let block = create_test_block(0, Hash::ZERO, producer.public_key(), vec![coinbase]);
    node.add_block(block).await.unwrap();

    // Update sender wallet
    let outpoint = Outpoint {
        tx_hash: coinbase_hash,
        index: 0,
    };
    let entry = UtxoEntry {
        output: Output::normal(initial_amount, sender_wallet.address()),
        height: 0,
        is_coinbase: true,
    };
    sender_wallet.add_utxo(outpoint, entry);

    assert_eq!(sender_wallet.balance(), initial_amount);
    assert_eq!(recipient_wallet.balance(), 0);

    // Send 50 DOLI to recipient (with 0.001 DOLI fee)
    let send_amount = coins_to_units(50);
    let fee = 100_000; // 0.001 DOLI

    let send_tx = sender_wallet
        .create_send_transaction(&recipient_wallet.address(), send_amount, fee)
        .unwrap();

    let send_tx_hash = send_tx.hash();

    // Include transaction in next block
    let coinbase2 = Transaction::new_coinbase(params.block_reward(1), crypto::hash::hash(producer.public_key().as_bytes()), 1);
    let block2 = create_test_block(
        1,
        node.best_hash().await,
        producer.public_key(),
        vec![coinbase2, send_tx.clone()],
    );
    node.add_block(block2).await.unwrap();

    // Update recipient wallet
    let recipient_outpoint = Outpoint {
        tx_hash: send_tx_hash,
        index: 0,
    };
    let recipient_entry = UtxoEntry {
        output: Output::normal(send_amount, recipient_wallet.address()),
        height: 1,
        is_coinbase: false,
    };
    recipient_wallet.add_utxo(recipient_outpoint, recipient_entry);

    // Update sender with change
    if send_tx.outputs.len() > 1 {
        let change_outpoint = Outpoint {
            tx_hash: send_tx_hash,
            index: 1,
        };
        let change_entry = UtxoEntry {
            output: send_tx.outputs[1].clone(),
            height: 1,
            is_coinbase: false,
        };
        sender_wallet.add_utxo(change_outpoint, change_entry);
    }

    println!("Sender balance: {} DOLI", units_to_coins(sender_wallet.balance()));
    println!("Recipient balance: {} DOLI", units_to_coins(recipient_wallet.balance()));

    // Verify balances
    assert_eq!(recipient_wallet.balance(), send_amount);
    assert_eq!(sender_wallet.balance(), initial_amount - send_amount - fee);
}

/// Test multiple transactions in sequence
#[tokio::test]
async fn test_wallet_multiple_transactions() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 20);
    let node = Arc::new(TestNode::new(config));

    let mut alice = TestWallet::new();
    let mut bob = TestWallet::new();
    let mut charlie = TestWallet::new();
    let producer = KeyPair::generate();
    let params = ConsensusParams::mainnet();

    // Give Alice initial funds
    let initial = coins_to_units(1000);
    let coinbase = Transaction::new_coinbase(initial, alice.address(), 0);
    let coinbase_hash = coinbase.hash();

    let block = create_test_block(0, Hash::ZERO, producer.public_key(), vec![coinbase]);
    node.add_block(block).await.unwrap();

    let outpoint = Outpoint { tx_hash: coinbase_hash, index: 0 };
    alice.add_utxo(outpoint, UtxoEntry {
        output: Output::normal(initial, alice.address()),
        height: 0,
        is_coinbase: true,
    });

    // Transaction 1: Alice → Bob (300 DOLI)
    let tx1 = alice
        .create_send_transaction(&bob.address(), coins_to_units(300), 100_000)
        .unwrap();
    let tx1_hash = tx1.hash();

    // Transaction 2 will be Alice → Charlie (200 DOLI) using change from tx1
    // First, we need to track Alice's change
    let alice_change = tx1.outputs.get(1).map(|o| o.amount).unwrap_or(0);
    if alice_change > 0 {
        alice.add_utxo(
            Outpoint { tx_hash: tx1_hash, index: 1 },
            UtxoEntry {
                output: tx1.outputs[1].clone(),
                height: 1,
                is_coinbase: false,
            },
        );
    }

    let tx2 = alice
        .create_send_transaction(&charlie.address(), coins_to_units(200), 100_000)
        .unwrap();
    let tx2_hash = tx2.hash();

    // Include both transactions
    let coinbase2 = Transaction::new_coinbase(params.block_reward(1), crypto::hash::hash(producer.public_key().as_bytes()), 1);
    let block2 = create_test_block(
        1,
        node.best_hash().await,
        producer.public_key(),
        vec![coinbase2, tx1.clone(), tx2.clone()],
    );
    node.add_block(block2).await.unwrap();

    // Update Bob's wallet
    bob.add_utxo(
        Outpoint { tx_hash: tx1_hash, index: 0 },
        UtxoEntry {
            output: Output::normal(coins_to_units(300), bob.address()),
            height: 1,
            is_coinbase: false,
        },
    );

    // Update Charlie's wallet
    charlie.add_utxo(
        Outpoint { tx_hash: tx2_hash, index: 0 },
        UtxoEntry {
            output: Output::normal(coins_to_units(200), charlie.address()),
            height: 1,
            is_coinbase: false,
        },
    );

    // Update Alice's remaining change
    if tx2.outputs.len() > 1 {
        alice.add_utxo(
            Outpoint { tx_hash: tx2_hash, index: 1 },
            UtxoEntry {
                output: tx2.outputs[1].clone(),
                height: 1,
                is_coinbase: false,
            },
        );
    }

    println!("Alice: {} DOLI", units_to_coins(alice.balance()));
    println!("Bob: {} DOLI", units_to_coins(bob.balance()));
    println!("Charlie: {} DOLI", units_to_coins(charlie.balance()));

    assert_eq!(bob.balance(), coins_to_units(300));
    assert_eq!(charlie.balance(), coins_to_units(200));
}

/// Test wallet balance tracking across reorg
#[tokio::test]
async fn test_wallet_reorg_handling() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 30);
    let node = Arc::new(TestNode::new(config));

    let mut wallet = TestWallet::new();
    let producer = KeyPair::generate();

    // Initial chain: wallet receives in block 1
    let genesis = generate_test_chain(1, &producer, 5_000_000_000)[0].clone();
    node.add_block(genesis.clone()).await.unwrap();

    let amount = coins_to_units(100);
    let coinbase = Transaction::new_coinbase(amount, wallet.address(), 1);
    let coinbase_hash = coinbase.hash();

    let block1 = create_test_block(1, genesis.hash(), producer.public_key(), vec![coinbase]);
    node.add_block(block1.clone()).await.unwrap();

    // Wallet tracks UTXO
    let outpoint = Outpoint { tx_hash: coinbase_hash, index: 0 };
    wallet.add_utxo(outpoint.clone(), UtxoEntry {
        output: Output::normal(amount, wallet.address()),
        height: 1,
        is_coinbase: true,
    });

    assert_eq!(wallet.balance(), amount);

    // Continue chain to block 3
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());
    let coinbase2 = create_coinbase(2, &pubkey_hash, 5_000_000_000);
    let block2 = create_test_block(2, block1.hash(), producer.public_key(), vec![coinbase2]);
    node.add_block(block2.clone()).await.unwrap();

    let coinbase3 = create_coinbase(3, &pubkey_hash, 5_000_000_000);
    let block3 = create_test_block(3, block2.hash(), producer.public_key(), vec![coinbase3]);
    node.add_block(block3).await.unwrap();

    assert_eq!(node.height().await, 3);
    assert_eq!(wallet.balance(), amount);

    // REORG: Revert to genesis, build alternative chain where wallet doesn't receive
    node.revert_blocks(3).await.unwrap();
    assert_eq!(node.height().await, 0);

    // Wallet should update - UTXO is now invalid
    wallet.remove_utxo(&outpoint);
    assert_eq!(wallet.balance(), 0);

    // Build alternative chain (wallet not included)
    let mut prev = genesis.hash();
    for i in 1..5 {
        let cb = create_coinbase(i, &pubkey_hash, 5_000_000_000);
        let blk = create_test_block(i, prev, producer.public_key(), vec![cb]);
        prev = blk.hash();
        node.add_block(blk).await.unwrap();
    }

    assert_eq!(node.height().await, 4);
    assert_eq!(wallet.balance(), 0); // Wallet didn't receive in alternative chain
}

/// Test spending immediately after receiving
#[tokio::test]
async fn test_wallet_immediate_spend() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 40);
    let node = Arc::new(TestNode::new(config));

    let mut sender = TestWallet::new();
    let recipient = TestWallet::new();
    let producer = KeyPair::generate();

    // Receive in block 0
    let amount = coins_to_units(50);
    let coinbase = Transaction::new_coinbase(amount, sender.address(), 0);
    let coinbase_hash = coinbase.hash();

    let block0 = create_test_block(0, Hash::ZERO, producer.public_key(), vec![coinbase]);
    node.add_block(block0.clone()).await.unwrap();

    sender.add_utxo(
        Outpoint { tx_hash: coinbase_hash, index: 0 },
        UtxoEntry {
            output: Output::normal(amount, sender.address()),
            height: 0,
            is_coinbase: true,
        },
    );

    // Immediately create spend in block 1
    // (Note: in real system, coinbase has maturity requirement)
    let send_tx = sender
        .create_send_transaction(&recipient.address(), coins_to_units(40), 100_000)
        .unwrap();

    let coinbase1 = create_coinbase(1, &crypto::hash::hash(producer.public_key().as_bytes()), 5_000_000_000);
    let block1 = create_test_block(
        1,
        block0.hash(),
        producer.public_key(),
        vec![coinbase1, send_tx],
    );

    node.add_block(block1).await.unwrap();

    // Sender should have change
    // Recipient should have received amount
    println!("Transaction included in block 1");
}

/// Test wallet with many small UTXOs (consolidation scenario)
#[tokio::test]
async fn test_wallet_utxo_consolidation() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 50);
    let node = Arc::new(TestNode::new(config));

    let mut wallet = TestWallet::new();
    let producer = KeyPair::generate();

    // Receive many small amounts across multiple blocks
    let mut prev_hash = Hash::ZERO;

    for i in 0..20u64 {
        let small_amount = coins_to_units(5); // 5 DOLI each
        let coinbase = Transaction::new_coinbase(small_amount, wallet.address(), i);
        let coinbase_hash = coinbase.hash();

        let block = create_test_block(i, prev_hash, producer.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        node.add_block(block).await.unwrap();

        wallet.add_utxo(
            Outpoint { tx_hash: coinbase_hash, index: 0 },
            UtxoEntry {
                output: Output::normal(small_amount, wallet.address()),
                height: i,
                is_coinbase: true,
            },
        );
    }

    assert_eq!(wallet.utxos.len(), 20);
    assert_eq!(wallet.balance(), coins_to_units(100)); // 20 * 5 DOLI

    // Now consolidate by sending to self
    let recipient = TestWallet::new();
    let consolidation_tx = wallet
        .create_send_transaction(&recipient.address(), coins_to_units(90), coins_to_units(1))
        .unwrap();

    // This should have consumed multiple UTXOs
    println!("Consolidation tx inputs: {}", consolidation_tx.inputs.len());
    println!("Remaining wallet UTXOs: {}", wallet.utxos.len());

    // Wallet should have fewer UTXOs now (some were spent)
    assert!(wallet.utxos.len() < 20);
}

/// Test balance calculation accuracy
#[tokio::test]
async fn test_wallet_balance_accuracy() {
    init_test_logging();

    let mut wallet = TestWallet::new();

    // Add various amounts
    let amounts = [
        coins_to_units(100),
        coins_to_units(50) + 12345,
        coins_to_units(25) + 67890,
        1u64,
        u64::MAX / 100,
    ];

    for (i, &amount) in amounts.iter().enumerate() {
        let fake_hash = crypto::hash::hash(&i.to_le_bytes());
        wallet.add_utxo(
            Outpoint { tx_hash: fake_hash, index: 0 },
            UtxoEntry {
                output: Output::normal(amount, wallet.address()),
                height: i as u64,
                is_coinbase: false,
            },
        );
    }

    let expected: u64 = amounts.iter().sum();
    assert_eq!(wallet.balance(), expected);

    // Remove one
    let first_hash = crypto::hash::hash(&0usize.to_le_bytes());
    wallet.remove_utxo(&Outpoint { tx_hash: first_hash, index: 0 });

    let expected_after: u64 = amounts[1..].iter().sum();
    assert_eq!(wallet.balance(), expected_after);
}

/// Test transaction signing verification
#[tokio::test]
async fn test_wallet_signature_verification() {
    init_test_logging();

    let mut wallet = TestWallet::new();

    // Fund wallet
    let amount = coins_to_units(100);
    let fake_hash = crypto::hash::hash(b"funding_tx");
    wallet.add_utxo(
        Outpoint { tx_hash: fake_hash, index: 0 },
        UtxoEntry {
            output: Output::normal(amount, wallet.address()),
            height: 0,
            is_coinbase: true,
        },
    );

    // Create transaction
    let recipient = TestWallet::new();
    let tx = wallet
        .create_send_transaction(&recipient.address(), coins_to_units(50), 100_000)
        .unwrap();

    // Verify signatures (BIP-143: per-input signing hash)
    for (i, input) in tx.inputs.iter().enumerate() {
        let msg = tx.signing_message_for_input(i);
        let result = signature::verify(msg.as_bytes(), &input.signature, wallet.keypair.public_key());
        assert!(result.is_ok(), "Signature verification failed for input {}", i);
    }
}
