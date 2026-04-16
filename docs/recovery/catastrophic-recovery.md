# Catastrophic Network Recovery Procedure

## When to use

When multiple nodes are forked, stuck, or diverged and manual per-node fixes are insufficient. Signs: 3+ nodes with different hashes at same height, rollback cascades, multiple `FORK` or `BEHIND` statuses in explorer.

## Prerequisites

- SSH access to all servers (ai1-ai5, ai7-ai11)
- All nodes must be stopped before starting
- Choose ONE canonical node (highest height, largest data dir)

## Step 0: Identify canonical node

```bash
for s in ai1 ai2 ai3 ai4 ai5; do
  ssh $s "for f in /var/log/doli/mainnet/*.log; do
    name=\$(basename \$f .log)
    line=\$(grep STATE_FP \$f | tail -1)
    if [ -n \"\$line\" ]; then
      h=\$(echo \$line | grep -oP 'h=\d+')
      sr=\$(echo \$line | grep -oP 'sr=\S{16}')
      echo \"\$name \$h \$sr\"
    fi
  done" 2>/dev/null
done
```

Pick the node with highest `h=` value. Verify its data dir is intact:
```bash
ssh <server> "du -sh /mainnet/<node>/data/"
```

In this doc we use **N12 on ai5** as the canonical node.

## Step 1: Stop ALL nodes on ALL servers

```bash
ssh ai1 "sudo systemctl stop doli-mainnet-seed doli-mainnet-n1 doli-mainnet-n2 doli-mainnet-n3" &
ssh ai2 "sudo systemctl stop doli-mainnet-seed doli-mainnet-n4 doli-mainnet-n5" &
ssh ai3 "sudo systemctl stop doli-mainnet-seed doli-mainnet-ivan doli-mainnet-santiago" &
ssh ai4 "sudo systemctl stop doli-mainnet-n6 doli-mainnet-n7 doli-mainnet-n8" &
ssh ai5 "sudo systemctl stop doli-mainnet-n9 doli-mainnet-n10 doli-mainnet-n11 doli-mainnet-n12" &
# ai7-ai11: stop all doli-mainnet-* services
for s in ai7 ai8 ai9 ai10 ai11; do
  ssh $s "sudo systemctl stop \$(systemctl list-units --type=service --state=running --no-legend | grep -oE 'doli-mainnet-\S+' | tr '\n' ' ')" &
done
wait
```

Verify: `pgrep -a doli-node` on each server should return nothing.

## Step 2: Wipe all nodes EXCEPT canonical

### Core servers (ai1-ai5) — wipe data/*

```bash
# ai1
ssh ai1 "rm -rf /mainnet/seed/data/* /mainnet/n1/data/* /mainnet/n2/data/* /mainnet/n3/data/*"
# ai2
ssh ai2 "rm -rf /mainnet/seed/data/* /mainnet/n4/data/* /mainnet/n5/data/*"
# ai3
ssh ai3 "rm -rf /mainnet/ivan/data/* /mainnet/santiago/data/*"
# ai4
ssh ai4 "rm -rf /mainnet/n6/data/* /mainnet/n7/data/* /mainnet/n8/data/*"
# ai5 — PRESERVE N12
ssh ai5 "rm -rf /mainnet/n9/data/* /mainnet/n10/data/* /mainnet/n11/data/*"
```

### External servers (ai7-ai11) — PRESERVE WALLETS

**CRITICAL**: ai7 and ai11 have `wallet.json` inside `data/`. Must backup first.

