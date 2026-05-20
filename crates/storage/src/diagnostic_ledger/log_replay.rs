//! Log-replay parser for offline retrospective fork analysis.
//!
//! Parses raw node log files (with embedded ANSI colour codes) into
//! `DiagnosticEvent` values that the classifier can consume.  Designed for
//! stream processing of multi-GiB log files — never loads the whole file.
//!
//! # Usage
//!
//! ```ignore
//! let events = replay_log_file(Path::new("n10.log"))?;
//! for re in &events {
//!     println!("{:?} replayed={}", re.event.kind, re.replayed_from_log);
//! }
//! ```

use std::io::{BufRead, BufReader};
use std::path::Path;

use super::types::{DiagnosticEvent, EventKind, EventPayload};

// ---------------------------------------------------------------------------
// ReplayedEvent — Option-A wrapper (keeps DiagnosticEvent on-disk format
// untouched)
// ---------------------------------------------------------------------------

/// A diagnostic event synthesised from a historical log line.
///
/// The `replayed_from_log` flag distinguishes replay-sourced events from
/// production-emitted ones without altering the persisted `DiagnosticEvent`
/// bincode format (Option A — see M3 brief).
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayedEvent {
    /// The underlying diagnostic event.
    pub event: DiagnosticEvent,
    /// Always `true` for events produced by this module.
    pub replayed_from_log: bool,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum log-file size this module will process (5 GiB).
const MAX_LOG_FILE_BYTES: u64 = 5 * 1024 * 1024 * 1024;

// ---------------------------------------------------------------------------
// ANSI stripping
// ---------------------------------------------------------------------------

/// Strip ANSI escape codes (CSI sequences) from a log line.
///
/// Removes byte sequences of the form `ESC [ ... <letter>` where the
/// parameter bytes are `0x20..=0x3F` and the final byte is `0x40..=0x7E`.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Expect '[' next
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                              // Consume parameter bytes until a letter terminates
                loop {
                    match chars.next() {
                        Some(t) if t.is_ascii_alphabetic() => break,
                        Some(_) => continue,
                        None => break,
                    }
                }
            }
            // else: bare ESC not followed by '[' — drop the ESC
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Timestamp parsing
// ---------------------------------------------------------------------------

/// Parse an RFC-3339 / ISO-8601 timestamp into milliseconds since UNIX epoch.
fn parse_timestamp_ms(ts: &str) -> Option<u64> {
    let dt = chrono::DateTime::parse_from_rfc3339(ts).ok()?;
    Some(dt.timestamp_millis() as u64)
}

// ---------------------------------------------------------------------------
// Line structure: TIMESTAMP   LEVEL   TARGET:   MESSAGE
// ---------------------------------------------------------------------------

/// Intermediate parse of a structured log line.
struct LogLine<'a> {
    timestamp_ms: u64,
    message: &'a str,
}

/// Parse the common tracing-subscriber prefix.
fn parse_log_prefix(clean: &str) -> Option<LogLine<'_>> {
    // Format: "TIMESTAMP  LEVEL  target:  message"
    // The timestamp ends at the first space.
    let ts_end = clean.find(' ')?;
    let ts_str = clean[..ts_end].trim();
    let timestamp_ms = parse_timestamp_ms(ts_str)?;

    // Find the target separator ": " after the module path. There may be
    // multiple ": " (e.g. "doli_node::node::rollback:  Rolling back ...").
    // Skip past LEVEL token first.
    let rest = clean[ts_end..].trim_start();
    // LEVEL is one of TRACE/DEBUG/INFO/WARN/ERROR
    let level_end = rest.find(' ')?;
    let after_level = rest[level_end..].trim_start();
    // Now find the first ": " which separates target from message.
    // The target itself may contain "::" (e.g. doli_node::node::rollback).
    // The separator is a bare ":" (not part of "::") followed by the message.
    let msg_start = find_target_end(after_level)?;
    let message = after_level[msg_start..].trim_start();

    Some(LogLine {
        timestamp_ms,
        message,
    })
}

