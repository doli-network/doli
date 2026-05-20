//! Per-pattern sub-parsers for log-replay line classification.
//!
//! Each function matches a single log pattern (e.g. `[BLOCK] Applied`,
//! `[ROLLBACK] Initiating:`) and returns a `DiagnosticEvent` or `None`.

use crate::diagnostic_ledger::types::{DiagnosticEvent, EventKind, EventPayload};

// ---------------------------------------------------------------------------
// Shared helpers (used by all sub-parsers)
// ---------------------------------------------------------------------------

/// Helper to extract a named `key=value` token from a message substring.
fn extract_kv<'a>(msg: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("{}=", key);
    let start = msg.find(&needle)? + needle.len();
    let rest = &msg[start..];
    // Value ends at next space, pipe, comma, or end of string.
    let end = rest.find([' ', '|', ',']).unwrap_or(rest.len());
    let val = &rest[..end];
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

/// Helper: parse a u64 from a key=value pair.
fn kv_u64(msg: &str, key: &str) -> Option<u64> {
    extract_kv(msg, key)?.parse().ok()
}

/// Helper: parse a u32 from a key=value pair.
fn kv_u32(msg: &str, key: &str) -> Option<u32> {
    extract_kv(msg, key)?.parse().ok()
}

/// Build a `DiagnosticEvent` skeleton with common defaults for replay.
fn replay_event(
    kind: EventKind,
    timestamp_ms: u64,
    height: Option<u64>,
    payload: EventPayload,
) -> DiagnosticEvent {
    DiagnosticEvent {
        event_id: ulid::Ulid::new().to_string(),
        kind,
        timestamp_ms,
        height,
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload,
    }
}

// ---------------------------------------------------------------------------
// Sub-parsers (12 patterns)
// ---------------------------------------------------------------------------

/// `[BLOCK] Applied h=N hash=H producer=P slot=S txs=T epoch=E`
pub(super) fn parse_block_applied(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
    if !msg.contains("[BLOCK] Applied") {
        return None;
    }
    let h = kv_u64(msg, "h")?;
    let hash = extract_kv(msg, "hash")?.to_string();
    let producer = extract_kv(msg, "producer")?.to_string();
    let slot = kv_u32(msg, "slot")?;
    let txs = kv_u32(msg, "txs").unwrap_or(0);

    Some(replay_event(
        EventKind::BlockApplied,
        ts,
        Some(h),
        EventPayload::BlockApplied {
            slot,
            block_hash: hash,
            producer_pubkey: producer,
            from_peer_id: None,
            received_at_ms: None,
            applied_at_ms: ts,
            validation_duration_ms: 0,
            mode: "Full".to_string(),
            tx_count: txs,
        },
    ))
}

/// `Block rejected:` or `BlockRejected:` -- extract the rejection reason.
pub(super) fn parse_block_rejected(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
    // Case-insensitive search for the two known variants.
    let lower = msg.to_lowercase();
    let idx = lower
        .find("block rejected:")
        .or_else(|| lower.find("blockrejected:"))?;
    let reason_start = msg[idx..].find(':')? + idx + 1;
    let reason = msg[reason_start..].trim().to_string();

    Some(replay_event(
        EventKind::BlockRejected,
        ts,
        None,
        EventPayload::BlockRejected {
            slot: 0,
            block_hash: "(unknown)".to_string(),
            producer_pubkey: "(unknown)".to_string(),
            from_peer_id: None,
            rejection_reason: reason,
            mode: "Full".to_string(),
        },
    ))
}

/// `[BLOCK] REJECT slot=S h=H producer=P error=REASON`
///
/// Empirically required: real testnet logs (n10.log line 3053925) use this
/// format for block rejections — not `Block rejected:`. Discovered during
/// INC-I-081 fixture creation (M4). The `error=` value after `[ECON_*]` tag
/// contains the reason including "EpochReward" when relevant.
pub(super) fn parse_block_reject_structured(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
    if !msg.contains("[BLOCK] REJECT") {
        return None;
    }
    let h = kv_u64(msg, "h");
    let slot = kv_u32(msg, "slot").unwrap_or(0);
    let producer = extract_kv(msg, "producer")
        .unwrap_or("(unknown)")
        .to_string();

    // The reason starts after "error=" and runs to end of line.
    let reason = msg
        .find("error=")
        .map(|idx| msg[idx + "error=".len()..].trim().to_string())
        .unwrap_or_else(|| "(unknown)".to_string());

    Some(replay_event(
        EventKind::BlockRejected,
        ts,
        h,
        EventPayload::BlockRejected {
            slot,
            block_hash: "(unknown)".to_string(),
            producer_pubkey: producer,
            from_peer_id: None,
            rejection_reason: reason,
            mode: "Full".to_string(),
        },
    ))
}

