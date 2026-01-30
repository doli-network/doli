# Attack Analysis: DOLI Anti-Sybil Defenses

This document analyzes potential attack vectors against the DOLI protocol and explains how the implemented defenses protect against them.

## Overview

DOLI implements multiple anti-Sybil mechanisms:

1. **Weight by Seniority** - Producer power increases with time active (1-4 years)
2. **Chained VDF Registration** - Sequential registration prevents parallel attacks
3. **Maturity Cooldown on Exit** - Producers who exit lose all accumulated seniority
4. **Activity Gap Penalty** - Dormant producers lose up to 50% of effective weight
5. **40% Veto Threshold** - Higher threshold makes governance attacks more expensive
6. **Bond Stacking with Anti-Whale Cap** - Max 100 bonds (100,000 DOLI) per producer
7. **Deterministic Round-Robin** - No lottery variance, guaranteed proportional allocation

These defenses work together to protect the network against various attack vectors.

---

## Attack Vector 1: Whale Instant Takeover

### Description
An attacker with significant capital attempts to instantly gain majority control of the network by registering many producer nodes simultaneously.

### Attack Scenario
1. Whale accumulates capital to cover bond requirements for 200+ nodes
2. Whale attempts to register all 200 nodes in the same block/epoch
3. With 200 nodes vs 100 existing, whale has 66% of producers
4. Whale immediately pushes malicious software update

### Defenses

**Chained VDF Registration** prevents parallel registration:
- Each registration must reference the hash of the previous registration
- Registrations must be sequential, not parallel
- Attacker can only register one node per registration-processing window
- At one registration per block, 200 registrations takes 200 blocks (~3.3 hours)

**Weight by Seniority** nullifies instant influence:
- All 200 new nodes start with weight 1
- Existing producers with 1-4 years tenure have weights 2-4
- Even with 200 nodes, attacker's total weight is only 200
- 50 senior producers (4 years) have total weight 200
- Veto threshold is 40% of *weight*, not *count*

### Result
Attack fails because:
- Registrations cannot happen instantly
- New producers have minimal voting power
- Senior producers can veto any malicious proposals

---

## Attack Vector 2: Long-Term Sybil Infiltration

### Description
An attacker slowly registers nodes over several years to accumulate enough seniority weight to dominate voting.

### Attack Scenario
1. Attacker registers 50 nodes over 4 years (slow infiltration)
2. After 4 years, each node has weight 4 (total: 200 weight)
3. Attacker attempts to push malicious update
4. Network has 100 other producers with varying weights

### Defenses

**Economic Cost** makes attack expensive:
- Bond requirement: 1000 DOLI per node (Era 0)
- 50 nodes × 1000 DOLI = 50,000 DOLI locked for 4 years
- Bond decreases over time but still significant

**Distributed Veto Power**:
- 100 honest producers with average weight 2.5 = 250 total weight
- Attacker's 200 weight is only 44% of total (200/450)
- Veto threshold at 40% requires attacker to control significant weight
- Any suspicious activity triggers community response

**Network Growth**:
- Over 4 years, honest producer count likely grows
- Early entrants have equal seniority advantage
- Attack cost increases as network grows

### Result
Attack requires massive sustained capital investment with uncertain outcome. Economic incentives favor honest participation.

---

## Attack Vector 3: Hit-and-Run Voting

### Description
Producer registers, votes for malicious proposal, then exits before consequences materialize.

### Attack Scenario
1. Producer registers and gains seniority
2. Producer votes to approve malicious software update
3. Producer immediately initiates exit
4. Malicious update causes damage
5. Producer re-registers to repeat

### Defenses

**Maturity Cooldown on Exit**:
- Exit history is permanently recorded
- Re-registering producers start fresh at weight 1
- All accumulated seniority is lost
- 30-day unbonding period gives time to detect malicious votes

**Exit History Tracking**:
- `has_prior_exit` flag marks repeat registrants
- Producers with prior exits have reduced trust
- Community can identify suspicious re-registration patterns

