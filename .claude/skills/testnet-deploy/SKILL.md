# SKILL: Testnet Development & Deployment

> **Classification**: Internal QA Infrastructure
> **Scope**: Development, compilation, distribution, and validation of DOLI testnet binaries
> **Owner**: E. Weil
> **Last updated**: 2026-03-07 (rev2: post-deployment lessons applied)

---

## 0. GOLDEN RULES (NON-NEGOTIABLE)

| # | Rule | Consequence of Violation |
|---|------|--------------------------|
| G1 | **NEVER create a GitHub Release** — no `gh release create`, no draft releases, no tag-based releases | Mainnet contamination |
| G2 | **NEVER tag or bump versions** — no `git tag`, no `cargo set-version`, no version edits in `Cargo.toml` | False release signal |
| G3 | **NEVER touch mainnet binaries or services** — no copy/restart of any `doli-mainnet-*` service | Production outage |
| G4 | **NEVER deploy to mainnet paths** — `/opt/doli/target/release/`, `~/repos/doli/target/release/` on omegacortex are MAINNET | Binary corruption |
| G5 | **All distribution MUST be verified with `md5sum`** — source and every destination must match | Silent binary mismatch |
| G6 | **Commits and pushes ARE allowed** — required before compilation on omegacortex | Normal development flow |

**If in doubt**: STOP. Ask Ivan. The cost of a bad testnet deploy is hours of debugging. The cost of touching mainnet is catastrophic.

---

## 1. ARCHITECTURE OVERVIEW

```
 Mac (development)        omegacortex (compilation)       N3 / N5 (remote testnet)
 +-----------------+      +------------------------+      +---------------------+
 | Code editor     |      | git pull               |      | Receive binary      |
 | cargo check     | ---> | cargo build --release  | ---> | md5sum verify       |
 | git commit/push |  SSH | md5sum source binary   |  SCP | systemctl restart   |
 +-----------------+      | distribute locally     |      | RPC health check    |
                          | SCP to N3, N5          |      +---------------------+
                          +------------------------+
```

### What gets built and distributed

| Binary | Crate | Purpose |
|--------|-------|---------|
| `doli-node` | `bins/node` | Testnet node (block production, sync, P2P) |
| `doli` | `bins/cli` | Testnet CLI (balance, send, producer commands) |

Both binaries are ALWAYS distributed together to maintain version consistency.

---

## 2. TESTNET BINARY INVENTORY

### Target paths (testnet-only, segregated from mainnet)

| Host | Binary Path | Services Using It |
|------|-------------|-------------------|
| omegacortex (72.60.228.233) | `/opt/doli/testnet/doli-node` | `doli-testnet-nt{1,2,3,4,5}`, `doli-testnet-archiver` |
| omegacortex | `/opt/doli/testnet/doli` | CLI for testnet operations |
| N3 (147.93.84.44:50790) | `/opt/doli/testnet/doli-node` | `doli-testnet-nt{6,7,8}` |
| N3 | `/opt/doli/testnet/doli` | CLI for testnet operations |
| N5 (72.60.70.166:50790) | `/opt/doli/testnet/doli-node` | `doli-testnet-nt{9,10,11,12}` |
| N5 | `/opt/doli/testnet/doli` | CLI for testnet operations |

> **N4 has no testnet nodes.** Never distribute testnet binaries to N4.

### Mainnet paths (NEVER TOUCH)

| Host | Mainnet Path | DO NOT MODIFY |
|------|-------------|---------------|
| omegacortex | `~/repos/doli/target/release/doli-node` | N1, N2, N6, Archiver |
| N3 | `~/doli-node` | N3 mainnet |
| N4 | `/opt/doli/target/release/doli-node` | N4, N8-N12 mainnet |
| N5 | `/opt/doli/target/release/doli-node` | N5, N7 mainnet |

---

## 3. DEVELOPMENT PHASE (Mac Local)

### 3.1 Code, build, test locally

All commands run inside Nix shell:

```bash
# Enter nix shell (if not already)
nix --extra-experimental-features "nix-command flakes" develop

# Iterate: edit code, then verify
cargo build 2>/tmp/build.log && grep -iE "error|warn" /tmp/build.log | head -20
cargo clippy -- -D warnings 2>/tmp/clippy.log && grep -iE "error|warn" /tmp/clippy.log | head -20
cargo fmt --check
cargo test 2>/tmp/test.log && grep -iE "error|warn|fail|pass|ok" /tmp/test.log | head -30
```

### 3.2 Commit and push

Only after all four checks pass:

```bash
git add -A
git commit --author="E. Weil <weil@doli.network>" -m "<type>(<scope>): <description>"
git push origin main
```

**Allowed commit types**: `fix`, `feat`, `refactor`, `test`, `docs`, `chore`

> Commits and pushes are explicitly allowed. This is NOT a release — it's source code synchronization for compilation.

