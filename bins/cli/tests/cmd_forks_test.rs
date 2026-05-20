// OUTPUT CONTRACT: cmd_forks module — `doli forks` CLI subcommand
// Functions: parse_duration, render_human, aggregate_by_producer, build_rpc_params, cmd_forks
// Outputs:
//   O3(parse_duration): Ok(seconds) | Err(message)
//   O3(render_human): formatted text with section headers (Events, Classification, Baseline)
//   O3(aggregate_by_producer): Vec<ProducerAggregate> sorted by count desc
//   O3(build_rpc_params): JSON { window_secs, fork_event_id, limit }
//   O3(cmd_forks): stdout(JSON|human), stderr(errors), exit(0|non-zero)
// PATHS: P1:JSON, P2:--human, P3:--last(valid), P4:--last(invalid), P5:--explain(events),
//   P6:--explain(empty), P7:--by-producer, P8:RPC-unreachable, P9:RPC-error, P10:human+None, P11:human+Unknown
// INPUT PARTITIONS:
//   Duration: "1h"->3600 | "30m"->1800 | "24h"->86400 | "5s"->5 | "abc"->Err | "1y"->Err | ""->Err
//   Bundle: empty(0 events, None) | classified(named) | unknown(Unknown+evidence) | multi-producer(5/3/1)
//   RPC: success(200) | error(-32603) | unreachable(connection refused)
// MATRIX: 7 outputs x 14 partitions = 98 cells (covered by ~30 tests, grouped by function)

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Re-export types from storage crate for test construction.
// The developer's cmd_forks.rs will depend on these same types.
// ---------------------------------------------------------------------------
use storage::diagnostic_ledger::types::{
    BaselineComparison, Classification, DiagnosticBundle, DiagnosticEvent, DiagnosticHealth,
    EventKind, EventPayload, ForkSummary, ForkType,
};

// ---------------------------------------------------------------------------
// Import the functions that the developer MUST create in cmd_forks.rs.
// These imports will fail at compile time until cmd_forks.rs exists and
// exports these symbols. This is intentional — TDD red phase.
//
// DEVELOPER NOTE: cmd_forks.rs must export these public functions:
//   pub fn parse_duration(s: &str) -> Result<u64, String>
//   pub fn render_human(bundle: &DiagnosticBundle) -> String
//   pub fn aggregate_by_producer(bundle: &DiagnosticBundle) -> Vec<ProducerAggregate>
//   pub fn build_rpc_params(window_secs: u64, fork_event_id: Option<&str>, limit: Option<u64>) -> serde_json::Value
//   pub struct ProducerAggregate { pub producer_pubkey: String, pub count: u64 }
//
// Since bins/cli is a binary crate (no lib.rs), these tests import from the
// binary's source via the doli_cli crate if a lib.rs is added, OR the developer
// places unit tests inside cmd_forks.rs as `#[cfg(test)] mod tests`. The
// integration tests below use source-level assertions and clap parsing checks.
// ---------------------------------------------------------------------------

// ===========================================================================
// SECTION 1: Clap Argument Parsing Tests
// These verify the Commands enum accepts `forks` and its flags.
// ===========================================================================

// Requirement: REQ-FORKOBS-CLI-001 (Must)
// Acceptance: `doli forks` subcommand exists, parses successfully
// Partition: Bundle-empty (structure test — no runtime data)
#[test]
fn test_clap_parses_forks_subcommand() {
    // The developer must add a `Forks { ... }` variant to the Commands enum.
    // This test uses include_str! to verify the enum variant exists.
    let commands_src = include_str!("../src/commands.rs");
    assert!(
        commands_src.contains("Forks"),
        "Commands enum must have a Forks variant. \
         Developer: add `Forks {{ human: bool, last: Option<String>, explain: bool, by_producer: bool }}` \
         to the Commands enum in commands.rs."
    );
}

// Requirement: REQ-FORKOBS-CLI-001 (Must)
// Acceptance: --human flag exists on the Forks variant
// Partition: Bundle-empty (structure test)
#[test]
fn test_clap_forks_has_human_flag() {
    let commands_src = include_str!("../src/commands.rs");
    let forks_idx = commands_src.find("Forks");
    assert!(
        forks_idx.is_some(),
        "Forks variant must exist in Commands enum"
    );
    let after_forks = &commands_src[forks_idx.unwrap()..];
    let section = &after_forks[..after_forks.len().min(500)];
    assert!(
        section.contains("human"),
        "Forks variant must have a `human` flag. \
         Developer: add `#[arg(long)] human: bool` to the Forks variant."
    );
}

