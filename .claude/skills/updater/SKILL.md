# updater — DOLI Auto-Update & Governance
<!-- @INDEX
ENTRY-POINTS       14-45
OPERATIONS         46-63
STRUCTS            64-103
FUNCTIONS          104-286
HARDFORK-SCHEDULE  287-308
DATA-FLOWS         309-441
DEPENDENCIES       442-465
CONSTRAINTS        466-525
PATTERNS           526-593
@/INDEX -->

## ENTRY-POINTS

Public API re-exported from `crates/updater/src/lib.rs` (14 files: apply, download, vote, enforcement, verification, install_gate, trust_root, hardfork, params, types, constants, util, test_keys, watchdog — 13 modules + lib.rs).

**apply**: `apply_update`, `auto_apply_from_github`, `backup_current`, `current_binary_path`, `extract_binary_from_tarball`, `extract_named_binary_from_tarball`, `install_binary`, `install_skills_from_tarball`, `restart_node`, `rollback`

**download**: `download_binary`, `download_checksums_txt`, `download_from_url`, `download_signatures_json`, `fetch_github_release`, `fetch_latest_release`, `verify_hash`, `GithubReleaseInfo`

**vote**: `Vote`, `VoteMessage`, `VoteTracker`

**enforcement**: `check_production_allowed`, `grace_period_deadline`, `grace_period_deadline_for_network`, `in_grace_period`, `in_grace_period_for_network`, `veto_deadline`, `veto_period_ended`, `ProductionBlocked`, `VersionEnforcement`

**verification**: `calculate_veto_result`, `sign_release_hash`, `verify_release_signatures`, `verify_release_with_trust_root`

**install_gate**: `verify_release_artifact` — the artifact-bound install gate (INC-I-172 F1)

**trust_root**: `TrustRoot`, `TrustRootProvenance`

**hardfork**: `HardForkInfo`, `HardForkSchedule`

**params**: `UpdateParams`

**constants**: `assert_production_keys`, `bootstrap_maintainer_keys`, `get_maintainer_keys`, `is_using_placeholder_keys`, `BOOTSTRAP_MAINTAINER_KEYS_MAINNET`, `BOOTSTRAP_MAINTAINER_KEYS_TESTNET`, `CHECK_INTERVAL`, `GITHUB_API_URL`, `GITHUB_RELEASES_URL`, `GITHUB_REPO`, `GRACE_PERIOD`, `REQUIRED_SIGNATURES`, `VETO_PERIOD`, `VETO_THRESHOLD_PERCENT`

**util**: `current_timestamp`, `current_version`, `is_newer_version`, `platform_identifier`

**test_keys**: `create_test_release_signatures`, `should_use_test_keys`, `sign_with_test_key`, `test_maintainer_pubkeys`, `TestMaintainerKey`, `TEST_MAINTAINER_KEYS`

**watchdog** (pub mod): `UpdateWatchdog`, `WatchdogState` — **NOT WIRED**: zero production callers; no node rolls back automatically (INC-I-172 AUDIT-P1-014). `UpdateConfig::auto_rollback` and `--no-auto-rollback` are inert.