/// Find the byte offset where the target path ends and the message begins.
///
/// The target is something like `doli_node::node::rollback:  `. We need the
/// first `:` that is NOT immediately preceded or followed by another `:`.
fn find_target_end(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b':' {
            // Check if this is part of "::" — skip those.
            let next_is_colon = i + 1 < len && bytes[i + 1] == b':';
            let prev_is_colon = i > 0 && bytes[i - 1] == b':';
            if !next_is_colon && !prev_is_colon {
                // This is the target-end colon. Message starts after it.
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// Per-pattern sub-parsers
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

/// `[BLOCK] Applied h=N hash=H producer=P slot=S txs=T epoch=E`
fn parse_block_applied(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
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

/// `Block rejected:` or `BlockRejected:` — extract the rejection reason.
fn parse_block_rejected(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
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

/// `[ROLLBACK] Initiating: depth=D local_h=L target_h=T gap=G
///  empty_headers=E shallow_count=S`
fn parse_rollback_initiating(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
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
fn parse_rolling_back_from(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
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
fn parse_reorg_complete(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
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
fn parse_health(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
    if !msg.contains("[HEALTH]") {
        return None;
    }
    let h = kv_u64(msg, "h")?;
    let net_tip_h = kv_u64(msg, "net_tip_h").unwrap_or(0);
    let peers = kv_u32(msg, "peers").unwrap_or(0);

    // Extract state from state="..." or state=Idle etc.
    let state = extract_kv(msg, "state").map(|s| {
        // The value may be quoted: "Syncing:Headers" — strip quotes.
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
fn parse_stuck_sync(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
    if !msg.contains("Stuck-sync detected:") {
        return None;
    }
    // Extract local_h and network_tip from the parenthetical.
    let local_h = kv_u64(msg, "local_h").unwrap_or(0);
    // network_tip is formatted as "network_tip=41100)" — parse digits only.
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
fn parse_snap_attempted(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
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
fn parse_snap_completed(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
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
fn parse_snap_failed(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
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
fn parse_chain_break(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
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
fn parse_fork_guard(ts: u64, msg: &str) -> Option<DiagnosticEvent> {
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

// ---------------------------------------------------------------------------
// Top-level parse_line
// ---------------------------------------------------------------------------

/// Parse a single raw log line (possibly with ANSI codes) into a
/// `DiagnosticEvent`.
///
/// Returns `None` for lines that don't match any known diagnostic pattern
/// (silent skip — this is expected for the vast majority of log lines).
pub fn parse_line(line: &str) -> Option<DiagnosticEvent> {
    let clean = strip_ansi(line);
    let log = parse_log_prefix(&clean)?;
    let ts = log.timestamp_ms;
    let msg = log.message;

    // Dispatch in rough frequency order (BLOCK and HEALTH are the most
    // common diagnostic-relevant lines).

    // Skip [APPLY_START] — paired event, no DiagnosticEvent of its own.
    if msg.contains("[APPLY_START]") {
        return None;
    }

    if msg.contains("[BLOCK] Applied") {
        return parse_block_applied(ts, msg);
    }
    if msg.contains("[HEALTH]") {
        return parse_health(ts, msg);
    }
    if msg.contains("[ROLLBACK] Initiating:") {
        return parse_rollback_initiating(ts, msg);
    }
    if msg.contains("Rolling back from height") {
        return parse_rolling_back_from(ts, msg);
    }
    if msg.contains("Stuck-sync detected:") {
        return parse_stuck_sync(ts, msg);
    }
    if msg.contains("Reorg complete:") || msg.contains("[REORG] Completed") {
        return parse_reorg_complete(ts, msg);
    }
    if msg.contains("Chain break") || msg.contains("chain break") {
        return parse_chain_break(ts, msg);
    }
    if msg.contains("[FORK_GUARD]") {
        return parse_fork_guard(ts, msg);
    }
    // Snap sync patterns (check "Stuck-sync" already handled above)
    {
        let lower = msg.to_lowercase();
        if lower.contains("snap sync attempted") || lower.contains("[snap] attempted") {
            return parse_snap_attempted(ts, msg);
        }
        if lower.contains("snap sync completed") || lower.contains("[snap] completed") {
            return parse_snap_completed(ts, msg);
        }
        if lower.contains("snap sync failed") || lower.contains("[snap] failed") {
            return parse_snap_failed(ts, msg);
        }
    }
    // Block rejected (least common, check last)
    {
        let lower = msg.to_lowercase();
        if lower.contains("block rejected:") || lower.contains("blockrejected:") {
            return parse_block_rejected(ts, msg);
        }
    }

    None
}

// ---------------------------------------------------------------------------
// replay_log_file
// ---------------------------------------------------------------------------

/// Validate that a file size is within the 5 GiB cap.
///
/// Extracted to a standalone function for unit-testability.
pub fn check_file_size(size_bytes: u64) -> Result<(), std::io::Error> {
    if size_bytes > MAX_LOG_FILE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "log file too large: {} bytes (cap {} bytes / 5 GiB)",
                size_bytes, MAX_LOG_FILE_BYTES
            ),
        ));
    }
    Ok(())
}

/// Stream-parse a log file into `ReplayedEvent` values.
///
/// Uses `BufReader::lines()` so the full file is never loaded into memory.
/// Rejects files larger than 5 GiB up-front.
pub fn replay_log_file(path: &Path) -> std::io::Result<Vec<ReplayedEvent>> {
    let metadata = std::fs::metadata(path)?;
    check_file_size(metadata.len())?;

    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();

    for line_result in reader.lines() {
        let line = line_result?;
        if let Some(ev) = parse_line(&line) {
            events.push(ReplayedEvent {
                event: ev,
                replayed_from_log: true,
            });
        }
    }

    Ok(events)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic_ledger::types::{EventKind, EventPayload};

    // Helper: wrap a message in the ANSI-decorated tracing format.
    fn ansi_line(ts: &str, level: &str, target: &str, msg: &str) -> String {
        format!(
            "\x1b[2m{}\x1b[0m \x1b[33m{}\x1b[0m \x1b[2m{}\x1b[0m\x1b[2m:\x1b[0m {}",
            ts, level, target, msg
        )
    }

    // -------------------------------------------------------------------
    // 1. test_strip_ansi_removes_csi_sequences
    // -------------------------------------------------------------------
    #[test]
    fn test_strip_ansi_removes_csi_sequences() {
        let input = "\x1b[2m2026-04-29T21:13:36.882836Z\x1b[0m \x1b[33m WARN\x1b[0m \x1b[2mdoli_node::node::periodic\x1b[0m\x1b[2m:\x1b[0m [HEALTH] h=0";
        let clean = strip_ansi(input);
        assert!(!clean.contains('\x1b'));
        assert!(clean.contains("2026-04-29T21:13:36.882836Z"));
        assert!(clean.contains("[HEALTH] h=0"));
    }

    // -------------------------------------------------------------------
    // 2. test_strip_ansi_handles_no_escapes
    // -------------------------------------------------------------------
    #[test]
    fn test_strip_ansi_handles_no_escapes() {
        let plain = "2026-04-29T21:13:36.882836Z  INFO  doli_node::node::apply_block:  [BLOCK] Applied h=100";
        assert_eq!(strip_ansi(plain), plain);
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
    // 8. test_parse_line_returns_none_for_unknown_pattern
    // -------------------------------------------------------------------
    #[test]
    fn test_parse_line_returns_none_for_unknown_pattern() {
        assert!(parse_line("whatever random text").is_none());
        assert!(parse_line("").is_none());

        // A valid log line but not a diagnostic pattern
        let line = ansi_line(
            "2026-04-29T21:13:36.882836Z",
            " WARN",
            "doli_node::producer",
            "--force-start specified: skipping duplicate key detection",
        );
        assert!(parse_line(&line).is_none());
    }

    // -------------------------------------------------------------------
    // 9. test_replay_log_file_streams_small_fixture
    // -------------------------------------------------------------------
    #[test]
    fn test_replay_log_file_streams_small_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("test.log");

        let lines = [
            ansi_line("2026-05-01T00:00:01.000000Z", " INFO", "doli_node::node::apply_block", "[BLOCK] Applied h=1 hash=aabb producer=0011 slot=10 txs=1 epoch=0"),
            ansi_line("2026-05-01T00:00:02.000000Z", " INFO", "doli_node::node::apply_block", "[BLOCK] Applied h=2 hash=ccdd producer=0022 slot=11 txs=0 epoch=0"),
            ansi_line("2026-05-01T00:00:03.000000Z", " WARN", "doli_node::node::periodic", "[HEALTH] h=2 s=11 hash=ccdd0000 | peers=5 best_peer_h=100 best_peer_s=200 net_tip_h=100 net_tip_s=200 | sync_fails=0 state=\"Idle\" | snap_epoch=0 snap_bonds=3 snap_producers=3"),
            ansi_line("2026-05-01T00:00:04.000000Z", " INFO", "doli_node::node::rollback", "[ROLLBACK] Initiating: depth=1 local_h=2 target_h=1 gap=0 empty_headers=0 shallow_count=0"),
            ansi_line("2026-05-01T00:00:05.000000Z", " INFO", "doli_node::node::rollback", "Rolling back from height 2 to 1 for fork recovery"),
        ];

        std::fs::write(&log_path, lines.join("\n")).unwrap();

        let result = replay_log_file(&log_path).unwrap();
        assert_eq!(result.len(), 5);
        assert!(result.iter().all(|r| r.replayed_from_log));
        assert_eq!(result[0].event.kind, EventKind::BlockApplied);
        assert_eq!(result[1].event.kind, EventKind::BlockApplied);
        assert_eq!(result[2].event.kind, EventKind::RecoveryClassifyCall);
        assert_eq!(result[3].event.kind, EventKind::RollbackStarted);
        assert_eq!(result[4].event.kind, EventKind::RollbackStarted);
    }

    // -------------------------------------------------------------------
    // 10. test_file_size_cap_rejects_oversize
    // -------------------------------------------------------------------
    #[test]
    fn test_file_size_cap_rejects_oversize() {
        // Unit-test the check_file_size function directly.
        assert!(check_file_size(0).is_ok());
        assert!(check_file_size(MAX_LOG_FILE_BYTES).is_ok());
        assert!(check_file_size(MAX_LOG_FILE_BYTES + 1).is_err());
        let err = check_file_size(6 * 1024 * 1024 * 1024).unwrap_err();
        assert!(err.to_string().contains("too large"));
    }

    // -------------------------------------------------------------------
    // 11. test_replayed_event_wraps_diagnostic_event
    // -------------------------------------------------------------------
    #[test]
    fn test_replayed_event_wraps_diagnostic_event() {
        let ev = DiagnosticEvent {
            event_id: ulid::Ulid::new().to_string(),
            kind: EventKind::BlockApplied,
            timestamp_ms: 1716200000000,
            height: Some(100),
            correlation_key: None,
            caused_by_event_id: None,
            is_cascade_origin: false,
            payload: EventPayload::BlockApplied {
                slot: 10,
                block_hash: "abcd".to_string(),
                producer_pubkey: "0011".to_string(),
                from_peer_id: None,
                received_at_ms: None,
                applied_at_ms: 1716200000000,
                validation_duration_ms: 0,
                mode: "Full".to_string(),
                tx_count: 1,
            },
        };
        let replayed = ReplayedEvent {
            event: ev.clone(),
            replayed_from_log: true,
        };
        assert!(replayed.replayed_from_log);
        assert_eq!(replayed.event, ev);

        // Verify the underlying DiagnosticEvent round-trips through
        // bincode without any format changes.
        let encoded = crate::diagnostic_ledger::types::encode_event(&ev).expect("encode");
        let decoded = crate::diagnostic_ledger::types::decode_event(&encoded).expect("decode");
        assert_eq!(decoded, ev);
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
    // 16. test_apply_start_is_skipped
    // -------------------------------------------------------------------
    #[test]
    fn test_apply_start_is_skipped() {
        let line = ansi_line(
            "2026-05-07T11:13:52.184040Z",
            " INFO",
            "doli_node::node::block_handling",
            "[APPLY_START] slot=111482 hash=b381563e59c1ecf8",
        );
        assert!(parse_line(&line).is_none());
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
