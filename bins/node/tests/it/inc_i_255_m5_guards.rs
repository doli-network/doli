//! INC-I-255 M5 guards (AUDIT-P1-517) — behaviour the decoy fix must keep:
//! a valid third header still yields a slash, re-delivery never adds one,
//! and below the gate the detector only logs.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(&mut self, Block, PeerId) -> Result<()>
//   (reaches EquivocationDetector -> handle_equivocation -> submit_equivocation_slash)
//   O1 mempool SlashProducer entries naming the accused (ids, total)
//   O2 the detector's "EQUIVOCATION DETECTED" WARN (tracing event)
//   O3 return: ignored (intake verdicts unchanged by M5)
// OUTPUT CONTRACT: fn doli_core::validation::validate_transaction(&Transaction, &ValidationContext)
//   O1 return for the admitted (D, A) slash
// PATHS: P-ON devnet default slash gate | P-OFF child, slash gate env-raised to 1e9
// MATRIX
//   O1 x P-ON x valid D -> A -> B -> [canonical(D,A)], validate_transaction Ok (inc_i_255_m5_valid_decoy_*)
//   O1 x P-ON x A x12 -> [] ; then B -> [canonical(A,B)] ; then (B, A) x12 -> same, total 1 (inc_i_255_m5_redelivery_*)
//   O1,O2 x P-OFF x A -> B -> WARN logged, [] (inc_i_255_m5_below_gate_*); parent control at P-ON -> one slash
// INPUT PARTITIONS:
//   third header: valid sibling D | none
//   delivery: once | 12 re-deliveries before and after admission
//   gate: devnet default | env-raised far above the tip

use std::sync::{Arc, Mutex};

use doli_core::transaction::Transaction;
use doli_core::validation::{validate_transaction, ValidationContext};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

use super::inc_i_236_m4_common::header_block;
use super::inc_i_255_m1_common::*;
use super::inc_i_255_m2_flood_common::resident_slash_total;
use super::inc_i_255_m5_common::*;

const MOD: &str = "inc_i_255_m5_guards::";
const BELOW: u64 = 1_000_000_000;
const REDELIVERIES: usize = 12;

async fn resident_tx(s: &Scene, id: &crypto::Hash) -> Transaction {
    let pool = s.w.node.mempool.read().await;
    let found = pool
        .iter()
        .find(|(h, _)| *h == id)
        .map(|(_, e)| e.tx.clone());
    found.expect("FIXTURE: the admitted slash is resident")
}

async fn chain_ctx(s: &Scene) -> ValidationContext {
    let node = &s.w.node;
    let net = node.config.network.params();
    let height = tip(node).await.0 + 1;
    let ts = node.params.slot_to_timestamp(SLOT);
    ValidationContext::new(node.params.clone(), node.config.network, ts, height)
        .with_inc_i_236_block_signature_activation_height(
            net.inc_i_236_block_signature_activation_height,
        )
        .with_inc_i_255_slashing_activation_height(net.effective_slashing_activation_height())
}

// REQ-SLASH-005 — Decision: a failure means the decoy fix also drops a VALID third header, so a real equivocation seen as (D, A) is no longer slashed (AUDIT-P1-517).
#[tokio::test]
async fn inc_i_255_m5_valid_decoy_slashes_first_pair() {
    let mut s = scene().await;
    let genesis = tip(&s.w.node).await.1;
    let (pk, kp) = (s.pk(), s.w.accused.clone());
    let d = header_block(&mut s.w.node, 1, SLOT, genesis, &pk, Some(&kp), 2).await;
    assert!(
        d.hash() != s.a.hash() && d.hash() != s.b.hash(),
        "FIXTURE: D is a third header"
    );
    let (a, b) = (s.a.clone(), s.b.clone());
    for blk in [&d, &a, &b] {
        s.feed(blk).await;
    }
    let want = s.id_of(&d, &a);
    assert_eq!(
        s.slashes().await,
        vec![want],
        "valid D then A then B: one slash from (D, A)"
    );
    let tx = resident_tx(&s, &want).await;
    let ctx = chain_ctx(&s).await;
    let r = validate_transaction(&tx, &ctx);
    assert!(
        r.is_ok(),
        "the admitted (D, A) slash must pass consensus validation; got {r:?}"
    );
}

// REQ-SLASH-004 — Decision: a failure means re-delivering a stored header pairs with itself or a re-delivered sibling adds a second slash (AUDIT-P1-517).
#[tokio::test]
async fn inc_i_255_m5_redelivery_never_adds_a_slash() {
    let mut s = scene().await;
    let (a, b) = (s.a.clone(), s.b.clone());
    for _ in 0..REDELIVERIES {
        s.feed(&a).await;
    }
    assert!(
        s.slashes().await.is_empty(),
        "A re-delivered {REDELIVERIES}x alone must never pair"
    );
    s.feed(&b).await;
    assert_eq!(
        s.slashes().await,
        vec![s.ab_id()],
        "B after A: one canonical slash"
    );
    for _ in 0..REDELIVERIES {
        s.feed(&b).await;
        s.feed(&a).await;
    }
    assert_eq!(
        s.slashes().await,
        vec![s.ab_id()],
        "re-delivery after admission adds nothing"
    );
    assert_eq!(
        resident_slash_total(&s.w.node).await,
        1,
        "one slash tx in the mempool"
    );
}

#[derive(Clone, Default)]
struct Warns(Arc<Mutex<Vec<String>>>);

struct Msg(String);

impl tracing::field::Visit for Msg {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for Warns {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if *event.metadata().level() > tracing::Level::WARN {
            return;
        }
        let mut msg = Msg(String::new());
        event.record(&mut msg);
        if msg.0.contains("EQUIVOCATION") {
            self.0.lock().unwrap().push(msg.0);
        }
    }
}

async fn below_gate_body() {
    let warns = Warns::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(warns.clone()));
    let mut s = scene().await;
    let (a, b) = (s.a.clone(), s.b.clone());
    s.feed(&a).await;
    s.feed(&b).await;
    let logged = warns.0.lock().unwrap().clone();
    assert!(
        !logged.is_empty(),
        "below the gate the detector must still log EQUIVOCATION"
    );
    assert!(
        s.slashes().await.is_empty(),
        "below the gate no slash tx may exist"
    );
}

// REQ-SLASH-004 — Decision: a failure means the decoy fix changed the pre-activation detector (no WARN) or created a slash below the gate (AUDIT-P1-517).
#[tokio::test]
async fn inc_i_255_m5_below_gate_warns_without_tx() {
    if in_child() {
        below_gate_body().await;
        return;
    }
    run_in_child(
        &format!("{MOD}inc_i_255_m5_below_gate_warns_without_tx"),
        BELOW,
    );
    let mut s = scene().await;
    let (a, b) = (s.a.clone(), s.b.clone());
    s.feed(&a).await;
    s.feed(&b).await;
    assert_eq!(
        s.slashes().await,
        vec![s.ab_id()],
        "NON-VACUITY: at the default gate the same pair is slashed"
    );
}
