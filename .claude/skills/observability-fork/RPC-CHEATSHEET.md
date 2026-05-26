# Observability-Fork RPC Cheatsheet

Concrete curl payloads for every fork-observability RPC method.
All examples target local devnet seed at port 28500. Adjust port for other nodes.

---

## getForkDiagnostic

Full diagnostic bundle for the last hour:
```bash
curl -s -X POST http://127.0.0.1:28500 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getForkDiagnostic","params":{"window_secs":3600},"id":1}' \
  | python3 -m json.tool
```

Last 6 hours, only 50 events:
```bash
curl -s -X POST http://127.0.0.1:28500 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getForkDiagnostic","params":{"window_secs":21600,"limit":50},"id":1}'
```

Causal chain from a specific event (replace ULID):
```bash
curl -s -X POST http://127.0.0.1:28500 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getForkDiagnostic","params":{"fork_event_id":"01HXZ..."},"id":1}'
```

Key response fields:
- `.result.classification.fork_type` — variant name (e.g., `"TipRaceNatural"`, `{"ChainBreakLoop":{...}}`)
- `.result.classification.confidence` — float 0.0-1.0
- `.result.classification.recommended_action` — `"normal_operation"` / `"investigate_producer"` / `"restart_with_resync"` / etc.
- `.result.fork_summary.fork_events_in_window` — count of fork-relevant events
- `.result.health.events_dropped_total` — non-zero means ring buffer overflowed; some events lost

---

## getFleetForkDiagnostic

Devnet (seed + N1-N2):
```bash
curl -s -X POST http://127.0.0.1:28500 \
  -H 'Content-Type: application/json' \
  -d '{
    "jsonrpc":"2.0","method":"getFleetForkDiagnostic",
    "params":{
      "peer_rpcs":["http://127.0.0.1:28500","http://127.0.0.1:28501","http://127.0.0.1:28502"],
      "window_secs":3600
    },"id":1
  }' | python3 -m json.tool
```

Testnet local (seed + n1-n12):
```bash
PEERS=$(python3 -c "print(','.join(f'\"http://127.0.0.1:{p}\"' for p in range(8500,8513)))")
curl -s -X POST http://127.0.0.1:8500 \
  -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"getFleetForkDiagnostic\",\"params\":{\"peer_rpcs\":[$PEERS],\"window_secs\":3600},\"id\":1}" \
  | python3 -m json.tool
```

Key response fields:
- `.result.divergence_table[]` — heights with competing hashes across fleet (empty = no fork)
- `.result.fork_groups[]` — `peers_on_canonical`, `peers_on_fork`, `peers_undecided`
- `.result.fleet_summary.majority_classification` — most common ForkType across peers
- `.result.queried_peers[].error` — `"timeout"` / `"connection-refused"` / `"method-not-found"` (old node)

Env overrides:
```bash
DOLI_FLEET_MAX_PEERS=100 DOLI_FLEET_PEER_TIMEOUT_SECS=10 curl ...
```

---

## getStateRootDebug

```bash
curl -s -X POST http://127.0.0.1:28500 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getStateRootDebug","params":{},"id":1}'
```

Response:
```json
{
  "height": 12345,
  "bestHash": "abc...",
  "stateRoot": "def...",
  "csHash": "111...",
  "utxoHash": "222...",
  "psHash": "333...",
  "utxoCount": 4200,
  "producerCount": 14,
  "totalMinted": 12345000000,
  "registrationSeq": 17
}
```

Compare two nodes at the same height (devnet):
```bash
for PORT in 28500 28501 28502; do
  echo "=== port $PORT ==="
  curl -s -X POST http://127.0.0.1:$PORT \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"getStateRootDebug","params":{},"id":1}' \
    | python3 -c "import sys,json; r=json.load(sys.stdin)['result']; print(f\"h={r['height']} root={r['stateRoot'][:16]}... utxo={r['utxoHash'][:16]}... ps={r['psHash'][:16]}...\")"
done
```

