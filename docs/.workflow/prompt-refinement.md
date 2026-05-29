# Prompt Refinement — omega-redesign (INC-I-096)

Original: `/omega-redesign --scope="AMM value-conservation" --incident=INC-I-096`

Anchors detected:
  - (none in natural language — input is structured flags only)
  - `--scope="AMM value-conservation"` → KEEP — intentional user scope constraint (Rule 6), do NOT broaden
  - `--incident=INC-I-096` → KEEP — evidence loader, not an anchor

Domain context preserved:
  - [incident] INC-I-096 — critical, status=investigating, domain=defi-amm-consensus-balance
  - [escalation] Came from omega-doctor as a "Hydra pattern": 5 confirmed findings in the AMM value-conservation layer, all sharing ONE root pattern — declared pool state (new_reserve_a/b, new_total_lp) is NOT bound to actual transaction inputs. RC-B (INC-I-092) applied this binding to CreatePool ONLY.
  - [symptom] MPTX008 / InsufficientFunds rejects legitimate RemoveLiquidity (native conservation blind to Pool reserve release; Pool UTXO amount=0, reserves in extra_data).
  - [defect cluster] D1 conservation blind to reserve flows (mempool pool.rs:383, consensus utxo.rs:210-217); D2 mempool/consensus input-counting parity divergence (pool.rs:892,916 counts LPShare as native); D3 RemoveLiquidity has no proportional withdrawal binding; D4 Swap B→A no exact output binding; SEC-LOGIC-001 shares_burned derived from attacker-controlled new_total_lp; SEC-LOGIC-002 Swap B→A new_reserve_b unbound to token inputs.
  - [current state] Liveness fix implemented but gated inert (inc_i_096_activation_height = u64::MAX mainnet/testnet, 0 devnet). Build/clippy/fmt green; 9 consensus + 25 mempool + 35 AMM tests pass. T10 drain reproduction test ignored pending backing.
  - [mainnet] amm_activation_height = u64::MAX — NO live AMM, NO production impact yet. Redesign can land before activation.
  - [mode] Proposal-only — `--fix` NOT passed.

Regression context: (not applicable — feature was never live; this is a pre-activation design integrity problem, not a deployed regression)

Refined: Redesign the AMM value-conservation layer (INC-I-096 scope). The problem is structural, not a single bug: across mempool admission, consensus validation, and apply_block, the AMM value-conservation model treats Pool UTXOs as ordinary native-amount UTXOs, and the validation pipeline trusts attacker-declared pool state (new_reserve_a/b, new_total_lp) instead of binding it to actual consumed inputs. Successive code-level patches (INC-I-092 RC-A signature exemption, RC-B CreatePool input-backing) have each unmasked a sibling vulnerability — a Hydra pattern indicating the conservation/authorization model itself is the wrong shape. Map the full problem space across all AMM tx types (CreatePool, AddLiquidity, RemoveLiquidity, Swap A→B and B→A) and all three enforcement sites. Propose a unified value-conservation architecture where pool reserve flows and LP supply changes are conserved and bound to inputs by construction, not by per-type ad-hoc checks.

Redesign directive: Map the full problem space before proposing a new design. The architectural weakness may extend beyond the described module — examine the Pool UTXO accounting model, the is_native_amount conservation equation, and the per-tx-type validation invariants as one system.
