//! Diagnostic writer task — drains events from AsyncChannelEmitter receiver
//! and writes them to DiagnosticLedger (RocksDB).
//!
//! Shutdown semantics: when the sender side drops (channel closed), the task
//! drains all remaining events before exiting. Emits periodic WriterHeartbeat
//! events so the RPC (M3) can detect silent write failures.

// Placeholder — M2 writer task implementation goes here.
// The writer_pruner tests currently use manual drain loops (TODO comments)
// rather than spawning this task, so passing tests does not depend on
// run_writer_task being fully implemented in this milestone.