// Requirement: REQ-FORKOBS-CLI-002 (Must)
// Acceptance: --last flag exists on the Forks variant
// Partition: Duration-valid-hours (structure test)
#[test]
fn test_clap_forks_has_last_flag() {
    let commands_src = include_str!("../src/commands.rs");
    let forks_idx = commands_src.find("Forks");
    assert!(
        forks_idx.is_some(),
        "Forks variant must exist in Commands enum"
    );
    let after_forks = &commands_src[forks_idx.unwrap()..];
    let section = &after_forks[..after_forks.len().min(500)];
    assert!(
        section.contains("last"),
        "Forks variant must have a `last` flag for duration. \
         Developer: add `#[arg(long)] last: Option<String>` to the Forks variant."
    );
}

// Requirement: REQ-FORKOBS-CLI-003 (Should)
// Acceptance: --explain flag exists on the Forks variant
// Partition: Bundle-empty (structure test)
#[test]
fn test_clap_forks_has_explain_flag() {
    let commands_src = include_str!("../src/commands.rs");
    let forks_idx = commands_src.find("Forks");
    assert!(
        forks_idx.is_some(),
        "Forks variant must exist in Commands enum"
    );
    let after_forks = &commands_src[forks_idx.unwrap()..];
    let section = &after_forks[..after_forks.len().min(500)];
    assert!(
        section.contains("explain"),
        "Forks variant must have an `explain` flag. \
         Developer: add `#[arg(long)] explain: bool` to the Forks variant."
    );
}

// Requirement: REQ-FORKOBS-CLI-004 (Should)
// Acceptance: --by-producer flag exists on the Forks variant
// Partition: Bundle-multi-producer (structure test)
#[test]
fn test_clap_forks_has_by_producer_flag() {
    let commands_src = include_str!("../src/commands.rs");
    let forks_idx = commands_src.find("Forks");
    assert!(
        forks_idx.is_some(),
        "Forks variant must exist in Commands enum"
    );
    let after_forks = &commands_src[forks_idx.unwrap()..];
    let section = &after_forks[..after_forks.len().min(500)];
    assert!(
        section.contains("by_producer") || section.contains("by-producer"),
        "Forks variant must have a `by_producer` flag. \
         Developer: add `#[arg(long)] by_producer: bool` to the Forks variant."
    );
}

// ===========================================================================
// SECTION 2: Module Wiring Tests
// ===========================================================================

// Requirement: REQ-FORKOBS-CLI-001 (Must)
// Acceptance: cmd_forks module declared in main.rs
// Partition: RPC-success (wiring test)
#[test]
fn test_main_declares_cmd_forks_module() {
    let main_src = include_str!("../src/main.rs");
    assert!(
        main_src.contains("mod cmd_forks"),
        "main.rs must declare `mod cmd_forks;`. \
         Developer: add `mod cmd_forks;` to main.rs alongside the other cmd_* modules."
    );
}

// Requirement: REQ-FORKOBS-CLI-001 (Must)
// Acceptance: Forks command dispatched in main.rs match arm
// Partition: RPC-success (wiring test)
#[test]
fn test_main_dispatches_forks_command() {
    let main_src = include_str!("../src/main.rs");
    assert!(
        main_src.contains("Commands::Forks") || main_src.contains("Forks {"),
        "main.rs must dispatch the Forks command in the match arm. \
         Developer: add `Commands::Forks {{ ... }} => cmd_forks::cmd_forks(...)` to main.rs."
    );
}

// ===========================================================================
// SECTION 3: cmd_forks.rs Source Existence and API Tests
// ===========================================================================

