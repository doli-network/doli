# channels — DOLI Payment Channels
<!-- @INDEX
ENTRY-POINTS: lines 18-32
STRUCTS: lines 34-103
FUNCTIONS: lines 105-195
STATE-MACHINE: lines 197-240
DATA-FLOWS: lines 242-300
DEPENDENCIES: lines 302-315
CONSTRAINTS: lines 317-345
PATTERNS: lines 347-365
-->

## ENTRY-POINTS

Public re-exports from `crates/channels/src/lib.rs`:
- `ChannelRecord` — channel.rs:14
- `ChannelConfig` — config.rs:8
- `ChannelError`, `Result` — error.rs:6,65
- `ChannelManager` — manager.rs:22
- `ChannelBalance`, `ChannelId`, `ChannelState` — types.rs:9,39,105

`ChannelManager::new(config)` — manager.rs:31 (entry for the tick-based monitoring loop)
`ChannelManager::run()` — manager.rs:45 (async loop, connects to node RPC, polls channels)
`ChannelManager::store()` / `store_mut()` — manager.rs:177,182 (external access to channel state)

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
- `RpcUtxo`, `TxStatus`, `BlockInfo` — rpc.rs:41,58,73
- `TxStatus::confirmed()` — rpc.rs:67; true if `confirmations > 0`

### store.rs
- `ChannelStore { path: PathBuf, channels: Vec<ChannelRecord> }` — store.rs:13

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
- `derive_channel_seed(keypair, channel_id) -> [u8;32]` — commitment.rs:63; MUST use private key (AUDIT-CHAN-001)
- `CommitmentPair::new(number, balance, seed)` — commitment.rs:74
- `CommitmentPair::build_local_commitment(funding_hash, idx, local_pk, remote_pk, dispute_height, capacity, fee) -> Result<Transaction>` — commitment.rs:101; enforces `balance.total() + htlc_total == capacity`
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
- `build_funding_tx_with_change(inputs_with_amounts, alice, bob, capacity, fee, change_pk) -> Result<Transaction>` — funding.rs:56; auto-calculates change to prevent supply leak
- `sign_funding_input(tx, input_index, keypair) -> Result<()>` — funding.rs:97

### close.rs — close transaction builders
- `build_cooperative_close(funding_hash, idx, local_pk, remote_pk, balance, capacity, fee) -> Result<Transaction>` — close.rs:19; requires `balance.total() == capacity`; outputs are plain `Normal`
- `sign_cooperative_close(tx, local_keypair, remote_pubkey, remote_sig, local_pk_hash, remote_pk_hash) -> Result<()>` — close.rs:65; sorts sigs by pubkey hash for deterministic multisig
- `build_force_close(commitment, funding_hash, idx, local_pk, remote_pk, dispute_height, capacity, fee) -> Result<Transaction>` — close.rs:102; delegates to `CommitmentPair::build_local_commitment`
- `build_penalty_tx(revoked_hash, to_local_idx, amount, claim_pk, keypair, preimage, fee) -> Result<Transaction>` — close.rs:128; sets covenant witness + signs input
- `build_delayed_claim(commitment_hash, idx, amount, claim_pk, keypair, fee) -> Result<Transaction>` — close.rs:162

### htlc.rs — HtlcManager methods
- `add_outgoing(payment_hash, amount, expiry_height) -> u64` — htlc.rs:27; returns htlc_id
- `add_incoming(htlc_id, payment_hash, amount, expiry_height)` — htlc.rs:47
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
- `Invoice::new(payment_hash, amount, description, payee_pk_hash, expiry_secs)` — invoice.rs:27
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
- `ChannelStore::active_channels()`, `all_channels()`, `find()`, `find_mut()` — store.rs:47-62
- `ChannelStore::find_by_funding(tx_hash, output_index)` — store.rs:67
- `ChannelStore::add(channel) -> bool` — store.rs:75; false if duplicate ID

### rpc.rs
- `RpcClient::ping()` — rpc.rs:119
- `RpcClient::get_height() -> Result<BlockHeight>` — rpc.rs:124; calls `getChainInfo`
- `RpcClient::get_utxos(address) -> Result<Vec<RpcUtxo>>` — rpc.rs:130; calls `getUtxos`
- `RpcClient::submit_transaction(tx_hex) -> Result<String>` — rpc.rs:136; calls `sendTransaction`
- `RpcClient::get_transaction_status(tx_hash) -> Result<TxStatus>` — rpc.rs:142; calls `getTransaction`
- `RpcClient::get_block(height) -> Result<BlockInfo>` — rpc.rs:148; calls `getBlockByHeight`
- `RpcClient::broadcast_transaction(tx) -> Result<String>` — rpc.rs:154; serializes via `tx.serialize()` -> hex

