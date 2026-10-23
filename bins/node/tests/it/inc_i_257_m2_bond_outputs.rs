//! INC-I-257 M2 — node paths of the Bond-output rule at the devnet gate (0):
//! a Transfer with a Bond output and an AddBond with a foreign-owner Bond are
//! refused by mempool, never selected by the builder, and invalid in a block;
//! the well-formed twins still pass. Plus the bounded slash-target cost scan.
//!
// covers: output_authority tx_types transaction pool assembly tx_processing slash_admission slash_burn
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(..) via the node's mempool -> O1 return, O2 residency
// OUTPUT CONTRACT: fn Node::build_block_content(..) -> O3 whether the tx is selected
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> O4 return, O5 tip moved
// OUTPUT CONTRACT: fn source_scan(file, fn) -> O6 owner lookups in slash_targets, O7 burn selection calls
// PATHS: P-TRANSFER Transfer creating a Bond | P-FOREIGN AddBond Bond owned by another hash
//        | P-TWIN the same spend paying Normal / the AddBond owned by the producer | P-SCAN source scan
// INPUT PARTITIONS:
//   tx type: Transfer | AddBond; Bond owner: own | foreign; output type of the twin: Normal | own Bond
//   scanned bodies: slash_targets (mempool) | slash_burn_in_batch | slash_burn_in_utxo_set
// MATRIX:
//   O1-O5 x {P-TRANSFER, P-TWIN} -> inc_i_257_m2_req_unbond_013_node_refuses_transfer_with_bond_output
//   O1-O5 x {P-FOREIGN, P-TWIN} -> inc_i_257_m2_req_unbond_013_node_refuses_addbond_with_foreign_owner
//   O6 x P-SCAN -> inc_i_257_m2_req_unbond_sec_004_one_owner_lookup_per_slash
//   O7 x P-SCAN -> inc_i_257_m2_req_unbond_010_both_burn_paths_select_owner_by_burnable

use std::path::Path;

use crypto::{Hash, KeyPair};
use doli_core::transaction::{Input, Transaction};
use storage::Outpoint;

use super::inc_i_255_m1_common::full_block_with;
use super::inc_i_255_m2_common::{addr, outpoints, rig, Keys, Rig, COIN, COIN_TAG, EPOCH};
use super::inc_i_255_m4_common::admit_at;
use super::inc_i_255_m6_common::{bond_maker, seed_coin, transfer, FEE_OUTBID};
use super::inc_i_257_m1_common::FEE;
use super::inc_i_257_m2_common::{block_effect, Effect};

#[derive(Debug)]
struct Verdict {
    mempool: String,
    resident: bool,
    selected: bool,
    block: Effect,
}

async fn verdict(r: &mut Rig, tx: &Transaction) -> Verdict {
    let tip = r.best().await.0;
    let admitted = admit_at(&r.node, tx, tip).await;
    let resident = r.node.mempool.read().await.contains(&tx.hash());
    let (_, selected) = full_block_with(&mut r.node, &[]).await;
    r.node.mempool.write().await.remove_transaction(&tx.hash());
    let block = block_effect(r, vec![tx.clone()]).await;
    Verdict {
        mempool: format!("{admitted:?}"),
        resident,
        selected: selected.contains(&tx.hash()),
        block,
    }
}

fn assert_refused(v: &Verdict, case: &str) {
    assert!(
        v.mempool.starts_with("Err") && !v.resident,
        "{case}: mempool must refuse; {v:?}"
    );
    assert!(!v.selected, "{case}: the builder must not select it; {v:?}");
    assert!(
        v.block.applied.starts_with("Err") && !v.block.tip_moved,
        "{case}: a block holding it must be invalid; {v:?}"
    );
}

fn assert_accepted(v: &Verdict, case: &str) {
    assert!(
        v.mempool.starts_with("Ok"),
        "NON-VACUITY {case}: mempool admits; {v:?}"
    );
    assert!(
        v.block.applied == "Ok" && v.block.tip_moved,
        "NON-VACUITY {case}: the block applies; {v:?}"
    );
}

async fn armed_rig() -> Rig {
    assert_eq!(
        doli_core::Network::Devnet
            .params()
            .inc_i_257_unbonding_activation_height,
        0,
        "FIXTURE: devnet gate"
    );
    let mut r = rig(&Keys::new()).await;
    r.advance_to(EPOCH).await;
    r
}

