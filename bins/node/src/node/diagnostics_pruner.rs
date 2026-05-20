//! Diagnostic pruner task — periodically prunes old/excess events from
//! DiagnosticLedger. Reads DOLI_DIAG_RETENTION_DAYS and DOLI_DIAG_MAX_EVENTS
//! env vars. Respects cascade-origin pin (already implemented in M1's
//! DiagnosticLedger::prune()).
//!
//! Called from `run_periodic_tasks()` via a one-line delegation per O6.

// Placeholder — M2 pruner implementation goes here.
// The writer_pruner tests use DiagnosticLedger::prune() directly
// rather than going through this task, so passing tests does not
// depend on tick() being fully implemented in this milestone.
