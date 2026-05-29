> ⚠️ **SUPERSEDED (2026-05-29).** The "Option 2 / live-P2P" plan below (milestones
> M0–M7: a `/doli/channel/1.0.0` subsystem, node channel key, trustless force-close,
> penalty/watchtower) was **REJECTED**. Economic premise check: DOLI settles on-chain
> in ~10s at a flat fee, so Lightning/Raiden-style channels solve a problem DOLI does
> not have. The shipped fix is the **trimmed scope**:
>   1. **Cooperative-close handoff** — `channel close` (build+sign offer) +
>      `channel close-finish` (counterparty co-signs+broadcasts). Library:
>      `channels::close::{build_cooperative_close_offer, finalize_cooperative_close_offer}`.
>      Test: `crates/channels/tests/inc_i_093_cooperative_close_handoff.rs`.
>   2. **Bridge proof + UX** — `crates/core/tests/inc_i_093_bridge_htlc.rs` proves the
>      evaluator is correct; it also surfaced a **real refund witness bug** (the CLI's
>      `branch(right)+none()` omitted the signed-refund signature) which is now fixed in
>      `cmd_bridge.rs`. MPTX007 now returns an actionable explanation.
>   3. **`--force`** returns a clear "not supported (roadmap)" error.
> No consensus change, no activation height, no synchronized deploy (wallet/CLI only).
> Trustless channels remain a separate roadmap item gated on a use case + economic review.
> The analysis below is retained for historical context only.

# INC-I-093 — DeFi multi-party CLI flows — Analysis & Plan (Option 2: full scope incl. force-close) [SUPERSEDED]

## Architecture Context
- **On-chain evaluator is correct** (`crates/core/src/conditions/eval.rs`). Proven by `crates/channels/tests/inc_i_092_close_covenant.rs` (PASSES). The covenant accepts: 2-of-2 multisig (funding), Hashlock+Timelock (HTLC claim), Sig+TimelockExpiry (signed refund), to_local revocable/delayed, penalty.
- **Crypto machinery exists** in `crates/channels/`:
  - `commitment.rs` (679L): `CommitmentPair`, `build_local_commitment` (revocable to_local + to_remote + HTLC outs), `RevocationStore`, `generate_revocation_preimage`, `build_penalty_witness`, `build_delayed_claim_witness`, `build_htlc_claim/timeout_witness`.
  - `conditions.rs` (586L): `funding_condition` (2-of-2), `to_local_condition` (Or(penalty=Sig+Revocation, delayed=Sig+Timelock)), `to_remote_output`, HTLC offered/received.
  - `close.rs`: `build_cooperative_close`, `sign_cooperative_close` (takes remote sig → 2-of-2 covenant witness), `build_force_close`, `build_penalty_tx`, `build_delayed_claim`.
  - `protocol.rs`: ChannelMessage TYPES (OpenChannel, UpdateCommitment, RevokeAndAck, AddHtlc, …) — **defined but NOT wired to any transport**.
  - `manager.rs` (185L) thin store wrapper; `monitor.rs` ChainMonitor; `state_machine.rs` validate_transition; `watchtower.rs` stub.

## Confirmed root causes
1. **Channel cooperative-close (CONFIRMED BUG)** — `bins/cli/src/cmd_channel.rs:352-400`: builds the close tx and sets only `tx.inputs[i].signature` (the input sig). It **never calls `sign_cooperative_close`**, so the 2-of-2 *covenant witness* is never set in `extra_data` → evaluator finds no covenant satisfaction → MPTX007. Also no counterparty-signature collection exists.
2. **Channel force-close (FEATURE GAP)** — `cmd_channel.rs:296-351` builds a commitment via `build_local_commitment` and signs only locally. The commitment spends the 2-of-2 funding output, which REQUIRES both parties' signatures. There is no field/flow to obtain & store the counterparty's signature on our commitment, so unilateral broadcast can never satisfy the funding covenant. `channel pay` (line 208) is local-store-only — no commitment/revocation exchange, no transport.
3. **Bridge HTLC claim/refund (NOT A WITNESS BUG)** — `cmd_bridge.rs` builds `branch(left)+preimage(P)`; evaluator (`eval.rs:70-106`) needs only `hash_with_domain(HASHLOCK_DOMAIN,P)==expected_hash` AND `height>=lock_height`. `parse_witness` stores raw preimage (correct). The stress-test MPTX007 was a **usage artifact** (claim before `lock_height` — swap sets `lock=height+3` — or wrong preimage), mirroring the proven single-sig channel case. Exception: `--multisig-threshold`-wrapped HTLC claim lacks covenant multisig sigs (edge case).

