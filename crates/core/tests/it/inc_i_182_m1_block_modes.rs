//! INC-I-182 M1 — core block validation gives one verdict and one code for a
//! producer tx in Full, Light and Replay mode.
//!
// covers: producer_auth transaction types unbonding defaults env_loader network_params pool assembly tx_processing validation_checks withdrawal_holdings genesis register bonds withdrawal exit unbonding_tx core data block rpc_client
//
// OUTPUT CONTRACT: fn validate_block_with_mode(&Block, &ValidationContext, ValidationMode) -> Result<(), ValidationError>
//   O1 return: Ok | Err; Err text holds [ERRTX-PAUTH001] for an unauthorized producer tx; pure
// PATHS: P-FULL | P-LIGHT | P-REPLAY; height gate - 1 | gate
// MATRIX
//   O1 x 3 kinds x unsigned x {P-FULL, P-LIGHT, P-REPLAY} x gate -> coded
//   O1 x 3 kinds x unsigned x {P-LIGHT, P-REPLAY} x gate - 1 -> Ok; P-FULL x gate - 1 -> not coded
//   O1 x 3 kinds x signed x gate -> Light/Replay Ok, Full not coded (fails later on the header VDF)
//   all cells -> inc_i_182_m1_req_pauth_002_block_modes_same_code
// INPUT PARTITIONS:
//   block: one producer tx, header valid for the time checks, fake VDF (Full stops after the tx loop)

use crypto::Hash;
use doli_core::block::{compute_merkle_root, Block, BlockHeader};
use doli_core::transaction::Transaction;
use doli_core::validation::{validate_block_with_mode, ValidationContext, ValidationMode};
use vdf::{VdfOutput, VdfProof};

use super::inc_i_182_m1_core_fx::*;

const TAG: u8 = 0x40;
const T: u64 = 100;

pub fn block_of(tx: Transaction) -> Block {
    let txs = vec![tx];
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::from_bytes([0x01; 32]),
        merkle_root: compute_merkle_root(&txs),
        presence_root: Hash::ZERO,
        genesis_hash: genesis(),
        timestamp: T,
        slot: T as u32,
        producer: pk(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, txs)
}

pub fn block_ctx(height: u64) -> ValidationContext {
    let mut c = ctx(height, GATE);
    c.current_time = T;
    c
}

// REQ-PAUTH-002 — Decision: a failure means Full, Light and Replay block validation judge the same producer tx differently, so a node that syncs and a node that validates live fork on that block.
#[test]
fn inc_i_182_m1_req_pauth_002_block_modes_same_code() {
    let mut bad = Vec::new();
    for kind in KINDS {
        let unsigned_block = block_of(unsigned(kind, TAG));
        let signed_block = block_of(signed(kind, TAG));
        for mode in [
            ValidationMode::Full,
            ValidationMode::Light,
            ValidationMode::Replay,
        ] {
            let full = matches!(mode, ValidationMode::Full);
            let below = validate_block_with_mode(&unsigned_block, &block_ctx(GATE - 1), mode);
            if coded(&below) || (!full && below.is_err()) {
                bad.push(format!(
                    "{kind:?} unsigned below the gate, {mode:?}: {below:?}"
                ));
            }
            let at = validate_block_with_mode(&unsigned_block, &block_ctx(GATE), mode);
            if !coded(&at) {
                bad.push(format!("{kind:?} unsigned at the gate, {mode:?}: {at:?}"));
            }
            let ok = validate_block_with_mode(&signed_block, &block_ctx(GATE), mode);
            if coded(&ok) || (!full && ok.is_err()) {
                bad.push(format!("{kind:?} signed at the gate, {mode:?}: {ok:?}"));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "block verdicts per mode:\n{}",
        bad.join("\n")
    );
}
