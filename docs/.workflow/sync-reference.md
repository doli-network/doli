# Reference Docs Sync Report — 2026-03-29

## Summary

Synced `docs/rpc_reference.md` and `docs/cli.md` against source code.

Source files read:
- `crates/rpc/src/methods/dispatch.rs` (39 method registrations)
- `crates/rpc/src/methods/*.rs` (16 handler files)
- `crates/rpc/src/types/**/*.rs` (4 type files)
- `crates/rpc/src/error.rs` (error codes)
- `crates/rpc/src/ws.rs` (WebSocket events)
- `crates/rpc/src/server.rs` (server setup)
- `bins/cli/src/commands.rs` (all subcommands/flags)
- `bins/cli/src/main.rs` (command dispatch)

## RPC Drift Found

### DRIFT-RPC-1: Error code -32007 says "Epoch not complete" — should be "Pool not found"
- **File:** docs/rpc_reference.md:1480
- **Code:** crates/rpc/src/error.rs:35 — `pub const POOL_NOT_FOUND: i32 = -32007;`
- **Fix:** Change error code -32007 description from "Epoch not complete" to "Pool not found"

### DRIFT-RPC-2: Error codes -32008 and -32009 do not exist in code
- **File:** docs/rpc_reference.md:1481-1482
- **Code:** crates/rpc/src/error.rs — no codes -32008 or -32009 defined
- **Fix:** Remove stale error codes -32008 ("Already claimed") and -32009 ("No reward")

### DRIFT-RPC-3: "Nix development environment" reference in cli.md is stale
- **File:** docs/cli.md:12-26
- **Code:** No Nix configuration in the project
- **Fix:** Remove Nix references, simplify to direct binary usage

## CLI Drift Found

### DRIFT-CLI-1: Section "Running Commands" references Nix shell — no Nix in project
- **File:** docs/cli.md:11-27
- **Fix:** Remove Nix references, keep cargo run and built binary examples

## All Other Areas — No Drift

- RPC method count: 39 methods in dispatch.rs, 39 documented — MATCHES
- RPC method names: all 39 methods verified — MATCHES
- RPC parameters: all param types checked against code types — MATCHES
- RPC response fields: verified against type structs — MATCHES
- WebSocket events: new_block and new_tx documented correctly — MATCHES
- CLI subcommands: all verified against commands.rs — MATCHES
- CLI flags: all flags verified against clap definitions — MATCHES
- CLI global options: -w, -r, -n verified against Cli struct — MATCHES
- Environment variables: DOLI_RPC_URL, DOLI_NETWORK, DOLI_WALLET_FILE verified — MATCHES
