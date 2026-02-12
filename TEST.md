DOLI Network Stability Test — Iterative Loop Until World-Class

Prime Directive

DOLI will become a many-thousands producer node network communicating
in 10-second slot windows. Every architectural decision, fix, and
optimization must be made with 150K producer nodes in mind. Never
apply a patch that works for 21 nodes but breaks at scale.

Definition of success: A node that ends up on a shorter/lighter fork
ALWAYS automatically reorgs back to the heaviest chain without any
manual intervention. No restart, no data wipe, no operator action.
MANUAL INTERVENTION FOR RECOVERY IS CONSIDERED A FAILURE.

Read the /network-setup skill FIRST before doing anything.


Test Protocol (Execute in exact order)

Phase 0: Clean Slate

doli-node devnet stop

doli-node devnet clean

Kill any zombie doli-node processes:

pkill -f doli-node; sleep 2

Phase 1: Genesis Network (5 producers)

doli-node devnet init --nodes 5

doli-node devnet start

Wait for genesis bootstrap to complete (24 blocks = ~4 minutes at 10s
slots). Verify all 5 nodes are synced, producing, and on the same
chain:

doli-node devnet status

Gate: All 5 nodes must show same height (±2 blocks), same hash, peers
> 0. Do NOT proceed until this passes.

Phase 2: Whale Producer (10-bond)

Create new wallet:

doli -w ~/.doli/devnet/keys/producer_5.json new

Get pubkey hash (USE "Pubkey Hash (32-byte)", NOT "Public Key"):

doli -w ~/.doli/devnet/keys/producer_5.json info

Transfer 11 DOLI from any genesis producer:

doli -r http://127.0.0.1:28545 -w ~/.doli/devnet/keys/producer_0.json
send <PUBKEY_HASH> 11

Wait 1 block (~10s), verify balance:

doli -r http://127.0.0.1:28545 -w ~/.doli/devnet/keys/producer_5.json balance

Register with 10 bonds:

doli -r http://127.0.0.1:28545 -w ~/.doli/devnet/keys/producer_5.json
producer register --bonds 10

Start node (NO --no-dht):

doli-node --network devnet --data-dir ~/.doli/devnet/data/node5 run \

    --producer --producer-key ~/.doli/devnet/keys/producer_5.json \

    --p2p-port 50308 --rpc-port 28550 --metrics-port 9095 \

    --bootstrap '/ip4/127.0.0.1/tcp/50303' \

    --chainspec ~/.doli/devnet/chainspec.json --yes \

    > ~/.doli/devnet/logs/node5.log 2>&1 &

Wait for ACTIVATION_DELAY (10 blocks, ~100s). Verify producing:

grep "Produced block\|Producing block" ~/.doli/devnet/logs/node5.log | tail -5

Gate: Node 5 (whale) must be synced, producing, same chain as genesis
nodes. 6/6 nodes healthy.

Phase 3: Small Producers (15 × 2-bond)

Create 15 wallets (producer_6 through producer_20):

for i in {6..20}; do

  doli -w ~/.doli/devnet/keys/producer_$i.json new -n "producer_$i"

done

Fund each with 3 DOLI. Use different source wallets to avoid UTXO double-spend:

for i in {6..20}; do

  src=$((i % 5))  # rotate through producers 0-4

  pubkey=$(doli -w ~/.doli/devnet/keys/producer_$i.json info
2>/dev/null | grep "Pubkey Hash (32-byte)" | sed 's/.*: //')

  doli -r http://127.0.0.1:$((28545 + src)) -w
~/.doli/devnet/keys/producer_$src.json send "$pubkey" 3

  sleep 12  # wait 1 block between sends from same source

done

Register all with 2 bonds:

for i in {6..20}; do

  doli -r http://127.0.0.1:28545 -w
~/.doli/devnet/keys/producer_$i.json producer register -b 2

done

Start all 15 nodes (NO --no-dht):

for i in {6..20}; do

  P2P=$((50303 + i))

  RPC=$((28545 + i))

  METRICS=$((9090 + i))

  doli-node --network devnet --data-dir ~/.doli/devnet/data/node$i run \

    --producer --producer-key ~/.doli/devnet/keys/producer_$i.json \

    --p2p-port $P2P --rpc-port $RPC --metrics-port $METRICS \

    --bootstrap '/ip4/127.0.0.1/tcp/50303' \

    --chainspec ~/.doli/devnet/chainspec.json --yes \

    > ~/.doli/devnet/logs/node$i.log 2>&1 &

  sleep 2

done

Wait for ACTIVATION_DELAY + sync (~3 minutes).