---

## 4. COMPILATION PHASE (omegacortex)

### 4.1 SSH to omegacortex and pull latest code

```bash
ssh ilozada@72.60.228.233
cd ~/repos/doli
git pull origin main
```

### 4.2 Build release binaries

```bash
nix --extra-experimental-features "nix-command flakes" develop --command bash -c \
  "cargo build --release 2>/tmp/build_release.log && echo 'BUILD OK' || echo 'BUILD FAILED'"

# Check for errors
grep -iE "error|warn" /tmp/build_release.log | head -20
```

### 4.3 Record source MD5 checksums

```bash
md5sum ~/repos/doli/target/release/doli-node ~/repos/doli/target/release/doli | tee /tmp/testnet_source_md5.txt
```

Save this output — it is the **reference checksum** for all distribution verification.

Example output:
```
a1b2c3d4e5f6...  /home/ilozada/repos/doli/target/release/doli-node
f6e5d4c3b2a1...  /home/ilozada/repos/doli/target/release/doli
```

---

## 5. STOP PHASE (MUST come before distribution)

> **CRITICAL**: You CANNOT copy a binary that a running process has open — Linux returns `ETXTBSY`
> ("Text file busy"). Always stop services BEFORE copying binaries to their paths.

### 5.1 Pre-flight: Ensure testnet directory exists on all hosts

Run once (idempotent):

```bash
# omegacortex (local)
sudo mkdir -p /opt/doli/testnet

# N3
ssh -p 50790 ilozada@147.93.84.44 'sudo mkdir -p /opt/doli/testnet'

# N5
ssh -p 50790 ilozada@72.60.70.166 'sudo mkdir -p /opt/doli/testnet'
```

### 5.2 Backup current binaries on ALL hosts (while still running is OK for .bak copy)

```bash
# omegacortex
sudo cp /opt/doli/testnet/doli-node /opt/doli/testnet/doli-node.bak
sudo cp /opt/doli/testnet/doli      /opt/doli/testnet/doli.bak

# N3
ssh -p 50790 ilozada@147.93.84.44 'sudo cp /opt/doli/testnet/doli-node /opt/doli/testnet/doli-node.bak && sudo cp /opt/doli/testnet/doli /opt/doli/testnet/doli.bak'

# N5
ssh -p 50790 ilozada@72.60.70.166 'sudo cp /opt/doli/testnet/doli-node /opt/doli/testnet/doli-node.bak && sudo cp /opt/doli/testnet/doli /opt/doli/testnet/doli.bak'
```

### 5.3 Stop all testnet services (remote-first, bootstrap-last)

```bash
# Step 1: N5 (NT9-NT12)
ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl stop doli-testnet-nt9 doli-testnet-nt10 doli-testnet-nt11 doli-testnet-nt12 && echo "N5 stopped"'

# Step 2: N3 (NT6-NT8)
ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl stop doli-testnet-nt6 doli-testnet-nt7 doli-testnet-nt8 && echo "N3 stopped"'

# Step 3: omegacortex (NT1-NT5 + Archiver)
sudo systemctl stop doli-testnet-nt1 doli-testnet-nt2 doli-testnet-nt3 doli-testnet-nt4 doli-testnet-nt5 doli-testnet-archiver && echo "omegacortex stopped"
```

> **Do NOT forget `doli-testnet-archiver`** — it uses the same binary and will also block with "Text file busy".

---

## 6. DISTRIBUTION PHASE (services must be stopped)

### 6.1 Distribute to N3 and N5 via SCP (from omegacortex)

```bash
# N5
scp -P 50790 ~/repos/doli/target/release/doli-node ilozada@72.60.70.166:/tmp/doli-node-testnet
scp -P 50790 ~/repos/doli/target/release/doli      ilozada@72.60.70.166:/tmp/doli-testnet

ssh -p 50790 ilozada@72.60.70.166 '
  sudo cp /tmp/doli-node-testnet /opt/doli/testnet/doli-node
  sudo cp /tmp/doli-testnet      /opt/doli/testnet/doli
  sudo chmod +x /opt/doli/testnet/doli-node /opt/doli/testnet/doli
  rm /tmp/doli-node-testnet /tmp/doli-testnet
  echo "N5 distribute OK"
'

# N3
scp -P 50790 ~/repos/doli/target/release/doli-node ilozada@147.93.84.44:/tmp/doli-node-testnet
scp -P 50790 ~/repos/doli/target/release/doli      ilozada@147.93.84.44:/tmp/doli-testnet

ssh -p 50790 ilozada@147.93.84.44 '
  sudo cp /tmp/doli-node-testnet /opt/doli/testnet/doli-node
  sudo cp /tmp/doli-testnet      /opt/doli/testnet/doli
  sudo chmod +x /opt/doli/testnet/doli-node /opt/doli/testnet/doli
  rm /tmp/doli-node-testnet /tmp/doli-testnet
  echo "N3 distribute OK"
'
```

