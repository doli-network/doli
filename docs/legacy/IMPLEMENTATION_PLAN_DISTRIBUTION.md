# Release Infrastructure Implementation Plan

This plan establishes a user-friendly distribution system so node operators can run DOLI without building from source.

---

## Overview

**Goal:** Enable users to run a DOLI node with a single download or command.

**Current State:** Users must clone repo, install Nix or system dependencies, and compile from source.

**Target State:**
- `wget` + `./doli-node run` (pre-built binary)
- `docker run doli/node` (containerized)

---

## Milestone 1: Dockerfile for Node Operators

Create a production-ready Docker image for running DOLI nodes.

### Task 1.1: Create Multi-Stage Dockerfile

**File:** `Dockerfile`

**Requirements:**
- Multi-stage build (builder + runtime)
- Builder stage: Rust toolchain + all build dependencies
- Runtime stage: Minimal base image with only runtime dependencies
- Support for all three networks (mainnet/testnet/devnet)
- Expose P2P, RPC, and metrics ports
- Volume mount for persistent data

**Acceptance Criteria:**
- [x] `docker build -t doli-node .` succeeds
- [x] `docker run doli-node` starts a node
- [x] Image size < 200MB (runtime stage)
- [x] Data persists across container restarts

### Task 1.2: Create Docker Compose Configuration

**File:** `docker-compose.yml`

**Requirements:**
- Service definition for doli-node
- Named volume for blockchain data
- Network configuration for each environment
- Optional Prometheus/Grafana stack for monitoring

**Acceptance Criteria:**
- [x] `docker compose up` starts a working node
- [x] `docker compose -f docker-compose.testnet.yml up` works
- [x] Data directory properly mounted

### Task 1.3: Create Docker Documentation

**File:** `docs/docker.md`

**Requirements:**
- Quick start guide
- Configuration options via environment variables
- Volume management
- Networking guide (port exposure)
- Producer mode setup
- Troubleshooting section

**Acceptance Criteria:**
- [x] New user can run a node following only this guide
- [x] All environment variables documented

---

## Milestone 2: Static Binary Builds

Create fully static binaries that run on any Linux without dependencies.

### Task 2.1: Add musl Target Support

**File:** `Cross.toml` (new) or updates to build scripts

**Requirements:**
- Configure `x86_64-unknown-linux-musl` target
- Handle static linking for: OpenSSL, GMP, RocksDB
- Use `cross` or custom Docker build environment

**Acceptance Criteria:**
- [x] `cargo build --release --target x86_64-unknown-linux-musl` succeeds
- [x] Binary runs on Alpine Linux (musl-based)
- [x] Binary runs on Ubuntu without installing any packages
- [x] `ldd` shows "statically linked" or "not a dynamic executable"

### Task 2.2: macOS Build Configuration

**Requirements:**
- Support `x86_64-apple-darwin` (Intel)
- Support `aarch64-apple-darwin` (Apple Silicon)
- Handle macOS-specific linking (Security framework, etc.)
- Create universal binary if feasible

**Acceptance Criteria:**
- [x] Binary runs on macOS 13+ Intel
- [x] Binary runs on macOS 13+ Apple Silicon
- [x] No Homebrew dependencies required at runtime

### Task 2.3: Create Build Script

**File:** `scripts/build_release.sh`

**Requirements:**
- Build for all supported targets
- Generate checksums (SHA256)
- Create tarball with binary + README
- Version tagging from git or Cargo.toml

**Acceptance Criteria:**
- [x] `./scripts/build_release.sh` produces all artifacts
- [x] Each artifact has accompanying `.sha256` file
- [x] Artifacts follow naming: `doli-node-{version}-{target}.tar.gz`

---

## Milestone 3: GitHub Actions Release Pipeline

Automate building and publishing releases on git tags.

### Task 3.1: Create CI Workflow for PRs

**File:** `.github/workflows/ci.yml`

**Requirements:**
- Trigger on pull requests and pushes to main
- Run `cargo fmt --check`
- Run `cargo clippy`
- Run `cargo test`
- Cache cargo registry and target directory

**Acceptance Criteria:**
- [x] PRs show pass/fail status
- [x] Build time < 15 minutes with caching
- [x] All checks must pass before merge

### Task 3.2: Create Release Workflow

**File:** `.github/workflows/release.yml`

**Requirements:**
- Trigger on tag push (`v*`)
- Build matrix:
  - `x86_64-unknown-linux-gnu`
  - `x86_64-unknown-linux-musl`
  - `x86_64-apple-darwin`
  - `aarch64-apple-darwin`
