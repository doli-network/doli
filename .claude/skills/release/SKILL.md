---
name: release
description: Cut, sign, and publish a DOLI release, and deploy binaries to the mainnet fleet. Covers the post-tag draft -> sign -> verify -> promote -> confirm sequence (sign-release.sh, doli release verify, publish-release.sh, monitor-release-signed.sh), plus per-node binary layout, MD5 verification, and correct replacement procedure.
user_invocable: true
---

# DOLI Release — Binary Build & Deploy

## BLOCKING: Post-Tag Release Sequence (INC-I-202)

A "release" here means two independent things. (a) **Publishing the signed GitHub release** — this
is what auto-updating external producers consume. (b) **The manual per-node binary deploy** in the
rest of this file. Neither substitutes for the other: a published release does not move the fleet
binaries, and a fleet deploy does not give external producers anything to update to.

Run these six steps in this order. Do not stop early.

| # | Step | Command |
|---|------|---------|
| 1 | Tag + push | `git tag vX.Y.Z && git push origin main --tags` |
| 2 | CI builds | (automatic — creates the release as a **DRAFT**) |
| 3 | **SIGN** | `./scripts/sign-release.sh X.Y.Z` |
| 4 | **VERIFY** | `doli release verify --version vX.Y.Z` |
| 5 | **PROMOTE** | `./scripts/publish-release.sh X.Y.Z` |
| 6 | **CONFIRM** | `./scripts/monitor-release-signed.sh` |

**1. Tag + push**

```bash
git tag vX.Y.Z && git push origin main --tags
```

**2. CI builds all 7 artifacts and creates the GitHub Release as a DRAFT**

A draft is invisible to the unauthenticated GitHub API. At this point the release is **NOT public**
and **NOT reachable by any node**: no `doli upgrade` and no auto-updating producer can see it. CI
also injects an "UNSIGNED DRAFT" banner into the notes and writes a "Release ... created as DRAFT"
job step summary. A tag alone publishes nothing.

**3. SIGN — collect the 3-of-5 maintainer quorum**

```bash
./scripts/sign-release.sh X.Y.Z
```

Keys are the rotated maintainer wallets at `~/.ssh/doli/maintainer-{1,2,3}.json`. Override the
location with `KEY_DIR`, or individual files with `KEY_1` / `KEY_2` / `KEY_3`. `DOLI_CLI` overrides
the CLI path (default `./target/release/doli`, else `doli` on PATH). The script assembles
SIGNATURES.json and uploads it to the draft. It accepts `X.Y.Z` or `vX.Y.Z`.

**4. VERIFY — the same check the node runs**

```bash
doli release verify --version vX.Y.Z
```

Add `--dir <path>` to verify a local manifest directory. This resolves against this host's
maintainer trust root.

**5. PROMOTE — script only**

```bash
./scripts/publish-release.sh X.Y.Z
```

Downloads SIGNATURES.json + CHECKSUMS.txt from the draft, refuses a missing, malformed, or
sub-threshold manifest by name and count (`THRESHOLD` default 3), runs `doli release verify`, and
only on success strips the CI banner and runs `gh release edit vX.Y.Z --draft=false --latest`. Any
failure leaves it a draft. **Never** promote with a hand-run `gh release edit --draft=false`.

**6. CONFIRM — read-only standing check**

```bash
./scripts/monitor-release-signed.sh
```

Asserts the newest `v*` tag (version-sorted) has a published release that verifies. Env:
`DOLI_CLI`, `REPO_DIR`, `REPO`. It never mutates a release — safe from cron.

## CRITICAL: Per-Node Binary Layout

Core servers (ai1-ai5) use **per-node binary copies**, NOT the shared `doli-node`.
Each systemd service runs its own copy: `doli-node-{name}`. Replacing only `doli-node` does NOTHING — the running services won't pick up the change.

## Binary Map

### ai1 — seed + n1, n2, n3

| Service | Binary Path | Owner |
|---------|-------------|-------|
| `doli-mainnet-seed` | `/mainnet/bin/doli-node-seed` | root |
| `doli-mainnet-n1` | `/mainnet/bin/doli-node-n1` | root |
| `doli-mainnet-n2` | `/mainnet/bin/doli-node-n2` | root |
| `doli-mainnet-n3` | `/mainnet/bin/doli-node-n3` | root |

### ai2 — seed + n4, n5 (build server)

| Service | Binary Path | Owner |
|---------|-------------|-------|
| `doli-mainnet-seed` | `/mainnet/bin/doli-node-seed` | root |
| `doli-mainnet-n4` | `/mainnet/bin/doli-node-n4` | root |
| `doli-mainnet-n5` | `/mainnet/bin/doli-node-n5` | root |

Build output: `~/repos/doli/target/release/doli-node`
**WARNING**: ai2's `/tmp/` may contain stale binaries from previous deploys. Always copy from `~/repos/doli/target/release/` directly.

### ai3 — seed + ivan, santiago