### 6.2 Distribute to omegacortex (local copy — services already stopped)

```bash
sudo cp ~/repos/doli/target/release/doli-node /opt/doli/testnet/doli-node
sudo cp ~/repos/doli/target/release/doli       /opt/doli/testnet/doli
sudo chmod +x /opt/doli/testnet/doli-node /opt/doli/testnet/doli
echo "omegacortex distribute OK"
```

---

## 7. VERIFICATION PHASE (MD5 — MANDATORY)

This step is **non-negotiable**. Every deployment must pass MD5 verification before starting any service.

### 7.1 Collect and compare MD5 from all hosts

```bash
echo "=== SOURCE (omegacortex build) ==="
cat /tmp/testnet_source_md5.txt

echo ""
echo "=== DESTINATION: omegacortex ==="
md5sum /opt/doli/testnet/doli-node /opt/doli/testnet/doli

echo ""
echo "=== DESTINATION: N3 ==="
ssh -p 50790 ilozada@147.93.84.44 'md5sum /opt/doli/testnet/doli-node /opt/doli/testnet/doli'

echo ""
echo "=== DESTINATION: N5 ==="
ssh -p 50790 ilozada@72.60.70.166 'md5sum /opt/doli/testnet/doli-node /opt/doli/testnet/doli'
```

### 7.2 Validation criteria

| Check | Pass | Fail Action |
|-------|------|-------------|
| All `doli-node` MD5 match source | Proceed to start | Re-copy from source, re-verify |
| All `doli` MD5 match source | Proceed to start | Re-copy from source, re-verify |
| Any mismatch | **STOP** | Investigate: network corruption, wrong file, partial transfer |

**All 6 checksums (2 binaries x 3 hosts) must match the source before proceeding.**

---

## 8. START PHASE (remote-first, bootstrap-last)

### 8.1 Start services

```bash
# Step 1: N5 (NT9-NT12)
ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl start doli-testnet-nt9 doli-testnet-nt10 doli-testnet-nt11 doli-testnet-nt12 && echo "N5 started"'

# Step 2: N3 (NT6-NT8)
ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl start doli-testnet-nt6 doli-testnet-nt7 doli-testnet-nt8 && echo "N3 started"'

# Step 3: omegacortex (NT1-NT5 + Archiver — bootstrap nodes LAST)
sudo systemctl start doli-testnet-nt1 doli-testnet-nt2 doli-testnet-nt3 doli-testnet-nt4 doli-testnet-nt5 doli-testnet-archiver && echo "omegacortex started"
```

> Start remote nodes first so they're ready to reconnect when bootstrap nodes come up.

---

## 9. POST-DEPLOYMENT VALIDATION

### 9.1 Wait for nodes to initialize

**Wait 35 seconds minimum.** Nodes need time to: load RocksDB, bind RPC port, establish P2P connections, and sync to chain tip. First-time health checks at 20s will show UNREACHABLE — this is normal.

```bash
sleep 35
```

### 9.2 Health check: all testnet nodes (including Archiver-T)

> **Uses `grep` instead of `python3`** — avoids quote-escaping issues over nested SSH.

```bash
echo "=== NT1-NT5 + Archiver (omegacortex) ==="
for port in 18501 18502 18503 18504 18505 18500; do
  echo -n "PORT $port: "
  R=$(curl -s --connect-timeout 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' 2>/dev/null)
  if [ -z "$R" ]; then echo "UNREACHABLE"
  else echo "$R" | grep -oP '"bestHeight":\d+|"version":"[^"]*"' | tr '\n' ' '; echo; fi
done

echo ""
echo "=== NT6-NT8 (N3) ==="
ssh -p 50790 -o ConnectTimeout=5 ilozada@147.93.84.44 '
for port in 18506 18507 18508; do
  echo -n "PORT $port: "
  R=$(curl -s --connect-timeout 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" 2>/dev/null)
  if [ -z "$R" ]; then echo "UNREACHABLE"
  else echo "$R" | grep -oP "\"bestHeight\":\d+|\"version\":\"[^\"]*\"" | tr "\n" " "; echo; fi
done'

echo ""
echo "=== NT9-NT12 (N5) ==="
ssh -p 50790 -o ConnectTimeout=5 ilozada@72.60.70.166 '
for port in 18509 18510 18511 18512; do
  echo -n "PORT $port: "
  R=$(curl -s --connect-timeout 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" 2>/dev/null)
  if [ -z "$R" ]; then echo "UNREACHABLE"
  else echo "$R" | grep -oP "\"bestHeight\":\d+|\"version\":\"[^\"]*\"" | tr "\n" " "; echo; fi
done'
```

### 9.3 Success criteria

