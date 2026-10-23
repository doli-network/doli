# Requirements: Live equivocation-defense proof and whitepaper alignment

## Scope

- **Gauntlet runners (new):** `scripts/gauntlet-gs028.sh`, `scripts/gauntlet-gs029.sh`, companion tests under `scripts/`. Helpers come from `scripts/gauntlet-gs027.sh` (`GS027_LIB_ONLY=1`) or a shared lib file; no copy-paste of the GS-027 cluster code.
- **Tooling:** a small transaction-forge example under `bins/node/examples/` (or a new `--kind` of `gs026_forge_block`); the CLI gate source in `bins/cli/src/cmd_producer/withdrawal.rs` and `bins/cli/src/cmd_producer/exit.rs`.
- **Docs and records:** `WHITEPAPER.md`, `WHITEPAPER-es.md`, `scripts/gauntlet-seed.sql`, `scripts/README.md` (gauntlet section), `specs/SPECS.md`, and the gauntlet rows in the institutional memory.
- **Not touched:** any consensus, validation, mempool, storage or apply code, unless a scenario finds a defect and a RED/GREEN test proves it.

## Summary (plain language)

Four protections are now in the code, each switched on at its own activation height: blocks must carry the producer's signature, a producer that signs two blocks for one slot loses its whole bond, withdrawn bond money stays locked (and can still be burned) for an unbonding period, and only the producer itself can change its own bonds. In-process tests prove each rule alone. This work proves them together on a small live local network. Scenario GS-028 shows that a cheating producer cannot save its bond by withdrawing first, and that nobody else can push a producer out. Scenario GS-029 shows that nodes on the current mainnet release (v6.44.2) and nodes on the new code agree on every block while real producer transactions flow, and that the network crosses the activation height without a split after the upgrade. Then the whitepaper is corrected so it describes these rules, each one "from its activation height".

## Trust Boundaries

| # | Where external data enters | What consumes it | Guard |
|---|---|---|---|
| TB-1 | JSON-RPC replies from the runner's own local nodes (hashes, keys, heights, amounts) | runner shell and Python, forge-tool arguments | REQ-LDP-023 |
| TB-2 | Process environment (`DOLI_INC_I_*` gate overrides) | CLI withdrawal and exit builders | REQ-LDP-002 |
| TB-3 | Throwaway wallet files in the runner's scratch dir | forge tool (signing), CLI | REQ-LDP-004 |
| TB-4 | Process table and file system (PIDs, scratch dirs, ports) | runner start/stop/cleanup | REQ-LDP-023 |
| TB-5 | Whitepaper text (public) | readers, published copies | REQ-LDP-045 |

Trust model (assumed, see A-1): every value read from RPC is untrusted for parsing; every node in a scenario is a throwaway local devnet node; no input reaches mainnet or the shared local test network unless an operator approves it (REQ-LDP-052).

## User Stories

- **US-1** As a release owner, I want a live run that shows an equivocating producer cannot keep withdrawn funds, so that I can pin the slashing and unbonding heights on evidence.
- **US-2** As a producer, I want proof that no third party can withdraw my bonds or exit me, so that my stake is safe after the authorization height.
- **US-3** As a release owner, I want proof that v6.44.2 and the new code agree block by block while producer transactions flow, so that a rolling upgrade cannot split the network.
- **US-4** As a devnet tester, I want the CLI to build the transaction shape the node expects at the current height, so that devnet runs with finite gates do not fail for a tooling reason.
- **US-5** As a reader of the whitepaper, I want it to describe the rules the code enforces and when they apply, so that I do not rely on retired behaviour.
- **US-6** As a gauntlet reviewer, I want each scenario reported as ran, passed or skipped, so that a skipped scenario is never read as a pass.
- **US-7** As a maintainer, I want the scenarios, runs and decisions recorded, so that later runs can find them.

## Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Dependencies | Status |
|----|------|-----------------|----------------------|--------------|--------|
| M1 | Live defense scenarios GS-028 and GS-029 | `scripts/gauntlet-gs028.sh`, `scripts/gauntlet-gs029.sh`, runner tests, tx-forge example, `bins/cli/src/cmd_producer/{withdrawal,exit}.rs`, CLI test | REQ-LDP-001, REQ-LDP-002, REQ-LDP-003, REQ-LDP-004, REQ-LDP-005, REQ-LDP-006, REQ-LDP-007, REQ-LDP-008, REQ-LDP-009, REQ-LDP-010, REQ-LDP-011, REQ-LDP-012, REQ-LDP-013, REQ-LDP-014, REQ-LDP-015, REQ-LDP-016, REQ-LDP-017, REQ-LDP-018, REQ-LDP-019, REQ-LDP-020, REQ-LDP-021, REQ-LDP-022, REQ-LDP-023, REQ-LDP-024, REQ-LDP-025, REQ-LDP-026, REQ-LDP-027, REQ-LDP-028, REQ-LDP-029, REQ-LDP-030, REQ-LDP-031, REQ-LDP-032, REQ-LDP-033, REQ-LDP-034, REQ-LDP-035 | Built binaries: HEAD node, CLI and forge; v6.44.2 node and CLI; pre-unbonding node and CLI | Planned |
| M2 | Whitepaper alignment, gauntlet honesty, records | `WHITEPAPER.md`, `WHITEPAPER-es.md`, whitepaper text check, `scripts/gauntlet-seed.sql`, `scripts/README.md`, memory rows | REQ-LDP-040, REQ-LDP-041, REQ-LDP-042, REQ-LDP-043, REQ-LDP-044, REQ-LDP-045, REQ-LDP-046, REQ-LDP-047, REQ-LDP-048, REQ-LDP-050, REQ-LDP-051, REQ-LDP-052, REQ-LDP-053, REQ-LDP-054, REQ-LDP-060, REQ-LDP-061, REQ-LDP-062 | M1 (run results feed the records; GS-028/029 runners must exist before registration) | Planned |

