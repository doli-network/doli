# updater — DOLI Auto-Update & Governance
<!-- @INDEX
ENTRY-POINTS: lines 13-38
STRUCTS: lines 39-78
FUNCTIONS: lines 79-231
HARDFORK-SCHEDULE: lines 232-251
DATA-FLOWS: lines 252-308
DEPENDENCIES: lines 309-324
CONSTRAINTS: lines 325-377
PATTERNS: lines 378-421
-->

## ENTRY-POINTS

Public API re-exported from `crates/updater/src/lib.rs`:

**apply**: `apply_update`, `auto_apply_from_github`, `backup_current`, `current_binary_path`, `extract_binary_from_tarball`, `extract_named_binary_from_tarball`, `install_binary`, `restart_node`, `rollback`

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

## STRUCTS

`Release` (`types.rs:23`): version, binary_sha256, binary_url_template, changelog, published_at (u64 Unix), signatures: Vec<MaintainerSignature>, target_networks: Vec<String>. Empty target_networks = all networks (backward compat).

`ReleaseMetadata` (`types.rs:14`): version, networks: Vec<String>, min_protocol_version: Option<u32>. From metadata.json in GitHub Release assets.

`MaintainerSignature` (`types.rs:49`): public_key (hex), signature (hex). Signs "{version}:{sha256}".

`SignaturesFile` (`types.rs:63`): version, checksums_sha256 (hex SHA-256 of CHECKSUMS.txt), signatures: Vec<MaintainerSignature>. Format for SIGNATURES.json uploaded to GitHub Releases.

`UpdateConfig` (`types.rs:76`): enabled, notify_only, auto_rollback, check_interval_secs, veto_period_secs, grace_period_secs, custom_url. Default: enabled=true, notify_only=false, auto_rollback=true, check_interval=6h, veto_period=5min, grace_period=1h.

`UpdateError` (`types.rs:113`): InsufficientSignatures, InvalidSignature, HashMismatch, DownloadFailed, InstallFailed, Network(reqwest), Io, Json, VetoPeriodActive {remaining_hours, message}, RejectedByVeto {veto_percent, threshold}, NotApproved.

`VoteResult` (`types.rs:157`): total_producers, veto_count, veto_percent: u8, approved: bool.

`UpdateParams` (`params.rs:32`): veto_period_secs, grace_period_secs, min_voting_age_secs, min_voting_age_blocks, check_interval_secs, crash_window_secs, crash_threshold: u32, seniority_maturity_blocks, seniority_step_blocks, network: Network. Build via UpdateParams::for_network(network).

`VersionEnforcement` (`enforcement.rs:66`): min_version, enforcement_time: u64, active: bool, binary_ready: bool. Production blocked only when binary_ready=true AND enforcement time passed AND version too old AND not timed out.

`ProductionBlocked` (`enforcement.rs:131`): current_version, required_version. Implements Display with banner message.

`HardForkInfo` (`hardfork.rs:15`): activation_height: u64, min_version: String, consensus_changes: Vec<String>.

`HardForkSchedule` (`hardfork.rs:50`): forks: Vec<HardForkInfo> (internal, sorted by height). Built via HardForkSchedule::for_network(network).

`Vote` (`vote.rs:12`): enum {Approve, Veto}. Approve and abstain have same effect.

`VoteMessage` (`vote.rs:21`): version, vote: Vote, producer_id (hex pubkey), timestamp: u64, signature (hex). Signs "{version}:{vote}:{timestamp}".

`VoteTracker` (`vote.rs:88`): version, vetos: HashSet<String>, approvals: HashSet<String>, producer_weights: HashMap<String,u64>. Supports count-based (legacy) and weight-based (anti-Sybil) rejection.

`UpdateWatchdog` (`watchdog.rs:57`): data_dir, crash_window_secs, crash_threshold. Reads/writes {data_dir}/watchdog_state.json.

`WatchdogState` (`watchdog.rs:17`): last_update_version: Option<String>, last_update_time: Option<u64>, crash_timestamps: Vec<u64>, clean_shutdown: bool.

`GithubReleaseInfo` (`download.rs:355`): version, tarball_url, expected_hash (per-platform binary hash from CHECKSUMS.txt), checksums_sha256 (SHA-256 of CHECKSUMS.txt file itself), changelog.

`TestMaintainerKey` (`test_keys.rs:17`): public_key: String, private_key: String.

## FUNCTIONS

### apply.rs
current_binary_path() -> Result<PathBuf> (apply.rs:16): gets running binary path, strips " (deleted)" suffix on Linux if binary was replaced while running.

