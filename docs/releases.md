# releases.md - Release Process & Verification

This document covers the DOLI release process, versioning scheme, and how to verify downloads.

---

## Versioning

DOLI follows [Semantic Versioning](https://semver.org/) (SemVer):

```
v{MAJOR}.{MINOR}.{PATCH}[-{PRERELEASE}]

Examples:
  v1.0.0      - First stable release
  v1.1.0      - New features, backward compatible
  v1.1.1      - Bug fixes only
  v2.0.0      - Breaking changes
  v1.2.0-rc1  - Release candidate
  v1.2.0-beta - Beta release
```

### Version Bumping Rules

| Change Type | Version Bump | Example |
|-------------|--------------|---------|
| Breaking protocol changes | MAJOR | v1.0.0 → v2.0.0 |
| New features (backward compatible) | MINOR | v1.0.0 → v1.1.0 |
| Bug fixes, performance improvements | PATCH | v1.0.0 → v1.0.1 |
| Pre-release versions | PRERELEASE | v1.1.0-rc1 |

---

## Supported Platforms

DOLI provides pre-built binaries for the following platforms:

| Platform | Target | Binary Type |
|----------|--------|-------------|
| Linux x64 | `x86_64-unknown-linux-gnu` | Dynamically linked |
| Linux x64 (static) | `x86_64-unknown-linux-musl` | Statically linked (recommended) |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | Dynamically linked |
| Linux ARM64 (static) | `aarch64-unknown-linux-musl` | Statically linked |
| macOS Intel | `x86_64-apple-darwin` | macOS 13+ |
| macOS Apple Silicon | `aarch64-apple-darwin` | macOS 13+ (M1/M2/M3) |

**Recommended:** Use the `musl` (static) builds for Linux - they run on any Linux distribution without dependencies.

---

## Downloading Releases

### GitHub Releases

All releases are published to: https://github.com/e-weil/doli/releases

```bash
# Download latest release (Linux x64 static)
VERSION=$(curl -s https://api.github.com/repos/e-weil/doli/releases/latest | grep tag_name | cut -d'"' -f4)
curl -LO "https://github.com/e-weil/doli/releases/download/${VERSION}/doli-${VERSION}-x86_64-unknown-linux-musl.tar.gz"

# Extract
tar xzf doli-${VERSION}-x86_64-unknown-linux-musl.tar.gz

# Install
sudo mv doli-node doli /usr/local/bin/
```

### Install Script

The easiest way to install or update:

```bash
# Install latest version
curl -L https://raw.githubusercontent.com/e-weil/doli/main/scripts/update.sh | bash

# Install specific version
curl -L https://raw.githubusercontent.com/e-weil/doli/main/scripts/update.sh | bash -s v1.0.0
```

### Docker Images

```bash
# Pull from GitHub Container Registry
docker pull ghcr.io/e-weil/doli-node:latest
docker pull ghcr.io/e-weil/doli-node:v1.0.0
```

---

## Verifying Downloads

Always verify downloads before running to ensure integrity and authenticity.

### SHA-256 Checksums

Each release includes checksum files:

```bash
# Download checksums
curl -LO https://github.com/e-weil/doli/releases/download/v1.0.0/SHA256SUMS.txt

# Verify
sha256sum -c SHA256SUMS.txt --ignore-missing
```

Example output:
```
doli-v1.0.0-x86_64-unknown-linux-musl.tar.gz: OK
```

### Manual Verification

```bash
# Calculate checksum
sha256sum doli-v1.0.0-x86_64-unknown-linux-musl.tar.gz

# Compare with published checksum
cat SHA256SUMS.txt | grep x86_64-unknown-linux-musl
```

### Verifying Docker Images

```bash
# Check image digest
docker inspect ghcr.io/e-weil/doli-node:v1.0.0 --format='{{.RepoDigests}}'

# Pull by digest (immutable)
docker pull ghcr.io/e-weil/doli-node@sha256:<digest>
```

---

## Release Artifacts

Each release includes:

| File | Description |
|------|-------------|
| `doli-{version}-{target}.tar.gz` | Binary tarball |
| `doli-{version}-{target}.tar.gz.sha256` | Individual checksum |
| `SHA256SUMS.txt` | Combined checksums for all platforms |
| `sbom.spdx.json` | Software Bill of Materials |

### Tarball Contents

```
doli-v1.0.0-x86_64-unknown-linux-musl/
├── doli-node    # Node binary
├── doli         # CLI binary
└── README.txt   # Quick start instructions
```

---

## Release Process (For Maintainers)

### Two types of releases

1. **Code-only release** — bug fixes, features, consensus changes with forward activation height. No genesis reset. Existing chain data is valid.
2. **Genesis reset release** — new genesis timestamp. All chain data must be wiped. See [genesis.md](./genesis.md) for the full genesis procedure.

### Code-only release procedure

**Step 1: Pre-flight checks**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test -p doli-core --lib
cargo test -p network --lib
```

**Step 2: Bump version and tag**
```bash
# Edit Cargo.toml version
sed -i 's/^version = "OLD"/version = "NEW"/' Cargo.toml
cargo generate-lockfile
git add Cargo.toml Cargo.lock
git commit --author "Ivan D. Lozada <ivan@doli.network>" -m "bump: vX.Y.Z — description"
git tag -a vX.Y.Z -m "vX.Y.Z — description"
git push origin main --tags
```

**Step 3: CI builds automatically** (~15 min)
- Builds binaries for Linux x86_64, macOS aarch64
- Creates GitHub Release with tarballs + CHECKSUMS.txt + empty SIGNATURES.json
- Builds GUI (Tauri) for all platforms

**Step 4: Build on ai2 and distribute**
```bash
ssh ai2 "source ~/.cargo/env && cd ~/repos/doli && git fetch origin && git reset --hard origin/main && cargo clean && LIBZ_SYS_STATIC=0 cargo build --release"
ssh ai2 "~/repos/doli/target/release/doli-node --version && md5sum ~/repos/doli/target/release/doli-node"
# Pull and distribute to all servers (see deploy procedure)
```

**Step 5: Sign release (AFTER CI completes)**
```bash
# Verify CI finished
gh run list --workflow=release.yml --limit 1 --json status,conclusion --jq '.[0]'

# Sign with 3 maintainers — they fetch CI's CHECKSUMS.txt automatically
ssh ai1 "/mainnet/bin/doli -w /mainnet/n1/keys/producer.json release sign --version vX.Y.Z"
ssh ai1 "/mainnet/bin/doli -w /mainnet/n2/keys/producer.json release sign --version vX.Y.Z"
ssh ai1 "/mainnet/bin/doli -w /mainnet/n3/keys/producer.json release sign --version vX.Y.Z"

# Assemble SIGNATURES.json from the 3 outputs (version + checksums_sha256 + signatures array)
# Upload:
gh release upload vX.Y.Z /tmp/SIGNATURES.json --clobber
```

**CRITICAL: NEVER sign before CI completes.** CI overwrites CHECKSUMS.txt with all-platform checksums. Signing before CI = invalid signatures = `doli upgrade` fails with "0/3 signatures".

**Step 6: Rolling deploy** (server by server, never all at once)
```bash
ssh $SERVER "sudo systemctl stop $SERVICES && sudo cp /tmp/doli-node-vX /mainnet/bin/doli-node && sudo chmod +x /mainnet/bin/doli-node && /mainnet/bin/doli-node --version && sudo systemctl start $SERVICES"
```

### Genesis reset release procedure

When a release includes a new genesis timestamp (chain reset):

1. Follow **all steps above** for the code release
2. **Additionally** follow the genesis procedure in [genesis.md](./genesis.md):
   - Update 3 files: `chainspec.mainnet.json`, `constants.rs`, `chainspec.rs`
   - Update genesis hash in test
   - Verify: `cargo test -p doli-core --lib -- test_mainnet_genesis_hash_hardcoded test_genesis_time`
3. **ALL servers must have the new binary AND wiped data before ANY node starts**
4. Startup validation prevents stale data: `StateDb genesis hash mismatch → crash`

### Consensus safety: fork_id + genesis_hash

Every block header contains two chain identity fields:

| Field | What it validates | When it changes |
|-------|-------------------|-----------------|
| `genesis_hash` | Chain identity (timestamp + network + slot_duration + message) | Only on genesis reset |
| `fork_id` | Active hard fork set (BLAKE3 of genesis_hash + sorted activation heights) | When new `HardForkSchedule` entries activate |

Both are checked at gossip level (O(1) drop) and validation level. A node with wrong genesis or wrong fork set cannot produce blocks that any peer accepts.

**How fork_id works (inspired by Ethereum EIP-2124):**

1. Each consensus-breaking change adds an entry to `HardForkSchedule` in `crates/updater/src/hardfork.rs` with an `activation_height` and `min_version`.
2. `fork_id = BLAKE3(genesis_hash || h1_le || h2_le || ...)` where h1, h2... are all activation heights in the schedule. Computed at startup using `u64::MAX` as height — includes ALL known forks regardless of current chain height.
3. The fork_id is embedded in every block header and committed to the block hash.

**Partitioning behavior:**

- **Before activation height**: All nodes (old and new binary) produce the same fork_id. Both versions coexist on the network. No urgency to update.
- **At activation height**: Nodes with the new binary apply new consensus rules. Nodes with the old binary apply old rules. Blocks produced by old nodes are **rejected** by new nodes (fork_id mismatch at gossip level, O(1) drop). Old nodes are effectively partitioned from the network.
- **After activation height**: Old nodes cannot produce accepted blocks, cannot sync, and cannot participate. They must update their binary to rejoin.

**Operational impact for node operators:**

- **Code-only releases** (no `HardForkSchedule` entry): fork_id does not change. Nodes can update at any time. No deadline.
- **Consensus releases** (new `HardForkSchedule` entry): fork_id changes at activation height. **All nodes must update before the activation height or they will be isolated from the network.** The `min_version` field in the schedule determines which versions are compatible.
- The auto-updater (`doli upgrade`) handles this automatically for nodes with auto-update enabled. Nodes with `--no-auto-update` must update manually before the deadline.

**How to check:**

```bash
# Current fork_id (same for all nodes with same binary):
doli-node --version  # Shows version — check against min_version in schedule

# Check if a hard fork is approaching:
# Look in crates/updater/src/hardfork.rs for entries with activation_height > current_height
```

**Startup protection**: If a node has stale data from a previous chain, it crashes immediately:
```
StateDb genesis hash mismatch!
StateDb has:    <old hash>
Chainspec has:  <new hash>
Fix: wipe data directory and restart to re-sync from peers.
```

### Release checklist

- [ ] All tests passing (`cargo test -p doli-core --lib && cargo test -p network --lib`)
- [ ] `cargo fmt --check` clean
- [ ] `cargo clippy -- -D warnings` clean
- [ ] Version bumped in Cargo.toml + Cargo.lock regenerated
- [ ] If genesis reset: 3 files updated + genesis hash test passes
- [ ] Tag created and pushed
- [ ] CI Release workflow completed successfully
- [ ] Binary built on ai2 with `cargo clean` (NEVER incremental)
- [ ] md5 verified on ALL 10 servers before starting
- [ ] If genesis reset: ALL servers wiped before starting
- [ ] SIGNATURES.json signed by 3 maintainers (N1, N2, N3) AFTER CI completes
- [ ] SIGNATURES.json uploaded with `--clobber`
- [ ] `doli upgrade` tested from external node

---

## Auto-Update System

DOLI nodes can automatically update to new versions:

1. **Notification:** Node detects new release (checks every 10 minutes)
2. **Veto Period:** 5-minute window for producers to reject (early network; target: 7 days)
3. **Grace Period:** 2 minutes to apply update (early network; target: 48 hours)
4. **Enforcement:** Outdated nodes cannot produce blocks

```bash
# Check for updates
doli-node update status

# Manual update
doli-node update apply

# Disable auto-updates
doli-node run --no-auto-update
```

See [auto_update_system.md](./auto_update_system.md) for details.

---

## Rollback

If a release causes issues:

```bash
# Download previous version
curl -LO https://github.com/e-weil/doli/releases/download/v1.0.0/doli-v1.0.0-x86_64-unknown-linux-musl.tar.gz

# Stop node
sudo systemctl stop doli-node

# Replace binary
sudo cp doli-node /usr/local/bin/

# Start node
sudo systemctl start doli-node
```

---

## Future Enhancements

Planned security improvements:

- [ ] GPG-signed releases
- [ ] Reproducible builds verification
- [ ] macOS code signing and notarization
- [ ] APT/YUM package repositories
- [ ] Homebrew formula

---

*Last updated: April 2026*