### channel.rs — ChannelRecord methods
- `transition(new_state) -> Result<()>` — channel.rs:59; enforces `state_machine::validate_transition` (AUDIT-CHAN-002)
- `advance_commitment() -> CommitmentNumber` — channel.rs:67
- `update_balance(new_balance)` — channel.rs:74
- `store_revocation(number, preimage)` — channel.rs:80

### monitor.rs
- `ChainMonitor::check_channel(channel) -> Result<Vec<MonitorEvent>>` — monitor.rs:62
  - `FundingBroadcast`: checks funding confirmation via `getTransaction`
  - `ForceClosing`: checks if `current_height >= close_height + dispute_window`
  - `Active`: checks HTLC expiry heights
- `ChainMonitor::update_height()` — monitor.rs:125

## STATE-MACHINE

Defined in `state_machine::validate_transition()` — state_machine.rs:10.
All transitions enforced via `ChannelRecord::transition()` — channel.rs:59.

```
Opening
  +--> FundingSigned        (AcceptChannel received, funding tx built)
  |     +--> FundingBroadcast (funding tx submitted)
  |           +--> Active   (funding confirmed >= funding_confirmations)
  +--> Closed               (abort -- any early state can abort directly to Closed)
FundingSigned --> Closed    (abort)
FundingBroadcast --> Closed (abort)

Active
  +--> CooperativeClosing   (mutual close initiated)
  |     +--> Closed
  +--> ForceClosing         (unilateral close broadcast by us)
  |     +--> AwaitingClaim  (dispute window expired)
  |     |     +--> Closed
  |     +--> Closed         (direct if no timelock needed)
  +--> CounterpartyClosing  (funding spent by counterparty detected)
  |     +--> PenaltyInFlight (revoked commitment detected)
  |     |     +--> Closed
  |     +--> AwaitingClaim  (legitimate unilateral close)
  |     |     +--> Closed
  |     +--> Closed
  +--> PenaltyInFlight      (revoked commitment detected directly from Active)
        +--> Closed
```

Terminal state: `Closed` only. `is_terminal()` returns true only for `Closed`.
`is_closing()` returns true for: `CooperativeClosing | ForceClosing | CounterpartyClosing | AwaitingClaim | PenaltyInFlight`

## DATA-FLOWS

### Channel Open (Single-Funded)
1. Initiator calls `build_funding_tx()` — funding.rs:19
2. Signs with `sign_funding_input()` — funding.rs:97
3. Sends `OpenChannel` protocol message — protocol.rs:29
4. Counterparty replies `AcceptChannel` — protocol.rs:46
5. Initiator sends `FundingCreated` — protocol.rs:59
6. Counterparty sends `FundingSigned` — protocol.rs:72
7. Record: `Opening -> FundingSigned -> FundingBroadcast`
8. `ChainMonitor` detects confirmation -> `FundingBroadcast -> Active`

### Off-Chain Payment (Update Protocol)
1. Sender: `HtlcManager::add_outgoing()` — htlc.rs:27
2. Sends `AddHtlc` message — protocol.rs:109
3. Both parties: build new `CommitmentPair`, exchange sigs via `UpdateCommitment` + `RevokeAndAck`
4. Receiver: receives preimage, calls `HtlcManager::fulfill()` — htlc.rs:70
5. Sends `FulfillHtlc` message — protocol.rs:123
6. HTLC resolved: `HtlcManager::resolve()` -> `gc_resolved()`
7. `ChannelRecord::update_balance()` + `advance_commitment()`

### Cooperative Close
1. Either party: `build_cooperative_close()` — close.rs:19
2. Exchange `CloseChannel` + `CloseAccepted` messages
3. `sign_cooperative_close()` — close.rs:65 (sorts sigs by pubkey hash)
4. Broadcast to L1; `Active -> CooperativeClosing -> Closed`

### Force Close (Unilateral)
1. `build_force_close()` — close.rs:102; broadcasts latest `CommitmentPair`
2. `Active -> ForceClosing`; `close_tx_hash` set
3. `ChainMonitor` waits `dispute_window` blocks
4. `DisputeWindowExpired` event -> `ForceClosing -> AwaitingClaim`
5. `build_delayed_claim()` — close.rs:162; spends `to_local` via delayed path (or_branches=[true])
6. `AwaitingClaim -> Closed`

### Penalty Flow (Breach Detection)
1. `ChainMonitor::check_channel()` detects `FundingSpent`
2. `RevocationStore::find_by_hash()` — commitment.rs:218 matches revocation hash from spending tx
3. `CounterpartyClosing -> PenaltyInFlight`
4. `build_penalty_tx()` — close.rs:128; uses `build_penalty_witness()` (or_branches=[false])
5. `PenaltyInFlight -> Closed`

