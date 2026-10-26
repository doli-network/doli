//! INC-I-182 M1 — below the producer-auth gate, a fixed chain carrying an
//! unsigned RequestWithdrawal, AddBond and Registration across an epoch boundary
//! gives the block hashes, state root and supply recorded on the base tree, and
//! a Replay twin reaches the same root.
//!
// covers: producer_auth transaction types unbonding defaults env_loader network_params pool assembly tx_processing validation_checks withdrawal_holdings genesis register bonds withdrawal exit unbonding_tx core data block rpc_client
//
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()> over a fixed 9-block chain
//   O1 return per block (Light on the main node, Replay on the twin)
//   O2 block hash per height; O3 state root at the tip; O4 UTXO supply at the tip; O5 twin root
// PATHS: child process, producer-auth gate env u64::MAX, genesis window env 0; fixed keys and slots
// MATRIX
//   O1-O5 x the fixed chain -> equal to the constants recorded on the base tree
//     (inc_i_182_m1_req_pauth_003_below_gate_chain_golden)
// INPUT PARTITIONS:
//   producer txs: unsigned legacy forms (68, 36, bincode bytes); heights 5-7 (epoch 1, deferred effects)
//   epoch boundary crossed at 8

use doli_core::transaction::TxType;
use doli_core::validation::ValidationMode;
use storage::Outpoint;

use super::inc_i_182_m1_node_fx::*;
use super::inc_i_255_m2_common::{live, snap, status, EPOCH};

const TEST: &str =
    "inc_i_182_m1_below_gate_golden::inc_i_182_m1_req_pauth_003_below_gate_chain_golden";

const GOLDEN_BLOCK_HASHES: [&str; 9] = [
    "9649d522141b84833529a692cd26f84e875acdbabdbfa97a95c1f018e96c7515",
    "4c8fa074dc1c2e502137ab49eb2ba8c05b2bdb4bd62584f793b98343be7bd28b",
    "6985cd4d207066143bf1a69ddc9b4ffd851ae9353f0ec21698dad236c0272237",
    "da272c1c63a6372ecfd97837a99d55ca79d50bfc792d049864a7a647e40b8468",
    "c13e97d83af789dbb3754b1aee29918a3848e3fac2ebf7caa9471068a8265017",
    "677810e38c372bb239e1bc7cb613d7c3dc210e4663b4c07129df84c544763860",
    "bec8b018ab64fe0170954d1f2f51fd57ce6558074ea94849bf836dd8946929df",
    "690cec7c4cf1ee85ecd1aa501f61ac8fe8b69c04954547a304f22da624238ef4",
    "84e84279cdefb7d8d9d2be50e6ca787cfa60748b447abb3e0a9a7bdc7a787f96",
];
const GOLDEN_STATE_ROOT: &str = "ed966cd2a85607cc61665ea4e840f182fcb9832a4e5f700274636e45fa4aef95";
const GOLDEN_SUPPLY: u64 = 1_800_776_999;

#[derive(Debug, PartialEq)]
struct Observed {
    hashes: Vec<String>,
    root: String,
    supply: u64,
    replay_root: String,
}

async fn fixed_chain() -> Observed {
    let mut p = px(&fixed_keys()).await;
    let g = p.genesis();
    let txs = [
        producer_tx(&p, TxType::RequestWithdrawal, Form::Unsigned, &g),
        producer_tx(&p, TxType::AddBond, Form::Unsigned, &g),
        producer_tx(&p, TxType::Registration, Form::Unsigned, &g),
    ];
    let created: Vec<Outpoint> = txs.iter().map(|t| Outpoint::new(t.hash(), 0)).collect();
    let mut blocks = Vec::new();
    for _ in 0..EPOCH {
        blocks.push(p.r.step(Vec::new()).await);
    }
    for tx in txs {
        blocks.push(p.r.step(vec![tx]).await);
    }
    for _ in 0..2 {
        blocks.push(p.r.step(Vec::new()).await);
    }
    assert_eq!(
        live(&p.r.node, &created).await,
        created,
        "NON-VACUITY: the payout and both Bond outputs exist"
    );
    assert!(
        status(&p.r.node, p.newcomer.public_key()).await.is_some(),
        "NON-VACUITY: the newcomer is in the producer set"
    );
    let s = snap(&p.r.node).await;
    let mut twin = px(&fixed_keys()).await;
    for (i, b) in blocks.iter().enumerate() {
        twin.r
            .node
            .apply_block(b.clone(), ValidationMode::Replay)
            .await
            .unwrap_or_else(|e| panic!("Replay of block #{}: {e:#}", i + 1));
    }
    Observed {
        hashes: blocks.iter().map(|b| b.hash().to_string()).collect(),
        root: s.root.to_string(),
        supply: s.supply,
        replay_root: snap(&twin.r.node).await.root.to_string(),
    }
}

fn golden() -> Observed {
    Observed {
        hashes: GOLDEN_BLOCK_HASHES.iter().map(|h| h.to_string()).collect(),
        root: GOLDEN_STATE_ROOT.to_string(),
        supply: GOLDEN_SUPPLY,
        replay_root: GOLDEN_STATE_ROOT.to_string(),
    }
}

// REQ-PAUTH-003 — Decision: a failure means the M1 change altered blocks, verdicts or state below its activation height, a retroactive consensus change that forks old and new nodes.
#[tokio::test]
async fn inc_i_182_m1_req_pauth_003_below_gate_chain_golden() {
    if !in_child() {
        return run_child(TEST, &env_at(u64::MAX));
    }
    let got = fixed_chain().await;
    println!("GOLDEN_OBSERVED {got:?}");
    assert_eq!(
        got,
        golden(),
        "below the gate the fixed chain must give the block hashes, state root and supply recorded on the base tree"
    );
}
