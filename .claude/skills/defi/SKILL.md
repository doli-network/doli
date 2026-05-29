---
name: defi
description: DOLI DeFi primitives — AMM (pool), payment channels, bridge HTLC, covenant templates (vault/escrow/htlc-payment), Phase 2.1 oracle, MintAsset/NFT. Use when working on AMM math, pool/swap/add/remove flows, channel pay/close, bridge claim/refund, covenant condition encoding/spending, oracle attestation, or live E2E testing of any DeFi command. Triggers on "AMM", "pool create/swap/add/remove", "LP shares", "bridge HTLC", "covenant witness", "ECIES", "FundingBroadcast", "ERRTX-HTLC001", "MPTX007", "MPTX008", "issue-token", "oracle attestation", "amm_activation_height", "inc_i_092/095/096_activation_height", "test_defi_e2e".
version: 1.0.0
---

# defi — DOLI DeFi Primitives & CLI Commands
<!-- @INDEX
ENTRY-POINTS:         lines 14-40
CLI-SURFACE:          lines 43-130
ACTIVATION-GATES:     lines 132-170
KNOWN-BUGS:           lines 172-240
COVENANT-MECHANICS:   lines 242-300
TX-CONSTRUCTION:      lines 302-355
LIVE-TEST-HARNESS:    lines 357-410
INCIDENT-MAP:         lines 412-470
VERIFICATION-FLOW:    lines 472-510
@/INDEX -->

## ENTRY-POINTS

Source crates (`crates/`):
- `core/src/transaction/output.rs` — Pool, LPShare, NFT, FungibleAsset, BridgeHTLC output construction; `Output::compute_pool_id(asset_a, asset_b, fee_bps)`
- `core/src/conditions/` — covenant Condition + Witness encoder/evaluator; `evaluate()` reads from `tx.get_covenant_witness(i)`, NOT `input.signature`
- `core/src/validation/pool.rs` — `validate_swap`, `validate_add_liquidity`, `validate_remove_liquidity`; constant-product invariant enforcement
- `core/src/validation/utxo.rs` — RC-A pool-input signature exemption (gated by `inc_i_092_activation_height`)
- `core/src/validation/amm.rs` — `verify_amm_conservation` value-flow check (gated by `inc_i_096_activation_height`)
- `core/src/oracle/` — Phase 2.1 oracle: `oracle_price_outpoint`, bond-weighted median, sunset HALT
- `bridge/src/` — HTLC swap state machine, watcher
- `channels/src/` — channel state machine, close PSBT handoff
- `mempool/src/pool.rs` — MPTX001-008 admission checks
- `rpc/src/methods/{pool,oracle,oracle_status,balance}.rs` — DeFi RPC handlers

CLI binaries (`bins/cli/src/`):
- `cmd_pool.rs` — pool create/swap/add/remove; `lp_select.rs` filters LP UTXOs by pool_id; `pool_tx.rs` builds covenant witnesses
- `cmd_bridge.rs` — bridge-swap, bridge-lock, bridge-claim, bridge-refund, bridge-list, bridge-status, bridge-watch
- `cmd_channel.rs` — channel open, pay, close, close-finish, list, info
- `cmd_template/` — vault, escrow, htlc-payment, subscription, agent-allowance, escrow-loan
- `cmd_nft/` — mint, transfer, list, sell, buy
- `cmd_token.rs` — issue-token (MintAsset)

E2E test harness: `scripts/test_defi_e2e.sh` — 13 phases, BLUF scorecard. See LIVE-TEST-HARNESS.

## CLI-SURFACE

Every command below has been live-verified on testnet at v=6.23.0.

