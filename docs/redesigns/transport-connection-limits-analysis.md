# INC-I-014: Connection Limit Analysis — RAM Crash at 103+ Nodes

## Verdict: Targeted Fix, NOT Architectural Redesign

The root cause is a **2-line omission** in the existing libp2p `ConnectionLimits` config.
The proposed "Custom Transport wrapper" (Fix A) and "RSS admission controller" (Fix C) are
unnecessary — the library already provides the exact API needed.

---

## Root Cause (Code-Verified)

At `crates/network/src/service/mod.rs:143-147`, the code sets 4 of 6 available limit methods:

| Method | Set? | What it Controls |
|--------|------|-----------------|
| `with_max_established_per_peer` | Yes (1) | Max established connections per peer |
| `with_max_established` | Yes (max_peers×2) | Total established connections |
| `with_max_established_incoming` | Yes (max_peers×2) | Established inbound |
| `with_max_established_outgoing` | Yes (max_peers×2) | Established outbound |
| **`with_max_pending_incoming`** | **NO** | Pending inbound (handshaking) |
| **`with_max_pending_outgoing`** | **NO** | Pending outbound (handshaking) |

Each pending connection allocates ~1MB (Yamux 512KB + Noise buffers) with ZERO cap.
At 156 nodes, `pending_out=37/node` = 37MB/node in invisible handshake buffers.

### Why `with_max_pending_*` Solves This

Verified in `libp2p-connection-limits` v0.3.1 source (`lib.rs:208-272`):
- `handle_pending_inbound_connection()` checks limit **BEFORE** Noise+Yamux handshake
- `handle_pending_outbound_connection()` checks limit **BEFORE** dial proceeds
- `ConnectionDenied` return prevents buffer allocation entirely
- TCP socket is opened but expensive crypto/mux buffers are NOT allocated

---

## Evaluation of Proposed Fixes

| Fix | Verdict | Rationale |
|-----|---------|-----------|
| **A. Custom Transport wrapper** | REJECT | Reimplements what `with_max_pending_*` already does. ~100 lines of complex concurrent code vs 2 API calls. |
| **B. Yamux window 512KB→256KB** | ACCEPT (Should) | Defense-in-depth. Halves per-connection cost. Sync headers ~100KB, so 256KB is 2.5x headroom. |
| **C. RSS admission controller** | REJECT | Reactive (fires after damage). Pending limits are proactive (prevent allocation). Platform-specific. |

---

## Requirements

| ID | Priority | Requirement | Acceptance Criteria |
|----|----------|------------|-------------------|
| REQ-NET-CONN-001 | Must | Add `with_max_pending_incoming` and `with_max_pending_outgoing` to ConnectionLimits | 156-node test: pending never exceeds limit; RAM < 16GB |
| REQ-NET-CONN-002 | Must | Pending limits configurable via `DOLI_PENDING_LIMIT` env var | Default: max_peers. Override works. Logged in MEM-BOOT. |
| REQ-NET-CONN-003 | Should | Reduce Yamux window default from 512KB to 256KB | `transport.rs` constant changed. `DOLI_YAMUX_WINDOW` override works. Sync still works. |
| REQ-NET-CONN-004 | Should | Add pending counts to periodic health logs | `pending_in` and `pending_out` in MEM-CONN milestone logs |
| REQ-NET-CONN-005 | Won't | Custom Transport wrapper | N/A — library provides this |
| REQ-NET-CONN-006 | Won't | RSS-based admission controller | N/A — pending limits solve root cause |

---

## Blast Radius

- **Direct**: `service/mod.rs:143-147` — add 2 method calls + env var parsing
- **Direct**: `transport.rs:17` — change constant 524288 → 262144
- **Indirect**: None. Pending limits operate below application layer. Eviction, bootstrap, gossip unaffected.

## Risks

- **Bootstrap starvation (low)**: Pending limit = max_peers (25-50), more than Kademlia needs
- **Silent drops (low)**: Mitigated by REQ-NET-CONN-004 (pending counts in health logs)
- **Yamux 256KB too small (very low)**: Largest protocol message ~100KB; 2.5x headroom