NEW since 2026-05-11 scaffold: `install_skills_from_tarball` (apply.rs:511) — auto-update now also syncs `~/.doli/skills/` agent skill files from the release tarball. `STAGED_BINARY_PATH` hardened (apply.rs:189, ISSUE-174 #7) to close a TOCTOU symlink-swap root-exec vector in the sudo install fallback.

## OPERATIONS

| Task | Steps | Commands/Functions | Inputs | Success |
|------|-------|--------------------|--------|---------|
| Sign a release as maintainer | 1. Build binary+tarball, compute CHECKSUMS.txt 2. `sign_release_hash(keypair, version, checksums_sha256)` 3. Collect 3-of-5 sigs into SIGNATURES.json 4. Upload to GitHub Release | `sign_release_hash()` (verification.rs:27), CLI `doli release sign` | maintainer keypair, version, binary_sha256/checksums_sha256 | SIGNATURES.json with ≥3 valid `MaintainerSignature` entries |
| Verify a release's signatures | 1. Resolve the `TrustRoot` (node service: `maintainer_trust_root_fn`; node commands: `command_trust_root(data_dir, network)`; CLI: `resolve_upgrade_trust_root(data_dir, network)` — the CLI reads THIS host's `maintainer_state.bin` too since INC-I-172 AUDIT-P1-012, and reaches `TrustRoot::bootstrap` only when the host is genuinely unbootstrapped) 2. `verify_release_with_trust_root()` — or the shim `verify_release_signatures()` which resolves the bootstrap root for you | `verify_release_with_trust_root(release, &root)` (verification.rs) | `Release`, `&TrustRoot` | `Ok(distinct_signers: usize)` if DISTINCT valid signers ≥ `root.threshold()`; `TrustRootUnavailable` if the root is empty/sub-threshold (FAILS CLOSED, never falls back to compiled keys); else `InsufficientSignatures` |
| Authorise an INSTALL from a downloaded tarball | `verify_release_artifact()` — signature check PLUS artifact binding. Never hand-roll a `Release` out of `SIGNATURES.json` fields and verify that: both operands would come from the file under test (INC-I-172 F1) | `verify_release_artifact(&release_info, &tarball, &sf, &root)` (install_gate.rs) | `&GithubReleaseInfo`, tarball bytes, `&SignaturesFile`, `&TrustRoot` | `Ok(distinct_signers)`, or `ArtifactBindingMismatch{field}` / `InsufficientSignatures` / `TrustRootUnavailable` / `HashMismatch` |
| Vote to veto/approve an update | 1. Producer builds `VoteMessage::new(version, vote, producer_id)` 2. Sign over `message_bytes()` 3. Gossip signed vote 4. Receiver: `VoteMessage::verify()` then `VoteTracker::record_vote()` | `VoteMessage::new()`/`verify()` (vote.rs:41,61), `VoteTracker::record_vote()` (vote.rs:143) | version, Vote::{Approve,Veto}, producer keypair | One vote counted per producer_id (duplicate = false, no change) |
| Decide if an update is rejected | 1. Tally veto HEAD COUNT 2. `VoteTracker::should_reject(total_producers)` | `should_reject()` (vote.rs) | active producer count, recorded votes | `true` if veto count ≥40% (`VETO_THRESHOLD_PERCENT`). There is NO weighted variant — deleted in INC-I-172 (it never executed) |
| Apply an approved update (automated) | 1. Veto period ends + approved 2. `auto_apply_from_github(version, signed_checksums_sha256)` 3. ~~`UpdateWatchdog::record_update()`~~ (NOT WIRED — no caller exists) 4. `restart_node()` | `auto_apply_from_github()` (apply.rs:411), `restart_node()` (apply.rs:653) | version, signed checksums_sha256 (from verified SIGNATURES.json) | New binary (+ CLI + skills, best-effort) installed atomically; process re-execs |
| Apply an update manually (`doli-node update apply`) | 1. `apply_update(release, first_notified_at, approved, veto_percent, &root)` — checks veto, approval, THEN re-verifies against the CURRENT root 2. delegates to `auto_apply_from_github`, which binds the signed CHECKSUMS.txt hash to the tarball | `apply_update()` (apply.rs), root from `command_trust_root(data_dir, network)` | `Release`, NODE-LOCAL `first_notified_at`, approved bool, veto_percent, **`&TrustRoot` (required)** | Binary installed; caller must still call `restart_node()`. `--force` waives community APPROVAL only — never maintainer authority (INC-I-172 F2) |
| Roll back a bad update | 1. Manual trigger ONLY — crash-loop detection is NOT wired (AUDIT-P1-014) 2. `rollback()` restores `.backup` sibling 3. `restart_node()` | `rollback()` (apply.rs:632); ~~`UpdateWatchdog::check_and_maybe_rollback()`~~ has no caller | existing `{binary}.backup` file | Previous binary restored |
| Detect post-update crash loop — **NOT IMPLEMENTED, design only** | 1. On successful update: `UpdateWatchdog::record_update(version)` before restart 2. On clean exit: `record_clean_shutdown()` 3. On next startup: `check_and_maybe_rollback()` | `UpdateWatchdog::new(data_dir, network)` (watchdog.rs:65) | data_dir, network (for `crash_window_secs`) | `Some(bad_version)` WOULD be returned after `crash_threshold`(3) crashes inside the window — but nothing calls it, so nothing rolls back (AUDIT-P1-014) |
| Schedule a hard fork (consensus-breaking upgrade) | 1. Add `HardForkInfo{activation_height, min_version, consensus_changes}` to `HardForkSchedule::for_network()` match arm 2. Use a far-future placeholder height 3. Before deploy, operator sets real height via `floor((current_height+7200)/360)*360` | `HardForkSchedule::for_network(network)` (hardfork.rs:208), `.add()` (hardfork.rs:61) | target network, activation height, min_version, consensus_changes | All nodes independently derive the same `fork_id()`; version-incompatible nodes stop producing at/after `activation_height` |
| Gate block production on hard fork compliance | 1. `HardForkSchedule::for_network(network)` at startup 2. Each produce attempt: `schedule.should_stop_producing(height, current_version())` | `should_stop_producing()` (hardfork.rs:38,84) | current height, current binary version | Production paused with warning if version too old for an active fork |
| Enforce a minimum version on producers | 1. `VersionEnforcement::from_approved_release_with_params()` after approval 2. background download sets `binary_ready` 3. each block: `check_production_allowed(Some(&enforcement))` | `check_production_allowed()` (enforcement.rs:176) | `VersionEnforcement`, current running version | `Err(ProductionBlocked)` only if enforced+overdue+binary_ready+not timed out; else `Ok(())` |
| Use test keys on devnet | 1. `export DOLI_TEST_KEYS=1` 2. `create_test_release_signatures(version, sha256)` for 3 signed test sigs | `should_use_test_keys()`, `create_test_release_signatures()` (test_keys.rs:91,74) | env var, version, binary_sha256 | Devnet accepts test-signed releases without real maintainer keys |

## STRUCTS

`Release` (`types.rs:23`): version, binary_sha256, binary_url_template, changelog, published_at (u64 Unix), signatures: Vec<MaintainerSignature>, target_networks: Vec<String>. Empty target_networks = all networks (backward compat).

`ReleaseMetadata` (`types.rs:14`): version, networks: Vec<String>, min_protocol_version: Option<u32>. From metadata.json in GitHub Release assets.

`MaintainerSignature` (`types.rs:50`): public_key (hex), signature (hex). Signs "{version}:{sha256}".

`SignaturesFile` (`types.rs:63`): version, checksums_sha256 (hex SHA-256 of CHECKSUMS.txt), signatures: Vec<MaintainerSignature>. Format for SIGNATURES.json uploaded to GitHub Releases.

`UpdateConfig` (`types.rs:76`): enabled, notify_only, auto_rollback, check_interval_secs, veto_period_secs, grace_period_secs, custom_url. Default: enabled=true, notify_only=false, auto_rollback=true, check_interval=6h, veto_period=5min, grace_period=1h.

`UpdateError` (`types.rs`): InsufficientSignatures {found, required}, **TrustRootUnavailable {provenance, keys, threshold}** (INC-I-172 fail-closed), InvalidSignature, HashMismatch, DownloadFailed, InstallFailed, Network(reqwest), Io, Json, VetoPeriodActive {remaining_hours, message}, RejectedByVeto {veto_percent, threshold}, NotApproved.

`VoteResult` (`types.rs:157`): total_producers, veto_count, veto_percent: u8, approved: bool.

`UpdateParams` (`params.rs`): veto_period_secs, grace_period_secs, min_voting_age_secs, min_voting_age_blocks, check_interval_secs, crash_window_secs, crash_threshold: u32, seniority_maturity_blocks, seniority_step_blocks, network: Network. Build via `UpdateParams::for_network(network)`. Also carries its own `veto_deadline`/`grace_period_deadline`/`veto_period_ended`/`in_grace_period` methods mirroring the free functions in `enforcement.rs` but network-parametrized. **All four take a NODE-LOCAL `first_notified_at: u64`, not a `&Release`** (INC-I-172 F7b). The seniority fields are inert: `calculate_vote_weight`/`seniority_multiplier`/`is_eligible_to_vote` were deleted.

`VersionEnforcement` (`enforcement.rs:66`): min_version, enforcement_time: u64, active: bool, binary_ready: bool. Production blocked only when binary_ready=true AND enforcement time passed AND version too old AND not timed out.

`ProductionBlocked` (`enforcement.rs:132`): current_version, required_version. Implements Display with banner message.

`HardForkInfo` (`hardfork.rs:15`): activation_height: u64, min_version: String, consensus_changes: Vec<String>.

