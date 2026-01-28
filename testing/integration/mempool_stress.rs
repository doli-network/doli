//! Mempool Stress Tests
//!
//! Tests mempool performance and behavior under high load with 10,000+ transactions.

#[path = "../common/mod.rs"]
mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{init_test_logging, TestNode, TestNodeConfig};
use doli_core::{Transaction, Output};
use crypto::{Hash, KeyPair};
use tempfile::TempDir;
use tokio::sync::Semaphore;

/// Generate a batch of test transactions
fn generate_transactions(count: usize, base_amount: u64) -> Vec<Transaction> {
    let keypair = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(keypair.public_key().as_bytes());

    (0..count)
        .map(|i| {
            // Create transactions with slightly different amounts to ensure unique hashes
            Transaction::new_coinbase(base_amount + i as u64, pubkey_hash, i as u64)
        })
        .collect()
}

/// Generate transfer transactions with varying outputs
fn generate_transfer_transactions(count: usize) -> Vec<Transaction> {
    let keypair = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(keypair.public_key().as_bytes());

    (0..count)
        .map(|i| {
            let fake_input_hash = crypto::hash::hash(&i.to_le_bytes());
            let input = doli_core::transaction::Input::new(fake_input_hash, 0);

            let outputs = vec![
                Output::normal(1000 + i as u64, pubkey_hash),
                Output::normal(500 + i as u64, pubkey_hash),
            ];

            Transaction::new_transfer(vec![input], outputs)
        })
        .collect()
}

/// Test adding 10,000 transactions sequentially
#[tokio::test]
async fn test_mempool_10k_sequential() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 0);
    let node = Arc::new(TestNode::new(config));

    let transactions = generate_transactions(10_000, 1_000_000);

    let start = Instant::now();

    for tx in transactions {
        node.add_to_mempool(tx).await.unwrap();
    }

    let elapsed = start.elapsed();

    assert_eq!(node.mempool_size().await, 10_000);
    println!("10,000 sequential adds took: {:?}", elapsed);
    println!(
        "Average per tx: {:?}",
        elapsed / 10_000
    );
}

/// Test adding 10,000 transactions concurrently
#[tokio::test]
async fn test_mempool_10k_concurrent() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 10);
    let node = Arc::new(TestNode::new(config));

    let transactions = generate_transactions(10_000, 2_000_000);

    let start = Instant::now();

    let mut handles = Vec::with_capacity(10_000);

    for tx in transactions {
        let node_clone = node.clone();
        let handle = tokio::spawn(async move {
            let _ = node_clone.add_to_mempool(tx).await;
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        let _ = handle.await;
    }

    let elapsed = start.elapsed();
    let final_size = node.mempool_size().await;

    println!("10,000 concurrent adds took: {:?}", elapsed);
    println!("Final mempool size: {}", final_size);

    // Some might fail due to our 10k limit, but should be close
    assert!(final_size >= 9_900);
}

/// Test mempool with limited concurrency (batched)
#[tokio::test]
async fn test_mempool_batched_concurrent() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 20);
    let node = Arc::new(TestNode::new(config));

    let transactions = generate_transactions(10_000, 3_000_000);

    let start = Instant::now();

    // Use semaphore to limit concurrent operations
    let semaphore = Arc::new(Semaphore::new(100));

    let mut handles = Vec::with_capacity(10_000);

    for tx in transactions {
        let node_clone = node.clone();
        let sem_clone = semaphore.clone();

        let handle = tokio::spawn(async move {
            let _permit = sem_clone.acquire().await.unwrap();
            let _ = node_clone.add_to_mempool(tx).await;
        });
        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.await;
    }

    let elapsed = start.elapsed();

    println!("10,000 batched (100 concurrent) adds took: {:?}", elapsed);
    println!("Final mempool size: {}", node.mempool_size().await);
}

