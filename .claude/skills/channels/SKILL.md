# channels — DOLI Payment Channels
<!-- @INDEX
ENTRY-POINTS    14-34
STRUCTS         35-101
OPERATIONS      102-116
FUNCTIONS       117-221
STATE-MACHINE   222-255
DATA-FLOWS      256-311
DEPENDENCIES    312-333
CONSTRAINTS     334-390
PATTERNS        391-423
@/INDEX -->

## ENTRY-POINTS

Public re-exports from `crates/channels/src/lib.rs` (18 `pub mod` declarations, lib.rs:23-41):
- `ChannelRecord` — channel.rs:14
- `ChannelConfig` — config.rs:8
- `ChannelError`, `Result` — error.rs:6,65
- `ChannelManager` — manager.rs:22
- `ChannelBalance`, `ChannelId`, `ChannelState` — types.rs:9,39,105

### Production entry point (the ONLY currently-wired consumer)
`bins/cli/src/cmd_channel.rs::cmd_channel(wallet_path, rpc_endpoint, network, command)` — cmd_channel.rs:42.
Dispatches `ChannelCommands`: `Open`, `Pay`, `Close{force}`, `CloseFinish`, `List`, `Info` (defined in `bins/cli/src/commands.rs`, outside this domain).
The CLI wires only: `funding.rs` (funding tx), `close.rs` (cooperative close + offer/finish), `store.rs`, `types.rs`, `commitment::derive_channel_seed`, `channel.rs::try_activate`.

### Library-only entry points (implemented + unit-tested, NOT invoked by any binary)
`ChannelManager::new(config)` — manager.rs:31 / `ChannelManager::run()` — manager.rs:45: tick-based monitoring loop. No binary ever constructs a `ChannelManager`.
`ChainMonitor::check_channel()` — monitor.rs:62: funding-confirm / close-detect / HTLC-expiry polling. Unreachable in production — see CONSTRAINTS.
`CommitmentPair::build_local_commitment()` — commitment.rs:101: builds a signed per-update commitment tx. The CLI's `pay` command never calls this (see DATA-FLOWS drift note).
`ChannelGraph::find_route()` — router.rs:104, `Invoice`/`Payment` — invoice.rs, payment.rs: Phase 2 multi-hop routing, no caller anywhere in the workspace.
`WatchtowerSession` — watchtower.rs:28: Phase 3 stub, no caller.

## STRUCTS

### types.rs
- `ChannelId([u8; 32])` — types.rs:9; derived from `H(funding_tx_hash || output_index_le)`
- `ChannelBalance { local: Amount, remote: Amount }` — types.rs:105
- `InFlightHtlc { htlc_id, payment_hash, amount, expiry_height, direction, state, preimage }` — types.rs:168
- `FundingOutpoint { tx_hash: [u8;32], output_index: u32 }` — types.rs:187
- `CommitmentNumber = u64` — types.rs:101
- `PaymentDirection` — types.rs:146 (`Outgoing` | `Incoming`)
- `HtlcState` — types.rs:155 (`Pending` | `Fulfilled` | `Expired` | `Resolved`)

### channel.rs
- `ChannelRecord` — channel.rs:14; the persistent state container:
  - `channel_id`, `state`, `local_pubkey_hash`, `remote_pubkey_hash`
  - `funding_outpoint`, `capacity`, `balance: ChannelBalance`
  - `commitment_number`, `channel_seed: [u8;32]`
  - `revocation_store: RevocationStore`, `dispute_window: u64`
  - `htlcs: Vec<InFlightHtlc>`, `funding_confirmations: u32`
  - `created_at`, `updated_at: DateTime<Utc>`
  - `close_tx_hash`, `penalty_tx_hash: Option<String>`

### commitment.rs
- `CommitmentPair { number, balance, revocation_preimage, revocation_hash, remote_revocation_hash, htlcs }` — commitment.rs:23
- `RevocationStore { entries: Vec<(CommitmentNumber, [u8;32])> }` — commitment.rs:190; linear store, 40 bytes/entry

### config.rs
- `ChannelConfig { dispute_window, min_channel_capacity, reserve_percent, max_htlcs, htlc_minimum, fee_rate, max_htlc_expiry_delta, funding_confirmations, rpc_url, poll_interval_secs, store_path }` — config.rs:8
  - Mainnet defaults: dispute_window=144 blocks, funding_confirmations=3, max_htlcs=30
  - Testnet defaults: dispute_window=20 blocks, funding_confirmations=1

### htlc.rs
- `HtlcManager { next_id: u64, htlcs: Vec<InFlightHtlc> }` — htlc.rs:14

### monitor.rs
- `ChainMonitor { rpc: RpcClient, last_height: BlockHeight }` — monitor.rs:48
- `MonitorEvent` — monitor.rs:19: `FundingConfirmed`, `FundingSpent`, `RevokedCommitment`, `DisputeWindowExpired`, `HtlcExpired`