Gate: All 21 nodes synced, producing, same chain. Do NOT proceed until
this passes.

Phase 4: Stability Observation (200 blocks = ~33 minutes)

Monitor every 60 seconds for 33 minutes:

for i in $(seq 0 20); do

  if [ $i -le 4 ]; then RPC=$((28545 + i)); else RPC=$((28545 + i)); fi

  chain=$(curl -s http://127.0.0.1:$RPC -X POST -H 'Content-Type:
application/json' \

    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
2>/dev/null)

  net=$(curl -s http://127.0.0.1:$RPC -X POST -H 'Content-Type:
application/json' \

    -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}'
2>/dev/null)

  h=$(echo $chain | jq -r '.result.bestHeight // "DOWN"')

  hash=$(echo $chain | jq -r '.result.bestHash // "?"' | head -c 16)

  peers=$(echo $net | jq -r '.result.peer_count // "?"')

  echo "Node $i: height=$h hash=$hash peers=$peers"

done

Success criteria after 200 blocks:

[ ] All 21 nodes on same chain (same hash at same height)
[ ] No node stuck (all heights within 2 blocks of each other)
[ ] No node with 0 peers
[ ] Whale (node 5) earned ~10x rewards vs genesis nodes (proportional
to bond weight)
[ ] Small producers (6-20) earned ~2x rewards vs genesis nodes
[ ] Zero manual interventions required
[ ] No force_resync_from_genesis triggered (check logs)
[ ] No persistent forks (transient 1-2 block forks are normal,
resolved automatically)

Check for forks and resyncs:

grep -l "force_resync\|ForceResync\|FORK\|fork detected"
~/.doli/devnet/logs/node*.log

grep -c "reorg\|Reorg" ~/.doli/devnet/logs/node*.log

Check reward distribution:

for i in 0 5 6; do

  balance=$(doli -r http://127.0.0.1:$((28545)) -w
~/.doli/devnet/keys/producer_$i.json balance 2>/dev/null)

  echo "Producer $i (bonds=$([ $i -eq 5 ] && echo 10 || ([ $i -le 4 ]
&& echo 1 || echo 2))): $balance"

done

Phase 5: Generate Report

Produce a report with:

Final height and hash consensus across all 21 nodes
Peer count distribution (min, max, average)
Fork events detected and how they resolved
Reward distribution by bond weight
Any anomalies, stuck nodes, or manual interventions
Sync time for late-joining nodes (6-20)
GossipSub mesh health (peers per node)


Iteration Protocol

If ALL success criteria pass:

STOP. The network is stable. Report results and update /network-setup
skill with any learnings.

If ANY criterion fails:

Diagnose root cause. Do NOT apply surface patches. Ask:

Would this fix work with 150K nodes?
Does Ethereum/Bitcoin handle this case? How?
Is this a protocol flaw or an implementation bug?

Fix at the root. Common root causes and where to look:

Nodes stuck at height 0 → sync manager state machine (manager.rs)
Nodes forked → fork recovery / reorg handler (node.rs)
Nodes isolated → peer discovery / DHT (service.rs)
Gossip not reaching nodes → GossipSub mesh parameters (gossip.rs)
Timing issues → slot/VDF validation (validation.rs, consensus.rs)

Update /network-setup skill if the fix changes any operational knowledge.
Return to Phase 0 and repeat the ENTIRE test from clean slate.

Iteration rules:

Maximum 5 iterations. If not stable after 5, escalate with full analysis.
Each iteration must produce a changelog: what was wrong, what was fixed, why.
Never fix the same bug twice — if it recurs, the first fix was wrong.
Every fix must include: "At 150K nodes, this fix works because..."


Anti-Patterns (NEVER do these)

❌ force_resync_from_genesis as recovery — that's a sledgehammer, not a protocol
❌ --no-dht in any devnet/testnet/mainnet command
❌ Manual restart of nodes to "fix" forks
❌ Sleep/delay hacks to avoid race conditions
❌ Increasing timeouts without understanding why they fire
❌ Fixes that work for 21 nodes but break at 150K
❌ Suppressing error logs instead of fixing the error

Pro-Patterns (ALWAYS do these)

✅ Automatic reorg to heaviest chain (like Bitcoin/Ethereum)
✅ DHT-based peer discovery on all networks
✅ Network isolation by network ID, not by disabling discovery
✅ GossipSub mesh self-healing via Kademlia peer pool
✅ Surgical chain-switch sync (find common ancestor, roll back, apply
heavier chain)
✅ Every fix validated against "would this work at 150K nodes?"