/// `[ROLLBACK] Initiating: depth=D local_h=L target_h=T gap=G
///  empty_headers=E shallow_count=S`
pub(super) fn parse_rollback_initiating(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
    if !msg.contains("[ROLLBACK] Initiating:") {
        return None;
    }
    let from_h = kv_u64(msg, "local_h")?;
    let to_h = kv_u64(msg, "target_h")?;
    let depth = kv_u32(msg, "depth").unwrap_or(1);

    Some(replay_event(
        EventKind::RollbackStarted,
        ts,
        Some(from_h),
        EventPayload::RollbackStarted {
            from_height: from_h,
            to_height: to_h,
            trigger: "shallow_recovery".to_string(),
            cumulative_depth: depth,
        },
    ))
}

/// `Rolling back from height L to T for fork recovery`
pub(super) fn parse_rolling_back_from(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
    if !msg.contains("Rolling back from height") {
        return None;
    }
    // "Rolling back from height 110596 to 110595 for fork recovery"
    let after = msg.find("from height")? + "from height".len();
    let rest = &msg[after..];
    let mut parts = rest.split_whitespace();
    let from_h: u64 = parts.next()?.parse().ok()?;
    // skip "to"
    let to_keyword = parts.next()?;
    if to_keyword != "to" {
        return None;
    }
    let to_h: u64 = parts.next()?.parse().ok()?;

    Some(replay_event(
        EventKind::RollbackStarted,
        ts,
        Some(from_h),
        EventPayload::RollbackStarted {
            from_height: from_h,
            to_height: to_h,
            trigger: "fork_recovery".to_string(),
            cumulative_depth: 0,
        },
    ))
}

/// `Reorg complete: now at height H`
pub(super) fn parse_reorg_complete(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
    if !msg.contains("Reorg complete:") && !msg.contains("[REORG] Completed") {
        return None;
    }
    let height = kv_u64(msg, "height").or_else(|| {
        // "Reorg complete: now at height 12345"
        let after = msg.find("height")? + "height".len();
        msg[after..].split_whitespace().next()?.parse().ok()
    });

    Some(replay_event(
        EventKind::ReorgExecuted,
        ts,
        height,
        EventPayload::ReorgExecuted {
            old_tip_hash: "(unknown)".to_string(),
            new_tip_hash: "(unknown)".to_string(),
            rollback_depth: 0,
            applied_count: 0,
            weight_delta: 0,
            trigger_block_hash: "(unknown)".to_string(),
            trigger_from_peer_id: None,
        },
    ))
}

/// `[HEALTH] h=N s=S hash=H | peers=P best_peer_h=BPH ... net_tip_h=NTH
///  ... | sync_fails=SF state="STATE" | snap_epoch=SE ...`
pub(super) fn parse_health(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
    if !msg.contains("[HEALTH]") {
        return None;
    }
    let h = kv_u64(msg, "h")?;
    let net_tip_h = kv_u64(msg, "net_tip_h").unwrap_or(0);
    let peers = kv_u32(msg, "peers").unwrap_or(0);

    // Extract state from state="..." or state=Idle etc.
    let state = extract_kv(msg, "state").map(|s| {
        // The value may be quoted: "Syncing:Headers" -- strip quotes.
        s.trim_matches('"').to_string()
    });

    Some(replay_event(
        EventKind::RecoveryClassifyCall,
        ts,
        Some(h),
        EventPayload::RecoveryClassifyCall {
            local_height: h,
            network_tip_height: net_tip_h,
            peer_count: peers,
            last_applied_secs: 0,
            shallow_rollback_count: 0,
            snap_attempts: 0,
            last_rollback_local_height: None,
            in_grace_period: false,
            last_finality_height: None,
            action_returned: state,
            rule_matched: Some("log-replay-incomplete".to_string()),
        },
    ))
}

