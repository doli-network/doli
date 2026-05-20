# Milestone Progress — Workflow #346

**Feature**: Fork-diagnostic observability (Phase 1)
**Architecture**: `specs/fork-observability-architecture.md`
**Plan**: `docs/.workflow/milestone-plan.md`

| ID | Name | Status | Test-Writer | Developer | QA | Reviewer | Commit |
|----|------|--------|-------------|-----------|-----|---------|--------|
| M1 | Types + Ledger + Emitter Trait | COMPLETE | DONE (40 tests) | DONE (bccb1bdf) | DONE (APPROVED) | — | bccb1bdf |
| M2 | Writer Task + Emit Sites | DEV_DONE | DONE (29+3 tests) | DONE (1ffc5df8 + 32327fdc) | — | — | 1ffc5df8 + 32327fdc |
| M3 | Queries + Classifier + RPC | PENDING | — | — | — | — | — |
| M4 | CLI + Docs | PENDING | — | — | — | — | — |

## M1 Test Traceability

| Requirement | Test File | Test Function(s) |
|-------------|-----------|-------------------|
| REQ-FORKOBS-LEDGER-001 | diagnostic_ledger_io_test | test_open_creates_rocksdb_directory, test_open_is_separate_from_state_db, test_ledger_persistence_across_reopen |
| REQ-FORKOBS-LEDGER-002 | diagnostic_types_test | test_event_key_composite_format, test_event_key_ordering_within_kind; diagnostic_ledger_io_test: test_record_event_at_max_height |
| REQ-FORKOBS-LEDGER-003 | diagnostic_types_test | test_event_bincode_roundtrip_block_applied, test_event_bincode_roundtrip_all_kinds, test_format_marker_byte_present, test_schema_version_present_and_current, test_decoder_rejects_unknown_future_schema_version, test_decoder_accepts_current_schema_version, test_diagnostic_bundle_roundtrip_empty_events, test_diagnostic_bundle_roundtrip_with_classification |
| REQ-FORKOBS-LEDGER-004 | diagnostic_ledger_io_test | test_prune_removes_age_expired_events, test_prune_all_events_stale, test_prune_empty_ledger_is_noop |
| REQ-FORKOBS-LEDGER-005 | diagnostic_ledger_io_test | test_prune_respects_max_events_cap, test_prune_preserves_cascade_origin_pin, test_prune_preserves_multiple_cascade_origins |
| REQ-FORKOBS-LEDGER-006 | diagnostic_ledger_io_test | test_record_and_query_recent_roundtrip, test_record_to_degraded_ledger_is_noop, test_record_event_at_height_zero, test_record_event_with_none_height |
| REQ-FORKOBS-LEDGER-007 | diagnostic_ledger_io_test | test_record_and_query_range_by_kind |
| REQ-FORKOBS-LEDGER-008 | diagnostic_ledger_io_test | test_record_and_query_recent_roundtrip, test_query_recent_respects_window |
| REQ-FORKOBS-LEDGER-009 | diagnostic_ledger_io_test | test_open_fails_gracefully_on_permission_error, test_record_to_degraded_ledger_is_noop |
| REQ-FORKOBS-PERF-001 | diagnostic_emitter_test | test_async_channel_emitter_record_is_nonblocking, test_async_channel_emitter_drop_oldest_on_full, test_async_channel_emitter_dropped_counter_exposed |
| REQ-FORKOBS-PERF-002 | diagnostic_emitter_test | test_diagnostic_emitter_is_send_sync, test_arc_dyn_emitter_is_send_sync |
| REQ-FORKOBS-SEC-003 (prereq) | diagnostic_ledger_io_test | test_query_range_respects_limit_cap |
| REQ-FORKOBS-RETRO-003 | diagnostic_types_test | test_classification_unknown_variant_carries_evidence |
| REQ-FORKOBS-EMIT-010 | diagnostic_types_test | test_correlation_key_canonical_block_has_all_none, test_correlation_key_full_roundtrip |
| REQ-FORKOBS-EMIT-001 | diagnostic_types_test | test_block_provenance_roundtrip |
| REQ-FORKOBS-CLF-002 | diagnostic_types_test | test_fork_type_all_variants_roundtrip |
| O3 (cascade-origin pin) | diagnostic_ledger_io_test | test_prune_preserves_cascade_origin_pin, test_prune_preserves_multiple_cascade_origins |

## M2 Test Traceability

