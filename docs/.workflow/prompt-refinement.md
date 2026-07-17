# Prompt Refinement — omega-new-feature: disk-full graceful-halt watchdog

Original: implement number 2 feature

Anchors detected:
- "pause production + threshold error" (from prior discussion) → REFRAME — prescribed HOW; the WHAT is "node self-protects on low disk instead of ABRT core-dumping." Architect chooses mechanism.
- (no layer/depth anchors in the literal prompt)

Domain context preserved:
- [incident] nano (external mainnet producer) ABRT core-dumped in a crash-loop
- [root cause] disk 100% full (38G) — 29G unrotated log filled it
- [failure mode] ENOSPC -> write fails -> abort(signal 6); risk of mid-write state corruption; systemd crash-loop; core dump worsens disk pressure
- [goal] detect low free disk -> halt production + emit clear structured error; recover cleanly when space returns
- [constraint] must NOT be a consensus-rule change — voluntary non-production already = existing missed-slot behavior. No activation height.
- [constraint] must not itself write large data or worsen disk use
- [beneficiary] unmonitored external community producers (the /producers fleet)
- [git] DOLI Rust workspace; production path in bins/node/src/node/production.rs

Refined: Add a node-level self-protection so that when free disk space runs low, doli-node fails gracefully — stops producing blocks and surfaces a clear, structured error/log — instead of crashing (ABRT/ENOSPC) and risking state corruption. Recover automatically when space is reclaimed. Feature directive: Analyze how the feature integrates with existing architecture before implementing. The description defines WHAT, not HOW.
