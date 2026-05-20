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

mod parsers;

use std::io::{BufRead, BufReader};
use std::path::Path;

use super::types::DiagnosticEvent;

use parsers::{
    parse_block_applied, parse_block_rejected, parse_chain_break, parse_fork_guard, parse_health,
    parse_reorg_complete, parse_rollback_initiating, parse_rolling_back_from, parse_snap_attempted,
    parse_snap_completed, parse_snap_failed, parse_stuck_sync,
};

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
}