- Build Docker image and push to Docker Hub / GHCR
- Generate release notes from commits
- Upload artifacts to GitHub Releases
- Create checksums file

**Acceptance Criteria:**
- [x] `git tag v1.0.0 && git push --tags` triggers release
- [x] All 4 binary targets built and uploaded
- [x] Docker image pushed with version tag
- [x] Release page shows download links and checksums

### Task 3.3: Create Docker Publish Workflow

**File:** `.github/workflows/docker.yml` (or part of release.yml)

**Requirements:**
- Build and push to GitHub Container Registry (ghcr.io)
- Tag with version and `latest`
- Multi-arch image (amd64 + arm64) using buildx

**Acceptance Criteria:**
- [x] `docker pull ghcr.io/e-weil/doli-node:latest` works
- [x] `docker pull ghcr.io/e-weil/doli-node:v1.0.0` works
- [x] Image works on both amd64 and arm64

---

## Milestone 4: Documentation Updates

Update all user-facing docs to reflect new installation methods.

### Task 4.1: Update running_a_node.md

**Requirements:**
- Add "Download Pre-built Binary" as first/recommended option
- Add Docker installation section
- Keep source build as alternative
- Remove Nix as "recommended" for end users

**Acceptance Criteria:**
- [x] Binary download is the first listed option
- [x] Docker option documented
- [x] Clear for non-technical users

### Task 4.2: Update README.md

**Requirements:**
- Add quick start with pre-built binary
- Add Docker quick start
- Link to releases page
- Add badges (CI status, latest release, Docker pulls)

**Acceptance Criteria:**
- [x] README shows simplest path to running a node
- [x] Badges display correctly

### Task 4.3: Create RELEASES.md

**File:** `docs/releases.md` (lowercase per naming convention)

**Requirements:**
- Document release process
- Explain version numbering (semver)
- List supported platforms
- Document how to verify checksums
- GPG signing instructions (future)

**Acceptance Criteria:**
- [x] Release maintainers can follow this guide
- [x] Users can verify download integrity

---

## Milestone 5: Release Verification

Ensure releases work correctly before announcing.

### Task 5.1: Create Smoke Test Script

**File:** `scripts/smoke_test_release.sh`

**Requirements:**
- Download release artifact
- Verify checksum
- Start node in devnet mode
- Verify RPC responds
- Verify P2P port listening
- Clean shutdown

**Acceptance Criteria:**
- [x] Script exits 0 on success, non-zero on failure
- [x] Can run in CI after release build

### Task 5.2: Add Integration Test to Release Pipeline

**Requirements:**
- Run smoke test on each built binary
- Test Docker image starts correctly
- Fail release if smoke test fails

**Acceptance Criteria:**
- [x] Broken binaries don't get released
- [x] Release workflow includes test step

---

## Implementation Order

```
Week 1: Milestone 1 (Docker)
  ├── Task 1.1: Dockerfile
  ├── Task 1.2: Docker Compose
  └── Task 1.3: Docker docs

Week 2: Milestone 2 (Static Builds)
  ├── Task 2.1: musl target
  ├── Task 2.2: macOS builds
  └── Task 2.3: Build script

Week 3: Milestone 3 (GitHub Actions)
  ├── Task 3.1: CI workflow
  ├── Task 3.2: Release workflow
  └── Task 3.3: Docker publish

Week 4: Milestone 4 & 5 (Docs & Verification)
  ├── Task 4.1-4.3: Documentation
  └── Task 5.1-5.2: Smoke tests
```

---

## Success Metrics

| Metric | Target |
|--------|--------|
| Time to first block (new user) | < 5 minutes |
| Binary download size | < 50MB compressed |
| Docker image size | < 200MB |
| Supported platforms | 4 (Linux x64, Linux x64 static, macOS x64, macOS ARM) |
| CI build time | < 20 minutes |

---

## Dependencies & Prerequisites

- GitHub repository access for Actions
- Docker Hub or GHCR account for image hosting
- macOS runner access (GitHub-hosted or self-hosted)
- Code signing certificate (optional, for macOS notarization)

---

## Future Enhancements (Out of Scope)

- [ ] Windows builds
- [ ] APT/YUM package repositories
- [ ] Homebrew formula
- [ ] GPG-signed releases
- [ ] macOS notarization
- [ ] Reproducible builds verification

---

*Created: January 2026*
