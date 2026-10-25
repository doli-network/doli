//! INC-I-262 M2 / REQ-DEF-003 — an own slash whose including block is undone
//! is re-offered on a chain with missed slots after the accused's registration,
//! and the four paths agree on it. Devnet child processes.
//!
// covers: slash_redeliver slash_admission slash_window validation_checks assembly
//
// OUTPUT CONTRACT: fn Node::run_periodic_tasks(&mut self) -> Result<()> (-> redeliver_slashes)
//   O1 mempool: SlashProducer entries naming the accused (ids)
//   O2 self.slash_reported ledger (crate-private; observed through what the tick re-offers)
//   O3 canonical chain: SlashProducer txs naming the accused after the next block
// OUTPUT CONTRACT: fn validate_slash_target(&Transaction, &ValidationContext) -> Result<(), ValidationError>
//   O4 verdict seen by mempool admission, pool revalidation, builder selection, apply Light, apply Replay
// PATHS: P-RB rollback + missed-slot block, tick | P-RG reorg onto a sparse sibling, tick
//        | P-4 four paths on the sparse tip | P-PRE offence slot below the registration slot
//        | P-BELOW slashing gate u64::MAX
// INPUT PARTITIONS (registration block 21 at slot 21, offence slot 28, sparse tip 29 at slot 44,
//   judged height 30 slot 45): age 17 > blocks since registration 9; age <= height - gate 30;
//   age < unbonding_period 60; offence slot 28 >= registration slot | 20 < registration slot
// MATRIX:
//   O1,O2,O3 x P-RB    -> inc_i_262_m2_reoffer_after_rollback_with_missed_slots_mined_once
//   O1,O2,O3 x P-RG    -> inc_i_262_m2_reoffer_after_reorg_to_sparse_sibling_mined_once
//   O4 x P-4           -> inc_i_262_m2_offence_after_registration_accepted_on_every_path
//   O4 x P-PRE         -> inc_i_262_m2_offence_before_registration_slot_refused_on_every_path
//   O1,O4 x P-BELOW    -> inc_i_262_m2_below_gate_verdicts_unchanged

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::{Block, TxType};
use network::sync::ReorgResult;
use network::EquivocationProof;
use storage::ProducerStatus;

use super::inc_i_182_m1_node_fx::{producer_tx, px, Form, Px};
use super::inc_i_255_m1_common::{
    canonical, chain_genesis, evidence_header, full_block_with, mempool_slash_msg,
};
use super::inc_i_255_m2_common::{addr, queued_slash, Keys};
use super::inc_i_255_m4_common::{admit_at, revalidate_at};
use super::inc_i_256_m1_slash_fx::{offer, resident, tick, Offence};
use super::inc_i_256_m2_common::*;
use super::inc_i_257_m2_common::mine;

const MOD: &str = "inc_i_262_m2_reorg_missed_slots::";
const REG_HEIGHT: u64 = 21;
const OFFENCE: u32 = 28;
const SPARSE_SLOT: u32 = 44;
const BELOW_GATE: u64 = u64::MAX;

fn env(gate: Option<u64>) -> Vec<(&'static str, String)> {
    let mut e = vec![(GENESIS_ENV, "0".to_string())];
    e.extend(gate.map(|g| (GATE_ENV, g.to_string())));
    e
}

fn offence_at(kp: &KeyPair, slot: u32) -> Offence {
    let g = chain_genesis();
    let a = evidence_header(kp, g, slot, 0xA1, 0);
    let b = evidence_header(kp, g, slot, 0xB2, 0);
    let accused = *kp.public_key();
    let proof = EquivocationProof {
        producer: accused,
        block_header_1: a.clone(),
        block_header_2: b.clone(),
        slot,
    };
    Offence {
        accused,
        proof,
        tx: canonical(accused, a, b),
    }
}

