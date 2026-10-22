//! INC-I-257 M1 — below the slashing and unbonding gates, a fixed chain with
//! instant bond withdrawals, a spend of a payout in the next block, an epoch
//! boundary and a slash gives the block hashes and state root recorded on the
//! base tree.
//!
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()> over a fixed 10-block chain
//   O1 return per block (slash block: Light attempt refused, Replay apply Ok)
//   O2 block hash per height; O3 state root at the tip; O4 UTXO supply at the tip
// PATHS: child process, both gates env-set to u64::MAX (their mainnet value); fixed keys and slots
// MATRIX
//   O1-O4 x the fixed chain -> equal to the constants recorded on the base tree
// INPUT PARTITIONS:
//   withdrawal payout: spent in the next block | left unspent across the epoch boundary
//   slash block: Light (refused below the gate) | Replay (applied, burns nothing)

use crypto::KeyPair;
use doli_core::validation::ValidationMode;
use storage::Outpoint;

use super::inc_i_255_m2_common::*;
use super::inc_i_255_m6_common::transfer;
use super::inc_i_257_m1_common::{instant_withdrawal, FEE};

const TEST: &str =
    "inc_i_257_m1_below_gate_golden::inc_i_257_m1_req_unbond_017_below_gate_chain_golden";
const CHILD_ENV: &str = "INC_I_257_M1_GOLDEN_CHILD";
const GATE_ENVS: [&str; 2] = [
    "DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT",
    "DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT",
];

const GOLDEN_BLOCK_HASHES: [&str; 10] = [
    "9649d522141b84833529a692cd26f84e875acdbabdbfa97a95c1f018e96c7515",
    "4c8fa074dc1c2e502137ab49eb2ba8c05b2bdb4bd62584f793b98343be7bd28b",
    "6985cd4d207066143bf1a69ddc9b4ffd851ae9353f0ec21698dad236c0272237",
    "da272c1c63a6372ecfd97837a99d55ca79d50bfc792d049864a7a647e40b8468",
    "69b73a45612fbd7b13b80168e558667d532cd1cb80e6c07b5d6de601f6ca3e7a",
    "58c3cde0e16e44ee10d1aabcf1e0a0c3c0f184cccdb64ed3eb961c46180d7c7b",
    "4859daa89a49f75ada031b2c88b201f73ea2d3a0d07ab4fb82df115428989559",
    "b0c6026e22b88dc3e10017c53e8b3c12fd1060fa1b4e3cf8a1a631f8be04b410",
    "ad39d31794cd4655b93965ffc6d0d77143ffd46f4dfe8772f53df765e060a690",
    "50087af4948dbbcf7459cdf615adf430c3749441ba66aa2240d16c94f3e52bed",
];
const GOLDEN_STATE_ROOT: &str = "5b10e9fbbad9e937a4e9ec79a7da930e4d9ed35317aff65b47479b6d22ff6944";
const GOLDEN_SUPPLY: u64 = 1_700_776_997;

fn fixed_keys() -> Keys {
    Keys {
        honest: KeyPair::from_seed([0x11; 32]),
        accused: KeyPair::from_seed([0x22; 32]),
        delegator: KeyPair::from_seed([0x33; 32]),
    }
}

#[derive(Debug, PartialEq)]
struct Observed {
    hashes: Vec<String>,
    root: String,
    supply: u64,
    slash_light_refused: bool,
}

async fn fixed_chain() -> Observed {
    let mut r = rig(&fixed_keys()).await;
    let net = r.unit() - FEE;
    let mut hashes = Vec::new();
    for _ in 0..EPOCH {
        hashes.push(r.step(Vec::new()).await.hash());
    }
    let rw1 = instant_withdrawal(&r.keys.honest, &outpoints(HONEST_TAG, 1), net);
    let spend = transfer(&r.keys.honest, Outpoint::new(rw1.hash(), 0), net, FEE);
    let rw2 = instant_withdrawal(&r.keys.accused, &outpoints(ACCUSED_TAG, 1), net);
    hashes.push(r.step(vec![rw1]).await.hash());
    hashes.push(r.step(vec![spend]).await.hash());
    hashes.push(r.step(vec![rw2]).await.hash());
    hashes.push(r.step(Vec::new()).await.hash());
    let slash_block = r.next_block(vec![r.slash()], 0).await;
    let light = r.apply(slash_block.clone()).await;
    r.node
        .apply_block(slash_block.clone(), ValidationMode::Replay)
        .await
        .expect("FIXTURE: below the gate Replay applies the slash block");
    hashes.push(slash_block.hash());
    hashes.push(r.step(Vec::new()).await.hash());
    let s = snap(&r.node).await;
    Observed {
        hashes: hashes.iter().map(|h| h.to_string()).collect(),
        root: s.root.to_string(),
        supply: s.supply,
        slash_light_refused: light.is_err(),
    }
}

fn golden() -> Observed {
    Observed {
        hashes: GOLDEN_BLOCK_HASHES.iter().map(|h| h.to_string()).collect(),
        root: GOLDEN_STATE_ROOT.to_string(),
        supply: GOLDEN_SUPPLY,
        slash_light_refused: true,
    }
}

fn run_child() -> String {
    let mut cmd = std::process::Command::new(std::env::current_exe().expect("current exe"));
    cmd.args([TEST, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1");
    for env in GATE_ENVS {
        cmd.env(env, u64::MAX.to_string());
    }
    let out = cmd.output().expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child at both gates u64::MAX failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

// REQ-UNBOND-017 — Decision: a failure means the M1 change altered blocks or state below its activation height, a retroactive consensus change.
#[tokio::test]
async fn inc_i_257_m1_req_unbond_017_below_gate_chain_golden() {
    if std::env::var_os(CHILD_ENV).is_none() {
        let stdout = run_child();
        let line = stdout
            .lines()
            .find_map(|l| l.find("GOLDEN_OBSERVED").map(|i| &l[i..]));
        println!("{}", line.unwrap_or("GOLDEN_OBSERVED line missing"));
        return;
    }
    let got = fixed_chain().await;
    println!("GOLDEN_OBSERVED {got:?}");
    assert_eq!(
        got,
        golden(),
        "below both gates the fixed chain must give the block hashes, state root and supply recorded on the base tree"
    );
}