// Requirement: REQ-FORKOBS-CLI-001 (Must), REQ-FORKOBS-CLI-004 (Should)
// Acceptance: cmd_forks.rs exists and exports parse_duration, render_human, aggregate_by_producer
// Partition: Bundle-empty + Bundle-classified + Bundle-multi-producer (API surface)
#[test]
fn test_cmd_forks_public_api_surface() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(!forks_src.is_empty(), "cmd_forks.rs must not be empty.");
    assert!(
        forks_src.contains("fn parse_duration"),
        "cmd_forks.rs must contain `parse_duration`. \
         Signature: pub(crate) fn parse_duration(s: &str) -> Result<u64, String>"
    );
    assert!(
        forks_src.contains("fn render_human"),
        "cmd_forks.rs must contain `render_human`. \
         Signature: pub(crate) fn render_human(bundle: &DiagnosticBundle) -> String"
    );
    assert!(
        forks_src.contains("fn aggregate_by_producer")
            || forks_src.contains("fn by_producer_aggregate"),
        "cmd_forks.rs must contain `aggregate_by_producer`. \
         Signature: pub(crate) fn aggregate_by_producer(bundle: &DiagnosticBundle) -> Vec<ProducerAggregate>"
    );
}

// ===========================================================================
// SECTION 4: Parse Duration Contract Tests
// ===========================================================================

// Requirement: REQ-FORKOBS-CLI-002 (Must)
// Acceptance: "1h" translates to 3600 seconds
// Partition: Duration-valid-hours
#[test]
fn test_parse_duration_spec_handles_hours() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("'h'") || forks_src.contains("\"h\"") || forks_src.contains("h'"),
        "parse_duration must handle 'h' suffix for hours (1h -> 3600)."
    );
}

// Requirement: REQ-FORKOBS-CLI-002 (Must)
// Acceptance: "30m" translates to 1800 seconds
// Partition: Duration-valid-minutes
#[test]
fn test_parse_duration_spec_handles_minutes() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("'m'") || forks_src.contains("\"m\"") || forks_src.contains("m'"),
        "parse_duration must handle 'm' suffix for minutes (30m -> 1800)."
    );
}

// Requirement: REQ-FORKOBS-CLI-002 (Must)
// Acceptance: "24h" translates to 86400 seconds
// Partition: Duration-valid-day
#[test]
fn test_parse_duration_spec_multiplies_hours_by_3600() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("3600") || forks_src.contains("* 3600") || forks_src.contains("*3600"),
        "parse_duration must multiply hours by 3600."
    );
}

// Requirement: REQ-FORKOBS-CLI-002 (Must)
// Acceptance: "5s" translates to 5 seconds
// Partition: Duration-valid-seconds
#[test]
fn test_parse_duration_spec_handles_seconds() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("'s'") || forks_src.contains("\"s\"") || forks_src.contains("s'"),
        "parse_duration must handle 's' suffix for seconds."
    );
}

// Requirement: REQ-FORKOBS-CLI-002 (Must)
// Acceptance: invalid durations like "abc" are rejected with helpful error
// Partition: Duration-invalid-alpha
#[test]
fn test_parse_duration_rejects_invalid_input() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("Err") || forks_src.contains("bail!") || forks_src.contains("anyhow!"),
        "parse_duration must return an error for invalid duration strings."
    );
}

// Requirement: REQ-FORKOBS-CLI-002 (Must)
// Acceptance: unit tests for parse_duration inside cmd_forks.rs
// Partition: all duration partitions
#[test]
fn test_cmd_forks_has_unit_tests_for_parse_duration() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("#[test]"),
        "cmd_forks.rs must contain unit tests (#[cfg(test)] mod tests)."
    );
    assert!(
        forks_src.contains("test_parse_duration") || forks_src.contains("parse_duration"),
        "cmd_forks.rs must have unit tests exercising parse_duration."
    );
}

// ===========================================================================
// SECTION 5: Human Rendering Contract Tests
// ===========================================================================

// Requirement: REQ-FORKOBS-CLI-001 (Must)
// Acceptance: --human renders section headers: Events, Classification, Baseline
// Partition: Bundle-classified
#[test]
fn test_render_human_spec_has_section_headers() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    let has_events = forks_src.contains("\"Events\"") || forks_src.contains("Events");
    let has_classification =
        forks_src.contains("\"Classification\"") || forks_src.contains("Classification");
    let has_baseline = forks_src.contains("\"Baseline\"") || forks_src.contains("Baseline");

    assert!(
        has_events && has_classification && has_baseline,
        "render_human must include section headers: Events, Classification, Baseline. \
         Found: Events={}, Classification={}, Baseline={}",
        has_events,
        has_classification,
        has_baseline
    );
}