### router.rs
- `ChannelGraph { edges: HashMap<NodeId, Vec<ChannelEdge>> }` — router.rs:58
- `ChannelEdge { channel_id, source, target, capacity, fee_rate_ppm, base_fee }` — router.rs:16
- `Route { hops: Vec<RouteHop>, total_fee, total_amount }` — router.rs:39
- `RouteHop { channel_id, node_id, amount_to_forward, fee, expiry_delta }` — router.rs:47

### invoice.rs
- `Invoice { payment_hash, amount, description, payee_pubkey_hash, expiry_timestamp, created_at }` — invoice.rs:11

### payment.rs
- `Payment { payment_hash, preimage, total_amount, destination_amount, total_fees, status, hop_count, created_at }` — payment.rs:26
- `PaymentStatus` — payment.rs:13: `Pending` | `InFlight` | `Succeeded` | `Failed(String)`

### watchtower.rs
- `PenaltyBlob { tx_hint: [u8;16], encrypted_data: Vec<u8> }` — watchtower.rs:16
- `WatchtowerSession { channel_id, endpoint, session_token, blobs_uploaded }` — watchtower.rs:28

### rpc.rs
- `RpcClient { url: String, client: reqwest::Client }` — rpc.rs:15
- `RpcUtxo`, `TxStatus`, `BlockInfo` — rpc.rs:42,59,74
- `TxStatus::confirmed()` — rpc.rs:67; true if `confirmations > 0`

### store.rs
- `ChannelStore { path: PathBuf, channels: Vec<ChannelRecord> }` — store.rs:13

### close.rs (NEW: cooperative-close file handoff, 2026)
- `CooperativeCloseOffer { version, channel_id, partial_tx, initiator_pubkey, initiator_signature, initiator_pubkey_hash, counterparty_pubkey_hash }` — close.rs:111
  - All binary fields hex-encoded for a human-inspectable JSON file (mirrors the NFT PSBT sell-sign `--from` handoff).
- `COOPERATIVE_CLOSE_OFFER_VERSION: u8 = 1` — close.rs:98

## OPERATIONS

User-facing operations, all via `doli channel <subcommand>` (bins/cli/src/cmd_channel.rs; wallet-scoped, `--network` selects mainnet/testnet `ChannelConfig`):

| Task | Steps | Commands/Functions | Inputs | Success |
|------|-------|--------------------|--------|---------|
| Open a channel | 1. Resolve peer address 2. Select spendable Normal UTXOs covering capacity+fee 3. `build_funding_tx_with_change()` 4. Sign all inputs 5. Broadcast 6. Create local `ChannelRecord` at `FundingBroadcast` | `doli channel open <peer> <capacity> [--fee]`; `funding::build_funding_tx_with_change()` funding.rs:56; `ChannelId::from_funding_outpoint()` types.rs:14 | wallet with funded UTXOs, peer address, capacity >= `min_channel_capacity` | funding tx broadcast; channel record saved to `channels.json` in `FundingBroadcast` state |
| Send an off-chain payment | 1. Load channel by ID prefix 2. If `FundingBroadcast`, query RPC confirmations and lazily `try_activate()` (INC-I-097) 3. Require `Active` 4. `ChannelBalance::pay_local_to_remote()` 5. `update_balance()` + `advance_commitment()` 6. Save store | `doli channel pay <channel> <amount>`; `ChannelBalance::pay_local_to_remote()` types.rs:122; `ChannelRecord::try_activate()` channel.rs:78 | active channel, amount <= local balance | local balance ledger updated, commitment_number incremented — **no on-chain tx, no message to counterparty** (see CONSTRAINTS) |
| Close cooperatively (initiator) | 1. Look up channel, must not be terminal 2. `build_cooperative_close_offer()` — builds close tx + signs local half 3. Write `CooperativeCloseOffer` to JSON file (mode 0600 on Unix) 4. Transition local record to `CooperativeClosing` 5. Hand file to counterparty out-of-band | `doli channel close <channel> [--fee] [-o file]`; `close::build_cooperative_close_offer()` close.rs:150 | non-terminal channel, fee <= local balance | `close-<id>.json` written; counterparty instructed to run `close-finish` |
| Close cooperatively (counterparty) | 1. Read offer file 2. `finalize_cooperative_close_offer()` — verifies initiator's signature over the exact tx, then co-signs 2-of-2 witness 3. Broadcast 4. Mark local record `Closed` if present | `doli channel close-finish <file>`; `close::finalize_cooperative_close_offer()` close.rs:190 | valid offer JSON, wallet is the channel's counterparty | close tx broadcast; funds settled on-chain; local record (if any) marked `Closed` |
| List channels | 1. Open store 2. Filter active or all | `doli channel list [--all]` | none | table of channel_id/state/local/remote/capacity |
| View channel details | 1. Look up by ID prefix 2. Print full record | `doli channel info <channel>` | channel exists (any state) | full record dump incl. close/penalty tx hashes if present |
| Force-close / dispute / penalty | **NOT SUPPORTED** by the shipped CLI — `--force` always errors | `doli channel close <channel> --force` → `anyhow::bail!` cmd_channel.rs:338 | n/a | explicit rejection message citing INC-I-093; directs user to cooperative close/close-finish |
| Run a channel monitoring daemon | Library API exists (`ChannelManager::run()`) but is never instantiated by any shipped binary | `ChannelManager::new(config)` manager.rs:31 | n/a (unreachable in production) | n/a — library-only |

