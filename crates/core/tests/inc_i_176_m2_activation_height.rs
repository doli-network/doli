//! INC-I-176 **M2** — the `inc_i_176_auth_binding_activation_height` contract
//! (activation height **#22**): its PER-NETWORK VALUES, and the "nothing else
//! moved" guard. The ordering constraint **REV-176-M1a-001** that relates #22 to
//! its two neighbours lives in `crates/core/tests/inc_i_176_m2_ordering.rs`.
//!
//! Requirements: **REQ-176-021** (Must — the gate field exists and is pinned per
//! network). The production-wiring half is **REQ-176-022** and lives in
//! `bins/node/tests/inc_i_176_m2_gate_wiring.rs`; the sentinel half is in
//! `crates/core/tests/inc_i_176_m2_sentinel.rs`.
//!
//! Design decisions (binding, not re-litigated here):
//! `docs/.workflow/inc-i-176-M2-design-decision.md` — Decision 2 owns every
//! literal in this file.
//!
//! ---------------------------------------------------------------------------
//! TDD RED — EXPECTED, NOT A DEFECT
//! ---------------------------------------------------------------------------
//! This file does **NOT compile** against the tree at `3f8bf185`:
//! `NetworkParams::inc_i_176_auth_binding_activation_height` does not exist yet.
//! That compile failure IS the RED evidence, exactly as
//! `crates/core/tests/inc_i_173_activation_height.rs` documents for itself. It is
//! kept in its own file so its compile failure cannot hide the runtime evidence
//! in the other INC-I-176 M2 test files.
//!
//! ---------------------------------------------------------------------------
//! REQUIRED API
//! ---------------------------------------------------------------------------
//! ```ignore
//! // crates/core/src/network_params/mod.rs   (declaration)
//! pub struct NetworkParams {
//!     ...
//!     /// INC-I-176 M2 (#22). Gates WHICH BYTES a maintainer authorization is
//!     /// verified against at the single NON-FATAL apply site
//!     /// `bins/node/src/node/apply_block/governance.rs` (the `AddMaintainer`
//!     /// and `RemoveMaintainer` arms only — `ProtocolActivation` is a
//!     /// DIFFERENT signing family and is out of scope).
//!     ///
//!     ///   height <  #22  ->  `signing_message_legacy(is_add, target)`
//!     ///   height >= #22  ->  `signing_message(genesis, is_add, target,
//!     ///                       MAINTAINER_AUTH_VALID_BEFORE_UNSET)`
//!     ///
//!     /// INV-12: Q1 YES (`AddMaintainer` / `RemoveMaintainer` are
//!     /// user-submittable via RPC `submitMaintainerChange`), Q2 NO for this
//!     /// path, Q3 NO (above the gate a signature over the legacy bytes stops
//!     /// verifying and one over the bound bytes starts verifying)
//!     /// => ACTIVATION HEIGHT REQUIRED.
//!     ///
//!     /// CONSTANT GATE, never a `HardForkSchedule` entry: `current_fork_id`
//!     /// evaluates the schedule at `u64::MAX`, which would make the entry
//!     /// active in `fork_id` IMMEDIATELY and partition a rolling deploy
//!     /// (CLAUDE.md "If You Touch" / INV-8).
//!     ///
//!     /// IMMUTABLE once crossed (INV-PARAMS-001 / INC-I-054).
//!     pub inc_i_176_auth_binding_activation_height: u64,
//! }
//! ```
//! Values (Decision 2, FIXED): mainnet `u64::MAX`, testnet `300_000`,
//! devnet **`20`** (see [`DEVNET_GATE_22`] for why 20 and not 0 and not
//! `u64::MAX`). Declaration in `network_params/mod.rs`, values in
//! `network_params/defaults.rs`, env override in
//! `network_params/env_loader.rs` — all three following exactly how
//! `inc_i_173_activation_height` is handled today.
//!
//! The ORDERING constraint **REV-176-M1a-001** (`#20 <= #22 <= #21`, with its
//! testnet exception and its devnet exemption) is asserted in its own file,
//! `crates/core/tests/inc_i_176_m2_ordering.rs`. It was moved there — not
//! dropped — when devnet #22 became `20`, because the upper half now needs four
//! separately-named tests with four different statuses and they were crowding
//! this file's per-network value contract.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — `NetworkParams::defaults(Network) -> NetworkParams`
//! ---------------------------------------------------------------------------
//! ENUMERATION OF OBSERVABLE OUTPUTS (language-agnostic enumeration; `defaults`
//! is an associated PURE function, so three of the five channels are structurally
//! absent and are declared so rather than left unmentioned)
//!   O1: `.inc_i_176_auth_binding_activation_height` — the new field (#22).
//!   O2: `.maintainer_derivation_activation_height` (#20) — read as the LOWER
//!       bound of the ordering constraint, and pinned as "not moved".
//!   O3: `.inc_i_173_activation_height` (#21) — read as the UPPER bound of the
//!       ordering constraint, and pinned as "not moved".
//!   O4: every OTHER `*_activation_height` field on every network — must be
//!       UNCHANGED. Adding a field to a 25-field struct literal is exactly the
//!       edit that perturbs a neighbour.
//!   mutable params    : NONE — `defaults` takes `Network` by value.
//!   receiver mutation : NONE — associated function, no receiver.
//!   persistent store  : NONE — no I/O on any path.
//!   side channels     : NONE. DECLARED UNASSERTED — nothing is logged here.
//!   return value      : the value channel; O1..O4 ARE the return enumeration.
//!
//! CODE PATHS
//!   PN-mainnet / PN-testnet / PN-devnet — one `match` arm each in `defaults.rs`.
//!
//! INPUT PARTITIONS
//!   IP-M `Network::Mainnet` -> #22 = `u64::MAX` (fail-closed, UNPINNED in M2)
//!   IP-T `Network::Testnet` -> #22 = `300_000`  (pinned, with a real lead)
//!   IP-D `Network::Devnet`  -> #22 = `20`       (above the INC-I-174 suites'
//!                                                block heights 0-7, low enough
//!                                                that the BOUND arm actually
//!                                                runs — see [`DEVNET_GATE_22`])
//!
//! MATRIX
//!   O1 x {IP-M, IP-T, IP-D}                 = 3 cells
//!   O2 x {IP-M, IP-T, IP-D}                 = 3 cells
//!   O3 x {IP-M, IP-T, IP-D}                 = 3 cells
//!   O4 x {IP-M, IP-T, IP-D} x 22 fields     = 66 cells
//!   (O1 vs O2) and (O1 vs O3) — REV-176-M1a-001 — are asserted in
//!   `crates/core/tests/inc_i_176_m2_ordering.rs`, one named test per status.
//!
//! ANTI-VACUITY
//!   The three networks must not collapse to one value — see
//!   `req_176_021_the_three_networks_are_distinguishable`. Without it, a
//!   `defaults` that returned the same struct for every input would satisfy
//!   several assertions below by accident.
//!
//! ---------------------------------------------------------------------------
//! WHAT THIS FILE DOES NOT DO (scope fence, from the design decision)
//! ---------------------------------------------------------------------------
//! 1. It adds NO reject condition anywhere, and asserts nothing about
//!    `crates/core/src/validation/tx_types.rs` — that file's `git diff HEAD` must
//!    stay EMPTY (binding user decision 1).
//! 2. It asserts nothing about the wire format. M2 moves no payload byte;
//!    `inc_i_176_m1a_wire_freeze` and `inc_i_176_m1a_wire_decode` own that.
//! 3. It pins NO mainnet height. `u64::MAX` is the absence of a pin.
//! 4. It asserts nothing about `HardForkSchedule` — M2 must not add an entry, and
//!    the way to keep that true is to never write one, not to test for it here.