| Service | Binary Path | Owner |
|---------|-------------|-------|
| `doli-mainnet-seed` | `/mainnet/bin/doli-node-seed` | root |
| `doli-mainnet-ivan` | `/mainnet/bin/doli-node-ivan` | root |
| `doli-mainnet-santiago` | `/mainnet/bin/doli-node-santiago` | root |

### ai4 — n6, n7, n8

| Service | Binary Path | Owner |
|---------|-------------|-------|
| `doli-mainnet-n6` | `/mainnet/bin/doli-node-n6` | root |
| `doli-mainnet-n7` | `/mainnet/bin/doli-node-n7` | root |
| `doli-mainnet-n8` | `/mainnet/bin/doli-node-n8` | root |

### ai5 — n9, n10, n11, n12

| Service | Binary Path | Owner |
|---------|-------------|-------|
| `doli-mainnet-n9` | `/mainnet/bin/doli-node-n9` | root |
| `doli-mainnet-n10` | `/mainnet/bin/doli-node-n10` | root |
| `doli-mainnet-n11` | `/mainnet/bin/doli-node-n11` | root |
| `doli-mainnet-n12` | `/mainnet/bin/doli-node-n12` | root |

### Personal Servers — shared `doli-node`

| Server | SSH Alias | Binary Path | Services |
|--------|-----------|-------------|----------|
| Family | `doli-server-family` | `/mainnet/bin/doli-node` | adri, abraham, ori, isudoajl, alan, alessandro |
| Folsi | `doli-server-folsi` | `/mainnet/bin/doli-node` | folsi, miri, pastora |
| Leandro | `doli-server-leandro` | `/usr/bin/doli-node` | leandro |
| Caraquita | `doli-server-caraquita` | `/usr/bin/doli-node` | daniel |
| Nano | `doli-server-nano` | `/usr/bin/doli-node` | doli-mainnet (no suffix) |

## Deploy Procedure Per Server

### Core servers (ai1-ai5) — per-node binaries, root-owned

```bash
# 1. Stop all services on the server
ssh {server} "sudo systemctl stop {service1} {service2} ..."

# 2. Replace EACH per-node binary (requires sudo)
ssh {server} "sudo cp /tmp/doli-node /mainnet/bin/doli-node-{name1} && sudo cp /tmp/doli-node /mainnet/bin/doli-node-{name2} && ..."

# 3. Verify MD5 of EACH per-node binary
ssh {server} "md5sum /mainnet/bin/doli-node-{name1} /mainnet/bin/doli-node-{name2} ..."

# 4. Start all services
ssh {server} "sudo systemctl start {service1} {service2} ..."

# 5. Verify active
ssh {server} "systemctl is-active {service1} {service2} ..."
```

### Personal servers (family, folsi) — shared binary

```bash
ssh {server} "sudo systemctl stop {all services} && rm -f /mainnet/bin/doli-node && cp /tmp/doli-node /mainnet/bin/doli-node && md5sum /mainnet/bin/doli-node && sudo systemctl start {all services}"
```

### Personal servers (leandro, caraquita, nano) — /usr/bin path

```bash
ssh {server} "sudo systemctl stop {service} && sudo cp /tmp/doli-node /usr/bin/doli-node && md5sum /usr/bin/doli-node && sudo systemctl start {service}"
```

## Quick Reference: One-Liner Per Server

