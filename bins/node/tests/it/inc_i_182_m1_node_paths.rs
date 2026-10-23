//! INC-I-182 M1 path drivers: mempool admission, block builder selection and
//! Light-mode block apply (the rig has three scheduled producers, so Full
//! would refuse the node key on most slots) for one producer tx at a height.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
// covers: producer_auth pool assembly tx_processing validation_checks block

#![allow(dead_code)]

use doli_core::transaction::TxType;
use doli_core::validation::ValidationMode;
use tracing_subscriber::layer::SubscriberExt;

use super::inc_i_182_m1_common::{Logs, Verdict};
use super::inc_i_182_m1_node_fx::{producer_tx, px, Form};
use super::inc_i_255_m1_common::{admit, full_block_with};
use super::inc_i_255_m2_common::Keys;
use super::inc_i_255_m4_common::{admit_at, err_text, resident};
use super::inc_i_255_m6_common::block_with;

/// Admission with the node's own tip `at`.
pub async fn mempool_path(kind: TxType, form: Form, at: u64) -> Verdict {
    let mut p = px(&Keys::new()).await;
    p.r.advance_to(at).await;
    let tx = producer_tx(&p, kind, form, &p.genesis());
    let r = admit(&p.r.node, &tx).await;
    Verdict {
        accepted: r.is_ok(),
        text: format!("mempool at tip {at}: {r:?}"),
    }
}

/// Admitted judged at height 0, then the builder runs for height `at`.
pub async fn builder_path(kind: TxType, form: Form, at: u64) -> Verdict {
    let logs = Logs::default();
    let _g = tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let mut p = px(&Keys::new()).await;
    p.r.advance_to(at - 1).await;
    let tx = producer_tx(&p, kind, form, &p.genesis());
    let admission = admit_at(&p.r.node, &tx, 0).await;
    assert!(
        resident(&p.r.node, &tx).await,
        "PRECONDITION: {kind:?} {form:?} resident after admission judged at 0; got {admission:?}"
    );
    let selected = full_block_with(&mut p.r.node, &[]).await.1;
    let accepted = selected.contains(&tx.hash());
    Verdict {
        accepted,
        text: format!(
            "builder at {at}: selected {accepted}; log {:?}",
            logs.mentioning(&tx.hash().to_string())
        ),
    }
}

/// A builder block for height `at` carrying the tx, applied in Light mode.
pub async fn apply_path(kind: TxType, form: Form, at: u64) -> Verdict {
    let mut p = px(&Keys::new()).await;
    p.r.advance_to(at - 1).await;
    let tx = producer_tx(&p, kind, form, &p.genesis());
    let block = block_with(&mut p.r.node, std::slice::from_ref(&tx)).await;
    let r = p.r.node.apply_block(block, ValidationMode::Light).await;
    Verdict {
        accepted: r.is_ok(),
        text: format!("apply at {at}: {}", err_text(&r)),
    }
}

pub async fn verdicts(
    kind: TxType,
    form: Form,
    at: u64,
    builder: bool,
) -> Vec<(&'static str, Verdict)> {
    let mut v = vec![("mempool", mempool_path(kind, form, at).await)];
    if builder {
        v.push(("builder", builder_path(kind, form, at).await));
    }
    v.push(("apply", apply_path(kind, form, at).await));
    v
}
