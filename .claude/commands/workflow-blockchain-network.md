---
name: workflow:blockchain-network
description: Blockchain network infrastructure specialist — node setup, P2P networking, RPC infrastructure, network security, monitoring, and chain synchronization guidance.
---

# Workflow: Blockchain Network

Invoke ONLY the `blockchain-network` subagent for blockchain network infrastructure tasks.
Optional: `--scope="aspect"` to focus on a specific area (e.g., `--scope="rpc"`, `--scope="security"`, `--scope="monitoring"`, `--scope="sync"`, `--scope="validator"`).

## Task Types

The agent handles these categories of blockchain network infrastructure work:

1. **Node Setup** — deploy execution, consensus, validator, RPC, or archive nodes
2. **Network Analysis** — diagnose peer connectivity, sync issues, propagation delays
3. **RPC Infrastructure** — design load-balanced, rate-limited RPC/API endpoints
4. **Security Hardening** — firewall rules, peer filtering, eclipse attack prevention, DDoS protection
5. **Monitoring Setup** — Prometheus metrics, Grafana dashboards, alerting rules
6. **Network Topology Design** — multi-region deployment, peer diversity, geographic distribution
7. **Chain Synchronization** — choosing and configuring sync strategies (snap, full, archive, checkpoint)
8. **Validator Networking** — optimizing attestation/block propagation, reducing missed slots
9. **Client Migration** — switching between execution or consensus clients
10. **Multi-chain/Cross-chain** — bridge relayer networking, IBC configuration

## Supported Chains

- **Ethereum** (Geth, Reth, Nethermind, Erigon, Besu + Lighthouse, Prysm, Teku, Nimbus, Lodestar)
- **Cosmos / CometBFT** (any Cosmos SDK chain)
- **Solana** (validator and RPC nodes)
- **Substrate / Polkadot** (relay chain and parachains)
- **Other chains** — the agent adapts to any blockchain with P2P networking

## Usage Examples

```
/workflow:blockchain-network "set up an Ethereum validator node on Ubuntu with Reth + Lighthouse"
/workflow:blockchain-network "my Cosmos node keeps losing peers, help me diagnose" --scope="network-analysis"
/workflow:blockchain-network "design a production RPC infrastructure for Ethereum with load balancing" --scope="rpc"
/workflow:blockchain-network "harden my Solana validator against network attacks" --scope="security"
/workflow:blockchain-network "set up monitoring for my Ethereum node fleet with Prometheus and Grafana" --scope="monitoring"
/workflow:blockchain-network "migrate my execution client from Geth to Reth without downtime"
```

## What It Produces

Depending on the task type:
- **Configuration files** — client configs (`.toml`, `.yaml`, `.json`), docker-compose files, systemd service files
- **Shell scripts** — setup, maintenance, diagnostics, and health check scripts
- **Firewall rules** — explicit UFW/iptables rules for every port
- **Monitoring configs** — Prometheus scrape configs, alerting rules, Grafana data sources
- **Infrastructure reports** — analysis of current setup with findings and recommendations
- **Documentation** — node setup guides, operational runbooks, security checklists

## Single Agent Workflow

The blockchain-network agent handles the full lifecycle:

1. **Assess** — understand the user's blockchain, node type, and task
2. **Research** — verify current client versions and known issues
3. **Design** — produce the solution with exact configurations
4. **Validate** — verify config syntax and flag unverified options
5. **Deliver** — write approved files to disk

## Fail-Safe Controls

### Scope Awareness
- If `--scope` is provided, the agent focuses exclusively on that aspect
- If no scope is provided, the agent determines the appropriate scope from the task description

### Safety Guards
- The agent will REFUSE to produce configurations that expose admin APIs to the public internet
- The agent will WARN before recommending changes to running production nodes
- Every configuration includes rollback instructions
- Every deployment includes firewall rules

### Progress Recovery
If context limits are reached, the agent saves progress to `docs/.workflow/blockchain-network-progress.md`. The user can resume by re-invoking this command with the same task description.