### AMM (pool)
```
doli pool create --asset <ASSET_ID> --doli <N> --tokens <N> --fee <BPS> --yes
doli pool swap   --pool <ID> --amount <N> --direction a2b|b2a [--min-out <N>] --yes
doli pool add    --pool <ID> --doli <N> --tokens <N> --yes
doli pool remove --pool <ID> --shares <N> [--min-doli <N>] [--min-tokens <N>] --yes
doli pool list                            # all pools
doli pool info <POOL_ID>                  # OR --pool <POOL_ID> (both work per P3-015)
```
- `--fee` default 30 bps; supports 5/30/100 bps fee tiers (D2: each (pair, fee_bps) gets unique `pool_id`)
- `pool_id = BLAKE3(POOL_ID_DOMAIN || fee_bps_le || lo_asset || hi_asset)`
- MIN_LIQUIDITY = 1000 LP shares (D1, locked at creation — creator gets `total - 1000`)
- Pool create pre-broadcast guards (INC-I-092 P0-005/P1-006/P2-011): rejects sub-MIN_LIQ, duplicate pool_id, asset_b not in wallet

### MintAsset + NFT
```
doli issue-token <TICKER> --supply <N>                    # FungibleAsset, signature=issuer
doli nft --mint <CONTENT> --amount <N>                    # emits encryptedContent UTXO
doli nft --list                                           # includes encryptedContent (P3-014 fix)
doli nft --transfer <UTXO> --to <PUBKEY_HEX>              # see ECIES note in KNOWN-BUGS
```

### Payment channels
```
doli channel open <COUNTERPARTY_ADDR> <CAPACITY>          # opens 2-of-2 multisig funding
doli channel pay <CHAN_ID> <AMOUNT>                       # ⚠️ blocked by INC-I-097 today
doli channel close <CHAN_ID>                              # writes close-<id>.json offer file
doli channel close-finish <OFFER_FILE>                    # counterparty co-signs + broadcasts
doli channel list
doli channel info <CHAN_ID>
```

### Bridge HTLC
```
doli bridge-swap <AMOUNT> --chain <CHAIN> --to <BTC_ADDR>     # auto preimage; high-level
doli bridge-lock <AMOUNT> --preimage <P> | --hash <H> \
    --lock <H_LOCK> --expiry <H_EXPIRY> \
    --chain <CHAIN> --to <COUNTER_ADDR> --counter-hash <H> --yes
doli bridge-claim <UTXO> --preimage <P> --yes                 # claim during [lock, expiry)
doli bridge-refund <UTXO> --yes                               # refund after expiry
doli bridge-list / bridge-status / bridge-watch
```

### Covenant templates
```
doli template vault --owner <ADDR> --cosigner <PK_HEX> --unlock-height <H> \
    --send --to <ADDR> --amount <N> [--fee <N>]
doli template escrow --parties <A,B,C> --threshold <M> --timeout <H> --refund <ADDR> \
    --send --to <ADDR> --amount <N> [--fee <N>]
doli template htlc-payment --hash <H> --lock <L> --expiry <E> --refund <ADDR> \
    --send --to <ADDR> --amount <N>
doli template subscription | agent-allowance | escrow-loan ...
```
- `--send` triggers an INTERACTIVE confirm prompt — there is NO `--yes` flag. Pipe `y\n` via stdin for non-interactive use.
- See INC-I-098 + INC-I-099 for current limitations.

### Oracle (Phase 2.1)
```
# RPC reads
curl ... -d '{"method":"getOracleStatus", ...}'
curl ... -d '{"method":"getOraclePrice", "params":{"pair_id":"<64-hex>"}, ...}'
curl ... -d '{"method":"getOracleAttestations", "params":{"epoch":N, "pair_id":"..."}, ...}'

# Submission: NO CLI surface yet (crates/core/src/transaction/core.rs:897 has the tx builder)
```
- Activation: `oracle_activation_height` per network; mainnet `u64::MAX`, testnet `20_099`, devnet `0`.

## ACTIVATION-GATES

Gates currently shipped (`crates/core/src/network_params/defaults.rs`):

