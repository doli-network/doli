# Domain Diagnostic Reasoning Trace — INC-I-146 (proposed), RUN 473

Synthesizer: blockchain-domain-synthesizer · 2026-07-30

## Domain Reports Summary

**Fork** (`domain-investigation-fork.md`): Relevance MINIMAL — domain ruled out. All 4 fork hypotheses killed by runtime evidence (zero FORK_GUARD/WEDGE_ESCAPE/REORG/ROLLBACK events across the entire chain life; byte-identical hashes at h=1/70991/72531 between newest and oldest nodes). Quantified fork-retention worst case <11 MB (≤1 % of step). Unique contributions: the epoch-199 activation timeline (17:45:31Z, memory-neutral — the killer decoupling measurement), the E4 Prometheus table proving the pre-step week was flat, the `pending_proofs` latent (equivocation.rs:95), and the INC-I-143 postmortem-status correction. Gaps: could not attribute the still-ramping host component; rotated seed logs unscanned.

**Connectivity** (`domain-investigation-connectivity.md`): Relevance LOW/MEDIUM-as-trigger-channel — ruled out as causal by double arithmetic (207 MB/peer required vs 8.6 measured vs 1.7 from code). Unique contributions: the paired host-Δ/Σ-RSS closure (0.4 %/3 % residual — the single strongest measurement in the investigation), the systemd timezone conversion (16:31 WEST = 15:31 UTC), the `up{}` proof that 7 nodes were down all week, the SeenCache bound verification by code-read (satisfying the brief's explicit instruction), the glibc-arena smaps analysis, bonds 5→720 discovery, and the n6 SST-mtime provenance gap. Gaps: trigger for the arena residual unproven; 10-min windows cannot settle bounded-vs-leak.

**Parameters** (`domain-investigation-parameters.md`): Relevance HIGH — but as absent limits + measurement defects, not mis-tuning. Unique contributions: the first-ever per-process memory budget (204 MB configured ceiling — proving n6–n12 are 1.7× OVER budget), the "Apps"-expression ai5 series matching the user's numbers to two significant figures (0.48→2.04 = 4.25×), the mislabeled Grafana fleet-sum panel, watchdog-unarmed verification at both source and all-11-units level, CF-count/INC-I-104 regression rule-out, md5 binary-identity sweep, and the smaps_rollup 99.7 %-anonymous measurement. Gaps: cannot distinguish live heap from retained-free arenas; panel identification is inference from arithmetic match.

**Code** (`domain-investigation-code.md`): Relevance LOW for the symptom — but owns the root-cause defect. Unique contributions: journalctl proof of the Jul 22 11:29 stop → Jul 30 start lifecycle, the H7 dead-exporter confirmation (direct scrape + 8-day range query), the structural kill of diagnostic_ledger (merge-base proof — obsoletes INC-I-101/102), the O(1) proof of the lazy state-root memo, the O(N) attestation-loop confirmation (attestation.rs:203-206) with the CPU kill-test showing it is NOT biting, the retained-structures sweep (no unbounded collection with missing eviction), the June-29 dmesg OOM precedent (1.4–2.1 GB anon-RSS on these exact hosts, pre-genesis), and the +20–25 MB/h drift measurement. Gaps: leak-vs-settling unresolved at 4 h; validation.rs not read in full.

## Domain Relevance Analysis

Distribution: MINIMAL / LOW / HIGH / LOW. Under the classification rules this is nominally "one HIGH → single-domain", but the HIGH (parameters) is explicitly NOT a mis-tuned-parameter finding — it is (a) the measurement/aggregation defect and (b) absent protective limits. The causal defect (dead exporter) lives in code. So the correct classification is cross-domain: primary = observability (code exporter + params dashboard facet), presenting = memory/resource. The "all domains would be HIGH" suspicion trigger did not fire — two domains honestly self-ruled-out, which is itself a strong signal the investigation was not over-interpreting.

Anti-anchoring: all four reports were read in full before any conclusion was formed. The first-read report (fork) had the least explanatory content for the symptom; the synthesis root cause comes from the intersection of code (exporter defect) + parameters (panel + budget) + connectivity (closure arithmetic) — no first-report anchor.

## Cross-Domain Causation Analysis

Chain tested: observability defect [E1] → only host-level series visible [chain #2] → (independent input: 7 planned starts [chain #3]) → real host step [chain #4] → misattribution and incident filing [chain #5].

- Counterfactual test: without [E1], a per-node RSS panel would exist and read ≤378 MB flat — the misreading is impossible. PASS (causation).
- Precedence test: gauges zero since genesis (8-day range query) — defect predates event. PASS.
- Mechanism test: zero doli_* series ⇒ Grafana memory panels can only bind node_memory_* or the mislabeled fleet-sum; the latter stepped 398→774 MB purely from target reappearance. PASS.
- Reverse test on the user's candidate (producers 5→12): step 15:17–15:31Z precedes activation 17:45:31Z by 2 h 15 m; activation memory-flat on all hosts; CPU-side input (attestations 120→128) moved at activation where memory did NOT — measured decoupling in both directions. The candidate FAILS precedence, so it cannot be causal.

The genuine technical thread (post-snap residual, defect B) was deliberately kept OUT of the misdiagnosis chain and given its own PRELIMINARY status — merging them would have repeated the misattribution in the other direction (blaming the residual for a step it did not produce) or buried the residual under "premise falsified".

## Convergence Analysis

Eight convergence claims scored (see report matrix): five at 4/4, one at 3/4+consistent, two at 3/4-measured. Independence verified per claim: the step timestamp alone was established four independent ways (UTC app-log line, systemd ActiveEnterTimestamp, journalctl lifecycle events, Prometheus series presence/`up{}`); the arithmetic closure three independent ways (paired MemTotal−MemAvailable+ps; "Apps"-expression prediction; code-lens baseline+N×RSS). No cluster rests on a shared log line or a shared Prometheus expression. Confidence: 0.85 (3+ domain convergence) + 0.1 (confirmed cross-domain chain) → capped presentation at 0.96 given the two open sub-threads (B, D) explicitly carved out as PRELIMINARY/OPEN.

## Contradiction Analysis

Six contradictions identified and resolved (detail in report §Contradictions):
1. Step minute — timezone artifact (WEST/BST = UTC+1) + two process generations; resolved by conn's explicit conversion + UTC app logs. Dissolved fork's open question #5.
2. Byte deltas — two PromQL expressions + sample-time skew + page-cache wiggle; authoritative closure = conn's paired sampling; authoritative user-panel match = params' Apps series.
3. Producer transition epoch 198 vs 199 — snapshot boundary vs frozen-list boundary; compatible; both postdate the step.
4. diagnostic_ledger lead vs removal — resolved by measured git fact (merge-base); memory.db record stale.
5. INC-I-143 status — postmortem §12 supersedes stale DB record.
6. Drift linear-vs-decelerating — both consistent with a decelerating asymptote; joint window too short; explicitly left unclassified with a scheduled discriminator rather than force-resolved.

Evidence-quality ordering (measured > observed > inferred > assumed) was applied in each: e.g., in #4 the code investigator's merge-base proof (measured) beat the brief's DB record (recorded assumption); in #1 UTC-stamped app logs (measured) beat `ps -o lstart` renderings (observed, TZ-ambiguous).

## Confidence Evolution

- After fork report: fork ruled out (high confidence), symptom unexplained, timeline corrected to today, "fleet-wide" already doubtful.
- After connectivity report: symptom-as-described falsified (arithmetic closure), connectivity ruled out, glibc-arena residual surfaced. Working conf on "process placement, not leak" ~0.85.
- After parameters report: user's exact figures reproduced from ai5 Apps series; budget built; watchdog/MemoryMax absence confirmed; residual quantified as budget violation. Conf ~0.9.
- After code report: lifecycle proof, exporter defect confirmed as the misreading's mechanism, all remaining code hypotheses killed structurally, June-29 precedent bounding the risk of dismissing the drift. Conf 0.96 (converged); NOT higher because (a) the reporter's actual panel was inferred, not observed, and (b) defects B/D remain open.
- Shape-recurrence query (memory.db): INC-I-106/107 matched the instrument-untruth shape → RECURS, 3rd occurrence → root FIX at [E1] made mandatory and provided.
- Graphify: not provisioned (checked); the root is an absence-of-callers defect confirmed by runtime measurement, so no structural link required graph confirmation; noted the documented Rust-method blind spot as an additional reason runtime evidence is authoritative here.

## Framing Discipline

Per the synthesis directive: not written up as user error. The reporter observed a real step on a real dashboard and escalated correctly; the "nodes 6 and 12" phrasing was read charitably as the range n6–n12 (which makes the event narrative substantially accurate); each falsified premise is paired with what was actually seen and why that reading was reasonable given defect A. Lead is true-and-reassuring (no leak, no fork, chain healthy, arithmetic closes); defects A/B/C carry full weight as the genuine outcome of the investigation.