async fn bond_slots(p: &Px, pk: &PublicKey) -> Vec<Option<u32>> {
    let utxo = p.r.node.utxo_set.read().await;
    utxo.get_by_pubkey_hash(&addr(pk))
        .into_iter()
        .filter(|(_, e)| e.output.output_type == doli_core::transaction::OutputType::Bond)
        .map(|(_, e)| e.output.bond_creation_slot())
        .collect()
}

/// Newcomer registered by a real Register tx at `REG_HEIGHT`; dense chain to `OFFENCE`.
async fn registered(gate: u64) -> Px {
    let mut p = px(&Keys::new()).await;
    assert_eq!(
        p.r.node
            .config
            .network
            .params()
            .effective_slashing_activation_height(),
        gate,
        "FIXTURE: child slashing gate"
    );
    dense_to(&mut p.r, REG_HEIGHT - 1).await;
    let reg = producer_tx(&p, TxType::Registration, Form::Signed, &p.genesis());
    mine(&mut p.r, vec![reg], "the newcomer's registration").await;
    dense_to(&mut p.r, u64::from(OFFENCE)).await;
    let newcomer = *p.newcomer.public_key();
    let info = {
        let ps = p.r.node.producer_set.read().await;
        ps.get_by_pubkey(&newcomer)
            .map(|i| (i.status, i.registered_at))
    };
    assert_eq!(
        info,
        Some((ProducerStatus::Active, REG_HEIGHT)),
        "FIXTURE: the newcomer is Active, registered at the Register block"
    );
    let reg_slot =
        p.r.node
            .block_store
            .get_block_by_height(REG_HEIGHT)
            .expect("FIXTURE: block store read")
            .expect("FIXTURE: registration block stored")
            .header
            .slot;
    assert_eq!(
        bond_slots(&p, &newcomer).await,
        vec![Some(reg_slot)],
        "PREMISE: the registration Bond UTXO is stamped with the registration block's slot"
    );
    p
}

/// The newcomer's slash offered at the dense tip, then mined mid-epoch.
async fn mined(p: &mut Px) -> (Offence, Block, Block) {
    let o = offence_at(&p.newcomer, OFFENCE);
    offer(&mut p.r.node, &o).await;
    assert_eq!(
        resident(&p.r.node, &o).await,
        vec![o.id()],
        "PRECONDITION: the node's own slash path admits the slash at the dense tip"
    );
    let tip_slot = p.r.best().await.1;
    let sibling = p.r.next_block(Vec::new(), SPARSE_SLOT - tip_slot - 1).await;
    let including = p.r.step(vec![o.tx.clone()]).await;
    assert!(
        queued_slash(&p.r.node, &o.accused).await,
        "PRECONDITION: mid-epoch the mined slash is queued"
    );
    (o, including, sibling)
}

async fn mined_on_chain(p: &Px, pk: &PublicKey) -> usize {
    let tip = p.r.best().await.0;
    let mut n = 0;
    for h in 1..=tip {
        let b =
            p.r.node
                .block_store
                .get_block_by_height(h)
                .expect("FIXTURE: block store read")
                .unwrap_or_else(|| panic!("FIXTURE: canonical block {h} stored"));
        n += b
            .transactions
            .iter()
            .filter(|t| t.slash_data().is_some_and(|d| d.producer_pubkey == *pk))
            .count();
    }
    n
}

async fn assert_sparse_tip(p: &Px, o: &Offence) {
    let (next, judged_slot) = judged(&p.r).await;
    let age = u64::from(judged_slot - OFFENCE);
    assert_eq!(
        (next, judged_slot),
        (REG_HEIGHT + 9, SPARSE_SLOT + 1),
        "FIXTURE: sparse tip"
    );
    assert!(
        age > next - REG_HEIGHT && age <= next && age < 60,
        "FIXTURE: age {age} > blocks since registration {}, <= height {next}, < period",
        next - REG_HEIGHT
    );
    assert!(
        !queued_slash(&p.r.node, &o.accused).await,
        "PRECONDITION: the undo removed the queued Slash"
    );
}

