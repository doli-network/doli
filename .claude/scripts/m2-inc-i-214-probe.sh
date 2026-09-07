#!/usr/bin/env bash
# m2-inc-i-214-probe.sh — outcome probe for INC-I-214 M2.
#
# Measures what the release tooling actually SENDS to `gh` and `doli` when the
# documented draft -> sign -> verify -> promote -> confirm order is walked end to
# end. Runs the three real scripts against stubbed `gh`/`doli` that record argv;
# no network, no GitHub release, no repo mutation.
#
# Prints one line:
#   sign_checksums=<n>/3 promote_flags=<n>/2 verify_bootstrap=<n>/3
#
# sign_checksums   — `doli release sign` calls carrying --checksums (a DRAFT can
#                    only be signed from local bytes; the public URL 404s)
# promote_flags    — `gh release edit` calls carrying --prerelease=false AND
#                    --latest (both publish-release.sh branches: banner + no banner)
# verify_bootstrap — `doli release verify` calls carrying --trust-root bootstrap
#                    (both publish-release.sh runs + monitor-release-signed.sh must agree)
#
# SCRIPTS_DIR overrides which copy of the three scripts is walked, so the same probe
# can measure a past revision (`git show <rev>:scripts/... `) and the working tree.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPTS_DIR="${SCRIPTS_DIR:-$ROOT/scripts}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

BIN="$WORK/bin"; mkdir -p "$BIN"
export ARGV_LOG="$WORK/argv.log"
: > "$ARGV_LOG"

# --- stubs -----------------------------------------------------------------
cat > "$BIN/gh" <<'GH'
#!/usr/bin/env bash
printf 'gh %s\n' "$*" >> "$ARGV_LOG"
case "$1 ${2:-}" in
  "release view")
    if [[ "$*" == *"--json assets"* ]]; then
      printf 'CHECKSUMS.txt\ndoli-node-linux-x86_64\n'
      [[ "${GH_HAS_SIGS:-0}" == "1" ]] && printf 'SIGNATURES.json\n'
    elif [[ "$*" == *"--json body"* ]]; then
      printf '%s' "${GH_NOTES:-}"
    elif [[ "$*" == *"--json isDraft"* ]]; then
      printf '{"isDraft":false}\n'
    else
      printf 'v9.9.9\n'
    fi ;;
  "release download")
    dir=""; prev=""
    for a in "$@"; do [[ "$prev" == "--dir" ]] && dir="$a"; prev="$a"; done
    [[ -n "$dir" ]] || dir="."
    printf 'aaaa  doli-node-linux-x86_64\n' > "$dir/CHECKSUMS.txt"
    if [[ "$*" != *"CHECKSUMS.txt"* || "$*" == *"SIGNATURES.json"* ]]; then
      printf '{"version":"9.9.9","checksums_sha256":"aa","signatures":[{"s":1},{"s":2},{"s":3}]}\n' \
        > "$dir/SIGNATURES.json"
    fi ;;
esac
exit 0
GH

cat > "$BIN/doli" <<'DOLI'
#!/usr/bin/env bash
printf 'doli %s\n' "$*" >> "$ARGV_LOG"
if [[ "$*" == *"release sign"* ]]; then
  printf 'Signing release...\n{"version":"9.9.9","signer":"stub","signature":"deadbeef"}\n'
fi
exit 0
DOLI
chmod +x "$BIN/gh" "$BIN/doli"

# --- fixtures --------------------------------------------------------------
KEYS="$WORK/keys"; mkdir -p "$KEYS"
for i in 1 2 3; do printf '{"stub":%d}\n' "$i" > "$KEYS/maintainer-$i.json"; done

FIXTURE_REPO="$WORK/repo"; mkdir -p "$FIXTURE_REPO"
git -C "$FIXTURE_REPO" init -q 2>/dev/null
git -C "$FIXTURE_REPO" -c user.email=p@p -c user.name=p commit -q --allow-empty -m init 2>/dev/null
git -C "$FIXTURE_REPO" tag v9.9.9 2>/dev/null

export PATH="$BIN:$PATH"
export DOLI_CLI="$BIN/doli"
export KEY_DIR="$KEYS"
export REPO="stub/stub"

# --- walk the documented order --------------------------------------------
bash "$SCRIPTS_DIR/sign-release.sh" 9.9.9 >/dev/null 2>&1

GH_HAS_SIGS=1 GH_NOTES='' \
  bash "$SCRIPTS_DIR/publish-release.sh" 9.9.9 >/dev/null 2>&1
GH_HAS_SIGS=1 GH_NOTES='<!-- DOLI-UNSIGNED-DRAFT-WARNING:BEGIN -->
unsigned
<!-- DOLI-UNSIGNED-DRAFT-WARNING:END -->
notes' \
  bash "$SCRIPTS_DIR/publish-release.sh" 9.9.9 >/dev/null 2>&1

REPO_DIR="$FIXTURE_REPO" bash "$SCRIPTS_DIR/monitor-release-signed.sh" >/dev/null 2>&1

# --- count what reached the tools -----------------------------------------
sign_checksums=$(grep -c '^doli .*release sign .*--checksums ' "$ARGV_LOG" || true)
promote_flags=$(grep '^gh release edit ' "$ARGV_LOG" \
                  | grep -c -- '--prerelease=false' || true)
promote_latest=$(grep '^gh release edit ' "$ARGV_LOG" \
                  | grep -- '--prerelease=false' | grep -c -- '--latest' || true)
(( promote_flags = promote_latest < promote_flags ? promote_latest : promote_flags ))
verify_bootstrap=$(grep -c '^doli .*release verify .*--trust-root bootstrap' "$ARGV_LOG" || true)

echo "sign_checksums=${sign_checksums}/3 promote_flags=${promote_flags}/2 verify_bootstrap=${verify_bootstrap}/3"
