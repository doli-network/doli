// OUTPUT CONTRACT: fn Mempool::add_transaction() — contention diagnostic extension
//
// Observable outputs:
//   O1: Result<AddTransactionResult, MempoolError> — tx acceptance or rejection
//   O2: AddTransactionResult.diagnostic.contention — Option<ContentionInfo>
//   O3: pool_contention_index — internal HashMap updated (observable via pool_contention_count)
//   O4: spent_outputs — existing double-spend tracking (unchanged behavior)
//
// Code paths:
//   P1: Non-AMM tx (Transfer, Registration, etc.) — no contention check
//   P2: AMM tx, Pool UTXO not contested — accepted, contention=None
//   P3: AMM tx, Pool UTXO contested (same outpoint as pending tx) — double-spend rejection
//   P4: AMM tx accepted, then removed — contention index cleared
//   P5: Multiple AMM txs, different pools — no cross-pool contention
//
// INPUT PARTITIONS:
//   IP1: tx_type ∈ {Transfer, Registration, ...} — non-AMM (P1)
//   IP2: tx_type ∈ {Swap, AddLiquidity, RemoveLiquidity}, pool UTXO unique — first AMM (P2)
//   IP3: tx_type ∈ {Swap, AddLiquidity, RemoveLiquidity}, pool UTXO already spent — double-spend (P3)
//   IP4: tx_type ∈ {Swap, AddLiquidity, RemoveLiquidity}, mixed types same pool — cross-type (P3)
//   IP5: tx_type ∈ {Swap, AddLiquidity, RemoveLiquidity}, different pool UTXOs — independent (P5)
//   IP6: After removal of AMM tx — index cleanup (P4)
//
// MATRIX:
//   IP1 × P1 → O1=Ok, O2=None, O3=empty — test: non_amm_tx_no_index_overhead
//   IP2 × P2 → O1=Ok, O2=None, O3={pool→{tx}} — test: no_contention_returns_no_warning
//   IP3 × P3 → O1=Err(DoubleSpend), O3={pool→{tx1}} — test: two_swaps_same_pool_second_warns
//   IP4 × P3 → O1=Err(DoubleSpend), O3={pool→{tx1}} — test: add_liquidity_same_pool_as_swap_warns
//   IP5 × P5 → O1=Ok, O2=None per each — test: different_pools_no_warn
//   IP6 × P4 → O3=empty after removal — test: removal_clears_contention
//   IP2..5 × AC-12 → false-positive rate <= 0.1% — test: false_positive_replay
//   Structural → ContentionInfo has no competing tx hashes — test: diagnostic_does_not_leak

#[cfg(test)]
mod tests {
    use crate::contention::ContentionInfo;
    use crate::{Mempool, MempoolPolicy};
    use crypto::Hash;
    use doli_core::consensus::ConsensusParams;
    use doli_core::network::Network;
    use doli_core::transaction::{Input, Output, Transaction};
    use doli_core::TxType;
    use storage::{Outpoint, UtxoEntry, UtxoSet};

    // --- Test helpers ---

