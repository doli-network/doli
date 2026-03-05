#!/usr/bin/env bash
# sign-release.sh — Sign a DOLI release with 3/5 maintainer keys and upload SIGNATURES.json
#
# Usage:
#   ./scripts/sign-release.sh <version>
#
# Example:
#   ./scripts/sign-release.sh 1.1.10
#
# Prerequisites:
#   - GitHub Release with CHECKSUMS.txt must exist (created by CI)
#   - doli CLI binary built at target/release/doli (or in PATH)
#   - Producer key files at ~/.doli/mainnet/keys/producer_{1..5}.json
#   - gh CLI authenticated
#
# What it does:
#   1. Verifies the GitHub Release and CHECKSUMS.txt exist
#   2. Signs with producer keys 1, 2, 3 (3/5 quorum)
#   3. Assembles SIGNATURES.json
#   4. Uploads to the GitHub Release

set -euo pipefail

VERSION="${1:?Usage: $0 <version> (e.g., 1.1.10)}"
VERSION_BARE="${VERSION#v}"  # strip leading 'v' if present
REPO="e-weil/doli"

# --- Locate doli CLI ---
DOLI="${DOLI_CLI:-}"
if [[ -z "$DOLI" ]]; then
    if [[ -x "./target/release/doli" ]]; then
        DOLI="./target/release/doli"
    elif command -v doli &>/dev/null; then
        DOLI="doli"
    else
        echo "ERROR: doli CLI not found. Set DOLI_CLI env var or build with cargo build --release -p doli-cli"
        exit 1
    fi
fi
echo "Using doli CLI: $DOLI"

# --- Key paths (sign with 3 of 5 for quorum) ---
KEY_DIR="${KEY_DIR:-$HOME/.doli/mainnet/keys}"
KEYS=("$KEY_DIR/producer_1.json" "$KEY_DIR/producer_2.json" "$KEY_DIR/producer_3.json")

# Verify keys exist
for key in "${KEYS[@]}"; do
    if [[ ! -f "$key" ]]; then
        echo "ERROR: Key file not found: $key"
        echo "Set KEY_DIR to override (default: ~/.doli/mainnet/keys/)"
        exit 1
    fi
done

# --- Verify GitHub Release exists ---
echo ""
echo "=== Checking GitHub Release v${VERSION_BARE} ==="
if ! gh release view "v${VERSION_BARE}" --repo "$REPO" --json tagName -q .tagName &>/dev/null; then
    echo "ERROR: GitHub Release v${VERSION_BARE} not found."
    echo "Wait for CI to create it, or create manually:"
    echo "  gh release create v${VERSION_BARE} --repo $REPO --title 'v${VERSION_BARE}' --generate-notes"
    exit 1
fi
echo "Release v${VERSION_BARE} found."

# --- Verify CHECKSUMS.txt exists in the release ---
if ! gh release view "v${VERSION_BARE}" --repo "$REPO" --json assets -q '.assets[].name' | grep -q 'CHECKSUMS.txt'; then
    echo "ERROR: CHECKSUMS.txt not found in release v${VERSION_BARE}."
    echo "CI may still be building. Wait for the Release workflow to complete."
    exit 1
fi
echo "CHECKSUMS.txt found in release."

# --- Sign with each key ---
echo ""
echo "=== Signing v${VERSION_BARE} with 3 maintainer keys ==="

SIGNATURES=()
for i in "${!KEYS[@]}"; do
    key="${KEYS[$i]}"
    idx=$((i + 1))
    echo ""
    echo "--- Signing with producer_${idx} ($(basename "$key")) ---"

    # doli release sign outputs JSON to stdout, status to stderr
    sig_json=$("$DOLI" -w "$key" release sign --version "v${VERSION_BARE}" --key "$key" 2>/dev/null)

    if [[ -z "$sig_json" ]]; then
        echo "ERROR: Failed to sign with $key"
        exit 1
    fi

    SIGNATURES+=("$sig_json")
    echo "  Signed successfully."
done

# --- Download CHECKSUMS.txt SHA-256 (for the assembled file) ---
echo ""
echo "=== Downloading CHECKSUMS.txt to compute hash ==="
TMPDIR=$(mktemp -d)
gh release download "v${VERSION_BARE}" --repo "$REPO" --pattern "CHECKSUMS.txt" --dir "$TMPDIR"
CHECKSUMS_SHA256=$(shasum -a 256 "$TMPDIR/CHECKSUMS.txt" | awk '{print $1}')
echo "CHECKSUMS.txt SHA-256: $CHECKSUMS_SHA256"

# --- Assemble SIGNATURES.json ---
echo ""
echo "=== Assembling SIGNATURES.json ==="

# Build the signatures array from individual JSON blocks
SIGS_ARRAY=$(printf '%s\n' "${SIGNATURES[@]}" | jq -s '.')

# Create final SIGNATURES.json
SIGFILE="$TMPDIR/SIGNATURES.json"
jq -n \
    --arg version "$VERSION_BARE" \
    --arg checksums_sha256 "$CHECKSUMS_SHA256" \
    --argjson signatures "$SIGS_ARRAY" \
    '{version: $version, checksums_sha256: $checksums_sha256, signatures: $signatures}' \
    > "$SIGFILE"

echo "Generated: $SIGFILE"
echo ""
cat "$SIGFILE"
echo ""

# --- Validate structure ---
SIG_COUNT=$(jq '.signatures | length' "$SIGFILE")
if [[ "$SIG_COUNT" -lt 3 ]]; then
    echo "ERROR: Only $SIG_COUNT signatures collected, need at least 3."
    exit 1
fi
echo "Signature count: $SIG_COUNT (quorum met)"

# --- Upload to GitHub Release ---
echo ""
echo "=== Uploading SIGNATURES.json to v${VERSION_BARE} ==="

# Remove existing SIGNATURES.json if present (re-signing)
if gh release view "v${VERSION_BARE}" --repo "$REPO" --json assets -q '.assets[].name' | grep -q 'SIGNATURES.json'; then
    echo "Removing existing SIGNATURES.json..."
    gh release delete-asset "v${VERSION_BARE}" SIGNATURES.json --repo "$REPO" --yes 2>/dev/null || true
fi

gh release upload "v${VERSION_BARE}" "$SIGFILE" --repo "$REPO"
echo ""
echo "=== Done! ==="
echo "SIGNATURES.json uploaded to https://github.com/$REPO/releases/tag/v${VERSION_BARE}"
echo ""
echo "Nodes will auto-detect this release and apply after the veto period."

# Cleanup
rm -rf "$TMPDIR"
