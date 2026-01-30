# roadmap.md - Development Roadmap

This document outlines the DOLI development roadmap and milestones.

---

## Current Status: Pre-Mainnet

**Version:** 0.x (Development)

The protocol is feature-complete and undergoing security review and testing before mainnet launch.

---

## Phase 1: Foundation (Completed)

### Core Protocol
- [x] UTXO transaction model
- [x] Ed25519 signatures
- [x] BLAKE3 hashing
- [x] Wesolowski VDF over class groups
- [x] Block validation rules
- [x] Transaction validation rules
- [x] Genesis block generation

### Consensus
- [x] Proof of Time consensus
- [x] Deterministic producer selection (round-robin)
- [x] Bond stacking (1-100 bonds per producer)
- [x] Weight-based fork choice (seniority)
- [x] Epoch-based active set management
- [x] Fallback producer mechanism

### Networking
- [x] libp2p integration
- [x] GossipSub for block/tx propagation
- [x] Kademlia DHT for peer discovery
- [x] Header-first synchronization
- [x] Parallel body download
- [x] Peer scoring and rate limiting

### Storage
- [x] RocksDB integration
- [x] Block storage with indexing
- [x] UTXO set management
- [x] Chain state persistence
- [x] Producer registry

### Tooling
- [x] Full node binary (doli-node)
- [x] CLI wallet (doli)
- [x] JSON-RPC API
- [x] Prometheus metrics

---

## Phase 2: Hardening (In Progress)

### Security
- [x] Equivocation detection
- [x] Automatic slashing transactions
- [x] Anti-Sybil defenses (chained VDF registration)
- [x] 40% veto threshold (count-based)
- [x] Seniority-based weight (no activity penalty)
- [ ] External security audit
- [ ] Formal verification of critical paths
- [ ] Fuzzing campaign completion

### Testing
- [x] Unit test coverage
- [x] Integration tests
- [x] Property-based tests (proptest)
- [x] Devnet deployment
- [ ] Testnet deployment
- [ ] Stress testing (600+ producers)
- [ ] Long-running stability tests

### Documentation
- [x] Whitepaper
- [x] Protocol specification
- [x] Architecture documentation
- [x] Security model
- [x] Node operation guides
- [x] RPC reference
- [ ] API client libraries (Rust, JS, Python)

---

## Phase 3: Mainnet Launch

### Launch Preparation
- [ ] Final security audit completion
- [ ] Testnet stability confirmation (30+ days)
- [ ] Genesis ceremony preparation
- [ ] Bootstrap node deployment
- [ ] Block explorer deployment
- [ ] Community communication

### Genesis
- [ ] Genesis block creation
- [ ] Initial producer set bootstrap
- [ ] Network monitoring setup
- [ ] Incident response procedures

### Post-Launch
- [ ] 24/7 monitoring for first month
- [ ] Rapid response capability
- [ ] Community support channels

---

## Phase 4: Ecosystem (Post-Mainnet)

### Wallet Improvements
- [ ] GUI wallet application
- [ ] Mobile wallet (iOS/Android)
- [ ] Hardware wallet integration (Ledger, Trezor)
- [ ] Multi-signature support

### Infrastructure
- [ ] Public block explorer
- [ ] Public RPC endpoints
- [ ] Prometheus/Grafana dashboards (public templates)
- [ ] Docker images

### Developer Tools
- [ ] JavaScript SDK
- [ ] Python SDK
- [ ] Rust SDK improvements
- [ ] WebSocket subscriptions (RPC)
- [ ] Transaction builder libraries

### Light Clients
- [ ] SPV client implementation
- [ ] Light client protocol specification
- [ ] Mobile light client

---

## Phase 5: Optimization (Future)

### Performance
- [ ] VDF computation optimization
- [ ] Block propagation improvements
- [ ] Storage compaction
- [ ] Memory usage reduction

### Scalability Research
- [ ] Transaction throughput analysis
- [ ] Pruning strategies
- [ ] Archive node separation

### Privacy Research
- [ ] Confidential transactions feasibility
- [ ] Payment channels feasibility
- [ ] Privacy-preserving light clients

---

## Non-Goals

The following are explicitly NOT planned:

| Feature | Reason |
|---------|--------|
| Smart contracts | Scope creep; other chains serve this |
| Sharding | Unnecessary complexity for payment chain |
| On-chain governance | Protocol should be stable |
| Inflation adjustment | Fixed emission by design |
| Account model | UTXO is simpler and proven |

---

## Version Numbering

| Version | Meaning |
|---------|---------|
| 0.x.x | Development, breaking changes possible |
| 1.0.0 | Mainnet launch |
| 1.x.x | Backwards-compatible improvements |
| 2.0.0 | Reserved for hard forks (if ever needed) |

---

## Timeline (Tentative)

| Milestone | Target |
|-----------|--------|
| Testnet launch | Q1 2026 |
| Security audit completion | Q1 2026 |
| Mainnet genesis | Q1 2026 (Feb 1) |
| GUI wallet | Q2 2026 |
| Mobile wallet | Q3 2026 |
| Hardware wallet support | Q4 2026 |

**Note:** Dates are targets, not commitments. Security and stability take precedence over schedule.

---

## Contributing

### Priority Areas
1. Security review and testing
2. Documentation improvements
3. Client library development
4. Tooling and infrastructure

### How to Contribute
1. Check open issues on GitHub
2. Discuss significant changes before implementing
3. Follow code conventions (see CLAUDE.md)
4. Include tests with code changes
5. Update documentation as needed

### Code Review
All changes require review before merge:
- Protocol changes: 2+ maintainer approvals
- Other changes: 1+ maintainer approval

---

## Research Directions

Long-term research areas (no commitment to implementation):

### VDF Improvements
- Alternative VDF constructions (proof size, verification speed)
- Proof size reduction
- Note: Hardware acceleration provides no advantage in DOLI—producer selection uses Epoch Lookahead, not VDF output

### Consensus Extensions
- Faster finality mechanisms
- Cross-chain communication
- State channels

### Privacy
- Confidential amounts
- Stealth addresses
- Zero-knowledge proofs integration

---

## Changelog

### v0.9.0 (Current)
- Feature-complete protocol implementation
- Devnet operational
- Documentation complete

### v0.8.0
- Bond stacking implementation
- Weight-based fork choice
- Anti-grinding with consecutive tickets

### v0.7.0
- Producer registration with VDF
- Slashing mechanism
- Auto-update with veto

### Earlier versions
- Core protocol development
- Initial networking
- Storage layer

---

*Roadmap last updated: January 2026*

*This document reflects current plans and is subject to change based on community feedback, security findings, and technical constraints.*
