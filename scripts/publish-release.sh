#!/usr/bin/env bash
# publish-release.sh — Promote a DRAFT DOLI release to public Latest, but ONLY after the
# maintainer signatures on it verify (INC-I-202, REQ-202-004/REQ-202-005).
#
# Usage:
#   ./scripts/publish-release.sh <version>        # 6.26.3 or v6.26.3
#
# Prerequisites:
#   - CI created the release as a DRAFT (.github/workflows/release.yml, draft: true)
#   - scripts/sign-release.sh already uploaded SIGNATURES.json
#   - gh CLI authenticated, jq installed, doli CLI built or on PATH
#
# What it does:
#   1. Downloads SIGNATURES.json + CHECKSUMS.txt from the draft release
#   2. Refuses on a missing, malformed, or sub-threshold manifest, naming the count
#   3. Runs `doli release verify` against this host's maintainer trust root
#   4. Only on success: gh release edit <tag> --draft=false --latest
#
# Any failure leaves the release a draft: unreachable by nodes and by `doli upgrade`.
#
# 5. Only on success it also strips the CI unsigned-draft banner from the release
#    notes before promoting (REQ-202-007); a failed verification never touches notes.

set -euo pipefail

REPO="${REPO:-doli-network/doli}"
THRESHOLD="${THRESHOLD:-3}"

# Byte-exact markers: MUST match .github/workflows/release.yml (writer side).
BANNER_BEGIN='<!-- DOLI-UNSIGNED-DRAFT-WARNING:BEGIN -->'
BANNER_END='<!-- DOLI-UNSIGNED-DRAFT-WARNING:END -->'

if [[ $# -lt 1 ]]; then
    echo "ERROR: missing <version> argument — nothing was promoted." >&2
    echo "Usage: $0 <version>   (e.g. $0 6.26.3, or $0 v6.26.3)" >&2
    exit 2
fi

VERSION_BARE="${1#v}"
TAG="v${VERSION_BARE}"

# --- Locate doli CLI (same resolution order as sign-release.sh) ---
DOLI="${DOLI_CLI:-}"
if [[ -z "$DOLI" ]]; then
    if [[ -x "./target/release/doli" ]]; then
        DOLI="./target/release/doli"
    elif command -v doli >/dev/null 2>&1; then
        DOLI="doli"
    else
        echo "ERROR: doli CLI not found. Set DOLI_CLI, or build with cargo build --release -p doli-cli" >&2
        exit 1
    fi
fi

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

echo "=== Downloading release assets for $TAG ==="
if ! gh release download "$TAG" --repo "$REPO" --dir "$WORKDIR" \
        --pattern 'SIGNATURES.json' --pattern 'CHECKSUMS.txt' --clobber; then
    echo "WARNING: gh release download did not return every asset for $TAG." >&2
fi

MANIFEST="$WORKDIR/SIGNATURES.json"
CHECKSUMS="$WORKDIR/CHECKSUMS.txt"

if [[ ! -f "$MANIFEST" ]]; then
    echo "REFUSING to promote $TAG: SIGNATURES.json is absent from the release." >&2
    echo "  An unsigned release is not a verified release. Run scripts/sign-release.sh $VERSION_BARE first." >&2
    exit 1
fi

if [[ ! -f "$CHECKSUMS" ]]; then
    echo "REFUSING to promote $TAG: CHECKSUMS.txt is absent from the release." >&2
    echo "  The signatures cover that file; without it nothing can be verified." >&2
    exit 1
fi

if ! jq -e . "$MANIFEST" >/dev/null 2>&1; then
    echo "REFUSING to promote $TAG: SIGNATURES.json is not valid JSON." >&2
    exit 1
fi

SIG_COUNT="$(jq '.signatures | length' "$MANIFEST" 2>/dev/null)" || SIG_COUNT=0
[[ "$SIG_COUNT" =~ ^[0-9]+$ ]] || SIG_COUNT=0

if (( SIG_COUNT < THRESHOLD )); then
    echo "REFUSING to promote $TAG: SIGNATURES.json carries ${SIG_COUNT}/${THRESHOLD} maintainer signature(s)." >&2
    echo "  This is the INC-I-202 shape: a scaffold manifest that authorises nothing." >&2
    exit 1
fi

echo "=== Verifying $TAG against the maintainer trust root ==="
# The draft is invisible to the unauthenticated GitHub API, so verify the bytes just
# downloaded rather than letting the CLI fetch them again.
if ! "$DOLI" release verify --version "$TAG" --dir "$WORKDIR"; then
    echo "REFUSING to promote $TAG: 'doli release verify' failed. The release stays a DRAFT." >&2
    exit 1
fi

# Read the draft's current notes, best-effort: a failed lookup or an empty body
# just means there is no banner to strip, never a reason to abort a verified promotion.
DRAFT_NOTES=""
if RAW_NOTES="$(gh release view "$TAG" --repo "$REPO" --json body --jq '.body' 2>/dev/null)"; then
    DRAFT_NOTES="$RAW_NOTES"
fi

echo "=== Promoting $TAG to public Latest ==="
if [[ -n "$DRAFT_NOTES" ]] && grep -qF -- "$BANNER_BEGIN" <<<"$DRAFT_NOTES"; then
    STRIPPED_NOTES="$WORKDIR/notes-stripped.md"
    # Fixed-string line match (no BEGIN/END regex escaping needed for `<!--`/`-->`).
    awk -v b="$BANNER_BEGIN" -v e="$BANNER_END" '
        { if (index($0, b) > 0) skip=1
          if (skip == 0) print
          if (index($0, e) > 0) skip=0 }
    ' <<<"$DRAFT_NOTES" > "$STRIPPED_NOTES"
    gh release edit "$TAG" --repo "$REPO" --notes-file "$STRIPPED_NOTES" --draft=false --latest
else
    gh release edit "$TAG" --repo "$REPO" --draft=false --latest
fi
echo "Promoted $TAG: ${SIG_COUNT} maintainer signature(s) verified before publication."