## FUNCTIONS

### conditions.rs — L1 condition builders
- `funding_condition(alice, bob) -> Condition` — conditions.rs:19; 2-of-2 multisig, keys sorted lexicographically
- `funding_output(capacity, alice, bob) -> Result<Output>` — conditions.rs:26; `OutputType::Multisig`
- `to_local_condition(owner, counterparty, revocation_hash, dispute_height) -> Condition` — conditions.rs:43
  - Left/false: `And(Sig(counterparty), Hashlock(revocation_hash))` — penalty path
  - Right/true: `And(Sig(owner), Timelock(dispute_height))` — delayed claim
- `to_local_output(amount, owner, cp, rev_hash, dispute_height) -> Result<Output>` — conditions.rs:64
- `to_remote_output(amount, pubkey_hash) -> Output` — conditions.rs:82; plain `Normal` output, no condition
- `htlc_offered_condition(local, remote, payment_hash, expiry) -> Condition` — conditions.rs:90
  - Left: `And(Sig(remote), Hashlock(payment_hash))` — remote claims with preimage
  - Right: `And(Sig(local), TimelockExpiry(expiry))` — local refunds on timeout
- `htlc_received_condition(local, remote, payment_hash, expiry) -> Condition` — conditions.rs:131
  - Left: `And(Sig(local), Hashlock(payment_hash))` — local claims with preimage
  - Right: `And(Sig(remote), TimelockExpiry(expiry))` — remote refunds on timeout
- `htlc_offered_output(...)`, `htlc_received_output(...)` — conditions.rs:111,152; `OutputType::HTLC`
- `verify_encoding_size(condition) -> Result<usize>` — conditions.rs:169; must fit in `MAX_EXTRA_DATA_SIZE` (4096 bytes)

### commitment.rs — revocation and commitment construction
- `generate_revocation_preimage(seed, number) -> [u8;32]` — commitment.rs:43; `H(REVOCATION_DOMAIN || seed || number_le)`
- `revocation_hash(preimage) -> Hash` — commitment.rs:53; uses `HASHLOCK_DOMAIN` for L1 hashlock compatibility
- `derive_channel_seed(keypair, channel_id) -> [u8;32]` — commitment.rs:63; MUST use private key (AUDIT-CHAN-001). **This is the one commitment.rs function the CLI actually calls** (from `open`, to seed the local `ChannelRecord`).
- `CommitmentPair::new(number, balance, seed)` — commitment.rs:74
- `CommitmentPair::build_local_commitment(funding_hash, idx, local_pk, remote_pk, dispute_height, capacity, fee) -> Result<Transaction>` — commitment.rs:101; enforces `balance.total() + htlc_total == capacity`. Library-only (see ENTRY-POINTS).
- `CommitmentPair::verify_revocation(preimage, expected_hash) -> bool` — commitment.rs:180
- `RevocationStore::add(number, preimage)` — commitment.rs:201; deduplicates
- `RevocationStore::find_by_hash(hash) -> Option<&[u8;32]>` — commitment.rs:218; for breach detection
- `build_delayed_claim_witness(signing_hash, keypair) -> Witness` — commitment.rs:236; `or_branches=[true]`
- `build_penalty_witness(signing_hash, keypair, preimage) -> Witness` — commitment.rs:249; `or_branches=[false]`
- `build_htlc_claim_witness(signing_hash, keypair, preimage) -> Witness` — commitment.rs:266; `or_branches=[false]`
- `build_htlc_timeout_witness(signing_hash, keypair) -> Witness` — commitment.rs:283; `or_branches=[true]`

### funding.rs — funding transaction construction
- `build_funding_tx(inputs, alice_pk, bob_pk, capacity, change, dual_funded) -> Result<Transaction>` — funding.rs:19
  - Single-funded: `SighashType::All`; dual-funded: `SighashType::AnyoneCanPay` per input
- `build_funding_tx_with_change(inputs_with_amounts, alice, bob, capacity, fee, change_pk) -> Result<Transaction>` — funding.rs:56; auto-calculates change to prevent supply leak. **Called directly by `doli channel open`.**
- `sign_funding_input(tx, input_index, keypair) -> Result<()>` — funding.rs:97