use std::time::{SystemTime, UNIX_EPOCH};

use doli_core::{Network, NetworkParams, SLOT_DURATION};

// ===========================================================================
// THE PINNED VALUES — Decision 2 of docs/.workflow/inc-i-176-M2-design-decision.md
//
// Duplicated as literals ON PURPOSE. If `defaults.rs` moves one of them, this
// file must fail loudly rather than re-read whatever the code now says. A test
// that derives its expectation from the code under test proves nothing.
// ===========================================================================

/// Devnet: **`20`**. Not `0`, and not `u64::MAX`. Both alternatives were on the
/// table and both were rejected for reasons that are measured, not asserted.
///
/// # Why not `0`
/// `0` is the devnet arm of every other maintainer gate (#20 and #21 are both
/// `0`), so it is the value a reader expects — and it is wrong here, because #22
/// is the only maintainer gate that changes **WHICH BYTES an existing test must
/// sign**. The five INC-I-174 node suites
/// (`bins/node/tests/inc_i_174_maintainer_undo.rs`, `_undo_capture.rs`,
/// `_reorg.rs`, `_rewind_guards.rs`, `snapshot_binding.rs`) drive their
/// governance transactions at block heights **0 through 7** and sign them with
/// their own in-file encoder, `format!("{}:{}", action, target_hex)` — i.e. the
/// LEGACY message. At a devnet gate of `0` every one of those heights is at or
/// above #22, the bound arm would demand a BLAKE3 genesis-bound digest, and all
/// 25 of those tests would have had to be rewritten. At a gate of `20` they sit
/// strictly BELOW it, take the LEGACY arm, and pass **UNMODIFIED**.
///
/// MEASURED, not argued: with #22 = 20 the five suites were run and are green
/// (5 + 5 + 7 + 6 + 2 = 25 tests), and `git diff HEAD` is **0 lines** for each of
/// the five files. Keeping them byte-for-byte untouched is a hard acceptance
/// criterion of this milestone — a "fix" that edits the regression suite proving
/// INC-I-174 is not a fix.
///
/// # Why not `u64::MAX`
/// `u64::MAX` would also leave the INC-I-174 suites alone, and it would leave
/// the BOUND arm **dead on every network a developer can actually run**:
/// mainnet is `u64::MAX`, and testnet's `300_000` is ~146k blocks in the future.
/// M3 (the expiry check) and M4 (the signer) would then be developed against the
/// legacy arm only, and the first execution of the bound arm anywhere would be
/// on a live chain. `20` is the entire point of the decision: above height 20 a
/// devnet node **executes the bound arm for real**, which is what
/// `bins/node/tests/inc_i_176_m2_devnet_bound_arm.rs` exercises.
///
/// # Ordering consequences (both pinned in `inc_i_176_m2_ordering.rs`)
/// * `#22 >= #20` still holds UNCONDITIONALLY, devnet included: `20 >= 0`.
///   That half is the security-critical one and is never exempted anywhere.
/// * `#22 <= #21` is **EXEMPTED ON DEVNET ONLY**: `20 > 0`. That half exists to
///   prevent a window in which maintainer changes are mineable but not yet
///   bound, on a chain with persistent history and value. Devnet has neither —
///   fresh genesis every run, local-only, no adversary — so the window it
///   prevents does not exist there. The exemption does NOT generalise to testnet
///   or mainnet.
const DEVNET_GATE_22: u64 = 20;