backup_current() -> Result<PathBuf> (apply.rs:34): copies binary to .backup extension sibling. Async.

apply_update(release, approved, veto_percent) -> Result<()> (apply.rs:75): security checks (veto period ended + approved), download, hash verify, backup, install. Use auto_apply_from_github for automated flow.

install_binary(binary, target) -> Result<()> (apply.rs:152): atomic write (temp file + rename). Falls back to sudo cp on PermissionDenied. Async.

auto_apply_from_github(version, signed_checksums_sha256) -> Result<()> (apply.rs:255): full automated update flow. Verifies CHECKSUMS.txt integrity against signed hash (closes TOCTOU/AUDIT-UPDATE-002), downloads tarball, verifies hash, extracts, installs doli-node, also installs doli CLI (best-effort). Does NOT call restart_node() -- caller handles restart. Async.

extract_named_binary_from_tarball(tarball, name) -> Result<Vec<u8>> (apply.rs:346): finds entry by filename in .tar.gz. CI tarball format: doli-node-v{version}-{triple}/{name}.

extract_binary_from_tarball(tarball) -> Result<Vec<u8>> (apply.rs:382): wrapper for "doli-node".

rollback() -> Result<()> (apply.rs:387): restores from .backup sibling. Async.

restart_node() -> ! (apply.rs:408): Unix: exec() (replaces process); Windows: spawn + exit.

### download.rs
download_binary(release) -> Result<Vec<u8>> (download.rs:24): tries primary URL -> GitHub CDN -> fallback mirror. Async.

download_from_url(url) -> Result<Vec<u8>> (download.rs:72): HTTP GET with 5-min timeout. Async.

verify_hash(binary, expected_hash) -> Result<()> (download.rs:91): SHA-256, case-insensitive hex compare.

fetch_latest_release(custom_url, network) -> Result<Option<Release>> (download.rs:113): custom URL -> GitHub API -> fallback mirror. Filters by target_networks. Async.

fetch_github_release(version: Option<&str>) -> Result<GithubReleaseInfo> (download.rs:390): fetches specific version (or latest) from GitHub API, downloads CHECKSUMS.txt, parses per-platform hash. Async.

download_signatures_json(version) -> Result<Option<SignaturesFile>> (download.rs:547): fetches SIGNATURES.json from GitHub Releases. Returns None if 404. Async.

download_checksums_txt(version) -> Result<(String, String)> (download.rs:570): fetches CHECKSUMS.txt, returns (content, sha256). Async.

### verification.rs
sign_release_hash(keypair, version, binary_sha256) -> MaintainerSignature (verification.rs:27): signs "{version}:{sha256}". Ed25519.

verify_release_signatures(release, network) -> Result<()> (verification.rs:48): uses bootstrap keys only. Convenience for CLI contexts without on-chain state.

verify_release_signatures_with_keys(release, on_chain_keys, network) -> Result<()> (verification.rs:57): if on_chain_keys non-empty uses those, else falls back to bootstrap keys. Needs 3-of-5 valid signatures.

calculate_veto_result(veto_count, total_producers) -> VoteResult (verification.rs:149): veto_percent = count*100/total (0 if total=0). approved = veto_percent < 40.

### enforcement.rs
check_production_allowed(enforcement: Option<&VersionEnforcement>) -> Result<(), ProductionBlocked> (enforcement.rs:176): blocks production only if: enforcement_time passed AND current version old AND binary_ready=true AND elapsed < 30min. Download failure = warn + allow production.

veto_deadline(release) -> u64 (enforcement.rs:16): published_at + VETO_PERIOD.

veto_period_ended(release) -> bool (enforcement.rs:21).

grace_period_deadline(release) -> u64 (enforcement.rs:28): uses mainnet defaults. Prefer UpdateParams::grace_period_deadline for network-aware.

grace_period_deadline_for_network(release, network) -> u64 (enforcement.rs:33).

in_grace_period(release) -> bool (enforcement.rs:41).

in_grace_period_for_network(release, network) -> bool (enforcement.rs:49).

VersionEnforcement::from_approved_release(release) (enforcement.rs:82): mainnet defaults.

VersionEnforcement::from_approved_release_with_params(release, params) (enforcement.rs:95): preferred when network context available.

VersionEnforcement::should_enforce() / version_meets_requirement(current) / seconds_until_enforcement() / hours_until_enforcement() (enforcement.rs:106-124).

### params.rs
UpdateParams::for_network(network) -> Self (params.rs:57): all timing fields derived from network.*() methods on doli_core::Network.

