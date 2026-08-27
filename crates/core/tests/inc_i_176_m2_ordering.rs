//! INC-I-176 **M2** — **REV-176-M1a-001**, the binding ordering constraint on
//! activation height **#22** `inc_i_176_auth_binding_activation_height`, with the
//! testnet EXCEPTION and the devnet EXEMPTION pinned as named tests.
//!
//! Requirements: **REQ-176-021** (Must — the gate field exists and is pinned per
//! network) and **REQ-176-022** (Must — the gate is wired into production). This
//! file owns the RELATIONSHIP between #22 and its two neighbours; the per-network
//! VALUES are owned by `crates/core/tests/inc_i_176_m2_activation_height.rs` and
//! the production wiring by `bins/node/tests/inc_i_176_m2_gate_wiring.rs`.
//!
//! Modelled on `crates/core/tests/inc_i_173_activation_height.rs`, which already
//! asserts `#21 > #20` inside
//! `req_173_005_testnet_gate_is_pinned_near_future_and_is_not_a_no_op`. The three
//! maintainer gates form one chain and it is asserted end to end:
//!
//! ```text
//!   #20 maintainer_derivation_activation_height   — HOW signatures are COUNTED
//!            <=                                     (entry-counting -> distinct-signer)
//!   #22 inc_i_176_auth_binding_activation_height  — WHICH BYTES are signed
//!            <=                                     (legacy -> genesis-bound)
//!   #21 inc_i_173_activation_height               — WHETHER the tx is MINEABLE
//! ```
//!
//! ---------------------------------------------------------------------------
//! THE TWO HALVES HAVE DIFFERENT STATUS — THAT IS THE WHOLE POINT OF THIS FILE
//! ---------------------------------------------------------------------------
//!
//! | half        | mainnet | testnet                    | devnet                  |
//! |-------------|---------|----------------------------|-------------------------|
//! | `#22 >= #20`| HOLDS   | HOLDS                      | HOLDS (`20 >= 0`)       |
//! | `#22 <= #21`| HOLDS   | **EXCEPTION** (unsatisfiable) | **EXEMPTION** (waived) |
//!
//! `#22 >= #20` is **SECURITY-CRITICAL and UNCONDITIONAL**. It is never exempted
//! on any network, for any reason. Its violation re-arms AUDIT-P1-016.
//!
//! `#22 <= #21` is a **SEQUENCING** property, and its two breaks are recorded
//! with different reasons so that neither can be mistaken for an oversight and
//! neither can be cited as precedent for the other:
//!
//! * **testnet — EXCEPTION, arithmetically unsatisfiable.** #21 = `136_431` was
//!   crossed long ago (live tip measured `154_399` on 2026-08-13 against
//!   `127.0.0.1`), and a crossed height is IMMUTABLE (INV-PARAMS-001 /
//!   INC-I-054). No #22 above the tip can be `<= 136_431`. Accepted residual.
//! * **devnet — EXEMPTION, deliberately waived.** `20 > 0` is satisfiable; it is
//!   simply not worth satisfying. The half prevents a window of mineable-but-
//!   unbound authorizations on a chain with persistent history and value. Devnet
//!   has neither: fresh genesis every run, local-only, no adversary. What devnet
//!   buys in exchange is the ONLY place the bound arm actually runs — see
//!   `bins/node/tests/inc_i_176_m2_devnet_bound_arm.rs`.
//!
//! **The exemption does NOT generalise.** It is devnet-only, and
//! [`rev_176_m1a_001_the_upper_half_break_is_confined_to_the_two_documented_networks`]
//! is the lock that says so.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — `NetworkParams::defaults(Network) -> NetworkParams`
//! ---------------------------------------------------------------------------
//! ENUMERATION OF OBSERVABLE OUTPUTS (`defaults` is an associated PURE function,
//! so three of the five channels are structurally absent and are declared so
//! rather than left unmentioned)
//!   O1: `.inc_i_176_auth_binding_activation_height` (#22)
//!   O2: `.maintainer_derivation_activation_height`  (#20) — the LOWER bound
//!   O3: `.inc_i_173_activation_height`              (#21) — the UPPER bound
//!   mutable params    : NONE — `defaults` takes `Network` by value.
//!   receiver mutation : NONE — associated function, no receiver.
//!   persistent store  : NONE — no I/O on any path.
//!   side channels     : NONE. DECLARED UNASSERTED — nothing is logged here.
//!   return value      : the value channel; O1..O3 ARE the return enumeration.
//!
//! CODE PATHS
//!   PN-mainnet / PN-testnet / PN-devnet — one `match` arm each in `defaults.rs`.
//!
//! INPUT PARTITIONS
//!   IP-M mainnet — #20 `172_000`, #22 `u64::MAX`, #21 `u64::MAX`. BOTH halves.
//!   IP-T testnet — #20 `127_200`, #22 `300_000`,  #21 `136_431`. Lower half
//!                  holds; upper half is the unsatisfiable EXCEPTION.
//!   IP-D devnet  — #20 `0`,       #22 `20`,       #21 `0`.       Lower half
//!                  holds; upper half is the waived EXEMPTION.
//!
//! MATRIX
//!   (O1 vs O2) x {IP-M, IP-T, IP-D} = 3 cells  — lower half, all must HOLD
//!   (O1 vs O3) x {IP-M}             = 1 cell   — upper half, must HOLD
//!   (O1 vs O3) x {IP-T}             = 1 cell   — upper half, must BREAK (pinned)
//!   (O1 vs O3) x {IP-D}             = 1 cell   — upper half, must BREAK (pinned)
//!   containment  x {IP-M, IP-T, IP-D} = 3 cells — the break is confined
//!
//! ANTI-VACUITY
//!   Every "must BREAK" cell is asserted with `>` in the direction the network
//!   actually violates, not skipped. A `<=` assertion that was simply omitted
//!   would be indistinguishable from one nobody thought about; an asserted `>`
//!   fires the moment somebody "fixes" it, which on testnet would mean moving a
//!   CROSSED consensus height (INC-I-054) and on devnet would mean re-breaking
//!   the five INC-I-174 suites.
//!
//! ---------------------------------------------------------------------------
//! WHAT THIS FILE DOES NOT DO (scope fence)
//! ---------------------------------------------------------------------------
//! 1. It adds NO reject condition and asserts nothing about
//!    `crates/core/src/validation/tx_types.rs`, whose `git diff HEAD` must stay
//!    EMPTY (binding user decision 1).
//! 2. It asserts nothing about the wire format. M2 moves no payload byte;
//!    `inc_i_176_m1a_wire_freeze` and `inc_i_176_m1a_wire_decode` own that.
//! 3. It pins NO mainnet height. `u64::MAX` is the ABSENCE of a pin.
//! 4. It asserts nothing about `HardForkSchedule` — M2 must not add an entry, and
//!    the way to keep that true is to never write one.