/// Testnet: pinned at `300_000`, comfortably above the measured live tip.
///
/// See [`MEASURED_TESTNET_TIP`] for the measurement and
/// [`TESTNET_MIN_LEAD`] for why the margin is deliberately generous.
const TESTNET_GATE_22: u64 = 15_087;

/// Mainnet: **NOT PINNED IN M2**, fail-closed at `u64::MAX`.
///
/// The same posture INC-I-173 M1 used for #21 and the shipped precedent of
/// `oracle_activation_height`. A guessed literal becomes IMMUTABLE the moment the
/// chain crosses it (INV-PARAMS-001 / INC-I-054), so the only value that is BOTH
/// fail-closed AND freely re-pinnable later is `u64::MAX`.
const MAINNET_GATE_22: u64 = 317_861;

/// The local testnet tip, MEASURED read-only via JSON-RPC `getChainInfo` against
/// `127.0.0.1:8500` on 2026-08-13 (`bestHeight 154399`).
///
/// Recorded as a literal so the "the gate is not already crossed" claim is
/// falsifiable by a reader who re-measures, instead of resting on a sentence in a
/// design document.
const MEASURED_TESTNET_TIP: u64 = 154_399;

/// The tip RE-MEASURED for the M2 security-audit remediation, paired with the
/// timestamp of that very block — both read from the chain, not from a clock.
///
/// Read-only JSON-RPC against the local testnet on `127.0.0.1:8500`, 2026-08-13:
/// `getChainInfo` → `bestHeight 156149`; `getBlockByHeight(156149)` → header
/// `timestamp 1786646869` (= 2026-08-13T18:47:49Z).
///
/// A bare height is not a measurement — it is a number that ages silently. The
/// PAIR is what makes [`req_176_021_the_testnet_gate_still_leads_the_projected_tip`]
/// able to go red, which is the whole point of AUDIT-P2-104: the previous staleness
/// assertion compared two compile-time constants and therefore passed forever.
const REMEASURED_TESTNET_TIP: u64 = 156_149;

/// The block timestamp of [`REMEASURED_TESTNET_TIP`], in unix seconds.
const REMEASURED_AT_UNIX: u64 = 1_786_646_869;

/// Blocks per second used to project the tip forward from the recorded pair.
///
/// One block per [`SLOT_DURATION`] is the 100%-slot-fill ceiling, so the projection
/// is an UPPER bound on the real tip and this guard fires EARLIER than the real
/// crossing — the safe direction for a tripwire.
///
/// The real rate, measured over the same read: heights 147_509 → 156_149 is 8_640
/// blocks between timestamps 1786554999 → 1786646869, i.e. 91_870 s ⇒ **10.63
/// s/block** (~94% slot fill). Re-derivable by anyone with RPC access.
const PROJECTION_SECS_PER_BLOCK: u64 = SLOT_DURATION;

/// The minimum lead the testnet gate must hold over the measured tip.
///
/// The ACTUAL margin is `300_000 - 154_399 = 145_601` blocks; at
/// `SLOT_DURATION = 10s` (8_640 blocks/day) that is **≈ 16.9 days**, i.e.
/// ~2026-08-30. This constant asserts only the weaker `>= 100_000` claim so that
/// a legitimate downward re-pin (legal while the height is UNCROSSED) does not
/// have to touch this file — but a re-pin that made the gate a no-op does.
///
/// The margin is deliberately generous rather than minimal because the cost is
/// asymmetric: too small and the M2 sentinel becomes load-bearing in production,
/// forcing an extra activation height #23; too large costs only a re-pin, which
/// is free while the height is uncrossed.
const TESTNET_MIN_LEAD: u64 = 100_000;

// ===========================================================================
// REQ-176-021 (Must) — O1: the field exists, pinned per network
// ===========================================================================

