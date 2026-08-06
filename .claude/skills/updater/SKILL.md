# updater — DOLI Auto-Update & Governance
<!-- @INDEX
ENTRY-POINTS       14-41
OPERATIONS         42-58
STRUCTS            59-98
FUNCTIONS          99-283
HARDFORK-SCHEDULE  284-305
DATA-FLOWS         306-415
DEPENDENCIES       416-438
CONSTRAINTS        439-498
PATTERNS           499-568
@/INDEX -->

## ENTRY-POINTS

Public API re-exported from `crates/updater/src/lib.rs` (13 files: apply, download, vote, enforcement, verification, hardfork, params, types, constants, util, test_keys, watchdog — 12 modules + lib.rs).

**apply**: `apply_update`, `auto_apply_from_github`, `backup_current`, `current_binary_path`, `extract_binary_from_tarball`, `extract_named_binary_from_tarball`, `install_binary`, `install_skills_from_tarball`, `restart_node`, `rollback`

**download**: `download_binary`, `download_checksums_txt`, `download_from_url`, `download_signatures_json`, `fetch_github_release`, `fetch_latest_release`, `verify_hash`, `GithubReleaseInfo`

**vote**: `Vote`, `VoteMessage`, `VoteTracker`

**enforcement**: `check_production_allowed`, `grace_period_deadline`, `grace_period_deadline_for_network`, `in_grace_period`, `in_grace_period_for_network`, `veto_deadline`, `veto_period_ended`, `ProductionBlocked`, `VersionEnforcement`

**verification**: `calculate_veto_result`, `sign_release_hash`, `verify_release_signatures`, `verify_release_signatures_with_keys`

**hardfork**: `HardForkInfo`, `HardForkSchedule`

**params**: `UpdateParams`

**constants**: `assert_production_keys`, `bootstrap_maintainer_keys`, `get_maintainer_keys`, `is_using_placeholder_keys`, `BOOTSTRAP_MAINTAINER_KEYS_MAINNET`, `BOOTSTRAP_MAINTAINER_KEYS_TESTNET`, `CHECK_INTERVAL`, `FALLBACK_MIRROR`, `GITHUB_API_URL`, `GITHUB_RELEASES_URL`, `GITHUB_REPO`, `GRACE_PERIOD`, `REQUIRED_SIGNATURES`, `VETO_PERIOD`, `VETO_THRESHOLD_PERCENT`

**util**: `current_timestamp`, `current_version`, `is_newer_version`, `platform_identifier`

**test_keys**: `create_test_release_signatures`, `should_use_test_keys`, `sign_with_test_key`, `test_maintainer_pubkeys`, `TestMaintainerKey`, `TEST_MAINTAINER_KEYS`

**watchdog** (pub mod): `UpdateWatchdog`, `WatchdogState`

