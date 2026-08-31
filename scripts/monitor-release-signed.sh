#!/usr/bin/env bash
# monitor-release-signed.sh — read-only health check: is the newest v* release
# PUBLISHED and does it verify against the maintainer trust root? (INC-I-202
# M2.5, REQ-202-008)
#
# Usage:
#   ./scripts/monitor-release-signed.sh        # no positional arguments
#
# Env:
#   DOLI_CLI  — path to the doli CLI (default: ./target/release/doli, else PATH)
#   REPO_DIR  — repo whose tags to read (default: this script's own repo root)
#   REPO      — GitHub repo slug (default: doli-network/doli)
#
# What it does:
#   1. Finds the newest v* tag in REPO_DIR, version-sorted (never lexicographic)
#   2. Refuses if that tag has no GitHub release, or the release is a DRAFT
#   3. Runs `doli release verify` against this host's maintainer trust root
#   4. Exits 0 only when published AND verified. Never mutates any release.
#
# Suitable for cron or a one-shot manual check; exit code is the only contract.

set -euo pipefail

REPO="${REPO:-doli-network/doli}"
REPO_DIR="${REPO_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

# --- Locate doli CLI (same resolution order as publish-release.sh) ---
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

# Version-sorted, not lexicographic: v6.26.9 must never outrank v6.26.10.
TAG="$(git -C "$REPO_DIR" tag --list 'v*' --sort=-v:refname 2>/dev/null | head -1)" || true
if [[ -z "$TAG" ]]; then
    echo "UNHEALTHY: no v* tag found in $REPO_DIR — nothing to monitor." >&2
    exit 1
fi

# A gh failure means "no release for this tag", a refusal, not a pass.
DRAFT_JSON=""
if ! DRAFT_JSON="$(gh release view "$TAG" --repo "$REPO" --json isDraft 2>/dev/null)"; then
    echo "UNHEALTHY $TAG: no GitHub release found for the newest tag. Run scripts/sign-release.sh $TAG then scripts/publish-release.sh $TAG." >&2
    exit 1
fi

# Parsed locally, not via `gh --jq`, so a stub or transport that ignores that flag
# cannot silently hand us unparsed JSON where we expect a bare true/false.
IS_DRAFT="$(jq -r '.isDraft' <<<"$DRAFT_JSON" 2>/dev/null)" || IS_DRAFT="true"
if [[ "$IS_DRAFT" != "false" ]]; then
    echo "UNHEALTHY $TAG: release is still a DRAFT — unreachable by nodes and doli upgrade. Run scripts/publish-release.sh $TAG to promote it." >&2
    exit 1
fi

if ! "$DOLI" release verify --version "$TAG"; then
    echo "UNHEALTHY $TAG: 'doli release verify' failed — signatures are missing or sub-threshold. Run scripts/sign-release.sh $TAG to re-sign." >&2
    exit 1
fi

echo "HEALTHY $TAG: published and verified against the maintainer trust root."