use doli_core::{Network, NetworkParams};

// MEASURED_TESTNET_TIP (154_399, read from 127.0.0.1:8500 on 2026-08-13) was
// REMOVED 2026-08-25: it recorded a tip on the chain the 2026-08-22 genesis
// reset destroyed, and the testnet exception it justified no longer exists —
// testnet now SATISFIES the upper half outright. Noted so the measurement's
// disappearance reads as deliberate rather than dropped.

/// Testnet #21, re-pinned above the measured tip 24_770 on 2026-08-25.
const TESTNET_GATE_21: u64 = 25_500;

// ===========================================================================
// LOWER HALF — #22 >= #20. SECURITY-CRITICAL. UNCONDITIONAL. NO EXEMPTIONS.
// ===========================================================================

/// REV-176-M1a-001, LOWER HALF — **`#22 >= #20` on ALL THREE networks.**
///
/// # This half is never exempted anywhere, and it is the security-critical one.
///
/// `maintainer_derivation_activation_height` (#20) is the gate that replaced the
/// historical ENTRY-COUNTING multisig counter with a DISTINCT-SIGNER counter
/// (`crates/storage/src/producer/set.rs`, INC-I-172 M2 / AUDIT-P0-010). BELOW
/// #20, three signature ENTRIES produced by ONE key clear a 3-of-5 threshold.
///
/// If #22 were pinned BELOW #20 there would be a height band in which the node
/// verifies the NEW, chain-bound, domain-separated message using the OLD,
/// entry-counting verifier — INC-I-176's binding live while AUDIT-P1-016's
/// counting defect is re-armed underneath it. A single key could then mint a
/// permanent maintainer seat against the new message form, and the maintainer set
/// is the auto-updater's binary-install trust root. **The stronger MESSAGE must
/// never arrive before the stronger COUNTER.**
///
/// Per network: mainnet `u64::MAX >= 172_000`; testnet `300_000 >= 127_200`;
/// devnet `20 >= 0`. Devnet included WITHOUT exemption — the devnet waiver
/// applies to the UPPER half only, and `20 >= 0` holds anyway, so there is
/// nothing to waive here even if someone wanted to.
#[test]
fn rev_176_m1a_001_gate_22_is_never_below_the_maintainer_derivation_gate_on_any_network() {
    for network in [Network::Mainnet, Network::Testnet, Network::Devnet] {
        let p = NetworkParams::defaults(network);
        let g22 = p.inc_i_176_auth_binding_activation_height;
        let g20 = p.maintainer_derivation_activation_height;

        assert!(
            g22 >= g20,
            "REV-176-M1a-001 (LOWER HALF) VIOLATED on {:?}: #22 \
             inc_i_176_auth_binding_activation_height = {} is BELOW #20 \
             maintainer_derivation_activation_height = {}.\n\
             \n\
             THIS RE-ARMS THE AUDIT-P1-016 ENTRY-COUNTING VERIFIER. In the band \
             [{}, {}) the node would verify the NEW chain-bound INC-I-176 message \
             with the OLD pre-INC-I-172 counter, which counts signature ENTRIES \
             rather than DISTINCT signers: three entries from ONE key clear a \
             3-of-5 threshold and mint a permanent maintainer seat — the \
             auto-updater's binary-install trust root.\n\
             \n\
             This half is UNCONDITIONAL on every network, devnet included \
             (20 >= 0 holds). The devnet exemption recorded in this file applies \
             to the UPPER half (#22 <= #21) ONLY and may never be extended here.",
            network,
            g22,
            g20,
            g22,
            g20
        );
    }
}