```bash
# ai1
ssh ai1 "sudo systemctl stop doli-mainnet-seed doli-mainnet-n1 doli-mainnet-n2 doli-mainnet-n3 && sudo cp /tmp/doli-node /mainnet/bin/doli-node-seed && sudo cp /tmp/doli-node /mainnet/bin/doli-node-n1 && sudo cp /tmp/doli-node /mainnet/bin/doli-node-n2 && sudo cp /tmp/doli-node /mainnet/bin/doli-node-n3 && md5sum /mainnet/bin/doli-node-seed /mainnet/bin/doli-node-n1 /mainnet/bin/doli-node-n2 /mainnet/bin/doli-node-n3 && sudo systemctl start doli-mainnet-seed doli-mainnet-n1 doli-mainnet-n2 doli-mainnet-n3"

# ai2 (use build dir, NOT /tmp/)
ssh ai2 "sudo systemctl stop doli-mainnet-seed doli-mainnet-n4 doli-mainnet-n5 && sudo cp ~/repos/doli/target/release/doli-node /mainnet/bin/doli-node-seed && sudo cp ~/repos/doli/target/release/doli-node /mainnet/bin/doli-node-n4 && sudo cp ~/repos/doli/target/release/doli-node /mainnet/bin/doli-node-n5 && md5sum /mainnet/bin/doli-node-seed /mainnet/bin/doli-node-n4 /mainnet/bin/doli-node-n5 && sudo systemctl start doli-mainnet-seed doli-mainnet-n4 doli-mainnet-n5"

# ai3
ssh ai3 "sudo systemctl stop doli-mainnet-seed doli-mainnet-ivan doli-mainnet-santiago && sudo cp /tmp/doli-node /mainnet/bin/doli-node-seed && sudo cp /tmp/doli-node /mainnet/bin/doli-node-ivan && sudo cp /tmp/doli-node /mainnet/bin/doli-node-santiago && md5sum /mainnet/bin/doli-node-seed /mainnet/bin/doli-node-ivan /mainnet/bin/doli-node-santiago && sudo systemctl start doli-mainnet-seed doli-mainnet-ivan doli-mainnet-santiago"

# ai4
ssh ai4 "sudo systemctl stop doli-mainnet-n6 doli-mainnet-n7 doli-mainnet-n8 && sudo cp /tmp/doli-node /mainnet/bin/doli-node-n6 && sudo cp /tmp/doli-node /mainnet/bin/doli-node-n7 && sudo cp /tmp/doli-node /mainnet/bin/doli-node-n8 && md5sum /mainnet/bin/doli-node-n6 /mainnet/bin/doli-node-n7 /mainnet/bin/doli-node-n8 && sudo systemctl start doli-mainnet-n6 doli-mainnet-n7 doli-mainnet-n8"

# ai5
ssh ai5 "sudo systemctl stop doli-mainnet-n9 doli-mainnet-n10 doli-mainnet-n11 doli-mainnet-n12 && sudo cp /tmp/doli-node /mainnet/bin/doli-node-n9 && sudo cp /tmp/doli-node /mainnet/bin/doli-node-n10 && sudo cp /tmp/doli-node /mainnet/bin/doli-node-n11 && sudo cp /tmp/doli-node /mainnet/bin/doli-node-n12 && md5sum /mainnet/bin/doli-node-n9 /mainnet/bin/doli-node-n10 /mainnet/bin/doli-node-n11 /mainnet/bin/doli-node-n12 && sudo systemctl start doli-mainnet-n9 doli-mainnet-n10 doli-mainnet-n11 doli-mainnet-n12"

# family (shared binary)
ssh doli-server-family "sudo systemctl stop doli-mainnet-adri doli-mainnet-abraham doli-mainnet-ori doli-mainnet-isudoajl doli-mainnet-alan doli-mainnet-alessandro && rm -f /mainnet/bin/doli-node && cp /tmp/doli-node /mainnet/bin/doli-node && cp /tmp/doli /mainnet/bin/doli && md5sum /mainnet/bin/doli-node && sudo systemctl start doli-mainnet-adri doli-mainnet-abraham doli-mainnet-ori doli-mainnet-isudoajl doli-mainnet-alan doli-mainnet-alessandro"

# folsi (shared binary)
ssh doli-server-folsi "sudo systemctl stop doli-mainnet-folsi doli-mainnet-miri doli-mainnet-pastora && rm -f /mainnet/bin/doli-node && cp /tmp/doli-node /mainnet/bin/doli-node && cp /tmp/doli /mainnet/bin/doli && md5sum /mainnet/bin/doli-node && sudo systemctl start doli-mainnet-folsi doli-mainnet-miri doli-mainnet-pastora"

# leandro (/usr/bin path)
ssh doli-server-leandro "sudo systemctl stop doli-mainnet-leandro && sudo cp /tmp/doli-node /usr/bin/doli-node && sudo cp /tmp/doli /usr/bin/doli && md5sum /usr/bin/doli-node && sudo systemctl start doli-mainnet-leandro"

# caraquita (/usr/bin path)
ssh doli-server-caraquita "sudo systemctl stop doli-mainnet-daniel && sudo cp /tmp/doli-node /usr/bin/doli-node && sudo cp /tmp/doli /usr/bin/doli && md5sum /usr/bin/doli-node && sudo systemctl start doli-mainnet-daniel"

# nano (/usr/bin path, needs sudo cp)
ssh doli-server-nano "sudo systemctl stop doli-mainnet && sudo cp /tmp/doli-node /usr/bin/doli-node && sudo cp /tmp/doli /usr/bin/doli && md5sum /usr/bin/doli-node && sudo systemctl start doli-mainnet"
```

## Rules

1. **NEVER copy only to `doli-node`** on core servers — services run `doli-node-{name}`
2. **Per-node binaries are root-owned** — always use `sudo cp`
3. **MD5 at every hop**: build → local /tmp → remote /tmp → final path
4. **ai2 /tmp/ is unreliable** — always copy from `~/repos/doli/target/release/`
5. **Seeds before producers** on servers that run both
6. **Stop ALL services** on a server before replacing (shared binary → "Text file busy")
7. **Always use `systemctl stop/start`** — never `kill`/`pkill`
8. **A tag alone publishes nothing** — CI leaves the release a DRAFT that no node can reach; steps 3-6 above are mandatory (INC-I-202)
9. **Never promote by hand** — only `./scripts/publish-release.sh X.Y.Z` may clear the draft flag; a hand-run `gh release edit --draft=false` skips verification
10. **Never sign with the leaked pre-rotation producer key files** committed under `testnet/keys/` (INC-I-175) — their private halves are public; signing keys are the rotated wallets at `~/.ssh/doli/maintainer-{1,2,3}.json`