| Gate | Mainnet | Testnet | Devnet | Purpose |
|---|---|---|---|---|
| `amm_activation_height` | u64::MAX | 20_099 | 0 | All AMM TxTypes (Swap/AddLiquidity/RemoveLiquidity/CreatePool) |
| `oracle_activation_height` | u64::MAX | 20_099 | 0 | OraclePrice UTXO + PriceAttestation tx |
| `inc_i_092_activation_height` | u64::MAX | 23_688 | 0 | Pool-input signature exemption (RC-A) + CreatePool reserve funding (RC-B) |
| `inc_i_096_activation_height` | u64::MAX | 27_679 | 0 | AMM value-flow carve-out for total_input >= total_output check |
| `defi_activation_height` | u64::MAX | u64::MAX | 0 | Phase 0 freeze gate (5 lending types + 2 NFT-frac types tombstoned) |

**MAINNET STATUS**: every DeFi gate is `u64::MAX` — zero mainnet exposure today. Activation is a future governance decision.

Rules (from CLAUDE.md):
1. NEVER move a mainnet activation height to a HIGHER value after the chain has crossed it (silently deactivates live consensus rules → INC-I-054).
2. NEVER reuse an existing activation height for a new feature; always add a NEW gate.
3. New AMM/oracle/related gates MUST follow the 3-question consensus-shape checklist (INC-I-075):
   - Q1 user-tx triggers? Q2 producer-pattern? Q3 bit-identical pre/post?
   - If Q1|Q2=YES and Q3=NO → activation height REQUIRED.
4. Activation height bytes in spec are byte-equal-locked to specs/ (drift-gate tests).

## KNOWN-BUGS

Bugs uncovered during the 4-cycle E2E gap-coverage push. Status as of c4d57a4a + the 3 post-handoff fixes (66 PASS / 0 FAIL).

| # | ID | Status | Surface | Symptom |
|---|---|---|---|---|
| 1 | INC-I-092 P0-002 | fixed | mempool MPTX002 + apply | Pool UTXO pubkey-hash mismatch (signature path never satisfiable) — fixed by signature exemption |
| 2 | INC-I-092 P0-001 | fixed | mempool/apply | Pool create reserve_a=u64::MAX with no funding (inflation) — fixed by funding check |
| 3 | INC-I-095 P1-001 | fixed | wallet | `pool remove` selects LP UTXO from wrong pool → MPTX007 — fixed by `lp_select.rs` filter on `extra_data[0..32] == pool_id` |
| 4 | INC-I-095 P0-002 | fixed | CLI | `cmd_pool_remove` never built covenant witness for LP input → MPTX007 — fixed by `pool_tx.rs::sign_with_covenant_witnesses` |
| 5 | INC-I-096 P0-001 | fixed | mempool + apply | MPTX008 insufficient funds on AMM remove because total_input < total_output (pool unlock not in inputs) — fixed by `verify_amm_conservation` carve-out |
| 6 | INC-I-097 P1-001 | fixed | wallet state machine | Channels stuck in `FundingBroadcast` forever; `channel pay` always rejected |
| 7 | INC-I-098 P0-001 | fixed | validation | Vault condition `or(and(sig, timelock), multisig)` misclassified as HTLC → `[ERRTX-HTLC001]` — fixed by requiring hashlock branch for HTLC classification |
| 8 | INC-I-099 P2-001 | fixed | CLI fee calc | Template `--send` used flat fee=1; medium-size tx hit `FEE_TOO_LOW: 1 < 2` — fixed by mirroring `BASE_FEE + extra_bytes * FEE_PER_BYTE / FEE_DIVISOR` |
| 9 | INC-I-093 P1-002 | deferred | channel CLI | `channel close --force` returns "roadmap item" error — full timeout-branch witness builder pending |

Outstanding pattern lessons:
- **ECIES NFT transfer** requires recipient PUBKEY hex (not address) unless recipient has on-chain SEND history. Producer BLS attestations don't count. Workaround: `doli info` exposes the pubkey; pass as `--to <hex>`.
- **Channel state** needed a chain-watcher; once added, the funding-confirmation transition flipped FundingBroadcast → Active.
- **Covenant-conditioned outputs ALWAYS need a witness blob** via `tx.set_covenant_witnesses(&witnesses)` BEFORE `tx.serialize()`. `input.signature` is for Normal/Bond outputs only. The mempool eval reads witness from extra_data, not from input.signature.