## Consensus-shape / deploy (INC-I-075 checklist)
- Q1 user-tx triggers path? YES (channel close, bridge claim are user txs). Q2 producer/attestation? NO. Q3 bit-identical to old behaviour? **YES — on-chain semantics unchanged; the evaluator already accepts these witnesses.** ⇒ **No activation height. No synchronized deploy.** Pure wallet/CLI. Standard build→cp→codesign→restart.

## Triage Verdict
━━━ TRIAGE VERDICT ━━━
Path: FAST (root cause confirmed; localized to CLI + channels witness/exchange helpers; reference impl = NFT PSBT handoff + existing commitment crypto)
Confidence: conf(0.9, code-read + passing covenant test)
Reasoning: No prior failed fix attempts; cause is missing CLI witness construction + missing commitment-sig exchange, not a mysterious/architectural fault. Multi-milestone due to feature breadth, not diagnostic uncertainty.
━━━━━━━━━━━━━━━━━━━━━━

## Transport decision (SSF) — LIVE P2P (revised at user request)
Re-planned to **live P2P**, matching every production payment-channel network (Lightning BOLT #8 persistent connection; Raiden; Perun/Connext). File handoff is reserved only for the one-shot cooperative close (PSBT-idiomatic) and even that is now done live (`shutdown`/`closing_signed`). Per-payment file passing was rejected — it defeats the instant-micropayment purpose channels exist for.

### Grounding (existing infra reused, NOT built)
- `crates/network/src/behaviour.rs::DoliBehaviour` already aggregates libp2p `request_response` behaviours: `status` (`/doli/status/1.0.0`), `sync` (`/doli/sync/1.0.0`), `txfetch` (`/doli/txfetch/1.0.0`), each with a Codec (`protocols/{status,sync,txfetch}.rs`). Adding `channel` (`/doli/channel/1.0.0`) is a one-struct-field + one-codec extension.
- `bins/node/src/node/event_loop.rs::on_sync_request` is the exact handler pattern to mirror for `on_channel_request`.
- `crates/channels/src/protocol.rs` `ChannelMessage` (Open/Accept/FundingCreated/FundingSigned/UpdateCommitment/RevokeAndAck/AddHtlc/Close…) ALREADY has serde derives → wire payload ready.

### Architecture decision (single choice, SSF)
Channel subsystem runs **inside `doli-node`** (it owns the libp2p swarm, is always-on, and has `monitor.rs` for revoked-broadcast detection — exactly the LN-daemon shape). The node loads a **dedicated channel signing key** at startup (new `channel.key` in the node data dir; channels disabled if absent — additive, opt-in). The CLI stays a thin RPC client and drives channels via NEW RPC methods. Node negotiates live with the peer over `/doli/channel/1.0.0`.
- **Tradeoff stated plainly:** the node custodies the channel signing key (standard for LN nodes; separate from `producer.seed.txt` and from the user's main wallet).
- **Rejected alt (no menu):** node-as-dumb-relay with signing kept in the CLI wallet — reintroduces CLI liveness coupling + per-message node↔wallet round-trips; strictly worse.

### Deploy / consensus-shape (INC-I-075 + INC-I-062)
No consensus rules change, no block-content change, no validation change (evaluator already accepts every witness). New libp2p protocol negotiates support gracefully — a peer without `/doli/channel/1.0.0` simply won't channel-peer (no fork risk). New RPC methods are additive. ⇒ **No activation height, no synchronized deploy. Normal rolling node upgrade.**

> ⚠️ This is now FEATURE-scale (a new P2P subsystem + node key + RPC surface), beyond a typical `doctor --fix`. Proceeding under INC-I-093 at user direction; `/omega-new-feature` would be the canonical home.

## Milestones (TDD — reproduction/contract test FIRST each)
- **M0 — Channel wire protocol + transport plumbing.** `crates/network/src/protocols/channel.rs` (`/doli/channel/1.0.0` + `ChannelCodec`, mirror `txfetch.rs`); wire into `DoliBehaviour`, `NetworkEvent::ChannelRequest/Response`, service. Test: codec round-trip + two in-process swarms exchange a `ChannelMessage`.
- **M1 — Node channel key + ChannelManager service + read RPC.** Node loads/derives `channel.key` (opt-in). Node-side `ChannelManager` (owns `ChannelStore` + peer sessions) + `on_channel_request` handler. RPC `channel_list`/`channel_info`. Test: node boots with key; RPC returns empty list; absent key ⇒ channels disabled cleanly.
- **M2 — Open flow (live).** `OpenChannel→AcceptChannel→FundingCreated(funder sig)→FundingSigned(acceptor sig)`; both store co-signed initial commitment; funding broadcast. RPC `channel_open`; CLI `channel open`→RPC. Test: two nodes open; both hold a co-signed initial commitment that PASSES the 2-of-2 evaluator (force-closeable immediately).
- **M3 — Update flow (live payments + revocation).** `UpdateCommitment(new sig)↔RevokeAndAck(reveal prior preimage)`; both store latest co-signed commitment + revoke prior. RPC `channel_pay`; CLI `channel pay`→RPC (now a real two-party update). Test: after N payments both hold co-signed commitment at latest balance; prior states revoked in `RevocationStore`.
- **M4 — Cooperative close (live).** `CloseChannel↔CloseAccepted` → both sigs → `sign_cooperative_close` → broadcast. *Subsumes the original confirmed bug, done correctly over the wire.* RPC `channel_close`; CLI `channel close`. Test: close tx PASSES evaluator (inc_i_092 ground-truth, driven live).
- **M5 — Force-close + penalty + delayed-claim.** `channel close --force` broadcasts stored co-signed latest commitment (2-of-2 satisfied). Wire `monitor.rs`: detect revoked-commitment broadcast → submit `build_penalty_tx`; delayed-claim after dispute window via `build_delayed_claim`. Test: force-close from latest succeeds; broadcasting a revoked state lets counterparty penalty-sweep.
- **M6 — Bridge proof + UX** (independent of channels). Consensus test: correct claim PASSES, claim-before-`lock_height` FAILS, refund-before-expiry FAILS. CLI explains these instead of raw MPTX007. No witness change.
- **M7 — Docs/specs + E2E.** specs/protocol.md (`/doli/channel/1.0.0` + flows), docs/cli.md, docs/rpc_reference.md (new RPC), CLAUDE.md map. Extend `scripts/test_defi_e2e.sh` to drive two local nodes: open→pay→cooperative-close + force-close→penalty.

## Blast radius
- `crates/network/`: `behaviour.rs`, `protocols/{mod,channel}.rs`, `messages.rs`/`service/` (new event variants). NEW protocol — additive.
- `crates/channels/`: `manager.rs` (real service), `commitment.rs`/`store.rs`/`types.rs` (store remote commitment sig + session state), `protocol.rs` (message handlers).
- `bins/node/`: `init.rs` (load channel key), `event_loop.rs` (`on_channel_request`), `startup.rs` (manager wiring).
- `crates/rpc/`: new `channel_*` methods.
- `bins/cli/`: `cmd_channel.rs` (drive via RPC), `cmd_bridge.rs` (M6 UX), `commands.rs`.
- Tests: `crates/network/tests`, `crates/channels/tests`, `bins/node/tests`, E2E script.
- **NO** `crates/core` consensus/validation edits. **NO** `NetworkParams`/activation-height/HardForkSchedule changes.