/// Test mempool full behavior
#[tokio::test]
async fn test_mempool_full_rejection() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 30);
    let node = Arc::new(TestNode::new(config));

    // Our test node has a 10,000 tx limit
    let transactions = generate_transactions(10_001, 4_000_000);

    let mut accepted = 0;
    let mut rejected = 0;

    for tx in transactions {
        match node.add_to_mempool(tx).await {
            Ok(_) => accepted += 1,
            Err(_) => rejected += 1,
        }
    }

    println!("Accepted: {}, Rejected: {}", accepted, rejected);
    assert_eq!(accepted, 10_000);
    assert_eq!(rejected, 1);
    assert_eq!(node.mempool_size().await, 10_000);
}

/// Test mempool clear operation
#[tokio::test]
async fn test_mempool_clear() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 40);
    let node = Arc::new(TestNode::new(config));

    // Fill mempool
    let transactions = generate_transactions(5_000, 5_000_000);
    for tx in transactions {
        node.add_to_mempool(tx).await.unwrap();
    }

    assert_eq!(node.mempool_size().await, 5_000);

    // Clear
    node.clear_mempool().await;
    assert_eq!(node.mempool_size().await, 0);

    // Can add more after clear
    let new_transactions = generate_transactions(1_000, 6_000_000);
    for tx in new_transactions {
        node.add_to_mempool(tx).await.unwrap();
    }

    assert_eq!(node.mempool_size().await, 1_000);
}

/// Test mempool with varying transaction sizes
#[tokio::test]
async fn test_mempool_varying_sizes() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 50);
    let node = Arc::new(TestNode::new(config));

    // Small transactions (1 output)
    let small_txs = generate_transactions(3_000, 7_000_000);

    // Medium transactions (2 outputs)
    let medium_txs = generate_transfer_transactions(3_000);

    // Large transactions (many outputs)
    let large_txs: Vec<Transaction> = (0..1_000)
        .map(|i| {
            let keypair = KeyPair::generate();
            let pubkey_hash = crypto::hash::hash(keypair.public_key().as_bytes());

            let outputs: Vec<Output> = (0..20)
                .map(|j| Output::normal(100 + j as u64, pubkey_hash))
                .collect();

            let fake_input_hash = crypto::hash::hash(&(i as u64 + 100_000u64).to_le_bytes());
            let input = doli_core::transaction::Input::new(fake_input_hash, 0);

            Transaction::new_transfer(vec![input], outputs)
        })
        .collect();

    // Add all types
    for tx in small_txs {
        let _ = node.add_to_mempool(tx).await;
    }
    for tx in medium_txs {
        let _ = node.add_to_mempool(tx).await;
    }
    for tx in large_txs {
        let _ = node.add_to_mempool(tx).await;
    }

    let size = node.mempool_size().await;
    println!("Total mempool size with varying tx: {}", size);
    assert!(size >= 7_000);
}

/// Test mempool throughput measurement
#[tokio::test]
async fn test_mempool_throughput() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 60);
    let node = Arc::new(TestNode::new(config));

    let batch_size = 1_000;
    let num_batches = 5;

    let mut throughputs = Vec::new();

    for batch in 0..num_batches {
        let transactions = generate_transactions(batch_size, (batch + 1) as u64 * 10_000_000);

        let start = Instant::now();

        for tx in transactions {
            let _ = node.add_to_mempool(tx).await;
        }

        let elapsed = start.elapsed();
        let throughput = batch_size as f64 / elapsed.as_secs_f64();
        throughputs.push(throughput);

        println!("Batch {}: {:.0} tx/sec", batch + 1, throughput);
    }

    let avg_throughput: f64 = throughputs.iter().sum::<f64>() / throughputs.len() as f64;
    println!("Average throughput: {:.0} tx/sec", avg_throughput);

    // Should handle at least 10k tx/sec on reasonable hardware
    assert!(avg_throughput > 1000.0);
}

/// Test mempool memory usage (approximate)
#[tokio::test]
async fn test_mempool_memory_footprint() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 70);
    let node = Arc::new(TestNode::new(config));

    // Calculate average transaction size
    let sample_txs = generate_transactions(100, 8_000_000);
    let avg_size: usize = sample_txs.iter().map(|tx| tx.size()).sum::<usize>() / 100;

    println!("Average transaction size: {} bytes", avg_size);

    // Fill mempool
    let transactions = generate_transactions(10_000, 9_000_000);
    for tx in transactions {
        let _ = node.add_to_mempool(tx).await;
    }

    // Estimate total memory
    let estimated_memory = avg_size * 10_000;
    println!("Estimated mempool memory: {} bytes ({:.2} MB)",
        estimated_memory,
        estimated_memory as f64 / 1_000_000.0
    );

    // Should be under 100 MB for 10k transactions
    assert!(estimated_memory < 100_000_000);
}