/// One tick re-offers the slash; revalidation keeps it; the builder selects
/// it; one more block mines it; exactly one slash on the final chain.
async fn assert_reoffered_and_mined_once(p: &mut Px, o: &Offence, what: &str) {
    tick(&mut p.r.node).await;
    let after = resident(&p.r.node, o).await;
    if after != vec![o.id()] {
        let tip = p.r.best().await.0;
        let why = admit_at(&p.r.node, &o.tx, tip).await;
        panic!(
            "{what}: the tick must re-offer the slash; resident {after:?}; admission now: {:?}",
            mempool_slash_msg(&why).unwrap_or_else(|| format!("{why:?}"))
        );
    }
    let tip = p.r.best().await.0;
    revalidate_at(&p.r.node, tip).await;
    assert_eq!(
        resident(&p.r.node, o).await,
        vec![o.id()],
        "{what}: pool revalidation must keep the re-offered slash"
    );
    let (_, selected) = full_block_with(&mut p.r.node, &[]).await;
    assert!(
        selected.contains(&o.id()),
        "{what}: the builder must select the re-offered slash"
    );
    let block = p.r.next_block(vec![o.tx.clone()], 0).await;
    let res = p.r.apply(block).await;
    assert!(
        res.is_ok(),
        "{what}: apply of the re-offered slash: {res:?}"
    );
    assert!(
        queued_slash(&p.r.node, &o.accused).await,
        "{what}: the re-offered slash is mined and queued"
    );
    tick(&mut p.r.node).await;
    assert!(
        resident(&p.r.node, o).await.is_empty(),
        "{what}: queued again, the slash is not re-admitted"
    );
    assert_eq!(
        mined_on_chain(p, &o.accused).await,
        1,
        "{what}: exactly one mined slash for the offence on the final chain"
    );
}

// REQ-DEF-003 — Decision: a failure means a rollback of the block that mined this node's slash, followed by missed slots, makes the ledger refuse it as "predates" and forget it, so a valid offence inside the registration goes unpunished (INV-SLASH-005).
#[tokio::test]
async fn inc_i_262_m2_reoffer_after_rollback_with_missed_slots_mined_once() {
    const TEST: &str = "inc_i_262_m2_reoffer_after_rollback_with_missed_slots_mined_once";
    if !in_child() {
        return child(&format!("{MOD}{TEST}"), &env(None));
    }
    let mut p = registered(0).await;
    let (o, _, _) = mined(&mut p).await;
    p.r.rollback().await;
    block_at_slot(&mut p.r, SPARSE_SLOT).await;
    assert_sparse_tip(&p, &o).await;
    assert_reoffered_and_mined_once(&mut p, &o, "rollback + missed slots").await;
}

// REQ-DEF-003 — Decision: a failure means a reorg onto a sibling with missed slots loses this node's slash for good through a false "predates" refusal.
#[tokio::test]
async fn inc_i_262_m2_reoffer_after_reorg_to_sparse_sibling_mined_once() {
    const TEST: &str = "inc_i_262_m2_reoffer_after_reorg_to_sparse_sibling_mined_once";
    if !in_child() {
        return child(&format!("{MOD}{TEST}"), &env(None));
    }
    let mut p = registered(0).await;
    let (o, including, sibling) = mined(&mut p).await;
    let reorg = ReorgResult {
        rollback: vec![including.hash()],
        common_ancestor: including.header.prev_hash,
        new_blocks: vec![sibling.hash()],
        weight_delta: 1,
    };
    p.r.node
        .execute_reorg(reorg, sibling.clone())
        .await
        .unwrap_or_else(|e| panic!("FIXTURE: execute_reorg failed: {e:#}"));
    let hash: Hash = p.r.best().await.2;
    assert_eq!(
        hash,
        sibling.hash(),
        "PRECONDITION: the reorg replaced the including block"
    );
    assert_sparse_tip(&p, &o).await;
    assert_reoffered_and_mined_once(&mut p, &o, "reorg to a sparse sibling").await;
}