`HardForkSchedule` (`hardfork.rs:50`): forks: Vec<HardForkInfo> (internal, sorted by height). Built via `HardForkSchedule::for_network(network)`.

`Vote` (`vote.rs:12`): enum {Approve, Veto}. Approve and abstain have same effect.

`VoteMessage` (`vote.rs:21`): version, vote: Vote, producer_id (hex pubkey, serde alias "producerId"), timestamp: u64, signature (hex). Signs "{version}:{approve|veto}:{timestamp}".

`VoteTracker` (`vote.rs:88`): version, vetos: HashSet<String>, approvals: HashSet<String>, producer_weights: HashMap<String,u64>. Supports count-based (legacy) and weight-based (anti-Sybil) rejection.

`UpdateWatchdog` (`watchdog.rs:57`): data_dir, crash_window_secs, crash_threshold (hardcoded `DEFAULT_CRASH_THRESHOLD`=3, watchdog.rs:54 — NOT `network.crash_threshold()`). Reads/writes `{data_dir}/watchdog_state.json`.

`WatchdogState` (`watchdog.rs:17`): last_update_version: Option<String>, last_update_time: Option<u64>, crash_timestamps: Vec<u64>, clean_shutdown: bool.

`GithubReleaseInfo` (`download.rs:329`): version, tarball_url, expected_hash (per-platform binary hash from CHECKSUMS.txt), checksums_sha256 (SHA-256 of the CHECKSUMS.txt file itself), changelog.

`TestMaintainerKey` (`test_keys.rs:16`): public_key: String, private_key: String.

## FUNCTIONS

### apply.rs
`current_binary_path() -> Result<PathBuf>` (apply.rs:16): gets running binary path, strips " (deleted)" suffix on Linux if binary was replaced while running.

`backup_path() -> Result<PathBuf>` (apply.rs:27): `current_binary_path()` with `.backup` extension. Used by both `backup_current()` and `rollback()`.

`backup_current() -> Result<PathBuf>` (apply.rs:34): copies binary to `.backup` sibling. Async.

`apply_update(release, approved, veto_percent) -> Result<()>` (apply.rs:75): security checks (veto period ended + approved), download, hash verify, backup, install. Legacy/manual path — prefer `auto_apply_from_github` for the automated flow.

`install_binary(binary, target) -> Result<()>` (apply.rs:152): atomic write (temp `.new` file + rename). Falls back to `install_binary_sudo` on `PermissionDenied`. Async.

