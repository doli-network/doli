//! Release signature verification and veto calculation

use crypto::{PublicKey, Signature as CryptoSignature};
use doli_core::network::Network;
use tracing::{debug, error, info, warn};

use crate::constants::VETO_THRESHOLD_PERCENT;
use crate::trust_root::TrustRoot;
use crate::types::{MaintainerSignature, Release, Result, UpdateError, VoteResult};

// ============================================================================
// Release Signing
// ============================================================================

/// Sign a release hash with a maintainer's private key
///
/// Signs the message `"version:sha256"` which matches the format verified by
/// `verify_release_with_trust_root()`. Returns a `MaintainerSignature`
/// containing the public key and hex-encoded signature.
///
/// # Usage
///
/// ```ignore
/// let keypair = crypto::KeyPair::from_private_key(private_key);
/// let sig = sign_release_hash(&keypair, "0.2.0", "abcdef1234...");
/// println!("{}", serde_json::to_string(&sig).unwrap());
/// ```
pub fn sign_release_hash(
    keypair: &crypto::KeyPair,
    version: &str,
    binary_sha256: &str,
) -> MaintainerSignature {
    let message = format!("{}:{}", version, binary_sha256);
    let signature = crypto::signature::sign(message.as_bytes(), keypair.private_key());
    MaintainerSignature {
        public_key: keypair.public_key().to_hex(),
        signature: signature.to_hex(),
    }
}

// ============================================================================
// Signature Verification
// ============================================================================

/// Verify release signatures against the compile-time bootstrap root.
///
/// Compatibility shim for callers with no on-chain state — the CLI and the
/// `doli-node update verify` command. It resolves `TrustRoot::bootstrap(network)`
/// and delegates; it holds no verification logic of its own.
///
/// On success returns the number of DISTINCT signers found, so an operator-facing
/// caller can report what was actually verified instead of restating the threshold.
pub fn verify_release_signatures(release: &Release, network: Network) -> Result<usize> {
    verify_release_with_trust_root(release, &TrustRoot::bootstrap(network))
}

/// Verify that a release carries enough DISTINCT maintainer signatures to satisfy
/// the supplied trust root.
///
/// Fail-closed (INC-I-172 F1): an unusable root — empty, sub-threshold, or with a
/// threshold of zero — is refused with `UpdateError::TrustRootUnavailable`. There is
/// no fallback to `bootstrap_maintainer_keys`; only `TrustRoot::bootstrap` may reach
/// those, and only a caller that has never had on-chain state may ask for it.
///
/// Distinct-signer counting (F3): the loop is the covenant k-of-n shape that has been
/// mainnet-live since covenant activation at h=9150
/// (`crates/core/src/conditions/eval.rs`) — outer loop over the ROOT's keys, inner
/// loop over the release's signature entries, `break` on the first valid entry for
/// that key. Three signature entries produced by ONE key therefore count as ONE
/// signer, so a single stolen key can no longer clear a 3-of-5 gate.
///
/// On success returns the DISTINCT-signer count that satisfied the root. Callers that
/// print a result must print THIS number: printing `REQUIRED_SIGNATURES` instead tells
/// an operator with 5 valid signatures that only 3 were found (QA OBS-001).
pub fn verify_release_with_trust_root(release: &Release, root: &TrustRoot) -> Result<usize> {
    if !root.is_usable() {
        error!(
            "Refusing to verify release {}: {} trust root has {} key(s) for a threshold of {} \
             — an absent, empty or sub-threshold root authorises nothing and does NOT fall back \
             to the compiled bootstrap keys (INC-I-172 F1)",
            release.version,
            root.provenance(),
            root.keys().len(),
            root.threshold()
        );
        return Err(UpdateError::TrustRootUnavailable {
            provenance: root.provenance().to_string(),
            keys: root.keys().len(),
            threshold: root.threshold(),
        });
    }

    let message = format!("{}:{}", release.version, release.binary_sha256);
    let message_bytes = message.as_bytes();

    let mut valid_count = 0usize;
    for expected_key in root.keys() {
        for sig in &release.signatures {
            // Case-INSENSITIVE (F10). The root's keys are lowercase (`PublicKey::to_hex`,
            // and the compiled arrays), while `sig.public_key` is free-form JSON text
            // from SIGNATURES.json. An exact `String` comparison silently drops an
            // uppercase-hex entry and reports `InsufficientSignatures`, which tells the
            // operator "wrong key" when the truth is "wrong case". Hex has no case
            // semantics, so this narrows nothing: the bytes still have to match, and
            // the Ed25519 check below is still the thing that authorises.
            if !sig.public_key.eq_ignore_ascii_case(expected_key) {
                continue;
            }
            if signature_is_valid(sig, message_bytes) {
                valid_count += 1;
                debug!(
                    "Valid signature from maintainer: {}...",
                    &sig.public_key[..sig.public_key.len().min(16)]
                );
                break;
            }
        }
    }

    if valid_count >= root.threshold() {
        info!(
            "Release {} verified: {}/{} distinct maintainer signatures (trust root: {}, {} keys)",
            release.version,
            valid_count,
            root.threshold(),
            root.provenance(),
            root.keys().len()
        );
        Ok(valid_count)
    } else {
        Err(UpdateError::InsufficientSignatures {
            found: valid_count,
            required: root.threshold(),
        })
    }
}