Interpretation:
- `stateRoot` differs → diverged. Check sub-hashes.
- Only `utxoHash` differs → UTXO set divergence (use `getUtxoDiff`).
- Only `psHash` differs → ProducerSet divergence (scheduling may diverge at next epoch).
- Only `csHash` differs → ChainState metadata divergence (height, slot, or totalMinted differ).

---

## getUtxoDiff

Full UTXO hash dump from node A:
```bash
curl -s -X POST http://127.0.0.1:28500 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getUtxoDiff","params":{},"id":1}' \
  | python3 -c "import sys,json; r=json.load(sys.stdin)['result']; hashes=[e['hash'] for e in r['entries']]; print(json.dumps(hashes))" \
  > /tmp/node_a_hashes.json
```

Find differing entries on node B:
```bash
HASHES=$(cat /tmp/node_a_hashes.json)
curl -s -X POST http://127.0.0.1:28501 \
  -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"getUtxoDiff\",\"params\":{\"referenceHashes\":$HASHES},\"id\":1}" \
  | python3 -m json.tool
```

Response with diffs:
```json
{
  "height": 12345,
  "totalEntries": 4200,
  "diffCount": 3,
  "diffs": [
    {"outpoint": "abcd...", "hash": "1234...", "detail": "amt=5000000000 h=12300 type=1 ..."}
  ]
}
```

Note: Only works when node uses in-memory UTXO set. Returns error on RocksDb backend.

---

## getChainInfo (fork-monitor base method)

```bash
curl -s -X POST http://127.0.0.1:28500 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' \
  | python3 -c "import sys,json; r=json.load(sys.stdin)['result']; print(r['bestHeight'], r['bestHash'][:16])"
```

---

## getChainStats

```bash
curl -s -X POST http://127.0.0.1:28500 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getChainStats","params":{},"id":1}'
```

Response fields: `total_supply`, `address_count`, `utxo_count`, `active_producers`, `total_staked`, `height`, `reward_pool_balance`, `total_confirmed`

---

## fork-monitor.sh Quick Reference

```bash
# One-shot devnet
scripts/fork-monitor.sh

# One-shot testnet
scripts/fork-monitor.sh --testnet

# Continuous devnet (30s interval)
scripts/fork-monitor.sh --loop 30

# Continuous testnet (60s)
scripts/fork-monitor.sh --testnet --loop 60

# Custom endpoint file (one host:port per line)
scripts/fork-monitor.sh --endpoints /tmp/nodes.txt
```

Exit codes: 0=all agree, 1=FORK DETECTED, 2=no nodes reachable/parse error

FORK output example:
```
[2026-05-26 14:23:10] FORK DETECTED — 2 chain tips across 4 nodes!

  Group 1: hash=abc123...def456  height=15000
    Nodes: Seed, N1, N2
  Group 2: hash=fff999...aaa111  height=15000
    Nodes: N3

  ACTION: Run 'scripts/emergency-halt.sh' to stop all producers
```

---

## ForkType → Action mapping

| `classification.fork_type` | `recommended_action` | Confidence | Next step |
|---|---|---|---|
| `TipRaceNatural` | `normal_operation` | 0.70 | No action needed |
| `TipRaceHighLatency` | `investigate_latency` | 0.75 | Check network between producers |
| `ProducerEquivocation` | `investigate_producer` | 0.95 | See guardian SKILL.md |
| `EpochBoundaryInvalid` | `investigate_producer` | 0.90 | Check `calculate_epoch_rewards()` |
| `PostSnapDeadTip` | `investigate_snap_sync` | 0.80 | `auto_recover` in divergence_table |
| `RollbackLoop` | `investigate_recovery_params` | 0.85 | See guardian SKILL.md |
| `ChainBreakLoop{...}` | `restart_with_resync` | 0.85 | Stop node, wipe `{blocks,state_db,utxo,diagnostics}`, restart with snap |
| `Unknown{reason_unknown}` | `None` | 0.0 | Human escalation required |