| Check | Expected |
|-------|----------|
| All 13 nodes respond to RPC (12 producers + 1 archiver) | `"bestHeight":N "version":"X.Y.Z"` |
| Version matches expected build | Same `version` across all 13 nodes |
| Heights within 2 slots of each other | Chain is progressing, no fork |
| Blocks being produced | Height increases over 20 seconds |

### 9.4 Verify chain progression

Wait 20 seconds, re-query, confirm height advanced:

```bash
sleep 20
echo -n "NT1: "; curl -s --connect-timeout 3 -X POST http://127.0.0.1:18501 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | grep -oP '"bestHeight":\d+'
```

---

## 10. ROLLBACK PROCEDURE

If the new binary causes crashes, forks, or consensus failures. Backup was already created in Section 5.2.

### 10.1 Stop all services, restore .bak, restart

```bash
# Stop all (remote-first)
ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl stop doli-testnet-nt9 doli-testnet-nt10 doli-testnet-nt11 doli-testnet-nt12'
ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl stop doli-testnet-nt6 doli-testnet-nt7 doli-testnet-nt8'
sudo systemctl stop doli-testnet-nt1 doli-testnet-nt2 doli-testnet-nt3 doli-testnet-nt4 doli-testnet-nt5 doli-testnet-archiver

# Restore .bak on all hosts
sudo cp /opt/doli/testnet/doli-node.bak /opt/doli/testnet/doli-node
sudo cp /opt/doli/testnet/doli.bak      /opt/doli/testnet/doli
ssh -p 50790 ilozada@147.93.84.44 'sudo cp /opt/doli/testnet/doli-node.bak /opt/doli/testnet/doli-node && sudo cp /opt/doli/testnet/doli.bak /opt/doli/testnet/doli'
ssh -p 50790 ilozada@72.60.70.166 'sudo cp /opt/doli/testnet/doli-node.bak /opt/doli/testnet/doli-node && sudo cp /opt/doli/testnet/doli.bak /opt/doli/testnet/doli'

# Start all (remote-first)
ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl start doli-testnet-nt9 doli-testnet-nt10 doli-testnet-nt11 doli-testnet-nt12'
ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl start doli-testnet-nt6 doli-testnet-nt7 doli-testnet-nt8'
sudo systemctl start doli-testnet-nt1 doli-testnet-nt2 doli-testnet-nt3 doli-testnet-nt4 doli-testnet-nt5 doli-testnet-archiver
```

---

## 11. COMPLETE DEPLOYMENT CHECKLIST

Copy-paste checklist for every testnet deployment:

```
TESTNET DEPLOYMENT — <DATE> — <DESCRIPTION>

PRE-FLIGHT
[ ] Code changes committed and pushed from Mac
[ ] No `git tag` or version bump created
[ ] No GitHub Release created or drafted

COMPILE (omegacortex)
[ ] SSH to omegacortex
[ ] git pull origin main — latest code
[ ] cargo build --release — successful
[ ] MD5 recorded: doli-node = _______________
[ ] MD5 recorded: doli     = _______________

BACKUP (while services still running — .bak copy is safe)
[ ] omegacortex: .bak created
[ ] N3: .bak created
[ ] N5: .bak created

STOP (remote-first — MUST stop before copying binaries)
[ ] N5: NT9-NT12 stopped
[ ] N3: NT6-NT8 stopped
[ ] omegacortex: NT1-NT5 + Archiver stopped

DISTRIBUTE (services stopped — no "Text file busy")
[ ] N5: SCP + moved to /opt/doli/testnet/
[ ] N3: SCP + moved to /opt/doli/testnet/
[ ] omegacortex: copied to /opt/doli/testnet/

VERIFY MD5 (ALL MUST MATCH SOURCE)
[ ] omegacortex doli-node: _____ MATCH
[ ] omegacortex doli:      _____ MATCH
[ ] N3 doli-node:          _____ MATCH
[ ] N3 doli:               _____ MATCH
[ ] N5 doli-node:          _____ MATCH
[ ] N5 doli:               _____ MATCH

START (remote-first, bootstrap-last)
[ ] N5: NT9-NT12 started
[ ] N3: NT6-NT8 started
[ ] omegacortex: NT1-NT5 + Archiver started

POST-DEPLOY VALIDATION (wait 35s before checking)
[ ] All 13 nodes respond to RPC (12 producers + archiver)
[ ] Version matches across all nodes
[ ] Heights within 2 slots
[ ] Chain progressing (height advances after 20s)

SIGN-OFF
[ ] Deployment verified by: _______________
[ ] Mainnet nodes UNTOUCHED: confirmed
```

---

## 12. QUICK REFERENCE: FORBIDDEN COMMANDS

These commands must NEVER be executed during testnet development:

