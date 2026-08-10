# ARCHITECT-SESSION PROMPT — INC-I-172: on-chain rotatable trust root for maintainer / update-signing keys

> Paste everything below the line into a fresh session, then invoke the **architect** agent.
> It is self-contained. Your job is an **independent architecture evaluation** — you are
> expected to re-verify every claim against the code and to generate and attack your own
> alternatives. A proposed direction appears at the very end, clearly fenced; treat it as ONE
> hypothesis to challenge, not as the answer. Reject it if the evidence points elsewhere.

---

## YOUR TASK
DOLI is a Proof-of-Stake blockchain (Rust). Design how DOLI should be able to **rotate its
software-update signing keys (the "maintainer" keys) after a key compromise, without a genesis
reset and without a fleet-wide emergency binary redeploy** — i.e. make the update trust root
recoverable by on-chain action the way bond/producer identities already are.

Produce an architecture evaluation: candidate designs, failure modes of each, a recommendation
with confidence, a forward-only activation plan, and the open risks. Do **not** implement.

## ANTI-BIAS INSTRUCTIONS (read first)
1. **Re-verify from code.** Every factual claim in this prompt cites a file. Open each and
   confirm it before you rely on it. Treat un-cited statements as unverified.
2. **Generate your own alternatives** before reading the fenced proposal at the end. Aim for at
   least 3 structurally different designs.
3. **Attack every option**, including the fenced proposal. State how each fails.
4. **The fenced proposal is one engineer's view.** You may adopt, modify, or discard it. Say why.
5. If a claim here turns out to be wrong when you check the code, **report the correction** — that
   itself is a valuable output.

## BACKGROUND — DOLI trust model (verify against code)
- Consensus state = ChainState + UtxoSet + ProducerSet, converged across nodes via a state root.
- **Producers**: registered identities with bonds; weight drives block production + finality
  (`FINALITY_THRESHOLD_PCT = 67`, `crates/core/src/finality.rs`).
- **Maintainers**: a 3-of-5 quorum that signs software releases for the auto-update system
  (`REQUIRED_SIGNATURES = 3`, `crates/updater/src/constants.rs`).
- On mainnet, **N1–N5 are BOTH producers AND maintainers** (dual role — see the comment on
  `BOOTSTRAP_MAINTAINER_KEYS_MAINNET`, `crates/updater/src/constants.rs`).

## THE PROBLEM (verify each cited fact)
Two related weaknesses make any maintainer-key compromise (a public leak — INC-I-170 — OR a
future server hack that steals the N1–N5 private keys) into a fleet-wide emergency:

1. **The update-signing trust root is a compile-time constant, not on-chain state.**
   - `bins/node/src/run.rs:461` — the node builds `maintainer_keys_fn = move || Vec::new()`
     (always empty; intentional — see the comment at `run.rs:456-459`).
   - `bins/node/src/updater/service.rs:221-222` — passes that empty list to
     `verify_release_signatures_with_keys`.
   - `crates/updater/src/verification.rs:66-78` — an empty key list falls back to
     `bootstrap_maintainer_keys(network)`, the hardcoded constants.
   - ⇒ Release signatures are verified **only** against the hardcoded keys. Rotating them needs a
     new binary on every node.

2. **An on-chain maintainer-governance system exists and is enforced — but is not wired to
   release verification.**
   - `bins/node/src/node/apply_block/governance.rs:17-77` — `AddMaintainer`/`RemoveMaintainer`
     verify the 3-of-5 multisig (`verify_multisig` / `verify_multisig_excluding`) and reject
     insufficient signatures. Applied immediately, persisted to the data dir
     (`MaintainerState`), **not** part of the state root.
   - `crates/core/src/maintainer.rs:55-64` — `MaintainerSet.members: Vec<PublicKey>` (raw
     Ed25519); the set is "derived deterministically by replaying the blockchain" (first 5
     producers + Add/Remove txs).
   - This on-chain set is consumed only for `ProtocolActivation` verification
     (`governance.rs:80-104`) + self-governance + RPC — **not** for auto-update release
     verification.
   - The `run.rs:457-459` comment gives the stated reason the updater is left unwired:
     on-chain `ProducerInfo` stores BLAKE3 pubkey hashes, not raw Ed25519 keys, so the
     producer-derived bootstrap path can't supply verifiable keys — with a `TODO` to store raw
     keys on-chain. (Note: `AddMaintainer` targets already carry raw `PublicKey`s — verify whether
     that already makes raw keys recoverable from block history.)

3. **Role conflation.** Because N1–N5 hold one key used for both producing and signing releases, a
   single server compromise yields both bond/finality power and software-signing power.

### Impact to design against
An attacker with 3+ maintainer keys can sign a malicious release that auto-update nodes accept and
install (remote code execution across the fleet), gated only by a 7-day, **seniority-weighted**
veto (`VETO_THRESHOLD_PERCENT = 40`; `crates/updater/src/vote.rs`, `apply.rs`). The compromised
keys are the most senior producers, so the veto's reliability against exactly these keys is
**uncertain and UNVERIFIED** — tracing the veto math (who can veto, whether attacker seniority
blocks an honest veto) is part of your task.