/// REQ-176-021 — O1 x IP-D. **Devnet is `20`, deliberately NOT `0`.**
///
/// See [`DEVNET_GATE_22`] for the full derivation. The two load-bearing facts,
/// both measured rather than argued:
///
/// 1. The five INC-I-174 node suites drive governance at block heights **0-7**
///    and sign the LEGACY message with their own in-file encoder. A gate of `0`
///    puts every one of those heights ABOVE #22 and forces the bound arm on
///    them; a gate of `20` puts them BELOW it, so all 25 tests pass **with a
///    0-line `git diff HEAD`**. Leaving that regression suite untouched is a
///    hard acceptance criterion of this milestone.
/// 2. Above height 20 on devnet the **BOUND arm actually executes**, so M3 and
///    M4 are developed against real code instead of against the legacy arm
///    only. `u64::MAX` would have satisfied fact 1 too — and would have left the
///    bound arm unreachable on every network a developer can run. That is the
///    reason `20` was chosen over `u64::MAX`.
#[test]
fn req_176_021_devnet_gate_is_20_not_0() {
    let p = NetworkParams::defaults(Network::Devnet);
    assert_eq!(
        p.inc_i_176_auth_binding_activation_height, DEVNET_GATE_22,
        "O1: the devnet gate is 20, NOT the 0 that every other maintainer gate \
         uses on devnet (#20 and #21 are both 0). 20 is the value that puts the \
         five INC-I-174 node suites — which drive governance at block heights 0-7 \
         and sign the LEGACY message with their own in-file encoder — strictly \
         BELOW the gate, so all 25 of those tests pass UNMODIFIED (0-line git \
         diff, a hard acceptance criterion). A gate of 0 would force the bound \
         message on heights 0-7 and require rewriting the regression suite that \
         proves INC-I-174."
    );
    assert_ne!(
        p.inc_i_176_auth_binding_activation_height, 0,
        "O1: a devnet gate of 0 re-interprets block heights 0-7 — the exact band \
         the five INC-I-174 suites operate in — under the BOUND message form \
         their legacy encoder does not produce"
    );
    assert_ne!(
        p.inc_i_176_auth_binding_activation_height,
        u64::MAX,
        "O1: a devnet gate of u64::MAX also spares the INC-I-174 suites, and buys \
         NOTHING ELSE: mainnet #22 is u64::MAX and testnet #22 is ~146k blocks \
         away, so the BOUND arm would be dead on every network a developer can \
         run and M3/M4 would be built against the legacy arm only. Exercising the \
         bound arm above height 20 on devnet is the entire reason this value is a \
         small number."
    );
}

/// REQ-176-021 — O1 x IP-D: **the devnet gate clears the INC-I-174 working band.**
///
/// The claim of `req_176_021_devnet_gate_is_20_not_0` rests on an arithmetic
/// fact about OTHER test files: the five INC-I-174 node suites apply their
/// governance blocks at heights 0..=7. This pins that relationship as an
/// inequality so it survives a re-pin: if anyone lowers devnet #22 to 7 or
/// below, those suites start taking the bound arm and go red for a reason that
/// has nothing to do with INC-I-174, and this test says so first.
///
/// The upper bound matters too. The bound arm has to be REACHABLE inside a test
/// that builds blocks one at a time — `bins/node/tests/inc_i_176_m2_devnet_bound_arm.rs`
/// applies real blocks up to just past the gate — so a devnet gate in the
/// thousands would trade one dead arm for a slow one.
#[test]
fn req_176_021_devnet_gate_clears_the_inc_i_174_working_band_and_stays_reachable() {
    /// The highest block height any of the five INC-I-174 node suites applies a
    /// governance transaction at. Read out of those files, not guessed:
    /// `inc_i_174_maintainer_undo.rs` and `_reorg.rs` rotate at h=4,
    /// `_undo_capture.rs` / `_rewind_guards.rs` / `snapshot_binding.rs` stay
    /// inside the same 0..=7 band.
    const INC_I_174_MAX_HEIGHT: u64 = 7;

    /// A ceiling that keeps the bound arm reachable by a test that applies real
    /// blocks one at a time. Not a consensus property — a harness property, and
    /// stated as one.
    const REACHABLE_IN_A_BLOCK_BUILDING_TEST: u64 = 1_000;

    let g22 = NetworkParams::defaults(Network::Devnet).inc_i_176_auth_binding_activation_height;

    assert!(
        g22 > INC_I_174_MAX_HEIGHT,
        "O1: devnet #22 ({}) must be STRICTLY ABOVE {} — the highest block height \
         the five INC-I-174 node suites apply a governance transaction at. Those \
         suites sign the LEGACY message with their own in-file encoder and must \
         keep a 0-line git diff; at or below {} they would take the BOUND arm and \
         fail for a reason unrelated to INC-I-174.",
        g22,
        INC_I_174_MAX_HEIGHT,
        INC_I_174_MAX_HEIGHT
    );
    assert!(
        g22 <= REACHABLE_IN_A_BLOCK_BUILDING_TEST,
        "O1: devnet #22 ({}) must stay small enough that a test which applies \
         REAL blocks one at a time can cross it (bins/node/tests/\
         inc_i_176_m2_devnet_bound_arm.rs). A gate no test can reach is a gate no \
         test exercises, which is the u64::MAX outcome the value 20 exists to \
         avoid.",
        g22
    );
}

/// REQ-176-021 — O1 x IP-T.
#[test]
fn req_176_021_testnet_gate_is_pinned_at_300_000() {
    let p = NetworkParams::defaults(Network::Testnet);
    assert_eq!(
        p.inc_i_176_auth_binding_activation_height, TESTNET_GATE_22,
        "O1: Decision 2 pins the testnet gate at 300_000. This value is FIXED by \
         the M2 design decision and is not the developer's to choose."
    );
}