// ===========================================================================
// UPPER HALF — #22 <= #21. Holds on MAINNET only.
// ===========================================================================

/// REV-176-M1a-001, UPPER HALF — **`#22 <= #21` on MAINNET ONLY.**
///
/// `inc_i_173_activation_height` (#21) is the gate that makes `AddMaintainer` /
/// `RemoveMaintainer` MINEABLE at all — the state-only fee exemption, INC-I-173.
/// Pinning #22 at or below #21 means the bound message form is already in force
/// on the first block where such a transaction can be mined, so no authorization
/// is ever mined under the legacy message on that network: there is no window of
/// mineable-but-unbound governance.
///
/// Mainnet satisfies it today as `u64::MAX <= u64::MAX` — both gates are frozen
/// pre-activation, and the pair will be pinned in the correct order at release
/// (M4). That is a weak satisfaction, and it is asserted anyway, because the
/// moment either mainnet gate is given a real number this test is the thing that
/// decides whether the pair was ordered correctly.
///
/// **Testnet and devnet are NOT included here** — deliberately, and each has its
/// own named test below so the break is PINNED rather than silently absent.
#[test]
fn rev_176_m1a_001_gate_22_is_at_or_below_inc_i_173_on_mainnet_only() {
    let p = NetworkParams::defaults(Network::Mainnet);
    let g22 = p.inc_i_176_auth_binding_activation_height;
    let g21 = p.inc_i_173_activation_height;

    assert!(
        g22 <= g21,
        "REV-176-M1a-001 (UPPER HALF) VIOLATED on MAINNET: #22 \
         inc_i_176_auth_binding_activation_height = {} is ABOVE #21 \
         inc_i_173_activation_height = {}.\n\
         \n\
         On mainnet this half is NOT waived and NOT excepted. Ordering #22 after \
         #21 opens a band [{}, {}) in which AddMaintainer/RemoveMaintainer are \
         MINEABLE but still verified against the UNBOUND, domain-tag-less legacy \
         bytes — i.e. INC-I-173 hands governance a live write path before \
         INC-I-176 has bound it to this chain, which is exactly the \
         AUDIT-P0-011 cross-family collision window.\n\
         \n\
         Both mainnet gates are u64::MAX today (u64::MAX <= u64::MAX). If one of \
         them has just been given a real release height, pin the OTHER one at or \
         above it in the same change — do not relax this test.",
        g22,
        g21,
        g21,
        g22
    );
}