## HARD CONSTRAINTS (non-negotiable — from `CLAUDE.md`)
- **#0 RULE — NO GENESIS RESET** for a feature/storage change. Features activate **forward-only**
  at a future height (BIP9/BIP8 style). Never change the state root of existing blocks. Before
  proposing anything, answer: "does this need a genesis reset, or can it activate at a future
  height?" It must be the latter.
- **Consensus-rule change → activation height required.** **Block-content change → synchronized
  deploy.** State both answers for any consensus-visible part of your design (INV-12 three-question
  checklist; INC-I-062 / INV-8).
- **Do not break late-upgrading or not-yet-synced nodes.** ~20–30 external producers run
  auto-update and cannot be stopped in unison.
- **Encoder/decoder index parity** and **`CURRENT_PROTOCOL_VERSION` / `EPOCH_STATE_FORMAT_VERSION`
  discipline** if you touch serialized formats (see `CLAUDE.md` "If You Touch").
- This is remediation architecture; assume the current maintainer keys may already be compromised
  when you design the rotation authorization and the first-delivery path.

## KEY OPEN QUESTIONS TO ANSWER
1. What should the update-signing trust root be, such that a compromised key can be revoked by an
   on-chain action (or another mechanism that needs no fleet redeploy)?
2. Authorization: a rotation must be signed by the current quorum, which may be the compromised
   keys — how do you prevent the attacker from using the same power (front-running, adding their
   own key)? Is a producer-weighted or social-recovery override warranted?
3. First-delivery bootstrap: any node-side change still ships as a binary; for auto-update nodes
   that binary arrives through the channel the attacker controls. How is the very first
   trust-root-fixing binary delivered safely?
4. Should producer and maintainer roles be separated (distinct keys, maintainer keys offline/cold,
   hardware/threshold signing)? What does that cost operationally?
5. Is the on-chain `MaintainerSet` sufficient as the source of truth, or is a new
   transaction/field/format needed — and if so, what is its forward-only activation plan?
6. Interaction with the veto: should the veto threshold or weighting change so a compromised
   senior quorum cannot both push a malicious update and block the honest veto?

## DELIVERABLE
- ≥3 candidate designs, each with an explicit failure-mode analysis and a `RESOURCE COST` note.
- A recommendation with a confidence level and the reasoning trace.
- A forward-only activation/rollout plan (what activates at a future height vs what is a pure node
  binary change vs what is operational), explicitly confirming NO genesis reset.
- The first-delivery bootstrap plan for the trust-root-fixing binary.
- A short section on what you re-verified and any corrections to this prompt's claims.

## START BY
- Read the code files cited above. Read `.claude/skills/producer-retirement/SKILL.md` for the
  bond-side remediation pattern (transaction-only, per-operator re-key) that this problem is the
  update-side analogue of. Read incidents `INC-I-172` (this issue), `INC-I-170` (the key
  exposure), `INC-I-171` (unenforced vesting penalty) in `.omega/memory.db`.
- Confirm or correct the cited facts, then produce the evaluation.

---
---

## APPENDIX — ONE PROPOSED DIRECTION (this is a single engineer's view; CHALLENGE IT, do not anchor)

> Presented only after you have formed your own options. Adopt, modify, or discard with reasons.

The bond side of a key compromise is already solved transaction-only (retire + per-operator re-key,
no consensus change, no reset — see the `producer-retirement` skill). The claim is that the update
side can be brought to the same footing **forward-only, no genesis reset**, in layers:

1. **Wire the updater to the on-chain maintainer set.** Change `maintainer_keys_fn` (`run.rs:461`)
   to return the raw keys of the on-chain `MaintainerSet` instead of `Vec::new()`. Release
   verification then trusts the on-chain set; the hardcoded constants become only the genesis seed.
   This is a **node binary change, off-chain of consensus** — no activation height, no reset. Its
   only hard part is first-delivery (below). Precondition to verify: that raw Ed25519 maintainer
   keys are recoverable from block history (via `AddMaintainer` targets and/or a new field).
2. **Rotate via the existing enforced governance.** `AddMaintainer` new offline keys, then
   `RemoveMaintainer` the compromised ones — transactions already validated 3-of-5 at consensus.
   Forward-only, no reset.
3. **Separate roles + cold maintainer keys.** Stop using the producer key as the maintainer key;
   keep maintainer keys offline (hardware/threshold). Then a hot-node hack yields producer power
   only, not software-signing power.
4. **Harden the override.** Because step 2 is authorized by the current (possibly compromised)
   quorum, consider a producer-weight-gated or time-locked override so honest producers can rotate
   even if the quorum is hostile — and revisit the seniority-weighted veto so a compromised senior
   quorum cannot both push and un-veto.

**Known weak points of this direction (attack these):** (a) first-delivery of the step-1 binary
still rides the compromised update channel; (b) step 2's authorization is the compromised quorum
(race); (c) storing/recovering raw keys on-chain may need a new forward-activated tx/field, not
just wiring; (d) the veto may not protect against the most-senior keys. Whether these sink the
approach or are manageable is exactly what the evaluation must decide.
