# INC-I-236 Run A — Block intake paths and their signature gate

Every path that takes a block or block header from outside the node, the gate that now verifies the producer's
block signature at heights at or above `inc_i_236_block_signature_activation_height` (AH), and the tests that
prove it. Below the AH every path behaves as before, except the two sync-body hygiene rules noted under P4.

"Forged" = the header names a producer but `vdf_proof.pi` is signed by another key (or is garbage). "Unsigned" =
empty `pi`. "Prospective height" = local best height + 1, used only where the block height is not known yet.

Status values: `closed-at-AH` (this run adds the check before any side effect), `already-covered` (the check
existed before this run; a test locks it), `out-of-scope` (reason given).

| Path | Entry | Gate now | Test name(s) | Status |
|---|---|---|---|---|
| P1 Gossip swarm intake (blocks topic) | `gossip/relay_decision.rs` decide, from `service/gossip_intake.rs` | `MessageAcceptance::Reject` when prospective height >= AH and the signature fails: not relayed, no sender credit, no NewBlock event | req_auth_023_p1_gossip_forged_block_rejected, req_auth_023_p1_gossip_unsigned_block_above_ah_rejected, req_auth_023_p1_gossip_garbage_pi_rejected, req_auth_023_p1_gossip_unsigned_at_boundary_rejected, req_auth_023_p1_gossip_forged_rejected_with_staleness_disabled, req_auth_023_p1_gossip_signed_block_accepted, req_auth_023_p1_gossip_unsigned_below_ah_accepted, req_auth_023_p1_gossip_dormant_ah_accepts_forged | closed-at-AH |
| P2 Node gossip pre-filter | `node/network_events.rs` on_new_block_event | Dropped before the seen-hash dedup, rejected-tip insert, slot-sanity warnings and the sync tip-slot / peer notes (`header_auth.rs` gossip_event_authenticated, prospective height) | req_auth_021_p2_on_new_block_event_forged_no_tip_slot, req_auth_021_p2_on_new_block_event_forged_child_of_rejected_tip_not_recorded, req_auth_021_p2_on_new_block_event_valid_applied, req_auth_021_p2_on_new_block_event_unsigned_below_ah_as_today | closed-at-AH (the receive-latency metric still records first; report only) |
| P3 handle_new_block (shared sink for gossip and sync responses) | `node/block_handling.rs` handle_new_block | An unauthenticated block is dropped before the equivocation detector and before classification, so no orphan chase, fork cache, fork weight, wedge retain, reorg or fork recovery | req_auth_021_p3_forged_reorg_candidate_no_side_effect, req_auth_021_p3_unsigned_reorg_candidate_no_side_effect, req_auth_021_p3_forged_orphan_no_cache_no_chase, req_auth_021_p3_forged_worse_sibling_no_wedge_retain, req_auth_021_p3_signed_reorg_candidate_still_cached, req_auth_021_p3_below_ah_unsigned_orphan_cached_as_before | closed-at-AH (before this run only the detector and the stuck-fork signal were gated) |
| P4 Sync headers | `sync/headers.rs` validate_header | Signature at the header's real height | inc_i_236_m5_forged_signature_above_ah_rejected_at_that_header, inc_i_236_m5_empty_signature_above_ah_rejected_at_that_header | already-covered |
| P4 Sync bodies | `sync/bodies.rs` + `sync/body_intake.rs`, `manager/sync_engine/body_auth.rs` | Only bodies whose hash is in the outstanding request accepted (all heights); at h >= AH the body header signature is verified before `pending_blocks` insert; a refused body stays requested | req_auth_016_p4_forged_body_not_pending, req_auth_016_p4_unsigned_body_at_ah_not_pending, req_auth_016_p4_garbage_pi_body_above_ah_not_pending, req_auth_016_p4_genuine_resend_after_forged_accepted, req_auth_016_p4_unrequested_body_not_pending, req_auth_016_p4_body_without_active_request_not_pending, req_auth_016_p4_signed_bodies_above_ah_accepted, req_auth_016_p4_unsigned_bodies_below_ah_accepted | closed-at-AH |
| P4 Single-block response / network fork-walk intake | `sync/fork_recovery.rs` handle_block, `manager/fork_walk.rs` | A parent whose signature fails at prospective height >= AH is not handed to the walk; the walk keeps waiting and times out to another peer | req_auth_016_p8_fork_walk_forged_parent_not_cached, req_auth_016_p8_fork_walk_unsigned_parent_not_cached, req_auth_016_p8_fork_walk_boundary_garbage_parent_not_cached, req_auth_016_p8_fork_walk_signed_parent_advances, req_auth_016_p8_fork_walk_unsigned_parent_below_ah_advances | closed-at-AH |
| P5 Periodic in-order sync apply | `node/periodic.rs` -> apply_block(Light) | validate_block_with_mode verifies before `match mode` | req_auth_018_light_in_order_apply_rejects_forged_and_unsigned_accepts_valid | already-covered |
| P6 apply_block (all callers) | `node/apply_block/mod.rs` | validate_block_for_apply -> validate_block_with_mode | req_auth_018_full_handle_new_block_rejects_forged_and_unsigned_accepts_valid, req_auth_018_light_floor_fallback_handle_new_block_rejects_forged_and_unsigned_accepts_valid, req_auth_018_light_in_order_apply_rejects_forged_and_unsigned_accepts_valid, req_auth_018_replay_new_for_replay_rejects_forged_and_unsigned_accepts_valid, req_auth_018_direct_successor_rejects_forged_and_unsigned_accepts_valid | already-covered |
| P7 execute_reorg | `node/block_handling.rs` execute_reorg | Every incoming fork block verified at its exact height (ancestor height + 1 + i) before any rollback or state write; failure returns Err with no state change | req_auth_020_p7_execute_reorg_forged_fork_no_rollback, req_auth_020_below_ah_unsigned_fork_reorgs_unchanged | closed-at-AH |
| P8 Fork-recovery parent walk (node) | `node/fork_recovery.rs` evaluate_recovered_fork, `node/fork_walk.rs` | Fork blocks verified at exact heights before fork weight or cache insert; a signature failure does not mark the shared branch hash rejected | req_auth_020_p8_recovered_fork_forged_no_weight_no_cache | closed-at-AH |
| P9 try_apply_cached_chain | `node/fork_recovery.rs` | Eligibility, then apply_block(Full); bodies can enter the cache only through P3/P8, which are now gated | req_auth_018_full_handle_new_block_rejects_forged_and_unsigned_accepts_valid, req_auth_020_p8_recovered_fork_forged_no_weight_no_cache | already-covered |
| P10 Wedge escape | `node/wedge_escape.rs` | Sibling verified at its exact height before fork weight, cache and retained-tip writes | req_auth_020_p10_wedge_escape_forged_no_reorg | closed-at-AH |
| P11 Operator forced reorg | `node/force_reorg.rs` | Planned fork blocks verified at exact heights before the eligibility loop and execute_reorg | req_auth_020_p11_force_reorg_forged_no_reorg | closed-at-AH |
| P12 Snap sync anchor + post-snap Light arm | `node/snapshot_install.rs` install_snapshot_prelude (first step of apply_snap_snapshot) | Anchor header must hash to the quorum block hash (all heights) and carry a valid signature at its height; refusal goes through the existing refused path, before any write | req_auth_017_p12_snap_anchor_forged_refused, req_auth_017_p12_snap_anchor_hash_mismatch_refused, req_auth_017_p12_snap_anchor_valid_installed, req_auth_017_p12_snap_anchor_unsigned_below_ah_installed, req_auth_017_post_snap_light_arm_rejects_forged_and_unsigned_accepts_valid | closed-at-AH |
| P13 backfillFromPeer | `rpc/methods/backfill.rs` | verify_block_signature_at before put_block_canonical; a failing block is skipped, nothing written | req_auth_019_p13_backfill_forged_not_stored | closed-at-AH |
| P14 Archive import / archive height backfill | `storage/archiver/import.rs` (bridgeFromArchive, automatic integrity repair, `restore --from`) | verify_block_signature_at before put_block_canonical; force mode cannot replace a stored block with a forged copy | req_auth_019_p14_archive_import_forged_not_imported, req_auth_019_p14_archive_backfill_heights_forged_not_written, req_auth_019_p14_archive_force_backfill_does_not_replace_with_forged, req_auth_019_p14_archive_import_unsigned_history_below_ah_unchanged | closed-at-AH |
| P15 restore --from-rpc | `node/operations/restore.rs` | verify_block_signature_at before put_block_canonical; a failing block is skipped and counted | req_auth_019_p15_restore_from_rpc_forged_not_stored | closed-at-AH |
| P16 recover (Replay) | `node/operations/chain.rs` -> apply_block(Replay) | validate_block_with_mode verifies before `match mode` | req_auth_018_replay_new_for_replay_rejects_forged_and_unsigned_accepts_valid | already-covered |
| P17 Equivocation detector input | `sync/equivocation.rs` check_block, sole caller handle_new_block | Reached only by authenticated headers (P3) | req_auth_022_detector_forged_leaves_no_seen_record, req_auth_022_detector_forged_far_slot_does_not_evict_real_record, req_auth_022_detector_forged_conflict_emits_no_equivocation_warn, req_auth_007_forged_unsigned_after_real_block_yields_no_proof_no_slash | already-covered; slot bounding for validly signed sync responses is out-of-scope (authenticated input, separate issue) |
| P18 Orphan chase / fork-cache drain | handle_new_block orphan arm; `node/fork_cache.rs` drain | Orphan arm gated by P3; drain applies through apply_block | req_auth_021_p3_forged_orphan_no_cache_no_chase, req_auth_021_f76_node_forged_pi_variant_after_genuine_orphan_keeps_genuine | closed-at-AH (orphan arm); drain already-covered |
| P19 Peer scoring | `network/scoring.rs`, gossipsub | Forged blocks get Reject (no first-delivery credit) | req_auth_023_p1_gossip_forged_block_rejected | out-of-scope beyond "no credit": an explicit penalty for senders of forged blocks is a separate change (report only) |
| P20 Other entry points | no submit/import RPC; other gossip topics ignored | n/a | n/a | out-of-scope: no block intake on these entries |
| P21 Dedup / staleness (finding 76) | gossipsub message id on raw bytes; pi-blind hash keys in node caches | Blocks topic: a pi-variant is rejected before any node-side dedup or cache write | req_auth_021_f76_forged_pi_variant_first_genuine_accepted, req_auth_021_f76_node_forged_pi_variant_first_genuine_applied, req_auth_021_f76_node_forged_pi_variant_after_genuine_orphan_keeps_genuine, req_auth_sec_002_unsigned_copy_first_does_not_block_signed_copy | closed-at-AH for blocks; see below |

