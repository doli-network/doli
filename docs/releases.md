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

All releases are published to: https://github.com/doli-network/doli/releases

```bash
# Download latest release (Linux x64 static)
VERSION=$(curl -s https://api.github.com/repos/doli-network/doli/releases/latest | grep tag_name | cut -d'"' -f4)
curl -LO "https://github.com/doli-network/doli/releases/download/${VERSION}/doli-${VERSION}-x86_64-unknown-linux-musl.tar.gz"

# Extract
tar xzf doli-${VERSION}-x86_64-unknown-linux-musl.tar.gz

# Install
sudo mv doli-node doli /usr/local/bin/
```

### Install Script

The easiest way to install or update:

```bash
# Install latest version
curl -L https://raw.githubusercontent.com/doli-network/doli/main/scripts/update.sh | bash

# Install specific version
curl -L https://raw.githubusercontent.com/doli-network/doli/main/scripts/update.sh | bash -s v1.0.0
```

### Docker Images

```bash
# Pull from GitHub Container Registry
docker pull ghcr.io/doli-network/doli-node:latest
docker pull ghcr.io/doli-network/doli-node:v1.0.0
```

---

## Verifying Downloads

Always verify downloads before running to ensure integrity and authenticity.

### SHA-256 Checksums

Each release includes checksum files:

```bash
# Download checksums
curl -LO https://github.com/doli-network/doli/releases/download/v1.0.0/SHA256SUMS.txt

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
docker inspect ghcr.io/doli-network/doli-node:v1.0.0 --format='{{.RepoDigests}}'

# Pull by digest (immutable)
docker pull ghcr.io/doli-network/doli-node@sha256:<digest>
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

### Creating a Release

1. **Update version in Cargo.toml:**
   ```bash
   # Update workspace version
   vim Cargo.toml  # Change version = "X.Y.Z"
   ```

2. **Create and push tag:**
   ```bash
   git add Cargo.toml
   git commit -m "chore: bump version to vX.Y.Z"
   git tag vX.Y.Z
   git push origin main --tags
   ```

3. **GitHub Actions automatically:**
   - Builds binaries for all platforms
   - Builds multi-arch Docker images
   - Creates the GitHub Release **as a DRAFT** with artifacts and an empty
     SIGNATURES.json scaffold (INC-I-202: a draft is invisible to the unauthenticated
     API, so no node and no `doli upgrade` can reach an unsigned release)
   - Generates release notes from commits

4. **Sign the release (after CI completes):**
   ```bash
   # Option A: If gh CLI is on the signing machine
   ./scripts/sign-release.sh X.Y.Z

   # Option B: Split workflow (keys on omegacortex, gh on Mac)
   # See .claude/skills/doli-ops/SKILL.md Section 4.6 for full procedure
   # Summary:
   #   1. SSH to omegacortex, sign with producer keys 1-3 using doli release sign
   #   2. SCP the assembled SIGNATURES.json to Mac
   #   3. gh release delete-asset + upload from Mac
   #   4. Verify with: gh release download vX.Y.Z --pattern SIGNATURES.json
   ```

5. **Publish the draft (only after signing):**
   ```bash
   ./scripts/publish-release.sh X.Y.Z
   ```
   The script downloads SIGNATURES.json + CHECKSUMS.txt from the draft, refuses a
   missing, malformed, or sub-threshold manifest by name and count, runs
   `doli release verify --version vX.Y.Z --dir <tmp>` against this host's maintainer
   trust root, and only then runs `gh release edit vX.Y.Z --draft=false --latest`.
   On success it also strips the CI unsigned-draft banner from the release notes
   before promoting; a failed verification never touches the notes. Any failure
   leaves the release a draft. Never promote by hand.

6. **Monitor that the newest release stays healthy (recurring, read-only):**
   ```bash
   ./scripts/monitor-release-signed.sh
   ```
   Single predicate: the newest `v*` tag (version-sorted) has a **published**
   (non-draft) GitHub release whose signatures **verify**. Exit code 0 means
   healthy; any non-zero exit names the tag and the fix (`sign-release.sh` or
   `publish-release.sh`). It never writes to any release — safe to run from
   cron or by hand. Two env vars: `DOLI_CLI` (doli binary path) and `REPO_DIR`
   (repo whose tags to read; defaults to this checkout).

   Suggested cron line (**not installed by this repo** — no crontab entry is
   added and no remote host is touched):
   ```
   */15 * * * * cd /path/to/doli && ./scripts/monitor-release-signed.sh || echo "release unhealthy" | mail -s "DOLI release alert" you@example.com
   ```
   Edit a crontab with `crontab -e`, interactively. **Never** pipe
   `crontab -l` through `sed` to edit it — an empty `sed` output silently
   wipes the whole crontab.

### Release Checklist

- [ ] All tests passing on main branch
- [ ] Version bumped in Cargo.toml
- [ ] CHANGELOG.md updated (if maintained)
- [ ] Tag created and pushed
- [ ] GitHub Actions workflow completed
- [ ] Binaries tested on target platforms
- [ ] Docker images verified
- [ ] Release notes reviewed
- [ ] SIGNATURES.json signed by 3/5 maintainers (see [auto_update_system.md](./auto_update_system.md))
- [ ] SIGNATURES.json uploaded to release artifacts
- [ ] **BLOCKING — draft promoted with `./scripts/publish-release.sh X.Y.Z`**, never with
      a hand-run `gh release edit --draft=false` (INC-I-202)
- [ ] **BLOCKING — maintainer-rotation ordering checked** (see
      [Maintainer rotation: mandatory release ordering](#maintainer-rotation-mandatory-release-ordering)
      below). Violating this order stops auto-update on every node in the fleet.

---

## Maintainer rotation: mandatory release ordering

**This section is a hard ordering constraint, not advice. Read it before any release that
carries a maintainer-set change, and before any release that crosses
`maintainer_derivation_activation_height`.**

The trust-root containment guard added in INC-I-172 M1 refuses **any** on-chain maintainer set
whose members differ from the compiled bootstrap five: `TrustRoot::resolve` returns an empty
on-chain root, `is_usable()` is false, and **every release is refused on every host**. Before
INC-I-172 M2 that never latched, because an on-chain rotation reverted within one block. M2 makes
a rotation **durable**, so the refusal becomes durable too.

### The required order

1. **Ship the containment lift first.** The height-aware `TrustRoot::resolve` (INC-I-172 R4) must
   be released, and **fleet adoption confirmed**, BEFORE mainnet crosses
   `maintainer_derivation_activation_height`.
2. **Then, and only then, submit the first `AddMaintainer` / `RemoveMaintainer`.** Do not submit a
   rotation transaction while any node still runs a binary without the lift.

### What breaks if the order is violated

The instant the first rotation succeeds above the gate, **every node stops accepting any release** —
including the external auto-updating producers that cannot be reached by SSH. The binary that lifts
the containment then has to be delivered through the channel the rotation just closed. Recovery is
manual, host by host. An adversary holding the current quorum can trigger this deliberately with one
transaction: a cheap, fleet-wide, unattended denial of the security-patch pipeline.

It is **fail-closed**, so it is not a key-compromise hole. It is an operational trap, and it fires on
the exact action the maintainer-rotation work exists to enable.

Detail and evidence: `docs/.workflow/inc-i-172-M3-scope.md` §R4 and §R6; finding **AUDIT-P1-017** in
`docs/.workflow/security-audit-report-M2.md`.

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
curl -LO https://github.com/doli-network/doli/releases/download/v1.0.0/doli-v1.0.0-x86_64-unknown-linux-musl.tar.gz

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

*Last updated: March 2026*