### Result
Attack is costly (lost seniority) and detectable (prior exit flag). Rational actors avoid this strategy.

---

## Attack Vector 4: Coordinated Rapid Exit

### Description
Attacker coordinates multiple compromised producers to vote, exit, and re-register in synchronized waves.

### Attack Scenario
1. Attacker controls 30 producer keys
2. All 30 vote for malicious proposal
3. All 30 exit simultaneously
4. After unbonding, all 30 re-register
5. Repeat cycle

### Defenses

**Chained Registration** prevents coordinated re-entry:
- Even if 30 exit, they can only re-register one at a time
- Sequential requirement breaks coordination
- Months to rebuild presence

**Permanent Seniority Loss**:
- Each re-registration starts at weight 1
- 30 producers × weight 1 = only 30 total weight
- Existing honest producers maintain higher weights

**30-Day Unbonding Window**:
- Community has 30 days to analyze suspicious voting
- Time to implement countermeasures
- Social/economic consequences can be applied

### Result
Coordination overhead is high, seniority recovery is slow, and attack window is limited.

---

## Attack Vector 5: VDF Computation Farm

### Description
Attacker uses specialized hardware to pre-compute VDF proofs for many registrations.

### Attack Scenario
1. Attacker builds VDF computation cluster
2. Pre-computes VDF proofs for 1000 registrations
3. Submits all registrations rapidly

### Defenses

**Chained VDF Registration**:
- Each VDF must be computed after knowing the previous registration hash
- Cannot pre-compute in parallel
- Computation farm provides no advantage

**Sequential Dependency**:
```
reg_1: prev_hash = ZERO
reg_2: prev_hash = hash(reg_1)  ← must wait for reg_1
reg_3: prev_hash = hash(reg_2)  ← must wait for reg_2
```

### Result
Computational power cannot be parallelized. Attack provides no speedup over honest sequential registration.

---

## Attack Vector 6: Bond Manipulation

### Description
Attacker exploits bond mechanics to minimize capital locked while maximizing control.

### Attack Scenario
1. Register with minimum bond
2. Exit and recover bond
3. Use recovered bond for new registration
4. Attempt to multiply influence

### Defenses

**4-Year Bond Lock**:
- Bond is locked for full commitment period (4 years)
- Cannot recover bond quickly
- Capital efficiency attack is ineffective

**Maturity Cooldown**:
- Exiting resets seniority to 1
- No benefit to exit/re-register cycle
- Optimal strategy is long-term commitment

**Bond Burn on Slashing**:
- Double-production evidence → 100% bond burned
- No partial recovery for misbehavior
- Strong disincentive for malicious actions

### Result
Bond mechanics favor long-term honest participation over short-term manipulation.

---

## Attack Vector 7: Update Voting Manipulation

### Description
Attacker attempts to push malicious software update by manipulating the veto process.

### Attack Scenario
1. Attacker publishes update with hidden malicious code
2. Signs update manifest with maintainer keys
3. Avoids triggering veto from honest producers
4. Update auto-applies after veto period

### Defenses

**7-Day Veto Period**:
- Sufficient time for community review
- Code audits can identify malicious changes
- Any producer can trigger veto investigation

**40% Weighted Veto Threshold**:
- 40% of total *effective* weight needed to reject
- Senior producers have more veto power
- Dormant attackers lose up to 50% weight (activity penalty)
- Single senior producer (weight 4) has significant influence

**Multi-Signature Requirement**:
- Maintainer keys required to sign releases
- Compromising one key insufficient
- Key rotation procedures in place

### Result
Malicious updates require compromising maintainer keys AND avoiding detection for 7 days. Defense in depth makes success unlikely.

---

## Attack Vector 8: Early Active Attacker

### Description
An attacker enters at network genesis with perfect activity, accumulating seniority alongside honest founders. Unlike "late Sybil" attacks, this attacker has maximum time advantage.

