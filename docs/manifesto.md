# manifesto.md - Project Philosophy

*"Time is the only fair currency."*

---

## The Problem

Fifteen years after Bitcoin's genesis block, the promise of decentralized money remains unfulfilled for most of humanity.

**Proof of Work** created an arms race. What began as CPU mining became GPU farms, then ASICs, then industrial operations consuming more electricity than mid-sized countries. The average person cannot participate in consensus. Mining pools—centralized intermediaries—now control block production.

**Proof of Stake** replaced energy with capital. Those with the most coins determine the chain's future. Wealth begets wealth. Staking pools emerged to aggregate small holdings, recreating the very intermediaries blockchain was meant to eliminate. The barrier to entry shifted from hardware to wealth.

Both systems share a fundamental flaw: **they require scarce resources that can be accumulated.**

Hashpower can be bought. Stake can be concentrated. Storage can be hoarded.

The rich get richer. The powerful stay powerful. Meet the new boss, same as the old boss.

---

## The Insight

What if the scarce resource could not be accumulated?

**Time cannot be hoarded.** A billionaire experiences one second per second, same as anyone else. A nation-state with unlimited budget cannot acquire "more time" than a student with a laptop.

This is not a new observation—it's why we measure interest rates in time, why we speak of "spending" time. But until recently, there was no way to prove that sequential time had passed without trusting the prover.

Verifiable Delay Functions changed this. First formalized in 2018, VDFs allow anyone to prove that a specific number of sequential operations were performed. The computation cannot be parallelized. Time becomes verifiable.

---

## Our Principles

### 1. Time as Foundation

DOLI anchors consensus to sequential time. Block production requires computing a VDF that takes approximately 700ms of sequential computation. This cannot be accelerated by adding parallel hardware.

One hour of DOLI consensus requires one hour of wall-clock time. This is true whether you're an individual running a single node or a corporation running a thousand.

### 2. Deterministic, Not Probabilistic

In Bitcoin, a miner with 1% of hashpower has a 1% chance of finding each block. This means extreme variance—they might find two blocks in a day, or none in a month. Pools exist to smooth this variance.

In DOLI, producer selection is deterministic round-robin based on bond count. A producer with 1% of total bonds produces exactly 1% of blocks. There is no luck. There is no variance. There is no need for pools.

**The smallest participant receives the same percentage return as the largest.**

### 3. Minimal Viable Protocol

DOLI implements what is necessary and nothing more:

- UTXO model for transactions (proven, simple, parallelizable)
- Ed25519 signatures (fast, secure, well-studied)
- BLAKE3 hashing (modern, fast, secure)
- VDF over class groups (sequential, verifiable)

We reject complexity for its own sake. Every feature is a potential attack surface. Every option is a decision someone must make. The protocol should be simple enough that one person can understand it completely.

### 4. No Special Treatment

- No premine
- No ICO
- No treasury
- No foundation allocation
- No investor tokens
- No team rewards

Every DOLI in circulation comes from block production. The genesis block contains exactly one coinbase transaction with the standard 1 DOLI reward.

The founders participate on equal footing with everyone else. We register as producers, lock our bonds, and earn rewards by producing blocks—same as anyone.

### 5. Code is Law

Transactions are final. There is no mechanism to reverse transactions, freeze accounts, or modify history.

| Situation | Protocol Response |
|-----------|-------------------|
| Lost private key | Funds lost permanently |
| Erroneous transaction | Not reversible |
| Court order | Not enforceable |
| "Hack" recovery | Not possible |

This is not a bug. This is the feature. A money that can be censored or reversed is not sound money—it's permission with extra steps.

### 6. Predictable Emission

The supply schedule is fixed at genesis:

- 25,228,800 DOLI total supply
- 1 DOLI per block (Era 1)
- Halving every ~4 years
- No inflation surprises
- No governance votes to change emission

You can calculate exactly how many DOLI will exist at any future date. This predictability is a feature, not a limitation.

---

## What We Are Not

### Not a "Platform"

DOLI is money. It is not trying to be a world computer, a database, or an application platform. Other projects serve those purposes. DOLI transfers value.

### Not "Innovative"

Every component of DOLI uses well-understood cryptography and proven designs. The UTXO model is from Bitcoin. VDFs were formalized by Boneh et al. Ed25519 is standard. BLAKE3 is documented.

The innovation, if any, is in the combination: using VDFs not as a randomness beacon but as the primary consensus mechanism. This is novel, but the components are not.

### Not Governed

There is no on-chain governance. There are no token votes on protocol parameters. The rules are the rules.

Software updates follow a conservative path: maintainers propose, producers can veto. The threshold for veto (40%) is high enough to prevent frivolous blocks but low enough to stop malicious changes.

This is not democracy. It is closer to constitutional constraint—the rules are hard to change by design.

---

## Why Now

The cryptographic primitives for Proof of Time didn't exist until 2018. The engineering to make them practical took additional years. The infrastructure to deploy (libp2p, async Rust, modern databases) matured recently.

We build now because we can build now.

The need has always existed. People deserve money that:
- Cannot be debased by governments
- Cannot be censored by corporations
- Does not require trusting intermediaries
- Does not require industrial-scale operations to participate
- Provides fair returns regardless of wealth

Previous attempts required compromises. Proof of Time eliminates them.

---

## The Future We Seek

In ten years:

- A student in Lagos runs a DOLI producer on a commodity laptop, earning the same percentage return as a fund in Singapore
- Cross-border payments settle in 10 seconds without correspondent banks
- Savings cannot be inflated away by monetary policy
- No one asks permission to transact
- The protocol is unchanged because it didn't need to change

This future requires no faith in the goodness of institutions or the wisdom of governance. It requires only math, code, and time.

**Time is the only fair currency.**

---

## Genesis Message

The genesis block of DOLI contains:

> *"Time is the only fair currency. 01/Feb/2026"*

This serves as proof that no blocks were mined before February 1, 2026, and as a permanent statement of intent.

---

*DOLI v1.0*

*E. Weil*
*weil@doli.network*