```bash
# FORBIDDEN — GitHub releases
gh release create ...
gh release edit ...
gh release upload ...

# FORBIDDEN — Version tagging
git tag ...
cargo set-version ...

# FORBIDDEN — Mainnet service interaction
sudo systemctl restart doli-mainnet-*
sudo systemctl stop doli-mainnet-*

# FORBIDDEN — Mainnet binary paths
cp anything /opt/doli/target/release/doli-node    # N3/N4/N5 mainnet
cp anything ~/repos/doli/target/release/doli-node  # omegacortex mainnet

# FORBIDDEN — Mainnet data
rm -rf ~/.doli/mainnet/...
```

---

## 13. SYSTEMD SERVICE REFERENCE

### Testnet services per host

| Host | Services | Key Paths |
|------|----------|-----------|
| omegacortex | `doli-testnet-nt{1,2,3,4,5}`, `doli-testnet-archiver` | `~/doli-test/keys/nt{1-5}.json` |
| N3 | `doli-testnet-nt{6,7,8}` | `~/doli-test/keys/nt{6,7,8}.json` |
| N5 | `doli-testnet-nt{9,10,11,12}` | `~/doli-test/keys/nt{9,10,11,12}.json` |

### Check service status

```bash
# omegacortex
systemctl status doli-testnet-nt{1,2,3,4,5} --no-pager

# N3
ssh -p 50790 ilozada@147.93.84.44 'systemctl status doli-testnet-nt6 doli-testnet-nt7 doli-testnet-nt8 --no-pager'

# N5
ssh -p 50790 ilozada@72.60.70.166 'systemctl status doli-testnet-nt9 doli-testnet-nt10 doli-testnet-nt11 doli-testnet-nt12 --no-pager'
```

### Check logs (last 50 lines)

```bash
# omegacortex NT1
journalctl -u doli-testnet-nt1 -n 50 --no-pager

# N3 NT6
ssh -p 50790 ilozada@147.93.84.44 'journalctl -u doli-testnet-nt6 -n 50 --no-pager'

# N5 NT9
ssh -p 50790 ilozada@72.60.70.166 'journalctl -u doli-testnet-nt9 -n 50 --no-pager'
```

---

## 14. VERIFIED STATE (2026-03-07)

All 12 testnet systemd services confirmed pointing to `/opt/doli/testnet/doli-node`. Binary segregation is complete — no overlap with mainnet paths.

| Check | Status |
|-------|--------|
| omegacortex NT1-NT5 ExecStart | `/opt/doli/testnet/doli-node` |
| N3 NT6-NT8 ExecStart | `/opt/doli/testnet/doli-node` |
| N5 NT9-NT12 ExecStart | `/opt/doli/testnet/doli-node` |
| MD5 consistency (3 hosts) | `doli-node: 52b126fc28839187fa9b303352ad5293` / `doli: fb9e56d46f10b4348b781456e3d3f021` |
| Mainnet binaries untouched | Confirmed |

### Service flags

| Host | Nodes | Extra Flags |
|------|-------|-------------|
| omegacortex | NT1-NT5 | All have `--relay-server --rpc-bind 0.0.0.0 --yes --force-start` |
| N3 | NT6-NT8 | `--yes --force-start`, NT7-NT8 bootstrap N3:40300 + omegacortex:40300 |
| N5 | NT9-NT12 | `--yes --force-start`, bootstrap omegacortex:40300 |

---

## 15. MAINNET AUTO-UPDATE ACTIVATION (Post-Release)

> **When to use**: After Ivan has explicitly requested a version bump + tag + GitHub Release,
> and the release is published and verified on GitHub. This section activates the auto-update
> pipeline so all mainnet nodes worldwide pick up and apply the new binary automatically.

### 15.0 Prerequisites (ALL must be true)

| Check | How to verify |
|-------|---------------|
| GitHub Release exists | `gh release view v<VERSION>` shows the release |
| CI workflow completed | GitHub Actions "chore: bump version" workflow shows green |
| Release has tarball assets | `gh release view v<VERSION> --json assets --jq '.assets[].name'` lists `.tar.gz` for linux-x64 |
| Release has CHECKSUMS.txt | Same command shows `CHECKSUMS.txt` (CI-generated) |
| Release has skeleton SIGNATURES.json | Same command shows `SIGNATURES.json` (CI creates it with 0 signatures) |
| Testnet validated | All 13 testnet nodes running the same version, chain progressing |
| Ivan approved | Explicit "proceed" from Ivan for mainnet activation |

> **CI produces all assets automatically.** When you push a version bump commit, CI builds
> tarballs for all platforms, generates `CHECKSUMS.txt` with SHA-256 hashes, and creates a
> skeleton `SIGNATURES.json` (version + checksums_sha256 filled in, signatures array empty).
> The only manual step is signing with 3 maintainer keys and re-uploading SIGNATURES.json.
>
> **If CI failed**: Check GitHub Actions. Without tarball assets, nodes will detect the
> new version but `auto_apply_from_github()` will fail. Fix CI or use manual deployment.

