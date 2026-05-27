//! INC-I-090 D1: classify_and_dispatch must emit RecoveryClassifyCall on every
//! recovery iteration, including when RecoveryAction::None is returned.
//!
//! These tests verify the fix to block_lifecycle.rs:626-630 where ctx_for_emit
//! was set to None when action==RecoveryAction::None, causing EMIT-007 in
//! periodic.rs to silently skip emission. This made signal_d (recovery_attempts>20)
//! in classifier rule (h) ChainBreakLoop unreachable.
//!
//! OUTPUT CONTRACT: fn classify_and_dispatch(&mut self, shallow_rollback_count: u32)
//!                  -> (RecoveryAction, Option<RecoveryContext>)
//!   O1: RecoveryAction -- the action returned by the classifier
//!   O2: Option<RecoveryContext> -- ctx_for_emit, used by periodic.rs EMIT-007
//! PATHS:
//!   P1: action=None (healthy chain, no evidence) -> O2 MUST be Some
//!   P2: action=non-None (gap triggers action)    -> O2 MUST be Some
//! INPUT PARTITIONS:
//!   IP1: gap=0, no evidence (classifier returns None) -- exercises the bug path
//!   IP2: gap=100, no fork evidence (classifier returns HeaderFirstSync or SnapSync)
//! MATRIX: 1 output (O2) x 2 partitions = 2 cells
//!   IP1: O2=Some(ctx) (FAILS pre-fix: was None; PASSES post-fix)
//!   IP2: O2=Some(ctx) (already works -- regression guard)
//! .test_verified

use libp2p::PeerId;

use crypto::Hash;

use crate::sync::manager::recovery::RecoveryAction;
use crate::sync::manager::{SyncConfig, SyncManager, SyncPipelineData, SyncState};

/// INC-I-090 D1 (FAIL->PASS): classify_and_dispatch returns Some(ctx) even
/// when the classifier returns RecoveryAction::None.
///
/// Before fix: ctx_for_emit is None when action==None (block_lifecycle.rs:626-630).
/// After fix: ctx_for_emit is always Some, allowing EMIT-007 to fire on every
/// recovery iteration so signal_d (recovery_attempts>20) can trigger rule (h).
#[test]
fn test_inc_i090_d1_classify_and_dispatch_emits_ctx_on_action_none() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    manager.local_height = 1000;
    manager.local_slot = 1000;
    manager.network.network_tip_height = 1000;

    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 1000, Hash::ZERO, 1000);
    manager.add_peer(peer2, 1000, Hash::ZERO, 1000);
    manager.state = SyncState::Idle;
    manager.pipeline_data = SyncPipelineData::None;

    let (action, ctx_for_emit) = manager.classify_and_dispatch(0);

    assert_eq!(
        action,
        RecoveryAction::None,
        "Precondition: healthy chain with no evidence must return RecoveryAction::None"
    );

    assert!(
        ctx_for_emit.is_some(),
        "INC-I-090 D1: classify_and_dispatch must return Some(ctx) even when \
         action==RecoveryAction::None so EMIT-007 fires and the classifier \
         can count all recovery iterations for signal_d (rule h ChainBreakLoop)"
    );

    let ctx = ctx_for_emit.unwrap();
    assert_eq!(ctx.local_height, 1000);
    assert_eq!(ctx.network_tip_height, 1000);
    assert_eq!(ctx.peer_count, 2);
    assert!(!ctx.in_grace_period);
}

/// INC-I-090 D1 regression guard: classify_and_dispatch still returns
/// Some(ctx) when action is non-None (existing behavior preserved).
#[test]
fn test_inc_i090_d1_classify_and_dispatch_still_emits_ctx_on_action_some() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    manager.local_height = 1000;
    manager.local_slot = 1000;
    manager.network.network_tip_height = 1100;

    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 1100, Hash::ZERO, 1100);
    manager.add_peer(peer2, 1100, Hash::ZERO, 1100);
    manager.state = SyncState::Idle;
    manager.pipeline_data = SyncPipelineData::None;

    let (action, ctx_for_emit) = manager.classify_and_dispatch(0);

    assert_ne!(
        action,
        RecoveryAction::None,
        "Precondition: 100-block gap should trigger a non-None action"
    );

    assert!(
        ctx_for_emit.is_some(),
        "Regression: classify_and_dispatch must still return Some(ctx) for non-None actions"
    );
}
