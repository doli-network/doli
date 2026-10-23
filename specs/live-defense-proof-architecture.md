# Architecture: Live equivocation-defense proof and whitepaper alignment

Requirements: `specs/live-defense-proof-requirements.md` (REQ-LDP-001..062, milestones M1 and M2). Status: PENDING.
Source of truth order: code > this spec > `scripts/README.md` > whitepaper. No consensus, validation, mempool, storage or apply rule changes, except DC-5, which is conditional on a live-confirmed defect.

## 1. SSF design (the one design)

Extract the GS-027 cluster code into one sourced, side-effect-free shell library driven by neutral `DN_*` globals plus one Python helper for every JSON verdict; write GS-028 and GS-029 as thin phase scripts on that library; add one transaction-forge example; change the two CLI call sites from `NetworkParams::defaults` to `NetworkParams::load`; register tokens, seed rows and README lines; edit the whitepaper in EN and ES; record runs and decisions. GS-027 keeps its behaviour and its self-test.

Why this and not a copy: the scope forbids copy-paste of the cluster code, and `GS027_LIB_ONLY=1` returns at `gauntlet-gs027.sh:118`, before every cluster function (`:153-330`), so sourcing GS-027 as it stands delivers nothing a new runner needs (observed). Moving the guard below the functions was the subtraction alternative; it fails because `start_node` hard-codes one binary and two gates (`:219-225`) and `cleanup` hard-codes the `gs027.*` glob (`:205`), so every new runner would redefine three functions, which is the copy the scope forbids. conf(0.8, observed).

Scalability ceiling: the library serves one cluster per host of up to about 10 nodes, because the sampler issues 3 RPCs per node every 2 s under an 8 s curl timeout and a poll must finish inside one 10 s slot; the Python history check is N x H block reads (about 600 at N=6, H=100), linear.

━━━ RESOURCE COST — SUMMARY ━━━
Dimensions:
  CPU:      +moderate during a run only: 6 node processes with VDF work per 10 s slot, one sampler (observed)
  Memory:   +about 6 x node resident set for the run; 0 at rest (observed)
  IO:       +3 RPCs per node per 2 s during a run; 0 at rest (observed)
  Network:  +loopback only; 0 off-host (observed)
  Disk:     +one scratch dir per run under TMPDIR, deleted on exit unless kept (observed)
  Latency:  0 on any node path; the CLI fix adds one env read at process start (inferred)
Inevitability: AVOIDABLE
Cheaper alternative: run no live cluster and rely on the in-process tests already named in the requirements (D2)
Why this proposal anyway: the four gated rules were never exercised together nor against the mainnet binary; the in-process tests cannot show a mixed-version boundary or a withdraw-then-slash race on real nodes
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## 2. Design space gate

- Existing patterns reused: ephemeral devnet runner (GS-026/027), `res`/`verdict` output contract, own-PID discipline (`alive && ours`), `python3 -I` JSON parsing, G-Set seeding, sibling forge, companion offline test with planted-violation controls, seed-SQL upsert rows, standalone SKIP arm in `scripts/gauntlet.sh`.
- Stack constraints: bash runners, `python3 -I`, Rust examples under `bins/node/examples/` (reqwest is already a dependency), no new RPC fields (no RPC exposes the 257/182 gates or `unbonding_period`).
- Performance requirements: wall time per run (section 8); no node-path latency budget is affected.
- Dependency direction: runners -> library -> binaries over RPC; runners never import each other.
- Prediction made before reading: "GS-027 exposes its cluster helpers in library mode". Wrong: the guard precedes them. Design impact: extraction instead of sourcing (section 1).

Constraint table (institutional memory, scope gauntlet/slashing/unbonding):

| # | Past evidence | Constraint on this design |
|---|---|---|
| C1 | A runner validated only standalone, not with the exact seed runner text, broke the harness (outcome, score -1) | DC-4 tests `bash -c "<runner>"` with `scripts/` on PATH |
| C2 | Missing skip-arm tokens made every plain run FAIL with "unknown assertion token" (GS-023/024) | DC-4 adds tokens in the same commit as the seed rows |
| C3 | A control registered as `expect_fail=1` would count a SKIP (rc 2) as the expected failure | No control row; controls run through env with `expect_fail=0` rows |
| C4 | A stall on an ephemeral devnet is not evidence until the same harness runs on the pre-fix binary | GS-028 control first; GS-029 negative control before the HEAD run |
| C5 | GS-027 counted lines with `wc -l` over `printf` without newline (bug) | Python helper prints one JSON line; bash counts with `wc -w` |
| C6 | INC-I-054: an epoch-state rebuild after restart forked at the next boundary | DC-3 asserts 0 `[EPOCH_REBUILD]` and 0 snap-sync lines after the same-dir restart |
| C7 | INC-I-075: a producer-set change applied at a mixed-version boundary split the fleet | DC-3 requires the flushing boundary while the old nodes run, and compares state roots there |
| C8 | `pipefail` + `grep -q` SIGPIPE false negatives; zsh `PIPESTATUS` empty | Verdict commands never end in a pipe; callers read `$?` directly |
| C9 | Hotspot `cmd_producer/common.rs`: CLI vesting arithmetic must stay byte-identical to the node bound | DC-1 touches only the params source, never the vesting math |

## 3. Module map (files touched per milestone)