### close.rs — close transaction builders (all called by the CLI except `build_force_close`/`build_penalty_tx`/`build_delayed_claim`)
- `build_cooperative_close(funding_hash, idx, local_pk, remote_pk, balance, capacity, fee) -> Result<Transaction>` — close.rs:20; requires `balance.total() == capacity`; outputs are plain `Normal`
- `sign_cooperative_close(tx, local_keypair, remote_pubkey, remote_sig, local_pk_hash, remote_pk_hash) -> Result<()>` — close.rs:66; sorts sigs by pubkey hash for deterministic multisig
- `build_cooperative_close_offer(channel_id, funding_hash, idx, initiator_pkh, cp_pkh, balance, capacity, fee, initiator_kp) -> Result<CooperativeCloseOffer>` — close.rs:150; builds the tx, signs the initiator's half, returns a portable offer. **Called by `doli channel close`.**
- `finalize_cooperative_close_offer(offer, finisher_keypair) -> Result<Transaction>` — close.rs:190; verifies the initiator's signature over the exact tx (rejects tampering) then co-signs. **Called by `doli channel close-finish`.**
- `decode_pubkey(hex)`, `decode_signature(hex)` — close.rs:128,137; internal helpers for offer deserialization
- `build_force_close(commitment, funding_hash, idx, local_pk, remote_pk, dispute_height, capacity, fee) -> Result<Transaction>` — close.rs:232; delegates to `CommitmentPair::build_local_commitment`. Library-only.
- `build_penalty_tx(revoked_hash, to_local_idx, amount, claim_pk, keypair, preimage, fee) -> Result<Transaction>` — close.rs:258; sets covenant witness + signs input. Library-only.
- `build_delayed_claim(commitment_hash, idx, amount, claim_pk, keypair, fee) -> Result<Transaction>` — close.rs:292. Library-only.

### htlc.rs — HtlcManager methods (library-only; no binary constructs an `HtlcManager`)
- `add_outgoing(payment_hash, amount, expiry_height) -> u64` — htlc.rs:27; returns htlc_id
- `add_incoming(htlc_id, payment_hash, amount, expiry_height)` — htlc.rs:48
- `fulfill(htlc_id, preimage) -> Result<Amount>` — htlc.rs:70; verifies `H(preimage) == payment_hash`
- `expire(htlc_id, current_height) -> Result<Amount>` — htlc.rs:89; fails if `current_height < expiry_height`
- `resolve(htlc_id) -> Result<()>` — htlc.rs:108; only from `Fulfilled` or `Expired`
- `pending()`, `total_outgoing_pending()`, `total_incoming_pending()`, `all()` — htlc.rs:127-155
- `gc_resolved()` — htlc.rs:158; removes `Resolved` HTLCs

### router.rs
- `ChannelGraph::add_channel(edge)` — router.rs:87; adds bidirectional edges
- `ChannelGraph::find_route(source, destination, amount) -> Option<Route>` — router.rs:104; Dijkstra, fee as cost metric, filters edges with `capacity < amount`
- `ChannelEdge::fee_for_amount(amount) -> Amount` — router.rs:33; `base_fee + (amount * fee_rate_ppm) / 1_000_000`

### invoice.rs
- `Invoice::new(payment_hash, amount, description, payee_pk_hash, expiry_secs)` — invoice.rs:28
- `Invoice::encode() -> String` — invoice.rs:47; `"doli:pay:<base64>"`
- `Invoice::decode(s) -> Option<Self>` — invoice.rs:54; strips prefix, base64+JSON decode
- `Invoice::is_expired() -> bool` — invoice.rs:61

### payment.rs
- `Payment::from_route(payment_hash, route, destination_amount) -> Self` — payment.rs:47
- `Payment::succeed(preimage) -> bool` — payment.rs:65; validates `H(preimage) == payment_hash` (AUDIT-ROUTE-003)
- `Payment::fail(reason)` — payment.rs:76
- `Payment::is_terminal() -> bool` — payment.rs:80

### store.rs
- `ChannelStore::open(path) -> Result<Self>` — store.rs:20; creates empty if not exists
- `ChannelStore::save() -> Result<()>` — store.rs:38; atomic write (tmp -> rename)
- `ChannelStore::active_channels()`, `all_channels()`, `find()`, `find_mut()` — store.rs:47-64
- `ChannelStore::find_by_funding(tx_hash, output_index)` — store.rs:67
- `ChannelStore::add(channel) -> bool` — store.rs:75; false if duplicate ID