| Requirement | Test File | Test Function(s) |
|-------------|-----------|-------------------|
| REQ-FORKOBS-EMIT-001 | diagnostic_emit_test | test_apply_block_success_emits_block_applied, test_apply_block_gossip_has_peer_provenance, test_apply_block_self_produced_has_none_provenance, test_apply_block_reorg_replay_has_none_provenance, test_multiple_blocks_each_produce_one_event, test_already_known_block_emits_nothing, test_event_ids_are_unique |
| REQ-FORKOBS-EMIT-002 | diagnostic_emit_test | test_apply_block_failure_emits_block_rejected |
| REQ-FORKOBS-EMIT-003 | diagnostic_emit_test | test_classify_gossip_block_fork_emits_event, test_classify_gossip_block_orphan_emits_event, test_classify_gossip_block_rejected_emits_event, test_classify_gossip_block_reorg_candidate_emits_event |
| REQ-FORKOBS-EMIT-004 | diagnostic_emit_test | test_rollback_emits_started_and_completed, test_rollback_at_genesis_emits_nothing |
| REQ-FORKOBS-EMIT-005 | diagnostic_emit_test | test_rollback_emits_started_and_completed (caused_by_event_id assertion) |
| REQ-FORKOBS-EMIT-006 | diagnostic_writer_pruner_test | test_reorg_executed_event_structure |
| REQ-FORKOBS-EMIT-007 | diagnostic_writer_pruner_test | test_recovery_classify_event_has_all_11_fields |
| REQ-FORKOBS-EMIT-010 | diagnostic_emit_test | test_fork_event_carries_correlation_key, test_canonical_block_applied_has_empty_correlation_key |
| REQ-FORKOBS-EMIT-011 / O4 | diagnostic_emit_test | test_apply_block_signature_includes_provenance_param, test_apply_block_gossip_has_peer_provenance, test_apply_block_self_produced_has_none_provenance, test_apply_block_reorg_replay_has_none_provenance |
| REQ-FORKOBS-LEDGER-004 | diagnostic_writer_pruner_test | test_pruner_removes_age_expired_events |
| REQ-FORKOBS-LEDGER-005 | diagnostic_writer_pruner_test | test_pruner_count_cap_prunes_oldest |
| REQ-FORKOBS-LEDGER-006 | diagnostic_writer_pruner_test | test_writer_task_drains_channel_to_ledger, test_writer_task_shutdown_drains_pending |
| REQ-FORKOBS-PERF-001 | diagnostic_writer_pruner_test | test_writer_task_increments_dropped_counter_on_overflow |
| REQ-FORKOBS-SEC-001 | diagnostic_writer_pruner_test | test_no_ip_address_in_any_event_payload |
| REQ-FORKOBS-SEC-005 | diagnostic_emit_test | test_node_with_no_config_emits_events |
| REQ-FORKOBS-SEC-006 | diagnostic_writer_pruner_test | test_no_activation_height_added, test_no_hardfork_schedule_entry_added |
| Emit failure graceful | diagnostic_emit_test | test_emit_failure_does_not_affect_apply_block |
| E2E wiring (LEDGER-006) | diagnostic_e2e_test | test_e2e_event_flows_from_emit_to_ledger |
| E2E pruning (LEDGER-004) | diagnostic_e2e_test | test_e2e_pruner_removes_old_events |
| E2E graceful degradation (LEDGER-009) | diagnostic_e2e_test | test_e2e_node_starts_when_diagnostics_dir_unwritable |

## M2 Follow-Up Notes
- Writer task (`diagnostic_writer.rs`): drains AsyncChannelEmitter receiver in batches of 16, writes to DiagnosticLedger, emits WriterHeartbeat every 60s directly to ledger (FM-4b), drains all remaining events on shutdown.
- Pruner task (`diagnostics_pruner.rs`): runs every 60s, reads DOLI_DIAG_RETENTION_DAYS (default 30) and DOLI_DIAG_MAX_EVENTS (default 100k) from env, calls ledger.prune().
- Init wiring (`init.rs`): production Node::new() opens DiagnosticLedger, creates AsyncChannelEmitter(1024), spawns writer+pruner tasks via tokio::spawn with watch::channel shutdown. Falls back to NoOpEmitter on failure.
- Test/replay constructors: unchanged (NoOpEmitter, no tasks spawned).
- Genesis-mismatch guard at `block_handling.rs:~433` (ExtendsTip arm): PENDING REVIEWER DECISION — added in 1ffc5df8, not modified in follow-up. Reviewer will rule on keep/refactor/revert.

## Decisions in effect (from architect O1-O7)
- O1: trait DiagnosticEmitter + AsyncChannelEmitter writer task
- O2: bincode + format-marker byte
- O3: cascade-origin pin in pruner
- O4: explicit `Option<BlockProvenance>` parameter on apply_block (user-approved, supersedes REQ-FORKOBS-EMIT-011 side-channel)
- O5: first-match-wins classifier rules; 300s temporal window for PostSnapDeadTip; "no other signals" = no fork-classified event in same correlation_key group
- O6: extract `bins/node/src/node/diagnostics_pruner.rs` from periodic.rs
- O7: `ulid` crate accepted as new dependency