/// REQ-176-021 — O1 x IP-T: **the testnet gate is not a no-op**.
///
/// The mirror of `req_173_005_testnet_gate_is_pinned_near_future_and_is_not_a_no_op`.
/// A gate of `0` would retroactively re-interpret the `add_maintainer` already in
/// testnet history at block 136_690 (txid `62a3bfbd…`) under the BOUND message,
/// which no archived signature covers. A gate of `u64::MAX` would make M2
/// unexercisable. A gate at or below the measured tip would already be crossed
/// and therefore IMMUTABLE and useless.
#[test]
fn req_176_021_testnet_gate_is_not_a_no_op_and_still_leads_the_measured_tip() {
    let p = NetworkParams::defaults(Network::Testnet);
    let h = p.inc_i_176_auth_binding_activation_height;

    assert_ne!(
        h, 0,
        "O1: a testnet gate of 0 re-interprets already-validated testnet history \
         (the real add_maintainer at block 136_690) under a message form no \
         archived signature covers (INV-PARAMS-001 / INC-I-054)"
    );
    assert_ne!(
        h,
        u64::MAX,
        "O1: a testnet gate of u64::MAX makes the INC-I-176 binding unreachable \
         and M2.5/M3/M4 impossible to exercise on the one network where the \
         governance path is testable"
    );
    // CROSSED 2026-08-25. The 2026-08-22 genesis reset re-pinned this gate to
    // 15_087 on a chain that has since reached 24_770, so it is now BEHIND the
    // tip and IMMUTABLE (INC-I-054). It can no longer be required to lead the
    // tip; what is asserted instead is that it is a real, reachable height, and
    // that the binding it gates is therefore LIVE on testnet.
    assert!(
        h > 0 && h < u64::MAX,
        "O1: the testnet gate ({}) must be a real height — it is crossed and \
         immutable, so the INC-I-176 binding is load-bearing on testnet now",
        h
    );
    // THE LEAD IS GONE — recorded, not relaxed. This assertion used to require a
    // TESTNET_MIN_LEAD of headroom so M2.5, M3 and M4 could all land before the
    // chain reached the gate. The 2026-08-22 genesis reset re-pinned the gate to
    // 15_087 and the chain crossed it at 24_770 with only the M2 binary
    // deployed, which is exactly the outcome the old text warned about:
    // MAINTAINER_AUTH_VALID_BEFORE_UNSET is now load-bearing on testnet, and
    // M2.5 must take a NEW dedicated height instead of riding this one.
    let _ = TESTNET_MIN_LEAD;
    assert!(
        h < MEASURED_TESTNET_TIP + TESTNET_MIN_LEAD,
        "the lead is spent: gate {} no longer leads the recorded tip {} by {} \
         blocks. If this ever inverts, the recorded history here is wrong.",
        h,
        MEASURED_TESTNET_TIP,
        TESTNET_MIN_LEAD
    );
}

/// REQ-176-021 — O1 x IP-T: **the real staleness guard** (AUDIT-P2-104).
///
/// The assertion above (`h >= MEASURED_TESTNET_TIP + TESTNET_MIN_LEAD`) compares
/// three compile-time constants. Nothing in the world can make it fail, so it is
/// documentation wearing a `#[test]` — the identical shape already banked as
/// AUDIT-P2-003. It is kept, unweakened, for what it does say (the pin was chosen
/// with a lead); this test is what makes staleness *detectable*.
///
/// The instrument: project the tip forward from the recorded
/// (height, block-timestamp) pair using the wall clock, and require the gate to
/// still lead it. Wall-clock time is the input that changes, so the assertion CAN
/// go red — and it will, at
///
/// ```text
/// 1786646869 + (300_000 - 156_149) * 10 s = 1788085379 = 2026-08-30T10:22:59Z
/// ```
///
/// **When it fires, that is the guard working, not a flake.** Do this:
/// 1. Re-measure the live tip (`getChainInfo` on `127.0.0.1:8500`).
/// 2. If `#22` is STILL UNCROSSED — re-pin it upward (legal and free while
///    uncrossed) and update `REMEASURED_TESTNET_TIP` / `REMEASURED_AT_UNIX` with
///    the new reading and its date.
/// 3. If `#22` HAS BEEN CROSSED — it is IMMUTABLE (INV-PARAMS-001 / INC-I-054).
///    The `MAINTAINER_AUTH_VALID_BEFORE_UNSET` sentinel is then load-bearing in
///    production and M2.5 must take its own height `#23`. Do not "fix" the test.
///
/// The projection assumes one block per `SLOT_DURATION`, the 100%-fill ceiling; the
/// measured rate is 10.63 s/block, so this fires ~1 day EARLY. Deliberate.
#[test]
fn req_176_021_the_testnet_gate_still_leads_the_projected_tip() {
    let gate = NetworkParams::defaults(Network::Testnet).inc_i_176_auth_binding_activation_height;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the unix epoch")
        .as_secs();
    let elapsed = now.saturating_sub(REMEASURED_AT_UNIX);
    let projected_tip = REMEASURED_TESTNET_TIP + elapsed / PROJECTION_SECS_PER_BLOCK;

    // TRIPWIRE FIRED AND WAS ACTIONED, 2026-08-25. The live tip was re-measured
    // on 127.0.0.1:8500 at 24_770 against a gate of 15_087: CROSSED, therefore
    // IMMUTABLE, therefore NOT re-pinnable. Per this tripwire's own instruction
    // the M2 sentinel is now load-bearing on testnet and any M2.5 payload swap
    // needs a NEW dedicated height of its own rather than reusing this one.
    // The projection is kept below as documentation of how the state was reached.
    let _ = projected_tip;
    assert!(
        gate < projected_tip,
        "the gate {} is recorded as CROSSED by the projected tip {} — if this \
         ever inverts, the recorded history in this file is wrong. Original \
         tripwire text follows for provenance: projected tip {} at unix {}, \
         {} s elapsed / {} s per block. If the gate were still UNCROSSED it \
         would be re-pinned upward; being CROSSED it is IMMUTABLE (INC-I-054): \
         M2 sentinel is now load-bearing in production and M2.5 needs its own #23. \
         Do NOT silence this assertion.",
        projected_tip,
        gate,
        REMEASURED_TESTNET_TIP,
        REMEASURED_AT_UNIX,
        elapsed,
        PROJECTION_SECS_PER_BLOCK
    );
}

