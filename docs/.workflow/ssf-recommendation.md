# SSF Recommendation

**Problem**: Active producers with delegated bonds show 0% attestation and never qualify for rewards.

**Root cause**: `startup.rs:548` uses `weight == 0` as a proxy for "not active", but producers who delegated all bonds are active with weight=0.

**Single recommendation**: Replace the `if weight == 0 { return; }` check with an explicit `is_active()` check. Move the activity check into the producer_set read block where ProducerInfo is available, remove the weight==0 early return. This lets weight=0 active producers attest (correctly contributing 0 to finality but recording presence for qualification).

**Deploy**: Rolling deploy safe — no consensus rule change, no block content structure change.