UpdateParams::calculate_vote_weight(bond_count: u32, blocks_active: u64) -> f64 (params.rs:100): bond_count * (1.0 + min(years,4) * 0.75). years = blocks_active / seniority_step_blocks. Stored as (weight * 100) as u64 in VoteTracker for 2-decimal precision.

UpdateParams::seniority_multiplier(blocks_active) -> f64 (params.rs:108): standalone multiplier without bonds.

UpdateParams::is_eligible_to_vote(blocks_since_registration) -> bool (params.rs:115).

### vote.rs
VoteTracker::new(version) / VoteTracker::with_weights(version, weights) (vote.rs:112-133).

VoteTracker::record_vote(producer_id, vote) -> bool (vote.rs:143): false if already voted (one vote per producer, no change).

VoteTracker::should_reject(total_producers) -> bool (vote.rs:173): count-based, legacy.

VoteTracker::should_reject_weighted(total_weight) -> bool (vote.rs:193): weight-based anti-Sybil. Preferred method.

VoteTracker::veto_weight() / approval_weight() / veto_percent_weighted(total_weight) (vote.rs:204-233).

VoteMessage::message_bytes() (vote.rs:52): "{version}:{approve|veto}:{timestamp}" as bytes.

VoteMessage::verify(expected_producer) (vote.rs:61): checks producer_id match + Ed25519 signature.

### watchdog.rs
UpdateWatchdog::new(data_dir, network) -> Self (watchdog.rs:65): uses network.crash_window_secs(). Default crash_threshold = 3.

UpdateWatchdog::record_update(version) (watchdog.rs:74): call before node restart after applying update.

UpdateWatchdog::record_clean_shutdown() (watchdog.rs:85): call on graceful shutdown.

UpdateWatchdog::check_and_maybe_rollback() -> Option<String> (watchdog.rs:95): call on startup. Returns bad version if threshold reached. Prunes crashes outside window. State persists in watchdog_state.json.

UpdateWatchdog::clear() (watchdog.rs:148): reset state (e.g., after manual rollback).

### hardfork.rs
HardForkInfo::is_active(current_height) -> bool (hardfork.rs:26): current_height >= activation_height.

HardForkInfo::version_is_compatible(current_version) -> bool (hardfork.rs:31): min_version not newer than current.

HardForkInfo::should_stop_producing(current_height, current_version) -> bool (hardfork.rs:38).

HardForkInfo::blocks_until_activation(current_height) -> u64 (hardfork.rs:43).

HardForkSchedule::add(fork) (hardfork.rs:61): duplicate heights replace (warn + retain latest). Maintains sorted order.

HardForkSchedule::should_stop_producing(height, version) -> bool (hardfork.rs:84): ANY fork triggers stop.

HardForkSchedule::next_pending(height) -> Option<&HardForkInfo> (hardfork.rs:91).

HardForkSchedule::log_activations(current_height) (hardfork.rs:96): logs exact activation moments.

HardForkSchedule::active_forks(height) -> Vec<&HardForkInfo> (hardfork.rs:108).

HardForkSchedule::fork_id(genesis_hash, current_height) -> crypto::Hash (hardfork.rs:132): BLAKE3(genesis || h1_le || h2_le || ...) over active fork heights sorted ascending. Returns Hash::ZERO if no active forks. Used for peer handshake fork discrimination.

HardForkSchedule::default_schedule() -> Self (hardfork.rs:168): network-independent empty schedule (backward compat).

HardForkSchedule::for_network(network) -> Self (hardfork.rs:208): compile-time baked schedule per network. See HARDFORK-SCHEDULE section.

### util.rs
current_timestamp() -> u64 (util.rs:4): Unix seconds.

current_version() -> &'static str (util.rs:12): CARGO_PKG_VERSION at compile time.

is_newer_version(new, current) -> bool (util.rs:17): simple (major, minor, patch) tuple comparison, strips leading 'v'.

platform_identifier() -> &'static str (util.rs:32): "linux-x64" | "linux-arm64" | "macos-x64" | "macos-arm64" | "unknown". Compile-time detection.

### constants.rs
bootstrap_maintainer_keys(network) -> &'static [&'static str; 5] (constants.rs:70): Mainnet -> MAINNET keys; Testnet|Devnet -> TESTNET keys.

get_maintainer_keys(network) -> Vec<&'static str> (constants.rs:107): returns test keys if DOLI_TEST_KEYS=1 AND network=Devnet, else bootstrap keys.

is_using_placeholder_keys(network) -> bool (constants.rs:81): true if any key starts with "00000000".

assert_production_keys(network) (constants.rs:91): panics if placeholder keys detected. Call during node init.

### test_keys.rs
TEST_MAINTAINER_KEYS: LazyLock<[TestMaintainerKey; 5]> (test_keys.rs:42): deterministic from seeds 1-5.