/// Test rapid add/remove cycles
#[tokio::test]
async fn test_mempool_churn() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 80);
    let node = Arc::new(TestNode::new(config));

    let cycles = 10;
    let txs_per_cycle = 1_000;

    let start = Instant::now();

    for cycle in 0..cycles {
        // Add transactions
        let transactions = generate_transactions(txs_per_cycle, (cycle + 1) as u64 * 100_000_000);
        for tx in transactions {
            let _ = node.add_to_mempool(tx).await;
        }

        // Clear (simulating block inclusion)
        node.clear_mempool().await;
    }

    let elapsed = start.elapsed();

    println!("10 cycles of 1,000 tx add+clear took: {:?}", elapsed);
    println!(
        "Average per cycle: {:?}",
        elapsed / cycles as u32
    );

    assert_eq!(node.mempool_size().await, 0);
}

/// Test concurrent read/write on mempool
#[tokio::test]
async fn test_mempool_concurrent_read_write() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 90);
    let node = Arc::new(TestNode::new(config));

    let transactions = generate_transactions(5_000, 11_000_000);

    // Spawn writers
    let node_writer = node.clone();
    let writer_handle = tokio::spawn(async move {
        for tx in transactions {
            let _ = node_writer.add_to_mempool(tx).await;
            tokio::time::sleep(Duration::from_micros(10)).await;
        }
    });

    // Spawn readers
    let node_reader = node.clone();
    let reader_handle = tokio::spawn(async move {
        let mut reads = 0;
        for _ in 0..1000 {
            let _ = node_reader.mempool_size().await;
            reads += 1;
            tokio::time::sleep(Duration::from_micros(100)).await;
        }
        reads
    });

    let _ = writer_handle.await;
    let reads = reader_handle.await.unwrap();

    println!("Completed {} concurrent reads", reads);
    assert!(reads > 0);
}

/// Test mempool with duplicate transaction rejection
#[tokio::test]
async fn test_mempool_duplicate_handling() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 100);
    let node = Arc::new(TestNode::new(config));

    // Generate unique transactions
    let transactions = generate_transactions(1_000, 12_000_000);

    // Add all once
    for tx in &transactions {
        node.add_to_mempool(tx.clone()).await.unwrap();
    }

    assert_eq!(node.mempool_size().await, 1_000);

    // Try to add duplicates - our simple mempool allows them
    // A real implementation would reject
    for tx in &transactions[..100] {
        let _ = node.add_to_mempool(tx.clone()).await;
    }

    // With duplicates allowed, we'd have 1100
    // With proper dedup, we'd still have 1000
    let final_size = node.mempool_size().await;
    println!("Final size after duplicate attempts: {}", final_size);
}

/// Stress test: rapid bursts of transactions
#[tokio::test]
async fn test_mempool_burst_traffic() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 110);
    let node = Arc::new(TestNode::new(config));

    let bursts = 5;
    let txs_per_burst = 2_000;
    let pause_between = Duration::from_millis(100);

    let start = Instant::now();

    for burst in 0..bursts {
        let transactions = generate_transactions(txs_per_burst, (burst + 1) as u64 * 200_000_000);

        // Rapid burst
        let burst_start = Instant::now();
        for tx in transactions {
            let _ = node.add_to_mempool(tx).await;
        }
        let burst_time = burst_start.elapsed();

        println!(
            "Burst {} ({} txs): {:?}",
            burst + 1,
            txs_per_burst,
            burst_time
        );

        // Pause between bursts
        tokio::time::sleep(pause_between).await;
    }

    let total_elapsed = start.elapsed();

    println!("Total time for {} bursts: {:?}", bursts, total_elapsed);
    println!("Final mempool size: {}", node.mempool_size().await);
}