```bash
# ai7 — backup wallets
ssh ai7 "for n in adri abraham ori isudoajl alan alessandro; do
  cp /var/lib/doli/mainnet/\$n/data/wallet.json /tmp/wallet_\$n.json 2>/dev/null && echo \"backed up \$n\"
done"

# ai11 — backup wallets
ssh ai11 "for n in folsi miri pastora; do
  cp /var/lib/doli/mainnet/\$n/data/wallet.json /tmp/wallet_\$n.json 2>/dev/null && echo \"backed up \$n\"
done"

# Wipe (sudo required on ai7/ai11)
ssh ai7 "for n in adri abraham ori isudoajl alan alessandro; do
  sudo rm -rf /var/lib/doli/mainnet/\$n/data/*
done"
ssh ai11 "for n in folsi miri pastora; do
  sudo rm -rf /var/lib/doli/mainnet/\$n/data/*
done"

# ai8, ai9, ai10 — no wallet in data, simple wipe
ssh ai8 "rm -rf /mainnet/leandro/data/*"
ssh ai9 "sudo rm -rf /var/lib/doli/mainnet/blocks /var/lib/doli/mainnet/state_db /var/lib/doli/mainnet/utxo_store /var/lib/doli/mainnet/signed_slots.db"
ssh ai10 "rm -rf /mainnet/daniel/data/*"

# Restore wallets (sudo + chown required)
ssh ai7 "for n in adri abraham ori isudoajl alan alessandro; do
  sudo cp /tmp/wallet_\$n.json /var/lib/doli/mainnet/\$n/data/wallet.json
  sudo chown isudoajl:isudoajl /var/lib/doli/mainnet/\$n/data/wallet.json
done"
ssh ai11 "for n in folsi miri pastora; do
  sudo cp /tmp/wallet_\$n.json /var/lib/doli/mainnet/\$n/data/wallet.json
  sudo chown doli:doli /var/lib/doli/mainnet/\$n/data/wallet.json
done"
```

## Step 3: Rsync from canonical node

### Local (same server as canonical)

```bash
ssh ai5 "for n in n9 n10 n11; do
  rsync -a --exclude='producer.lock' --exclude='signed_slots.db' /mainnet/n12/data/ /mainnet/\$n/data/ && echo \"\$n done\"
done"
```

### Cross-server (one node per server)

```bash
ssh ai1 "rsync -a --exclude='producer.lock' --exclude='signed_slots.db' -e ssh ai5:/mainnet/n12/data/ /mainnet/n1/data/" &
ssh ai2 "rsync -a --exclude='producer.lock' --exclude='signed_slots.db' -e ssh ai5:/mainnet/n12/data/ /mainnet/n4/data/" &
ssh ai3 "rsync -a --exclude='producer.lock' --exclude='signed_slots.db' -e ssh ai5:/mainnet/n12/data/ /mainnet/ivan/data/" &
ssh ai4 "rsync -a --exclude='producer.lock' --exclude='signed_slots.db' -e ssh ai5:/mainnet/n12/data/ /mainnet/n6/data/" &
wait
```

### Local spread (within each server)

```bash
ssh ai1 "for n in seed n2 n3; do rsync -a --exclude='producer.lock' --exclude='signed_slots.db' /mainnet/n1/data/ /mainnet/\$n/data/ && echo \"\$n done\"; done"
ssh ai2 "for n in seed n5; do rsync -a --exclude='producer.lock' --exclude='signed_slots.db' /mainnet/n4/data/ /mainnet/\$n/data/ && echo \"\$n done\"; done"
ssh ai3 "rsync -a --exclude='producer.lock' --exclude='signed_slots.db' /mainnet/ivan/data/ /mainnet/seed/data/ && echo 'seed done'
rsync -a --exclude='producer.lock' --exclude='signed_slots.db' /mainnet/ivan/data/ /mainnet/santiago/data/ && echo 'santiago done'"
ssh ai4 "for n in n7 n8; do rsync -a --exclude='producer.lock' --exclude='signed_slots.db' /mainnet/n6/data/ /mainnet/\$n/data/ && echo \"\$n done\"; done"
```

**Note**: ai7-ai11 external nodes do NOT need rsync — they recover via snap sync on startup.

## Step 4: Start seeds FIRST