NEW since 2026-05-11 scaffold: `install_skills_from_tarball` (apply.rs:511) — auto-update now also syncs `~/.doli/skills/` agent skill files from the release tarball. `STAGED_BINARY_PATH` hardened (apply.rs:189, ISSUE-174 #7) to close a TOCTOU symlink-swap root-exec vector in the sudo install fallback.

## OPERATIONS

| Task | Steps | Commands/Functions | Inputs | Success |
|------|-------|--------------------|--------|---------|
| Sign a release as maintainer | 1. Build binary+tarball, compute CHECKSUMS.txt 2. `sign_release_hash(keypair, version, checksums_sha256)` 3. Collect 3-of-5 sigs into SIGNATURES.json 4. Upload to GitHub Release | `sign_release_hash()` (verification.rs:27), CLI `doli release sign` | maintainer keypair, version, binary_sha256/checksums_sha256 | SIGNATURES.json with ≥3 valid `MaintainerSignature` entries |
| Verify a release's signatures | 1. Fetch release + on-chain maintainer keys (if node) 2. `verify_release_signatures_with_keys()` or `verify_release_signatures()` (CLI, no on-chain state) | `verify_release_signatures_with_keys(release, on_chain_keys, network)` (verification.rs:57) | `Release`, network, optional on-chain keys | `Ok(())` if ≥`REQUIRED_SIGNATURES`(3) valid sigs from allowed keys, else `InsufficientSignatures` |
| Vote to veto/approve an update | 1. Producer builds `VoteMessage::new(version, vote, producer_id)` 2. Sign over `message_bytes()` 3. Gossip signed vote 4. Receiver: `VoteMessage::verify()` then `VoteTracker::record_vote()` | `VoteMessage::new()`/`verify()` (vote.rs:41,61), `VoteTracker::record_vote()` (vote.rs:143) | version, Vote::{Approve,Veto}, producer keypair | One vote counted per producer_id (duplicate = false, no change) |
| Decide if an update is rejected | 1. Tally weighted votes 2. `VoteTracker::should_reject_weighted(total_weight)` (preferred) or legacy `should_reject(total_producers)` | `should_reject_weighted()` (vote.rs:193), `calculate_vote_weight()` (params.rs:100) | total active weight/count, recorded votes | `true` if veto weight/count ≥40% (`VETO_THRESHOLD_PERCENT`) |
| Apply an approved update (automated) | 1. Veto period ends + approved 2. `auto_apply_from_github(version, signed_checksums_sha256)` 3. `UpdateWatchdog::record_update()` 4. `restart_node()` | `auto_apply_from_github()` (apply.rs:411), `restart_node()` (apply.rs:653) | version, signed checksums_sha256 (from verified SIGNATURES.json) | New binary (+ CLI + skills, best-effort) installed atomically; process re-execs |
| Apply an update manually (legacy path) | 1. `apply_update(release, approved, veto_percent)` — checks veto/approval itself 2. downloads/hashes/backs-up/installs | `apply_update()` (apply.rs:75) | `Release`, approved bool, veto_percent | Binary installed; caller must still call `restart_node()` |
| Roll back a bad update | 1. Detect crash loop (watchdog) or manual trigger 2. `rollback()` restores `.backup` sibling 3. `restart_node()` | `rollback()` (apply.rs:632), `UpdateWatchdog::check_and_maybe_rollback()` (watchdog.rs:95) | existing `{binary}.backup` file | Previous binary restored; watchdog state cleared so it doesn't re-trigger |
| Detect post-update crash loop | 1. On successful update: `UpdateWatchdog::record_update(version)` before restart 2. On clean exit: `record_clean_shutdown()` 3. On next startup: `check_and_maybe_rollback()` | `UpdateWatchdog::new(data_dir, network)` (watchdog.rs:65) | data_dir, network (for `crash_window_secs`) | `Some(bad_version)` returned after `crash_threshold`(3) crashes inside the window → caller rolls back |
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

`UpdateError` (`types.rs:115`): InsufficientSignatures, InvalidSignature, HashMismatch, DownloadFailed, InstallFailed, Network(reqwest), Io, Json, VetoPeriodActive {remaining_hours, message}, RejectedByVeto {veto_percent, threshold}, NotApproved.

`VoteResult` (`types.rs:157`): total_producers, veto_count, veto_percent: u8, approved: bool.

`UpdateParams` (`params.rs:32`): veto_period_secs, grace_period_secs, min_voting_age_secs, min_voting_age_blocks, check_interval_secs, crash_window_secs, crash_threshold: u32, seniority_maturity_blocks, seniority_step_blocks, network: Network. Build via `UpdateParams::for_network(network)`. Also carries its own `veto_deadline`/`grace_period_deadline`/`veto_period_ended`/`in_grace_period` methods (params.rs:120-140) mirroring the free functions in `enforcement.rs` but network-parametrized.

`VersionEnforcement` (`enforcement.rs:66`): min_version, enforcement_time: u64, active: bool, binary_ready: bool. Production blocked only when binary_ready=true AND enforcement time passed AND version too old AND not timed out.

`ProductionBlocked` (`enforcement.rs:132`): current_version, required_version. Implements Display with banner message.

`HardForkInfo` (`hardfork.rs:15`): activation_height: u64, min_version: String, consensus_changes: Vec<String>.

`HardForkSchedule` (`hardfork.rs:50`): forks: Vec<HardForkInfo> (internal, sorted by height). Built via `HardForkSchedule::for_network(network)`.

`Vote` (`vote.rs:12`): enum {Approve, Veto}. Approve and abstain have same effect.

`VoteMessage` (`vote.rs:21`): version, vote: Vote, producer_id (hex pubkey, serde alias "producerId"), timestamp: u64, signature (hex). Signs "{version}:{approve|veto}:{timestamp}".

`VoteTracker` (`vote.rs:88`): version, vetos: HashSet<String>, approvals: HashSet<String>, producer_weights: HashMap<String,u64>. Supports count-based (legacy) and weight-based (anti-Sybil) rejection.

`UpdateWatchdog` (`watchdog.rs:57`): data_dir, crash_window_secs, crash_threshold (hardcoded `DEFAULT_CRASH_THRESHOLD`=3, watchdog.rs:54 — NOT `network.crash_threshold()`). Reads/writes `{data_dir}/watchdog_state.json`.

`WatchdogState` (`watchdog.rs:17`): last_update_version: Option<String>, last_update_time: Option<u64>, crash_timestamps: Vec<u64>, clean_shutdown: bool.

`GithubReleaseInfo` (`download.rs:355`): version, tarball_url, expected_hash (per-platform binary hash from CHECKSUMS.txt), checksums_sha256 (SHA-256 of the CHECKSUMS.txt file itself), changelog.

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
`download_binary(release) -> Result<Vec<u8>>` (download.rs:24): tries primary URL → GitHub CDN → fallback mirror. Async.

`download_from_url(url) -> Result<Vec<u8>>` (download.rs:72): HTTP GET with 5-min timeout. Async.

`verify_hash(binary, expected_hash) -> Result<()>` (download.rs:91): SHA-256, case-insensitive hex compare.

`fetch_latest_release(custom_url, network) -> Result<Option<Release>>` (download.rs:113): custom URL → GitHub API → fallback mirror. Filters by target_networks via `filter_release_by_network` (download.rs:177, private). Async.

`fetch_from_github() -> Result<Option<Release>>` (download.rs:205, private): builds a `Release` directly from GitHub Release assets (CHECKSUMS.txt + optional SIGNATURES.json + optional metadata.json) — no release.json needed.

`fetch_github_release(version: Option<&str>) -> Result<GithubReleaseInfo>` (download.rs:390): fetches a specific version (or latest) from GitHub API, downloads CHECKSUMS.txt, parses per-platform hash via `platform_target_triple()` (download.rs:373, private). Async.

`download_signatures_json(version) -> Result<Option<SignaturesFile>>` (download.rs:547): fetches SIGNATURES.json from GitHub Releases. Returns `None` on 404. Async.

`download_checksums_txt(version) -> Result<(String, String)>` (download.rs:570): fetches CHECKSUMS.txt, returns (content, sha256). Async.

`parse_iso8601_timestamp(s) -> Option<u64>` (download.rs:496, private): hand-rolled parser for GitHub's `"YYYY-MM-DDThh:mm:ssZ"` format — no chrono dependency.

### verification.rs
`sign_release_hash(keypair, version, binary_sha256) -> MaintainerSignature` (verification.rs:27): signs "{version}:{sha256}". Ed25519.

`verify_release_signatures(release, network) -> Result<()>` (verification.rs:48): uses bootstrap keys only. Convenience for CLI contexts without on-chain state.

`verify_release_signatures_with_keys(release, on_chain_keys, network) -> Result<()>` (verification.rs:57): if on_chain_keys non-empty uses those, else falls back to bootstrap keys. Needs 3-of-5 valid signatures.

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

`UpdateParams::calculate_vote_weight(bond_count: u32, blocks_active: u64) -> f64` (params.rs:100): bond_count * (1.0 + min(years,4) * 0.75). years = blocks_active / seniority_step_blocks. Stored as (weight * 100) as u64 in VoteTracker for 2-decimal precision.

`UpdateParams::seniority_multiplier(blocks_active) -> f64` (params.rs:108): standalone multiplier without bonds.

`UpdateParams::is_eligible_to_vote(blocks_since_registration) -> bool` (params.rs:115).

`UpdateParams::veto_deadline/grace_period_deadline/veto_period_ended/in_grace_period` (params.rs:120,125,130,135): network-parametrized duplicates of the `enforcement.rs` free functions — use these when a `UpdateParams` is already in scope.

### vote.rs
`VoteTracker::new(version)` / `VoteTracker::with_weights(version, weights)` (vote.rs:112,126).

`VoteTracker::record_vote(producer_id, vote) -> bool` (vote.rs:143): false if already voted (one vote per producer, no change).

`VoteTracker::veto_count()/approval_count()/total_votes()` (vote.rs:156,161,166).

`VoteTracker::should_reject(total_producers) -> bool` (vote.rs:173): count-based, legacy.

`VoteTracker::should_reject_weighted(total_weight) -> bool` (vote.rs:193): weight-based anti-Sybil. Preferred method.

`VoteTracker::veto_weight()/approval_weight()/veto_percent(total_producers)/veto_percent_weighted(total_weight)` (vote.rs:204,212,220,228).

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
-> verify_release_signatures_with_keys() [3-of-5 Ed25519]
-> Veto period begins (published_at + VETO_PERIOD)
-> Producers vote via VoteMessage (signed, gossipped)
-> VoteTracker.should_reject_weighted() at deadline
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

Signature Verification Key Selection:
```
verify_release_signatures_with_keys(release, on_chain_keys, network):
  if on_chain_keys.is_empty():
    keys = bootstrap_maintainer_keys(network)  [compile-time static]
  else:
    keys = on_chain_keys  [first 5 registered producers on-chain]
  count valid Ed25519 sigs from known keys
  require >= REQUIRED_SIGNATURES (3)
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
- `bins/cli/` — `doli release sign` (wraps `sign_release_hash`), `doli upgrade` (shows signature/veto status via `download_signatures_json`/`calculate_veto_result`), `doli-node update apply` (manual `apply_update`/`auto_apply_from_github` trigger per banner text in `enforcement.rs:153` / `apply.rs` docstrings).

## CONSTRAINTS

Governance rules (no exceptions):
- ALL updates require veto period (configurable, currently 5 min; target 7 days mainnet — module doc at lib.rs:5-16 still says "7-day"/"2 epochs", constants.rs:12 comment says "early network")
- 40% producer veto threshold (weighted by bonds x seniority)
- 3-of-5 maintainer signatures required (Ed25519)
- `REQUIRED_SIGNATURES = 3` (constants.rs:29)
- `VETO_THRESHOLD_PERCENT = 40` (constants.rs:26)
- `VETO_PERIOD = 5 * 60s` (constants.rs:13) — early network, target 7 days
- `GRACE_PERIOD = 3600s` (constants.rs:16)
- `CHECK_INTERVAL = 6 * 3600s` (constants.rs:116)

Production blocking rules (enforcement.rs:176):
- Blocked only when ALL true: enforcement_time passed + old version + binary_ready=true + elapsed < 30min
- Download failure -> warn + allow production (network must not halt for infra failure)
- `ENFORCEMENT_TIMEOUT_SECS = 30 * 60` (enforcement.rs:62)

Vote weight formula (params.rs:100):
- weight = bond_count * (1.0 + min(years,4) * 0.75)
- Seniority caps at 4 years (max multiplier 4.0x)
- Step: 1 year = seniority_step_blocks blocks (Devnet=144, production via Network)
- Stored as (weight * 100) as u64 in VoteTracker (2-decimal precision)
- Anti-Sybil: 100 new bonds = 100x1.0=100; 25 four-year veterans = 25x4.0=100

Maintainer keys:
- Mainnet: N1-N5 are both producers AND maintainers (dual role)
- Testnet: NT1-NT5 are both producers AND maintainers
- N6-N12 / NT6-NT12: producers only, cannot sign releases
- Bootstrap keys are static fallback; on-chain keys (first 5 registered producers) take precedence once synced
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

Platform identifiers: "linux-x64" | "linux-arm64" | "macos-x64" | "macos-arm64" | "unknown". Maps to Rust target triples (`platform_target_triple()`, download.rs:373) for CHECKSUMS.txt/tarball asset matching.

Agent skill sync (`install_skills_from_tarball`, apply.rs:511):
- Best-effort — failure never blocks the node/CLI binary update
- Destructive: `remove_dir_all(~/.doli/skills/)` before extracting — any locally hand-edited skill file under that path is wiped on update

install_binary fallback: On PermissionDenied (root-owned paths like /usr/local/bin/), uses sudo rm -f + sudo cp via `STAGED_BINARY_PATH`. On Linux, must delete before copy — cp fails with "Text file busy" on running binary. Requires passwordless sudo for the doli group.

## PATTERNS

Network-aware timing: Always use `UpdateParams::for_network(network)` instead of global constants. Mainnet/Testnet = production timing; Devnet = accelerated (veto=60s, grace=30s). `UpdateConfig::default()` uses mainnet defaults.

Key selection:
```
In running node (has on-chain state): verify_release_signatures_with_keys(release, &on_chain_keys, network)
In CLI (no on-chain state):           verify_release_signatures(release, network)  // bootstrap keys
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

VoteTracker weight storage:
```
let w = params.calculate_vote_weight(bond_count, blocks_active);
let weight_u64 = (w * 100.0) as u64;  // 100x multiplier for 2-decimal precision
weights.insert(producer_id, weight_u64);
let tracker = VoteTracker::with_weights(version, weights);
tracker.should_reject_weighted(total_weight)  // uses same scale
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

GitHub repo: `e-weil/doli` (constants.rs:120). API: `https://api.github.com/repos/e-weil/doli/releases/latest`. Fallback: `https://releases.doli.network`.