### Multi-Hop Payment (Phase 2)
1. Build `ChannelGraph`, call `find_route(source, dest, amount)` — router.rs:104
2. Create `Invoice` — invoice.rs:27; encode as `doli:pay:<base64>`
3. `Payment::from_route()` — payment.rs:47
4. Add HTLCs along route (onion-peel style -- not yet implemented)
5. On success: `Payment::succeed(preimage)` — validates hash before marking

### Manager Tick Loop
`ChannelManager::tick()` — manager.rs:66:
1. `monitor.update_height()` — fetches current chain height
2. For each active channel: `monitor.check_channel()` -> collect `MonitorEvent`s
3. `handle_event()` dispatches: FundingConfirmed -> transition Active; RevokedCommitment -> PenaltyInFlight; DisputeWindowExpired -> AwaitingClaim; FundingSpent -> CounterpartyClosing; HtlcExpired -> log warning
4. `store.save()` — atomic JSON write

## DEPENDENCIES

External crates (from usage patterns):
- `doli_core` — `Amount`, `BlockHeight`, `ConditionError`, `Transaction`, `Input`, `Output`, `OutputType`, `SighashType`, `Condition`, `Witness`, `WitnessSignature`, `HASHLOCK_DOMAIN`
- `crypto` — `Hash`, `KeyPair`, `PublicKey`, `Signature`, `Hasher`, `hash_with_domain`, `sign_hash`, `ADDRESS_DOMAIN`
- `serde` + `serde_json` — all structs implement `Serialize`/`Deserialize`
- `reqwest` — async HTTP for `RpcClient`
- `tokio` — async runtime (`tokio::time::sleep` in manager loop)
- `tracing` — `debug!`, `info!`, `warn!`, `error!` throughout
- `chrono` — `DateTime<Utc>` on `ChannelRecord`, timestamps on `Invoice`/`Payment`
- `hex` — encode/decode tx hashes for RPC calls
- `thiserror` — `ChannelError` derivation
- `tempfile` — used in `store.rs` tests only

## CONSTRAINTS

### Supply Invariant
`balance.total() + htlc_total == capacity` enforced in:
- `CommitmentPair::build_local_commitment()` — commitment.rs:114; returns `CapacityMismatch` on violation
- `build_cooperative_close()` — close.rs:30; returns `CapacityMismatch` if `balance.total() != capacity`

### Reserve Invariant
`ChannelConfig::reserve_percent` (default 1%); each side must maintain >= reserve.
Reserve calculated via `reserve_for_capacity(capacity)` — config.rs:81.
Not auto-enforced in HtlcManager -- must be checked by caller.

### State Machine Enforcement
All state transitions MUST go through `ChannelRecord::transition()` — channel.rs:59.
Direct `self.state = x` bypasses `validate_transition` — never do this (AUDIT-CHAN-002).

### Revocation Seed Security (AUDIT-CHAN-001)
`derive_channel_seed()` — commitment.rs:63 — MUST use private key bytes.
Using only public data would allow anyone to compute all revocation preimages, breaking LN-Penalty.

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
Cooperative close sigs sorted by pubkey hash — close.rs:81.
Both must be deterministic -- order affects script hash and on-chain validation.

### Atomic Store Writes
`ChannelStore::save()` — store.rs:38 — writes to `.tmp` then renames. Never write the file directly.

## PATTERNS

### Or-branch selection in witnesses
Left branch (false) = immediate/penalty path; Right branch (true) = timelocked/delayed path.
Consistent across all channel conditions:
- `to_local`: false=penalty (counterparty + revocation preimage), true=delayed claim (owner after timelock)
- HTLC offered/received: false=claim with preimage, true=timeout refund

### Commitment number monotonicity
`ChannelRecord::advance_commitment()` — channel.rs:67. Never decrement or reset.
Each state update increments; revealing old preimage lets counterparty sweep via penalty.

### RevocationStore growth
Grows by one entry per channel update. No pruning. Each entry is 40 bytes (8B number + 32B preimage).

### Dual-funded channels via AnyoneCanPay
`build_funding_tx(..., dual_funded=true)` sets `SighashType::AnyoneCanPay` on all inputs.
Each party signs their own inputs independently; the other adds theirs later.

### RPC JSON-RPC 2.0 pattern
All RPC calls go through `RpcClient::call<T>(method, params)` — rpc.rs:90.
Returns `ChannelError::Rpc` on error response or missing result.

### Phase roadmap
- Phase 1 (implemented): single channel open/pay/close/penalty; all of state_machine, funding, commitment, close, htlc, conditions, monitor, manager, store, rpc, protocol
- Phase 2 (structs present, coordination incomplete): multi-hop routing (router.rs), invoices (invoice.rs), payment.rs
- Phase 3 (stub only): watchtower delegation (watchtower.rs), splice-in/out