| Milestone | File | Change |
|---|---|---|
| M1 | `scripts/gauntlet-devnet-lib.sh` (new, about 240 lines) | cluster functions moved from GS-027 `:153-330` plus `start_cluster`, `write_chainspec`, `gen_wallets`, `check_ports`, `restart_node`, `cli_as`, `wait_tx_mined`, `sampler_start/stop` |
| M1 | `scripts/gauntlet-devnet.py` (new, about 260 lines) | GS-027 `sample.py` body plus subcommands `scan`, `removed`, `producers-norm`, `history`, `attrib`, `forkcheck` |
| M1 | `scripts/gauntlet-gs027.sh` (495 -> about 330 lines) | sources the library after `ROOT=`; keeps header, `res`/`verdict`, its pure helpers, the `GS027_LIB_ONLY` guard, mktemp, trap, the gate env literal, evidence and assertions |
| M1 | `scripts/test_gauntlet_gs027.sh` | four lines: `c_node_env` and `c_harness_safety` scan runner + library; `unsafe()` rm exemption accepts the `"$DN_TAG".` glob; `kill "$PID"` check scans runner + library |
| M1 | `scripts/gauntlet-gs028.sh` (new, <= 480), `scripts/test_gauntlet_gs028.sh` (new) | DC-3 |
| M1 | `scripts/gauntlet-gs029.sh` (new, <= 480), `scripts/test_gauntlet_gs029.sh` (new) | DC-3; if the runner passes 480 lines, P7 probes move to `scripts/gauntlet-gs029-probes.sh`, sourced |
| M1 | `bins/node/examples/gs028_forge_tx/{main.rs,shape.rs}` (new, one example target) | DC-2; `bins/node/tests/it/inc_i_182_m3_forge_shape.rs` includes `shape.rs` by `#[path]` |
| M1 | `bins/cli/src/unbonding_tx.rs`, `bins/cli/src/cmd_producer/withdrawal.rs:150`, `exit.rs:170`, `bins/cli/tests/` | DC-1 |
| M1 (conditional) | `crates/storage/src/producer/set_lifecycle.rs`, `info.rs`, `tests.rs` | DC-5 |
| M2 | `WHITEPAPER.md`, `WHITEPAPER-es.md`, `scripts/test_whitepaper_defense_text.sh` (new) | DC-4 |
| M2 | `scripts/gauntlet.sh` (skip arm + header), `scripts/gauntlet-seed.sql`, `scripts/README.md`, `specs/SPECS.md`, memory rows | DC-4 |

