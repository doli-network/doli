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
   - Creates GitHub Release with artifacts and empty SIGNATURES.json scaffold
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

*Last updated: March 2026*