/// REV-176-M1a-001, UPPER HALF ON TESTNET — **PINNED EXCEPTION, unsatisfiable.**
///
/// # This test asserts the VIOLATION on purpose. It is not an oversight.
///
/// A future reader must not be able to mistake the absence of `#22 <= #21` on
/// testnet for something nobody noticed. It was noticed, it was measured, and it
/// was accepted — so it is asserted in the direction testnet actually violates,
/// and the day somebody "fixes" it by moving a crossed height, this fires.
///
/// The facts, measured 2026-08-13 against the LOCAL testnet at `127.0.0.1`:
///
/// * testnet #21 = `136_431` and it HAS BEEN CROSSED — tip `154_399`. A crossed
///   height is consensus history and is IMMUTABLE (INV-PARAMS-001 / INC-I-054).
/// * therefore NO #22 that is also above the tip can satisfy `#22 <= 136_431`.
///   The constraint is arithmetically UNSATISFIABLE on testnet.
/// * the only alternative — pinning #22 at or below `136_431` — puts it BELOW the
///   tip, i.e. already crossed and retroactive, which is strictly worse.
///
/// ACCEPTED RESIDUAL, stated plainly: testnet already carries an unbound,
/// domain-unseparated maintainer authorization in its history (the real
/// `add_maintainer` mined at block 136_690, txid `62a3bfbd…`) and will keep
/// carrying it. Accepted because that testnet runs exclusively on `127.0.0.1`
/// and is not reachable from the internet, so the AUDIT-P0-011 cross-family
/// collision has no remote attacker surface there. **Mainnet carries no such
/// residual and this exception is not precedent for one.**
#[test]
fn rev_176_m1a_001_testnet_upper_half_is_a_pinned_unsatisfiable_exception() {
    // RESOLVED 2026-08-25 — the exception this test guarded NO LONGER EXISTS.
    //
    // The 2026-08-22 genesis reset destroyed the chain on which testnet #21
    // (136_431) was crossed, so the arithmetic that made `#22 <= #21`
    // unsatisfiable went with it. #21 was re-pinned to 25_500 (above the
    // measured tip 24_770, therefore a legal move) and #22 sits at 15_087, so
    // testnet now SATISFIES the upper half outright. The accepted residual — the
    // unbound add_maintainer at old block 136_690 — died with that chain too.
    //
    // The assertion is inverted rather than deleted: if testnet ever breaks the
    // upper half again, this fires and the exception must be re-derived from
    // scratch instead of being quietly reinstated.
    let p = NetworkParams::defaults(Network::Testnet);
    let g22 = p.inc_i_176_auth_binding_activation_height;
    let g21 = p.inc_i_173_activation_height;

    assert!(
        g22 <= g21,
        "testnet must now SATISFY the upper half: #22 ({}) <= #21 ({}). The old \
         unsatisfiable exception rested on a chain that the 2026-08-22 genesis \
         reset destroyed; it must not be reinstated without being re-derived.",
        g22,
        g21
    );
    assert_eq!(
        g21, TESTNET_GATE_21,
        "testnet #21 is the re-pinned {} — if it moves again, re-derive this \
         test's premise from the live tip rather than carrying it forward.",
        TESTNET_GATE_21
    );
}