## Requirements

Security requirements carry the tag **SEC** and are all Must.

| ID | Requirement | Priority | Story | MS |
|----|------------|----------|-------|----|
| REQ-LDP-001 | The CLI withdrawal and exit commands take the gate heights and unbonding period from the same parameter loader the node uses, so devnet env overrides apply | Must | US-4 | M1 |
| REQ-LDP-002 | **SEC** The CLI change has no effect on mainnet or testnet: env vars cannot move a gate there | Must | US-4 | M1 |
| REQ-LDP-003 | A forge tool builds and submits a RequestWithdrawal that names another producer, paid and signed only by a third party | Must | US-2 | M1 |
| REQ-LDP-004 | **SEC** The forge tool works only on devnet and loopback, reads only key files it is given, and never prints key material | Must | US-2 | M1 |
| REQ-LDP-005 | GS-028 cluster: one finite height for all four gates, enough producers to stay above the floor, selectable node and CLI builds | Must | US-1 | M1 |
| REQ-LDP-006 | GS-028 sequence: the target's whole-bond withdrawal is mined before the equivocation evidence reaches the network | Must | US-1 | M1 |
| REQ-LDP-007 | GS-028: exactly one slash for the target is mined, at one height, on every node | Must | US-1 | M1 |
| REQ-LDP-008 | GS-028: the slash block burns exactly the locked unbonding amount | Must | US-1 | M1 |
| REQ-LDP-009 | GS-028: the withdrawal block and the slash block together remove the whole bond from supply | Must | US-1 | M1 |
| REQ-LDP-010 | GS-028: the target never holds a spendable output from its withdrawal, and the locked output is gone after the slash | Must | US-1 | M1 |
| REQ-LDP-011 | GS-028: a third-party forced exit is refused by every node for the authorization reason and never mined | Must | US-2 | M1 |
| REQ-LDP-012 | GS-028: tips stay equal and production continues after the slash | Must | US-1 | M1 |
| REQ-LDP-013 | GS-028 control on the pre-unbonding build (`f737cc2d`) fails on the funds assertions with the precondition met | Must | US-1 | M1 |
| REQ-LDP-014 | GS-029 cluster: v6.44.2 and HEAD nodes in one devnet, a v6.44.2 genesis producer, one gate height H | Must | US-3 | M1 |
| REQ-LDP-015 | GS-029 phase 1: the HEAD CLI sends transfer, Registration, AddBond and RequestWithdrawal below H; all are mined | Must | US-3 | M1 |
| REQ-LDP-016 | GS-029: an epoch boundary applies the producer changes while v6.44.2 still runs, with one producer set everywhere | Must | US-3 | M1 |
| REQ-LDP-017 | GS-029: block history, supply and state root agree across versions; every v6.44.2-built block stays canonical | Must | US-3 | M1 |
| REQ-LDP-018 | GS-029 phase 2: the v6.44.2 nodes restart on HEAD with their own data dirs before H and rejoin | Must | US-3 | M1 |
| REQ-LDP-019 | GS-029 phase 3: the network crosses H with no fork; unsigned tx and block refused, signed tx mined | Must | US-3 | M1 |
| REQ-LDP-020 | GS-029 negative control: gates below the mixed phase produce a FAIL on the named fork assertion | Must | US-3 | M1 |
| REQ-LDP-021 | A control run counts only when its preconditions pass and the expected assertion fails for the expected reason | Must | US-1, US-3 | M1 |
| REQ-LDP-022 | Both runners keep the gauntlet output contract and exit codes | Must | US-6 | M1 |
| REQ-LDP-023 | **SEC** Runner safety: own PIDs and scratch dirs only, validated RPC values, no shell evaluation of node output | Must | US-6 | M1 |
| REQ-LDP-024 | No new source, runner or tool file exceeds 500 lines (tests 800); files already over the budget before this work are recorded as modularity debt | Must | US-6 | M1 |
| REQ-LDP-025 | GS-026 and GS-027 still PASS at HEAD | Must | US-1 | M1 |
| REQ-LDP-026 | Each M1 run (GS-028 control and HEAD, GS-029 HEAD and negative control, GS-026/027 regressions) gets a gauntlet run record | Must | US-7 | M1 |
| REQ-LDP-027 | GS-029: after H, a HEAD-CLI withdrawal pays into a locked unbonding output | Should | US-3 | M1 |
| REQ-LDP-028 | GS-029: a v6.44.2-built block carries a signed producer transaction (reported metric) | Should | US-3 | M1 |
| REQ-LDP-029 | Companion tests cover the pure runner helpers (supply arithmetic, normalisation, hash history) | Should | US-6 | M1 |
| REQ-LDP-030 | A test shows the forge tool's forced-exit transaction matches the in-process fixture shape | Should | US-2 | M1 |
| REQ-LDP-031 | GS-029: the newly registered producer produces at least one block after activation | Could | US-3 | M1 |
| REQ-LDP-032 | Forge tool: wrong-key variant (tail signed by the third party) is also refused | Could | US-2 | M1 |
| REQ-LDP-033 | GS-029: per-slot tip sampling reports transient divergence as a metric | Could | US-3 | M1 |
| REQ-LDP-034 | Live RED of the forced exit on a pre-fix build (needs bonds at a non-derived address) | Won't | US-2 | M1 |
| REQ-LDP-035 | Staggered gate heights and the pending-root crossing inside GS-029 | Won't | US-3 | M1 |
| REQ-LDP-040 | Whitepaper §6.1: a block is valid only with the scheduled producer's signature, from its activation height | Must | US-5 | M2 |
| REQ-LDP-041 | Whitepaper §7.4: one-step withdrawal into a locked unbonding output that stays slashable | Must | US-5 | M2 |
| REQ-LDP-042 | Whitepaper: producer-changing transactions need the producer's own signature | Must | US-5 | M2 |
| REQ-LDP-043 | Whitepaper §11.3: full burn including locked funds, exclusion at the next epoch boundary, no reporter reward, key barred for life | Must | US-5 | M2 |
| REQ-LDP-044 | Whitepaper: retired wording removed elsewhere (§19.4 type list, parameter table, summary list) | Must | US-5 | M2 |
| REQ-LDP-045 | **SEC** Whitepaper wording limits: "from its activation height", no height, no date, no claim of mainnet activation; EN and ES equal in meaning | Must | US-5 | M2 |
| REQ-LDP-046 | A committed text check over both whitepaper files, RED before the edit and GREEN after | Should | US-5 | M2 |
| REQ-LDP-047 | Report of whitepaper changes (old and new meaning per section) | Must | US-5 | M2 |
| REQ-LDP-048 | Whitepaper transaction-type count checked against the code and corrected or flagged | Could | US-5 | M2 |
| REQ-LDP-050 | Registered runners of opt-in scenarios arm their own injection, or the scenario is marked manual-only with a reason | Should | US-6 | M2 |
| REQ-LDP-051 | The shared `gauntlet.sh` pass counts once; a SKIP is never counted as a pass | Should | US-6 | M2 |
| REQ-LDP-052 | Full registered gauntlet at HEAD with perturbation, on local networks only, behind operator approvals | Should | US-6 | M2 |
| REQ-LDP-053 | **SEC** Gauntlet operation limits during REQ-LDP-052 | Must | US-6 | M2 |
| REQ-LDP-054 | `scripts/README.md` documents GS-028, GS-029 and the corrected opt-in usage | Should | US-6 | M2 |
| REQ-LDP-060 | Scenario rows: GS-028 active with its runner, GS-029 new, in the seed file and in memory | Should | US-7 | M2 |
| REQ-LDP-061 | Decision row for the whitepaper changes | Should | US-7 | M2 |
| REQ-LDP-062 | Incident links INC-I-236, INC-I-255, INC-I-257, INC-I-182 on the new rows | Should | US-7 | M2 |

