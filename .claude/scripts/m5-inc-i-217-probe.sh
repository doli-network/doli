#!/usr/bin/env bash
# INC-I-217 / run 552 / M5 outcome probe.
#
# Measures three properties of the SHIPPED crates, by executing them. Modeled
# on `.claude/scripts/m4-inc-i-217-probe.sh`: temporary harness test files
# under `crates/core/tests/zz_m5_probe_*.rs`, written, run, and removed by a
# `trap cleanup EXIT`. A harness that fails to compile prints 0 for its
# counter(s) -- that is the correct reading of "the capability is absent",
# NOT a script failure, so this script never aborts on a cargo failure
# (`set -uo pipefail`, no `-e`).
#
#   ROTATION_GATE_NETWORKS_FROZEN (harness A)
#     How many of the three shipped networks expose
#     `NetworkParams::defaults(n).bls_key_rotation_activation_height == u64::MAX`.
#     That field does NOT exist before M5, so harness A does not compile
#     today -> counter 0 (expected). After M5 it should read 3 (all three
#     networks frozen pre-activation).
#
#   ROTATION_VERDICTS_DISTINCT / ROTATION_WELLFORMED_ACCEPTED (harness B)
#     A 10-member `RotateBlsKey` corpus (1 well-formed + 9 malformed
#     variants) is run through the SHIPPED public validator
#     `validate_transaction(&tx, &ctx)` at current_height = 1_000_000.
#     ROTATION_VERDICTS_DISTINCT counts distinct verdict strings (Ok(()) is
#     one distinct verdict; each distinct Err(e) is normalised to its
#     leading "[...]" bracket token when the message has one, else the
#     whole message, before counting distinct values).
#     ROTATION_WELLFORMED_ACCEPTED is 1/0 for whether corpus member #1
#     (the well-formed one) validated Ok.
#
#     Harness B deliberately does NOT call
#     `ValidationContext::with_bls_key_rotation_activation_height` -- that
#     builder does not exist before M5, and calling it would make harness B
#     itself fail to compile today, losing the "before" evidence entirely.
#     Harness B therefore always exercises the context's DEFAULT rotation
#     gate value. Today M4's arm rejects RotateBlsKey unconditionally
#     regardless of height, so the gate value is moot; after M5 ships, the
#     documented default is u64::MAX (fail-closed) with current_height only
#     1_000_000, so harness B keeps measuring "gate left at its default"
#     behavior even after M5 -- it is NOT expected to change across the M5
#     boundary by itself. See harness C for the gate-open measurement.
#
#   ROTATION_VERDICTS_DISTINCT_GATED / ROTATION_WELLFORMED_ACCEPTED_GATED
#   (harness C, informational -- not one of the three named counters, but
#   the forward-compatible measurement of M5's actual effect)
#     Identical corpus and validator call, but the context forces the gate
#     OPEN via `ValidationContext::with_bls_key_rotation_activation_height(0)`
#     at current_height = 1. That builder does not exist before M5, so
#     harness C does not compile today -> both counters 0 (expected). Re-run
#     this same script after M5 ships to get the real "gate open" verdict
#     spread from `rotate_stateless`.
#
# Arrangement note (nested-heredoc bash gotcha): the corpus body is shared
# between harness B and C. It is written ONCE to a temporary .inc.rs file via
# a plain top-level heredoc, then `cat`-appended into both harness files.
# It is NOT captured into a shell variable via `$(cat <<'EOF' ... EOF)`:
# Rust's `&'static` lifetime apostrophe is an unbalanced single-quote that
# bash's command-substitution scanner (which must track quote state to find
# the matching `)`) trips over even inside a quoted heredoc, corrupting the
# whole script's parse. Plain `cat > file <<'EOF'` has no such scanner and is
# unaffected.
#
# None of the temporary files are ever committed; they are written and
# removed by this script alone.
set -uo pipefail

REPO="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel)}"
cd "$REPO" || exit 1