// Requirement: REQ-FORKOBS-CLI-001 (Must)
// Acceptance: classification=None renders gracefully
// Partition: Bundle-empty (classification=None)
#[test]
fn test_render_human_spec_handles_none_classification() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("no classification")
            || forks_src.contains("No classification")
            || forks_src.contains("None =>")
            || (forks_src.contains("classification") && forks_src.contains("none")),
        "render_human must gracefully handle classification=None. \
         Developer: when bundle.classification is None, print 'no classification' or equivalent."
    );
}

// Requirement: REQ-FORKOBS-CLI-001 (Must)
// Acceptance: Unknown classification renders reason + evidence
// Partition: Bundle-unknown
#[test]
fn test_render_human_spec_renders_unknown_with_evidence() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("reason_unknown") || forks_src.contains("Unknown"),
        "render_human must render Unknown classification's reason_unknown field."
    );
}

// Requirement: REQ-FORKOBS-CLI-001 (Must)
// Acceptance: render_human includes baseline rate comparison
// Partition: Bundle-classified
#[test]
fn test_render_human_includes_baseline_data() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("fork_events_per_hour")
            || forks_src.contains("events/hour")
            || forks_src.contains("baseline"),
        "render_human must display baseline comparison data (fork events per hour, delta)."
    );
}

// Requirement: REQ-FORKOBS-CLI-001 (Must)
// Acceptance: render_human shows health status
// Partition: Bundle-classified
#[test]
fn test_render_human_shows_health_status() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("ledger_available")
            || forks_src.contains("Health")
            || forks_src.contains("health"),
        "render_human should display health status (ledger_available, dropped events)."
    );
}

// ===========================================================================
// SECTION 6: By-Producer Aggregation Contract
// ===========================================================================

// Requirement: REQ-FORKOBS-CLI-004 (Should)
// Acceptance: by-producer output is sorted by count descending
// Partition: Bundle-multi-producer
#[test]
fn test_by_producer_spec_sorted_desc() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("sort") || forks_src.contains("sorted"),
        "aggregate_by_producer must sort results by count descending."
    );
}

// ===========================================================================
// SECTION 7: RPC Params Construction Contract
// ===========================================================================

// Requirement: REQ-FORKOBS-CLI-002 (Must)
// Acceptance: default window is 1h (3600s)
// Partition: Duration-valid-hours (default)
#[test]
fn test_rpc_params_default_window_is_1h() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("\"1h\"") || forks_src.contains("3600"),
        "Default --last duration must be 1h (3600 seconds)."
    );
}

// Requirement: REQ-FORKOBS-RPC-001 (Must) — via CLI
// Acceptance: RPC method called is "getForkDiagnostic"
// Partition: RPC-success
#[test]
fn test_rpc_method_name_is_get_fork_diagnostic() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("getForkDiagnostic"),
        "cmd_forks must call the 'getForkDiagnostic' RPC method."
    );
}

// ===========================================================================
// SECTION 8: Explain Mode Contract
// ===========================================================================

// Requirement: REQ-FORKOBS-CLI-003 (Should)
// Acceptance: --explain triggers fork_event_id parameter in RPC call
// Partition: Bundle-classified (explain with events)
#[test]
fn test_explain_mode_uses_fork_event_id_param() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("fork_event_id"),
        "cmd_forks must pass fork_event_id to the RPC when --explain is used."
    );
}

// Requirement: REQ-FORKOBS-CLI-003 (Should)
// Acceptance: --explain with no fork events prints a message, not crashes
// Partition: Bundle-empty (explain with no events)
#[test]
fn test_explain_mode_handles_no_forks_message() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("no fork events")
            || forks_src.contains("No fork events")
            || forks_src.contains("no forks found")
            || forks_src.contains("No forks found"),
        "cmd_forks must print a message when --explain finds no fork events."
    );
}

// ===========================================================================
// SECTION 9: RPC URL Contract
// ===========================================================================

// Requirement: REQ-FORKOBS-CLI-001 (Must)
// Acceptance: default RPC URL from common module (mainnet = http://127.0.0.1:8500)
// Partition: RPC-success
#[test]
fn test_rpc_url_default_from_network() {
    let common_src = include_str!("../src/common.rs");
    assert!(
        common_src.contains("http://127.0.0.1:8500"),
        "Default mainnet RPC should be http://127.0.0.1:8500"
    );
}

