# Prompt Refinement — INC-I-149 (follow-up challenge, 2026-08-05)

Original:
related to the last 2 o 3 commits, was detected that when a new node its launched the first block created was differente from genesis o something like that. When i interrogate omega telling to him that in mainnet we dont have that problem sugestion something originate that regresion he tell me that maybe was not happening with short blockchain and maybe bc now its long (on testnet) now 88,361 Height the problem its there but i just deploy a new node on mainnet, now on Height 120,221 and the new node syn perfectly. The commit related with the binary 6.24.0 deployed on mainnet is 9647b809ac01cf5ee537c0d2f1e5f20e0ea56686. This makes me wonder, was everything that was done regarding this supposed problem really necessary, since we don't have it on the mainnet? Where did this regression occur? I repeat, we don't have this problem on the mainnet; I just started a node and it synchronized perfectly with snap-sync.

Anchors detected:
- "related to the last 2 o 3 commits ... regression" → REFRAME — detection time is not introduction time. Whether any recent commit INTRODUCED the behavior is a hypothesis to verify by git archeology, not a premise.
- "maybe was not happening with short blockchain and maybe bc now its long" → STRIP — cause anchor inherited from a prior conversation. The recorded INC-I-149 controlled experiment identifies the trigger as a boot condition (--producer flag + empty data dir), not chain length.
- "supposed problem ... we don't have it on the mainnet" → REFRAME — a clean mainnet snap-sync is evidence only about the path that was exercised (a syncing boot on commit 9647b809). It does not exercise the recorded trigger condition; absence of symptom there must be explained, not assumed to refute the defect.

Domain context preserved:
- [terminal] Testnet at height 88,361 when the problem was detected; mainnet at height 120,221 when the new node was deployed and snap-synced cleanly.
- [git] Mainnet binary 6.24.0 corresponds to commit 9647b809ac01cf5ee537c0d2f1e5f20e0ea56686.
- [db] INC-I-149 (status: resolved) — controlled experiment 2026-08-04: same node, same empty data dir, only --producer differs. WITH producer → self-minted fossil block 1; WITHOUT → clean snap sync, tip identical to fleet. Node self-recovered in ~92s; fossil block 1 remains.
- ⚠️ CONSTRAINT: Mainnet is live. Investigation only — no deploys, no code changes, no node restarts from this session.

Refined:
Determine, with git evidence: (1) whether the INC-I-149 bootstrap first-block defect (producer node on empty data dir mints its own block 1 before snap sync) is a regression introduced by any commit AFTER mainnet baseline 9647b809, or a latent defect already present IN that baseline; (2) why the new mainnet node at height 120,221 snap-synced cleanly — i.e., which trigger condition of the recorded root cause was absent in that deployment; (3) whether the INC-I-149 work (mint-guard fix + NetworkEvidence structural fix + regression tests) was necessary in light of the mainnet observation, including whether any of it has actually shipped to mainnet. Produce an assessment only — no fix, no deploy.

Regression context: baseline commit 9647b809 (mainnet 6.24.0, observed clean); suspect range 9647b809..HEAD plus the uncommitted working tree. Git archeology of the `height > 1 &&` exclusion in the bootstrap mint guard is REQUIRED before accepting or rejecting the "regression" framing.
