// covers: slash_state slash_window slash_evidence slash_admission pool unbonding entry error assembly tx_processing validation_checks mod lib block transaction types set_registration set_lifecycle defaults network_params
//! INC-I-256 M2 / R-4 — the tenure floor: evidence whose offence slot is not
//! after max(registered_at, effective slashing gate) is refused with one error
//! on mempool, builder, apply and replay; an offence inside the tenure is
//! accepted on all four. Devnet child processes.
//!
// OUTPUT CONTRACT: fn validate_slash_target(tx, ctx) -> Result<(), ValidationError>
//   O1/O2: none (pure; &Transaction, &ValidationContext)
//   O3: return — Ok | InvalidSlash(PRE_TENURE), observed through four callers:
//       mempool add_transaction, builder selection, apply_block Light and Replay
// PATHS: P1 accepted (offence inside the tenure) | P2 refused PRE_TENURE
// INPUT PARTITIONS (tenure = next height - floor, age = parent slot + 1 - offence slot):
//   floor = registered_at (real Register tx at R, gate 0): P1a offence after R | P2a offence before R
//   floor = gate G = 6 (genesis accused, registered_at 0): P1b offence after G | P2b offence before G
//   P1c dense chain, offence AT slot G (the activation block's own slot): age == tenure, judged
//       at G + 1 (tenure 1, earliest inclusion) and at G + 5 (tenure 5)
// MATRIX: O3 x 4 paths x 6 cells = 24 cells
//   P1a + P2a pre_registration_refused_on_every_path | P1b + P2b before_gate_refused_on_every_path
//   P1c x 2 heights slash_window_offence_at_gate_block_slot_accepted_on_every_path

use doli_core::TxType;
use storage::ProducerStatus;

use super::inc_i_182_m1_node_fx::{producer_tx, px, Form};
use super::inc_i_255_m2_common::{rig, Keys, Rig};
use super::inc_i_256_m2_common::*;
use super::inc_i_257_m2_common::mine;

const MOD: &str = "inc_i_256_m2_tenure::";
const GATE: u64 = 6;

async fn registered_at(r: &Rig, pk: &crypto::PublicKey) -> (Option<ProducerStatus>, Option<u64>) {
    let ps = r.node.producer_set.read().await;
    let info = ps.get_by_pubkey(pk);
    (info.map(|i| i.status), info.map(|i| i.registered_at))
}

// R-4 / AUDIT-P2-547 / AUDIT-P2-546 — Decision: a failure means evidence from before the accused's current registration slashes it on some path, so a new registration pays for an offence it did not commit, or the paths split.
#[tokio::test]
async fn pre_registration_refused_on_every_path() {
    if !in_child() {
        let env = [(GENESIS_ENV, "0".to_string())];
        return child(
            &format!("{MOD}pre_registration_refused_on_every_path"),
            &env,
        );
    }
    let mut p = px(&Keys::new()).await;
    dense_to(&mut p.r, 5).await;
    let reg = producer_tx(&p, TxType::Registration, Form::Signed, &p.genesis());
    mine(&mut p.r, vec![reg], "the newcomer's registration").await;
    let reg_height = p.r.best().await.0;
    dense_to(&mut p.r, 9).await;
    let newcomer = *p.newcomer.public_key();
    assert_eq!(
        registered_at(&p.r, &newcomer).await,
        (Some(ProducerStatus::Active), Some(reg_height)),
        "FIXTURE: the newcomer is Active, registered at the Register block"
    );
    let (next, _) = judged(&p.r).await;
    let tenure = next - reg_height;

    let inside = slash_at(&p.newcomer, 9);
    let age = age_at_next(&p.r, &inside).await;
    assert!(age < tenure, "FIXTURE: control age {age} < tenure {tenure}");
    let control = four_paths(&mut p.r, &inside, 0).await;
    assert_accepted_everywhere("offence inside the tenure", &control);

    let before = slash_at(&p.newcomer, reg_height as u32 - 1);
    let age = age_at_next(&p.r, &before).await;
    assert!(
        age >= tenure && age < 60,
        "FIXTURE: age {age} >= tenure {tenure}, inside the period"
    );
    let probe = four_paths(&mut p.r, &before, 0).await;
    print_invalid_accepted("pre_tenure", &probe);
    assert!(
        probe.resident,
        "FIXTURE: the builder judged a resident slash"
    );
    assert_refused_everywhere("offence before the registration", &probe, PRE_TENURE);
}