`STAGED_BINARY_PATH = "/var/lib/doli/update.bin"` (apply.rs:189): staging path for the sudo-fallback install. **Security-hardened (ISSUE-174 #7)**: previously `/tmp/doli-update-binary` — world-writable `/tmp` + predictable name allowed a local-user TOCTOU race to win root code execution when the auto-updater fired. Now lives in `/var/lib/doli/` (mode 2770 doli:doli, installer-created) and is opened with `O_NOFOLLOW` (apply.rs:270) to defeat symlink swaps.

`INSTALLED_BINARY_MODE = 0o755` (apply.rs:199, `#[cfg(unix)]`): the mode every installed DOLI binary must carry, `rwxr-xr-x`. Both branches of `install_binary` install it. **INC-I-153: never lower this, and never stage below it** — see the install-path section for the full derivation.

`install_binary_sudo(binary, target) -> Result<()>` (apply.rs:230, private): writes to `STAGED_BINARY_PATH` with mode 0o755 (`INSTALLED_BINARY_MODE`) + `O_NOFOLLOW`, `sudo rm -f` then `sudo cp` (Linux "Text file busy" workaround for overwriting a running binary), cleans up staged file, then **verifies the postcondition**: stats the installed target and returns `Err(InstallFailed)` unless `installed_mode & 0o001 != 0` (apply.rs:314-370). INC-I-153 — a zero-exit `cp` proves the bytes landed, not the mode.

`auto_apply_from_github(version, signed_checksums_sha256) -> Result<()>` (apply.rs:411): full automated update flow (8 steps). Verifies CHECKSUMS.txt integrity against signed hash (closes TOCTOU/AUDIT-UPDATE-002), downloads tarball, verifies hash, extracts, installs doli-node, best-effort installs doli CLI, best-effort installs agent skills (`install_skills_from_tarball`, step 8). Does NOT call `restart_node()` — caller handles restart. Async.

`install_skills_from_tarball(tarball) -> Result<usize>` (apply.rs:511): **NEW**. Extracts `*/skills/**` entries from the release tarball to `~/.doli/skills/`, clearing any previously installed skills first (`remove_dir_all`). Returns count of `SKILL.md` files installed. Best-effort — caller treats failure as non-fatal.

`extract_named_binary_from_tarball(tarball, name) -> Result<Vec<u8>>` (apply.rs:591): finds entry by filename in `.tar.gz`. CI tarball format: `doli-node-v{version}-{triple}/{name}`.

`extract_binary_from_tarball(tarball) -> Result<Vec<u8>>` (apply.rs:627): wrapper for "doli-node".

`rollback() -> Result<()>` (apply.rs:632): restores from `.backup` sibling. Async.

`restart_node() -> !` (apply.rs:653): Unix: `exec()` (replaces process); Windows: spawn + exit.

### download.rs
`download_binary(release) -> Result<Vec<u8>>` (download.rs:23): tries primary URL → GitHub CDN. No fallback mirror (removed in INC-I-157). Async.

`download_from_url(url) -> Result<Vec<u8>>` (download.rs:64): HTTP GET with 5-min timeout. Async.

`verify_hash(binary, expected_hash) -> Result<()>` (download.rs:83): SHA-256, case-insensitive hex compare.

`fetch_latest_release(custom_url, network) -> Result<Option<Release>>` (download.rs:104): custom URL → GitHub API. No fallback mirror (removed in INC-I-157). Filters by target_networks via `filter_release_by_network` (download.rs:151, private). Async.

`fetch_from_github() -> Result<Option<Release>>` (download.rs:179, private): builds a `Release` directly from GitHub Release assets (CHECKSUMS.txt + optional SIGNATURES.json + optional metadata.json) — no release.json needed.

`fetch_github_release(version: Option<&str>) -> Result<GithubReleaseInfo>` (download.rs:364): fetches a specific version (or latest) from GitHub API, downloads CHECKSUMS.txt, parses per-platform hash via `platform_target_triple()` (download.rs:347, private). Async.

`download_signatures_json(version) -> Result<Option<SignaturesFile>>` (download.rs:521): fetches SIGNATURES.json from GitHub Releases. Returns `None` on 404. Async.

`download_checksums_txt(version) -> Result<(String, String)>` (download.rs:544): fetches CHECKSUMS.txt, returns (content, sha256). Async.

`parse_iso8601_timestamp(s) -> Option<u64>` (download.rs:470, private): hand-rolled parser for GitHub's `"YYYY-MM-DDThh:mm:ssZ"` format — no chrono dependency.

### verification.rs
`sign_release_hash(keypair, version, binary_sha256) -> MaintainerSignature` (verification.rs:27): signs "{version}:{sha256}". Ed25519.

`verify_release_signatures(release, network) -> Result<()>` (verification.rs): shim — resolves `TrustRoot::bootstrap(network)` and delegates. For CLI contexts with no on-chain state.

`verify_release_with_trust_root(release, &TrustRoot) -> Result<usize>` (verification.rs): THE signature entry point; returns the DISTINCT-signer count on success (print THAT, never `REQUIRED_SIGNATURES`). (1) `!root.is_usable()` → `error!` + `TrustRootUnavailable`, STOP — no fallback to compiled keys. (2) DISTINCT-SIGNER count using the covenant k-of-n shape (outer loop over `root.keys()`, inner over signature entries, `break` on first valid) — 3 entries from 1 key count as 1. Key comparison is ASCII case-insensitive (F10). (3) `valid >= root.threshold()`.

`verify_release_artifact(&GithubReleaseInfo, tarball, &SignaturesFile, &TrustRoot) -> Result<usize>` (install_gate.rs): THE install entry point, and what every path that WRITES a binary must call. Checks four links, any break blocks: L1 `sf.version` == the release tag (modulo `v`); L2 `sf.checksums_sha256` == `sha256(release_info.checksums_body)` — recomputed from the BYTES, not read from the `checksums_sha256` field; L3 `verify_release_with_trust_root`; L4 `sha256(tarball)` == the per-platform hash parsed from THOSE verified bytes. Without L1/L2 the signature check is circular and a replayed genuine SIGNATURES.json authorises any tarball (INC-I-172 F1).

`TrustRoot` (trust_root.rs): `bootstrap(network)` (keys = compiled array, threshold = `REQUIRED_SIGNATURES`, provenance `Bootstrap`) / `on_chain(keys, threshold)` (provenance `OnChain`, empty is representable and fails closed) / `keys()` / `threshold()` / `provenance()` / `is_usable()` = `threshold >= 1 && keys.len() >= threshold`.

`calculate_veto_result(veto_count, total_producers) -> VoteResult` (verification.rs:149): veto_percent = count*100/total (0 if total=0). approved = veto_percent < 40.

`verify_ed25519(pubkey_bytes, message, sig_bytes) -> bool` (verification.rs:171, private): parses raw key/sig bytes and verifies via `crypto::signature::verify`.

### enforcement.rs
`check_production_allowed(enforcement: Option<&VersionEnforcement>) -> Result<(), ProductionBlocked>` (enforcement.rs:176): blocks production only if: enforcement_time passed AND current version old AND binary_ready=true AND elapsed < 30min. Download failure = warn + allow production.

`veto_deadline(release) -> u64` (enforcement.rs:16): published_at + VETO_PERIOD.

`veto_period_ended(release) -> bool` (enforcement.rs:21).

`grace_period_deadline(release) -> u64` (enforcement.rs:28): uses mainnet defaults. Prefer `UpdateParams::grace_period_deadline` for network-aware.

`grace_period_deadline_for_network(release, network) -> u64` (enforcement.rs:33).

`in_grace_period(release) -> bool` (enforcement.rs:41).

`in_grace_period_for_network(release, network) -> bool` (enforcement.rs:49).

`ENFORCEMENT_TIMEOUT_SECS: u64 = 30*60` (enforcement.rs:62).

`VersionEnforcement::from_approved_release(release)` (enforcement.rs:82): mainnet defaults.

`VersionEnforcement::from_approved_release_with_params(release, params)` (enforcement.rs:95): preferred when network context available.

`VersionEnforcement::should_enforce()/version_meets_requirement(current)/seconds_until_enforcement()/hours_until_enforcement()` (enforcement.rs:106-123).

### params.rs
`UpdateParams::for_network(network) -> Self` (params.rs:57): all timing fields derived from `network.*()` methods on `doli_core::Network`.

`calculate_vote_weight` / `seniority_multiplier` / `is_eligible_to_vote`: **DELETED** (INC-I-172 F8). They had only `#[cfg(test)]` callers; the weighted veto never executed. Do not reintroduce them without a caller.

`UpdateParams::veto_deadline/grace_period_deadline/veto_period_ended/in_grace_period(first_notified_at: u64)`: network-parametrized duplicates of the `enforcement.rs` free functions — use these when an `UpdateParams` is already in scope. They take the NODE-LOCAL first-observed timestamp, never a `&Release`.

### vote.rs
`VoteTracker::new(version)`. (`with_weights`/`set_weights` were deleted — INC-I-172 F8.)

`VoteTracker::record_vote(producer_id, vote) -> bool` (vote.rs:143): false if already voted (one vote per producer, no change).

`VoteTracker::veto_count()/approval_count()/total_votes()` (vote.rs:156,161,166).

`VoteTracker::should_reject(total_producers) -> bool`: the ONLY rejection test. Veto head count ≥ 40%.

`VoteTracker::veto_percent(total_producers) -> u8`. (`should_reject_weighted`, `veto_weight`, `approval_weight`, `veto_percent_weighted` were deleted — INC-I-172 F8.)

`VoteTracker::version()/veto_producers()` (vote.rs:236,241).

`VoteMessage::new(version, vote, producer_id)` (vote.rs:41): unsigned constructor, stamps `current_timestamp()`.

`VoteMessage::message_bytes()` (vote.rs:52): "{version}:{approve|veto}:{timestamp}" as bytes.

`VoteMessage::verify(expected_producer)` (vote.rs:61): checks producer_id match + Ed25519 signature.

### watchdog.rs
`WatchdogState::load(data_dir)/save(data_dir)` (watchdog.rs:34,46, on WatchdogState): JSON at `{data_dir}/watchdog_state.json`; `load` defaults on missing/corrupt file.

`UpdateWatchdog::new(data_dir, network) -> Self` (watchdog.rs:65): `crash_window_secs` from `network.crash_window_secs()`; `crash_threshold` is the hardcoded constant `DEFAULT_CRASH_THRESHOLD = 3` (watchdog.rs:54), NOT network-derived.

`UpdateWatchdog::record_update(version)` (watchdog.rs:74): call before node restart after applying update.

`UpdateWatchdog::record_clean_shutdown()` (watchdog.rs:85): call on graceful shutdown.

`UpdateWatchdog::check_and_maybe_rollback() -> Option<String>` (watchdog.rs:95): call on startup. Returns bad version if threshold reached. Prunes crashes outside window. State persists in watchdog_state.json.

`UpdateWatchdog::clear()` (watchdog.rs:148): reset state (e.g., after manual rollback).

### hardfork.rs
`HardForkInfo::is_active(current_height) -> bool` (hardfork.rs:26): current_height >= activation_height.

`HardForkInfo::version_is_compatible(current_version) -> bool` (hardfork.rs:31): min_version not newer than current.

`HardForkInfo::should_stop_producing(current_height, current_version) -> bool` (hardfork.rs:38).

`HardForkInfo::blocks_until_activation(current_height) -> u64` (hardfork.rs:43).

`HardForkSchedule::new() -> Self` (hardfork.rs:56): empty schedule.

`HardForkSchedule::add(fork)` (hardfork.rs:61): duplicate heights replace (warn + retain latest). Maintains sorted order.

`HardForkSchedule::should_stop_producing(height, version) -> bool` (hardfork.rs:84): ANY fork triggers stop.

`HardForkSchedule::next_pending(height) -> Option<&HardForkInfo>` (hardfork.rs:91).

`HardForkSchedule::log_activations(current_height)` (hardfork.rs:96): logs exact activation moments.

`HardForkSchedule::active_forks(height) -> Vec<&HardForkInfo>` (hardfork.rs:108).

`HardForkSchedule::all() -> &[HardForkInfo]` (hardfork.rs:116); `is_empty()` (hardfork.rs:121).

`HardForkSchedule::fork_id(genesis_hash, current_height) -> crypto::Hash` (hardfork.rs:132): BLAKE3(genesis || h1_le || h2_le || ...) over active fork heights sorted ascending. Returns Hash::ZERO if no active forks. Used for peer handshake fork discrimination.

`HardForkSchedule::default_schedule() -> Self` (hardfork.rs:168): network-independent empty schedule (backward compat).

`HardForkSchedule::for_network(network) -> Self` (hardfork.rs:208): compile-time baked schedule per network. See HARDFORK-SCHEDULE section.

### util.rs
`current_timestamp() -> u64` (util.rs:4): Unix seconds.

`current_version() -> &'static str` (util.rs:12): CARGO_PKG_VERSION at compile time.

`is_newer_version(new, current) -> bool` (util.rs:17): simple (major, minor, patch) tuple comparison, strips leading 'v'.

`platform_identifier() -> &'static str` (util.rs:32): "linux-x64" | "linux-arm64" | "macos-x64" | "macos-arm64" | "unknown". Compile-time detection.

### constants.rs
`bootstrap_maintainer_keys(network) -> &'static [&'static str; 5]` (constants.rs:70): Mainnet → MAINNET keys; Testnet|Devnet → TESTNET keys.

`get_maintainer_keys(network) -> Vec<&'static str>` (constants.rs:107): returns test keys if DOLI_TEST_KEYS=1 AND network=Devnet, else bootstrap keys.

`is_using_placeholder_keys(network) -> bool` (constants.rs:81): true if any key starts with "00000000".

`assert_production_keys(network)` (constants.rs:91): panics if placeholder keys detected. Call during node init.

### test_keys.rs
`TEST_MAINTAINER_KEYS: LazyLock<[TestMaintainerKey; 5]>` (test_keys.rs:42): deterministic from seeds 1-5.

`sign_with_test_key(maintainer_index, message) -> Option<String>` (test_keys.rs:63).

`create_test_release_signatures(version, binary_sha256) -> Vec<(String,String)>` (test_keys.rs:74): signs with first 3 test keys (minimum 3-of-5 required).

`should_use_test_keys() -> bool` (test_keys.rs:91): env var DOLI_TEST_KEYS=1.

## HARDFORK-SCHEDULE

Current entries in `HardForkSchedule::for_network()` match arms (hardfork.rs:208-241) — this is the code that runs; the surrounding doc comment (hardfork.rs:178-207) still describes an older 10_000_080/7.0.0 M-Choice1 placeholder plan that was **superseded** by the actual entries below (doc comment is stale relative to code — code is SOT):

Mainnet: NO entries (hardfork.rs:211-218). Genesis reset means all features active from h=0. REWARDS_EPOCH_LIST_FIX is gated by a constant in rewards.rs/schedule.rs (NOT in HardForkSchedule — adding an entry changes fork_id immediately).

Testnet h=3,100, min_version="6.18.2": "EpochState state root inclusion (M-Choice1)" — INC-I-034 / M-Choice1 (hardfork.rs:222-228).

Testnet h=4,836, min_version="6.18.6": "Testnet HF deployment" (hardfork.rs:229-233).

Devnet: NO entries (hardfork.rs:235-238). Devnet resets constantly, tests via fixtures.

REWARDS_EPOCH_LIST_FIX note: NOT in HardForkSchedule because adding an entry changes fork_id immediately. Rolling deploy safe; old binary diverges at h=13320 (Mainnet). Gated by REWARDS_EPOCH_LIST_FIX_HEIGHT constant in rewards.rs/schedule.rs.

fork_id algorithm (hardfork.rs:132): BLAKE3(genesis_hash || h1_le || h2_le || ...) over activation heights of ALL active forks sorted ascending. Pre-first-fork = Hash::ZERO. Used in peer handshake to partition legacy peers from post-HF peers.

Operator formula for real activation height: `floor((current_height + 7200) / 360) * 360` — aligns to next epoch boundary at least 2 hours ahead of deploy.

**M-Choice1 output-contract test suite** (hardfork.rs:373-543, `m_choice1_epoch_snapshot_hf_tests`): locks the schedule's *shape*, not just presence. `test_m_choice1_schedule_has_epoch_snapshot_hf` requires Testnet to carry an entry whose `consensus_changes` text contains BOTH an EpochState/EpochSnapshot marker AND the phrase "state root" (case-insensitive) — rewording that text without preserving both markers fails CI. Mainnet: entry optional but must have `activation_height > 0` if present. Devnet: entry optional, must be `activation_height == 0` if present. `test_m_choice1_fork_id_changes_at_activation` pins fork_id transition (ZERO → non-ZERO → stable) at the exact Mainnet activation height.

CLAUDE.md Rule #0: Activation heights are IMMUTABLE once crossed. Never move them forward. New features get their own height.

## DATA-FLOWS

Normal Update Flow:
```
GitHub Release published
-> CI creates CHECKSUMS.txt + tarball per platform (+ agent skills)
-> Maintainers: doli release sign -> SIGNATURES.json uploaded
-> Node polls GitHub API (every 6h via check_interval)
-> fetch_latest_release() -> Release struct built
-> maintainer_trust_root_fn() -> TrustRoot (fails closed if empty/sub-threshold)
-> verify_release_with_trust_root() [k distinct Ed25519 signers]
-> Veto period begins (first_notified_at + VETO_PERIOD)  <- NODE-LOCAL, not published_at
-> Producers vote via VoteMessage (signed, gossipped)
-> VoteTracker.should_reject() at deadline
-> Before install: RE-verify against the CURRENT TrustRoot (auto_apply AND apply_update,
   which now takes `&TrustRoot` as a required parameter — INC-I-172 F2); drop if revoked
-> If < 40% veto: approved
-> Grace period: veto_deadline + GRACE_PERIOD
-> auto_apply_from_github(version, signed_checksums_sha256)
     -> fetch_github_release() [re-fetches CHECKSUMS.txt]
     -> integrity check: fetched checksums_sha256 == signed_checksums_sha256 [TOCTOU close]
     -> download_from_url(tarball_url)
     -> verify_hash(tarball, expected_hash) [per-platform hash from CHECKSUMS.txt]
     -> extract_binary_from_tarball()
     -> backup_current()
     -> install_binary() [atomic rename, or O_NOFOLLOW-staged sudo fallback]
     -> best-effort: install CLI binary, install_skills_from_tarball()
-> UpdateWatchdog.record_update(version)
-> restart_node() [exec() on Unix]
```

Crash Detection / Rollback:
```
Node starts -> UpdateWatchdog.check_and_maybe_rollback()
  -> if last_update_version set AND not clean_shutdown:
       push crash timestamp, prune window
       if crashes >= threshold (3): rollback() -> returns version
       caller: rollback() -> restart_node()
  -> if clean_shutdown: clear crash history -> None
```

Hard Fork Gating:
```
try_produce_block()
  -> HardForkSchedule::for_network(network)
  -> schedule.should_stop_producing(current_height, current_version)
  -> true -> block production, log warning
```

Production Enforcement:
```
UpdateService detects approved release
  -> VersionEnforcement::from_approved_release_with_params(release, &params)
  -> binary_ready = false (until download completes)
  -> download begins in background
  -> on success: binary_ready = true
  -> each produce attempt: check_production_allowed(Some(&enforcement))
       -> blocks only if: should_enforce() AND !version_meets_requirement() AND binary_ready AND not timed out (30min)
```

Trust-Root Selection (INC-I-172 F1 — resolved ONCE at the composition root,
`bins/node/src/updater/trust_root_wiring.rs::resolve_trust_root`):
```
members non-empty                        -> TrustRoot::on_chain(members, set.threshold)
members empty AND last_derived_height==0 -> TrustRoot::bootstrap(network)   [never had a set]
members empty AND last_derived_height>0  -> TrustRoot::on_chain(vec![], t)  [FAILS CLOSED]
maintainer_state lock unavailable        -> TrustRoot::on_chain(vec![], 3)  [FAILS CLOSED]

operator command (doli-node upgrade/update verify/update apply)
                                         -> command_trust_root(data_dir, network)  [F3: reads
                                            this host's maintainer_state.bin, Err is FATAL]
doli CLI (bins/cli)                      -> TrustRoot::bootstrap(network)  [not the node host]

verify_release_with_trust_root(release, &root):
  if !root.is_usable(): error! + TrustRootUnavailable   <- NO fallback to compiled keys
  for key in root.keys():                                <- DISTINCT signers, covenant shape
      for sig in release.signatures: if matches && verifies { valid += 1; break }
  require valid >= root.threshold()
```

Install-Gate Binding (INC-I-172 F1 — `crates/updater/src/install_gate.rs`). Every path
that WRITES a binary goes through this; a signature check alone is not a gate:
```
verify_release_artifact(&release_info, tarball, &sf, &root):
  L1 sf.version            == release_info.version (modulo leading "v")  else ArtifactBindingMismatch
  L2 sf.checksums_sha256   == sha256(release_info.checksums_body)        else ArtifactBindingMismatch
       ^ recomputed from BYTES; the checksums_sha256 FIELD is never trusted
  L3 verify_release_with_trust_root(Release{sf.version, sf.checksums_sha256, sf.signatures}, root)
  L4 sha256(tarball)       == platform_tarball_hash(release_info.checksums_body)  else HashMismatch
```

Binary Install Path Selection:
```
install_binary(binary, target):
  try fs::write(target.new) + rename  [direct, self-owned paths e.g. /mainnet/bin/]
  on PermissionDenied:
    install_binary_sudo(binary, target)
      -> stage at /var/lib/doli/update.bin (mode 0o755, O_NOFOLLOW)
      -> sudo rm -f target; sudo cp staged target
      -> cleanup staged file
      -> POSTCONDITION (INC-I-153): stat the INSTALLED target, and return
         Err(InstallFailed) unless `installed_mode & 0o001 != 0`. Fail-closed.
```

INC-I-153 — why the staged mode is `0o755` and not `0o750`:
`sudo rm -f` unlinks the target, so `sudo cp` always takes its CREATE path and the new
inode gets `staged_mode & ~umask`. Masking can only CLEAR bits, never add them, so the
staged file must already carry every bit the installed binary needs. Staging at `0o750`
can never yield o+x under any umask; systemd runs the node as `User=doli` while the
privileged copy leaves the file `root:root`, so the service account is in the OTHER class
and `execve` consults the other-execute bit alone → `status=203/EXEC`. This bricked a
mainnet producer. **Never lower the staged mode below `0o755`.**

Staging at `0o755` is NECESSARY BUT NOT SUFFICIENT — a site with `Defaults umask=0027`
still installs `0o750`. sudo's effective umask (`caller | sudoers Defaults`) is not
readable or settable through sudo, so the only trustworthy evidence is the mode on disk.
Hence the read-back. Notes on the guard:
- The predicate is exactly `installed_mode & 0o001 == 0` → `Err`, not `& 0o111 != 0o111`:
  the service account is neither owner nor group, so only S_IXOTH decides `execve`, and a
  stricter test would falsely reject e.g. `0o745`, aborting the upgrade after the target
  was already replaced.
- A best-effort in-process `set_permissions(target, 0o755)` runs first. It is a belt, not
  the guarantee — on the normal path `sudo cp` left the file `root:root` and it is refused
  EPERM; its outcome is only folded into the error message for diagnosis.
- The guard is FAIL-CLOSED, not self-healing: it runs AFTER `sudo rm -f` + `sudo cp` have
  already replaced the target, and no restore path exists (`rollback()` does an
  unprivileged `fs::copy` into a root-owned dir → EACCES). It converts a SILENT brick into
  a LOUD one; it does not prevent it. Operator recovery is the `sudo chmod 755 <target>`
  printed in the error.
- No new privileged verb was added. The sudoers whitelist is still exactly `rm -f` + `cp`
  (INV-SUDOERS-EXACT); the `sudo chmod` in the error text is inert operator advice, never
  passed to `Command`.

## DEPENDENCIES

```
crates/updater -> doli_core (Network, consensus consts)
              -> crypto (KeyPair, PrivateKey, PublicKey, Signature, Hash, Hasher)
              -> reqwest (async HTTP)
              -> sha2 (SHA-256 for binary hash)
              -> serde / serde_json
              -> flate2 + tar (tarball extraction)
              -> tokio (async runtime)
              -> tracing (logging)
              -> hex
              -> thiserror
              -> libc (O_NOFOLLOW flag, apply.rs)
              -> tempfile (test only)
```

Network timing (veto_period_secs, grace_period_secs, seniority_step_blocks, etc.) comes from `doli_core::Network` methods — not hardcoded in updater. Updater reads them via `UpdateParams::for_network(network)`.

**Used by** (per CLAUDE.md code map; exact call sites not re-verified this session — `rg`/glob unavailable in-session, cross-check with grep before relying on line numbers):
- `bins/node/` — production loop consults `HardForkSchedule::should_stop_producing()` and `check_production_allowed()` before producing a block; startup/init calls `assert_production_keys()`; periodic task polls `fetch_latest_release()` / drives the veto→grace→apply→watchdog pipeline; graceful shutdown calls `UpdateWatchdog::record_clean_shutdown()`.
- `bins/cli/` — `doli release sign` (wraps `sign_release_hash`); `doli upgrade` GATES the install on maintainer signatures (`download_signatures_json` → `verify_release_signatures`, `bins/cli/src/cmd_upgrade.rs:87-120`) and returns `Err` before extract/install on failure, on a verification error, or when `SIGNATURES.json` is absent (INC-I-172 F6) — it does NOT merely display status, and `calculate_veto_result` has no CLI caller; `doli-node update apply` (manual `apply_update`/`auto_apply_from_github` trigger per banner text in `enforcement.rs:153` / `apply.rs` docstrings).
- **NOT verified anywhere:** `doli-node upgrade` (`bins/node/src/commands/misc.rs::handle_upgrade_command`) downloads and installs with a checksum check only — no maintainer signature verification. Do not treat it as an equivalent of `doli upgrade`.

## CONSTRAINTS

Governance rules (no exceptions):
- ALL updates require a veto period (configurable, currently 5 min). Report the CONFIGURED value; there is no 7-day period anywhere in code.
- 40% producer veto threshold, by HEAD COUNT (no bond or seniority weighting)
- The veto window is measured from the NODE-LOCAL `first_notified_at`, never from `Release::published_at` (unsigned, attacker-supplied)
- Verification FAILS CLOSED: an empty or sub-threshold on-chain trust root refuses, it never falls back to the compiled bootstrap keys
- 3-of-5 maintainer signatures required (Ed25519)
- `REQUIRED_SIGNATURES = 3` (constants.rs:29)
- `VETO_THRESHOLD_PERCENT = 40` (constants.rs:26)
- `VETO_PERIOD = 5 * 60s` (constants.rs) — the enforced value
- `GRACE_PERIOD = 3600s` (constants.rs:16)
- `CHECK_INTERVAL = 6 * 3600s` (constants.rs:116)

Production blocking rules (enforcement.rs:176):
- Blocked only when ALL true: enforcement_time passed + old version + binary_ready=true + elapsed < 30min
- Download failure -> warn + allow production (network must not halt for infra failure)
- `ENFORCEMENT_TIMEOUT_SECS = 30 * 60` (enforcement.rs:62)

Vote weight formula: **THERE IS NONE.** One active producer, one veto vote. The
bond x seniority formula was documented for years but only ever ran in tests; it was
deleted in INC-I-172 F8. The only Sybil barrier on the veto is the registration bond.

Maintainer keys:
- Mainnet: N1-N5 are both producers AND maintainers (dual role)
- Testnet: NT1-NT5 are both producers AND maintainers
- N6-N12 / NT6-NT12: producers only, cannot sign releases
- Bootstrap keys are **NOT a fallback.** They are used only by a root resolved as `Bootstrap` — a node that has NEVER established an on-chain set (`members` empty AND `last_derived_height == 0`), and the `doli` CLI, which is not the node host. An on-chain set that exists and is empty resolves to an unusable `OnChain` root and REFUSES; it never degrades to the compiled keys (INC-I-172 F1). See `:450` and the resolution table in `bins/node/src/updater/trust_root_wiring.rs`.
- `doli-node upgrade` / `update verify` / `update apply` resolve the ON-CHAIN root from this host's `maintainer_state.bin` via `command_trust_root(data_dir, network)` (INC-I-172 F3) — they do not use the compiled keys
- `is_using_placeholder_keys()` must return false before mainnet launch
- `assert_production_keys(network)` panics on placeholder; call during node init

Hard fork constraints (from CLAUDE.md #0 RULE):
- Once activated on mainnet, activation height is IMMUTABLE — never move forward
- **NEVER add entries to `HardForkSchedule` for rolling deploys** — `current_fork_id()` uses `u64::MAX`, making ALL entries active in fork_id immediately regardless of actual chain height. A rolling deploy across nodes with different `fork_id` computations partitions the mesh.
- For rolling-deploy-safe feature gates: use constant gates (e.g., `REWARDS_EPOCH_LIST_FIX_HEIGHT`) in `NetworkParams`/consensus modules, NOT `HardForkSchedule`
- `fork_id()` filters active forks at the given height — all entries past their height appear in fork_id immediately
- The M-Choice1 test suite (hardfork.rs:373-543) enforces schedule *shape* — changing Testnet's consensus_changes wording without keeping both an epoch-state marker and "state root" breaks CI

TOCTOU / symlink protection (AUDIT-UPDATE-002, ISSUE-174 #7):
- `auto_apply_from_github()` receives `signed_checksums_sha256` (the hash maintainers actually signed)
- Re-fetches CHECKSUMS.txt and compares — mismatch = abort (possible tampered release)
- `expected_hash` in `GithubReleaseInfo` = per-platform binary hash FROM CHECKSUMS.txt (not SHA256 of CHECKSUMS.txt itself)
- Sudo-fallback staging path moved from world-writable `/tmp/doli-update-binary` to `/var/lib/doli/update.bin` (mode 2770 parent, 0o755 file since INC-I-153 — was 0o750), opened with `O_NOFOLLOW` — closes a local-user symlink-swap race that could win root code execution via the auto-updater's `sudo cp`. The `0o750`→`0o755` widening does NOT reopen the race: both closures are the staging DIRECTORY (2770, which `other` cannot even traverse) and `O_NOFOLLOW`, neither of which depends on the file's own mode; and no write bit is granted to group or other.
- Sudoers rule MUST reference the exact `STAGED_BINARY_PATH` string; see `install.sh`/`postinst.sh`

Watchdog behavior:
- Persisted in `{data_dir}/watchdog_state.json`
- `crash_threshold` is a hardcoded constant (3), NOT derived from `network.crash_threshold()` — only `crash_window_secs` is network-aware
- Clean shutdown clears crash history -> no false rollback
- After rollback, state cleared -> no re-trigger

Platform identifiers: "linux-x64" | "linux-arm64" | "macos-x64" | "macos-arm64" | "unknown". Maps to Rust target triples (`platform_target_triple()`, download.rs:347) for CHECKSUMS.txt/tarball asset matching.

Agent skill sync (`install_skills_from_tarball`, apply.rs:511):
- Best-effort — failure never blocks the node/CLI binary update
- Destructive: `remove_dir_all(~/.doli/skills/)` before extracting — any locally hand-edited skill file under that path is wiped on update

install_binary fallback: On PermissionDenied (root-owned paths like /usr/local/bin/), uses sudo rm -f + sudo cp via `STAGED_BINARY_PATH`. On Linux, must delete before copy — cp fails with "Text file busy" on running binary. Requires passwordless sudo for the doli group.

## PATTERNS

Network-aware timing: Always use `UpdateParams::for_network(network)` instead of global constants. Mainnet/Testnet = production timing; Devnet = accelerated (veto=60s, grace=30s). `UpdateConfig::default()` uses mainnet defaults.

Key selection:
```
In running node (has on-chain state): verify_release_with_trust_root(release, &maintainer_trust_root_fn()())
In CLI (no on-chain state):           verify_release_signatures(release, network)  // TrustRoot::bootstrap(network)
In devnet tests:                      DOLI_TEST_KEYS=1 -> get_maintainer_keys(Devnet) returns test keys
```

Hard fork schedule usage (node startup / block production):
```
let schedule = HardForkSchedule::for_network(network);
if schedule.should_stop_producing(current_height, current_version()) { /* pause */ }
schedule.log_activations(current_height);  // at epoch boundary
let id = schedule.fork_id(&genesis_hash, current_height);  // peer handshake
```

Adding a new hard fork (hardfork.rs:208):
```
1. Add entry to HardForkSchedule::for_network() for affected networks
2. Use far-future placeholder height (>= current_height + reasonable lead time)
3. Operator updates real height before deploy: floor((current_height + 7200) / 360) * 360
4. NEVER use HardForkSchedule for rolling-deploy features -- fork_id changes for all peers immediately
5. For rolling-safe: use a constant height gate in the relevant module
6. If the entry is load-bearing for a test contract (like M-Choice1), keep the marker words in consensus_changes text
```

VoteTracker usage (head count — no weights, INC-I-172 F8):
```
let mut tracker = VoteTracker::new(version);
tracker.record_vote(producer_id, Vote::Veto);   // false if that producer already voted
tracker.should_reject(active_producer_count)    // true if veto count >= 40%
```

Test keys activation (devnet CI only):
```
DOLI_TEST_KEYS=1 -> should_use_test_keys()=true -> get_maintainer_keys(Devnet) returns test pubkeys
-> create_test_release_signatures(version, sha256) -> 3 valid signatures for tests
```

Binary update chain of trust:
```
Maintainers sign "{version}:{SHA256(CHECKSUMS.txt)}"
-> SIGNATURES.json uploaded
-> Node: CHECKSUMS.txt hash verified against signed value [TOCTOU]
-> CHECKSUMS.txt parsed for per-platform tarball hash
-> Tarball downloaded + SHA-256 verified against CHECKSUMS.txt entry
-> Binary extracted from tarball
```

Staged-binary O_NOFOLLOW pattern (apply.rs:230-374, security-critical — replicate for any future privileged-write helper):
```
1. create_dir_all(parent) if missing (ownership is operator's responsibility, not this code's)
2. remove_file(staged) best-effort — narrows symlink-swap window to one syscall
3. OpenOptions::new().write(true).create(true).truncate(true).mode(0o755).custom_flags(O_NOFOLLOW)
4. write + sync_all + set_permissions(0o755)   [chmod(2): exact, umask-independent]
5. sudo rm -f target; sudo cp staged target
6. remove_file(staged) cleanup (both success and failure paths)
7. POSTCONDITION: stat(target); Err(InstallFailed) unless mode & 0o001 != 0  [INC-I-153]
```
Step 7 is the one that is easy to omit and expensive to omit: steps 1-6 all succeed on a
host where the installed binary is not executable by the service account.

Enforcement timeout safety: If `auto_apply_from_github` fails (network error, wrong tarball name), `binary_ready` stays false -> production continues with warning. If `enforcement_time + 30min` passes with old version, enforcement auto-expires. Prevents indefinite production halt from infrastructure failures.

GitHub repo: `doli-network/doli` (constants.rs:132). API: `https://api.github.com/repos/doli-network/doli/releases/latest`. No fallback mirror — the dangling `releases.doli.network` fallback was removed in INC-I-157 (it fed `binary_url_template`, which `download_binary` tries FIRST, so a dangling name there was a hijack primitive).
