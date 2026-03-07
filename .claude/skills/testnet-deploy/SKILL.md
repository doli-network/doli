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

## 5. DISTRIBUTION PHASE

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

### 5.2 Distribute to omegacortex (local copy)

```bash
sudo cp ~/repos/doli/target/release/doli-node /opt/doli/testnet/doli-node
sudo cp ~/repos/doli/target/release/doli       /opt/doli/testnet/doli
sudo chmod +x /opt/doli/testnet/doli-node /opt/doli/testnet/doli
```

### 5.3 Distribute to N3

```bash
# Copy to temporary location (user-writable), then move to final path
scp -P 50790 ~/repos/doli/target/release/doli-node ilozada@147.93.84.44:/tmp/doli-node-testnet
scp -P 50790 ~/repos/doli/target/release/doli      ilozada@147.93.84.44:/tmp/doli-testnet

ssh -p 50790 ilozada@147.93.84.44 '
  sudo cp /tmp/doli-node-testnet /opt/doli/testnet/doli-node
  sudo cp /tmp/doli-testnet      /opt/doli/testnet/doli
  sudo chmod +x /opt/doli/testnet/doli-node /opt/doli/testnet/doli
  rm /tmp/doli-node-testnet /tmp/doli-testnet
'
```

### 5.4 Distribute to N5

```bash
scp -P 50790 ~/repos/doli/target/release/doli-node ilozada@72.60.70.166:/tmp/doli-node-testnet
scp -P 50790 ~/repos/doli/target/release/doli      ilozada@72.60.70.166:/tmp/doli-testnet

ssh -p 50790 ilozada@72.60.70.166 '
  sudo cp /tmp/doli-node-testnet /opt/doli/testnet/doli-node
  sudo cp /tmp/doli-testnet      /opt/doli/testnet/doli
  sudo chmod +x /opt/doli/testnet/doli-node /opt/doli/testnet/doli
  rm /tmp/doli-node-testnet /tmp/doli-testnet
'
```

---

## 6. VERIFICATION PHASE (MD5 — MANDATORY)

This step is **non-negotiable**. Every deployment must pass MD5 verification before any service restart.

### 6.1 Collect MD5 from all destinations

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

### 6.2 Validation criteria

| Check | Pass | Fail Action |
|-------|------|-------------|
| All `doli-node` MD5 match source | Proceed to restart | Re-copy from source, re-verify |
| All `doli` MD5 match source | Proceed to restart | Re-copy from source, re-verify |
| Any mismatch | **STOP** | Investigate: network corruption, wrong file, partial transfer |

**All 6 checksums (2 binaries x 3 hosts) must match the source before proceeding.**

---

## 7. SERVICE RESTART PHASE

### 7.1 Restart order

Restart in this order to minimize chain disruption:

| Step | Host | Command | Nodes Affected |
|------|------|---------|----------------|
| 1 | N5 | `sudo systemctl restart doli-testnet-nt{9,10,11,12}` | NT9-NT12 |
| 2 | N3 | `sudo systemctl restart doli-testnet-nt{6,7,8}` | NT6-NT8 |
| 3 | omegacortex | `sudo systemctl restart doli-testnet-nt{1,2,3,4,5}` | NT1-NT5 (bootstrap) |

> Restart remote nodes first, bootstrap nodes last. This ensures remote nodes reconnect to running bootstrap nodes.

### 7.2 Execute restarts

```bash
# Step 1: N5 (NT9-NT12)
ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl restart doli-testnet-nt9 doli-testnet-nt10 doli-testnet-nt11 doli-testnet-nt12'

# Step 2: N3 (NT6-NT8)
ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl restart doli-testnet-nt6 doli-testnet-nt7 doli-testnet-nt8'

# Step 3: omegacortex (NT1-NT5)
sudo systemctl restart doli-testnet-nt1 doli-testnet-nt2 doli-testnet-nt3 doli-testnet-nt4 doli-testnet-nt5
```

---

## 8. POST-DEPLOYMENT VALIDATION

### 8.1 Wait for nodes to start

Allow 15-30 seconds for nodes to initialize and re-establish P2P connections.

### 8.2 Health check: all testnet nodes

```bash
echo "=== NT1-NT5 (omegacortex) ==="
for port in 18545 18546 18547 18548 18549; do
  echo -n "PORT $port: "
  curl -s --connect-timeout 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | \
    python3 -c "import sys,json; d=json.load(sys.stdin).get('result',{}); print(f'height={d.get(\"bestHeight\",\"?\")} version={d.get(\"version\",\"?\")}')" 2>/dev/null || echo "UNREACHABLE"
done

echo ""
echo "=== NT6-NT8 (N3) ==="
ssh -p 50790 -o ConnectTimeout=5 ilozada@147.93.84.44 '
for port in 18545 18546 18547; do
  echo -n "PORT $port: "
  curl -s --connect-timeout 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" | \
    python3 -c "import sys,json; d=json.load(sys.stdin).get(\"result\",{}); print(f\"height={d.get(\\\"bestHeight\\\",\\\"?\\\")} version={d.get(\\\"version\\\",\\\"?\\\")}\") " 2>/dev/null || echo "UNREACHABLE"
done'

echo ""
echo "=== NT9-NT12 (N5) ==="
ssh -p 50790 -o ConnectTimeout=5 ilozada@72.60.70.166 '
for port in 18545 18546 18547 18548; do
  echo -n "PORT $port: "
  curl -s --connect-timeout 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" | \
    python3 -c "import sys,json; d=json.load(sys.stdin).get(\"result\",{}); print(f\"height={d.get(\\\"bestHeight\\\",\\\"?\\\")} version={d.get(\\\"version\\\",\\\"?\\\")}\") " 2>/dev/null || echo "UNREACHABLE"
done'
```