sign_with_test_key(maintainer_index, message) -> Option<String> (test_keys.rs:63).

create_test_release_signatures(version, binary_sha256) -> Vec<(String,String)> (test_keys.rs:74): signs with first 3 test keys (minimum 3-of-5 required).

should_use_test_keys() -> bool (test_keys.rs:91): env var DOLI_TEST_KEYS=1.

## HARDFORK-SCHEDULE

Current entries (hardfork.rs:208-241):

Mainnet: NO entries. Genesis reset means all features active from h=0. REWARDS_EPOCH_LIST_FIX gated by constant in rewards.rs/schedule.rs (NOT in HardForkSchedule — adding an entry changes fork_id immediately).

Testnet h=3100, min_version="6.18.2": "EpochState state root inclusion (M-Choice1)" — INC-I-034 / M-Choice1.

Testnet h=4836, min_version="6.18.6": "Testnet HF deployment".

Devnet: NO entries. Devnet resets constantly, tests via fixtures.

REWARDS_EPOCH_LIST_FIX note: NOT in HardForkSchedule because adding an entry changes fork_id immediately. Rolling deploy safe; old binary diverges at h=13320 (Mainnet). Gated by REWARDS_EPOCH_LIST_FIX_HEIGHT constant in rewards.rs/schedule.rs.

fork_id algorithm (hardfork.rs:132): BLAKE3(genesis_hash || h1_le || h2_le || ...) over activation heights of ALL active forks sorted ascending. Pre-first-fork = Hash::ZERO. Used in peer handshake to partition legacy peers from post-HF peers.

Operator formula for real activation height: floor((current_height + 7200) / 360) * 360 -- aligns to next epoch boundary at least 2 hours ahead of deploy.

CLAUDE.md Rule #0: Activation heights are IMMUTABLE once crossed. Never move them forward. New features get their own height.

## DATA-FLOWS

Normal Update Flow:
  GitHub Release published
  -> CI creates CHECKSUMS.txt + tarball per platform
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
       -> install_binary() [atomic rename or sudo fallback]
  -> UpdateWatchdog.record_update(version)
  -> restart_node() [exec() on Unix]

Crash Detection / Rollback:
  Node starts -> UpdateWatchdog.check_and_maybe_rollback()
    -> if last_update_version set AND not clean_shutdown:
         push crash timestamp, prune window
         if crashes >= threshold (3): rollback() -> returns version
         caller: rollback() -> restart_node()
    -> if clean_shutdown: clear crash history -> None

Hard Fork Gating:
  try_produce_block()
    -> HardForkSchedule::for_network(network)
    -> schedule.should_stop_producing(current_height, current_version)
    -> true -> block production, log warning

Production Enforcement:
  UpdateService detects approved release
    -> VersionEnforcement::from_approved_release_with_params(release, &params)
    -> binary_ready = false (until download completes)
    -> download begins in background
    -> on success: binary_ready = true
    -> each produce attempt: check_production_allowed(Some(&enforcement))
         -> blocks only if: should_enforce() AND !version_meets_requirement() AND binary_ready AND not timed out (30min)

Signature Verification Key Selection:
  verify_release_signatures_with_keys(release, on_chain_keys, network):
    if on_chain_keys.is_empty():
      keys = bootstrap_maintainer_keys(network)  [compile-time static]
    else:
      keys = on_chain_keys  [first 5 registered producers on-chain]
    count valid Ed25519 sigs from known keys
    require >= REQUIRED_SIGNATURES (3)

## DEPENDENCIES

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
                -> tempfile (test only)

Network timing (veto_period_secs, grace_period_secs, seniority_step_blocks, etc.) comes from doli_core::Network methods -- not hardcoded in updater. Updater reads them via UpdateParams::for_network(network).

## CONSTRAINTS

Governance rules (no exceptions):
- ALL updates require veto period (configurable, currently 5 min; target 7 days mainnet)
- 40% producer veto threshold (weighted by bonds x seniority)
- 3-of-5 maintainer signatures required (Ed25519)
- REQUIRED_SIGNATURES = 3 (constants.rs:29)
- VETO_THRESHOLD_PERCENT = 40 (constants.rs:26)
- VETO_PERIOD = 5 * 60s (constants.rs:13) -- early network, target 7 days
- GRACE_PERIOD = 3600s (constants.rs:16)
- CHECK_INTERVAL = 6 * 3600s (constants.rs:116)