### Attack Scenario
1. Attacker registers 10-15 nodes at block 0 (same time as founders)
2. Maintains perfect activity (no gaps, no penalties)
3. Buys DOLI cheaply at launch (~$2/DOLI)
4. After 4 years, attempts to block critical governance upgrades
5. Network growth is slow (benevolent: +5 nodes/year)

### Simulation Results (Early Active Attacker Test)

| Attacker Nodes | DOLI Cost | Years with Veto (40%) | Final % |
|----------------|-----------|----------------------|---------|
| 3              | 3,000     | Never                | 7.6%    |
| 5              | 5,000     | 1 year               | 12.1%   |
| 10             | 10,000    | 3 years              | 21.6%   |
| 15             | 15,000    | 5 years              | 29.2%   |
| 20             | 20,000    | 7 years              | 35.5%   |
| 30             | 30,000    | 9+ years             | 45.2%   |

**Assumptions**: 5 founders, +5 nodes/year benevolent growth, $2/DOLI at launch, $50/month server cost.

### Cost Analysis

For **sustained 4-year governance blocking**:
- Minimum nodes required: **15** (with 40% threshold)
- DOLI cost: 15,000 DOLI × $2 = $30,000
- Server cost: 15 nodes × $50/month × 48 months = $36,000
- **Total attack cost: ~$66,000**

With original 33% threshold, only 10 nodes ($44,000) would suffice.

### Defenses

**40% Veto Threshold** (raised from 33%):
- Requires 50% more weight to block proposals
- 10-node attacker loses veto power 1 year earlier
- Forces attacker to commit more capital upfront

**Activity Gap Penalty**:
- Dormant producers lose 10% weight per week of inactivity
- Maximum 50% penalty for sustained dormancy
- "Register and wait" strategy becomes ineffective
- Attackers must actively maintain nodes or lose influence

**Network Growth Dilution**:
- Each new honest producer dilutes attacker's percentage
- After 4 years with +5 nodes/year, attacker is heavily diluted
- Long-term veto requires disproportionate initial investment

### Result
Attack is feasible at ~$66K but:
- Requires 4-year sustained commitment
- Dilution makes long-term blocking increasingly expensive
- Activity penalty punishes "sleeper" strategies
- 40% threshold (vs 33%) reduces attack window by 1-2 years

**Risk Level**: Medium. Acceptable for a network where $66K sustained attack only provides temporary governance blocking, not theft or protocol corruption.

---

## Attack Vector 9: Low-Weight Fork Attack

### Description
An attacker attempts to create a competing chain by producing many blocks with newly registered (low-weight) producers.

### Attack Scenario
1. Attacker registers multiple new producers (weight 1 each)
2. Attacker builds a competing chain with these low-weight producers
3. Chain has more blocks but less total weight
4. Attacker tries to convince network to accept their chain

### Defenses

**Weight-Based Fork Choice Rule**:
- Chain selection uses accumulated producer weight, not block count
- Chain with higher total weight wins, regardless of length
- Low-weight producers contribute less to chain weight

**Example**:
```
Honest chain: 5 blocks from weight-4 producers = 20 total weight
Attack chain: 10 blocks from weight-1 producers = 10 total weight
→ Honest chain wins (20 > 10) despite fewer blocks
```

**Automatic Equivocation Detection**:
- Network tracks blocks by (producer, slot) pairs
- Double-signing (same producer, same slot, different blocks) is detected automatically
- Equivocation proofs trigger automatic slash transactions
- 100% bond burn for detected equivocation

### Result
Attack requires controlling enough weight to exceed honest chain. Low-weight Sybil nodes cannot outweigh established producers.

---

## Attack Vector 10: Bond Stacking Whale Dominance

### Description
A wealthy attacker attempts to dominate block production by stacking the maximum bonds on a single producer identity.

### Attack Scenario
1. Whale registers with maximum 100 bonds (100,000 DOLI)
2. Attempts to produce majority of blocks
3. With 100 bonds vs 50 honest producers with 1 bond each, whale controls 66%