### rpc.rs
- `RpcClient::ping()` — rpc.rs:119
- `RpcClient::get_height() -> Result<BlockHeight>` — rpc.rs:124; calls `getChainInfo`
- `RpcClient::get_utxos(address) -> Result<Vec<RpcUtxo>>` — rpc.rs:130; calls `getUtxos`
- `RpcClient::submit_transaction(tx_hex) -> Result<String>` — rpc.rs:136; calls `sendTransaction`
- `RpcClient::get_transaction_status(tx_hash) -> Result<TxStatus>` — rpc.rs:142; calls `getTransaction`. **This is the call the CLI uses to lazily check funding confirmations for `try_activate` (via a separate `RpcClient` instance constructed inline in cmd_channel.rs).**
- `RpcClient::get_block(height) -> Result<BlockInfo>` — rpc.rs:148; calls `getBlockByHeight`
- `RpcClient::broadcast_transaction(tx) -> Result<String>` — rpc.rs:154; serializes via `tx.serialize()` -> hex

### channel.rs — ChannelRecord methods
- `transition(new_state) -> Result<()>` — channel.rs:59; enforces `state_machine::validate_transition` (AUDIT-CHAN-002)
- `try_activate(confirmations, required) -> bool` — channel.rs:78 (**INC-I-097**); lazily flips `FundingBroadcast -> Active` once `confirmations >= required.max(1)`. Pure/I/O-free — the CLI supplies `confirmations` from its own RPC query. No monitor loop calls this; it only runs when the user next invokes `pay`.
- `advance_commitment() -> CommitmentNumber` — channel.rs:93
- `update_balance(new_balance)` — channel.rs:100
- `store_revocation(number, preimage)` — channel.rs:106

### monitor.rs (library-only)
- `ChainMonitor::check_channel(channel) -> Result<Vec<MonitorEvent>>` — monitor.rs:62
  - `FundingBroadcast`: checks funding confirmation via `getTransaction`
  - `ForceClosing`: checks if `current_height >= close_height + dispute_window`
  - `Active`: checks HTLC expiry heights
- `ChainMonitor::update_height()` — monitor.rs:125

## STATE-MACHINE

Defined in `state_machine::validate_transition()` — state_machine.rs:10.
All transitions enforced via `ChannelRecord::transition()` — channel.rs:59. `try_activate()` (channel.rs:78) is a thin wrapper that also goes through `transition()`.

```
Opening
  +--> FundingSigned        (AcceptChannel received, funding tx built -- library protocol; CLI skips straight to FundingBroadcast)
  |     +--> FundingBroadcast (funding tx submitted)
  |           +--> Active   (funding confirmed >= funding_confirmations; CLI drives this lazily via try_activate())
  +--> Closed               (abort -- any early state can abort directly to Closed)
FundingSigned --> Closed    (abort)
FundingBroadcast --> Closed (abort)

Active
  +--> CooperativeClosing   (mutual close initiated -- CLI's `close` command)
  |     +--> Closed         (CLI's `close-finish` command)
  +--> ForceClosing         (unilateral close broadcast by us -- LIBRARY ONLY, no CLI path)
  |     +--> AwaitingClaim  (dispute window expired)
  |     |     +--> Closed
  |     +--> Closed         (direct if no timelock needed)
  +--> CounterpartyClosing  (funding spent by counterparty detected -- LIBRARY ONLY)
  |     +--> PenaltyInFlight (revoked commitment detected)
  |     |     +--> Closed
  |     +--> AwaitingClaim  (legitimate unilateral close)
  |     |     +--> Closed
  |     +--> Closed
  +--> PenaltyInFlight      (revoked commitment detected directly from Active -- LIBRARY ONLY)
        +--> Closed
```

Terminal state: `Closed` only. `is_terminal()` returns true only for `Closed`.
`is_closing()` returns true for: `CooperativeClosing | ForceClosing | CounterpartyClosing | AwaitingClaim | PenaltyInFlight`

## DATA-FLOWS