/// `Stuck-sync detected: no block applied for Ns, behind by B blocks
///  (local_h=L, network_tip=T). Gap too large ... requesting snap sync.`
pub(super) fn parse_stuck_sync(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
    if !msg.contains("Stuck-sync detected:") {
        return None;
    }
    // Extract local_h and network_tip from the parenthetical.
    let local_h = kv_u64(msg, "local_h").unwrap_or(0);
    // network_tip is formatted as "network_tip=41100)" -- parse digits only.
    let net_tip = {
        let needle = "network_tip=";
        msg.find(needle).and_then(|i| {
            let start = i + needle.len();
            let rest = &msg[start..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            rest[..end].parse::<u64>().ok()
        })
    }
    .unwrap_or(0);

    Some(replay_event(
        EventKind::SnapSyncAttempted,
        ts,
        Some(local_h),
        EventPayload::SnapSyncAttempted {
            local_height: local_h,
            target_height: net_tip,
            source_peer_id: "(stuck-sync)".to_string(),
        },
    ))
}

/// `Snap sync attempted` or `[SNAP] Attempted`
pub(super) fn parse_snap_attempted(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
    let lower = msg.to_lowercase();
    if !lower.contains("snap sync attempted") && !lower.contains("[snap] attempted") {
        return None;
    }
    let local_h = kv_u64(msg, "local_h")
        .or_else(|| kv_u64(msg, "local_height"))
        .unwrap_or(0);
    let target_h = kv_u64(msg, "target_h")
        .or_else(|| kv_u64(msg, "target_height"))
        .unwrap_or(0);

    Some(replay_event(
        EventKind::SnapSyncAttempted,
        ts,
        Some(local_h),
        EventPayload::SnapSyncAttempted {
            local_height: local_h,
            target_height: target_h,
            source_peer_id: "(unknown)".to_string(),
        },
    ))
}

/// `Snap sync completed` or `[SNAP] Completed`
pub(super) fn parse_snap_completed(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
    let lower = msg.to_lowercase();
    if !lower.contains("snap sync completed") && !lower.contains("[snap] completed") {
        return None;
    }
    Some(replay_event(
        EventKind::SnapSyncCompleted,
        ts,
        None,
        EventPayload::SnapSyncCompleted {
            result: "success".to_string(),
            duration_ms: 0,
        },
    ))
}

/// `Snap sync failed` or `[SNAP] Failed`
pub(super) fn parse_snap_failed(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
    let lower = msg.to_lowercase();
    if !lower.contains("snap sync failed") && !lower.contains("[snap] failed") {
        return None;
    }
    let error = msg
        .find(':')
        .map(|i| msg[i + 1..].trim().to_string())
        .unwrap_or_else(|| "(unknown)".to_string());

    Some(replay_event(
        EventKind::SnapSyncFailed,
        ts,
        None,
        EventPayload::SnapSyncFailed {
            error,
            duration_ms: 0,
        },
    ))
}

/// `[HEADER_DEBUG] Chain break: ... valid_so_far=N`
pub(super) fn parse_chain_break(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
    if !msg.contains("Chain break") && !msg.contains("chain break") {
        return None;
    }
    let valid_so_far = kv_u32(msg, "valid_so_far").unwrap_or(0);

    Some(replay_event(
        EventKind::ChainBreakDetected,
        ts,
        None,
        EventPayload::ChainBreakDetected {
            expected_prev_hash: extract_kv(msg, "expected")
                .unwrap_or("(unknown)")
                .to_string(),
            actual_prev_hash: extract_kv(msg, "header.prev_hash")
                .unwrap_or("(unknown)")
                .to_string(),
            header_slot: kv_u32(msg, "header_slot").unwrap_or(0),
            valid_so_far_count: valid_so_far,
            from_peer_id: "(unknown)".to_string(),
        },
    ))
}

/// `[FORK_GUARD] Better block dropped ...` or `[FORK_GUARD] Dropping fork
///  block ...`
pub(super) fn parse_fork_guard(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
    if !msg.contains("[FORK_GUARD]") {
        return None;
    }
    let block_hash = extract_kv(msg, "hash")
        .or_else(|| {
            // "Dropping fork block ABCD1234 at h=..."
            let marker = "fork block ";
            let start = msg.find(marker)? + marker.len();
            let rest = &msg[start..];
            let end = rest.find(' ').unwrap_or(rest.len());
            Some(&rest[..end])
        })
        .unwrap_or("(unknown)")
        .to_string();

    let height = kv_u64(msg, "h");

    let fork_kind = if msg.contains("Better block dropped") {
        Some("BetterBlockDropped".to_string())
    } else {
        Some("HeightOccupied".to_string())
    };

    Some(replay_event(
        EventKind::ForkBlockReceived,
        ts,
        height,
        EventPayload::ForkBlockReceived {
            block_hash,
            block_slot: kv_u32(msg, "slot").unwrap_or(0),
            block_height_estimate: height,
            producer_pubkey: "(unknown)".to_string(),
            from_peer_id: "(unknown)".to_string(),
            classification: "ForkBlock".to_string(),
            fork_kind,
            local_tip_hash: "(unknown)".to_string(),
            local_tip_height: 0,
        },
    ))
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::super::parse_line;
    use crate::diagnostic_ledger::types::{EventKind, EventPayload};

    // Helper: wrap a message in the ANSI-decorated tracing format.
    fn ansi_line(ts: &str, level: &str, target: &str, msg: &str) -> String {
        format!(
            "\x1b[2m{}\x1b[0m \x1b[33m{}\x1b[0m \x1b[2m{}\x1b[0m\x1b[2m:\x1b[0m {}",
            ts, level, target, msg
        )
    }

    // -------------------------------------------------------------------
    // 3. test_parse_block_applied_line
    // -------------------------------------------------------------------
    #[test]
    fn test_parse_block_applied_line() {
        let line = ansi_line(
            "2026-05-07T11:13:52.184040Z",
            " INFO",
            "doli_node::node::apply_block",
            "[BLOCK] Applied h=43344 hash=b381563e59c1ecf8 producer=2d27fdcc slot=111482 txs=2 epoch=1204",
        );
        let ev = parse_line(&line).expect("should parse");
        assert_eq!(ev.kind, EventKind::BlockApplied);
        assert_eq!(ev.height, Some(43344));
        assert!(ev.correlation_key.is_none());
        assert!(!ev.is_cascade_origin);
        match &ev.payload {
            EventPayload::BlockApplied {
                slot,
                block_hash,
                producer_pubkey,
                tx_count,
                mode,
                from_peer_id,
                ..
            } => {
                assert_eq!(*slot, 111482);
                assert_eq!(block_hash, "b381563e59c1ecf8");
                assert_eq!(producer_pubkey, "2d27fdcc");
                assert_eq!(*tx_count, 2);
                assert_eq!(mode, "Full");
                assert!(from_peer_id.is_none());
            }
            other => panic!("unexpected payload: {:?}", other),
        }
    }

    // -------------------------------------------------------------------
    // 4. test_parse_rollback_initiating
    // -------------------------------------------------------------------
    #[test]
    fn test_parse_rollback_initiating() {
        let line = ansi_line(
            "2026-05-19T21:16:02.260898Z",
            " INFO",
            "doli_node::node::rollback",
            "[ROLLBACK] Initiating: depth=2 local_h=100 target_h=99 gap=5 empty_headers=0 shallow_count=2",
        );
        let ev = parse_line(&line).expect("should parse");
        assert_eq!(ev.kind, EventKind::RollbackStarted);
        assert_eq!(ev.height, Some(100));
        match &ev.payload {
            EventPayload::RollbackStarted {
                from_height,
                to_height,
                trigger,
                cumulative_depth,
            } => {
                assert_eq!(*from_height, 100);
                assert_eq!(*to_height, 99);
                assert_eq!(trigger, "shallow_recovery");
                assert_eq!(*cumulative_depth, 2);
            }
            other => panic!("unexpected payload: {:?}", other),
        }
    }

    // -------------------------------------------------------------------
    // 5. test_parse_health_line
    // -------------------------------------------------------------------
    #[test]
    fn test_parse_health_line() {
        let line = ansi_line(
            "2026-04-29T21:14:00.988734Z",
            " WARN",
            "doli_node::node::periodic",
            "[HEALTH] h=0 s=0 hash=adef0972 | peers=2 best_peer_h=21943 best_peer_s=45963 net_tip_h=21943 net_tip_s=45963 | sync_fails=0 state=\"Syncing:Headers\" | snap_epoch=0 snap_bonds=5 snap_producers=5",
        );
        let ev = parse_line(&line).expect("should parse");
        assert_eq!(ev.kind, EventKind::RecoveryClassifyCall);
        assert_eq!(ev.height, Some(0));
        match &ev.payload {
            EventPayload::RecoveryClassifyCall {
                local_height,
                network_tip_height,
                peer_count,
                action_returned,
                rule_matched,
                ..
            } => {
                assert_eq!(*local_height, 0);
                assert_eq!(*network_tip_height, 21943);
                assert_eq!(*peer_count, 2);
                assert_eq!(action_returned.as_deref(), Some("Syncing:Headers"));
                assert_eq!(rule_matched.as_deref(), Some("log-replay-incomplete"));
            }
            other => panic!("unexpected payload: {:?}", other),
        }
    }

    // -------------------------------------------------------------------
    // 6. test_parse_stuck_sync_line
    // -------------------------------------------------------------------
    #[test]
    fn test_parse_stuck_sync_line() {
        let line = ansi_line(
            "2026-05-06T13:49:13.802318Z",
            " WARN",
            "network::sync::manager::cleanup",
            "Stuck-sync detected: no block applied for 301s, behind by 41099 blocks (local_h=1, network_tip=41100). Gap too large for rollback \u{2014} requesting snap sync.",
        );
        let ev = parse_line(&line).expect("should parse");
        assert_eq!(ev.kind, EventKind::SnapSyncAttempted);
        assert_eq!(ev.height, Some(1));
        match &ev.payload {
            EventPayload::SnapSyncAttempted {
                local_height,
                target_height,
                source_peer_id,
            } => {
                assert_eq!(*local_height, 1);
                assert_eq!(*target_height, 41100);
                assert_eq!(source_peer_id, "(stuck-sync)");
            }
            other => panic!("unexpected payload: {:?}", other),
        }
    }

    // -------------------------------------------------------------------
    // 7. test_parse_chain_break
    // -------------------------------------------------------------------
    #[test]
    fn test_parse_chain_break() {
        let line = ansi_line(
            "2026-05-06T13:44:13.824939Z",
            " WARN",
            "network::sync::headers",
            "[HEADER_DEBUG] Chain break: header.prev_hash=7dd7cfff expected=34d50c44 header_slot=6975 valid_so_far=0",
        );
        let ev = parse_line(&line).expect("should parse");
        assert_eq!(ev.kind, EventKind::ChainBreakDetected);
        match &ev.payload {
            EventPayload::ChainBreakDetected {
                valid_so_far_count,
                expected_prev_hash,
                actual_prev_hash,
                header_slot,
                ..
            } => {
                assert_eq!(*valid_so_far_count, 0);
                assert_eq!(expected_prev_hash, "34d50c44");
                assert_eq!(actual_prev_hash, "7dd7cfff");
                assert_eq!(*header_slot, 6975);
            }
            other => panic!("unexpected payload: {:?}", other),
        }
    }

    // -------------------------------------------------------------------
    // 12. test_parse_rolling_back_from
    // -------------------------------------------------------------------
    #[test]
    fn test_parse_rolling_back_from() {
        let line = ansi_line(
            "2026-05-19T21:16:02.260958Z",
            " INFO",
            "doli_node::node::rollback",
            "Rolling back from height 110362 to 110361 for fork recovery",
        );
        let ev = parse_line(&line).expect("should parse");
        assert_eq!(ev.kind, EventKind::RollbackStarted);
        match &ev.payload {
            EventPayload::RollbackStarted {
                from_height,
                to_height,
                trigger,
                cumulative_depth,
            } => {
                assert_eq!(*from_height, 110362);
                assert_eq!(*to_height, 110361);
                assert_eq!(trigger, "fork_recovery");
                assert_eq!(*cumulative_depth, 0);
            }
            other => panic!("unexpected payload: {:?}", other),
        }
    }

    // -------------------------------------------------------------------
    // 13. test_parse_reorg_complete
    // -------------------------------------------------------------------
    #[test]
    fn test_parse_reorg_complete() {
        let line = ansi_line(
            "2026-05-19T22:00:00.000000Z",
            " INFO",
            "doli_node::node::block_handling",
            "Reorg complete: now at height 110370",
        );
        let ev = parse_line(&line).expect("should parse");
        assert_eq!(ev.kind, EventKind::ReorgExecuted);
        assert_eq!(ev.height, Some(110370));
    }

    // -------------------------------------------------------------------
    // 14. test_parse_block_rejected
    // -------------------------------------------------------------------
    #[test]
    fn test_parse_block_rejected() {
        let line = ansi_line(
            "2026-05-20T00:00:00.000000Z",
            " WARN",
            "doli_node::node::apply_block",
            "Block rejected: invalid producer signature",
        );
        let ev = parse_line(&line).expect("should parse");
        assert_eq!(ev.kind, EventKind::BlockRejected);
        match &ev.payload {
            EventPayload::BlockRejected {
                rejection_reason, ..
            } => {
                assert_eq!(rejection_reason, "invalid producer signature");
            }
            other => panic!("unexpected payload: {:?}", other),
        }
    }

    // -------------------------------------------------------------------
    // 15. test_parse_fork_guard
    // -------------------------------------------------------------------
    #[test]
    fn test_parse_fork_guard() {
        let line = ansi_line(
            "2026-05-20T00:00:00.000000Z",
            " INFO",
            "doli_node::node::block_handling",
            "[FORK_GUARD] Dropping fork block abcd1234ef56 at h=50000 slot 100 \u{2014} keeping canonical slot 99",
        );
        let ev = parse_line(&line).expect("should parse");
        assert_eq!(ev.kind, EventKind::ForkBlockReceived);
        assert_eq!(ev.height, Some(50000));
        match &ev.payload {
            EventPayload::ForkBlockReceived {
                block_hash,
                fork_kind,
                ..
            } => {
                assert_eq!(block_hash, "abcd1234ef56");
                assert_eq!(fork_kind.as_deref(), Some("HeightOccupied"));
            }
            other => panic!("unexpected payload: {:?}", other),
        }
    }

    // -------------------------------------------------------------------
    // 17. test_parse_real_log_lines_from_n10
    // -------------------------------------------------------------------
    #[test]
    fn test_parse_real_log_lines_from_n10() {
        // These are exact copies of real lines from ~/testnet/logs/n10.log
        // (with ANSI codes as raw escapes).
        let block_line = "\x1b[2m2026-05-07T11:13:52.184040Z\x1b[0m \x1b[32m INFO\x1b[0m \x1b[2mdoli_node::node::apply_block\x1b[0m\x1b[2m:\x1b[0m [BLOCK] Applied h=43344 hash=b381563e59c1ecf819eedebda67ad87a0b919b47c3c7a204e61d291ce9acd05a producer=2d27fdcc slot=111482 txs=2 epoch=1204";
        let ev = parse_line(block_line).expect("real BLOCK line");
        assert_eq!(ev.kind, EventKind::BlockApplied);
        assert_eq!(ev.height, Some(43344));

        let health_line = "\x1b[2m2026-04-29T21:14:00.988734Z\x1b[0m \x1b[33m WARN\x1b[0m \x1b[2mdoli_node::node::periodic\x1b[0m\x1b[2m:\x1b[0m [HEALTH] h=0 s=0 hash=adef097295a73ec541f401b802e942c66c932291b6d36de1b22afae17522900a | peers=2 best_peer_h=21943 best_peer_s=45963 net_tip_h=21943 net_tip_s=45963 | sync_fails=0 state=\"Syncing:Headers\" | snap_epoch=0 snap_bonds=5 snap_producers=5";
        let ev = parse_line(health_line).expect("real HEALTH line");
        assert_eq!(ev.kind, EventKind::RecoveryClassifyCall);

        let stuck_line = "\x1b[2m2026-05-06T13:49:13.802318Z\x1b[0m \x1b[33m WARN\x1b[0m \x1b[2mnetwork::sync::manager::cleanup\x1b[0m\x1b[2m:\x1b[0m Stuck-sync detected: no block applied for 301s, behind by 41099 blocks (local_h=1, network_tip=41100). Gap too large for rollback \u{2014} requesting snap sync.";
        let ev = parse_line(stuck_line).expect("real stuck-sync line");
        assert_eq!(ev.kind, EventKind::SnapSyncAttempted);
    }
}