### 8.3 Success criteria

| Check | Expected |
|-------|----------|
| All 12 nodes respond to RPC | `height=N version=X.Y.Z` |
| Version matches expected build | Same `version` across all nodes |
| Heights within 2 slots of each other | Chain is progressing, no fork |
| Blocks being produced | Height increases over 20 seconds |

### 8.4 Verify chain progression

Wait 20 seconds, re-query NT1, and confirm height advanced:

```bash
sleep 20
curl -s -X POST http://127.0.0.1:18545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
```

---

## 9. ROLLBACK PROCEDURE

If the new binary causes crashes, forks, or consensus failures:

### 9.1 Keep the previous binary

Before distribution (Section 5), back up the current testnet binary:

```bash
# On omegacortex
sudo cp /opt/doli/testnet/doli-node /opt/doli/testnet/doli-node.bak
sudo cp /opt/doli/testnet/doli      /opt/doli/testnet/doli.bak

# On N3
ssh -p 50790 ilozada@147.93.84.44 'sudo cp /opt/doli/testnet/doli-node /opt/doli/testnet/doli-node.bak && sudo cp /opt/doli/testnet/doli /opt/doli/testnet/doli.bak'

# On N5
ssh -p 50790 ilozada@72.60.70.166 'sudo cp /opt/doli/testnet/doli-node /opt/doli/testnet/doli-node.bak && sudo cp /opt/doli/testnet/doli /opt/doli/testnet/doli.bak'
```

### 9.2 Restore previous binary

```bash
# On all hosts: restore .bak and restart
# omegacortex
sudo cp /opt/doli/testnet/doli-node.bak /opt/doli/testnet/doli-node
sudo cp /opt/doli/testnet/doli.bak      /opt/doli/testnet/doli
sudo systemctl restart doli-testnet-nt{1,2,3,4,5}

# N3
ssh -p 50790 ilozada@147.93.84.44 'sudo cp /opt/doli/testnet/doli-node.bak /opt/doli/testnet/doli-node && sudo cp /opt/doli/testnet/doli.bak /opt/doli/testnet/doli && sudo systemctl restart doli-testnet-nt6 doli-testnet-nt7 doli-testnet-nt8'

# N5
ssh -p 50790 ilozada@72.60.70.166 'sudo cp /opt/doli/testnet/doli-node.bak /opt/doli/testnet/doli-node && sudo cp /opt/doli/testnet/doli.bak /opt/doli/testnet/doli && sudo systemctl restart doli-testnet-nt9 doli-testnet-nt10 doli-testnet-nt11 doli-testnet-nt12'
```

---

## 10. COMPLETE DEPLOYMENT CHECKLIST

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

BACKUP
[ ] omegacortex: .bak created
[ ] N3: .bak created
[ ] N5: .bak created

DISTRIBUTE
[ ] omegacortex: copied to /opt/doli/testnet/
[ ] N3: SCP + moved to /opt/doli/testnet/
[ ] N5: SCP + moved to /opt/doli/testnet/

VERIFY MD5 (ALL MUST MATCH SOURCE)
[ ] omegacortex doli-node: _____ MATCH
[ ] omegacortex doli:      _____ MATCH
[ ] N3 doli-node:          _____ MATCH
[ ] N3 doli:               _____ MATCH
[ ] N5 doli-node:          _____ MATCH
[ ] N5 doli:               _____ MATCH

RESTART (remote-first order)
[ ] N5: NT9-NT12 restarted
[ ] N3: NT6-NT8 restarted
[ ] omegacortex: NT1-NT5 restarted

POST-DEPLOY VALIDATION
[ ] All 12 nodes respond to RPC
[ ] Version matches across all nodes
[ ] Heights within 2 slots
[ ] Chain progressing (height advances after 20s)

SIGN-OFF
[ ] Deployment verified by: _______________
[ ] Mainnet nodes UNTOUCHED: confirmed
```

---

## 11. QUICK REFERENCE: FORBIDDEN COMMANDS

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

## 12. SYSTEMD SERVICE REFERENCE

### Testnet services per host

| Host | Services | Key Paths |
|------|----------|-----------|
| omegacortex | `doli-testnet-nt{1,2,3,4,5}` | `~/doli-test/keys/nt{1-5}.json` |
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

## 13. VERIFIED STATE (2026-03-07)

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
| N3 | NT6-NT8 | `--yes --force-start`, NT7-NT8 bootstrap N3:40303 + omegacortex:40303 |
| N5 | NT9-NT12 | `--yes --force-start`, bootstrap omegacortex:40303 |