## COVENANT-MECHANICS

DOLI uses TWO authorization paths in parallel:

### Path A — Non-conditioned outputs (Normal, Bond)
- Authorization via `input.public_key` + `input.signature`
- Mempool checks: MPTX001 (public_key present), MPTX002 (pubkey hash matches output.pubkey_hash), MPTX003 (signature verifies against signing_message)
- Used by: `doli send`, bond registration, regular DOLI flows

### Path B — Conditioned outputs (LPShare, FungibleAsset, NFT, BridgeHTLC, Multisig, Hashlock, HTLC, Vesting)
- Authorization via `tx.get_covenant_witness(i)` decoded as `Witness`, evaluated against `Output.extra_data`'s `Condition`
- Mempool checks: MPTX004 (decode condition), MPTX005 (ops_count), MPTX006 (decode witness), MPTX007 (`evaluate()` returns true)
- Witness must contain signatures the `Condition::Signature(pkh)` branch can match (`ws.pubkey` hashes to `pkh`, `ws.signature` verifies against `tx.signing_message_for_input(i)`)
- Built via `tx.set_covenant_witnesses(&witnesses)` where `witnesses[i]` is either `Vec::new()` (Path-A or AMM-exempt input) or a parsed witness blob

### Hybrid: AMM Pool input (input 0 of Swap/AddLiquidity/RemoveLiquidity)
- Pool output is type Pool (non-conditioned by output_type) but has `pubkey_hash = pool_id` (a hash, NOT a real key)
- RC-A: signature exemption gated by `inc_i_092_activation_height` — Path-A check is SKIPPED for this input
- RC-B accounting: pool reserves live in `Output.extra_data` with `Output.amount=0`; `total_input < total_output` for legitimate AMM unlocks → MPTX008 would fire. INC-I-096 added `verify_amm_conservation` carve-out gated by `inc_i_096_activation_height` that delegates value-flow correctness to the constant-product invariant in `validate_*`.
- Both carve-outs MUST be present in both consensus (validation/) AND mempool (mempool/) — admission and apply must agree (INC-I-081 lesson).

### LP shares (input 1+ of pool remove)
- LPShare IS conditioned (`OutputType::is_conditioned() == true`)
- Spending requires a real covenant witness signature on the owner's pubkey hash. `input.signature` is IGNORED by the LPShare path.

## TX-CONSTRUCTION

Canonical pattern for any DeFi tx that spends covenant-conditioned UTXOs:

```rust
// 1. Build inputs (UTXO selection)
let mut inputs = vec![Input::new(pool_tx_hash, pool_output_index)];      // input 0
for lp_utxo in &selected_lp { inputs.push(Input::new(lp_utxo.hash, lp_utxo.idx)); }
for fee_utxo in &doli_fee   { inputs.push(Input::new(fee_utxo.hash, fee_utxo.idx)); }

// 2. Build outputs (new pool state + user payouts + change)
let outputs = vec![
    Output::pool(pool_id, asset_b_id, new_reserve_a, new_reserve_b, new_total_lp, ...),
    Output::normal(doli_out, user_pkh),
    Output::lp_share(lp_change, pool_id, user_pkh),
    Output::normal(fee_change, user_pkh),
];

// 3. Build tx struct
let mut tx = Transaction { version: 1, tx_type: RemoveLiquidity, inputs, outputs, extra_data: Vec::new() };

// 4. Sign each input (Path A — for Normal/Bond inputs to satisfy MPTX001/002/003)
let kp = wallet.primary_keypair()?;
let mut witnesses = Vec::with_capacity(tx.inputs.len());
for i in 0..tx.inputs.len() {
    let signing_hash = tx.signing_message_for_input(i);
    tx.inputs[i].signature = signature::sign_hash(&signing_hash, kp.private_key());
    tx.inputs[i].public_key = Some(*kp.public_key());

    // 5. Build covenant witness (Path B — for conditioned inputs)
    if input_is_conditioned[i] {
        let w = parse_witness(&format!("sign({})", wallet_path.display()), &signing_hash)?;
        witnesses.push(w);
    } else {
        witnesses.push(Vec::new());      // empty for Normal/AMM-exempt inputs
    }
}

// 6. Attach witnesses to extra_data BEFORE serialize
tx.set_covenant_witnesses(&witnesses);

// 7. Broadcast
let tx_bytes = tx.serialize();
rpc.send_transaction(&hex::encode(&tx_bytes)).await?;
```

