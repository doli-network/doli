// OUTPUT CONTRACT: fn apply_block(&mut self, block: Block, mode: ValidationMode) -> Result<()>
//   + fn build_block_content(&mut self, Hash, u32, u64, u32, PublicKey)
//       -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   (bins/node/src/node/apply_block/mod.rs; bins/node/src/node/production/assembly.rs)
//   O1 return value      apply_block Ok | Err — block verdict
//   O2 persistent store  state_db UTXOs: FA outputs of the legit flow present after apply
//   O3 return value      build_block_content tx list — membership of the violating tx
// PATHS:
//   P-W7-COINBASE     coinbase whose single output is FungibleAsset / LPShare / Pool
//   P-HONEST-CB       coinbase Output::normal(reward, reward_pool)
//   P-BUILDER         counterfeit-FA Transfer injected via Mempool::add_system_transaction
//                     (skips UTXO checks) + an honest native Transfer control
//   P-LEGIT           anchored issuance (asset_id = compute_asset_id(inputs[0])) then FA split
//   P-BELOW-AH        config.network = Testnet (AH 78_199), phantom-pool Transfer at h=1
// INPUT PARTITIONS:
//   coinbase output type ∈ {FungibleAsset, LPShare, Pool} (reject) vs {Normal} (accept);
//   builder candidates ∈ {violating, honest control}; issuance asset_id = H(inputs[0]) with
//   Σ = total_supply, then Σout = Σin split; height h=1 < AH (Testnet) vs h ≥ AH=0 (Devnet).
// MATRIX:
//   O1    x P-W7-COINBASE -> w7_coinbase_with_non_normal_output_is_rejected
//   O1    x P-HONEST-CB   -> w7_honest_coinbase_block_still_applies
//   O3    x P-BUILDER     -> builder_drops_counterfeit_fa_transfer
//   O1,O2 x P-LEGIT       -> legit_anchored_issuance_then_fa_transfer_applies
//   O1    x P-BELOW-AH    -> below_ah_phantom_transfer_keeps_pre_fix_verdict

use std::time::{SystemTime, UNIX_EPOCH};

use crypto::Hash;
use doli_core::network::Network;
use doli_core::transaction::{Output, OutputType, Transaction, TxType};
use doli_node::node::Node;

use crate::inc_i_234_output_authority::{
    addr, apply, block, block_with_coinbase, counterfeit_fa_transfer, fa_supply, honest_coinbase,
    input, make_node, phantom_transfer, seed, sign, FEE_BPS, FUNDING, FUND_PREV,
};

fn coinbase_with(node: &Node, output: Output) -> Transaction {
    let mut cb = honest_coinbase(node, 1, 1);
    cb.outputs[0] = output;
    cb
}