A="crates/core/tests/zz_m5_probe_gate_field.rs"
B="crates/core/tests/zz_m5_probe_verdicts.rs"
C="crates/core/tests/zz_m5_probe_verdicts_gated.rs"
CORPUS_INC="crates/core/tests/zz_m5_probe_corpus.inc.rs.tmp"
cleanup() { rm -f "$A" "$B" "$C" "$CORPUS_INC"; }
trap cleanup EXIT
cleanup

cat > "$A" <<'PROBE_A'
// TEMPORARY outcome probe harness (INC-I-217 M5). Removed by the probe script.
// Does NOT compile before M5: `NetworkParams::bls_key_rotation_activation_height`
// does not exist yet. That is the expected, correct reading of "the
// capability is absent" -- counter prints 0.
use doli_core::network::Network;
use doli_core::network_params::NetworkParams;

#[test]
fn probe_rotation_gate_networks_frozen() {
    let networks = [Network::Mainnet, Network::Testnet, Network::Devnet];
    let frozen = networks
        .iter()
        .filter(|n| NetworkParams::defaults(**n).bls_key_rotation_activation_height == u64::MAX)
        .count();
    println!("PROBE_ROTATION_GATE_NETWORKS_FROZEN={frozen}");
}
PROBE_A

# Shared corpus body (functions only, no ctx() and no #[test]), used by both
# harness B (ungated) and harness C (gated). Kept in one file so the two
# harnesses cannot drift from each other.
cat > "$CORPUS_INC" <<'CORPUS'
use crypto::{bls_sign_pop, sign_rotation_pop, BlsKeyPair, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::genesis::genesis_hash;
use doli_core::network::Network;
use doli_core::transaction::{
    rotation_auth_digest, Input, Output, RotateBlsData, Transaction, TxType,
};
use doli_core::validation::{self, ValidationContext, ValidationError};

/// Build the well-formed corpus member: correct Ed25519 auth signature over
/// `rotation_auth_digest` of the spent outpoint, correct rotation PoP, and
/// `producer` = the Ed25519 key that also signs the input.
fn well_formed() -> Transaction {
    let producer_kp = KeyPair::generate();
    let genesis_bytes = *genesis_hash(Network::Devnet).as_bytes();

    let input = Input {
        public_key: Some(*producer_kp.public_key()),
        ..Input::new(crypto::Hash::from_bytes([0xABu8; 32]), 3)
    };
    let prev_tx_hash_bytes = *input.prev_tx_hash.as_bytes();
    let output_index = input.output_index;

    let bls_kp = BlsKeyPair::from_seed(&[7u8; 32]).expect("bls keypair");
    let new_bls_pubkey = *bls_kp.public_key().as_bytes();
    let producer_ed25519 = *producer_kp.public_key().as_bytes();

    let digest = rotation_auth_digest(
        &genesis_bytes,
        &new_bls_pubkey,
        &prev_tx_hash_bytes,
        output_index,
    );
    let auth_sig = crypto::signature::sign(&digest, producer_kp.private_key());

    let pop = sign_rotation_pop(
        bls_kp.secret_key(),
        bls_kp.public_key(),
        &genesis_bytes,
        &producer_ed25519,
    )
    .expect("rotation pop");

    let data = RotateBlsData {
        producer: producer_ed25519,
        new_bls_pubkey,
        bls_pop: *pop.as_bytes(),
        signature: *auth_sig.as_bytes(),
    };

    Transaction {
        version: 1,
        tx_type: TxType::RotateBlsKey,
        inputs: vec![input],
        outputs: vec![Output::normal(1, crypto::Hash::from_bytes([0x01u8; 32]))],
        extra_data: data.encode().to_vec(),
    }
}

/// The 10-member corpus. Index 0 is always the well-formed member.
fn corpus() -> Vec<(&'static str, Transaction)> {
    let base_tx = well_formed();
    let base_input = base_tx.inputs[0].clone();
    let base_output = base_tx.outputs[0].clone();
    let base_data = RotateBlsData::decode(&base_tx.extra_data).expect("base data decodes");

    let mut v = vec![("well_formed", base_tx.clone())];

    // 2. two inputs instead of one.
    let mut tx = base_tx.clone();
    tx.inputs.push(base_input.clone());
    v.push(("two_inputs", tx));

    // 3. zero inputs.
    let mut tx = base_tx.clone();
    tx.inputs.clear();
    v.push(("zero_inputs", tx));

    // 4. two outputs instead of one.
    let mut tx = base_tx.clone();
    tx.outputs.push(base_output.clone());
    v.push(("two_outputs", tx));

    // 5. extra_data truncated to 239 bytes.
    let mut tx = base_tx.clone();
    tx.extra_data.truncate(239);
    v.push(("truncated_239", tx));

    // 6. extra_data extended to 241 bytes.
    let mut tx = base_tx.clone();
    tx.extra_data.push(0u8);
    v.push(("extended_241", tx));

    // 7. new_bls_pubkey = 48 bytes of 0xFF (not a valid G1 point).
    let mut data7 = base_data.clone();
    data7.new_bls_pubkey = [0xFFu8; 48];
    let mut tx = base_tx.clone();
    tx.extra_data = data7.encode().to_vec();
    v.push(("bad_bls_pubkey", tx));

    // 8. signature = 64 bytes of zero (bad Ed25519 auth signature).
    let mut data8 = base_data.clone();
    data8.signature = [0u8; 64];
    let mut tx = base_tx.clone();
    tx.extra_data = data8.encode().to_vec();
    v.push(("zero_signature", tx));

    // 9. PoP produced with the REGISTRATION DST instead of the rotation DST.
    let bls_kp = BlsKeyPair::from_seed(&[7u8; 32]).expect("bls keypair");
    let wrong_pop =
        bls_sign_pop(bls_kp.secret_key(), bls_kp.public_key()).expect("registration pop");
    let mut data9 = base_data.clone();
    data9.bls_pop = *wrong_pop.as_bytes();
    let mut tx = base_tx.clone();
    tx.extra_data = data9.encode().to_vec();
    v.push(("wrong_dst_pop", tx));

    // 10. producer field set to a DIFFERENT Ed25519 public key than the one
    //     that signed the input.
    let other_kp = KeyPair::generate();
    let mut data10 = base_data.clone();
    data10.producer = *other_kp.public_key().as_bytes();
    let mut tx = base_tx.clone();
    tx.extra_data = data10.encode().to_vec();
    v.push(("mismatched_producer", tx));

    v
}

/// Normalise a verdict to its comparison key: `Ok(())` -> a fixed sentinel;
/// `Err(e)` -> the leading bracket token in the Display string if present
/// (matched generically, so it covers both today's non-hyphenated
/// [ERRTX001]-style codes and M5's hyphenated [ERRTX-ROT001]-style codes),
/// else the whole string.
fn verdict_key(res: &Result<(), ValidationError>) -> String {
    match res {
        Ok(()) => "OK".to_string(),
        Err(e) => {
            let s = e.to_string();
            let open = s.find('[');
            let close = s.find(']');
            match (open, close) {
                (Some(start), Some(end)) if end > start => s[start..=end].to_string(),
                _ => s,
            }
        }
    }
}
CORPUS

# ---- Harness B: ungated (compiles before and after M5) --------------------
cat > "$B" <<'PROBE_B_HEAD'
// TEMPORARY outcome probe harness (INC-I-217 M5). Removed by the probe script.
// Ungated: uses the context's DEFAULT rotation gate (no
// with_bls_key_rotation_activation_height call), so this compiles both
// before and after M5. See the probe script header for why.
PROBE_B_HEAD
cat "$CORPUS_INC" >> "$B"
cat >> "$B" <<'PROBE_B_TAIL'

fn ctx() -> ValidationContext {
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, 1_000_000)
}