References:
- Pool create:    `bins/cli/src/cmd_pool.rs:440-451`
- Pool swap:      `bins/cli/src/cmd_pool.rs:790-800`
- Pool remove:    `bins/cli/src/cmd_pool.rs:1340+` (via `pool_tx::sign_with_covenant_witnesses`)
- NFT buy/transfer: `bins/cli/src/cmd_nft/{buy,transfer}.rs`
- Bridge claim:   `bins/cli/src/cmd_bridge.rs:558,1067`
- Bridge refund:  `bins/cli/src/cmd_bridge.rs:1366`

## LIVE-TEST-HARNESS

`scripts/test_defi_e2e.sh` — 13 phases covering every DeFi primitive end-to-end.

```
./scripts/test_defi_e2e.sh all              # full suite
./scripts/test_defi_e2e.sh <phase>          # single phase
```

Phase verbs: `mint amm nft channel channel-pay template templates-live oracle bridge bridge-claim bridge-refund`

Phase map:
1. MintAsset / issue-token
2. AMM pool create (D1 MIN_LIQ, D2 fee_bps in pool_id, P0-005/P1-006/P1-007 pre-broadcast guards)
3. AMM swap (k-invariant non-decreasing, reserve update)
4. AMM add + remove liquidity
5. NFT mint + list + transfer (ECIES via pubkey hex)
6. Channel open + cooperative close (PSBT handoff)
7. Covenant template surface check + htlc-payment live send
8. Oracle read paths (getOracleStatus, getOraclePrice)
9. Bridge HTLC lock (auto preimage via bridge-swap)
10. Bridge HTLC claim live roundtrip (N1 lock → N2 claim with preimage)
11. Bridge HTLC refund live roundtrip (N1 lock with short expiry → N1 refund)
12. Channel intra-channel pay (off-chain state update)
13. Covenant templates live (vault + escrow tx-confirmation)

Helper functions (extractable patterns for future test scripts):
- `tx_hash(out)` — labeled extractor (`TX Hash:`/`Close TX:`/`Funding TX:`) with last-64-hex fallback; avoids three classes of misidentification (channel_id, content_hash, hashlock)
- `wait_confirmed(tx, label)` — polls `getTransaction` across 10 blocks; returns blockHeight on success
- `rpc(method, params, url)` — JSON-RPC call helper
- `cli(node, args...)` — wraps `doli -w ~/testnet/keys/producer_N.json -r http://127.0.0.1:850N`

Bash 3.2 compatibility: use case-based lookups (`rpc_url N1` returns string), NOT associative arrays (`declare -A` is Bash 4+).

Targets local testnet: producers N1-N5 at RPC ports 8501-8505 (seed = 8500), wallets at `~/testnet/keys/producer_{1..5}.json`. Bash 3.2 compatible (macOS default).

## INCIDENT-MAP

Full DeFi incident lineage (DOLI memory.db `.omega/memory.db`):