// REQ-UNBOND-013 — Decision: a failure means a node relays, builds or applies a Transfer that mints a Bond output (one path out of step with the others is the INC-I-173 builder/apply split).
#[tokio::test]
async fn inc_i_257_m2_req_unbond_013_node_refuses_transfer_with_bond_output() {
    let mut r = armed_rig().await;
    let coin = outpoints(COIN_TAG, 1)[0];
    let pk = r.accused_pk();
    let tx = bond_maker(&r.keys.accused, coin, COIN, &pk, 10_000);
    let v = verdict(&mut r, &tx).await;
    assert_refused(&v, "Transfer with a Bond output");
    let twin = transfer(&r.keys.accused, coin, COIN, FEE);
    assert_accepted(
        &verdict(&mut r, &twin).await,
        "the same coin spent to a Normal output",
    );
}

fn add_bond(r: &Rig, kp: &KeyPair, coin: Outpoint, owner: Hash, lock: u64) -> Transaction {
    let unit = r.unit();
    let mut input = Input::new(coin.tx_hash, coin.index);
    input.public_key = Some(*kp.public_key());
    let mut tx = Transaction::new_add_bond(vec![input], *kp.public_key(), 1, unit, lock);
    tx.outputs[0].pubkey_hash = owner;
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    tx
}

// REQ-UNBOND-013 — Decision: a failure means a registered producer can add bond weight held under a foreign owner, which the slash of that producer never burns.
#[tokio::test]
async fn inc_i_257_m2_req_unbond_013_node_refuses_addbond_with_foreign_owner() {
    let mut r = armed_rig().await;
    let kp = r.keys.honest.clone();
    let pk = *kp.public_key();
    let coin = seed_coin(&r.node, &pk, 0xE1, r.unit() + FEE_OUTBID).await;
    let lock = r.best().await.0 + r.node.params.blocks_per_era + 1_000;
    let foreign = add_bond(&r, &kp, coin, Hash::from_bytes([0x99; 32]), lock);
    let v = verdict(&mut r, &foreign).await;
    assert_refused(&v, "AddBond with a foreign-owner Bond");
    let own = add_bond(&r, &kp, coin, addr(&pk), lock);
    assert_accepted(&verdict(&mut r, &own).await, "the producer's own AddBond");
}

fn source(rel: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// Text of the fn whose signature starts with `sig`, up to the first line that is `}` at its indent.
fn fn_body<'a>(text: &'a str, sig: &str) -> &'a str {
    let start = text
        .find(sig)
        .unwrap_or_else(|| panic!("NON-VACUITY: `{sig}` not found"));
    let line = text[..start].rsplit('\n').next().unwrap_or("");
    let indent = line.len() - line.trim_start().len();
    let close = format!("\n{}}}\n", " ".repeat(indent));
    let end = text[start..]
        .find(&close)
        .map(|i| start + i + close.len())
        .unwrap_or(text.len());
    &text[start..end]
}

// REQ-UNBOND-SEC-004 — Decision: a failure means each SlashProducer costs more than one owner prefix scan (or a full UTXO scan), so block validation cost per slash is no longer bounded.
#[test]
fn inc_i_257_m2_req_unbond_sec_004_one_owner_lookup_per_slash() {
    let src = source("crates/mempool/src/slash_admission.rs");
    let body = fn_body(&src, "pub fn slash_targets");
    assert_eq!(
        body.matches("get_by_pubkey_hash(").count(),
        1,
        "slash_targets must do exactly one owner lookup per SlashProducer:\n{body}"
    );
    for banned in ["iter_all(", "get_bond_entries(", "canonical_digest("] {
        assert!(
            !body.contains(banned),
            "slash_targets must not call {banned}:\n{body}"
        );
    }
}

// REQ-UNBOND-010 — Decision: a failure means the apply and rebuild burn paths select different sets (rebuild still Bond-only through get_bond_entries), so a rebuilt node diverges from one that applied the slash.
#[test]
fn inc_i_257_m2_req_unbond_010_both_burn_paths_select_owner_by_burnable() {
    let src = source("bins/node/src/node/apply_block/slash_burn.rs");
    assert!(
        !src.contains("get_bond_entries("),
        "slash_burn.rs must not select by get_bond_entries"
    );
    for f in ["fn slash_burn_in_batch", "fn slash_burn_in_utxo_set"] {
        let body = fn_body(&src, f);
        assert!(
            body.contains("is_slash_burnable("),
            "{f} must filter the owner's UTXOs with is_slash_burnable:\n{body}"
        );
    }
}