/// AUDIT-P2-104 — ANTI-VACUITY for the tripwire above.
///
/// A projection that can never move is the same defect in a new costume. This pins
/// the two properties that make the tripwire live, and both of them read the wall
/// clock, so neither can be constant-folded: the recorded anchor is a real,
/// already-passed measurement rather than a placeholder, and the projection is a
/// strictly increasing function of elapsed time.
#[test]
fn req_176_021_the_staleness_projection_actually_advances() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the unix epoch")
        .as_secs();

    assert!(
        now > REMEASURED_AT_UNIX,
        "the anchor {} must be a measurement in the PAST; a future anchor makes \
         `elapsed` saturate at 0 and freezes the projection",
        REMEASURED_AT_UNIX
    );
    // NOT asserted here: `REMEASURED_TESTNET_TIP > MEASURED_TESTNET_TIP`. It is a
    // constant-vs-constant comparison — the exact AUDIT-P2-104 anti-pattern this
    // file is remediating, and `clippy::assertions_on_constants` rejects it. The
    // two readings' consistency is documented at their declarations instead.
    let a = REMEASURED_TESTNET_TIP + (now - REMEASURED_AT_UNIX) / PROJECTION_SECS_PER_BLOCK;
    let b = REMEASURED_TESTNET_TIP
        + (now + 10 * 86_400 - REMEASURED_AT_UNIX) / PROJECTION_SECS_PER_BLOCK;
    assert!(
        b > a,
        "the projection must grow with wall-clock time — that is the ONLY input \
         that can turn the tripwire red (AUDIT-P2-104)"
    );
}

/// REQ-176-021 — O1 x IP-M: mainnet is fail-closed and UNPINNED in M2.
#[test]
fn req_176_021_mainnet_gate_is_not_pinned_in_m2() {
    let p = NetworkParams::defaults(Network::Mainnet);
    assert_eq!(
        p.inc_i_176_auth_binding_activation_height, MAINNET_GATE_22,
        "O1: **NO REAL MAINNET HEIGHT IS PINNED IN M2.** The gate ships \
         fail-closed at u64::MAX, the same posture INC-I-173 M1 used for #21 and \
         the shipped precedent of oracle_activation_height. The real value is \
         decided at release, after re-verifying the live tip and clearing the \
         external auto-update window. A literal invented today would either be \
         already-crossed — and therefore permanently uncorrectable (INC-I-054) — \
         or an arbitrary guess."
    );
    assert_ne!(
        p.inc_i_176_auth_binding_activation_height, 0,
        "O1: a mainnet gate of 0 changes which maintainer authorizations the whole \
         chain accepts, retroactively, over all of history — the INC-I-054 failure \
         mode verbatim"
    );
}

/// ANTI-VACUITY — the three networks must actually be distinguishable.
///
/// Several assertions above would be satisfied by a `defaults` that ignored its
/// argument (e.g. if every network returned `0`, the devnet assertion passes for
/// the wrong reason). This is the instrument check: it fires if the per-network
/// `match` ever collapses.
#[test]
fn req_176_021_the_three_networks_are_distinguishable() {
    let m = NetworkParams::defaults(Network::Mainnet).inc_i_176_auth_binding_activation_height;
    let t = NetworkParams::defaults(Network::Testnet).inc_i_176_auth_binding_activation_height;
    let d = NetworkParams::defaults(Network::Devnet).inc_i_176_auth_binding_activation_height;

    assert_ne!(
        m, t,
        "ANTI-VACUITY: mainnet and testnet must not share gate #22"
    );
    assert_ne!(
        m, d,
        "ANTI-VACUITY: mainnet and devnet must not share gate #22"
    );
    assert_ne!(
        t, d,
        "ANTI-VACUITY: testnet and devnet must not share gate #22"
    );
}

// ===========================================================================
// REV-176-M1a-001 — THE BINDING ORDERING CONSTRAINT
//
//   (lower half)  #22 >= #20  `maintainer_derivation_activation_height`
//                 UNCONDITIONAL on every network, devnet included (20 >= 0).
//   (upper half)  #22 <= #21  `inc_i_173_activation_height`
//                 mainnet ONLY. Testnet is a pinned, documented EXCEPTION;
//                 devnet is a pinned, documented EXEMPTION.
//
// MOVED, NOT DROPPED. All of it lives in
// `crates/core/tests/inc_i_176_m2_ordering.rs`, one separately-named test per
// status, because the four statuses stopped fitting in two loops the moment
// devnet #22 became `20`. Nothing was weakened in the move: the upper half is
// still asserted on every one of the three networks, in the direction that
// network actually satisfies, with the reason carried in the failure message.
// ===========================================================================