### 15.1 How the auto-update system works (reference)

```
1. GitHub Release published with CHECKSUMS.txt (CI builds tarballs)
2. Maintainers sign the release (3 of 5 required) → SIGNATURES.json uploaded
3. Running nodes poll GitHub API every ~10 min (mainnet/testnet early network) for new releases
4. Node sees newer version → downloads SIGNATURES.json → verifies 3/5 sigs
5. Veto period begins (5 min early network, target 7 days)
6. If <40% veto weight → APPROVED → grace period → auto-apply + restart
7. If >=40% veto weight → REJECTED → no update
```

**Key files:**
- `CHECKSUMS.txt` — SHA-256 hashes per platform tarball (CI-generated)
- `SIGNATURES.json` — 3/5 maintainer signatures over `"version:sha256(CHECKSUMS.txt)"`
- `metadata.json` — Network targeting (`{"networks":["mainnet","testnet"]}`)

### 15.2 Step 1: Verify release assets on GitHub

```bash
gh release view v<VERSION> --json assets --jq '.assets[].name'
```

**Expected CI-generated assets:**
```
CHECKSUMS.txt
SIGNATURES.json
doli-v<VERSION>-aarch64-apple-darwin.pkg
doli-v<VERSION>-aarch64-apple-darwin.tar.gz
doli-v<VERSION>-x86_64-unknown-linux-gnu.deb
doli-v<VERSION>-x86_64-unknown-linux-gnu.rpm
doli-v<VERSION>-x86_64-unknown-linux-gnu.tar.gz
```