// ============================================================================
// Veto Calculation
// ============================================================================

/// Calculate veto result.
///
/// FAIL-CLOSED on `total_producers == 0` (INC-I-172 M1, AUDIT-P1-015). The denominator
/// used to be fed by a `try_read().unwrap_or(0)` on the producer set, so ordinary lock
/// contention produced a zero here; `checked_div` then returned `None`, the percentage
/// was reported as 0, and `0 < 40` APPROVED the update. Any number of veto votes became
/// "0% — APPROVED", with no attacker action required.
///
/// A zero producer count is not "nobody vetoed". It is "the size of the electorate is
/// unknown", and a veto percentage cannot be computed from it at all. The caller should
/// prefer to take no decision at all on an unknown count — see
/// `UpdateService::check_veto_status`, which now defers the transition — but this
/// function is public and must not hand out an approval on its own.
pub fn calculate_veto_result(veto_count: usize, total_producers: usize) -> VoteResult {
    if total_producers == 0 {
        return VoteResult {
            total_producers,
            veto_count,
            // 0 is the only honest rendering of "not computable"; `approved` carries the
            // decision, and it is NO.
            veto_percent: 0,
            approved: false,
        };
    }

    let veto_percent = ((veto_count * 100) / total_producers) as u8;
    let approved = veto_percent < VETO_THRESHOLD_PERCENT;

    VoteResult {
        total_producers,
        veto_count,
        veto_percent,
        approved,
    }
}

// ============================================================================
// Helpers (private)
// ============================================================================

/// Decode and check one signature entry against `message`.
///
/// A malformed hex public key or signature is a non-match, not an error: the outer
/// loop simply keeps looking for a usable entry from the same expected key.
fn signature_is_valid(sig: &MaintainerSignature, message: &[u8]) -> bool {
    let pubkey_bytes = match hex::decode(&sig.public_key) {
        Ok(bytes) => bytes,
        Err(_) => {
            warn!("Invalid hex in public key: {}", sig.public_key);
            return false;
        }
    };
    let sig_bytes = match hex::decode(&sig.signature) {
        Ok(bytes) => bytes,
        Err(_) => {
            warn!("Invalid hex in signature for key {}", sig.public_key);
            return false;
        }
    };
    let ok = verify_ed25519(&pubkey_bytes, message, &sig_bytes);
    if !ok {
        // CHAR-safe truncation (AUDIT-P3-010). `len().min(16)` is still a BYTE index and
        // panics when byte 16 lands inside a multi-byte UTF-8 sequence — and this string
        // comes straight out of the origin's SIGNATURES.json.
        warn!(
            "Invalid signature from maintainer: {}...",
            sig.public_key.chars().take(16).collect::<String>()
        );
    }
    ok
}

/// Verify Ed25519 signature using doli-crypto
fn verify_ed25519(pubkey_bytes: &[u8], message: &[u8], sig_bytes: &[u8]) -> bool {
    use crypto::signature::verify;

    // Parse public key
    let pubkey = match PublicKey::try_from_slice(pubkey_bytes) {
        Ok(pk) => pk,
        Err(_) => return false,
    };

    // Parse signature
    let signature = match CryptoSignature::try_from_slice(sig_bytes) {
        Ok(sig) => sig,
        Err(_) => return false,
    };

    // Verify
    verify(message, &signature, &pubkey).is_ok()
}