Binding (REQ-AUTH-022): req_auth_022_binding_every_hashed_field_breaks_signature (a valid signature moved to a
header with any one hashed field changed fails), req_auth_022_doli_sign_preimage_hex_rejected and
req_auth_022_doli_sign_raw_preimage_unreachable (a `doli sign` signature never verifies as a block signature;
the CLI takes text, and the domain's 0xFF byte is not valid UTF-8). Helper: req_auth_019_verify_block_signature_at_*.

## Disposition of open audit findings

### AUDIT-P1-203 (no consensus-authority gate between gossip ingress and reorg admission)
Closed at the AH for forged (wrong-key) and unsigned blocks. Above the AH such a block is rejected at the swarm
(P1), dropped before classification in handle_new_block (P3), refused by every fork door before weight, cache or
rollback (P7, P8, P10, P11), and execute_reorg itself refuses an unsigned or forged fork before the rollback.
Tests: req_auth_023_p1_gossip_forged_block_rejected, req_auth_021_p3_forged_reorg_candidate_no_side_effect,
req_auth_021_p3_unsigned_reorg_candidate_no_side_effect, req_auth_020_p7_execute_reorg_forged_fork_no_rollback,
req_auth_020_p8_recovered_fork_forged_no_weight_no_cache, req_auth_020_p10_wedge_escape_forged_no_reorg,
req_auth_020_p11_force_reorg_forged_no_reorg.

Still open:
- Below the AH, by design: unsigned blocks are valid there, so the gate cannot apply.
- Above the AH, for a block validly signed by a key that is NOT a registered producer: the signature is
  self-consistent, so it passes the new gate. The ReorgCandidate arm of handle_new_block still caches it and
  weighs it with `effective_weight(...).unwrap_or(1)` before execute_reorg, and that arm does not call
  check_producer_eligibility. check_producer_eligibility does reject such a block
  (req_auth_020_unregistered_signed_producer_rejected_by_eligibility), and the fork-walk, wedge-escape,
  cached-chain and forced-reorg doors call it, but the gossip ReorgCandidate arm does not. The remaining part of
  AUDIT-P1-203 is therefore an eligibility gap on that one arm, not a signature gap.

### Finding 76 (dedup keyed on unauthenticated fields)
Blocks topic: closed at the AH. A forged pi-variant (same header hash, wrong pi) is rejected at the swarm before
the node sees it and, if it reaches the node by another path, is dropped before every pi-blind dedup or cache
key (seen-hash set, rejected tips, fork cache, orphan cache). The genuine signed block delivered after it is
still accepted and applied (req_auth_021_f76_forged_pi_variant_first_genuine_accepted,
req_auth_021_f76_node_forged_pi_variant_first_genuine_applied), and a forged variant cannot overwrite a cached
genuine orphan (req_auth_021_f76_node_forged_pi_variant_after_genuine_orphan_keeps_genuine).

Attestations and transactions: open, out of scope for this run.