Library contract (`DN_*`, set by the runner before any call; the library defines functions only): `DN_TAG` (scratch prefix and cleanup glob), `S` (scratch, created by the runner's own `mktemp`), `N`, `P`, `DN_BIN[i]`, `DN_ENV` (array of `KEY=VAL` exported to every node), `DN_LOG[i]`, `DN_KEY[i]` (producer wallet or empty), `DN_CLI`, `DN_CLI_ENV` (array for `cli_as`), `DN_FORGE`, `DN_SIB_PK` (sibling target, empty = any producer), `DN_SIB_MIN` (lowest sibling height), `DN_PHASE_MAX`, `DN_KEEP`. `skip` prints `SKIP $DN_TAG: <reason>` and exits 2.

## 4. Definite Changes

### DC-1 Library split and CLI gate source (resolves a; REQ-LDP-001, REQ-LDP-002, REQ-LDP-025)

GS-027 stays green because every idiom its self-test pins stays in the runner file: the `GS027_LIB_ONLY` guard still precedes any top-level `mktemp|trap|curl|lsof|start_node|sleep|nohup` (`test:127-134`; the `. lib` line matches none of them and the library has no side effect); the seven helpers are still defined after sourcing (`:136-147`, `all_equal` now comes from the library through the runner); every `GS027_*` name the runner reads is in its header and README entry (`:301-311`; the library reads only `DN_*`); both gate names appear as `NAME=` in the runner's `DN_ENV` literal (`:318-319`); `mktemp -d ... gs027.XXXXXX` stays in the runner (`:424`); the seed block, the emitted ids and the skip arm are untouched (`:346-402`). Two checks must read runner + library: `--log-level info` (`:321`) and `kill ... "$PID"` (`:426`), and the rm exemption (`:413`) must accept the library's `*/"$DN_TAG".*) rm -rf "$S"` arm. The planted-violation controls (6 of 6) are unchanged. conf(0.75, observed). Proof: `scripts/test_gauntlet_gs027.sh` all PASS, then a GS-027 live run PASS; on a FAIL, run the pre-refactor runner at HEAD to separate a refactor fault from a code fault.

CLI: add `pub fn builder_params(network: Network) -> &'static NetworkParams { NetworkParams::load(network) }` in `unbonding_tx.rs`; `withdrawal.rs:150` and `exit.rs:170` call it instead of `defaults`. `load` applies the devnet-only env overrides (`env_loader.rs:577-608`, `:83-86`) and never reads a data-dir `.env` (that path is `load_env_for_network`, which the CLI must not call). Tests: (A) a child-process test (pattern `inc_i_182_m1_common.rs:168-194`, because `load` is cached in a `OnceLock`) sets the 257 gate and `DOLI_UNBONDING_PERIOD`, asserts devnet params carry them and that `build_withdrawal_tx` for mainnet and testnet is byte-identical with and without the variables; (B) a wiring test that the two call sites no longer contain `NetworkParams::defaults(` (`include_str!`). (A) and (B) are RED on the parent commit. conf(0.8, observed). Blast radius: the two call sites; their only caller is `cmd_producer/dispatch.rs`.

━━━ RESOURCE COST — DC-1 ━━━
Dimensions:
  CPU:      +constant: one env scan at CLI start (observed)
  Memory:   +constant: one static NetworkParams per CLI process (observed)
  IO:       +one extra file read when a runner sources the library (observed)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  +under 1 ms per CLI invocation (inferred)
Inevitability: AVOIDABLE
Cheaper alternative: move the GS027_LIB_ONLY guard below the cluster functions and pass the gate height to the CLI as a new flag
Why this proposal anyway: the guard move copies three functions per runner and keeps the gs027.* cleanup glob as a trap; a CLI flag lets a CLI and a node disagree silently, which is the error class GS-029 phase 1 depends on
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### DC-2 Transaction forge (resolves d; REQ-LDP-003, 004, 030, 032)

`gs028_forge_tx --kind forced-exit|forced-exit-wrong-tail|addbond-untailed --payer-key <wallet.json> --victim <pkhex> --bonds <n> --payout normal|unbonding --lock-until <h> --ports <p,p,...> [--fee 1] [--dry-run]`. `shape.rs` is pure and builds the fixture shape of `inc_i_182_m1_common.rs:57-66`: inputs = the payer's own Normal coin signed by the payer, one output to `bond_owner(victim)`, no tail. `main.rs` loads the key (pattern `gs026_forge_block.rs:140-149`), takes the coin from `getUtxos` and the genesis hash from `getChainInfo`, and submits the same hex to every port via `sendTransaction`. Guards: host fixed to `127.0.0.1`; `getNetworkParams.network == devnet` on every port before any send (else exit 3, nothing sent); only the given wallet path is read; no wallet field is printed (the companion test keeps the planted-print control). Output: `FORGED kind= txid= victim=<8hex> payout= lock_until=`, then per port `SUBMIT port= accepted=0|1 code=<ERRTX-...|none> err="<=160 printable chars>"`. The code is matched in `error.message` and `error.data.reason`, never in `error.data.error_code` (that field is the generic `INVALID_TRANSACTION`). Exit: 0 built and every submit attempted, 2 bad args, 3 guard refused, 4 RPC unreachable. Verdicts stay in the runner.

Shape rule at HEAD above the gate: payout Unbonding to `bond_owner(victim)` with `lock_until >= best + unbonding_period + 20`, because `check_output_placement` runs before `check_producer_auth` (`validation/transaction.rs:115-117`) and a wrong shape is refused with an `ERRTX-UNBOND...` code, not `ERRTX-PAUTH001`. On the control the payout is Normal (type 16 does not decode there). conf(0.7, observed).

━━━ RESOURCE COST — DC-2 ━━━
Dimensions:
  CPU:      +constant per invocation: one Ed25519 signature, six RPC posts (observed)
  Memory:   N-A (observed)
  IO:       +one wallet read (observed)
  Network:  +about 7 loopback RPC calls per invocation (observed)
  Disk:     +one example binary in target/ (observed)
  Latency:  N-A (observed)
Inevitability: AVOIDABLE
Cheaper alternative: add a hidden --producer flag to the CLI request-withdrawal command
Why this proposal anyway: a forging flag in the shipped wallet is an attack tool for users; an example binary is devnet-guarded and never installed
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### DC-3 Runners GS-028 and GS-029 (resolves b, c and e; REQ-LDP-005..033)

**GS-028 cluster:** N=6, P=5 (index 0..4; 1 = victim V, 2 = third party T, target X chosen from {0,3,4}), `GENESIS_BLOCKS=10`, `BOOTSTRAP=12`, `E=15` (`DOLI_BLOCKS_PER_REWARD_EPOCH`, so each producer has about 3 slots per epoch and X keeps at least 2 after W), `H=30` for all four gates (`DOLI_INC_I_236/255/257/182_*`), pending-root 0, devnet `unbonding_period` 60 and `vesting_quarter_slots` 60 untouched, slot 10 s. Boundaries at 15, 30, 45, 60. The sampler runs in the background for the whole run.

**GS-028 sequence** (heights are expectations, deadlines are hard):
1. Wait for height 31 on every node. T holds a matured EpochReward from the boundary at 15 (maturity 10); if `getUtxos(T)` shows no spendable coin, wait one epoch, then SKIP "third party unfunded".
2. Forced-exit probe (DC-2) against V on all six ports; expect six refusals carrying `ERRTX-PAUTH001`; record its txid. Its "never mined over the next epoch" and "V unchanged after the next boundary" checks run during steps 3-7 at no extra cost. A refused tx is never relayed (`rpc/methods/transaction.rs:237` returns before `:242`), so W and S stay clean.
3. Pick X = the candidate with most slots in heights 36..44 (`getSlotSchedule`), require >= 2; read B = `getProducer(X).bondAmount`. `cli_as X producer request-withdrawal --count <bondCount>` with `DN_CLI_ENV` (the four gates) and an explicit `--rpc` to node 0; parse `TX: <64hex>`, validate hex.
4. Wait until the withdrawal is at one height W on every node; deadline W <= 40. Overrun: retry in the next epoch (at most 2 epochs), then SKIP "withdrawal not mined before the epoch's evidence window". Read U = the amount of its single output, its type and lock height via `getTransaction`.
5. Evidence: the first block by X with height > W that every node holds (poll <= 2 slots); publish its sibling at once (`inject_sibling` with `DN_SIB_PK=X`, `DN_SIB_MIN=W+1`; parent = height-1, same slot). If `INJECTED` is missing, retry with X's next block in the same epoch, at most 3 attempts. Fallback when X has no block before the boundary at 45: the newest X block in [W-3, W) whose slot is at most 4 slots old at injection time; if none, SKIP "evidence window missed". The gossip age limit is 6 slots (`gossip/validation.rs:35`).
6. Slash: scan heights (X_h, X_h+6] for one `slash_producer` tx naming X; S expected in X_h+1..X_h+3. No slash by X_h+6 is a FAIL of `gs028-one-slash-mined` with `slash_height=none` (never SKIP).
7. Observe to max(S, next boundary after S) + 6; read status, UTXOs and unbonding balance on every node.

Numbers read from the chain, never assumed: B; U (the CLI's vesting rule makes U about 0.25 B minus the fee before slot 60); the output type and lock height at W; issued(h) = coinbase outputs of h; per-tx input and output amounts for removed(h) (an input with no amount in `getTransaction` is resolved through its outpoint; an unresolvable input makes removed(h) undefined and the assertion FAIL). Supply = most frequent `getChainStats.totalSupply` sample per height, cross-checked with the `[SLASH] burned N units` log line at S.

Assertions: `gs028-withdrawal-before-evidence`, `gs028-one-slash-mined`, `gs028-slash-burns-locked` (removed(S) == U), `gs028-full-bond-leaves-supply` (removed(W) + removed(S) == B), `gs028-producer-gets-nothing`, `gs028-forced-exit-refused` (prints `code=` and `mined=` per node), `gs028-tips-equal`, `gs028-production-continues`, and the new `gs028-target-status-slashed`: after the first boundary that follows S, `getProducer(X).status == slashed` on every node and no node log holds `Deferred slash failed`. METRIC: `SLASHED_NODES= target= W= S= B= U= attempts= status= probe_code= control=`.

**GS-028 control** (`GS028_CONTROL=1`: node and CLI from the pre-unbonding build, forge `--payout normal`; the 257/182 variables are ignored there) declares before running expected-FAIL = {slash-burns-locked, full-bond-leaves-supply, producer-gets-nothing, forced-exit-refused, target-status-slashed} and preconditions = {withdrawal-before-evidence without its lock clause, tips-equal}. Two conclusive paths: (i) S mined while X is still Active and removed(S) == 0; (ii) a boundary makes X Exited first, the slash is refused, S = none. Both show the payout survived: the W:0 output is `normal`, spendable, amount U. Exit 1 only when the preconditions PASS and at least one funds assertion FAILs with those values; otherwise `SKIP gs028: control inconclusive (<why>)`, exit 2, and the operator re-runs.

**Probable defect** (Skeptic A0, verified against the code): the boundary applies the queue FIFO (`set_core.rs:110-127`); `apply_withdrawal` sets `Exited` at bond_count 0 (`info.rs:443-445`); `slash_producer` then refuses `[STOR041]` because `is_active()` is false for `Exited` (`set_lifecycle.rs:177-182`, `info.rs:170-175`), the error is only logged (`set_core.rs:124-126`), and `register` accepts an `Exited` key again (`set_registration.rs:24-31`). The same happens when the boundary falls between W and S. The burn itself is unaffected (UTXO layer). conf(0.7, observed). **Severity if the live run confirms it: P1.** Not P0: the whole bond is burned, every node reaches the same `Exited` deterministically (no fork, no loss to honest parties), and the path is dormant on mainnet and testnet (both gates `u64::MAX`). P1 because the lifetime bar, the only lasting deterrent of the slashing rule and a whitepaper claim (REQ-LDP-043), is false on exactly the attack path this scenario models, and the fix is local to the gated path. The developer proves it live first (`gs028-target-status-slashed` FAIL at HEAD plus the `[STOR041]` log line on every node), then applies DC-5 only if confirmed; the §11.3 "barred for life" sentence is written only after DC-5 is GREEN.

**GS-029 cluster:** N=6; nodes 0-3 genesis producers (node 0 with `bond_count: 2`), node 4 HEAD validator, node 5 old validator; nodes 3 and 5 run the v6.44.2 binary (`DN_BIN`), the rest HEAD. `E=8`, `GENESIS_BLOCKS=10`, `BOOTSTRAP=12`, `H=72` for the four gates on HEAD nodes and the CLI (the old binary ignores the variables; it receives pending-root 0 like every node), slot 10 s. Roles are identified by binary path, sha256 and pubkey in the header line, never by version string (both report the same version).

**GS-029 phase machine** (height deadlines; an overrun in HEAD mode prints `SKIP gs029: <phase> overran`):
- P0 setup and G-Set seeding; SKIP unless `gset_ready == N`.
- P1 funding (deadline 34): funder F = node 0 holds a matured EpochReward >= 2 bond units + fees.
- P2 mixed transactions (all mined at heights < 56): transfer F -> NEW through node 5's RPC (the old validator); Registration of NEW (`--bonds 1`) through node 4; AddBond by producer 1 (`--count 1`) through node 2; partial RequestWithdrawal by producer 0 (`--count 1` of 2) through node 0, the fee paid from a matured reward. Distinct producers avoid the CLI ledger-mismatch abort (`withdrawal.rs:65-72`). Every tx must show one height on every node.
- P3 boundary check at b3 + 1, where b3 is the first boundary after the last mined tx (b3 <= 56): `producers-norm` (key, status, bond_count) identical on all six and showing NEW, bond_count 2 for producer 1 and 1 for producer 0; `history 1..b3+1` identical; supply equal per sampled height; `getStateRootDebug` equal wherever all nodes report the same (height, bestHash), components printed on mismatch.
- P4 restart (deadline 62): stop node 3, then node 5, by PID, one at a time with >= 2 slots between; keep the old log as `node.old.log`; `restart_node i` sets `DN_BIN[i]=HEAD`, `DN_LOG[i]=node.head.log` and starts on the same data dir with the same gate env. PASS iff the first `bestHeight` >= the stop height, the node reaches the common tip, and all logs hold 0 `[EPOCH_REBUILD]`, 0 `[SNAP_SYNC]`/`action=SnapSync` and 0 `EQUIVOCATION DETECTED` lines.
- P5 the boundary at 64 falls after the restart; state root equal there.
- P6 crossing: `history R_stop..H+10` identical on every node.
- P7 probes (heights 73..90): an AddBond from the old CLI (fallback: forge `--kind addbond-untailed`) against every node, refused with `ERRTX-PAUTH001` and never mined (the submit line with the code is required, otherwise FAIL, never a vacuous PASS); one forge `--kind unsigned` block, built for the next slot of a HEAD producer paused by `pauseProduction` and resumed after, whose `INJECTED` hash is absent at its height on every node; an AddBond from the HEAD CLI mined by H+12; (Should) a HEAD-CLI withdrawal whose single Unbonding output has a lock height above its block.

Attribution: from one HEAD node, (height, hash, producer, txs) for every height; old-built = producer pubkey of node 3 at heights <= its stop height (only the old binary ran that key). `gs029-old-producer-built` needs >= 1 such block in phase 1; METRIC `old_built_producer_txs` counts producer txs in them. `gs029-old-blocks-canonical`: every (height, hash, producer) tip the sampler saw on any node during phases 1-3 must be on the final common chain; the old log's produced-block lines are a secondary signal only when present (count reported, never required, because that format is unverified).

Fork versus stall: a fork is at least two distinct hashes at one height <= the common minimum height, or a sampled tip absent from the final chain; a stall is no progress of the minimum height for `GS029_STALL_SLOTS` (6) slots with no fork evidence. HEAD mode: fork -> FAIL `gs029-mixed-no-fork` (first differing height, producer of each side), stall -> SKIP. Negative control (`GS029_NEGATIVE=1`: HEAD gates and CLI gates at BOOT+2 = 14, phases P0-P3 only): precondition = tips equal at every height <= 14; after it a fork is a conclusive FAIL of `gs029-mixed-no-fork` or `gs029-old-blocks-canonical` and the runner exits 1; a stall or dead node with no fork evidence prints `SKIP gs029: control inconclusive`, exits 2, and is re-run (never reported as the result). Expected mechanism: HEAD rejects the unsigned old-producer header at gossip above its gate while the old nodes accept both chains, so the first old-producer slot at or above 14 yields two hashes at one height.

━━━ RESOURCE COST — DC-3 ━━━
Dimensions:
  CPU:      +one 6-node cluster for 13-20 min per run, two restarts in GS-029 (inferred)
  Memory:   +6 node resident sets for the run (observed)
  IO:       +about 18 RPCs per 2 s plus N x H block reads at the end (observed)
  Network:  +loopback only (observed)
  Disk:     +one scratch dir (6 data dirs, logs, block JSON), deleted on exit (observed)
  Latency:  N-A (observed)
Inevitability: AVOIDABLE
Cheaper alternative: extend GS-027 with withdrawal and forced-exit steps and GS-026's mixed phase with transactions, behind env knobs
Why this proposal anyway: GS-027 is at 495 lines and GS-026 at 517, so any knob breaks the 500-line limit; the control build, the five-producer floor, the mixed boundary and the same-dir restart (INC-I-075, INC-I-054) are absent from both
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### DC-4 Registration, records, whitepaper EN and ES (resolves f and g; REQ-LDP-022, 026, 040..054, 060..062)

`scripts/gauntlet.sh`: two case arms after `:753` (`gs028-*` and `gs029-*` tokens, `ok=2`, standalone reason naming the runner) and two header bullets after the GS-027 bullet. `scripts/gauntlet-seed.sql`: GS-028 upsert (status active, runner `scripts/gauntlet-gs028.sh`, `expect_fail 0`, assertions = the nine runner ids, `incident_ids` INC-I-236/255/257/182, description in the STANDALONE/INJECTS/ASSERTS/safety/SKIP form) and a new GS-029 row in the same form; never `expect_fail=1` for any row. README: one bullet per runner after the GS-027 bullet (env knobs, control modes, output contract, exact commands) and two Quick Reference rows. `gauntlet_runs` rows are written by the orchestrator, one per run: `run_id`, `status` = the runner's verdict (`pass`/`fail`), `scenarios_run` 1, `scenarios_passed` 0 or 1, `git_sha`, `duration_seconds`, `mechanism_id` = the scenario id (`GS-026`..`GS-029`), and `failures` = JSON `{scenario, mode: head|control|negative, expected_fail: [...], preconditions: [...], conclusive: bool, lines: [every PASS/FAIL/METRIC line], exit, env: {knobs}}`.

Gauntlet honesty (R-4 scope): the seven rows that share the plain `gauntlet.sh` run keep one runner text; the local run script executes each distinct runner text once and attributes per-scenario results from that run's own per-scenario lines, counting a SKIP line as ran-not-passed. Opt-in rows: GS-009, GS-019 and GS-022 (no chain write) get runner text with both the flag and the confirm variable (`GAUNTLET_GS0NN_CONFIRM=1 bash scripts/gauntlet.sh --gs0NN`); GS-010, GS-014, GS-016 and the chaos injection (chain write or data wipe) get status `manual` with the reason in the description and the full command kept in the runner column for an operator. A seed-file test checks this. Runs without operator approval: every ephemeral or offline runner (GS-020, 023, 025, 026, 027, 028, 029). Needs approval (a) HEAD on the shared local test network and (b) per chain-writing scenario: the plain run and the opt-in rows; until approved they are reported "not run - approval pending", with the note that the four new rules are dormant on that network. One live network at a time.

Whitepaper passages (both files share the structure; EN line numbers, ES within 2 lines):
- §6.1 Block Validity (EN 443, ES "Validez de bloques"): add one validity condition: the header carries the scheduled producer's signature over the signing message, and nodes refuse an unsigned or wrongly signed block, from its activation height.
- §7.4 Bond Lifecycle (EN 590; the paragraph at 600 and the sentence at 602): replace the two-step text with: one `RequestWithdrawal` pays into a single locked unbonding output owned by the producer, spendable after the unbonding period (60,480 blocks on mainnet); while locked it can still be burned by a slash; the bond leaves the active set at the next epoch boundary; FIFO vesting penalties unchanged; from its activation height. Add that Registration, AddBond and RequestWithdrawal are valid only with the named producer's own signature, from their activation height (REQ-LDP-042).
- §11.3 (EN 862-863, ES 860-861): 100% of the bond burned, including withdrawn funds still locked; exclusion at the next epoch boundary; no reward to the reporter; the slashed key can never register again (a new identity needs a new key and a new bond); from its activation height. The "never register again" sentence depends on DC-5 being GREEN or the defect being refuted live.
- Type list (EN 1150, ES 1148): drop `ClaimWithdrawal` from the examples; compare the "27 types" count with `crates/core/src/transaction/types.rs` and correct or flag it (REQ-LDP-048).
- Parameter table (EN 1253, ES 1251): the "Unbonding period" row keeps 60,480 blocks and loses "7-day two-step". Summary list (EN 1281, ES 1279): "two-step withdrawal" -> "one-step locked withdrawal".
Wording limits (REQ-LDP-045): every changed passage contains "activation height" (ES: "altura de activacion", in the file's accent-free spelling), no height number except the unbonding period, no date, no sentence saying the rules are in force on mainnet; ES follows EN meaning. Text check `scripts/test_whitepaper_defense_text.sh`: RED on the current text (two-step, `ClaimWithdrawal`, "Immediate exclusion", "new registration required" present; no signature condition), GREEN after; it also refuses a 4+ digit number other than 60,480 and any `20NN-` date inside the changed passages. The decision row (REQ-LDP-061) states the drift entries stay open until heights are pinned; the report says the published copies outside the repository are unchanged.

━━━ RESOURCE COST — DC-4 ━━━
Dimensions:
  CPU:      N-A (observed)
  Memory:   N-A (observed)
  IO:       N-A (observed)
  Network:  N-A (observed)
  Disk:     +a few KB of SQL, README, whitepaper text and memory rows (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: register the two runners only, leave the opt-in rows and the counting as they are, and write the aligned whitepaper text as a proposal under docs/
Why this proposal anyway: the previous record counted six never-armed scenarios and one run seven times as passes, and a proposal file leaves the retired two-step wording as the published design intent
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### DC-5 Conditional fix: a queued slash wins over a queued or applied withdrawal (resolves c)

Applied only if GS-028 at HEAD confirms the defect. Shape, inside the gated deferred-slash path: (1) `set_lifecycle.rs::slash_producer` refuses only `Slashed` (keeps `[STOR041]` for that case); for `Exited` it applies `info.slash(height)` like `Active`/`Unbonding` (`is_active()` already admits `Unbonding`, `info.rs:170-175`); the admission layer has already proved burnable collateral existed (`slash_state.rs:80-82`, the `locked_collateral` arm), so the ledger mirrors the UTXO truth. (2) `info.rs::apply_withdrawal` auto-exits only when the status is not `Slashed`, so a slash applied earlier in the same boundary or at an earlier boundary is never downgraded.

No new activation height, per the three questions (INV-12): (1) a user-submittable SlashProducer cannot reach `slash_producer` below the gate: its evidence is refused below `effective_slashing_activation_height` (`slash_evidence.rs:32`, `slash_admission.rs:129`), and `slash_producer` has one non-test caller, the deferred Slash arm (`set_core.rs:124`; graph blast radius at 2 hops: `set_core.rs`, `set_lifecycle.rs`); (2) the equivocation handler submits only at or above the same gate (`slash_submit.rs:25`, `block_handling.rs:178`); (3) `apply_withdrawal` differs only when the status is already `Slashed`, which needs a prior applied slash, so below the gate every reachable input is bit-identical. Both gates are `u64::MAX` on mainnet and testnet, so no mined block there can take the new arm. Apply and rebuild share `apply_pending_updates_with_cap`, so parity holds. RED/GREEN unit tests in `crates/storage/src/producer/` (ProducerSet only, no node): (a) queue `RequestWithdrawal` (whole bond) then `Slash`, apply once -> status `Slashed`, `register` of the same key -> `[STOR011]`; (b) withdrawal applied at one boundary (`Exited`), `Slash` queued, next boundary -> `Slashed`; (c) `Slash` then withdrawal in one queue -> stays `Slashed`. (a) and (b) are RED on the parent (`Exited`, `register` Ok). Live GREEN: `gs028-target-status-slashed` PASS. Follow-up: an incident and an invariant row (Level 2), since the rule "a slash is never lost to a status race" has no test today. conf(0.75, observed).

**DC-5 status (INC-I-259):** defect confirmed live (GS-028 at HEAD: `exited` and `[STOR041]` on 6/6 nodes); fix applied in `set_lifecycle.rs` and `info.rs`; unit tests (a), (b), (c) RED then GREEN in `crates/storage/tests/it/inc_i_259_slash_race.rs` (an integration module, since `producer/tests.rs` is over the size limit); live GREEN (`gs028-target-status-slashed` PASS) pending a GS-028 re-run.

━━━ RESOURCE COST — DC-5 ━━━
Dimensions:
  CPU:      +constant: one status match per deferred slash (observed)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: reorder the boundary queue so Slash entries apply before RequestWithdrawal entries
Why this proposal anyway: reordering misses the boundary-between-W-and-S case (the producer is already Exited when the slash is queued) and would still let apply_withdrawal downgrade Slashed to Exited
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## 5. Data flows

- Runner -> `write_chainspec` + `DN_ENV` -> `start_cluster` -> `seed_gset` -> genesis -> blocks; the sampler appends `S <height> <supply>` and `T <node> <height> <hash> <producer>` lines per node.
- `cli_as` (gate env + `--rpc`) -> `sendTransaction` -> mempool (placement, auth, holdings) -> block -> apply -> pending update -> reward-epoch boundary flush (`state_update.rs:181-206`) -> ProducerSet.
- Forge sibling -> gossip (age <= 6 slots) -> detector on observers -> slash tx -> block -> burn of Bond and locked Unbonding outputs -> supply drop -> boundary flush -> status.
- Tx forge -> `sendTransaction` per port -> refusal text -> `SUBMIT` line -> runner regex -> verdict.
- RPC (`getChainInfo`, `getChainStats`, `getBlockByHeight`, `getTransaction`, `getProducers`, `getProducer`, `getUtxos`, `getSlotSchedule`, `getStateRootDebug`, `getMempoolTransactions`) -> `gauntlet-devnet.py` -> `res` lines -> `verdict; exit $?` -> `gauntlet_runs` row.

## 6. Failure modes

| # | Failure | Detection | Response |
|---|---|---|---|
| F1 (a) | Library refactor changes GS-027 behaviour or sources with side effects | GS-027 self-test (`:136-145`), then a live GS-027 run | Run the pre-refactor runner at HEAD to separate refactor from code fault; fix the library, never the assertions |
| F2 (b) | Withdrawal mined after X's last slot of the epoch, or sibling misses the 6-slot window | W > 40; no X block before 45; no `INJECTED` or detection line | Retry next epoch (<= 2); up to 3 sibling attempts; fallback block before W; then SKIP, never FAIL |
| F3 (b) | removed(h) undefined (input amount missing, sampler gap) | helper exit code | Resolve via outpoint; cross-check `[SLASH] burned`; still undefined -> FAIL, never PASS |
| F4 (b) | T has no spendable coin for the probe fee | `getUtxos(T)` empty | Wait one epoch; then SKIP "third party unfunded" |
| F5 (c) | Target ends `Exited`, slash status lost | `gs028-target-status-slashed` FAIL + `[STOR041]` log | P1: DC-5 with RED/GREEN; the whitepaper sentence waits |
| F6 (c) | DC-5 changes a reachable mainnet/testnet path | three-question check, gate grep | Gates are `u64::MAX` and the only caller is gated; otherwise an activation height is required and the run stops |
| F7 (d) | Forge payout shape wrong -> `ERRTX-UNBOND...` instead of PAUTH001; runner matches `data.error_code` | `SUBMIT code=`; companion fixture | Unbonding to `bond_owner(V)`, lock >= best+period+20 at HEAD, Normal on the control; match message/`data.reason` only |
| F8 (d) | Control slash refused (boundary before S) or precondition unmet | S = none; declared set | FAIL with values, conclusive by the W:0 spendable output; precondition unmet -> exit 2, re-run, never exit 1 |
| F9 (e) | Old binary has no `producer_gset.bin` or another format | `seed_gset` wait, `gset_ready` | Check the old source first; raise `GS029_GENESIS_DELAY` (<= 400 s) for gossip; SKIP with diagnosis |
| F10 (e) | HEAD CLI cannot talk to the old node's RPC for the transfer | CLI error text | FAIL `gs029-mixed-txs-mined` with the text (a real compatibility finding, not a harness fault) |
| F11 (e) | Phase-1 tx near H refused (Normal payout at/above H) | mined height >= H-16 | Deadlines keep all txs < 56; overrun -> SKIP |
| F12 (e) | Restart rebuilds epoch state, snap-syncs or double-produces | log tokens | FAIL `gs029-restart-same-datadir` (INC-I-054 class); >= 2 slots between stop and start |
| F13 (e) | Fork heals between samples; negative control stalls without a fork | sampled tips vs final chain; stall rule | `gs029-old-blocks-canonical` uses every sampled tip; stall -> inconclusive, exit 2, re-run |
| F14 (e) | `--kind unsigned` forge never injects (production not paused) | no `INJECTED` line | FAIL, not a vacuous PASS; the forge targets the next slot of a paused HEAD producer |
| F15 (f) | Token missing in `gauntlet.sh`; a control row with `expect_fail=1` | companion skip-arm and seed-file tests | Tokens and rows land in one commit; controls run through env only |
| F16 (g) | Whitepaper names a height or date or claims mainnet activation | text check | RED; edit until GREEN in EN and ES |
| F17 | Port collision, a build running, or wall time over budget | `check_ports`, `build_running`, phase deadlines | SKIP; one cluster at a time; cut order REQ-LDP-052, Could, Should; M1 Musts never cut |

No protection mechanism (rate limit, backoff, watchdog) is added to the node; the runner's retries and deadlines are harness-side and interact with none of the registered protection surface.

## 7. Security budget (trust boundaries TB-1..TB-5 of the requirements)

| Boundary | Attack vector | Concrete mitigation |
|---|---|---|
| TB-1 RPC replies | Crafted JSON reaching a shell command or arithmetic | `python3 -I` parses; bash checks `case '' | *[!0-9]*` for digits and `[0-9a-f]{64}` for hashes before use; no `eval`; a parse error is FAIL or SKIP, never PASS (companion tests feed malformed input) |
| TB-2 gate env | A foreign `.env` or stale variable moves a CLI gate | `load` reads the process env only; `cli_as` sets every gate explicitly; the REQ-LDP-002 test proves mainnet and testnet ignore the variables |
| TB-3 wallets | Key material leaks to logs or stdout | Forge reads only the given path and prints no wallet field; the companion test scans for secret prints with a planted control |
| TB-4 processes and files | Signalling a foreign process, deleting a foreign dir, touching the shared fleet | `alive && ours` on the own scratch path before any signal; cleanup only under `*/"$DN_TAG".*`; the `unsafe()` scan forbids pkill/killall/ssh/systemctl/launchctl and writes under the fleet or user devnet dirs |
| TB-5 whitepaper | Public text over-claims activation | Text check forbids heights, dates and mainnet claims in the changed passages |
| Forge | Reaching a non-devnet node | Host fixed to loopback; `getNetworkParams.network == devnet` on every port before any send |

## 8. Performance and time budget per run

| Run | Heights | Wall time | Cap |
|---|---|---|---|
| GS-028 HEAD and control | about 31 + 1-2 epochs + 6 = 52-66 | 12-15 min each (+2.5 min per retry epoch) | `DN_PHASE_MAX` scaled to `(H - h) x slot x 1.5`, waits in segments |
| GS-029 HEAD | about 90 | 16-20 min | same |
| GS-029 negative | until a fork is stable over 3 samples, at most 40 | 7-10 min | same |
| GS-026, GS-027 regressions | as today | about 10 min each | unchanged |
| Companion tests | offline | < 15 s each | - |

## 9. Degradation (SKIP policy)

SKIP (exit 2) only for an unmet precondition: tools, binaries, busy ports, a running build, an incomplete G-Set, an unfunded third party, the evidence window missed after the retries, a phase deadline overrun in HEAD mode, or an inconclusive control. Everything measured after the preconditions hold is PASS or FAIL. A SKIP is never counted as a pass, is re-run, and is reported with its reason. When GS-029 cannot seed the old binary, the report says GS-029 was not run and why; it does not fall back to an all-HEAD cluster.

## 10. Traceability (REQ -> component -> assertion or test)

| REQ | Component | Assertion / test |
|---|---|---|
| REQ-LDP-001 | DC-1 | child-process test (A), wiring test (B), RED on parent |
| REQ-LDP-002 | DC-1 | test (A) mainnet/testnet byte-identical |
| REQ-LDP-003, 030, 032 | DC-2 | `inc_i_182_m3_forge_shape.rs`; `gs028-forced-exit-refused` input; wrong-tail line |
| REQ-LDP-004 | DC-2 | guard tests (non-devnet, non-loopback, no secret print) |
| REQ-LDP-005, 014 | DC-3 | precondition SKIP lines; env-selectable binaries; header role line |
| REQ-LDP-006 | DC-3 | `gs028-withdrawal-before-evidence` |
| REQ-LDP-007 | DC-3 | `gs028-one-slash-mined`, `gs028-target-status-slashed` |
| REQ-LDP-008 | DC-3 | `gs028-slash-burns-locked` |
| REQ-LDP-009 | DC-3 | `gs028-full-bond-leaves-supply` |
| REQ-LDP-010 | DC-3 | `gs028-producer-gets-nothing` |
| REQ-LDP-011 | DC-2, DC-3 | `gs028-forced-exit-refused` |
| REQ-LDP-012 | DC-3 | `gs028-tips-equal`, `gs028-production-continues` |
| REQ-LDP-013, 021 | DC-3 | control declarations, conclusive rule, run record |
| REQ-LDP-015 | DC-3 | `gs029-mixed-txs-mined`, `gs029-old-producer-built` |
| REQ-LDP-016 | DC-3 | `gs029-mixed-producer-set-equal` |
| REQ-LDP-017 | DC-3 | `gs029-mixed-no-fork`, `gs029-mixed-state-equal`, `gs029-old-blocks-canonical` |
| REQ-LDP-018 | DC-3 | `gs029-restart-same-datadir` |
| REQ-LDP-019 | DC-3, block forge | `gs029-crossing-no-fork`, `gs029-unsigned-tx-refused`, `gs029-unsigned-block-refused`, `gs029-signed-tx-mined` |
| REQ-LDP-020 | DC-3 | negative-control run record, fork/stall rule |
| REQ-LDP-022, 023, 029 | DC-3 | `test_gauntlet_gs028.sh`, `test_gauntlet_gs029.sh` (contract, malformed input, helpers) |
| REQ-LDP-024 | all M1 files | `wc -l` at the milestone gate |
| REQ-LDP-025 | DC-1 | GS-026 and GS-027 live runs |
| REQ-LDP-026 | DC-4 | `gauntlet_runs` rows, ids in the report |
| REQ-LDP-027 | DC-3 | `gs029-post-gate-unbonding` |
| REQ-LDP-028, 031, 033 | DC-3 | METRIC fields |
| REQ-LDP-034, 035 | - | deferred (Won't) |
| REQ-LDP-040..044, 048 | DC-4 | text check per passage; type-count comparison with `transaction/types.rs` |
| REQ-LDP-045, 046 | DC-4 | `scripts/test_whitepaper_defense_text.sh` RED then GREEN (no heights, no dates, no mainnet claim, EN/ES parity) |
| REQ-LDP-047, 052, 053 | report | final report sections |
| REQ-LDP-050, 051, 054 | DC-4 | seed-file test; run report ran/passed/skipped; README bullets and Quick Reference rows |
| REQ-LDP-060, 061, 062 | DC-4 | row queries (scenario rows, decision row, incident links) |

## 11. Design decision hypotheses

| Decision | Winner | conf | Eliminated alternatives |
|---|---|---|---|
| Helper sharing | Neutral sourced library + Python helper (DC-1) | 0.8 observed | guard move (copies three functions); runner-only lib (copies 140 lines); Python-only runner (PID safety stays in bash); Rust harness (loses the contract) |
| GS-028 order | Withdraw first, then sibling of X's first block above W (DC-3) | 0.7 observed | X-before-W as primary (about 50 s window minus CLI and mining latency); evidence after exit as default (proves only the locked-collateral arm) |
| Refusal signal | `error.message`/`data.reason` regex (DC-2) | 0.7 observed | `data.error_code` (generic); node log only (racy) |
| Defect handling | Prove live, then DC-5 inside the gated path | 0.75 observed | queue reordering (misses the two-boundary case); new activation height (unreachable paths need none) |
| Gauntlet honesty | Dedupe by runner text, manual status for chain writers (DC-4) | 0.65 observed | expect_fail controls (count SKIP as pass); unregistering the opt-ins (hides them) |

Quality audit: 5 major decisions; 3-5 alternatives each; bases observed 5, inferred 0, assumed 0; winner confidence 0.65-0.8; subtraction alternatives explored 2 (guard move, extend GS-026/027); "do nothing" explored 1 (in-process tests only); winner complexity cost: 1 library, 1 helper, 2 runners, 2 companion tests, 1 example, 1 text check; closest simpler alternative: extend GS-027 at conf 0.5 (line limit). Constraint table entries used: 9.

## Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Dependencies | Status |
|----|------|-----------------|----------------------|--------------|--------|
| M1 | Live defense scenarios GS-028 and GS-029 | `scripts/gauntlet-devnet-lib.sh`, `scripts/gauntlet-devnet.py`, `scripts/gauntlet-gs027.sh` (refactor), `scripts/test_gauntlet_gs027.sh`, `scripts/gauntlet-gs028.sh`, `scripts/test_gauntlet_gs028.sh`, `scripts/gauntlet-gs029.sh`, `scripts/test_gauntlet_gs029.sh`, `bins/node/examples/gs028_forge_tx/`, `bins/node/tests/it/inc_i_182_m3_forge_shape.rs`, `bins/cli/src/unbonding_tx.rs`, `bins/cli/src/cmd_producer/{withdrawal,exit}.rs`, CLI tests; conditional `crates/storage/src/producer/{set_lifecycle,info}.rs` | REQ-LDP-001, REQ-LDP-002, REQ-LDP-003, REQ-LDP-004, REQ-LDP-005, REQ-LDP-006, REQ-LDP-007, REQ-LDP-008, REQ-LDP-009, REQ-LDP-010, REQ-LDP-011, REQ-LDP-012, REQ-LDP-013, REQ-LDP-014, REQ-LDP-015, REQ-LDP-016, REQ-LDP-017, REQ-LDP-018, REQ-LDP-019, REQ-LDP-020, REQ-LDP-021, REQ-LDP-022, REQ-LDP-023, REQ-LDP-024, REQ-LDP-025, REQ-LDP-026, REQ-LDP-027, REQ-LDP-028, REQ-LDP-029, REQ-LDP-030, REQ-LDP-031, REQ-LDP-032, REQ-LDP-033, REQ-LDP-034, REQ-LDP-035 | Built binaries: HEAD node, CLI and forges; v6.44.2 node and CLI; pre-unbonding node and CLI | PENDING |
| M2 | Whitepaper alignment, gauntlet honesty, records | `WHITEPAPER.md`, `WHITEPAPER-es.md`, `scripts/test_whitepaper_defense_text.sh`, `scripts/gauntlet.sh`, `scripts/gauntlet-seed.sql`, `scripts/README.md`, `specs/SPECS.md`, memory rows | REQ-LDP-040, REQ-LDP-041, REQ-LDP-042, REQ-LDP-043, REQ-LDP-044, REQ-LDP-045, REQ-LDP-046, REQ-LDP-047, REQ-LDP-048, REQ-LDP-050, REQ-LDP-051, REQ-LDP-052, REQ-LDP-053, REQ-LDP-054, REQ-LDP-060, REQ-LDP-061, REQ-LDP-062 | M1 (run results feed the records; the §11.3 lifetime-bar sentence waits for DC-5 GREEN or a live refutation) | PENDING |

Sizing note: M1 is at the limit (two large runners count double); the library and helper are built first and proven by the GS-027 regression before either new runner starts, so each later step touches at most two modules.
