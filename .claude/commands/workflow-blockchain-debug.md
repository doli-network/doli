---
name: workflow:blockchain-debug
description: Blockchain debug specialist — diagnoses and fixes active connectivity problems, peer failures, sync issues, RPC unreachability, and Engine API breakdowns on blockchain nodes.
---

# Workflow: Blockchain Debug

Invoke ONLY the `blockchain-debug` subagent for diagnosing and fixing active blockchain connectivity problems.
Optional: `--scope="layer"` to focus on a specific diagnostic area (e.g., `--scope="peers"`, `--scope="sync"`, `--scope="rpc"`, `--scope="engine-api"`, `--scope="firewall"`).

## When to Use This

Use this command when something is **broken right now**:
- Nodes have zero peers or are losing peers
- Sync is stuck or regressing
- RPC endpoint is unreachable
- Engine API (EL<->CL) is failing
- Validators are missing attestations
- Nodes are partitioned from the network

Do **NOT** use this for:
- Setting up new nodes from scratch -> use `/workflow:blockchain-network`
- Designing network topology -> use `/workflow:blockchain-network`
- Writing monitoring/alerting configs -> use `/workflow:blockchain-network`
- Security auditing or hardening -> use `/workflow:blockchain-network --scope="security"`
- Performance optimization -> use `/workflow:blockchain-network`

## Supported Chains

- **Ethereum** (Geth, Reth, Nethermind, Erigon, Besu + Lighthouse, Prysm, Teku, Nimbus, Lodestar)
- **Cosmos / CometBFT** (any Cosmos SDK chain)
- **Solana** (validator and RPC nodes)
- **Substrate / Polkadot** (relay chain and parachains)
- **Other chains** — the agent adapts diagnostic methodology to any blockchain with P2P networking

## Usage Examples

```
/workflow:blockchain-debug "my Geth node has 0 peers and sync is stuck"
/workflow:blockchain-debug "Lighthouse says el_offline:true, Engine API not connecting" --scope="engine-api"
/workflow:blockchain-debug "Cosmos validator keeps losing persistent peers" --scope="peers"
/workflow:blockchain-debug "RPC endpoint returns connection refused on port 8545" --scope="rpc"
/workflow:blockchain-debug "Solana validator is delinquent and falling behind"
/workflow:blockchain-debug "validator missing attestations every other epoch" --scope="sync"
```

## What It Produces

- **Root Cause Analysis report** saved to `docs/.workflow/blockchain-debug-rca.md` with:
  - Symptoms confirmed with diagnostic evidence
  - Diagnosis steps taken (including failed hypotheses)
  - Root cause identified with evidence
  - Fix applied (after user approval)
  - Verification that the fix worked
  - Prevention recommendations

## Debug Methodology

The agent follows a strict 7-phase methodology:

```
Phase 1: Gather Symptoms    -> understand what is broken, extract key data points
Phase 2: Confirm the Issue   -> verify the symptom is real using diagnostic commands
Phase 3: Isolate the Layer   -> network/transport, protocol, application, or inter-component
Phase 4: Diagnose Root Cause -> targeted diagnostics within the identified layer
Phase 5: Fix                 -> propose fix, WAIT for user approval, apply
Phase 6: Verify              -> independently confirm the fix resolved the issue
Phase 7: Document            -> produce Root Cause Analysis report
```

## Safety Controls

### Read-Only by Default
All diagnostic commands (checking ports, reading logs, querying APIs) run without requiring approval. State-changing commands (restarts, config edits, data deletion) always require explicit user approval.

### Destructive Action Gate
Before any command that restarts a service, modifies a config file, deletes data, or changes firewall rules, the agent:
1. Presents the exact command
2. Explains the risk
3. Provides rollback instructions
4. Waits for explicit "yes" before proceeding

### Security Guardrails
The agent will refuse to:
- Disable JWT authentication
- Open all ports as a "quick fix"
- Bind RPC to 0.0.0.0 without auth
- Expose admin APIs to the internet

### Progress Recovery
If context limits are reached during a complex investigation, the agent saves progress to `docs/.workflow/blockchain-debug-progress.md` with symptoms confirmed, hypotheses tested, and remaining investigation steps.