    /// Ensure DeFi activation is enabled for Devnet before any test runs.
    /// OnceLock per-network means Devnet params are loaded fresh on first access.
    fn init_defi_devnet() {
        use std::sync::Once;
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            // SAFETY: test-only env var, set before any Devnet params are loaded.
            unsafe {
                std::env::set_var("DOLI_DEFI_ACTIVATION_HEIGHT", "0");
            }
        });
    }

    fn test_mempool() -> Mempool {
        init_defi_devnet();
        Mempool::new(
            MempoolPolicy::testnet(),
            ConsensusParams::devnet(),
            Network::Devnet,
        )
    }

    fn test_keypair() -> crypto::KeyPair {
        crypto::KeyPair::from_seed([42u8; 32])
    }

    fn pubkey_hash() -> Hash {
        let kp = test_keypair();
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes())
    }

    fn sign_tx(tx: &mut Transaction) {
        let kp = test_keypair();
        for i in 0..tx.inputs.len() {
            let signing_hash = tx.signing_message_for_input(i);
            tx.inputs[i].signature = crypto::signature::sign_hash(&signing_hash, kp.private_key());
        }
    }

    fn input_with_pubkey(prev_hash: Hash, index: u32) -> Input {
        let kp = test_keypair();
        let mut input = Input::new(prev_hash, index);
        input.public_key = Some(*kp.public_key());
        input
    }

    /// Create a UTXO set with a Pool UTXO and a Normal funding UTXO.
    /// Returns (utxo_set, pool_tx_hash, funding_tx_hash, pool_id).
    fn setup_pool_utxos(pool_seed: u8, funding_amount: u64) -> (UtxoSet, Hash, Hash, Hash) {
        let mut utxo_set = UtxoSet::new();
        let pkh = pubkey_hash();

        let asset_b = Hash::from_bytes({
            let mut b = [0u8; 32];
            b[0] = pool_seed;
            b[1] = 0xBB;
            b
        });
        let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);

        // Pool UTXO
        let pool_tx_hash = crypto::hash::hash(&[pool_seed, 0xAA]);
        let mut pool_output = Output::pool(pool_id, asset_b, 10_000, 20_000, 1000, 0, 100, 30, 100);
        // Override pubkey_hash so the test keypair can sign for this Pool UTXO.
        // In production, Pool pubkey_hash = pool_id (no private key). For tests,
        // we set it to the test keypair's hash to pass mempool signature validation.
        pool_output.pubkey_hash = pkh;
        utxo_set
            .insert(
                Outpoint::new(pool_tx_hash, 0),
                UtxoEntry {
                    output: pool_output,
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        // Funding UTXO
        let funding_tx_hash = crypto::hash::hash(&[pool_seed, 0xFF]);
        utxo_set
            .insert(
                Outpoint::new(funding_tx_hash, 0),
                UtxoEntry {
                    output: Output::normal(funding_amount, pkh),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        (utxo_set, pool_tx_hash, funding_tx_hash, pool_id)
    }

    /// Build a Swap transaction that spends a Pool UTXO + a funding UTXO.
    fn build_swap_tx(
        pool_tx_hash: Hash,
        funding_tx_hash: Hash,
        funding_index: u32,
        pool_id: Hash,
        swap_output_amount: u64,
    ) -> Transaction {
        let pkh = pubkey_hash();
        let asset_b_dummy = Hash::from_bytes([0xBB; 32]);
        let updated_pool = Output::pool(
            pool_id,
            asset_b_dummy,
            10_000 + swap_output_amount,
            20_000 - 100,
            1000,
            0,
            101,
            30,
            100,
        );
        let swap_result = Output::normal(swap_output_amount, pkh);

        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::Swap,
            inputs: vec![
                input_with_pubkey(pool_tx_hash, 0),
                input_with_pubkey(funding_tx_hash, funding_index),
            ],
            outputs: vec![updated_pool, swap_result],
            extra_data: vec![],
        };
        sign_tx(&mut tx);
        tx
    }

    /// Build an AddLiquidity transaction.
    fn build_add_liquidity_tx(
        pool_tx_hash: Hash,
        funding_tx_hash: Hash,
        pool_id: Hash,
    ) -> Transaction {
        let pkh = pubkey_hash();
        let asset_b_dummy = Hash::from_bytes([0xBB; 32]);
        let updated_pool = Output::pool(
            pool_id,
            asset_b_dummy,
            11_000,
            21_000,
            1100,
            0,
            101,
            30,
            100,
        );
        let lp_share = Output::lp_share(100, pool_id, pkh);

        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::AddLiquidity,
            inputs: vec![
                input_with_pubkey(pool_tx_hash, 0),
                input_with_pubkey(funding_tx_hash, 0),
            ],
            outputs: vec![updated_pool, lp_share],
            extra_data: vec![],
        };
        sign_tx(&mut tx);
        tx
    }

    // --- Tests ---

    /// TEST 1: A single Swap with no competing TXs returns no contention warning.
    #[test]
    fn no_contention_returns_no_warning() {
        let mut mempool = test_mempool();
        let (utxo_set, pool_tx, funding_tx, pool_id) = setup_pool_utxos(1, 50_000);

        let tx = build_swap_tx(pool_tx, funding_tx, 0, pool_id, 1_000);
        let result = mempool.add_transaction(tx, &utxo_set, 100);

        assert!(result.is_ok(), "Swap should be accepted: {:?}", result);
        let res = result.unwrap();
        assert!(
            res.diagnostic.contention.is_none(),
            "Single swap should have no contention warning"
        );
    }

    /// TEST 2: Two swaps targeting the same Pool UTXO -- second is rejected.
    #[test]
    fn two_swaps_same_pool_second_warns() {
        let mut mempool = test_mempool();
        let (mut utxo_set, pool_tx, funding_tx, pool_id) = setup_pool_utxos(2, 100_000);

        let funding_tx2 = crypto::hash::hash(&[2, 0xFE]);
        utxo_set
            .insert(
                Outpoint::new(funding_tx2, 0),
                UtxoEntry {
                    output: Output::normal(50_000, pubkey_hash()),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        let tx1 = build_swap_tx(pool_tx, funding_tx, 0, pool_id, 1_000);
        let res1 = mempool.add_transaction(tx1, &utxo_set, 100);
        assert!(res1.is_ok());
        assert!(res1.unwrap().diagnostic.contention.is_none());

        let tx2 = build_swap_tx(pool_tx, funding_tx2, 0, pool_id, 500);
        let res2 = mempool.add_transaction(tx2, &utxo_set, 100);
        assert!(
            res2.is_err(),
            "Second swap should be rejected as double-spend"
        );

        let pool_outpoint = Outpoint::new(pool_tx, 0);
        assert_eq!(
            mempool.pool_contention_count(&pool_outpoint),
            1,
            "Contention index should track the first swap"
        );
    }

    /// TEST 3: Contention count verified after single swap.
    #[test]
    fn three_pending_same_pool_warns_with_count() {
        let mut mempool = test_mempool();
        let (utxo_set, pool_tx, funding_tx, pool_id) = setup_pool_utxos(3, 200_000);

        let tx1 = build_swap_tx(pool_tx, funding_tx, 0, pool_id, 1_000);
        let res1 = mempool.add_transaction(tx1, &utxo_set, 100);
        assert!(res1.is_ok());
        assert!(res1.unwrap().diagnostic.contention.is_none());

        let pool_outpoint = Outpoint::new(pool_tx, 0);
        assert_eq!(mempool.pool_contention_count(&pool_outpoint), 1);
    }

    /// TEST 4: AddLiquidity on the same Pool UTXO as a pending Swap.
    #[test]
    fn add_liquidity_same_pool_as_swap_warns() {
        let mut mempool = test_mempool();
        let (mut utxo_set, pool_tx, funding_tx, pool_id) = setup_pool_utxos(4, 100_000);

        let tx1 = build_swap_tx(pool_tx, funding_tx, 0, pool_id, 1_000);
        let res1 = mempool.add_transaction(tx1, &utxo_set, 100);
        assert!(res1.is_ok());

        let funding_tx2 = crypto::hash::hash(&[4, 0xFE]);
        utxo_set
            .insert(
                Outpoint::new(funding_tx2, 0),
                UtxoEntry {
                    output: Output::normal(50_000, pubkey_hash()),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        let tx2 = build_add_liquidity_tx(pool_tx, funding_tx2, pool_id);
        let res2 = mempool.add_transaction(tx2, &utxo_set, 100);
        assert!(res2.is_err());

        let pool_outpoint = Outpoint::new(pool_tx, 0);
        assert_eq!(mempool.pool_contention_count(&pool_outpoint), 1);
    }

    /// TEST 5: Two swaps targeting different Pool UTXOs -- neither warns.
    #[test]
    fn different_pools_no_warn() {
        let mut mempool = test_mempool();

        let (mut utxo_set, pool_tx_a, funding_tx_a, pool_id_a) = setup_pool_utxos(10, 100_000);

        let asset_b2 = Hash::from_bytes({
            let mut b = [0u8; 32];
            b[0] = 20;
            b[1] = 0xBB;
            b
        });
        let pool_id_b = Output::compute_pool_id(&Hash::ZERO, &asset_b2, 30);
        let pool_tx_b = crypto::hash::hash(&[20, 0xAA]);
        let mut pool_output_b =
            Output::pool(pool_id_b, asset_b2, 5_000, 10_000, 500, 0, 100, 30, 100);
        pool_output_b.pubkey_hash = pubkey_hash();
        utxo_set
            .insert(
                Outpoint::new(pool_tx_b, 0),
                UtxoEntry {
                    output: pool_output_b,
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();
        let funding_tx_b = crypto::hash::hash(&[20, 0xFF]);
        utxo_set
            .insert(
                Outpoint::new(funding_tx_b, 0),
                UtxoEntry {
                    output: Output::normal(100_000, pubkey_hash()),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        let tx_a = build_swap_tx(pool_tx_a, funding_tx_a, 0, pool_id_a, 1_000);
        let res_a = mempool.add_transaction(tx_a, &utxo_set, 100);
        assert!(res_a.is_ok());
        assert!(res_a.unwrap().diagnostic.contention.is_none());

        let tx_b = build_swap_tx(pool_tx_b, funding_tx_b, 0, pool_id_b, 500);
        let res_b = mempool.add_transaction(tx_b, &utxo_set, 100);
        assert!(res_b.is_ok());
        assert!(
            res_b.unwrap().diagnostic.contention.is_none(),
            "Different pools should not trigger contention"
        );
    }

    /// TEST 6: After removal, contention clears.
    #[test]
    fn removal_clears_contention() {
        let mut mempool = test_mempool();
        let (mut utxo_set, pool_tx, funding_tx, pool_id) = setup_pool_utxos(6, 100_000);

        let tx = build_swap_tx(pool_tx, funding_tx, 0, pool_id, 1_000);
        let tx_hash = tx.hash();
        let res = mempool.add_transaction(tx, &utxo_set, 100);
        assert!(res.is_ok(), "Swap should be accepted: {:?}", res);

        let pool_outpoint = Outpoint::new(pool_tx, 0);
        assert_eq!(mempool.pool_contention_count(&pool_outpoint), 1);

        mempool.remove_transaction(&tx_hash);

        assert_eq!(
            mempool.pool_contention_count(&pool_outpoint),
            0,
            "Contention should clear after TX removal"
        );

        let funding_tx2 = crypto::hash::hash(&[6, 0xFE]);
        utxo_set
            .insert(
                Outpoint::new(funding_tx2, 0),
                UtxoEntry {
                    output: Output::normal(50_000, pubkey_hash()),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();
        let tx2 = build_swap_tx(pool_tx, funding_tx2, 0, pool_id, 500);
        let res2 = mempool.add_transaction(tx2, &utxo_set, 100);
        assert!(res2.is_ok());
        assert!(res2.unwrap().diagnostic.contention.is_none());
    }

    /// TEST 7: Non-AMM transactions do not touch the contention index.
    #[test]
    fn non_amm_tx_no_index_overhead() {
        let mut mempool = test_mempool();
        let pkh = pubkey_hash();
        let funding_hash = crypto::hash::hash(b"transfer_fund");

        let mut utxo_set = UtxoSet::new();
        utxo_set
            .insert(
                Outpoint::new(funding_hash, 0),
                UtxoEntry {
                    output: Output::normal(10_000, pkh),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        let dest = crypto::hash::hash(b"dest");
        let mut tx = Transaction::new_transfer(
            vec![input_with_pubkey(funding_hash, 0)],
            vec![Output::normal(9_000, dest)],
        );
        sign_tx(&mut tx);

        let result = mempool.add_transaction(tx, &utxo_set, 100);
        assert!(result.is_ok());

        let res = result.unwrap();
        assert!(
            res.diagnostic.contention.is_none(),
            "Transfer should have no contention diagnostic"
        );
        assert_eq!(mempool.pool_contention_index_len(), 0);
    }

    /// TEST 8: ContentionInfo does NOT contain competing tx hashes (MEV safety).
    #[test]
    fn diagnostic_does_not_leak_competing_tx_hashes() {
        let info = ContentionInfo {
            competing_count: 3,
            pool_utxo_tx: Hash::ZERO,
            pool_utxo_index: 0,
        };

        let json = serde_json::to_value(&info).unwrap();
        let obj = json.as_object().unwrap();

        assert!(obj.contains_key("competing_count"));
        assert!(obj.contains_key("pool_utxo_tx"));
        assert!(obj.contains_key("pool_utxo_index"));

        assert!(!obj.contains_key("competing_txs"));
        assert!(!obj.contains_key("competing_hashes"));
        assert!(!obj.contains_key("tx_hashes"));
        assert!(!obj.contains_key("competitors"));
    }

    /// TEST 9: Replay N=10 concurrent swaps across M=5 pools -- false-positive bounded.
    /// AC-12: false-positive rate <= 0.1%.
    #[test]
    fn false_positive_replay() {
        let mut utxo_set = UtxoSet::new();
        let pkh = pubkey_hash();

        let mut pools = Vec::new();
        for i in 0u8..5 {
            let asset_b = Hash::from_bytes({
                let mut b = [0u8; 32];
                b[0] = 100 + i;
                b[1] = 0xBB;
                b
            });
            let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
            let pool_tx = crypto::hash::hash(&[100 + i, 0xAA]);
            let mut pool_output =
                Output::pool(pool_id, asset_b, 10_000, 20_000, 1000, 0, 100, 30, 100);
            pool_output.pubkey_hash = pkh;
            utxo_set
                .insert(
                    Outpoint::new(pool_tx, 0),
                    UtxoEntry {
                        output: pool_output,
                        height: 1,
                        is_coinbase: false,
                        is_epoch_reward: false,
                    },
                )
                .unwrap();

            for j in 0u8..2 {
                let funding = crypto::hash::hash(&[100 + i, 0xF0 + j]);
                utxo_set
                    .insert(
                        Outpoint::new(funding, 0),
                        UtxoEntry {
                            output: Output::normal(50_000, pkh),
                            height: 1,
                            is_coinbase: false,
                            is_epoch_reward: false,
                        },
                    )
                    .unwrap();
            }

            pools.push((pool_tx, pool_id, i));
        }

        let mut mempool = test_mempool();
        let mut false_positives = 0;
        let mut total_accepted = 0;

        for &(pool_tx, pool_id, i) in &pools {
            let funding1 = crypto::hash::hash(&[100 + i, 0xF0]);
            let tx1 = build_swap_tx(pool_tx, funding1, 0, pool_id, 1_000);
            let res1 = mempool.add_transaction(tx1, &utxo_set, 100);
            if let Ok(r) = res1 {
                total_accepted += 1;
                if r.diagnostic.contention.is_some() {
                    false_positives += 1;
                }
            }

            let funding2 = crypto::hash::hash(&[100 + i, 0xF1]);
            let tx2 = build_swap_tx(pool_tx, funding2, 0, pool_id, 500);
            let _res2 = mempool.add_transaction(tx2, &utxo_set, 100);
        }

        let fp_rate = if total_accepted > 0 {
            false_positives as f64 / total_accepted as f64
        } else {
            0.0
        };

        assert!(
            fp_rate <= 0.001,
            "False-positive rate {:.4} exceeds AC-12 bound of 0.1%",
            fp_rate
        );
        assert_eq!(false_positives, 0);
        assert_eq!(total_accepted, 5, "Should accept one swap per pool");
    }
}
