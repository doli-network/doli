#!/usr/bin/env bash
# publish_release.sh — Combine maintainer signatures and upload release.json
#
# Usage:
#   ./scripts/publish_release.sh <version> <sig1.json> <sig2.json> <sig3.json> [sig4.json] [sig5.json]
#
# Prerequisites:
#   - gh CLI authenticated (gh auth login)
#   - GitHub Actions release workflow completed for <version>
#   - At least 3 signature files from: doli-node release sign --key <key> --version <version>
#
# What it does:
#   1. Validates inputs (version tag, minimum 3 signatures)
#   2. Downloads CHECKSUMS.txt from the GitHub release
#   3. Extracts the canonical hash (linux-x64-musl)
#   4. Fetches release notes from GitHub
#   5. Combines signatures into release.json
#   6. Uploads release.json to the GitHub release

set -euo pipefail

REPO="e-weil/doli"
BINARY_NAME="doli-node"

# --- Argument parsing ---

if [ $# -lt 4 ]; then
    echo "Usage: $0 <version> <sig1.json> <sig2.json> <sig3.json> [sig4.json] [sig5.json]"
    echo ""
    echo "Example:"
    echo "  $0 v0.2.0 sig_producer1.json sig_producer2.json sig_producer3.json"
    echo ""
    echo "Generate signature files with:"
    echo "  doli-node release sign --key ~/.doli/mainnet/keys/producer_1.json --version v0.2.0 > sig_producer1.json"
    exit 1
fi

VERSION="$1"
shift
SIG_FILES=("$@")

# Ensure version starts with 'v'
if [[ ! "$VERSION" =~ ^v ]]; then
    echo "Error: Version must start with 'v' (e.g., v0.2.0)"
    exit 1
fi

# Strip 'v' for the version string used inside release.json
VERSION_STR="${VERSION#v}"

# Validate minimum signature count
if [ ${#SIG_FILES[@]} -lt 3 ]; then
    echo "Error: At least 3 signature files required (got ${#SIG_FILES[@]})"
    exit 1
fi

# Validate all signature files exist and are valid JSON
echo "Validating ${#SIG_FILES[@]} signature files..."
for sig_file in "${SIG_FILES[@]}"; do
    if [ ! -f "$sig_file" ]; then
        echo "Error: Signature file not found: $sig_file"
        exit 1
    fi
    if ! jq -e '.public_key and .signature' "$sig_file" > /dev/null 2>&1; then
        echo "Error: Invalid signature format in $sig_file (expected {\"public_key\":\"...\",\"signature\":\"...\"})"
        exit 1
    fi
    pubkey=$(jq -r '.public_key' "$sig_file")
    echo "  OK: ${pubkey:0:16}...${pubkey: -8} ($sig_file)"
done

# --- Wait for release to be published ---

echo ""
echo "Checking GitHub release $VERSION..."

# Check if release exists
if ! gh release view "$VERSION" --repo "$REPO" > /dev/null 2>&1; then
    echo "Release $VERSION not found. Waiting for GitHub Actions to complete..."
    echo "(Press Ctrl+C to cancel)"

    for i in $(seq 1 60); do
        sleep 10
        if gh release view "$VERSION" --repo "$REPO" > /dev/null 2>&1; then
            echo "Release $VERSION found!"
            break
        fi
        echo "  Still waiting... (${i}/60)"
        if [ "$i" -eq 60 ]; then
            echo "Error: Timed out waiting for release $VERSION (10 minutes)"
            exit 1
        fi
    done
fi

# --- Download CHECKSUMS.txt ---

echo ""
echo "Downloading CHECKSUMS.txt..."
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

gh release download "$VERSION" --repo "$REPO" --pattern "CHECKSUMS.txt" --dir "$TMPDIR"

if [ ! -f "$TMPDIR/CHECKSUMS.txt" ]; then
    echo "Error: CHECKSUMS.txt not found in release assets"
    exit 1
fi

echo "CHECKSUMS.txt contents:"
cat "$TMPDIR/CHECKSUMS.txt"
echo ""

# Extract the canonical hash (linux-x64-musl)
BINARY_SHA256=$(grep "x86_64-unknown-linux-musl" "$TMPDIR/CHECKSUMS.txt" | awk '{print $1}')

if [ -z "$BINARY_SHA256" ]; then
    echo "Error: Could not find linux-x64-musl hash in CHECKSUMS.txt"
    echo "Available entries:"
    cat "$TMPDIR/CHECKSUMS.txt"
    exit 1
fi

echo "Canonical binary hash (linux-x64-musl): $BINARY_SHA256"

# --- Fetch release notes ---

echo ""
echo "Fetching release notes..."
CHANGELOG=$(gh release view "$VERSION" --repo "$REPO" --json body -q '.body' 2>/dev/null || echo "Release $VERSION")

# --- Build release.json ---

echo ""
echo "Building release.json..."

# Combine signatures into a JSON array
SIGS_JSON="["
first=true
for sig_file in "${SIG_FILES[@]}"; do
    if [ "$first" = true ]; then
        first=false
    else
        SIGS_JSON+=","
    fi
    SIGS_JSON+=$(cat "$sig_file")
done
SIGS_JSON+="]"

# Get current Unix timestamp
PUBLISHED_AT=$(date +%s)

# Build the release.json
RELEASE_JSON=$(jq -n \
    --arg version "$VERSION_STR" \
    --arg sha256 "$BINARY_SHA256" \
    --arg url_template "https://github.com/$REPO/releases/download/$VERSION/$BINARY_NAME-$VERSION-{platform}.tar.gz" \
    --arg changelog "$CHANGELOG" \
    --argjson published_at "$PUBLISHED_AT" \
    --argjson signatures "$SIGS_JSON" \
    '{
        version: $version,
        binary_sha256: $sha256,
        binary_url_template: $url_template,
        changelog: $changelog,
        published_at: $published_at,
        signatures: $signatures
    }')

echo "$RELEASE_JSON" > "$TMPDIR/release.json"
echo "Generated release.json:"
echo "$RELEASE_JSON" | jq .

# --- Upload release.json ---

echo ""
echo "Uploading release.json to GitHub release $VERSION..."
gh release upload "$VERSION" "$TMPDIR/release.json" --repo "$REPO" --clobber

echo ""
echo "Done! release.json uploaded to $VERSION"
echo ""
echo "Producers will auto-detect the new version via:"
echo "  doli-node update check"
echo ""
echo "Or apply immediately:"
echo "  doli-node update apply"