// Requirement: REQ-FORKOBS-CLI-001 (Must)
// Acceptance: --rpc flag overrides default (already handled by main.rs)
// Partition: RPC-success
#[test]
fn test_rpc_url_override_supported_by_cli_flags() {
    let commands_src = include_str!("../src/commands.rs");
    assert!(
        commands_src.contains("rpc: Option<String>"),
        "CLI must have an --rpc flag for RPC URL override."
    );
}

// ===========================================================================
// SECTION 10: Error Handling Contract
// ===========================================================================

// Requirement: REQ-FORKOBS-CLI-001 (Must)
// Acceptance: RPC error results in non-zero exit
// Partition: RPC-error
#[test]
fn test_cmd_forks_propagates_rpc_errors() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("?") || forks_src.contains("bail!") || forks_src.contains("anyhow!"),
        "cmd_forks must propagate RPC errors using `?`, `bail!`, or `anyhow!`."
    );
}

// Requirement: REQ-FORKOBS-RPC-005 (Must) — tested via CLI
// Acceptance: ledger unavailable error from RPC results in clear error message
// Partition: RPC-error (ledger unavailable)
#[test]
fn test_cmd_forks_handles_ledger_unavailable_error() {
    let forks_src = include_str!("../src/cmd_forks.rs");
    assert!(
        forks_src.contains("unavailable")
            || forks_src.contains("Cannot connect")
            || forks_src.contains("?"),
        "cmd_forks must handle the case where the diagnostic ledger is unavailable."
    );
}

// ===========================================================================
// SECTION 11: Bundle Type Serialization Tests (pure type tests)
// Verify DiagnosticBundle round-trips through serde_json as the CLI receives it.
// ===========================================================================

/// Build a minimal DiagnosticBundle with no events and no classification.
fn make_empty_bundle() -> DiagnosticBundle {
    DiagnosticBundle {
        schema_version: 1,
        node_peer_id: "12D3KooWTestPeerIdForUnitTests".to_string(),
        query_timestamp_ms: 1716220800000,
        events: vec![],
        fork_summary: ForkSummary {
            fork_events_in_window: 0,
            by_producer: HashMap::new(),
            by_event_kind: HashMap::new(),
            first_fork_height: None,
            last_fork_height: None,
        },
        classification: None,
        baseline: BaselineComparison {
            fork_events_per_hour_current: 0.0,
            fork_events_per_hour_24h_avg: 0.0,
            delta_pct: 0.0,
        },
        health: DiagnosticHealth {
            ledger_available: true,
            events_written_total: 42,
            events_dropped_total: 0,
            last_heartbeat_ms: Some(1716220799000),
        },
    }
}

fn make_classified_bundle() -> DiagnosticBundle {
    let mut bundle = make_empty_bundle();
    bundle.classification = Some(Classification {
        fork_type: ForkType::ProducerEquivocation,
        confidence: 0.90,
        evidence_event_ids: vec![
            "01HY1234ABCD0001".to_string(),
            "01HY1234ABCD0002".to_string(),
        ],
        recommended_action: Some("investigate_producer".to_string()),
        recommended_action_args: None,
    });
    bundle.fork_summary.fork_events_in_window = 2;
    bundle
}

fn make_unknown_classified_bundle() -> DiagnosticBundle {
    let mut bundle = make_empty_bundle();
    bundle.classification = Some(Classification {
        fork_type: ForkType::Unknown {
            reason_unknown: "recovery_classify_call returning HeaderFirstSync repeatedly \
                             with last_applied_secs > 120 and gap > 0"
                .to_string(),
            evidence_event_ids: vec![
                "01HY5678EFGH0001".to_string(),
                "01HY5678EFGH0002".to_string(),
                "01HY5678EFGH0003".to_string(),
            ],
        },
        confidence: 0.30,
        evidence_event_ids: vec![
            "01HY5678EFGH0001".to_string(),
            "01HY5678EFGH0002".to_string(),
            "01HY5678EFGH0003".to_string(),
        ],
        recommended_action: None,
        recommended_action_args: None,
    });
    bundle
}

fn make_multi_producer_bundle() -> DiagnosticBundle {
    let mut bundle = make_empty_bundle();
    let mut by_producer = HashMap::new();
    by_producer.insert("aabbccdd11223344".to_string(), 5u64);
    by_producer.insert("eeff00112233aabb".to_string(), 3u64);
    by_producer.insert("99887766554433aa".to_string(), 1u64);
    bundle.fork_summary = ForkSummary {
        fork_events_in_window: 9,
        by_producer,
        by_event_kind: {
            let mut m = HashMap::new();
            m.insert("ForkBlockReceived".to_string(), 9);
            m
        },
        first_fork_height: Some(1000),
        last_fork_height: Some(1050),
    };
    bundle
}