#[test]
fn probe_rotation_verdicts_default_gate() {
    let c = ctx();
    let members = corpus();
    let mut keys: Vec<String> = members
        .iter()
        .map(|(_, tx)| verdict_key(&validation::validate_transaction(tx, &c)))
        .collect();
    keys.sort();
    keys.dedup();
    let distinct = keys.len();

    let accepted = i32::from(validation::validate_transaction(&members[0].1, &c).is_ok());

    println!("PROBE_ROTATION_VERDICTS_DISTINCT={distinct}");
    println!("PROBE_ROTATION_WELLFORMED_ACCEPTED={accepted}");
}
PROBE_B_TAIL

# ---- Harness C: gate forced OPEN (compiles only after M5) -----------------
cat > "$C" <<'PROBE_C_HEAD'
// TEMPORARY outcome probe harness (INC-I-217 M5). Removed by the probe script.
// Gated: forces the rotation gate OPEN via
// ValidationContext::with_bls_key_rotation_activation_height(0), a builder
// that does not exist before M5 -- this file does NOT compile today, and
// both counters read 0 (expected: "the capability is absent"). Re-run after
// M5 ships to get the real gate-open verdict spread from rotate_stateless.
PROBE_C_HEAD
cat "$CORPUS_INC" >> "$C"
cat >> "$C" <<'PROBE_C_TAIL'