// R-4 / AUDIT-P2-547 — Decision: a failure means an offence from before the slashing gate is punishable on some path (retroactive punishment), or an offence after it is refused.
#[tokio::test]
async fn before_gate_refused_on_every_path() {
    if !in_child() {
        let env = [(GATE_ENV, GATE.to_string())];
        return child(&format!("{MOD}before_gate_refused_on_every_path"), &env);
    }
    let mut r = rig(&Keys::new()).await;
    assert_eq!(
        r.node
            .config
            .network
            .params()
            .effective_slashing_activation_height(),
        GATE,
        "FIXTURE: child slashing gate"
    );
    let accused = r.accused_pk();
    assert_eq!(
        registered_at(&r, &accused).await.1,
        Some(0),
        "FIXTURE: genesis accused"
    );
    dense_to(&mut r, 10).await;
    let (next, _) = judged(&r).await;
    let tenure = next - GATE;

    let after = slash_at(&r.keys.accused, 9);
    let age = age_at_next(&r, &after).await;
    assert!(age < tenure, "FIXTURE: control age {age} < tenure {tenure}");
    let control = four_paths(&mut r, &after, 0).await;
    assert_accepted_everywhere("offence after the gate", &control);

    let before = slash_at(&r.keys.accused, GATE as u32 - 1);
    let age = age_at_next(&r, &before).await;
    assert!(
        age >= tenure && age < 60,
        "FIXTURE: age {age} >= tenure {tenure}, inside the period"
    );
    let probe = four_paths(&mut r, &before, 0).await;
    assert!(
        probe.resident,
        "FIXTURE: the builder judged a resident slash"
    );
    assert_refused_everywhere("offence before the gate", &probe, PRE_TENURE);
}

// R-4 / AUDIT-P2-547 / INC-I-256 — Decision: a failure means an equivocation in the activation block's own slot is refused on some path at every later height (the GS-027 live failure: detected, never mined), or the four paths split on it.
#[tokio::test]
async fn slash_window_offence_at_gate_block_slot_accepted_on_every_path() {
    const TEST: &str = "slash_window_offence_at_gate_block_slot_accepted_on_every_path";
    if !in_child() {
        let env = [(GATE_ENV, GATE.to_string())];
        return child(&format!("{MOD}{TEST}"), &env);
    }
    let mut r = rig(&Keys::new()).await;
    assert_eq!(
        r.node
            .config
            .network
            .params()
            .effective_slashing_activation_height(),
        GATE,
        "FIXTURE: child slashing gate"
    );
    let accused = r.accused_pk();
    assert_eq!(
        registered_at(&r, &accused).await.1,
        Some(0),
        "FIXTURE: genesis accused, floor = gate"
    );
    dense_to(&mut r, GATE).await;
    let tx = slash_at(&r.keys.accused, GATE as u32);

    for tip in [GATE, GATE + 4] {
        dense_to(&mut r, tip).await;
        let (next, _) = judged(&r).await;
        let (age, tenure) = (age_at_next(&r, &tx).await, next - GATE);
        assert_eq!(
            age, tenure,
            "FIXTURE: offence at the activation block's slot on a dense chain"
        );
        assert!(age < 60, "FIXTURE: age {age} inside the period");
        let probe = four_paths(&mut r, &tx, 0).await;
        assert_accepted_everywhere(
            &format!(
                "offence at slot {GATE}, judged at height {next}: age {age} == tenure {tenure}"
            ),
            &probe,
        );
    }
}