### Channel Open (SHIPPED — `doli channel open`)
1. CLI resolves peer address, selects spendable Normal UTXOs — cmd_channel.rs:110
2. `build_funding_tx_with_change()` — funding.rs:56 (auto change, prevents supply leak)
3. CLI signs every input directly (not via `sign_funding_input()`) — cmd_channel.rs:182
4. Broadcast via `RpcClient::send_transaction` (CLI's own rpc_client, not `channels::rpc`)
5. `ChannelId::from_funding_outpoint()` — types.rs:14; `derive_channel_seed()` — commitment.rs:63
6. `ChannelRecord` constructed directly at `FundingBroadcast`, `balance = (capacity, 0)`, saved via `ChannelStore::add()` + `save()`

### Off-Chain Payment (SHIPPED — `doli channel pay`, SIMPLIFIED vs. the LN-Penalty design)
1. Load channel by ID prefix from `ChannelStore`
2. If `FundingBroadcast`: query `get_transaction_status()`, call `try_activate(confs, required)` — channel.rs:78 (**INC-I-097** lazy activation, no background monitor)
3. Require `state.is_active()`
4. `ChannelBalance::pay_local_to_remote(amount)` — types.rs:122
5. `ChannelRecord::update_balance()` + `advance_commitment()` — channel.rs:100,93
6. **DRIFT NOTE**: no signed commitment tx is built, no protocol message (`UpdateCommitment`/`RevokeAndAck`) is exchanged with the counterparty, and no revocation preimage is stored. The full commitment/HTLC/revocation machinery below exists in the crate but this shipped flow bypasses it entirely — it is a local balance ledger only, trusting the eventual cooperative-close co-signature as the sole settlement guarantee.

### Cooperative Close via Offer/Finish files (SHIPPED — `doli channel close` + `close-finish`)
1. Initiator: `build_cooperative_close_offer()` — close.rs:150; builds tx, signs local half over `signing_message_for_input(0)`
2. Initiator writes `CooperativeCloseOffer` JSON to a file (mode 0600 on Unix — cmd_channel.rs:19); local record -> `CooperativeClosing`
3. File handed to counterparty out-of-band (mirrors the NFT PSBT sell-sign `--from` pattern)
4. Counterparty: `finalize_cooperative_close_offer()` — close.rs:190; verifies initiator's signature over the exact tx (anti-tamper), co-signs via `sign_cooperative_close()` — close.rs:66 (sorts sigs by pubkey hash)
5. Counterparty broadcasts; on `MPTX007` error surfaces a targeted hint (wrong wallet finalizing) — cmd_channel.rs:458
6. Both sides mark their local record `Closed` if present

### Full Commitment/HTLC Update Protocol (LIBRARY-ONLY — implemented, unit-tested, no caller)
1. Sender: `HtlcManager::add_outgoing()` — htlc.rs:27
2. Would send `AddHtlc` message — protocol.rs:109 (protocol.rs messages are defined but nothing serializes/transports them — Phase 2 relay was never built)
3. Both parties would build new `CommitmentPair`, exchange sigs via `UpdateCommitment` + `RevokeAndAck`
4. Receiver: `HtlcManager::fulfill()` — htlc.rs:70
5. Would send `FulfillHtlc` message — protocol.rs:124
6. HTLC resolved: `HtlcManager::resolve()` -> `gc_resolved()`
7. `ChannelRecord::update_balance()` + `advance_commitment()`

### Force Close / Penalty Flow (LIBRARY-ONLY — CLI explicitly rejects `--force`, see CONSTRAINTS/INC-I-093)
1. `build_force_close()` — close.rs:232; broadcasts latest `CommitmentPair`
2. `ChainMonitor` would wait `dispute_window` blocks — monitor.rs:89
3. `build_delayed_claim()` — close.rs:292; spends `to_local` via delayed path (or_branches=[true])
4. Breach path: `RevocationStore::find_by_hash()` — commitment.rs:218 matches revocation hash from a spending tx; `build_penalty_tx()` — close.rs:258 (or_branches=[false])
5. None of this is reachable from any shipped binary today.

### Multi-Hop Payment (LIBRARY-ONLY — Phase 2, no caller)
1. Build `ChannelGraph`, call `find_route(source, dest, amount)` — router.rs:104
2. Create `Invoice` — invoice.rs:28; encode as `doli:pay:<base64>`
3. `Payment::from_route()` — payment.rs:47
4. Add HTLCs along route (onion-peel style — not implemented)
5. On success: `Payment::succeed(preimage)` — validates hash before marking

### Manager Tick Loop (LIBRARY-ONLY — no binary ever constructs a `ChannelManager`)
`ChannelManager::tick()` — manager.rs:66:
1. `monitor.update_height()` — fetches current chain height
2. For each active channel: `monitor.check_channel()` -> collect `MonitorEvent`s
3. `handle_event()` dispatches: FundingConfirmed -> transition Active; RevokedCommitment -> PenaltyInFlight; DisputeWindowExpired -> AwaitingClaim; FundingSpent -> CounterpartyClosing; HtlcExpired -> log warning
4. `store.save()` — atomic JSON write

## DEPENDENCIES

External crates (from usage patterns):
- `doli_core` — `Amount`, `BlockHeight`, `ConditionError`, `Transaction`, `Input`, `Output`, `OutputType`, `SighashType`, `Condition`, `Witness`, `WitnessSignature`, `HASHLOCK_DOMAIN`, `validation::validate_transaction_with_utxos` (used only by the ground-truth test, not by production code)
- `crypto` — `Hash`, `KeyPair`, `PublicKey`, `Signature`, `Hasher`, `hash_with_domain`, `sign_hash`, `verify_hash`, `ADDRESS_DOMAIN`
- `serde` + `serde_json` — all structs implement `Serialize`/`Deserialize`
- `reqwest` — async HTTP for `RpcClient`
- `tokio` — async runtime (`tokio::time::sleep` in manager loop)
- `tracing` — `debug!`, `info!`, `warn!`, `error!` throughout
- `chrono` — `DateTime<Utc>` on `ChannelRecord`, timestamps on `Invoice`/`Payment`
- `hex` — encode/decode tx hashes and offer fields
- `thiserror` — `ChannelError` derivation
- `rand` — declared in Cargo.toml dependencies but no direct `rand::` usage found in any of the 18 src modules read — likely unused/transitive; not confirmed (glob/grep unavailable this session, see note below)
- `tempfile` — used in `store.rs` tests only

Used By:

| Used By | Location | What For |
|---------|----------|----------|
| `doli-cli` (`bins/cli`) | `bins/cli/src/cmd_channel.rs`, `bins/cli/Cargo.toml:20` | The only production consumer. Wires `funding.rs`, `close.rs` (incl. offer/finish), `store.rs`, `types.rs`, `commitment::derive_channel_seed`, `channel.rs::try_activate`. Command enum `ChannelCommands` lives in `bins/cli/src/commands.rs` (outside this domain). |
| (none found) | — | `bins/node`, `crates/rpc`, `crates/mempool` do not depend on `channels` per `CLAUDE.md`'s code map — channel txs are ordinary L1 transactions once broadcast, validated by the same consensus path as any other Multisig/HTLC-conditioned output. Not exhaustively verified: Glob/Grep tools failed in this session (`posix_spawn 'rg'` — ripgrep missing from PATH), so cross-crate usage was checked only via known `Cargo.toml` files, not a full-workspace search. |

## CONSTRAINTS

### Shipped product is cooperative-only (MAJOR — read before touching `pay` or `close`)
The CLI (`bins/cli/src/cmd_channel.rs`) explicitly rejects `--force` with a message citing **INC-I-093**: "Unilateral force-close is not supported in this build... Trustless force-close (pre-signed commitments, penalty/watchtower) is a roadmap item gated on a concrete use case + economic review." — cmd_channel.rs:338.
Consequently `pay` never builds a signed commitment tx or exchanges revocation preimages (see DATA-FLOWS drift note) — it is a local balance ledger, settled only at cooperative close time. Do not assume the LN-Penalty security model (safety against a broadcast-revoked-state attack) is live in production; it exists in the crate as unused library code, exercised only by unit tests.

### Supply Invariant
`balance.total() + htlc_total == capacity` enforced in:
- `CommitmentPair::build_local_commitment()` — commitment.rs:115; returns `CapacityMismatch` on violation
- `build_cooperative_close()` — close.rs:31; returns `CapacityMismatch` if `balance.total() != capacity`

### Reserve Invariant
`ChannelConfig::reserve_percent` (default 1%); each side must maintain >= reserve.
Reserve calculated via `reserve_for_capacity(capacity)` — config.rs:81.
Not auto-enforced anywhere (not in `HtlcManager`, not in the CLI's `pay` path) — must be checked by caller.

### State Machine Enforcement
All state transitions MUST go through `ChannelRecord::transition()` — channel.rs:59, including the `try_activate()` fast path (channel.rs:78).
Direct `self.state = x` bypasses `validate_transition` — never do this (AUDIT-CHAN-002).

### Lazy Activation Floor (INC-I-097)
`try_activate(confirmations, required)` — channel.rs:78 — requires `confirmations >= required.max(1)` even if `required == 0` is misconfigured, to avoid activating an unconfirmed channel. No-op unless `state == FundingBroadcast`. Only invoked from the CLI's `pay` path (cmd_channel.rs:272) — a channel that is never paid into will sit at `FundingBroadcast` forever regardless of actual confirmations.

### Self-Channel Rejection (P1-007)
`ensure_distinct_channel_parties(local, remote)` — cmd_channel.rs:32 — rejects opening a channel to your own wallet address before any UTXO is selected. Enforced only in the CLI, not in `funding.rs`/`conditions.rs` (a degenerate same-key 2-of-2 is technically constructible at the library level — see `conditions.rs` test `funding_condition_same_keys`).

### Cooperative-Close Offer Anti-Tamper
`finalize_cooperative_close_offer()` — close.rs:190 — verifies the initiator's signature over `signing_message_for_input(0)` of the EXACT deserialized tx before co-signing. Any post-signing modification to `partial_tx` invalidates the initiator's signature and the finalize call fails with `InvalidSignature`.

### Revocation Seed Security (AUDIT-CHAN-001)
`derive_channel_seed()` — commitment.rs:63 — MUST use private key bytes.
Using only public data would allow anyone to compute all revocation preimages, breaking LN-Penalty. (Currently moot for `pay` since no revocations are ever generated in the shipped flow, but the seed IS derived and stored on every `open` for forward compatibility.)

### Preimage Hash Domain
HTLC payment hashes use `HASHLOCK_DOMAIN`. Consistent with L1 condition evaluation.
Both `HtlcManager::fulfill()` — htlc.rs:78 and `Payment::succeed()` — payment.rs:66 use `hash_with_domain(HASHLOCK_DOMAIN, preimage)`.

### Revocation Hash Domain
Revocation hashes also use `HASHLOCK_DOMAIN` — commitment.rs:54.
This ensures L1 `Hashlock(revocation_hash)` evaluates correctly with witness preimage.

### Condition Encoding Size
All conditions must encode within `MAX_EXTRA_DATA_SIZE` (4096 bytes).
Typical sizes: funding ~68 bytes, to_local ~112 bytes, HTLC ~112 bytes.
Check with `verify_encoding_size()` — conditions.rs:169.

### Multisig Key Ordering
Funding output keys sorted lexicographically — conditions.rs:21.
Cooperative close sigs sorted by pubkey hash — close.rs:82; same ordering used in `sign_cooperative_close` for both the direct-signature path and the offer/finish path.
Both must be deterministic -- order affects script hash and on-chain validation.

### Atomic Store Writes
`ChannelStore::save()` — store.rs:38 — writes to `.tmp` then renames. Never write the file directly.

### Ground-Truthed Consensus Compatibility (INC-I-092 RC-C)
`crates/channels/tests/inc_i_092_close_covenant.rs` proves, against the REAL consensus evaluator (`doli_core::validation::validate_transaction_with_utxos`), that a both-signed cooperative close satisfies the 2-of-2 funding covenant and a single-signed one does not. The MPTX007 stress-test failures were single-party USAGE artifacts, not a covenant/witness encoding bug. Do not re-derive this; see `CLAUDE.md`'s "If You Touch" section.

## PATTERNS

### Or-branch selection in witnesses
Left branch (false) = immediate/penalty path; Right branch (true) = timelocked/delayed path.
Consistent across all channel conditions:
- `to_local`: false=penalty (counterparty + revocation preimage), true=delayed claim (owner after timelock)
- HTLC offered/received: false=claim with preimage, true=timeout refund

### Commitment number monotonicity
`ChannelRecord::advance_commitment()` — channel.rs:93. Never decrement or reset.
Each state update increments; revealing old preimage lets counterparty sweep via penalty (moot today since no revocations are ever created — see CONSTRAINTS).

### RevocationStore growth
Grows by one entry per channel update. No pruning. Each entry is 40 bytes (8B number + 32B preimage). Currently always empty in shipped usage.

### Dual-funded channels via AnyoneCanPay
`build_funding_tx(..., dual_funded=true)` sets `SighashType::AnyoneCanPay` on all inputs.
Each party signs their own inputs independently; the other adds theirs later. Library-only — the CLI's `open` always builds single-funded via `build_funding_tx_with_change()`.

### File-based one-shot 2-party handoff (cooperative close)
`CooperativeCloseOffer` (close.rs:111) + `build_cooperative_close_offer()`/`finalize_cooperative_close_offer()` (close.rs:150,190) mirror the NFT PSBT sell-sign `--from` pattern: one party builds+signs+serializes to a hex/JSON file, the other verifies+co-signs+broadcasts. No network transport required — file passing is idiomatic for a one-shot 2-of-2 finalization. See `cmd_channel.rs` `write_offer_file()` (mode 0600).

### Lazy on-demand state advancement
`try_activate()` (channel.rs:78, INC-I-097) replaces a background monitor with an on-demand check performed by the next user action (`pay`) that needs the channel to be `Active`. Pure/I/O-free function; the CLI supplies the confirmation count from its own RPC call immediately before invoking it. Consider this pattern before wiring any new "background daemon" style feature — the codebase currently prefers on-demand lazy checks over always-running loops for the CLI-driven flows.

### RPC JSON-RPC 2.0 pattern
All RPC calls in `channels::rpc` go through `RpcClient::call<T>(method, params)` — rpc.rs:90.
Returns `ChannelError::Rpc` on error response or missing result. Note: the CLI's `open`/`pay`/`close-finish` commands use a SEPARATE `RpcClient` (`bins/cli/src/rpc_client.rs`, outside this domain) — `channels::rpc::RpcClient` is only used directly inside `cmd_channel.rs`'s `try_activate` confirmation check and by `ChainMonitor`/`ChannelManager` (library-only).

### Phase roadmap (crate docs, `lib.rs:17-21` — status confirmed against actual callers this pass)
- Phase 1 (implemented, PARTIALLY SHIPPED): open/close/cooperative-close-offer are wired into the CLI; pay is a simplified local-ledger version; force-close/penalty/monitor/manager are implemented + tested but have zero callers outside `#[cfg(test)]`.
- Phase 2 (structs present, no integration): multi-hop routing (router.rs), invoices (invoice.rs), payment.rs, full protocol.rs message exchange.
- Phase 3 (stub only): watchtower delegation (watchtower.rs), splice-in/out.