fn ctx() -> ValidationContext {
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, 1)
        .with_bls_key_rotation_activation_height(0)
}

#[test]
fn probe_rotation_verdicts_gate_open() {
    let c = ctx();
    let members = corpus();
    let mut keys: Vec<String> = members
        .iter()
        .map(|(_, tx)| verdict_key(&validation::validate_transaction(tx, &c)))
        .collect();
    keys.sort();
    keys.dedup();
    let distinct = keys.len();

    let accepted = i32::from(validation::validate_transaction(&members[0].1, &c).is_ok());

    println!("PROBE_ROTATION_VERDICTS_DISTINCT_GATED={distinct}");
    println!("PROBE_ROTATION_WELLFORMED_ACCEPTED_GATED={accepted}");
}
PROBE_C_TAIL

rm -f "$CORPUS_INC"

OUT_A=$(cargo test -q -p doli-core --test zz_m5_probe_gate_field -- --nocapture 2>/dev/null)
GATE_FROZEN=$(printf '%s' "$OUT_A" | sed -n 's/.*PROBE_ROTATION_GATE_NETWORKS_FROZEN=\([0-9]*\).*/\1/p' | head -1)
[ -z "$GATE_FROZEN" ] && GATE_FROZEN=0

OUT_B=$(cargo test -q -p doli-core --test zz_m5_probe_verdicts -- --nocapture 2>/dev/null)
VERDICTS=$(printf '%s' "$OUT_B" | sed -n 's/.*PROBE_ROTATION_VERDICTS_DISTINCT=\([0-9]*\).*/\1/p' | head -1)
WELLFORMED=$(printf '%s' "$OUT_B" | sed -n 's/.*PROBE_ROTATION_WELLFORMED_ACCEPTED=\([0-9]*\).*/\1/p' | head -1)
[ -z "$VERDICTS" ] && VERDICTS=0
[ -z "$WELLFORMED" ] && WELLFORMED=0

OUT_C=$(cargo test -q -p doli-core --test zz_m5_probe_verdicts_gated -- --nocapture 2>/dev/null)
VERDICTS_GATED=$(printf '%s' "$OUT_C" | sed -n 's/.*PROBE_ROTATION_VERDICTS_DISTINCT_GATED=\([0-9]*\).*/\1/p' | head -1)
WELLFORMED_GATED=$(printf '%s' "$OUT_C" | sed -n 's/.*PROBE_ROTATION_WELLFORMED_ACCEPTED_GATED=\([0-9]*\).*/\1/p' | head -1)
[ -z "$VERDICTS_GATED" ] && VERDICTS_GATED=0
[ -z "$WELLFORMED_GATED" ] && WELLFORMED_GATED=0

cleanup
echo "ROTATION_GATE_NETWORKS_FROZEN=${GATE_FROZEN} ROTATION_VERDICTS_DISTINCT=${VERDICTS} ROTATION_WELLFORMED_ACCEPTED=${WELLFORMED} ROTATION_VERDICTS_DISTINCT_GATED=${VERDICTS_GATED} ROTATION_WELLFORMED_ACCEPTED_GATED=${WELLFORMED_GATED}"
