# Genesis Ceremony Runbook

> Lessons from the 2026-03-11 network partition. Service files are consensus-critical.
> Never wipe data without backup. Never hand-edit service files.

## Pre-Genesis Checklist (T-48h)

- [ ] Compile and sign final release binary on ai2 (NEVER compile on ai1/ai3)
- [ ] Verify all producer service files use identical template via `scripts/install-services.sh validate`
- [ ] Record peer ID of each seed node before any wipes (libp2p identity file: `<data-dir>/identity.key`)
- [ ] Collect all producer pubkeys for chainspec `genesis_producers`
- [ ] Test `scripts/chain-reset.sh` on a spare node in isolation — verify it wipes correctly
- [ ] Confirm DNS round-robin for seed1/seed2/seed3.doli.network resolves all three seeds
- [ ] Run `scripts/health-check.sh` on all nodes — zero failures required
- [ ] Verify `LimitNOFILE=65535` in all service files

## Genesis Procedure (T-0)

1. Generate genesis block on ai1: `doli-node genesis --chainspec chainspec.mainnet.json`
2. Distribute `chainspec.mainnet.json` to ALL nodes. Do not start any node yet.
3. Start seed nodes first (ai3 — all 6 seeds). Wait for "Listening on" in logs.
4. Wait 60 seconds for seed nodes to discover each other (verify via `getPeerInfo` RPC).
5. Start producer nodes one at a time, 10 seconds apart. Monitor peer count after each.
6. Confirm all producers show same `genesis_hash` via `getChainInfo` before any produce blocks.
7. Let epoch 1 complete before declaring genesis stable.

## Recovery Order (if partition detected)

1. **STOP** — Do not wipe any node that is synced to the canonical chain.
2. Identify canonical chain: highest weight chain with >= 2/3 of producers.
3. Wipe ONLY nodes on a solo chain (0 peers, wrong genesis hash).
4. Restart wiped nodes with `--bootstrap` pointing to a confirmed canonical seed.
5. Wait for snap sync to complete before restarting the next node.
6. **Never wipe more than one node at a time.**

## Service File Rules

- All deployments MUST be generated from `scripts/install-services.sh` — **no manual edits**.
- The `--bootstrap` flag is **mandatory** on all nodes. A node without it has zero peer discovery after a data wipe. This was the root cause of the 2026-03-11 incident.
- Required settings in every service file:
  - `LimitNOFILE=65535`
  - `--force-start`
  - `--yes`
  - Proper log routing
  - At least 2 `--bootstrap` entries pointing to seed nodes

## Server Layout (v4, 2026-03-13)

| Server | Role |
|--------|------|
| ai1 | Testnet producers |
| ai2 | Mainnet producers (compile here) |
| ai3 | Seeds (mainnet + testnet, 3 per network) |

## Post-Genesis Verification

- [ ] `scripts/status.sh all` — all nodes producing, 20+ peers
- [ ] `scripts/balances.sh all` — expected initial balances
- [ ] `scripts/bonds.sh all` — all producers bonded
- [ ] Archive completeness: `ls <archive-dir>/blocks/*.block | wc -l` equals tip height
- [ ] State roots match across all nodes: `getStateRootDebug` on each
