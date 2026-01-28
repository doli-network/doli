# DOLI Roadmap

This document outlines the development roadmap for the DOLI protocol and ecosystem.

## Timeline Overview

```
2024 Q4 ─────── Research & Design
2025 Q1-Q2 ──── Core Development
2025 Q3 ─────── Testnet Launch
2025 Q4 ─────── Security Audits
2026 Q1 ─────── Mainnet Launch (Genesis: Feb 1, 2026)
2026+ ───────── Ecosystem Growth
```

---

## Phase 0: Research & Design (2024 Q4)

### Objectives
- Complete protocol specification
- Finalize VDF construction choice
- Design producer registration mechanism
- Economic modeling and simulation

### Deliverables
- [x] Whitepaper v1.0
- [ ] Protocol specification document
- [ ] Economic simulation results
- [ ] VDF benchmark analysis

### Key Decisions
- VDF construction: Hash-chain VDF (iterated SHA-256) with ~700ms target
- Hash function: BLAKE3-256
- Signature scheme: Ed25519
- Slot duration: 60s mainnet, 10s testnet, 5s devnet
- Bond mechanism with 30% era decay
- Epoch Lookahead for grinding prevention

---

## Phase 1: Core Development (2025 Q1-Q2)

### Q1 2025: Foundation

#### VDF Implementation
- [x] Hash-chain VDF (iterated SHA-256)
- [x] Dynamic calibration system (~700ms target)
- [x] VDF verifier (recompute chain)
- [x] Benchmark suite and test vectors

#### Cryptographic Primitives
- [ ] BLAKE3-256 integration
- [ ] Ed25519 key generation and signing
- [ ] Merkle tree implementation
- [ ] Address derivation

#### Data Structures
- [ ] Transaction format (UTXO model)
- [ ] Block header and body
- [ ] Producer registration transaction
- [ ] Bond output type

### Q2 2025: Networking & Consensus

#### P2P Network Layer
- [ ] Node discovery (DHT-based)
- [ ] Block propagation protocol
- [ ] Transaction mempool
- [ ] Peer reputation system

#### Consensus Engine
- [ ] Slot timing and genesis anchor
- [ ] Producer selection algorithm
- [ ] Chain selection rule (slot > height > hash)
- [ ] Fork detection and resolution

#### Producer System
- [ ] Registration VDF computation
- [ ] Dynamic difficulty adjustment
- [ ] Bond locking mechanism
- [ ] Inactivity tracking

#### Validation
- [ ] Transaction validation
- [ ] Block validation pipeline
- [ ] VDF proof verification
- [ ] Slashing evidence processing

---

## Phase 2: Testnet (2025 Q3)

### Testnet Alpha (July 2025)
- [ ] Internal testing network
- [ ] Core team nodes only
- [ ] Focus on consensus stability
- [ ] VDF parameter calibration

### Testnet Beta (August 2025)
- [ ] Public testnet launch
- [ ] Faucet for test coins
- [ ] Block explorer
- [ ] Basic wallet (CLI)

### Testing Focus
- [ ] Consensus under adversarial conditions
- [ ] Network partition recovery
- [ ] Producer registration stress test
- [ ] Slashing mechanism verification

### Tooling
- [ ] CLI wallet
- [ ] Block explorer
- [ ] Network statistics dashboard
- [ ] Log analysis tools

---

## Phase 3: Security & Audits (2025 Q4)

### External Audits
- [ ] Cryptographic review (VDF implementation)
- [ ] Protocol security audit
- [ ] Smart contract / transaction logic audit
- [ ] Network layer security review

### Bug Bounty Program
- [ ] Launch private bug bounty
- [ ] Engage security researchers
- [ ] Remediation of findings

### Documentation
- [ ] Complete API documentation
- [ ] Node operator guide
- [ ] Producer setup guide
- [ ] Security best practices

---

## Phase 4: Mainnet Launch (2026 Q1)

### Genesis Preparation (January 2026)
- [ ] Final security patches
- [ ] Genesis block parameters
- [ ] Initial node operators confirmed
- [ ] Launch coordination

### Genesis Event (February 1, 2026)
```
GENESIS_TIME = 2026-02-01T00:00:00Z
             = Unix timestamp 1769904000
```

### Bootstrap Phase
- First 10,080 blocks (~1 week)
- Open block production (no registration required)
- Producers register in parallel
- Producer set activates at block 10,080

### Post-Launch
- [ ] Network monitoring
- [ ] Incident response team
- [ ] Community support channels
- [ ] Exchange listings (organic)

---

## Phase 5: Ecosystem Growth (2026+)

### Developer Tools
- [ ] SDK for multiple languages (Rust, Go, TypeScript)
- [ ] RPC API specification
- [ ] Light client library
- [ ] Hardware wallet integration

### Wallet Ecosystem
- [ ] Desktop wallet (GUI)
- [ ] Mobile wallet (iOS/Android)
- [ ] Web wallet
- [ ] Hardware wallet support (Ledger, Trezor)

### Infrastructure
- [ ] Public RPC endpoints
- [ ] Block explorer enhancements
- [ ] Historical data archives
- [ ] Analytics platform

### Research
- [ ] VDF hardware acceleration monitoring
- [ ] T parameter adjustment analysis
- [ ] Network topology optimization
- [ ] Privacy enhancements research

---

## Technical Milestones

### VDF Performance Targets
| Hardware Tier | Block VDF (~700ms) | Registration VDF (~10min) |
|---------------|-------------------|---------------------------|
| Consumer CPU  | ✓ baseline        | ✓ baseline                |
| Server CPU    | ~600ms            | ~9 min                    |
| ASIC (future) | Monitor           | Monitor                   |

### Network Targets
| Metric              | Target          |
|---------------------|-----------------|
| Block propagation   | < 5 seconds     |
| Transaction latency | < 10 seconds    |
| Node count (year 1) | > 1,000         |
| Producer count      | > 100           |

### Performance Targets
| Metric               | Target              |
|----------------------|---------------------|
| TPS (sustained)      | > 100               |
| Block validation     | < 500ms             |
| VDF verification     | < 100ms             |
| Light client sync    | < 1 min (1 year)    |

---

## Community Milestones

### 2025
- [ ] Developer documentation complete
- [ ] Community Discord/Matrix
- [ ] First external contributors
- [ ] Testnet participation program

### 2026
- [ ] Mainnet launch event
- [ ] First producer operators (non-team)
- [ ] Exchange listings
- [ ] Merchant adoption pilots

### 2027+
- [ ] Decentralized governance discussion
- [ ] Protocol upgrade process
- [ ] Ecosystem grants program
- [ ] Academic partnerships

---

## Risk Factors & Mitigations

### VDF Hardware Acceleration
- **Risk**: Custom hardware breaks time assumptions
- **Mitigation**: T parameter can increase; detection via block timing

### Regulatory
- **Risk**: Cryptocurrency regulations
- **Mitigation**: Decentralized development, no pre-mine, no ICO

### Adoption
- **Risk**: Low producer participation
- **Mitigation**: Bootstrap phase with open production, economic incentives

### Security
- **Risk**: Protocol vulnerabilities
- **Mitigation**: Multiple audits, bug bounty, gradual rollout

---

## Version History

| Version | Date       | Changes                    |
|---------|------------|----------------------------|
| 0.1     | 2024-01    | Initial roadmap            |
| 1.0     | 2025-01    | Aligned with whitepaper v1 |

---

*This roadmap is subject to change based on development progress and community feedback.*

**Contact**: doli@protonmail.com
**Website**: www.doli.network