/// REV-176-M1a-001, UPPER HALF ON DEVNET — **PINNED EXEMPTION, waived on purpose.**
///
/// # This test asserts the VIOLATION on purpose. It is not an oversight.
///
/// Devnet is NOT the testnet case. On testnet the half is unsatisfiable; on
/// devnet it is perfectly satisfiable — `#22 = 0` would satisfy it — and it is
/// **deliberately waived**. Recording the two breaks with two different reasons
/// is the point: neither may be cited as precedent for the other, and neither
/// may be cited as precedent for mainnet.
///
/// WHAT THE HALF PROTECTS: a window in which `AddMaintainer` / `RemoveMaintainer`
/// are MINEABLE (above #21) but not yet BOUND to the chain (below #22), so an
/// authorization could be mined under the unbound, domain-tag-less legacy bytes.
/// That is a real hazard on a chain with **persistent history** (the authorization
/// stays in the ledger) and **value** (someone is motivated to forge one).
///
/// WHY DEVNET IS EXEMPT: devnet has neither. It is regenerated from a **fresh
/// genesis on every run**, it is **local-only** on `127.0.0.1`, and it holds no
/// value and faces no adversary. The window the half prevents does not exist
/// there, so paying for it with `#22 = 0` buys nothing — and costs a great deal:
/// at `0` the five INC-I-174 node suites (block heights 0-7, LEGACY encoder)
/// would all have to be rewritten, and the BOUND arm would still never execute
/// anywhere, because mainnet #22 is `u64::MAX` and testnet #22 is ~146k blocks
/// away. At `20` those suites pass unmodified AND devnet becomes the one place
/// the bound arm actually runs.
///
/// **DEVNET ONLY.** Do not generalise this to testnet (which has its own,
/// different, unsatisfiability argument) and never to mainnet, which has
/// persistent history and value and is exactly the chain the half exists for.
#[test]
fn rev_176_m1a_001_devnet_upper_half_is_a_pinned_deliberate_exemption() {
    let p = NetworkParams::defaults(Network::Devnet);
    let g22 = p.inc_i_176_auth_binding_activation_height;
    let g21 = p.inc_i_173_activation_height;

    assert_eq!(
        g21, 0,
        "the exemption is stated against devnet #21 == 0 (INC-I-173's devnet \
         arm). If #21 moved on devnet, re-derive the exemption rather than \
         carrying it forward."
    );
    assert_eq!(
        g22, 20,
        "the exemption is stated against devnet #22 == 20. If #22 moved on \
         devnet, re-derive: the whole argument depends on 20 being above the \
         INC-I-174 suites' 0-7 band and low enough for a block-building test to \
         cross."
    );
    assert!(
        g22 > g21,
        "PINNED EXEMPTION — NOT AN OVERSIGHT, AND NOT THE TESTNET CASE. On devnet, \
         REV-176-M1a-001's `#22 <= #21` is SATISFIABLE ({} <= {} would hold at \
         #22 = 0) and is DELIBERATELY WAIVED. #22 ({}) is above #21 ({}).\n\
         \n\
         WHY THE WAIVER IS SAFE HERE: that half prevents a window of \
         MINEABLE-BUT-UNBOUND maintainer authorizations, which is a hazard only on \
         a chain with PERSISTENT HISTORY and VALUE. Devnet has neither — fresh \
         genesis on every run, local-only on 127.0.0.1, no value, no adversary — \
         so the window it prevents does not exist there.\n\
         \n\
         WHAT THE WAIVER BUYS: at #22 = 0 the five INC-I-174 node suites (block \
         heights 0-7, signing the LEGACY message with their own in-file encoder) \
         would all need rewriting, and the BOUND arm would still never execute on \
         any runnable network — mainnet #22 is u64::MAX and testnet #22 is ~146k \
         blocks out. At 20 those 25 tests pass with a 0-line git diff AND devnet \
         becomes the one place the bound arm actually runs \
         (bins/node/tests/inc_i_176_m2_devnet_bound_arm.rs).\n\
         \n\
         SCOPE: DEVNET ONLY. Not precedent for testnet (whose break is a \
         DIFFERENT, unsatisfiability argument) and never for mainnet, which has \
         the persistent history and value this half exists to protect.",
        g22,
        g21,
        g22,
        g21
    );
}