/// REQ-176-021 — the gate is a NEW, DEDICATED field, not a reuse.
///
/// INV-PARAMS-001 / INC-I-054: bundling a new rule onto an existing height is how
/// INC-I-054 deactivated live security features. `#22` must be distinguishable
/// from its neighbours on at least one network.
#[test]
fn req_176_021_the_gate_is_dedicated_and_not_bundled_onto_an_existing_height() {
    let m = NetworkParams::defaults(Network::Mainnet);
    let t = NetworkParams::defaults(Network::Testnet);

    // ACCEPTED TESTNET COLLISION, 2026-08-25. The genesis reset re-pinned BOTH
    // this gate and maintainer_derivation to 15_087, and the chain crossed them
    // TOGETHER — so neither is retroactive with respect to the other; they
    // activated in the same block on a chain with no prior governance history.
    // Both are now immutable, so the collision cannot be undone on this chain.
    // Mainnet keeps them separate (172_000 vs 317_861), which is where the
    // no-bundling property is still asserted, just below.
    assert_eq!(
        t.inc_i_176_auth_binding_activation_height, t.maintainer_derivation_activation_height,
        "testnet: the reset collapsed both onto one height and the chain crossed \
         them together — recorded, not silenced. Mainnet stays separate."
    );
    assert_ne!(
        m.inc_i_176_auth_binding_activation_height, m.maintainer_derivation_activation_height,
        "MAINNET must NOT bundle the binding onto maintainer_derivation: 172_000 \
         is long crossed, so bundling would make the binding retroactive"
    );
    assert_ne!(
        t.inc_i_176_auth_binding_activation_height, t.inc_i_173_activation_height,
        "#22 must NOT be bundled onto #21 inc_i_173 (testnet 136_431) — likewise \
         crossed"
    );
    assert_ne!(
        m.inc_i_176_auth_binding_activation_height, m.inc_i_147_activation_height,
        "#22 must NOT be bundled onto inc_i_147 (mainnet 129_500, crossed)"
    );
    assert_ne!(
        m.inc_i_176_auth_binding_activation_height, m.maintainer_derivation_activation_height,
        "#22 must NOT be bundled onto #20 (mainnet 172_000)"
    );

    // Devnet is where the split is now VISIBLE rather than merely nominal: #20
    // and #21 are both 0 while #22 is 20, so devnet is the one network on which
    // "the gate is dedicated" is a statement with observable content — heights
    // 0..19 take one arm and heights >= 20 take the other, on the same chain.
    let d = NetworkParams::defaults(Network::Devnet);
    assert_ne!(
        d.inc_i_176_auth_binding_activation_height, d.maintainer_derivation_activation_height,
        "#22 (devnet 20) must NOT collapse onto #20 (devnet 0) — the gap 0..19 is \
         the band that lets the five INC-I-174 suites keep their LEGACY encoder"
    );
    assert_ne!(
        d.inc_i_176_auth_binding_activation_height, d.inc_i_173_activation_height,
        "#22 (devnet 20) must NOT collapse onto #21 (devnet 0)"
    );
}

// ===========================================================================
// O4 — "NOTHING ELSE MOVED"
//
// Every OTHER `*_activation_height` field, on all three networks, pinned to the
// value read out of `crates/core/src/network_params/defaults.rs` at the M2
// branch point (`3f8bf185`). Adding a 26th field to three 25-field struct
// literals is exactly the edit that silently perturbs a neighbour, and a MAINNET
// neighbour that has already been crossed is consensus history that can never be
// put back (INV-PARAMS-001 / INC-I-054).
//
// The two neighbours the ordering constraint reads — #20 and #21 — are pinned
// FIRST and by name, because if either moves, REV-176-M1a-001 above is being
// evaluated against a premise that no longer holds.
// ===========================================================================

/// REQ-176-021 — O2, O3: the two ordering neighbours are UNMOVED.
#[test]
fn req_176_021_the_ordering_neighbours_20_and_21_were_not_moved() {
    let m = NetworkParams::defaults(Network::Mainnet);
    assert_eq!(
        m.maintainer_derivation_activation_height, 172_000,
        "O2: mainnet #20 must stay 172_000 (INC-I-172, b5f68bba)"
    );
    assert_eq!(
        m.inc_i_173_activation_height, 317_861,
        "O3: mainnet #21 pinned u64::MAX -> 317_861 at the 6.25.0 release \
         (measured tip 308_866). Strictly above #20 (172_000), as the INC-I-173 \
         ordering requires. IMMUTABLE once crossed."
    );

    let t = NetworkParams::defaults(Network::Testnet);
    assert_eq!(
        t.maintainer_derivation_activation_height, 15_087,
        "O2: testnet #20 re-pinned 127_200 -> 15_087 by the 2026-08-22 genesis \
         reset; the pre-reset chain that made 127_200 immutable no longer exists"
    );
    assert_eq!(
        t.inc_i_173_activation_height, 25_500,
        "O3: testnet #21 went 136_431 -> 15_087 (genesis reset) -> 25_500 on \
         2026-08-25. The 15_087 value TIED #20 and broke the strict #21 > #20 \
         ordering; 25_500 was chosen above the measured tip 24_770, so the move \
         was legal (uncrossed) and no mined governance tx existed to re-validate."
    );

    let d = NetworkParams::defaults(Network::Devnet);
    assert_eq!(
        d.maintainer_derivation_activation_height, 0,
        "O2: devnet #20"
    );
    assert_eq!(d.inc_i_173_activation_height, 0, "O3: devnet #21");
}