// REQ-234-SEC-006 — Decision: a failure means a producer can mint Pool/FA/LP value through the coinbase (W7).
#[tokio::test]
async fn w7_coinbase_with_non_normal_output_is_rejected() {
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let asset = Hash::from_bytes([0xC7; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset, FEE_BPS);
    let mut accepted = Vec::new();
    for kind in ["fa", "lp", "pool"] {
        let (mut node, kp, _t) = make_node().await;
        let reward = node.params.block_reward(1);
        let out = match kind {
            "fa" => fa_supply(reward, pool_hash, asset, u64::MAX),
            "lp" => Output::lp_share(reward, pool_id, pool_hash),
            _ => {
                let mut p = Output::pool(pool_id, asset, reward, 1, 1_000, 0, 1, FEE_BPS, 1);
                p.amount = reward;
                p.pubkey_hash = pool_hash;
                p
            }
        };
        assert_ne!(out.output_type, OutputType::Normal);
        let cb = coinbase_with(&node, out);
        assert!(
            cb.is_coinbase(),
            "fixture: {kind} coinbase lost coinbase shape"
        );
        let v = {
            let blk = block_with_coinbase(&node, &kp, Hash::ZERO, 1, cb, vec![]);
            apply(&mut node, blk).await
        };
        if v.is_ok() {
            accepted.push(kind);
        }
    }
    assert!(
        accepted.is_empty(),
        "W7: blocks with non-Normal coinbase outputs were ACCEPTED: {accepted:?}"
    );
}

// REQ-234-SEC-006 — Decision: a failure means the coinbase rule rejects honest blocks (chain halt).
#[tokio::test]
async fn w7_honest_coinbase_block_still_applies() {
    let (mut node, kp, _t) = make_node().await;
    let v = {
        let blk = block(&node, &kp, Hash::ZERO, 1, 1, vec![]);
        apply(&mut node, blk).await
    };
    assert!(v.is_ok(), "honest coinbase block rejected: {v:?}");
}

// REQ-234-SEC-012 — Decision: a failure means the builder packs a tx its own apply rejects (self-fork / lost slot).
#[tokio::test]
async fn builder_drops_counterfeit_fa_transfer() {
    const HEIGHT: u64 = 100_001;
    let (mut node, kp, _t) = make_node().await;
    let pkh = addr(&kp);
    let bad =
        counterfeit_fa_transfer(&node, &kp, FUND_PREV, Hash::from_bytes([0xB2; 32]), 1_000).await;
    let control_prev = Hash::from_bytes([0x72; 32]);
    seed(&node, control_prev, 0, Output::normal(FUNDING, pkh)).await;
    let mut control = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![input(control_prev, 0, &kp)],
        outputs: vec![Output::normal(0, pkh)],
        extra_data: vec![],
    };
    control.outputs[0].amount = FUNDING - control.minimum_fee();
    sign(&mut control, &kp, false);
    let (bad_h, control_h) = (bad.hash(), control.hash());
    {
        let mut mp = node.mempool.write().await;
        mp.add_system_transaction(bad, HEIGHT)
            .expect("fixture: system lane admits the counterfeit (stateless-valid)");
        mp.add_system_transaction(control, HEIGHT)
            .expect("fixture: system lane admits the control");
    }
    let our_pubkey = *kp.public_key();
    let mut txs = None;
    for _ in 0..12 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        let slot = node.params.timestamp_to_slot(now);
        let built = node
            .build_block_content(Hash::ZERO, slot.saturating_sub(1), HEIGHT, slot, our_pubkey)
            .await
            .expect("build_block_content");
        if let Some((_, t, _)) = built {
            txs = Some(t);
            break;
        }
    }
    let txs = txs.expect("fixture: 12 consecutive slot-boundary aborts");
    let has = |h: Hash| txs.iter().any(|t| t.hash() == h);
    assert!(
        has(control_h),
        "fixture: honest control tx not included — builder test is vacuous"
    );
    assert!(
        !has(bad_h),
        "builder included the counterfeit-FA Transfer that apply must reject"
    );
}

// REQ-234-011 — Decision: a failure means the fix breaks legitimate anchored issuance or FA transfers.
#[tokio::test]
async fn legit_anchored_issuance_then_fa_transfer_applies() {
    let (mut node, kp, _t) = make_node().await;
    let pkh = addr(&kp);
    seed(&node, FUND_PREV, 0, Output::normal(FUNDING, pkh)).await;
    let asset = Output::compute_asset_id(&FUND_PREV, 0);
    let mut t1 = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![input(FUND_PREV, 0, &kp)],
        outputs: vec![fa_supply(1_000, pkh, asset, 1_000), Output::normal(0, pkh)],
        extra_data: vec![],
    };
    let fee1 = t1.minimum_fee();
    t1.outputs[1].amount = FUNDING - fee1;
    sign(&mut t1, &kp, false);
    let t1h = t1.hash();
    let b1 = block(&node, &kp, Hash::ZERO, 1, 1, vec![t1]);
    let b1h = b1.hash();
    let v1 = apply(&mut node, b1).await;
    assert!(v1.is_ok(), "anchored issuance rejected: {v1:?}");

    let change_in = FUNDING - fee1;
    let mut t2 = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![input(t1h, 0, &kp), input(t1h, 1, &kp)],
        outputs: vec![
            fa_supply(600, pkh, asset, 1_000),
            fa_supply(400, pkh, asset, 1_000),
            Output::normal(0, pkh),
        ],
        extra_data: vec![],
    };
    sign(&mut t2, &kp, true);
    let fee2 = t2.minimum_fee() + 10_000;
    t2.outputs[2].amount = change_in - fee2;
    sign(&mut t2, &kp, true);
    let t2h = t2.hash();
    let v2 = {
        let blk = block(&node, &kp, b1h, 2, 2, vec![t2]);
        apply(&mut node, blk).await
    };
    assert!(v2.is_ok(), "FA split transfer rejected: {v2:?}");
    let fa_total: u64 = node
        .state_db
        .iter_utxos()
        .iter()
        .filter(|(op, e)| op.tx_hash == t2h && e.output.output_type == OutputType::FungibleAsset)
        .map(|(_, e)| e.output.amount)
        .sum();
    assert_eq!(fa_total, 1_000, "FA outputs of the split not persisted");
}

