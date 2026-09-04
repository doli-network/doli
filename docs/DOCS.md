# DOCS - Documentation Index

Master index for all DOLI documentation.

---

## Specifications

| File | Description |
|------|-------------|
| [WHITEPAPER.md](/WHITEPAPER.md) | Complete protocol whitepaper - VDF-based blockchain with Proof of Time (PoT) consensus |
| [protocol.md](./protocol.md) | Wire protocol and message format specifications |
| [architecture.md](./architecture.md) | System architecture and component design (includes GUI desktop app - Section 11) |
| [security_model.md](./security_model.md) | Security model, threat analysis, and cryptographic guarantees |

## Networks

| File | Description |
|------|-------------|
| [genesis.md](./genesis.md) | **Mainnet genesis launch guide** - critical checklist, key generation, and launch procedure |
| [testnet.md](./testnet.md) | Testnet information, bootstrap nodes, and setup guide for joining |
| [devnet.md](./devnet.md) | Devnet guide - local development environment, bootstrap mode, configuration |
| [genesis_ceremony.md](./genesis_ceremony.md) | Genesis ceremony procedures and verification |
| [infrastructure.md](./infrastructure.md) | Infrastructure layout - servers, DNS, directory structure, service configuration |

## Guides

| File | Description |
|------|-------------|
| [rewards.md](./rewards.md) | Block rewards system - pooled epoch model, emission schedule, maturity rules |
| [cli.md](./cli.md) | Complete CLI reference - all WHITEPAPER operations via command line |
| [docker.md](./docker.md) | Docker deployment guide - containers, compose, and monitoring |
| [running_a_node.md](./running_a_node.md) | Node setup, operation, and **environment configuration** (.env files) |
| [becoming_a_producer.md](./becoming_a_producer.md) | Block producer onboarding |
| [rpc_reference.md](./rpc_reference.md) | RPC API documentation |
| [troubleshooting.md](./troubleshooting.md) | Common issues and solutions (disk-full/ENOSPC crash-loop + log rotation section lands with Option 1 M2 — design: `specs/disk-guardian-architecture.md`) |
| [archiver.md](./archiver.md) | **Block archiver & seed infrastructure** - archive format, seed/relay role, block explorer, disaster recovery, RPC methods |
| [disaster-recovery.md](./disaster-recovery.md) | Disaster recovery procedures (restore, backfill, hot backfill) |
| [releases.md](./releases.md) | Release process, versioning, and download verification |
| [buy_doli.md](./buy_doli.md) | DOLI/USDT exchange system - API, deployment, and operational guide |
| [faucet-bot.md](./faucet-bot.md) | Testnet faucet bot setup and operation |
| [producer_node_quickstart.md](./producer_node_quickstart.md) | Quick-start guide for producer node setup |
| [producer-ux-proposal.md](./producer-ux-proposal.md) | Producer UX improvement proposal |
| [configuration_verification.md](./configuration_verification.md) | Configuration verification procedures |

## Governance

| File | Description |
|------|-------------|
| [manifesto.md](./manifesto.md) | Project philosophy and principles |
| [roadmap.md](./roadmap.md) | Development roadmap and milestones |
| [auto_update_system.md](./auto_update_system.md) | Complete auto-update system - fail-closed TrustRoot release verification (INC-I-172 F1), distinct-signer 3-of-5, node-local veto/grace timing, count-based 40% producer veto (the seniority-weighted veto was deleted in INC-I-172 F8), binary replacement, rollback |

## Testing & Research

| File | Description |
|------|-------------|
| [whitepaper_test_plan.md](./whitepaper_test_plan.md) | Complete test plan for ALL WHITEPAPER functionalities |
| [battle_test.md](./battle_test.md) | Battle testing scenarios and results |
| [attack_analysis.md](./attack_analysis.md) | Security analysis and attack vectors. Carries a correction banner (INC-I-172 F8): every claim that a VETO vote is seniority-weighted is false; the veto is a producer head count whose only Sybil barrier is the registration bond |
| [extreme_devnet_600.md](./extreme_devnet_600.md) | Extreme network testing results |

## Redesign Analysis