Production blocking rules (enforcement.rs:176):
- Blocked only when ALL true: enforcement_time passed + old version + binary_ready=true + elapsed < 30min
- Download failure -> warn + allow production (network must not halt for infra failure)
- ENFORCEMENT_TIMEOUT_SECS = 30 * 60 (enforcement.rs:62)

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
- is_using_placeholder_keys() must return false before mainnet launch
- assert_production_keys(network) panics on placeholder; call during node init

Hard fork constraints (from CLAUDE.md):
- Once activated on mainnet, activation height is IMMUTABLE -- never move forward
- NEVER add entries to HardForkSchedule for rolling deploys -- fork_id changes immediately
- For rolling-deploy-safe feature gates: use constant gates (e.g., REWARDS_EPOCH_LIST_FIX_HEIGHT)
- fork_id() filters active forks at the given height -- all entries past their height appear in fork_id immediately

TOCTOU protection (AUDIT-UPDATE-002):
- auto_apply_from_github() receives signed_checksums_sha256 (the hash maintainers actually signed)
- Re-fetches CHECKSUMS.txt and compares -- mismatch = abort (possible tampered release)
- expected_hash in GithubReleaseInfo = per-platform binary hash FROM CHECKSUMS.txt (not SHA256 of CHECKSUMS.txt itself)

Watchdog behavior:
- Persisted in {data_dir}/watchdog_state.json
- Default crash_threshold = 3 crashes within crash_window_secs
- Clean shutdown clears crash history -> no false rollback
- After rollback, state cleared -> no re-trigger

Platform identifiers: "linux-x64" | "linux-arm64" | "macos-x64" | "macos-arm64" | "unknown". Maps to Rust target triples for tarball asset matching.

install_binary fallback: On PermissionDenied (root-owned paths like /usr/local/bin/), uses sudo rm -f + sudo cp. On Linux, must delete before copy -- cp fails with "Text file busy" on running binary. Requires passwordless sudo for doli group.

## PATTERNS

Network-aware timing: Always use UpdateParams::for_network(network) instead of global constants. Mainnet/Testnet = production timing; Devnet = accelerated (veto=60s, grace=30s). UpdateConfig::default() uses mainnet defaults.

Key selection:
  In running node (has on-chain state): verify_release_signatures_with_keys(release, &on_chain_keys, network)
  In CLI (no on-chain state): verify_release_signatures(release, network)  // bootstrap keys
  In devnet tests: set DOLI_TEST_KEYS=1 -> get_maintainer_keys(Devnet) returns test keys

Hard fork schedule usage (node startup / block production):
  let schedule = HardForkSchedule::for_network(network);
  if schedule.should_stop_producing(current_height, current_version()) { /* pause */ }
  schedule.log_activations(current_height);  // at epoch boundary
  let id = schedule.fork_id(&genesis_hash, current_height);  // peer handshake

Adding a new hard fork (hardfork.rs:208):
  1. Add entry to HardForkSchedule::for_network() for affected networks
  2. Use far-future placeholder height (>= current_height + reasonable lead time)
  3. Operator updates real height before deploy: floor((current_height + 7200) / 360) * 360
  4. NEVER use HardForkSchedule for rolling-deploy features -- fork_id changes for all peers immediately
  5. For rolling-safe: use a constant height gate in the relevant module

VoteTracker weight storage:
  let w = params.calculate_vote_weight(bond_count, blocks_active);
  let weight_u64 = (w * 100.0) as u64;  // 100x multiplier for 2-decimal precision
  weights.insert(producer_id, weight_u64);
  let tracker = VoteTracker::with_weights(version, weights);
  tracker.should_reject_weighted(total_weight)  // uses same scale

Test keys activation (devnet CI only):
  DOLI_TEST_KEYS=1 -> should_use_test_keys()=true -> get_maintainer_keys(Devnet) returns test pubkeys
  -> create_test_release_signatures(version, sha256) -> 3 valid signatures for tests

Binary update chain of trust:
  Maintainers sign "{version}:{SHA256(CHECKSUMS.txt)}"
  -> SIGNATURES.json uploaded
  -> Node: CHECKSUMS.txt hash verified against signed value [TOCTOU]
  -> CHECKSUMS.txt parsed for per-platform tarball hash
  -> Tarball downloaded + SHA-256 verified against CHECKSUMS.txt entry
  -> Binary extracted from tarball

Enforcement timeout safety: If auto_apply_from_github fails (network error, wrong tarball name), binary_ready stays false -> production continues with warning. If enforcement_time + 30min passes with old version, enforcement auto-expires. Prevents indefinite production halt from infrastructure failures.

GitHub repo: e-weil/doli (constants.rs:120). API: https://api.github.com/repos/e-weil/doli/releases/latest. Fallback: https://releases.doli.network.