## Acceptance Criteria

Notation: B = bond amount of the target (read from the chain), U = amount of the withdrawal's single output, W = height of the withdrawal block, S = height of the slash block. issued(h) = new value block h creates (coinbase outputs, read from the chain). removed(h) = supply(h-1) - supply(h) + issued(h) - (input minus output of every other transaction in h). Supply = `getChainStats.totalSupply`, the most frequent sample per height (GS-027 method).

### M1

- **REQ-LDP-001** - [ ] Given a devnet with `DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT=H` in the CLI environment and best height + 1 < H, when the CLI builds a withdrawal or exit, then the payout is Normal and carries the producer-auth tail. - [ ] At best height + 1 >= H the payout is Unbonding with the env-loaded unbonding period. - [ ] A CLI test is RED on the parent commit and GREEN after. - [ ] No other CLI behaviour changes.
- **REQ-LDP-002** - [ ] Given mainnet or testnet and any `DOLI_*` gate variable set, the built transaction is byte-identical to the one built with no variable set (test).
- **REQ-LDP-003** - [ ] Input: victim key, third-party wallet, RPC port(s), payout form (Normal or Unbonding with lock height). - [ ] Output: one RequestWithdrawal whose payload names the victim, whose inputs are the third party's own Normal coins signed by the third party, with no producer-auth tail. - [ ] It submits the same hex to each given node via `sendTransaction` and prints one line per node with the accept/refuse result and the error text.
- **REQ-LDP-004** - [ ] Refuses a non-devnet chain and any non-loopback address (exit non-zero, nothing sent). - [ ] Reads only the wallet paths passed in. - [ ] No private key or seed appears in its output or logs.
- **REQ-LDP-005** - [ ] At least 6 nodes and 5 genesis producers (target, victim, third party, two more), so at least 3 remain if two leave. - [ ] All four gates at one height H via the devnet env vars; pending-root gate 0. - [ ] H is past the bootstrap blocks and the first epoch; the epoch is long enough that the target has at least two slots per epoch. - [ ] Node, CLI and forge binaries are selectable by env so the same runner drives HEAD and the control.
- **REQ-LDP-006** - [ ] After H, the runner takes a block by the target held by every node, submits the target's whole-bond withdrawal with the CLI of the build under test, waits until it is mined at W on every node, then publishes a sibling of that target block no later than 5 slots after its slot. - [ ] If the window is missed, it retries with the target's next block in the same epoch; otherwise SKIP with the reason. - [ ] Assertion `gs028-withdrawal-before-evidence` PASS iff the withdrawal is at the same W on every node, has one output, W is before the sibling publication, and (HEAD) the output's lock height is above S.
- **REQ-LDP-007** - [ ] `gs028-one-slash-mined`: every node holds exactly one slash for the target, at the same S and with the same tx id, S > W. - [ ] The target's status is identical on every node and not active.
- **REQ-LDP-008** - [ ] `gs028-slash-burns-locked`: removed(S) == U on every node.
- **REQ-LDP-009** - [ ] `gs028-full-bond-leaves-supply`: removed(W) + removed(S) == B on every node.
- **REQ-LDP-010** - [ ] `gs028-producer-gets-nothing`: at every sample from W on, the withdrawal's output is never a spendable type; after S it is absent from the target's UTXOs and the target's unbonding balance is 0 on every node.
- **REQ-LDP-011** - [ ] `gs028-forced-exit-refused`: after H, the forge tool's forced exit against the victim is refused by every node with error code `ERRTX-PAUTH001` (RPC error text, or the node log if the RPC drops the code). - [ ] It is in no block over the next epoch. - [ ] The victim's status and bond count are unchanged after the next boundary. - [ ] The measured line prints `code=` and `mined=` per node.
- **REQ-LDP-012** - [ ] `gs028-tips-equal`: one hash at the common tip. - [ ] `gs028-production-continues`: common tip >= S + 6.
- **REQ-LDP-013** - [ ] On the pre-unbonding node and CLI (`f737cc2d`), `gs028-withdrawal-before-evidence` PASS and at least one of `gs028-slash-burns-locked`, `gs028-full-bond-leaves-supply`, `gs028-producer-gets-nothing` FAIL, with measured values that show the payout survived. - [ ] `gs028-forced-exit-refused` FAIL on the control is expected (no auth code exists there); its line must show `mined=0` unless the attempt was really mined.
- **REQ-LDP-014** - [ ] At least 6 nodes; at least 2 on v6.44.2, one of them a genesis producer; at least 4 genesis producers, each with the one bond that genesis completion backs (the chainspec `bond_count` is not honoured at genesis completion, so no genesis producer can start with two bonds; the first draft of this criterion assumed it could). - [ ] G-Set seeding as in GS-027; SKIP (not FAIL) if incomplete. - [ ] All four gates = H; pending-root gate 0.
- **REQ-LDP-015** - [ ] `gs029-mixed-txs-mined`: a transfer, a Registration (fresh wallet funded by the transfer), an AddBond and a RequestWithdrawal from the HEAD CLI are each in a block below H on every node (the withdrawal takes the producer's single genesis bond, so that producer exits at the next boundary; a partial withdrawal would need a second bond and a further boundary while the old nodes still run, which is recorded as a follow-up, not required here). - [ ] At least one of them is submitted through a v6.44.2 node's RPC. - [ ] The withdrawal's output is Normal; the HEAD CLI appends the 64-byte producer-signature tail to every producer tx (CLI unit tests), and the old producer mining those txs is the live proof that the old decoders accept the tail. - [ ] `gs029-old-producer-built`: at least one block in phase 1 was built by the v6.44.2 producer.
- **REQ-LDP-016** - [ ] `gs029-mixed-producer-set-equal`: after the first boundary that follows those txs, with v6.44.2 still running, the normalised producer set (key, status, bond count) is identical on every node and shows the registration, the added bond and the withdrawal.
- **REQ-LDP-017** - [ ] `gs029-mixed-no-fork`: after phase 1, the block hash at every height of phase 1 is identical on every node. - [ ] `gs029-mixed-state-equal`: supply equal at each sampled height; `getStateRootDebug` state root equal whenever all nodes report the same tip (component hashes printed on mismatch). - [ ] `gs029-old-blocks-canonical`: every block a v6.44.2 node logged as produced is on the final common chain.
- **REQ-LDP-018** - [ ] `gs029-restart-same-datadir`: the v6.44.2 nodes are stopped by PID and started on HEAD with the same data dirs before H. - [ ] Each resumes at or above its stop height without snap sync or a resync from genesis, and reaches the common tip. - [ ] At least one epoch boundary falls after the restart.
- **REQ-LDP-019** - [ ] `gs029-crossing-no-fork`: identical hash at every height from the restart to H + 10. - [ ] `gs029-unsigned-tx-refused`: an AddBond from the v6.44.2 CLI is refused by every node with `ERRTX-PAUTH001` and never mined. - [ ] `gs029-unsigned-block-refused`: a forge `--kind unsigned` block is in no node's chain. - [ ] `gs029-signed-tx-mined`: an AddBond from the HEAD CLI is mined.
- **REQ-LDP-020** - [ ] With the HEAD nodes' gates set below the start of the mixed phase, the run exits 1 and `gs029-mixed-no-fork` or `gs029-old-blocks-canonical` FAILs with the first differing height or the orphaned block named. - [ ] A SKIP of the control is re-run, never reported as its result.
- **REQ-LDP-021** - [ ] Each control declares its expected-FAIL assertions before it runs. - [ ] A control is conclusive only if its precondition assertions PASS and an expected assertion FAILs with matching measured values; any other result is reported as inconclusive.
- **REQ-LDP-022** - [ ] One `PASS|FAIL <id> <k=v>` line per assertion, a `METRIC` last line, exit 0 all PASS, 1 any FAIL, 2 SKIP with `SKIP gs02N: <reason>`. - [ ] A SKIP is used only for an unmet precondition. - [ ] Exit codes are read directly, never through a pipe.
- **REQ-LDP-023** - [ ] Signals only PIDs it started whose command line names its own scratch dir; removes only that dir; checks ports before start. - [ ] No pkill, killall, ssh, systemctl or launchctl; never touches the shared local test network. - [ ] Every RPC-derived value passed to a command is checked (hex or digits) first; unparsable JSON yields FAIL or SKIP, never PASS; no `eval`.
- **REQ-LDP-024** - [ ] `wc -l` of every NEW runner, lib, example and source file <= 500; test files <= 800. - [ ] A changed file that was already over the budget before this work (`scripts/gauntlet.sh`, `crates/storage/src/producer/info.rs`) grows only by the lines its change needs and is recorded as modularity debt for a later split. - [ ] `scripts/gauntlet-gs026.sh` does not grow.
- **REQ-LDP-025** - [ ] Both runners exit 0 at HEAD with all assertions PASS.
- **REQ-LDP-026** - [ ] One `gauntlet_runs` row per run with git SHA, status and the runner's assertion lines in `failures`; ids appear in the report.
- **REQ-LDP-027** - [ ] `gs029-post-gate-unbonding`: a withdrawal after H has one Unbonding output with a lock height above its block.
- **REQ-LDP-028** - [ ] METRIC reports how many producer txs were in v6.44.2-built blocks; 0 is reported, not failed.
- **REQ-LDP-029** - [ ] Tests feed fixed JSON to the helpers and assert removed(h), normalisation and hash-history verdicts, including a malformed-input case.
- **REQ-LDP-030** - [ ] Test compares the tool's built transaction fields with the `forced_exit()` fixture.
- **REQ-LDP-031** - [ ] One node runs the new producer's key; the chain holds a block by it after its activation delay.
- **REQ-LDP-032** - [ ] A forced exit with a tail signed by the third party's key is refused with `ERRTX-PAUTH001` on every node.
- **REQ-LDP-033** - [ ] METRIC reports the longest run of slots with differing tips; no verdict.
- **REQ-LDP-034, REQ-LDP-035** - N/A (deferred; see Out of Scope).

### M2

- **REQ-LDP-040** - [ ] §6.1 (EN, ES) lists the producer signature as a validity condition that applies from its activation height.
- **REQ-LDP-041** - [ ] §7.4 (EN, ES): no claim step; the withdrawal pays into one locked output spendable after the unbonding period (60,480 blocks on mainnet); while locked it can be burned by a slash; the bond leaves the active set at the next epoch boundary; vesting penalties unchanged.
- **REQ-LDP-042** - [ ] EN and ES state that Registration, AddBond and RequestWithdrawal are valid only with the named producer's own signature, from their activation height.
- **REQ-LDP-043** - [ ] §11.3 (EN, ES): 100% of the bond burned, including withdrawn funds still locked; exclusion at the next epoch boundary; no reward to the reporter; the slashed key can never register again (a new identity needs a new key and a new bond).
- **REQ-LDP-044** - [ ] Both files: `ClaimWithdrawal` is not listed as an available type; the parameter table and the summary list no longer describe a two-step withdrawal.
- **REQ-LDP-045** - [ ] Every changed passage contains "activation height" (ES: the file's own term). - [ ] No height number and no date in changed text. - [ ] No sentence says these rules are in force on mainnet. - [ ] ES follows the EN meaning and the ES file's spelling convention. - [ ] Published copies outside this repository are not changed.
- **REQ-LDP-046** - [ ] The check fails on the current text (retired phrases present) and passes after the edit, over EN and ES.
- **REQ-LDP-047** - [ ] The report lists each changed section with old and new meaning, states that no whitepaper text test existed before, and reminds the operator that the published copies still need a separate update.
- **REQ-LDP-048** - [ ] The "27 types" claim is compared with `crates/core/src/transaction/types.rs`; corrected if wrong, or flagged with the reason.
- **REQ-LDP-050** - [ ] Each opt-in scenario (GS-009, GS-010, GS-014, GS-016, GS-019, GS-022, and the chaos injection) has a registered runner that includes its flag and its confirm variable, or a non-active status with the reason in its description. - [ ] Seed file and memory rows agree. - [ ] A test reads the seed file and checks this.
- **REQ-LDP-051** - [ ] The scenarios that share the plain `gauntlet.sh` run are executed once and counted once. - [ ] Per-scenario results come from that run's per-scenario lines. - [ ] `scenarios_passed` excludes every scenario whose line is SKIP; the report gives ran, passed and skipped per scenario.
- **REQ-LDP-052** - [ ] Runs only after the operator approves (a) installing the HEAD node binary on the shared local test network and (b) each scenario that writes to a chain or wipes a node's data. - [ ] Unapproved scenarios are reported "not run - approval pending". - [ ] The report states that the new rules are dormant on that network (its gates are not set). - [ ] Cut first if time runs out.
- **REQ-LDP-053** - [ ] No contact with any mainnet host; no data wipe except the chaos step's own backed-up wipe; no pkill; one live network at a time, started only after every devnet runner PID has exited.
- **REQ-LDP-054** - [ ] The gauntlet section lists both new runners with their env knobs, control modes and output contracts, and the corrected opt-in commands.
- **REQ-LDP-060** - [ ] `scripts/gauntlet-seed.sql` holds GS-028 (status active, runner set) and GS-029; memory rows match; `gauntlet-run.sh` selects both.
- **REQ-LDP-061** - [ ] One active decision row describes the whitepaper changes and says the drift entries stay open until heights are pinned.
- **REQ-LDP-062** - [ ] `incident_ids` of GS-028 and GS-029 include the four incidents; the report gives the row ids.

## Architecture Context

### Module Boundaries

| Module | Responsibility | Depends on | Depended on by |
|---|---|---|---|
| Runners `gauntlet-gs028.sh`, `gauntlet-gs029.sh` | Build a throwaway devnet, drive traffic, measure, print the verdict | node, CLI and forge binaries; GS-027 helpers; RPC | `gauntlet-run.sh` via the seed rows; the report |
| Node binaries (HEAD, v6.44.2 `b4cff010`, pre-unbonding `f737cc2d`) | Consensus, mempool, apply, RPC; gates from env on devnet | `NetworkParams::load` | runners, CLI, forge |
| CLI (HEAD, v6.44.2, pre-unbonding) | Builds and signs producer txs and transfers; submits via RPC | `NetworkParams`, `unbonding_tx`, `producer_tx`, RPC client | runners |
| Block forge example | Sibling, forged and unsigned blocks over gossip | core, network, crypto, vdf | GS-027, GS-028, GS-029 |
| Tx forge (new) | Third-party RequestWithdrawal over RPC | core transaction builders, crypto | GS-028 |
| Seed file and memory rows | Register scenarios and runs | runner paths | `gauntlet-run.sh`, later briefings |
| Whitepaper EN/ES | Public description of the rules | the code (projection) | readers, published copies |

### Data Flows Through Affected Area

- Runner -> chainspec + env gates -> nodes start -> G-Set seeded -> genesis -> blocks.
- CLI -> `sendTransaction` -> mempool (auth, unbonding, holdings checks) -> block -> apply -> pending producer update -> epoch boundary flush -> ProducerSet.
- Forge sibling -> gossip (age limit: 6 slots) -> equivocation detector -> slash tx -> mempool -> block -> slash burn of Bond and locked Unbonding outputs -> supply.
- RPC (`getChainStats`, `getBlockByHeight`, `getTransaction`, `getProducers`, `getStateRootDebug`, `getUtxos`, `getBalance`) -> runner assertions -> verdict -> run record.

### Architectural Constraints & Invariants

- No consensus, validation or apply rule changes unless a scenario proves a defect with a RED/GREEN test; protocol and epoch-state versions stay unchanged (equal in all three trees, so a same-dir restart keeps epoch state).
- Gate env overrides exist on devnet only; mainnet and testnet gates stay compiled (`u64::MAX`). The CLI change must keep that property.
- Unbonding gate <= slashing gate; effective slashing height = max of the slashing, block-signature and pending-root gates.
- Producer mutations apply at the epoch boundary; a slash still lands on an exited producer while its unbonding output is locked.
- Runner safety pattern of GS-026/GS-027 (own PIDs, own scratch dir) is preserved.
- Whitepaper names no height or date; mainnet drift entries stay open until heights are pinned.

### Blast Radius

- Direct: the two CLI call sites (`withdrawal.rs:150`, `exit.rs:170`); the code graph shows their only callers are `cmd_producer/dispatch.rs`, and the builders (`unbonding_tx.rs`) take params as an argument, so the existing CLI tests that pass params are unaffected.
- Indirect: devnet scripts that set gate env vars now get CLI transactions that match the node; GUI and wallet-crate builders are not changed (out of scope).
- Runners, examples, docs, seed: no runtime consumer except the gauntlet harness and readers.

## Impact Analysis

- `bins/cli/src/cmd_producer/withdrawal.rs`, `exit.rs`: gate source changes on devnet only - risk low.
- `bins/node/examples/`: new or extended example - risk low (not in the node binary).
- `scripts/gauntlet-seed.sql`: runner text changes alter what `gauntlet-run.sh` executes; chain-writing scenarios become reachable - risk medium, guarded by REQ-LDP-052 and REQ-LDP-053.
- `WHITEPAPER*.md`: public text; risk of describing dormant rules as active - guarded by REQ-LDP-045.
- Regression areas: GS-026/027 (shared helpers), CLI withdrawal and exit tests.

## Decisions Resolved

| # | Decision | Consequence |
|---|---|---|
| D1 | CLI gate source fixed (REQ-LDP-001, REQ-LDP-002) | GS-029 sends a real HEAD-CLI withdrawal below H; the v6.44.2 nodes see the signed form a rolling upgrade will carry |
| D2 | Tx forge + live refusal at HEAD; RED proof by `bins/node/tests/it/inc_i_182_m1_red_forced_exit.rs` and `inc_i_182_m25_builder_parity.rs` (REQ-LDP-011, REQ-LDP-034) | On the control the holdings check already refuses a victim that owns live Bond UTXOs, so the live control cannot show the attack; the GS-028 control FAIL rests on the funds assertions |
| D3 | Unsigned-tx probe = v6.44.2 CLI AddBond; fallback = forge tool | A RequestWithdrawal from the old CLI would also break the unbonding rule, mixing two refusal reasons |
| D4 | Supply checks read B, U and issued(h) from the chain (REQ-LDP-008, REQ-LDP-009, REQ-LDP-010) | The vesting burn at W and the coinbase byte credit no longer break the check; the burn is attributed to the right block |
| D5 | Withdrawal mined, then sibling within 5 slots of the target block's slot; 6 nodes, 5 producers (REQ-LDP-005, REQ-LDP-006) | The evidence beats the gossip age limit; the floor of 3 holds even if two producers leave |
| D6 | GS-029: mixed boundary, same-dir restart, full hash history, state roots, canonical old blocks, named control FAIL (REQ-LDP-014, REQ-LDP-015, REQ-LDP-016, REQ-LDP-017, REQ-LDP-018, REQ-LDP-019, REQ-LDP-020, REQ-LDP-021) | A fork that heals between samples, or an orphaned old-producer block, is caught |
| D7 | R-4 is Should, behind approvals (REQ-LDP-050, REQ-LDP-051, REQ-LDP-052, REQ-LDP-053) | Nothing writes to a shared chain or wipes data without the operator |
| D8 | Whitepaper EN+ES in repo; no text test exists (a new check is added, REQ-LDP-046); published copies untouched | The report must say the published copies are stale until updated separately |

## Traceability Matrix

| Requirement ID | Priority | Assertion / Test | Implementation file |
|---|---|---|---|
| REQ-LDP-001 | Must | CLI gate-source test (RED/GREEN) | `bins/cli/src/cmd_producer/withdrawal.rs`, `exit.rs` |
| REQ-LDP-002 | Must | CLI test: mainnet/testnet byte-identical | same |
| REQ-LDP-003 | Must | `gs028-forced-exit-refused` input; REQ-LDP-030 test | tx-forge example (new) |
| REQ-LDP-004 | Must | forge refusal test (non-devnet, non-loopback) | tx-forge example |
| REQ-LDP-005 | Must | runner preconditions (SKIP lines) | `scripts/gauntlet-gs028.sh` |
| REQ-LDP-006 | Must | `gs028-withdrawal-before-evidence` | same |
| REQ-LDP-007 | Must | `gs028-one-slash-mined` | same |
| REQ-LDP-008 | Must | `gs028-slash-burns-locked` | same |
| REQ-LDP-009 | Must | `gs028-full-bond-leaves-supply` | same |
| REQ-LDP-010 | Must | `gs028-producer-gets-nothing` | same |
| REQ-LDP-011 | Must | `gs028-forced-exit-refused` | same + tx-forge |
| REQ-LDP-012 | Must | `gs028-tips-equal`, `gs028-production-continues` | same |
| REQ-LDP-013 | Must | control run record | same |
| REQ-LDP-014 | Must | runner preconditions | `scripts/gauntlet-gs029.sh` |
| REQ-LDP-015 | Must | `gs029-mixed-txs-mined`, `gs029-old-producer-built` | same |
| REQ-LDP-016 | Must | `gs029-mixed-producer-set-equal` | same |
| REQ-LDP-017 | Must | `gs029-mixed-no-fork`, `gs029-mixed-state-equal`, `gs029-old-blocks-canonical` | same |
| REQ-LDP-018 | Must | `gs029-restart-same-datadir` | same |
| REQ-LDP-019 | Must | `gs029-crossing-no-fork`, `gs029-unsigned-tx-refused`, `gs029-unsigned-block-refused`, `gs029-signed-tx-mined` | same + block forge |
| REQ-LDP-020 | Must | negative-control run record | same |
| REQ-LDP-021 | Must | control declarations in both runners | both runners |
| REQ-LDP-022 | Must | `scripts/test_gauntlet_gs028.sh`, `scripts/test_gauntlet_gs029.sh` | both runners |
| REQ-LDP-023 | Must | runner test: malformed RPC input | both runners; `scripts/gauntlet-devnet-lib.sh`, `scripts/gauntlet-devnet.py` |
| REQ-LDP-024 | Must | line count at the milestone gate | all new files |
| REQ-LDP-025 | Must | GS-026, GS-027 runs | existing runners; `scripts/gauntlet-gs027.sh` now on `scripts/gauntlet-devnet-lib.sh` + `scripts/gauntlet-devnet.py` |
| REQ-LDP-026 | Must | `gauntlet_runs` ids | memory rows |
| REQ-LDP-027 | Should | `gs029-post-gate-unbonding` | `scripts/gauntlet-gs029.sh` |
| REQ-LDP-028 | Should | METRIC field | same |
| REQ-LDP-029 | Should | runner companion tests | `scripts/test_gauntlet_gs028.sh`, `scripts/test_gauntlet_gs029.sh`; helpers in `scripts/gauntlet-devnet.py` (tested by `scripts/test_gauntlet_devnet_py.py`) |
| REQ-LDP-030 | Should | forge shape test | tx-forge example |
| REQ-LDP-031 | Could | METRIC field | `scripts/gauntlet-gs029.sh` |
| REQ-LDP-032 | Could | forge variant line | tx-forge example |
| REQ-LDP-033 | Could | METRIC field | `scripts/gauntlet-gs029.sh` |
| REQ-LDP-034 | Won't | in-process tests named in D2 | - |
| REQ-LDP-035 | Won't | - | - |
| REQ-LDP-040 | Must | `scripts/test_whitepaper_defense_text.sh` (pytest `scripts/test_defense_records.py::test_whitepaper_defense_text`) | `WHITEPAPER.md` §6.1 item 8, `WHITEPAPER-es.md` §6.1 item 8 |
| REQ-LDP-041 | Must | `scripts/test_whitepaper_defense_text.sh` (pytest `scripts/test_defense_records.py::test_whitepaper_defense_text`) | `WHITEPAPER*.md` §7.4 withdrawal and slashable paragraphs |
| REQ-LDP-042 | Must | `scripts/test_whitepaper_defense_text.sh` (pytest `scripts/test_defense_records.py::test_whitepaper_defense_text`) | `WHITEPAPER*.md` §7 producer-authorization paragraph |
| REQ-LDP-043 | Must | `scripts/test_whitepaper_defense_text.sh` (pytest `scripts/test_defense_records.py::test_whitepaper_defense_text`) | `WHITEPAPER*.md` §11.3 penalty list |
| REQ-LDP-044 | Must | `scripts/test_whitepaper_defense_text.sh` (pytest `scripts/test_defense_records.py::test_whitepaper_defense_text`) | `WHITEPAPER*.md` §19.4, summary list (§20.1); unbonding table row unchanged |
| REQ-LDP-045 | Must | `scripts/test_whitepaper_defense_text.sh` (pytest `scripts/test_defense_records.py::test_whitepaper_defense_text`) (no digits-as-heights, no dates, no network named) | every changed passage above |
| REQ-LDP-046 | Should | `scripts/test_whitepaper_defense_text.sh` (pytest `scripts/test_defense_records.py::test_whitepaper_defense_text`) | `scripts/test_whitepaper_defense_text.sh` |
| REQ-LDP-047 | Must | final report; EN/ES passage parity in `scripts/test_whitepaper_defense_text.sh` (pytest `scripts/test_defense_records.py::test_whitepaper_defense_text`) | - |
| REQ-LDP-048 | Could | `scripts/test_whitepaper_defense_text.sh` (pytest `scripts/test_defense_records.py::test_whitepaper_defense_text`) (count read from the TxType enum) | `WHITEPAPER*.md` §19.4 (27 -> 24) |
| REQ-LDP-050 | Should | `scripts/test_gauntlet_seed_optin.sh` (pytest `scripts/test_defense_records.py::test_gauntlet_seed_optin`) | `scripts/gauntlet-seed.sql`, memory `gauntlet_scenarios` |
| REQ-LDP-051 | Should | run report | gauntlet run record (`mechanism_id` `gauntlet.sh-default`), `scripts/README.md` counting paragraph |
| REQ-LDP-052 | Should | run report; `scripts/test_gauntlet_seed_optin.sh` (pytest `scripts/test_defense_records.py::test_gauntlet_seed_optin`) (no active runner writes a chain or wipes data) | `scripts/gauntlet-seed.sql` |
| REQ-LDP-053 | Must | run report; `scripts/test_gauntlet_seed_optin.sh` (pytest `scripts/test_defense_records.py::test_gauntlet_seed_optin`) (no pkill/killall/ssh in registered runners) | - |
| REQ-LDP-054 | Should | `scripts/test_gauntlet_seed_optin.sh` (pytest `scripts/test_defense_records.py::test_gauntlet_seed_optin`) (bullets, env knobs, commands, MANUAL-ONLY marks) | `scripts/README.md` |
| REQ-LDP-060 | Should | `scripts/test_gauntlet_seed_optin.sh` (pytest `scripts/test_defense_records.py::test_gauntlet_seed_optin`) | `scripts/gauntlet-seed.sql`, `scripts/gauntlet.sh` case arms, memory rows |
| REQ-LDP-061 | Should | `scripts/test_whitepaper_defense_text.sh` (pytest `scripts/test_defense_records.py::test_whitepaper_defense_text`) (decision row query, SKIP without memory) | memory `decisions` |
| REQ-LDP-062 | Should | `scripts/test_gauntlet_seed_optin.sh` (pytest `scripts/test_defense_records.py::test_gauntlet_seed_optin`) | `scripts/gauntlet-seed.sql`, memory rows |

## Specs Drift Detected

- `WHITEPAPER.md` / `WHITEPAPER-es.md` §7.4 (line 600), §11.3 (lines 862-863 EN, 860-861 ES), §19.4 (line 1150 EN, 1148 ES), parameter table (1253 / 1251), summary list (1281 / 1279): two-step withdrawal, immediate exclusion, re-registration after a slash, `ClaimWithdrawal` as a live type. The code keeps a slashed key out for good (`crates/storage/src/producer/set_registration.rs:24-31`, `set_lifecycle.rs:50,101`).
- §6.1 has no signature condition.
- The whitepaper counts 27 transaction types; the project map counts 24 variants (REQ-LDP-048).
- Registered gauntlet rows: GS-009, GS-010, GS-014 and GS-016 have no runner; GS-013, GS-015, GS-017, GS-018, GS-019, GS-021 and GS-022 share the plain `gauntlet.sh` run. This differs from the earlier account that the confirm variables were set without flags.
- `scripts/gauntlet-gs026.sh` is already over the 500-line limit (pre-existing).

## Assumptions

| # | Assumption (technical) | Plain language | Confirmed |
|---|---|---|---|
| A-1 | Trust model: RPC replies are untrusted for parsing; all nodes are throwaway local devnet nodes | We treat node replies as data, never as commands | No (operator) |
| A-2 | Producer-set changes flush at the reward-epoch boundary (`DOLI_BLOCKS_PER_REWARD_EPOCH`) | We know when a withdrawal takes effect | No (architect verifies) |
| A-3 | `sendTransaction` returns the refusal code in its error text; else the runner reads the node log | We can see why a tx was refused | No |
| A-4 | The v6.44.2 CLI can build an AddBond against a HEAD node's RPC; else the forge tool builds the unsigned AddBond | The old wallet still talks to new nodes | No |
| A-5 | Epoch reward payout at a boundary does not change total supply | Paying rewards moves coins, it does not create them | Partly (GS-027 window check) |
| A-6 | v6.44.2 and HEAD compute `getStateRootDebug` the same way for the same state; component hashes are printed to tell a format difference from a state difference | The state fingerprint is comparable across versions | No |
| A-7 | A `manual` status keeps a scenario out of `gauntlet-run.sh` (it selects `active`) | Manual-only scenarios are not auto-run | Yes (selector reads `status='active'`) |
| A-8 | On the pre-unbonding control the slash may or may not mine after the withdrawal; either way the funds assertions fail | The old build lets the cheater keep money | No |

## What We Do Not Understand

- Whether v6.44.2 applies a signed AddBond and RequestWithdrawal at the boundary exactly like HEAD (GS-029 measures it; unknown by design).
- Whether a node started with a not-yet-registered producer key waits quietly (REQ-LDP-031).
- Whether boundary blocks hold any transaction with inputs that would need its own term in removed(h).

## Identified Risks

- G-Set seeding with a mixed cluster SKIPs: re-run with a longer genesis delay; never count a SKIP.
- The sibling window is missed: retry rule in REQ-LDP-006.
- GS-029 finds a real v6.44.2 incompatibility: P0/P1 gets a RED/GREEN fix in this work and a new incident; the activation-height and deploy questions are asked before any rule change.
- Runtime: about 6-9 hours of wall time (two GS-028 runs, two GS-029 runs, two regressions, full test suites, R-4). Cut order: REQ-LDP-052, then Could items, then Should items; M1 Musts are never cut.
- One live network at a time; check disk headroom before any rebuild.

## Out of Scope (Won't)

- REQ-LDP-034 (Won't): live RED of the forced exit; needs a forged Registration with bonds at a non-derived address. The in-process tests prove it.
- REQ-LDP-035 (Won't): staggered gate heights and the pending-root crossing (mainnet crosses pending-root at its own future height); GS-029 uses one height for the four new gates.
- Pinning mainnet or testnet heights; push, release, deploy; updating published whitepaper copies.
- GUI and wallet-crate producer builders; slashing pre-pin findings; unbonding follow-ups; coverage of the extra test-network nodes by the service script.
- Aligning `specs/protocol.md` and `specs/security_model.md` (a separate doc-sync pass).