Confirm:
- `doli-v<VERSION>-x86_64-unknown-linux-gnu.tar.gz` exists (all mainnet nodes are linux-x64)
- `CHECKSUMS.txt` exists with SHA-256 hashes
- `SIGNATURES.json` exists (skeleton with 0 signatures — we'll fill it next)

**Verify the skeleton SIGNATURES.json has the checksums_sha256 we need:**

```bash
gh release download v<VERSION> --pattern "SIGNATURES.json" --dir /tmp/v<VERSION> --clobber
cat /tmp/v<VERSION>/SIGNATURES.json
```

Note the `checksums_sha256` value — all 3 maintainer signatures must match this.

### 15.3 Step 2: Sign the release (3 of 5 maintainer keys)

Each maintainer runs `doli release sign` with their producer key. This downloads
`CHECKSUMS.txt` from the GitHub Release, computes its SHA-256, and signs `"version:sha256"`.

**Maintainer keys (N1-N5 = first 5 registered producers = maintainers):**

| Maintainer | Key Location | Server |
|------------|-------------|--------|
| N1 | `~/.doli/mainnet/keys/producer_1.json` | omegacortex |
| N2 | `~/.doli/mainnet/keys/producer_2.json` | omegacortex |
| N3 | `~/.doli/mainnet/keys/producer_3.json` | omegacortex |
| N4 | `~/.doli/mainnet/keys/producer_5.json` | N4 (72.60.115.209) |
| N5 | `~/.doli/mainnet/keys/producer_4.json` | N5 (72.60.70.166) |

> **Reminder**: N4/N5 keys are SWAPPED (N4=producer_5, N5=producer_4). This is intentional.

**Sign with N1, N2, N3 (all on omegacortex — easiest 3/5):**

> **Note**: The CLI binary version used for signing doesn't matter — it just downloads
> CHECKSUMS.txt from GitHub and signs with the key. The mainnet CLI at
> `~/repos/doli/target/release/doli` works even if it's an older version.

```bash
# All 3 commands run from omegacortex (via SSH or directly)
ssh ilozada@72.60.228.233

# Sign with N1
~/repos/doli/target/release/doli release sign --version v<VERSION> \
  --key ~/.doli/mainnet/keys/producer_1.json 2>/dev/null

# Sign with N2
~/repos/doli/target/release/doli release sign --version v<VERSION> \
  --key ~/.doli/mainnet/keys/producer_2.json 2>/dev/null

# Sign with N3
~/repos/doli/target/release/doli release sign --version v<VERSION> \
  --key ~/.doli/mainnet/keys/producer_3.json 2>/dev/null
```

Each command outputs a JSON block AND a message line with the checksums_sha256:
```json
{
  "public_key": "202047256a8072a8...",
  "signature": "a1b2c3d4e5f6..."
}
```
```
Message: "1.1.31:3854fbfb8f72774e..."
               ^--- checksums_sha256 (must match across all 3 signs)
```
```

### 15.4 Step 3: Assemble and upload SIGNATURES.json

Collect the 3 signature blocks and assemble into `SIGNATURES.json`:

```bash
cat > /tmp/SIGNATURES.json << 'SIGEOF'
{
  "version": "<VERSION>",
  "checksums_sha256": "<SHA256_OF_CHECKSUMS_TXT>",
  "signatures": [
    {
      "public_key": "<N1_PUBKEY>",
      "signature": "<N1_SIG>"
    },
    {
      "public_key": "<N2_PUBKEY>",
      "signature": "<N2_SIG>"
    },
    {
      "public_key": "<N3_PUBKEY>",
      "signature": "<N3_SIG>"
    }
  ]
}
SIGEOF
```

> The `checksums_sha256` value is printed by `doli release sign` in the message line:
> `Message: "1.1.31:abc123..."` — the part after the colon is the checksums SHA-256.

Upload to the GitHub Release:

```bash
gh release upload v<VERSION> /tmp/SIGNATURES.json --clobber
```

### 15.5 Step 4: Upload network targeting metadata (optional)

By default, a release targets ALL networks. To restrict (e.g., mainnet-only or staged rollout):

```bash
# Target both mainnet and testnet (default behavior)
echo '{"version":"<VERSION>","networks":["mainnet","testnet"]}' > /tmp/metadata.json

# Target mainnet only
echo '{"version":"<VERSION>","networks":["mainnet"]}' > /tmp/metadata.json

# Target testnet only (staged rollout — test before mainnet)
echo '{"version":"<VERSION>","networks":["testnet"]}' > /tmp/metadata.json

gh release upload v<VERSION> /tmp/metadata.json --clobber
```

If `metadata.json` is not uploaded, the release targets all networks (backward compat).

### 15.6 Step 5: Verify SIGNATURES.json was uploaded correctly

Download the uploaded SIGNATURES.json and confirm it has 3 signatures:

```bash
gh release download v<VERSION> --pattern "SIGNATURES.json" --dir /tmp/verify --clobber
python3 -c "
import json
d = json.load(open('/tmp/verify/SIGNATURES.json'))
print(f'Version: {d[\"version\"]}')
print(f'Signatures: {len(d[\"signatures\"])}/5')
for s in d['signatures']:
    print(f'  - {s[\"public_key\"][:16]}...')
"
```

Expected: `Signatures: 3/5` with the N1, N2, N3 public key prefixes.

> **Note:** `doli update check` queries the **running node's** version via RPC (getNodeInfo),
> not the CLI binary's version. Point it at the correct RPC endpoint to verify a specific node.

### 15.7 Step 6: Monitor auto-update propagation

After SIGNATURES.json is uploaded, the process is **fully automatic** — no manual intervention needed.

**How nodes detect the update:**
- Each node's UpdateService polls GitHub API every **~10 minutes** (`update_check_interval_secs` in NetworkParams — early network setting)
- The first poll fires immediately on node startup (via `tokio::time::interval`)
- Already-running nodes will detect the update at their **next ~10 min poll cycle**
- No restart is needed — just wait (up to ~10 min worst case)
- Note: the default `CHECK_INTERVAL` constant is 6h, but mainnet/testnet override it to 10 min during early network phase

**Timeline per node (from detection, NOT from upload):**

```
T+0:00   Node polls GitHub → detects v<VERSION> → verifies 3/5 sigs
T+0:00   Veto period begins (5 min early network)
T+5:00   Veto period ends → if <40% veto → APPROVED
T+5:00   auto_apply_from_github() → download tarball, verify SHA-256, install, exec() restart
```

> **Key insight**: Different nodes will detect at different times (spread over ~10 min).
> Each node runs its own independent veto+apply cycle. This is a natural rolling upgrade.

**Monitor detection via `getUpdateStatus` RPC (shows pending update before apply):**

```bash
echo "=== Update detection status ==="
ssh ilozada@72.60.228.233 '
for port in 8501 8502 8506; do
  echo -n "PORT $port: "
  curl -s --connect-timeout 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getUpdateStatus\",\"params\":{},\"id\":1}" 2>/dev/null \
    | grep -oP "\"pending_update\":[^,}]+|\"veto_period_active\":[^,}]+"
  echo
done'
```

When a node detects the update: `"pending_update":"1.1.31"` and `"veto_period_active":true`.
After auto-apply: node restarts with new version, `pending_update` returns to `null`.

**Monitor version propagation across all mainnet nodes (post-apply):**

```bash
echo "=== N1-N2, N6 (omegacortex) ==="
ssh ilozada@72.60.228.233 '
for port in 8501 8502 8506; do
  echo -n "PORT $port: "
  R=$(curl -s --connect-timeout 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" 2>/dev/null)
  echo "$R" | grep -oP "\"bestHeight\":\d+|\"version\":\"[^\"]*\"" | tr "\n" " "; echo
done'

echo ""
echo "=== N3 ==="
ssh -p 50790 ilozada@147.93.84.44 '
  echo -n "PORT 8500: "
  R=$(curl -s --connect-timeout 3 -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" 2>/dev/null)
  echo "$R" | grep -oP "\"bestHeight\":\d+|\"version\":\"[^\"]*\"" | tr "\n" " "; echo'

echo ""
echo "=== N4 + N8-N12 ==="
ssh -p 50790 ilozada@72.60.115.209 '
for port in 8504 8508 8509 8510 8511 8512; do
  echo -n "PORT $port: "
  R=$(curl -s --connect-timeout 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" 2>/dev/null)
  echo "$R" | grep -oP "\"bestHeight\":\d+|\"version\":\"[^\"]*\"" | tr "\n" " "; echo
done'

echo ""
echo "=== N5 + N7 ==="
ssh -p 50790 ilozada@72.60.70.166 '
for port in 8505 8507; do
  echo -n "PORT $port: "
  R=$(curl -s --connect-timeout 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" 2>/dev/null)
  echo "$R" | grep -oP "\"bestHeight\":\d+|\"version\":\"[^\"]*\"" | tr "\n" " "; echo
done'
```

**Success criteria**: All nodes report `"version":"<NEW_VERSION>"` and heights within 2 slots.

### 15.8 Step 7: Verify chain health post-update

After all nodes have updated, confirm the chain is healthy:

```bash
# Wait 30 seconds, then check heights again
sleep 30

# Quick check: N1 height is advancing
ssh ilozada@72.60.228.233 'for i in 1 2; do
  echo -n "Check $i: "
  curl -s -X POST http://127.0.0.1:8501 \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" \
    | grep -oP "\"bestHeight\":\d+"
  sleep 15
done'
```

Heights should advance by ~1-2 every 10 seconds.

### 15.9 Troubleshooting auto-update

| Symptom | Cause | Fix |
|---------|-------|-----|
| Node doesn't detect update | Check interval is ~10 min (early network) | Wait, or restart node to trigger immediate check |
| "Insufficient signatures" | SIGNATURES.json missing or <3 valid sigs | Re-sign and re-upload |
| "Download failed" | Tarball asset missing from release | Upload tarball or use manual deploy |
| Node stuck on old version | `notify_only: true` in config | SSH in, run `doli update apply` manually |
| Veto rejected update | >40% stake vetoed | Investigate why, fix issues, publish new release |
| Update applied but node crashed | Bad binary | Watchdog auto-rollbacks after 3 crashes in 1h |

### 15.10 Emergency: Force manual update on specific node

If auto-update fails on a node and manual intervention is needed:

```bash
# SSH to the node's server
# Stop the service
sudo systemctl stop doli-mainnet-<service>

# Backup current binary
sudo cp /path/to/doli-node /path/to/doli-node.bak

# Copy new binary (from omegacortex build or download)
sudo cp /tmp/new-doli-node /path/to/doli-node
sudo chmod +x /path/to/doli-node

# Restart
sudo systemctl start doli-mainnet-<service>
```

> **CRITICAL**: Follow the N1/N2 protection rule — never stop N1 or N2 while any other
> node is syncing or broken. Only touch N1/N2 when ALL nodes are fully synchronized.

### 15.11 Complete auto-update activation checklist

```
MAINNET AUTO-UPDATE — v<VERSION> — <DATE>

PREREQUISITES
[ ] GitHub Release v<VERSION> exists
[ ] CI workflow completed (green)
[ ] CI assets present: .tar.gz, CHECKSUMS.txt, skeleton SIGNATURES.json
[ ] Testnet validated on same version
[ ] Ivan approved mainnet activation

SIGNING (3 of 5 maintainer keys — all on omegacortex)
[ ] N1 signed: public_key=________, signature=________
[ ] N2 signed: public_key=________, signature=________
[ ] N3 signed: public_key=________, signature=________
[ ] checksums_sha256 = ________________ (must match across all 3 signs)

ASSEMBLY & UPLOAD
[ ] SIGNATURES.json assembled with 3 signatures
[ ] SIGNATURES.json uploaded: gh release upload v<VERSION> /tmp/SIGNATURES.json --clobber
[ ] metadata.json uploaded (if network targeting needed)

VERIFICATION (immediate)
[ ] Downloaded SIGNATURES.json from GitHub has 3/5 signatures
[ ] Public key prefixes match N1, N2, N3

PROPAGATION (automatic — wait up to ~10 min, no manual action needed)
[ ] Nodes detect via getUpdateStatus RPC: "pending_update":"<VERSION>"
[ ] After veto (5 min) + auto-apply: nodes report new version via getChainInfo
[ ] Heights within 2 slots across all nodes
[ ] Chain progressing (heights advancing)
[ ] No crashes in logs

SIGN-OFF
[ ] Mainnet auto-update verified by: _______________
[ ] External producers notified (if applicable)
```