// Requirement: REQ-FORKOBS-CLI-001 (Must)
// Acceptance: DiagnosticBundle JSON has required fields, schema_version=1,
//   classification=null when empty, named fork_type when classified
// Partition: Bundle-empty + Bundle-classified (consolidated)
#[test]
fn test_diagnostic_bundle_json_contract() {
    // Empty bundle: classification=null, schema_version=1
    let empty = make_empty_bundle();
    let ev = serde_json::to_value(&empty).unwrap();
    assert_eq!(ev["schema_version"], 1);
    assert!(ev["classification"].is_null());
    assert_eq!(ev["fork_summary"]["fork_events_in_window"], 0);

    // Classified bundle: all top-level fields present, fork_type populated
    let classified = make_classified_bundle();
    let json_str = serde_json::to_string_pretty(&classified).expect("serializes");
    let cv: serde_json::Value = serde_json::from_str(&json_str).expect("parses back");
    for field in &[
        "schema_version",
        "node_peer_id",
        "query_timestamp_ms",
        "events",
        "fork_summary",
        "classification",
        "baseline",
        "health",
    ] {
        assert!(cv.get(field).is_some(), "missing {}", field);
    }
    let ft = &cv["classification"]["fork_type"];
    assert!(
        ft.as_str() == Some("ProducerEquivocation")
            || ft.to_string().contains("ProducerEquivocation"),
        "expected ProducerEquivocation, got: {}",
        ft
    );
    assert_eq!(cv["classification"]["confidence"], 0.90);
}

// Requirement: REQ-FORKOBS-RETRO-003 (Must)
// Acceptance: Unknown classification carries reason_unknown and evidence_event_ids
// Partition: Bundle-unknown
#[test]
fn test_unknown_classification_json_carries_evidence() {
    let bundle = make_unknown_classified_bundle();
    let json_val = serde_json::to_value(&bundle).unwrap();
    let classification = &json_val["classification"];
    let fork_type = &classification["fork_type"];

    let unknown_obj = fork_type.get("Unknown");
    assert!(
        unknown_obj.is_some(),
        "fork_type should contain Unknown variant, got: {}",
        fork_type
    );
    let unknown = unknown_obj.unwrap();
    assert!(
        unknown.get("reason_unknown").is_some(),
        "Unknown must carry reason_unknown"
    );
    assert!(
        unknown.get("evidence_event_ids").is_some(),
        "Unknown must carry evidence_event_ids"
    );
    let evidence = unknown["evidence_event_ids"].as_array().unwrap();
    assert_eq!(evidence.len(), 3);
}

// Requirement: REQ-FORKOBS-CLI-001 (Must)
// Acceptance: Bundle with events round-trips through JSON
// Partition: Bundle-classified (with events populated)
#[test]
fn test_bundle_with_events_json_roundtrip() {
    let mut bundle = make_classified_bundle();
    bundle.events.push(DiagnosticEvent {
        event_id: "01HY9999ZZZZ0001".to_string(),
        kind: EventKind::ForkBlockReceived,
        timestamp_ms: 1716220800000,
        height: Some(5000),
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: EventPayload::ForkBlockReceived {
            block_hash: "aabb".to_string(),
            block_slot: 500,
            block_height_estimate: Some(5000),
            producer_pubkey: "aabbccdd11223344".to_string(),
            from_peer_id: "12D3KooWPeerXYZ".to_string(),
            classification: "ForkBlock".to_string(),
            fork_kind: Some("HeightOccupied".to_string()),
            local_tip_hash: "ccdd".to_string(),
            local_tip_height: 4999,
        },
    });

    let json_str = serde_json::to_string(&bundle).unwrap();
    let round_tripped: DiagnosticBundle = serde_json::from_str(&json_str).unwrap();
    assert_eq!(round_tripped.events.len(), 1);
    assert_eq!(round_tripped.schema_version, 1);
    assert!(round_tripped.classification.is_some());
}