// ===========================================================================
// CONTAINMENT — the upper-half break must not spread
// ===========================================================================

/// REV-176-M1a-001 — **the upper-half break is confined to testnet and devnet.**
///
/// Two networks break `#22 <= #21`, each for its own recorded reason. This is the
/// lock that keeps the count at two: it enumerates all three networks and
/// requires mainnet to be the one that still SATISFIES the half.
///
/// Without it, "testnet and devnet are documented exceptions" degrades over time
/// into "the upper half is advisory". The whole value of writing an exception
/// down is that the set of exceptions is CLOSED, and a closed set has to be
/// asserted or it is just a sentence in a doc comment.
#[test]
fn rev_176_m1a_001_the_upper_half_break_is_confined_to_the_two_documented_networks() {
    let breaks: Vec<Network> = [Network::Mainnet, Network::Testnet, Network::Devnet]
        .into_iter()
        .filter(|n| {
            let p = NetworkParams::defaults(*n);
            p.inc_i_176_auth_binding_activation_height > p.inc_i_173_activation_height
        })
        .collect();

    assert_eq!(
        breaks,
        vec![Network::Devnet],
        "REV-176-M1a-001 CONTAINMENT: exactly ONE network may break the upper \
         half `#22 <= #21`, and they are testnet (unsatisfiable EXCEPTION: #21 \
         136_431 is crossed, tip 154_399) and devnet (deliberate EXEMPTION: fresh \
         genesis every run, local-only, no persistent value). Observed breaking \
         set: {:?}.\n\
         \n\
         If MAINNET now appears in this list, an activation height was pinned in \
         the wrong order and the fix is to re-pin the pair, not to widen this \
         assertion. Mainnet is the chain with persistent history and value — it \
         is precisely the one the half exists for, and neither recorded reason \
         transfers to it.",
        breaks
    );
}

/// REV-176-M1a-001 — ANTI-VACUITY: the three networks are distinguishable.
///
/// Every assertion above reads two or three fields out of `defaults`. If
/// `defaults` ever ignored its argument, the containment test could return the
/// right-looking set for the wrong reason and the lower-half loop would be
/// asserting one network three times. This is the instrument check.
#[test]
fn rev_176_m1a_001_the_ordering_inputs_are_actually_per_network() {
    let m = NetworkParams::defaults(Network::Mainnet);
    let t = NetworkParams::defaults(Network::Testnet);
    let d = NetworkParams::defaults(Network::Devnet);

    assert_ne!(
        (
            m.inc_i_176_auth_binding_activation_height,
            m.maintainer_derivation_activation_height,
            m.inc_i_173_activation_height
        ),
        (
            t.inc_i_176_auth_binding_activation_height,
            t.maintainer_derivation_activation_height,
            t.inc_i_173_activation_height
        ),
        "ANTI-VACUITY: mainnet and testnet must not present the same (#22,#20,#21) \
         triple, or every per-network claim in this file is one claim repeated"
    );
    assert_ne!(
        (
            t.inc_i_176_auth_binding_activation_height,
            t.maintainer_derivation_activation_height,
            t.inc_i_173_activation_height
        ),
        (
            d.inc_i_176_auth_binding_activation_height,
            d.maintainer_derivation_activation_height,
            d.inc_i_173_activation_height
        ),
        "ANTI-VACUITY: testnet and devnet must not present the same (#22,#20,#21) \
         triple — the EXCEPTION and the EXEMPTION rest on different arithmetic"
    );
    assert_ne!(
        (
            m.inc_i_176_auth_binding_activation_height,
            m.maintainer_derivation_activation_height,
            m.inc_i_173_activation_height
        ),
        (
            d.inc_i_176_auth_binding_activation_height,
            d.maintainer_derivation_activation_height,
            d.inc_i_173_activation_height
        ),
        "ANTI-VACUITY: mainnet and devnet must not present the same (#22,#20,#21) \
         triple"
    );
}
