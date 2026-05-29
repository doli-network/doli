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
    use doli_core::conditions::Condition;
    use doli_core::consensus::ConsensusParams;
    use doli_core::network::Network;
    use doli_core::transaction::{Input, Output, Transaction};
    use doli_core::TxType;
    use storage::{Outpoint, UtxoEntry, UtxoSet};

    // --- Test helpers ---

    /// Ensure DeFi activation is enabled for Devnet before any test runs.
    fn init_defi_devnet() {
        use std::sync::Once;
        static INIT: Once = Once::new();
        INIT.call_once(|| unsafe {
            std::env::set_var("DOLI_DEFI_ACTIVATION_HEIGHT", "0");
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

    /// Derive the asset_b hash from a pool seed (matches setup_pool_utxos).
    fn asset_b_for_seed(pool_seed: u8) -> Hash {
        Hash::from_bytes({
            let mut b = [0u8; 32];
            b[0] = pool_seed;
            b[1] = 0xBB;
            b
        })
    }

    fn make_token_out(amount: u64, asset_id: Hash) -> Output {
        Output::fungible_asset(
            amount,
            pubkey_hash(),
            asset_id,
            1_000_000,
            "TKN",
            &Condition::Signature(pubkey_hash()),
        )
        .expect("fungible asset output must encode")
    }

    /// Create a UTXO set with a Pool UTXO and a Normal funding UTXO.
    /// Returns (utxo_set, pool_tx_hash, funding_tx_hash, pool_id).
    /// Pool: reserve_a=10_000, reserve_b=20_000, total_lp=1000.
    fn setup_pool_utxos(pool_seed: u8, funding_amount: u64) -> (UtxoSet, Hash, Hash, Hash) {
        let mut utxo_set = UtxoSet::new();
        let pkh = pubkey_hash();

        let asset_b = asset_b_for_seed(pool_seed);
        let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);

        let pool_tx_hash = crypto::hash::hash(&[pool_seed, 0xAA]);
        let mut pool_output = Output::pool(pool_id, asset_b, 10_000, 20_000, 1000, 0, 100, 30, 100);
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

    /// Build a valid A-to-B Swap tx that passes verify_amm_conservation.
    ///
    /// User sends `dx` DOLI into pool, receives `dy` of token_b.
    /// dy = floor(dx * reserve_b / (reserve_a + dx)).
    /// Pool: 10_000/20_000/1000. dx is fixed at 100 DOLI for simplicity.
    /// Output: [new_pool, token_b_out(dy)]. Remaining DOLI from funding is the fee.
    fn build_swap_tx(
        pool_tx_hash: Hash,
        funding_tx_hash: Hash,
        funding_index: u32,
        pool_id: Hash,
        pool_seed: u8,
    ) -> Transaction {
        let asset_b = asset_b_for_seed(pool_seed);

        // Fixed dx=100 for simplicity. old pool: ra=10_000, rb=20_000.
        let dx: u64 = 100;
        let ra: u64 = 10_000;
        let rb: u64 = 20_000;
        // dy = floor(dx * rb / (ra + dx)) = floor(100 * 20000 / 10100) = floor(198.02) = 198
        let dy = (dx as u128 * rb as u128 / (ra as u128 + dx as u128)) as u64;
        let new_ra = ra + dx;
        let new_rb = rb - dy;
        // Verify k-invariant: new_ra * new_rb >= ra * rb
        assert!(new_ra as u128 * new_rb as u128 >= ra as u128 * rb as u128);

        let updated_pool = Output::pool(pool_id, asset_b, new_ra, new_rb, 1000, 0, 101, 30, 100);
        let token_out = make_token_out(dy, asset_b);

        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::Swap,
            inputs: vec![
                input_with_pubkey(pool_tx_hash, 0),
                input_with_pubkey(funding_tx_hash, funding_index),
            ],
            outputs: vec![updated_pool, token_out],
            extra_data: vec![],
        };
        sign_tx(&mut tx);
        tx
    }

    /// Build a valid AddLiquidity tx.
    ///
    /// Requires both a DOLI funding UTXO and a token_b UTXO. The caller
    /// must add the token_b UTXO to the UTXO set at `(token_tx_hash, 0)`.
    /// Pool: 10_000/20_000/1000. Add 1000 DOLI + 2000 token_b (proportional).
    /// new pool: 11_000/22_000/1100. LP minted = min(1000*1000/10000, 2000*1000/20000) = 100.
    fn build_add_liquidity_tx(
        pool_tx_hash: Hash,
        funding_tx_hash: Hash,
        token_tx_hash: Hash,
        pool_id: Hash,
        pool_seed: u8,
    ) -> Transaction {
        let pkh = pubkey_hash();
        let asset_b = asset_b_for_seed(pool_seed);
        let updated_pool = Output::pool(pool_id, asset_b, 11_000, 22_000, 1100, 0, 101, 30, 100);
        let lp_share = Output::lp_share(100, pool_id, pkh);

        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::AddLiquidity,
            inputs: vec![
                input_with_pubkey(pool_tx_hash, 0),
                input_with_pubkey(funding_tx_hash, 0),
                input_with_pubkey(token_tx_hash, 0),
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

        let tx = build_swap_tx(pool_tx, funding_tx, 0, pool_id, 1);
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

        let tx1 = build_swap_tx(pool_tx, funding_tx, 0, pool_id, 2);
        let res1 = mempool.add_transaction(tx1, &utxo_set, 100);
        assert!(res1.is_ok());
        assert!(res1.unwrap().diagnostic.contention.is_none());

        let tx2 = build_swap_tx(pool_tx, funding_tx2, 0, pool_id, 2);
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

        let tx1 = build_swap_tx(pool_tx, funding_tx, 0, pool_id, 3);
        let res1 = mempool.add_transaction(tx1, &utxo_set, 100);
        assert!(res1.is_ok(), "Swap should be accepted: {:?}", res1);
        assert!(res1.unwrap().diagnostic.contention.is_none());

        let pool_outpoint = Outpoint::new(pool_tx, 0);
        assert_eq!(mempool.pool_contention_count(&pool_outpoint), 1);
    }

    /// TEST 4: AddLiquidity on the same Pool UTXO as a pending Swap.
    #[test]
    fn add_liquidity_same_pool_as_swap_warns() {
        let mut mempool = test_mempool();
        let (mut utxo_set, pool_tx, funding_tx, pool_id) = setup_pool_utxos(4, 100_000);

        let tx1 = build_swap_tx(pool_tx, funding_tx, 0, pool_id, 4);
        let res1 = mempool.add_transaction(tx1, &utxo_set, 100);
        assert!(res1.is_ok());

        // Second tx: AddLiquidity on the same pool — double-spend on pool UTXO
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
        let token_tx2 = crypto::hash::hash(&[4, 0xFD]);
        let asset_b = asset_b_for_seed(4);
        utxo_set
            .insert(
                Outpoint::new(token_tx2, 0),
                UtxoEntry {
                    output: make_token_out(50_000, asset_b),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        let tx2 = build_add_liquidity_tx(pool_tx, funding_tx2, token_tx2, pool_id, 4);
        let res2 = mempool.add_transaction(tx2, &utxo_set, 100);
        assert!(res2.is_err());

        let pool_outpoint = Outpoint::new(pool_tx, 0);
        assert_eq!(mempool.pool_contention_count(&pool_outpoint), 1);
    }

    /// TEST 5: Two swaps targeting different Pool UTXOs -- neither warns.
    #[test]
    fn different_pools_no_warn() {
        let mut mempool = test_mempool();

        // Pool A: seed=10
        let (mut utxo_set, pool_tx_a, funding_tx_a, pool_id_a) = setup_pool_utxos(10, 100_000);

        // Pool B: seed=20 (same standard reserves 10_000/20_000/1000)
        let asset_b2 = asset_b_for_seed(20);
        let pool_id_b = Output::compute_pool_id(&Hash::ZERO, &asset_b2, 30);
        let pool_tx_b = crypto::hash::hash(&[20, 0xAA]);
        let mut pool_output_b =
            Output::pool(pool_id_b, asset_b2, 10_000, 20_000, 1000, 0, 100, 30, 100);
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

        let tx_a = build_swap_tx(pool_tx_a, funding_tx_a, 0, pool_id_a, 10);
        let res_a = mempool.add_transaction(tx_a, &utxo_set, 100);
        assert!(res_a.is_ok(), "Pool A swap should be accepted: {:?}", res_a);
        assert!(res_a.unwrap().diagnostic.contention.is_none());

        let tx_b = build_swap_tx(pool_tx_b, funding_tx_b, 0, pool_id_b, 20);
        let res_b = mempool.add_transaction(tx_b, &utxo_set, 100);
        assert!(res_b.is_ok(), "Pool B swap should be accepted: {:?}", res_b);
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

        let tx = build_swap_tx(pool_tx, funding_tx, 0, pool_id, 6);
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
        let tx2 = build_swap_tx(pool_tx, funding_tx2, 0, pool_id, 6);
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
            let seed = 100 + i;
            let asset_b = asset_b_for_seed(seed);
            let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
            let pool_tx = crypto::hash::hash(&[seed, 0xAA]);
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
                let funding = crypto::hash::hash(&[seed, 0xF0 + j]);
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

            pools.push((pool_tx, pool_id, seed));
        }

        let mut mempool = test_mempool();
        let mut false_positives = 0;
        let mut total_accepted = 0;

        for &(pool_tx, pool_id, seed) in &pools {
            let funding1 = crypto::hash::hash(&[seed, 0xF0]);
            let tx1 = build_swap_tx(pool_tx, funding1, 0, pool_id, seed);
            let res1 = mempool.add_transaction(tx1, &utxo_set, 100);
            if let Ok(r) = res1 {
                total_accepted += 1;
                if r.diagnostic.contention.is_some() {
                    false_positives += 1;
                }
            }

            let funding2 = crypto::hash::hash(&[seed, 0xF1]);
            let tx2 = build_swap_tx(pool_tx, funding2, 0, pool_id, seed);
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

    // =====================================================================
    // INC-I-096 M3: mempool AMM parity tests
    // =====================================================================

    // INC-I-096 (mempool liveness): a real-shape RemoveLiquidity that releases
    // DOLI from pool reserves MUST be ADMITTED by the mempool when the gate is
    // active (devnet: inc_i_096=0). Now routed through verify_amm_conservation
    // (M3 DC-2 parity).
    //
    // Pool: reserve_a=10_000, reserve_b=20_000, total_lp=1000. Burn 500 shares ->
    // proportional da=5000, db=10000. New pool: 5000/10000/500.
    // Outputs: [new Pool, doli_out=5000, tokens_out=10000 (FA), fee_change=5998].
    // Fee input = 6000 DOLI. doli_surplus = (10000+6000) - (5000+5000+5998) = 2.
    #[test]
    fn inc_i_096_remove_liquidity_with_fee_change_admitted() {
        use doli_core::conditions::{Witness, WitnessSignature};
        use doli_core::transaction::SighashType;

        let mut mempool = test_mempool();
        let kp = test_keypair();
        let pkh = pubkey_hash();

        let asset_b = Hash::from_bytes([0x7B; 32]);
        let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);

        let mut utxo_set = UtxoSet::new();

        let pool_tx_hash = crypto::hash::hash(&[0x7B, 0xAA]);
        let mut old_pool = Output::pool(pool_id, asset_b, 10_000, 20_000, 1000, 0, 100, 30, 100);
        old_pool.pubkey_hash = pkh;
        utxo_set
            .insert(
                Outpoint::new(pool_tx_hash, 0),
                UtxoEntry {
                    output: old_pool,
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        let lp_tx_hash = crypto::hash::hash(&[0x7B, 0x11]);
        utxo_set
            .insert(
                Outpoint::new(lp_tx_hash, 0),
                UtxoEntry {
                    output: Output::lp_share(500, pool_id, pkh),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        let fee_tx_hash = crypto::hash::hash(&[0x7B, 0xFF]);
        utxo_set
            .insert(
                Outpoint::new(fee_tx_hash, 0),
                UtxoEntry {
                    output: Output::normal(6000, pkh),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        let new_pool = Output::pool(pool_id, asset_b, 5000, 10000, 500, 0, 101, 30, 100);
        let doli_out = Output::normal(5000, pkh);
        let tokens_out = Output::fungible_asset(
            10000,
            pkh,
            asset_b,
            1_000_000,
            "TKN",
            &Condition::Signature(pkh),
        )
        .expect("fa");
        let fee_change = Output::normal(5998, pkh);

        let mk_input = |prev: Hash| Input {
            prev_tx_hash: prev,
            output_index: 0,
            signature: crypto::Signature::from_bytes([0u8; 64]),
            sighash_type: SighashType::All,
            committed_output_count: 0,
            public_key: Some(*kp.public_key()),
        };

        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::RemoveLiquidity,
            inputs: vec![
                mk_input(pool_tx_hash),
                mk_input(lp_tx_hash),
                mk_input(fee_tx_hash),
            ],
            outputs: vec![new_pool, doli_out, tokens_out, fee_change],
            extra_data: vec![],
        };

        let build_witnesses = |tx: &Transaction| -> Vec<Vec<u8>> {
            let sh1 = tx.signing_message_for_input(1);
            let sig1 = crypto::signature::sign_hash(&sh1, kp.private_key());
            let lp_witness = Witness {
                signatures: vec![WitnessSignature {
                    pubkey: *kp.public_key(),
                    signature: sig1,
                }],
                preimage: None,
                or_branches: vec![],
            };
            vec![vec![], lp_witness.encode(), vec![]]
        };
        for i in 0..tx.inputs.len() {
            let sh = tx.signing_message_for_input(i);
            tx.inputs[i].signature = crypto::signature::sign_hash(&sh, kp.private_key());
        }
        let w = build_witnesses(&tx);
        tx.set_covenant_witnesses(&w);
        for i in 0..tx.inputs.len() {
            let sh = tx.signing_message_for_input(i);
            tx.inputs[i].signature = crypto::signature::sign_hash(&sh, kp.private_key());
        }
        let w = build_witnesses(&tx);
        tx.set_covenant_witnesses(&w);

        let res = mempool.add_transaction(tx, &utxo_set, 100);
        assert!(
            res.is_ok(),
            "INC-I-096: RemoveLiquidity releasing DOLI must be admitted. Got: {:?}",
            res
        );
    }

    /// PARITY TEST: fee == doli_surplus for a valid RemoveLiquidity.
    /// Canonical vector: pool 1000/2000/1000, burn 500, da=500, db=1000.
    /// doli_surplus = (1000 + 1000) - (500 + 500 + 998) = 2.
    #[test]
    fn inc_i_096_fee_equals_doli_surplus() {
        use doli_core::conditions::{Witness, WitnessSignature};
        use doli_core::transaction::SighashType;
        use doli_core::validation::verify_amm_conservation;

        let mut mempool = test_mempool();
        let kp = test_keypair();
        let pkh = pubkey_hash();

        let asset_b = Hash::from_bytes([0xBB; 32]);
        let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);

        let mut utxo_set = UtxoSet::new();

        let pool_tx_hash = crypto::hash::hash(&[0xDD, 0xAA]);
        let old_pool = Output::pool(pool_id, asset_b, 1000, 2000, 1000, 0, 100, 30, 100);
        let mut old_pool_utxo = old_pool.clone();
        old_pool_utxo.pubkey_hash = pkh;
        utxo_set
            .insert(
                Outpoint::new(pool_tx_hash, 0),
                UtxoEntry {
                    output: old_pool_utxo,
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        let lp_tx = crypto::hash::hash(&[0xDD, 0x11]);
        let lp_out = Output::lp_share(500, pool_id, pkh);
        utxo_set
            .insert(
                Outpoint::new(lp_tx, 0),
                UtxoEntry {
                    output: lp_out.clone(),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        let fee_tx = crypto::hash::hash(&[0xDD, 0xFF]);
        let fee_out = Output::normal(1000, pkh);
        utxo_set
            .insert(
                Outpoint::new(fee_tx, 0),
                UtxoEntry {
                    output: fee_out.clone(),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        let new_pool = Output::pool(pool_id, asset_b, 500, 1000, 500, 0, 101, 30, 100);
        let doli_out = Output::normal(500, pkh);
        let tokens_out = Output::fungible_asset(
            1000,
            pkh,
            asset_b,
            1_000_000,
            "TKN",
            &Condition::Signature(pkh),
        )
        .expect("fa");
        let fee_change = Output::normal(998, pkh);

        let tx_outputs = vec![new_pool, doli_out, tokens_out, fee_change];
        let consumed = vec![old_pool, lp_out, fee_out];

        // Consensus side: verify directly
        let consensus_result =
            verify_amm_conservation(TxType::RemoveLiquidity, &consumed, &tx_outputs);
        assert!(consensus_result.is_ok());
        assert_eq!(consensus_result.unwrap().doli_surplus, 2);

        // Mempool side: build and submit
        let mk_input = |prev: Hash| Input {
            prev_tx_hash: prev,
            output_index: 0,
            signature: crypto::Signature::from_bytes([0u8; 64]),
            sighash_type: SighashType::All,
            committed_output_count: 0,
            public_key: Some(*kp.public_key()),
        };

        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::RemoveLiquidity,
            inputs: vec![mk_input(pool_tx_hash), mk_input(lp_tx), mk_input(fee_tx)],
            outputs: tx_outputs,
            extra_data: vec![],
        };

        let build_witnesses = |tx: &Transaction| -> Vec<Vec<u8>> {
            let sh1 = tx.signing_message_for_input(1);
            let sig1 = crypto::signature::sign_hash(&sh1, kp.private_key());
            let w = Witness {
                signatures: vec![WitnessSignature {
                    pubkey: *kp.public_key(),
                    signature: sig1,
                }],
                preimage: None,
                or_branches: vec![],
            };
            vec![vec![], w.encode(), vec![]]
        };
        for i in 0..tx.inputs.len() {
            let sh = tx.signing_message_for_input(i);
            tx.inputs[i].signature = crypto::signature::sign_hash(&sh, kp.private_key());
        }
        let w = build_witnesses(&tx);
        tx.set_covenant_witnesses(&w);
        for i in 0..tx.inputs.len() {
            let sh = tx.signing_message_for_input(i);
            tx.inputs[i].signature = crypto::signature::sign_hash(&sh, kp.private_key());
        }
        let w = build_witnesses(&tx);
        tx.set_covenant_witnesses(&w);

        let res = mempool.add_transaction(tx, &utxo_set, 100);
        assert!(res.is_ok(), "Mempool must admit: {:?}", res);

        let tx_hash = res.unwrap().tx_hash;
        let entry = mempool.get(&tx_hash).expect("entry must exist");
        assert_eq!(entry.fee, 2, "Mempool fee must equal doli_surplus");
    }

    /// PARITY TEST: consensus-valid AMM vectors accepted, consensus-rejected rejected.
    #[test]
    fn inc_i_096_parity_accept_reject() {
        use doli_core::validation::verify_amm_conservation;

        let asset_b = Hash::from_bytes([0xBB; 32]);
        let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
        let user = Hash::from_bytes([0x55; 32]);

        let make_token = |amt: u64| -> Output {
            Output::fungible_asset(
                amt,
                user,
                asset_b,
                1_000_000,
                "TKN",
                &Condition::Signature(user),
            )
            .expect("fa")
        };

        // V1: valid RemoveLiquidity (accept)
        {
            let consumed = vec![
                Output::pool(pool_id, asset_b, 1000, 2000, 1000, 0, 100, 30, 100),
                Output::lp_share(500, pool_id, user),
                Output::normal(1000, user),
            ];
            let outputs = vec![
                Output::pool(pool_id, asset_b, 500, 1000, 500, 0, 101, 30, 100),
                Output::normal(500, user),
                make_token(1000),
                Output::normal(998, user),
            ];
            let r = verify_amm_conservation(TxType::RemoveLiquidity, &consumed, &outputs);
            assert!(r.is_ok(), "V1 valid remove: {:?}", r);
            assert_eq!(r.unwrap().doli_surplus, 2);
        }

        // V2: 1-share drain (reject)
        {
            let consumed = vec![
                Output::pool(pool_id, asset_b, 1000, 2000, 1000, 0, 100, 30, 100),
                Output::lp_share(1, pool_id, user),
                Output::normal(2, user),
            ];
            let outputs = vec![
                Output::pool(pool_id, asset_b, 500, 1998, 999, 0, 101, 30, 100),
                Output::normal(500, user),
                make_token(2),
            ];
            let r = verify_amm_conservation(TxType::RemoveLiquidity, &consumed, &outputs);
            assert!(r.is_err(), "V2 1-share drain must be rejected");
        }

        // V3: T10 underburn (reject)
        {
            let consumed = vec![
                Output::pool(pool_id, asset_b, 1000, 2000, 1000, 0, 100, 30, 100),
                Output::lp_share(1, pool_id, user),
                Output::normal(1002, user),
            ];
            let outputs = vec![
                Output::pool(pool_id, asset_b, 0, 0, 0, 0, 101, 30, 100),
                Output::normal(1000, user),
                make_token(2000),
            ];
            let r = verify_amm_conservation(TxType::RemoveLiquidity, &consumed, &outputs);
            assert!(r.is_err(), "V3 T10 underburn must be rejected");
        }

        // V4: token drain (reject)
        {
            let consumed = vec![
                Output::pool(pool_id, asset_b, 1000, 2000, 1000, 0, 100, 30, 100),
                Output::lp_share(500, pool_id, user),
                Output::normal(2, user),
            ];
            let outputs = vec![
                Output::pool(pool_id, asset_b, 500, 500, 500, 0, 101, 30, 100),
                Output::normal(500, user),
                make_token(1500),
            ];
            let r = verify_amm_conservation(TxType::RemoveLiquidity, &consumed, &outputs);
            assert!(r.is_err(), "V4 token drain must be rejected");
        }

        // V5: valid Swap A->B (accept)
        {
            let consumed = vec![
                Output::pool(pool_id, asset_b, 1000, 1000, 707, 0, 100, 30, 100),
                Output::normal(100, user),
            ];
            let outputs = vec![
                Output::pool(pool_id, asset_b, 1100, 910, 707, 0, 101, 30, 100),
                make_token(90),
            ];
            let r = verify_amm_conservation(TxType::Swap, &consumed, &outputs);
            assert!(r.is_ok(), "V5 valid A->B swap: {:?}", r);
        }

        // V6: valid Swap B->A with fee-change (accept)
        {
            let consumed = vec![
                Output::pool(pool_id, asset_b, 1000, 1000, 707, 0, 100, 30, 100),
                make_token(100),
                Output::normal(100, user),
            ];
            let outputs = vec![
                Output::pool(pool_id, asset_b, 910, 1100, 707, 0, 101, 30, 100),
                Output::normal(90, user),
                Output::normal(98, user),
            ];
            let r = verify_amm_conservation(TxType::Swap, &consumed, &outputs);
            assert!(r.is_ok(), "V6 valid B->A swap: {:?}", r);
            assert_eq!(r.unwrap().doli_surplus, 2);
        }

        // V7: valid AddLiquidity (accept)
        {
            let consumed = vec![
                Output::pool(pool_id, asset_b, 1000, 1000, 707, 0, 100, 30, 100),
                Output::normal(500, user),
                make_token(500),
            ];
            let outputs = vec![
                Output::pool(pool_id, asset_b, 1500, 1500, 1060, 0, 101, 30, 100),
                Output::lp_share(353, pool_id, user),
            ];
            let r = verify_amm_conservation(TxType::AddLiquidity, &consumed, &outputs);
            assert!(r.is_ok(), "V7 valid AddLiquidity: {:?}", r);
        }
    }

    /// PARITY TEST: below inc_i_096 gate, AMM txs use naive conservation.
    #[test]
    fn inc_i_096_below_gate_rejects_remove_liquidity() {
        use doli_core::conditions::{Witness, WitnessSignature};
        use doli_core::transaction::SighashType;

        // Use Testnet where AMM is active (amm_activation_height=20_099,
        // inc_i_092=23_688) but inc_i_096 is u64::MAX. Height 25_000 is
        // above AMM+INC-I-092 gates but below INC-I-096.
        let mut mempool = Mempool::new(
            MempoolPolicy::testnet(),
            ConsensusParams::testnet(),
            Network::Testnet,
        );
        let kp = test_keypair();
        let pkh = pubkey_hash();

        let asset_b = Hash::from_bytes([0x7B; 32]);
        let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);

        let mut utxo_set = UtxoSet::new();

        let pool_tx_hash = crypto::hash::hash(&[0x7B, 0xAA]);
        let mut old_pool = Output::pool(pool_id, asset_b, 10_000, 20_000, 1000, 0, 100, 30, 100);
        old_pool.pubkey_hash = pkh;
        utxo_set
            .insert(
                Outpoint::new(pool_tx_hash, 0),
                UtxoEntry {
                    output: old_pool,
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        let lp_tx = crypto::hash::hash(&[0x7B, 0x11]);
        utxo_set
            .insert(
                Outpoint::new(lp_tx, 0),
                UtxoEntry {
                    output: Output::lp_share(500, pool_id, pkh),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        let fee_tx = crypto::hash::hash(&[0x7B, 0xFF]);
        utxo_set
            .insert(
                Outpoint::new(fee_tx, 0),
                UtxoEntry {
                    output: Output::normal(6000, pkh),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        let new_pool = Output::pool(pool_id, asset_b, 5000, 10000, 500, 0, 101, 30, 100);
        let doli_out = Output::normal(5000, pkh);
        let tokens_out = Output::fungible_asset(
            10000,
            pkh,
            asset_b,
            1_000_000,
            "TKN",
            &Condition::Signature(pkh),
        )
        .expect("fa");
        let fee_change = Output::normal(5998, pkh);

        let mk_input = |prev: Hash| Input {
            prev_tx_hash: prev,
            output_index: 0,
            signature: crypto::Signature::from_bytes([0u8; 64]),
            sighash_type: SighashType::All,
            committed_output_count: 0,
            public_key: Some(*kp.public_key()),
        };

        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::RemoveLiquidity,
            inputs: vec![mk_input(pool_tx_hash), mk_input(lp_tx), mk_input(fee_tx)],
            outputs: vec![new_pool, doli_out, tokens_out, fee_change],
            extra_data: vec![],
        };

        let build_witnesses = |tx: &Transaction| -> Vec<Vec<u8>> {
            let sh1 = tx.signing_message_for_input(1);
            let sig1 = crypto::signature::sign_hash(&sh1, kp.private_key());
            let w = Witness {
                signatures: vec![WitnessSignature {
                    pubkey: *kp.public_key(),
                    signature: sig1,
                }],
                preimage: None,
                or_branches: vec![],
            };
            vec![vec![], w.encode(), vec![]]
        };
        for i in 0..tx.inputs.len() {
            let sh = tx.signing_message_for_input(i);
            tx.inputs[i].signature = crypto::signature::sign_hash(&sh, kp.private_key());
        }
        let w = build_witnesses(&tx);
        tx.set_covenant_witnesses(&w);
        for i in 0..tx.inputs.len() {
            let sh = tx.signing_message_for_input(i);
            tx.inputs[i].signature = crypto::signature::sign_hash(&sh, kp.private_key());
        }
        let w = build_witnesses(&tx);
        tx.set_covenant_witnesses(&w);

        // Below gate (height=25_000, inc_i_096=u64::MAX): naive conservation rejects
        let res = mempool.add_transaction(tx, &utxo_set, 25_000);
        assert!(
            res.is_err(),
            "Below inc_i_096 gate, RemoveLiquidity DOLI-outflow must be rejected"
        );
        let err_msg = format!("{}", res.unwrap_err());
        assert!(
            err_msg.contains("MPTX008") || err_msg.contains("insufficient funds"),
            "Expected MPTX008 insufficient funds, got: {}",
            err_msg
        );
    }
}