| File | Description |
|------|-------------|
| [redesigns/state-of-the-art-redesign-analysis.md](./redesigns/state-of-the-art-redesign-analysis.md) | DOLI vs. Web3 comparative analysis scoping document - capability inventory, gap re-analysis, acceptance criteria, requirements (REQ-SOTA-NNN). Input to the state-of-the-art architecture proposal at `specs/state-of-the-art-architecture.md`. |
| [redesigns/defi-subsystem-redesign-analysis.md](./redesigns/defi-subsystem-redesign-analysis.md) | DeFi subsystem (AMM, Lending, NFT Frac) scoping document — capability inventory (31 TX types, 15 output types, 11 condition primitives), 5 confirmed defects (DEF-1…DEF-5), 6 open design decisions, hard constraints C1-C7. Input to `specs/defi-subsystem-architecture.md`. |
| [redesigns/defi-foundations-redesign-analysis.md](./redesigns/defi-foundations-redesign-analysis.md) | DeFi foundations analyst scoping — 8 candidate primitives, 13 economic invariants, 12 acceptance criteria, agent-readiness rubric (4/4). Input to `specs/defi-foundations-economics.md` and `specs/defi-l1-foundations-architecture.md`. |
| [redesigns/oracle-structural-anchored-redesign-analysis.md](./redesigns/oracle-structural-anchored-redesign-analysis.md) | Oracle Phase 2.1 analyst scoping — 10 economic invariants, 7 acceptance criteria, 10 open design questions, capability inventory (free TxType 16, free OutputType 15). Input to `specs/oracle-structural-anchored-economics.md` (implemented 2026-05-25 behind `oracle_activation_height = u64::MAX`; M4–M8 shipped, RPC methods M9–M11 deferred). |
| [redesigns/event-subscriptions-redesign-analysis.md](./redesigns/event-subscriptions-redesign-analysis.md) | Event subscriptions Phase 2.2 analyst scoping — 10 operational invariants (EV-1..10), 8 acceptance criteria (AC-EV-1..8), 9 open design questions, existing WS capability inventory. Input to `specs/event-subscriptions.md` (proposal-only, not yet implemented; pending User Gate approval). |
| [redesigns/epoch-liveness-prune-redesign-analysis.md](./redesigns/epoch-liveness-prune-redesign-analysis.md) | Epoch-boundary liveness prune analyst scoping (INC-I-116) -- capability inventory (5 liveness signals, 8 epoch-boundary ops, 19 activation heights), root-cause reframe (filter exists, floor neutralizes it), 14 requirements (REQ-PRUNE-001..014), 10 open design questions. Input to `specs/epoch-liveness-prune-architecture.md`. |
| [redesigns/sync-snap-admission-redesign-analysis.md](./redesigns/sync-snap-admission-redesign-analysis.md) | SnapSync admission analyst scoping (INC-I-139) -- capability inventory (1 chokepoint X1, 3-term OR guard, 7 funnel feeders B1-B7, 1 redirect A1), BRITTLE verdict 4/5, 12 requirements (REQ-SNAP-001..012), zero-margin coupling (threshold==MINOR_FORK_GAP_MAX==50), 4 open questions. Input to `specs/sync-snap-admission-architecture.md`. |
| [redesigns/state-root-redesign-analysis.md](./redesigns/state-root-redesign-analysis.md) | State-root / 3-state commitment analyst scoping -- verified the root is NOT block-consensus-validated (no BlockHeader field, zero comparisons; snap-sync quorum anchor + diagnostics only), corrected call-site count 15->6, identified the dominant cost as the full CF_UTXO scan + deserialize + re-serialize + alloc (state_db/queries.rs:473) not BLAKE3, ~15-21x headroom vs the 16 MB metric. 10 requirements (REQ-SROOT-001..010). Input to `specs/state-root-commitment-architecture.md`. |
| [redesigns/attestation-verification-redesign-analysis.md](./redesigns/attestation-verification-redesign-analysis.md) | Attestation/vote verification scaling analyst scoping (INC-I-141) -- falsified the premise (per-block O(N) attestation verify does NOT exist on the live path; BLS aggregate verify is unreachable dead code; producer_bls_keys never populated), located the real O(N) term on the gossip-receive path (per-attestation Ed25519, off block critical path), 9 requirements (REQ-ATT-001..009). NOTE: its §1.2 topic claim (VOTES_TOPIC) is WRONG -- live topic is ATTESTATION_TOPIC (corrected in the spec). Input to `specs/attestation-gossip-scaling-architecture.md`. |
| [redesigns/maintainer-trust-root-redesign-analysis.md](./redesigns/maintainer-trust-root-redesign-analysis.md) | Maintainer / update-signing trust-root analyst scoping (INC-I-172) -- verified-claims table (5 corrections: veto is 5min not 7 days, unweighted not seniority-weighted, run.rs BLAKE3 blocker FALSE, no replay-from-genesis derivation, no consensus signature check), capability inventory (24 TxTypes, MaintainerSet 14 methods, 4 updater verify fns), veto-math verdict (compromised quorum defeats veto but NOT via seniority), F1 quorum already fully public (5/5, ~138 days), MoSCoW REQ-172-001..019. Input to `specs/maintainer-trust-root-architecture.md`. |
| [redesigns/attestation-bls-redesign-analysis.md](./redesigns/attestation-bls-redesign-analysis.md) | Attestation/BLS analyst scoping (INC-I-178) -- verified the deleted aggregate path was cryptographically incoherent (multi-message aggregation vs same-message verify vs wrong message), 1 encoder / 5 decoders / 1 stray-bit validator with divergent denominators, INC-I-191/192 already fixed by `13daee6f` (memory.db drift), no BLS key-rotation TxType, REQ-BLS-001..023 (MoSCoW). Input to `specs/attestation-bls-architecture.md`. |
| [.workflow/architecture-reasoning.md](./.workflow/architecture-reasoning.md) | Design-synthesizer reasoning trace for `specs/attestation-bls-architecture.md` -- conclusion-first convergence matrix, 10 contradictions resolved by code (X1-X10: 869 us vs 202 ms verify, 3 vs 4 denominators, INC-I-146 link refuted, 30/60 vs 54/60 both live), rejected evaluator claims, failure-mode filter log, radical tiebreaker (gap 0.00 -> minimum presented alone), confidence evolution. |
| [redesigns/state-only-fee-gate-redesign-analysis.md](./redesigns/state-only-fee-gate-redesign-analysis.md) | State-only tx fee-gate analyst scoping (INC-I-173) -- confirmed the 3-type utxo.rs:222 list is narrower than is_state_only() (9), so AddMaintainer/RemoveMaintainer + SlashProducer + ProtocolActivation are admitted/relayed but never mineable; 4 corrections (naive is_state_only() swap breaks genesis Registration; ProtocolActivation dies earlier at mempool FeeTooLow; ClaimReward/ClaimBond fail the balance check not the fee check -> conjunct is the mint guard; SlashProducer is node-generated and the highest-severity casualty). Root cause = drift between 5 hand-maintained lists (INC-I-057 precedent, ungated). INV-12 = activation height REQUIRED. 18 requirements (REQ-173-001..018), BRITTLE verdict. Input to `specs/state-only-fee-gate-architecture.md`. |
| [redesigns/maintainer-authorization-redesign-analysis.md](./redesigns/maintainer-authorization-redesign-analysis.md) | Maintainer authorization analyst scoping (INC-I-176) -- verified capability inventory, the ONE verification site (governance.rs:39,75, Option-return non-fatal), the shared-validator structural-only check (tx_types.rs:753), unauthenticated RPC, byte-identical mainnet/testnet key arrays, INV-12 = activation height REQUIRED (#22), BRITTLE 3/5, acceptance criteria REQ-176-NNN. Input to `specs/maintainer-authorization-architecture.md` (5-evaluator convergence synthesis, PROPOSAL-ONLY). |

## Legacy

Archived historical documents in `legacy/` subdirectory.

---

## Quick Navigation

```
docs/
├── DOCS.md                       # <- You are here (master index)
├── architecture.md               # System architecture
├── archiver.md                   # Block archiver & seed infrastructure
├── attack_analysis.md            # Security analysis
├── auto_update_system.md         # Auto-update documentation
├── battle_test.md                # Battle testing
├── becoming_a_producer.md        # Producer guide
├── buy_doli.md                   # Exchange system guide
├── cli.md                        # CLI reference
├── configuration_verification.md # Configuration verification
├── devnet.md                     # Devnet guide
├── disaster-recovery.md          # Disaster recovery procedures
├── docker.md                     # Docker deployment
├── extreme_devnet_600.md         # Extreme testing results
├── faucet-bot.md                 # Testnet faucet bot
├── genesis.md                    # Genesis launch guide
├── genesis_ceremony.md           # Genesis ceremony procedures
├── infrastructure.md             # Infrastructure layout
├── manifesto.md                  # Project philosophy
├── producer-ux-proposal.md       # Producer UX proposal
├── producer_node_quickstart.md   # Producer quickstart
├── protocol.md                   # Wire protocol spec
├── redesigns/
│   └── state-of-the-art-redesign-analysis.md  # Comparative analysis scoping
├── releases.md                   # Release process
├── rewards.md                    # Block rewards (pooled epoch)
├── roadmap.md                    # Development roadmap
├── rpc_reference.md              # API documentation
├── running_a_node.md             # Node operation guide
├── security_model.md             # Security model
├── testnet.md                    # Testnet setup
├── troubleshooting.md            # Common issues
├── whitepaper_test_plan.md       # Whitepaper test plan
└── legacy/                       # Archived documents
```

---

## Usage

Use `[[UID:*]]` grep patterns to navigate directly to specific sections within documentation files.