### Defenses

**Anti-Whale Cap (100 bonds maximum):**
- Single identity cannot exceed 100 bonds
- Forces whale to split across multiple identities
- Each identity requires separate VDF registration

**Chained VDF Registration:**
- Cannot register multiple identities simultaneously
- Sequential registration prevents instant dominance
- 200 identities = 200 registration windows

**Deterministic Round-Robin (NOT Lottery):**
- No variance exploitation possible
- Whale with 100 bonds gets exactly 100/N of slots
- Cannot "get lucky" and win more than proportional share

**Economic Cost Analysis:**
```
Scenario: Whale vs 100 honest producers (1 bond each)

Whale Strategy A: 100 bonds on 1 identity
  - Investment: 100,000 DOLI
  - Block share: 100/200 = 50%
  - No advantage over fair share

Whale Strategy B: 1 bond on 100 identities (requires 100 VDF registrations)
  - Investment: 100,000 DOLI + time cost (100 × registration time)
  - Block share: 100/200 = 50%
  - Same result, much higher effort
```

### Result
The anti-whale cap combined with deterministic allocation ensures:
- No single identity can dominate
- Multi-identity attacks require extensive time investment
- Economic return is always proportional to investment (no exploitation)

---

## Defense Effectiveness Summary

| Attack Vector | Primary Defense | Secondary Defense | Effectiveness |
|--------------|-----------------|-------------------|---------------|
| Whale Instant Takeover | Chained VDF | Weight by Seniority | High |
| Long-Term Sybil | Economic Cost | Distributed Veto | Medium-High |
| Hit-and-Run Voting | Maturity Cooldown | Exit History | High |
| Coordinated Rapid Exit | Chained Registration | Unbonding Period | High |
| VDF Computation Farm | Chained VDF | Sequential Dependency | Very High |
| Bond Manipulation | 4-Year Lock | Maturity Cooldown | High |
| Update Manipulation | Veto Period | Weighted Veto (40%) | High |
| Early Active Attacker | 40% Threshold | Activity Penalty | Medium |
| Low-Weight Fork Attack | Weight-Based Fork Choice | Equivocation Detection | Very High |
| Bond Stacking Whale | Anti-Whale Cap (100) | Deterministic Rotation | High |

---

## Recommendations for Node Operators

1. **Run honest nodes** - Economic incentives favor honesty
2. **Monitor veto proposals** - Participate in governance
3. **Maintain long-term presence** - Seniority increases influence
4. **Report suspicious activity** - Help protect the network
5. **Review software updates** - Use the 7-day veto period

---

## Future Considerations

Potential enhancements being researched:

1. **Veto Bond Requirement** - Require temporary bond (e.g., 10 DOLI) to vote BLOCK
   - Adds friction to frivolous vetoes
   - Bond returned after vote concludes (regardless of outcome)
   - Constant `VETO_BOND_AMOUNT` already defined for future implementation

2. **HOLD/BLOCK Voting** - Distinguish between "need more time" (HOLD) and "reject" (BLOCK)
   - HOLD extends review period without permanent rejection
   - BLOCK requires bond and triggers deeper review
   - Reduces governance gridlock from cautious voters

3. **Reputation Systems** - Track producer behavior over time
4. **Stake Delegation** - Allow passive stake to support trusted producers
5. **Slashing Extensions** - Additional slashable offenses beyond double-production
6. **Governance Evolution** - Adjust parameters based on network maturity

**Note**: Current defenses (40% threshold + activity penalty) are considered sufficient for launch. HOLD/BLOCK + bond is optional enhancement if governance attacks prove more sophisticated than modeled.

---

*This analysis covers known attack vectors as of protocol version 2.9. Security is an ongoing process - new vectors will be analyzed as they emerge.*

*Last updated: January 2026 - Added Early Active Attacker analysis, 40% veto threshold, activity gap penalty, weight-based fork choice rule, automatic equivocation detection.*