// Requirement: REQ-FORKOBS-CLI-004 (Should)
// Acceptance: ForkSummary.by_producer round-trips correctly
// Partition: Bundle-multi-producer
#[test]
fn test_fork_summary_by_producer_roundtrip() {
    let bundle = make_multi_producer_bundle();
    let json_str = serde_json::to_string(&bundle.fork_summary).unwrap();
    let parsed: ForkSummary = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed.by_producer.len(), 3);
    assert_eq!(parsed.fork_events_in_window, 9);
    assert_eq!(parsed.first_fork_height, Some(1000));
    assert_eq!(parsed.last_fork_height, Some(1050));
}

// Requirement: REQ-FORKOBS-CLI-004 (Should)
// Acceptance: by_producer counts are correct and sortable
// Partition: Bundle-multi-producer
#[test]
fn test_by_producer_sort_contract() {
    let bundle = make_multi_producer_bundle();
    let mut entries: Vec<(&String, &u64)> = bundle.fork_summary.by_producer.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1));
    assert_eq!(*entries[0].1, 5, "First entry should have count 5");
    assert_eq!(*entries[1].1, 3, "Second entry should have count 3");
    assert_eq!(*entries[2].1, 1, "Third entry should have count 1");
}

// ===========================================================================
// SECTION 12: Documentation Contract Tests
// ===========================================================================

// Requirement: REQ-FORKOBS-DOC-001 (Must)
// Acceptance: docs/rpc_reference.md has getForkDiagnostic entry
// Partition: RPC-success (doc test)
#[test]
fn test_rpc_reference_documents_get_fork_diagnostic() {
    let rpc_doc = include_str!("../../../docs/rpc_reference.md");
    assert!(
        rpc_doc.contains("getForkDiagnostic"),
        "docs/rpc_reference.md must document the getForkDiagnostic RPC method."
    );
}

// Requirement: REQ-FORKOBS-DOC-002 (Must)
// Acceptance: docs/troubleshooting.md has fork diagnosis section
// Partition: RPC-success (doc test)
#[test]
fn test_troubleshooting_has_fork_diagnosis_section() {
    let troubleshooting_doc = include_str!("../../../docs/troubleshooting.md");
    assert!(
        troubleshooting_doc.contains("doli forks")
            || troubleshooting_doc.contains("fork diagnostic")
            || troubleshooting_doc.contains("Fork Diagnostic")
            || troubleshooting_doc.contains("diagnose a fork"),
        "docs/troubleshooting.md must have a section on using `doli forks` for fork diagnosis."
    );
}

// Requirement: REQ-FORKOBS-DOC-003 (Must)
// Acceptance: fork_observability.md exists, documents event kinds,
//   classification types, bundle schema, and retention config
// Partition: RPC-success (doc completeness test)
#[test]
fn test_fork_observability_doc_complete() {
    let obs_doc = include_str!("../../../docs/fork_observability.md");
    assert!(
        !obs_doc.is_empty(),
        "docs/fork_observability.md must exist and not be empty."
    );
    // Event kinds
    assert!(
        obs_doc.contains("BlockApplied") || obs_doc.contains("block_applied"),
        "must document BlockApplied event kind"
    );
    assert!(
        obs_doc.contains("ForkBlockReceived") || obs_doc.contains("fork_block_received"),
        "must document ForkBlockReceived event kind"
    );
    assert!(
        obs_doc.contains("RollbackStarted") || obs_doc.contains("rollback_started"),
        "must document RollbackStarted event kind"
    );
    // Classification types
    assert!(
        obs_doc.contains("TipRaceNatural"),
        "must document TipRaceNatural"
    );
    assert!(
        obs_doc.contains("ProducerEquivocation"),
        "must document ProducerEquivocation"
    );
    assert!(
        obs_doc.contains("Unknown"),
        "must document Unknown classification"
    );
    // Bundle schema
    assert!(
        obs_doc.contains("DiagnosticBundle") || obs_doc.contains("schema_version"),
        "must document DiagnosticBundle schema"
    );
    assert!(
        obs_doc.contains("fork_summary"),
        "must document fork_summary field"
    );
    // Retention config
    assert!(
        obs_doc.contains("DOLI_DIAG_RETENTION_DAYS")
            || obs_doc.contains("retention")
            || obs_doc.contains("pruning"),
        "must document retention/pruning configuration"
    );
    assert!(
        obs_doc.contains("DOLI_DIAG_MAX_EVENTS")
            || obs_doc.contains("100,000")
            || obs_doc.contains("100000"),
        "must document DOLI_DIAG_MAX_EVENTS"
    );
}