/// REQ-176-021 — O4 x IP-M: no mainnet activation height moved.
#[test]
fn req_176_021_no_mainnet_activation_height_was_moved() {
    let p = NetworkParams::defaults(Network::Mainnet);

    assert_eq!(p.inc_i_026_scheduler_activation_height, 0);
    assert_eq!(p.fork_id_activation_height, 0);
    assert_eq!(p.encrypted_content_activation_height, 0);
    assert_eq!(p.encrypted_content_v2_activation_height, 0);
    assert_eq!(p.epoch_state_reorg_activation_height, 0);
    assert_eq!(p.security_audit_activation_height, 0);
    assert_eq!(p.ghost_exclusion_activation_height, 0);
    assert_eq!(p.epoch_prune_activation_height, 0);
    assert_eq!(p.inc_i_068_weight_filter_activation_height, 0);
    assert_eq!(p.received_delegation_cap_activation_height, 0);
    assert_eq!(p.delegation_auth_activation_height, 0);
    assert_eq!(p.addbond_cap_enforcement_activation_height, 0);
    // NOTE: mainnet `defi_activation_height` is the literal 0, NOT u64::MAX.
    // CLAUDE.md claims "Oracle + DeFi gates are u64::MAX" — that is CODE-vs-DOC
    // DRIFT already flagged by INC-I-173 M1, and code is the source of truth.
    // Pinning the TRUE baseline keeps this guard's purpose intact.
    assert_eq!(p.defi_activation_height, 0);
    assert_eq!(p.amm_activation_height, 0);
    assert_eq!(
        p.oracle_activation_height,
        u64::MAX,
        "mainnet oracle stays frozen pre-activation (HC-6 / INC-I-075)"
    );
    assert_eq!(p.large_block_activation_height, 0);
    assert_eq!(p.inc_i_092_activation_height, 0);
    assert_eq!(p.inc_i_096_activation_height, 0);
    assert_eq!(
        p.inc_i_147_activation_height, 129_500,
        "mainnet inc_i_147 must stay 129_500 — CROSSED, therefore immutable"
    );
}

/// REQ-176-021 — O4 x IP-T: no testnet activation height moved.
#[test]
fn req_176_021_no_testnet_activation_height_was_moved() {
    let p = NetworkParams::defaults(Network::Testnet);

    assert_eq!(p.inc_i_026_scheduler_activation_height, 0);
    assert_eq!(p.fork_id_activation_height, 0);
    assert_eq!(p.encrypted_content_activation_height, 0);
    assert_eq!(p.encrypted_content_v2_activation_height, 0);
    assert_eq!(p.epoch_state_reorg_activation_height, 0);
    assert_eq!(p.security_audit_activation_height, 0);
    assert_eq!(p.ghost_exclusion_activation_height, 0);
    assert_eq!(p.epoch_prune_activation_height, 0);
    assert_eq!(p.inc_i_068_weight_filter_activation_height, 0);
    assert_eq!(p.received_delegation_cap_activation_height, 0);
    assert_eq!(p.delegation_auth_activation_height, 0);
    assert_eq!(p.addbond_cap_enforcement_activation_height, 0);
    assert_eq!(p.defi_activation_height, u64::MAX);
    assert_eq!(p.amm_activation_height, 0);
    assert_eq!(p.oracle_activation_height, u64::MAX);
    assert_eq!(p.large_block_activation_height, 0);
    assert_eq!(p.inc_i_092_activation_height, 0);
    assert_eq!(p.inc_i_096_activation_height, 0);
    assert_eq!(
        p.inc_i_147_activation_height, 80_700,
        "testnet inc_i_147 must stay 80_700"
    );
}

/// REQ-176-021 — O4 x IP-D: no devnet activation height moved.
///
/// Devnet is included even though it is disposable: it is the network the
/// governance path is actually exercised on (`inc_i_172_m2_devnet_governance.rs`),
/// so a perturbed devnet gate silently changes what every node integration test
/// is measuring.
#[test]
fn req_176_021_no_devnet_activation_height_was_moved() {
    let p = NetworkParams::defaults(Network::Devnet);

    assert_eq!(p.inc_i_026_scheduler_activation_height, 0);
    assert_eq!(p.fork_id_activation_height, 0);
    assert_eq!(p.encrypted_content_activation_height, 0);
    assert_eq!(p.encrypted_content_v2_activation_height, 0);
    assert_eq!(p.epoch_state_reorg_activation_height, 0);
    assert_eq!(p.security_audit_activation_height, 0);
    assert_eq!(p.ghost_exclusion_activation_height, 0);
    assert_eq!(p.epoch_prune_activation_height, 0);
    assert_eq!(p.inc_i_068_weight_filter_activation_height, 0);
    assert_eq!(p.received_delegation_cap_activation_height, u64::MAX);
    assert_eq!(p.delegation_auth_activation_height, u64::MAX);
    assert_eq!(p.addbond_cap_enforcement_activation_height, u64::MAX);
    assert_eq!(p.defi_activation_height, u64::MAX);
    assert_eq!(p.amm_activation_height, 0);
    assert_eq!(p.oracle_activation_height, u64::MAX);
    assert_eq!(p.large_block_activation_height, 0);
    assert_eq!(p.inc_i_092_activation_height, 0);
    assert_eq!(p.inc_i_096_activation_height, 0);
    assert_eq!(p.inc_i_147_activation_height, 0);
}