```bash
ssh ai1 "sudo systemctl start doli-mainnet-seed && echo 'Seed1'"
ssh ai2 "sudo systemctl start doli-mainnet-seed && echo 'Seed2'"
ssh ai3 "sudo systemctl start doli-mainnet-seed && echo 'Seed3'"
```

Wait 10 seconds for seeds to discover each other.

## Step 5: Start producers (core)

```bash
ssh ai1 "sudo systemctl start doli-mainnet-n1 doli-mainnet-n2 doli-mainnet-n3 && echo 'ai1'"
ssh ai2 "sudo systemctl start doli-mainnet-n4 doli-mainnet-n5 && echo 'ai2'"
ssh ai3 "sudo systemctl start doli-mainnet-ivan doli-mainnet-santiago && echo 'ai3'"
ssh ai4 "sudo systemctl start doli-mainnet-n6 doli-mainnet-n7 doli-mainnet-n8 && echo 'ai4'"
ssh ai5 "sudo systemctl start doli-mainnet-n9 doli-mainnet-n10 doli-mainnet-n11 doli-mainnet-n12 && echo 'ai5'"
```

## Step 6: Start external producers

```bash
for s in ai7 ai8 ai9 ai10 ai11; do
  ssh $s "sudo systemctl start \$(systemctl list-units --type=service --all --no-legend | grep 'doli-mainnet' | grep -oE 'doli-mainnet-\S+' | tr '\n' ' ')" && echo "$s started" &
done
wait
```

## Step 7: Verify

```bash
for s in ai1 ai2 ai3 ai4 ai5; do
  ssh $s "for f in /var/log/doli/mainnet/*.log; do
    name=\$(basename \$f .log)
    line=\$(tail -20 \$f | grep -oP 'peers=\d+' | tail -1)
    h=\$(grep STATE_FP \$f | tail -1 | grep -oP 'h=\d+')
    echo \"\$name \$h \$line\"
  done" 2>/dev/null
done
```

All nodes should show same `h=` value (within 1-2 blocks) and `peers>0`.

## Step 8: Monitor for 30 minutes

Watch for FORK_GUARD drops, REJECT, rollback, or stuck_fork events:

```bash
ssh ai1 "tail -f /var/log/doli/mainnet/n1.log" | grep --line-buffered 'FORK_GUARD\|REJECT\|ROLLBACK\|stuck_fork\|SNAP_SYNC'
```

If any node forks within 30 minutes, investigate before deploying any changes.

## Estimated time

| Step | Duration |
|------|----------|
| Stop all | 30s |
| Wipe + wallet backup | 2 min |
| Cross-server rsync | 2-3 min |
| Local rsync | 1 min |
| Start seeds + wait | 15s |
| Start producers | 30s |
| Start externals | 30s |
| Verify | 1 min |
| **Total** | **~8 minutes** |

## Common pitfalls

1. **Forgetting wallet backup on ai7/ai11** — wallets inside `data/`, `rm -rf data/*` destroys them permanently
2. **Starting producers before seeds** — producers can't find peers, produce isolated blocks, create forks
3. **ai9 uses `/var/lib/doli/mainnet/`** not `/mainnet/*/data/` — different path, needs sudo
4. **rsync from ai5 requires SSH config** — ai7-ai11 may not resolve `ai5` hostname, use IP or configure SSH
5. **Not stopping ALL nodes first** — running nodes gossip stale blocks to recovering nodes

## Incident history

- 2026-04-15 04:49 UTC — N1 gap=50 from peer churn → snap sync
- 2026-04-15 14:52 UTC — Folsi rollback cascade (25 rollbacks) from ECON_EPOCH_NOT_BOUNDARY
- 2026-04-16 01:15 UTC — Full network cascade from synmgrefactor deploy (8 nodes restarted simultaneously)
- 2026-04-16 06:30 UTC — Recovery using this procedure (N12 as canonical, ~10 min total)
