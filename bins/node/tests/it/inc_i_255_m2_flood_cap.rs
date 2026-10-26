//! INC-I-255 M2 — the per-block slash cap bounds evidence verification cost:
//! one more than the cap is refused on every block path, exactly the cap is
//! accepted, and the builder never selects more than the cap.
//!
// OUTPUT CONTRACT: fn Node::validate_block_for_apply(&self, &Block, u64, ValidationMode) -> Result<(), ValidationError>
//   O1 return (InvalidSlash message) per mode
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return (InvalidSlash message) per mode; O2 chain tip; O3 producer-set slash records
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O1 SlashProducer transactions the builder selected
// PATHS: block validation x {Full, Light, Replay} | apply x {Full, Light, Replay} | builder selection
// MATRIX
//   validate/apply O1,O2 x cap+1 distinct accused -> InvalidSlash, one message, tip kept (inc_i_255_m2_cap_*_every_path, probe)
//   validate O1 x cap distinct accused x 3 modes -> Ok; apply Full O1,O2,O3 -> Ok, tip moved, all recorded (inc_i_255_m2_cap_exactly_cap_accepted)
//   builder O1 x mempool holding cap+2 valid slashes -> 1..=cap selected, and the built block validates (inc_i_255_m2_cap_builder_never_exceeds_cap)
// INPUT PARTITIONS:
//   slash count per block: cap | cap + 1 | cap + 2 resident
//   accused: distinct registered Active producers (no duplicate)

use doli_core::validation::ValidationMode;

use super::inc_i_255_m1_common::*;
use super::inc_i_255_m2_flood_common::*;

// REQ-SLASH-004 — Decision: a failure means a block with more slashes than the cap passes block validation, so evidence verification cost per block is unbounded (consensus DoS).
#[tokio::test]
async fn inc_i_255_m2_probe_cap() {
    let mut f = flock(SLASH_CAP + 1).await;
    let over = f.slashes(0..SLASH_CAP + 1);
    let (block, _) = full_block_with(&mut f.node, &over).await;
    let r = f
        .node
        .validate_block_for_apply(&block, 1, ValidationMode::Full)
        .await;
    assert!(
        block_slash_msg(&r).is_some(),
        "a block with {} slashes (cap {SLASH_CAP}) must be refused with InvalidSlash; got {r:?}",
        SLASH_CAP + 1
    );
}

// REQ-SLASH-004 — Decision: a failure names the block path (validation or apply, which mode) that still accepts an over-cap block or refuses it with another error.
#[tokio::test]
async fn inc_i_255_m2_cap_over_cap_refused_on_every_path() {
    let mut f = flock(SLASH_CAP + 1).await;
    let over = f.slashes(0..SLASH_CAP + 1);
    let (block, _) = full_block_with(&mut f.node, &over).await;
    refused_everywhere(&mut f.node, &block, "cap + 1").await;
    for i in 0..SLASH_CAP + 1 {
        assert!(
            !slash_recorded(&f.node, &f.pk(i)).await,
            "a refused block must record no slash (accused #{i})"
        );
    }
}

// REQ-SLASH-004 — Decision: a failure means the cap over-rejects (off by one) and a legitimate block with exactly the cap forks its producer off.
#[tokio::test]
async fn inc_i_255_m2_cap_exactly_cap_accepted() {
    let mut f = flock(SLASH_CAP).await;
    let at_cap = f.slashes(0..SLASH_CAP);
    let (block, _) = full_block_with(&mut f.node, &at_cap).await;
    for mode in MODES {
        let r = f.node.validate_block_for_apply(&block, 1, mode).await;
        assert!(
            r.is_ok(),
            "{mode:?}: a block with exactly the cap must validate; got {r:?}"
        );
    }
    let r = f
        .node
        .apply_block(block.clone(), ValidationMode::Full)
        .await;
    assert!(
        r.is_ok(),
        "Full apply must accept exactly the cap; got {:?}",
        r.as_ref().err().map(|e| format!("{e:#}"))
    );
    assert_eq!(
        tip(&f.node).await,
        (1, block.hash()),
        "tip must be the cap block"
    );
    for i in 0..SLASH_CAP {
        assert!(
            slash_recorded(&f.node, &f.pk(i)).await,
            "slash #{i} of the cap block must be recorded"
        );
    }
}

// REQ-SLASH-004 — Decision: a failure means the builder packs more slashes than the cap and every node rejects the block it just produced (self-fork), or packs none.
#[tokio::test]
async fn inc_i_255_m2_cap_builder_never_exceeds_cap() {
    let resident = SLASH_CAP + 2;
    let mut f = flock(resident).await;
    for tx in f.slashes(0..resident) {
        let r = admit(&f.node, &tx).await;
        assert!(
            r.is_ok(),
            "PRECONDITION: a valid slash is admitted; got {r:?}"
        );
    }
    assert_eq!(
        resident_slash_total(&f.node).await,
        resident,
        "PRECONDITION: every slash is resident before the builder runs"
    );
    let (block, selected) = full_block_with(&mut f.node, &[]).await;
    let n = slash_count(&block, &selected);
    assert!(
        (1..=SLASH_CAP).contains(&n),
        "the builder selected {n} slashes from {resident} resident; must be 1..={SLASH_CAP}"
    );
    let r = f
        .node
        .validate_block_for_apply(&block, 1, ValidationMode::Full)
        .await;
    assert!(
        r.is_ok(),
        "the builder's own block must validate; got {r:?}"
    );
}