// REQ-234-010 — Decision: a failure means the rule leaks below its activation height (consensus split on replay).
#[tokio::test]
async fn below_ah_phantom_transfer_keeps_pre_fix_verdict() {
    let (mut node, kp, _t) = make_node().await;
    node.config.network = Network::Testnet;
    assert!(Network::Testnet.params().inc_i_234_activation_height > 1);
    let (t1, _, _) = phantom_transfer(&node, &kp).await;
    let v = {
        let b = block(&node, &kp, Hash::ZERO, 1, 1, vec![t1]);
        let mut hdr = b.header.clone();
        let tp = doli_core::consensus::ConsensusParams::for_network(Network::Testnet);
        hdr.genesis_hash = tp.genesis_hash;
        hdr.timestamp = tp.genesis_time + tp.slot_duration;
        apply(&mut node, doli_core::Block::new(hdr, b.transactions)).await
    };
    assert!(
        v.is_ok(),
        "below AH the phantom-pool Transfer must keep the pre-fix (accept) verdict: {v:?}"
    );
}

// REQ-234-SEC-012 — Decision: a failure means a Pool-output Transfer admitted below the AH is packed into a block its own apply rejects (self-fork at the boundary).
#[tokio::test]
async fn builder_drops_w1_pool_output_transfer_admitted_below_ah() {
    const HEIGHT: u64 = 100_001;
    let (mut node, kp, _t) = make_node().await;
    let pkh = addr(&kp);
    let (bad, _, _) = phantom_transfer(&node, &kp).await;
    assert!(bad
        .outputs
        .iter()
        .any(|o| o.output_type == OutputType::Pool));
    let control_prev = Hash::from_bytes([0x73; 32]);
    seed(&node, control_prev, 0, Output::normal(FUNDING, pkh)).await;
    let mut control = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![input(control_prev, 0, &kp)],
        outputs: vec![Output::normal(0, pkh)],
        extra_data: vec![],
    };
    control.outputs[0].amount = FUNDING - control.minimum_fee();
    sign(&mut control, &kp, false);
    let (bad_h, control_h) = (bad.hash(), control.hash());
    let testnet_ah = Network::Testnet.params().inc_i_234_activation_height;
    {
        let mut mp = node.mempool.write().await;
        *mp = mempool::Mempool::new(
            mempool::MempoolPolicy::testnet(),
            node.params.clone(),
            Network::Testnet,
        );
        mp.add_system_transaction(bad, testnet_ah - 1)
            .expect("fixture: below-AH admission accepts the W1 Transfer");
        mp.add_system_transaction(control, testnet_ah - 1)
            .expect("fixture: below-AH admission accepts the control");
    }
    let our_pubkey = *kp.public_key();
    let mut txs = None;
    for _ in 0..12 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        let slot = node.params.timestamp_to_slot(now);
        let built = node
            .build_block_content(Hash::ZERO, slot.saturating_sub(1), HEIGHT, slot, our_pubkey)
            .await
            .expect("build_block_content");
        if let Some((_, t, _)) = built {
            txs = Some(t);
            break;
        }
    }
    let txs = txs.expect("fixture: 12 consecutive slot-boundary aborts");
    let has = |h: Hash| txs.iter().any(|t| t.hash() == h);
    assert!(
        has(control_h),
        "fixture: honest control tx not included — builder test is vacuous"
    );
    assert!(
        !has(bad_h),
        "builder included the W1 Pool-output Transfer that block validation at the AH rejects"
    );
    let user_txs: Vec<Transaction> = txs.into_iter().filter(|t| !t.is_coinbase()).collect();
    let v = {
        let blk = block(&node, &kp, Hash::ZERO, 1, 1, user_txs);
        apply(&mut node, blk).await
    };
    assert!(
        v.is_ok(),
        "block built from the builder's selection does not apply: {v:?}"
    );
}