// REQ-DEF-003 — Decision: a failure means mempool, builder, apply and replay disagree on (or all refuse) an offence after the accused's registration once missed slots follow it, so the verdict depends on chain density instead of the registration.
#[tokio::test]
async fn inc_i_262_m2_offence_after_registration_accepted_on_every_path() {
    const TEST: &str = "inc_i_262_m2_offence_after_registration_accepted_on_every_path";
    if !in_child() {
        return child(&format!("{MOD}{TEST}"), &env(None));
    }
    let mut p = registered(0).await;
    block_at_slot(&mut p.r, SPARSE_SLOT).await;
    let o = offence_at(&p.newcomer, OFFENCE);
    assert_sparse_tip(&p, &o).await;
    let probe = four_paths(&mut p.r, &o.tx, 0).await;
    print_invalid_accepted("after_registration_sparse", &probe);
    assert_accepted_everywhere(
        "offence after the registration, missed slots after it",
        &probe,
    );
}

// REQ-DEF-003 — Decision: a failure means evidence from before the accused's registration slot slashes it on some path once the floor is read from the Bond UTXO slot (a wrongful burn), or the paths split on it.
#[tokio::test]
async fn inc_i_262_m2_offence_before_registration_slot_refused_on_every_path() {
    const TEST: &str = "inc_i_262_m2_offence_before_registration_slot_refused_on_every_path";
    if !in_child() {
        return child(&format!("{MOD}{TEST}"), &env(None));
    }
    let mut p = registered(0).await;
    block_at_slot(&mut p.r, SPARSE_SLOT).await;
    let before = offence_at(&p.newcomer, REG_HEIGHT as u32 - 1);
    let age = age_at_next(&p.r, &before.tx).await;
    assert!(
        age <= judged(&p.r).await.0 && age < 60,
        "FIXTURE: age {age} inside the gate bound and the period"
    );
    let probe = four_paths(&mut p.r, &before.tx, 0).await;
    print_invalid_accepted("before_registration_slot", &probe);
    assert!(
        probe.resident,
        "FIXTURE: the builder judged a resident slash"
    );
    assert_refused_everywhere("offence before the registration slot", &probe, PRE_TENURE);
}

// REQ-DEF-003 — Decision: a failure means the floor change altered a slash verdict below the slashing gate, a consensus change on heights already mined.
#[tokio::test]
async fn inc_i_262_m2_below_gate_verdicts_unchanged() {
    const TEST: &str = "inc_i_262_m2_below_gate_verdicts_unchanged";
    if !in_child() {
        return child(&format!("{MOD}{TEST}"), &env(Some(BELOW_GATE)));
    }
    let mut p = registered(BELOW_GATE).await;
    let o = offence_at(&p.newcomer, OFFENCE);
    offer(&mut p.r.node, &o).await;
    assert!(
        resident(&p.r.node, &o).await.is_empty(),
        "below the gate the node's own slash path admits nothing"
    );
    block_at_slot(&mut p.r, SPARSE_SLOT).await;
    tick(&mut p.r.node).await;
    assert!(
        resident(&p.r.node, &o).await.is_empty(),
        "below the gate the ledger re-offers nothing"
    );
    let probe = four_paths(&mut p.r, &o.tx, 0).await;
    let [mempool, builder, apply, replay] = &probe.v;
    assert_eq!(
        mempool,
        &Verdict::Refused("Err(FeeTooLow(0, 1))".to_string()),
        "below-gate mempool verdict"
    );
    assert!(
        !builder.accepted(),
        "below the gate the builder selects no slash"
    );
    assert!(
        apply.names("insufficient fee: got 0, minimum 1"),
        "below-gate apply verdict; got {apply:?}"
    );
    assert_eq!(replay, &Verdict::Accepted, "below-gate replay verdict");
}