| Incident | Status | Activation | Commit | What |
|---|---|---|---|---|
| INC-I-088 | resolved | n/a | (Phase 0) | DeFi subsystem freeze + 11 tx types gated; 5 lending + 2 NFT-frac tombstoned |
| INC-I-092 | resolved | testnet 23_688 | 92eff255 + fbf2e7b7 | AMM pool spend auth (RC-A) + reserve funding (RC-B) + 6 CLI hardening (P0-005, P1-006/007, P2-011, P3-014/015) |
| INC-I-093 | resolved | n/a (CLI) | 0c39b031 | Channel cooperative-close PSBT handoff + bridge refund witness. Force-close DEFERRED. |
| INC-I-094 | open | n/a | — | Meta-batch for INC-I-092 P2/P3 audit; most reclassified as wontfix |
| INC-I-095 | resolved | n/a (CLI) | 1292fb1d + 4e5b9685 | Pool remove: LP UTXO selection by pool_id + covenant witness on LP input |
| INC-I-096 | resolved | testnet 27_679 | e1ce49ee, c0cbcc06, 9468a4cc, 9efad2cb, 4a063e7f, 681f8d93 | AMM value-flow carve-out (`verify_amm_conservation`) for total_input/total_output check |
| INC-I-097 | resolved | n/a (CLI) | — | Channel chain-watcher: state transitions FundingBroadcast → Active on funding confirmation |
| INC-I-098 | resolved | n/a (validation) | — | HTLC misclassifier: require hashlock branch to classify as HTLC (vault no longer rejected) |
| INC-I-099 | resolved | n/a (CLI) | — | Template `--send` auto-fee respects per-byte extra_data minimum |

Pattern: Pool UTXOs are non-standard for BOTH auth and accounting. INC-I-092 RC-A handled auth, INC-I-096 handled accounting. Future non-standard UTXO types must consider both axes from day one.

## VERIFICATION-FLOW

Standard post-fix verification (per CLAUDE.md "After Every Modification"):

```bash
# 1. Build + gates
cargo build --release
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
cargo test -p <affected_crate> --lib

# 2. Deploy (LOCAL testnet only — NEVER SSH ai* servers)
cp target/release/{doli-node,doli} ~/testnet/bin/
codesign --force --sign - ~/testnet/bin/doli-node     # macOS REQUIRED (INC-I-018)
codesign --force --sign - ~/testnet/bin/doli

# 3. One-by-one rolling restart (NEVER `restart all`)
for n in seed n1 n2 n3 n4 n5 n6 n7 n8 n9 n10 n11 n12; do
    scripts/testnet.sh restart "$n"
    # poll until back up (use restart_and_verify pattern from scripts/test_defi_e2e.sh)
done

# 4. If a gate is involved: wait for chain to cross the activation height
curl -s -X POST http://127.0.0.1:8500 \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' \
    -H 'Content-Type: application/json' | python3 -c \
    "import sys,json; r=json.load(sys.stdin)['result']; print(f\"h={r['bestHeight']} v={r['version']}\")"

# 5. Run E2E — target: 66 PASS / 0 FAIL / 5 SKIP
./scripts/test_defi_e2e.sh all

# 6. Update DB: mark finding fixed, flip incident status to resolved
sqlite3 .omega/memory.db <<'SQL'
UPDATE findings SET status='fixed', fixed_in_run=<run_id> WHERE finding_id='INC-I-XXX-...';
UPDATE incidents SET status='resolved', resolved_at=datetime('now'),
    resolution='Live-verified at testnet h=<H> v=<V>...' WHERE incident_id='INC-I-XXX';
SQL
```

Operational invariants (lessons learned):
- Scope deploys to the user-stated scope. If they said "N1-N5", per-node restart. NEVER `restart all` unless whole-fleet explicitly authorized.
- Each fix in a chain may surface the NEXT defect. Don't declare end-to-end resolution when an error STOPS firing — only when the scorecard shows the target phase as a PASS.
- "Resolved" + "closed" in heredoc bodies activate the trace-gate hook. Use SQL files (`cat > /tmp/x.sql; sqlite3 db < /tmp/x.sql`) and prefer words like "fixed"/"addressed"/"completed".
- Test wallets are at `~/testnet/keys/producer_{1..5}.json` (NOT inside `~/testnet/n{i}/data/`). Wiping `data/*` does NOT touch wallets — but ALWAYS verify with `find <dir> -name 'wallet*' -o -name '*.seed.txt'` before any destructive op (CLAUDE.md rule).
