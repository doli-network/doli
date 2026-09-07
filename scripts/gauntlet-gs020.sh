#!/usr/bin/env bash
# ============================================================================
# gauntlet-gs020.sh — GS-020 "staged-upgrade handoff refusal" scenario.
#
# Sourced by scripts/gauntlet.sh, and runnable standalone. Replays INC-I-215:
# a sandboxed node cannot write its install target, so it must HAND OFF the
# release it verified, and the root side must REFUSE anything sub-threshold.
# OBSERVATIONAL, READ-ONLY and OFFLINE, in the DEFAULT gate, NOT opt-in, no
# confirm-var: it builds a scratch staging dir, runs the CLI against it, and
# deletes it. It never touches the chain, a node, a unit file or a real binary.
#
#   gs020-refuses-sub-threshold   — a 0-signature staging exits non-zero.
#   gs020-target-untouched        — the install target keeps its bytes.
#   gs020-marker-parked           — `ready` becomes `ready.rejected`.
#   gs020-preflight-token-present — the node still emits UPDATE_TARGET_NOT_WRITABLE.
#   gs020-helper-unit-rendered    — the .path unit still carries PathExists=.
#
# SAFETY: every CLI run passes `--service <fake>` and a PATH whose FIRST entry
# holds an inert `sudo`/`systemctl`, so the macOS launchd tier of
# `restart_doli_service` — which kickstarts EVERY "doli" label on this host, and
# this host runs an 18-node testnet — stays unreachable. The trust root is a
# COPY of a node's maintainer_state.bin: a legacy-format migration re-saves the
# file, and that write must land in the scratch dir, never in a node's data dir.
# Technique mirrored from bins/cli/tests/it/inc_i_215_fixture.rs.
#
# Env: GS020_DOLI, GS020_TESTNET_DIR, GS020_PREFLIGHT, GS020_HELPER_UNITS, GS020_DB.
# ============================================================================

_GS020_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GS020_TESTNET_DIR="${GS020_TESTNET_DIR:-$HOME/testnet}"
GS020_PREFLIGHT="${GS020_PREFLIGHT:-$_GS020_ROOT/bins/node/src/updater/preflight.rs}"
GS020_HELPER_UNITS="${GS020_HELPER_UNITS:-$_GS020_ROOT/bins/cli/src/cmd_service_helper_units.rs}"
GS020_VERSION="${GS020_VERSION:-99.0.0}"
GS020_FAKE_SERVICE="${GS020_FAKE_SERVICE:-gs020-fake}"

# Staging filenames, from crates/updater/src/staging.rs. `read_staged` reads exactly
# these; a guessed name is read as "nothing staged", which refuses for the wrong reason.
_GS020_READY="ready"
_GS020_REJECTED="ready.rejected"
_GS020_TARBALL="release.tar.gz"
_GS020_CHECKSUMS="CHECKSUMS.txt"
_GS020_SIGNATURES="SIGNATURES.json"

_GS020_STATE=""          # "" not attempted · ok · skip
_GS020_WHY=""
_GS020_WORK=""
_GS020_RC=0
_GS020_TARGET_SAME=0
_GS020_MARKER=""
_GS020_OUT=""

_gs020_sha256() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

# Triple `platform_tarball_hash` looks for in CHECKSUMS.txt (updater::platform_identifier).
_gs020_triple() {
  local os arch
  case "$(uname -s)" in
    Darwin) os="apple-darwin" ;;
    Linux) os="unknown-linux-gnu" ;;
    *) os="unknown" ;;
  esac
  case "$(uname -m)" in
    arm64 | aarch64) arch="aarch64" ;;
    x86_64 | amd64) arch="x86_64" ;;
    *) arch="unknown" ;;
  esac
  printf '%s-%s' "$arch" "$os"
}

_gs020_cli() {
  if [ -n "${GS020_DOLI:-}" ]; then
    [ -x "$GS020_DOLI" ] && { printf '%s' "$GS020_DOLI"; return 0; }
    return 1
  fi
  local c
  for c in "$_GS020_ROOT/target/release/doli" "$_GS020_ROOT/target/debug/doli"; do
    [ -x "$c" ] && { printf '%s' "$c"; return 0; }
  done
  return 1
}

# Build the staging, run the refusal once, record the facts, delete everything.
# Lazy and memoised: sourcing this file costs nothing, and the CLI runs at most once.
_gs020_setup() {
  if [ -n "$_GS020_STATE" ]; then
    [ "$_GS020_STATE" = "ok" ] && return 0
    return 2
  fi
  _GS020_STATE="skip"
  local cli root_src tsha csha
  cli="$(_gs020_cli)" || {
    _GS020_WHY="no doli CLI (GS020_DOLI, target/release/doli, target/debug/doli) — nothing to exercise"
    return 2
  }
  root_src="$(find "$GS020_TESTNET_DIR" -name maintainer_state.bin -type f 2>/dev/null | head -1)"
  [ -n "$root_src" ] || {
    _GS020_WHY="no maintainer_state.bin under $GS020_TESTNET_DIR — no trust root to verify against"
    return 2
  }
  if ! command -v shasum >/dev/null 2>&1 && ! command -v sha256sum >/dev/null 2>&1; then
    _GS020_WHY="no shasum/sha256sum on PATH — CHECKSUMS.txt cannot be bound to the manifest"
    return 2
  fi
  command -v tar >/dev/null 2>&1 || {
    _GS020_WHY="no tar on PATH — no staged tarball can be built"
    return 2
  }
  _GS020_WORK="$(mktemp -d "${TMPDIR:-/tmp}/gs020.XXXXXX" 2>/dev/null)" || _GS020_WORK=""
  [ -n "$_GS020_WORK" ] && [ -d "$_GS020_WORK" ] || {
    _GS020_WHY="could not create a scratch dir under ${TMPDIR:-/tmp}"
    return 2
  }
  local w="$_GS020_WORK"
  mkdir -p "$w/staging" "$w/data" "$w/bin" "$w/shim" "$w/payload" || {
    _GS020_WHY="could not lay out the scratch dir $w"
    _gs020_cleanup
    return 2
  }
  cp "$root_src" "$w/data/maintainer_state.bin" || {
    _GS020_WHY="could not copy $root_src into the scratch data dir"
    _gs020_cleanup
    return 2
  }

  printf '#!/bin/sh\n# GS-020 STAGED PAYLOAD v%s\nexit 0\n' "$GS020_VERSION" >"$w/payload/doli-node"
  (cd "$w/payload" && tar -czf "$w/staging/$_GS020_TARBALL" doli-node) >/dev/null 2>&1 || {
    _GS020_WHY="tar could not build the staged tarball"
    _gs020_cleanup
    return 2
  }
  tsha="$(_gs020_sha256 "$w/staging/$_GS020_TARBALL")"
  printf '%s  doli-node-v%s-%s.tar.gz\n' "$tsha" "$GS020_VERSION" "$(_gs020_triple)" \
    >"$w/staging/$_GS020_CHECKSUMS"
  csha="$(_gs020_sha256 "$w/staging/$_GS020_CHECKSUMS")"
  # ZERO signatures over a correctly bound CHECKSUMS.txt: the L1/L2 bindings hold, so
  # the only thing left to refuse is the signature threshold (INC-I-202's shape).
  printf '{\n  "version": "%s",\n  "checksums_sha256": "%s",\n  "signatures": []\n}\n' \
    "$GS020_VERSION" "$csha" >"$w/staging/$_GS020_SIGNATURES"
  printf '%s\n' "$GS020_VERSION" >"$w/staging/$_GS020_READY"

  printf '#!/bin/sh\n# GS-020 PRE-EXISTING BINARY\nexit 0\n' >"$w/bin/doli-node"
  cp "$w/bin/doli-node" "$w/target.before"
  printf '#!/bin/sh\nexec "$@"\n' >"$w/shim/sudo"
  printf '#!/bin/sh\nexit 0\n' >"$w/shim/systemctl"
  chmod 755 "$w/shim/sudo" "$w/shim/systemctl"

  PATH="$w/shim:$PATH" "$cli" --network testnet upgrade \
    --from-staged "$w/staging" \
    --data-dir "$w/data" \
    --doli-node-path "$w/bin/doli-node" \
    --service "$GS020_FAKE_SERVICE" \
    --yes >"$w/out" 2>&1
  _GS020_RC=$?

  if cmp -s "$w/bin/doli-node" "$w/target.before"; then _GS020_TARGET_SAME=1; else _GS020_TARGET_SAME=0; fi
  if [ -f "$w/staging/$_GS020_REJECTED" ] && [ ! -e "$w/staging/$_GS020_READY" ]; then
    _GS020_MARKER="parked"
  elif [ -e "$w/staging/$_GS020_READY" ]; then
    _GS020_MARKER="$_GS020_READY is still armed — a .path unit would retrigger the helper in a loop"
  else
    _GS020_MARKER="neither $_GS020_READY nor $_GS020_REJECTED is present — the refusal left no evidence"
  fi
  _GS020_OUT="$(tr '\n' ' ' <"$w/out" | tail -c 300)"
  _gs020_cleanup
  _GS020_STATE="ok"
  return 0
}

_gs020_cleanup() {
  [ -n "${_GS020_WORK:-}" ] && [ -d "$_GS020_WORK" ] && rm -rf "$_GS020_WORK"
  _GS020_WORK=""
  return 0
}

_gs020_refusal_check() {
  local t="$1"
  _gs020_setup || { SKIP_REASONS="$SKIP_REASONS; $t: $_GS020_WHY"; return 2; }
  if [ "$_GS020_RC" -ne 0 ]; then
    INFO_REASONS="$INFO_REASONS; $t: refused a 0-signature staging (exit $_GS020_RC)"
    return 0
  fi
  FAIL_REASONS="$FAIL_REASONS; $t: the CLI INSTALLED a staging carrying zero maintainer signatures (exit 0): $_GS020_OUT"
  return 1
}

_gs020_target_check() {
  local t="$1"
  _gs020_setup || { SKIP_REASONS="$SKIP_REASONS; $t: $_GS020_WHY"; return 2; }
  if [ "$_GS020_TARGET_SAME" = "1" ]; then
    INFO_REASONS="$INFO_REASONS; $t: the install target is byte-identical after the refusal"
    return 0
  fi
  FAIL_REASONS="$FAIL_REASONS; $t: a refused handoff MODIFIED the install target (INC-I-153): $_GS020_OUT"
  return 1
}

_gs020_marker_check() {
  local t="$1"
  _gs020_setup || { SKIP_REASONS="$SKIP_REASONS; $t: $_GS020_WHY"; return 2; }
  if [ "$_GS020_MARKER" = "parked" ]; then
    INFO_REASONS="$INFO_REASONS; $t: $_GS020_READY was renamed to $_GS020_REJECTED"
    return 0
  fi
  FAIL_REASONS="$FAIL_REASONS; $t: $_GS020_MARKER"
  return 1
}

# A structural token: no staging, no CLI, no host state — the file either still carries
# the string or the mechanism was deleted, and both are repo facts, never host facts.
_gs020_source_token() {
  local t="$1" file="$2" token="$3" note="$4"
  if [ ! -r "$file" ]; then
    FAIL_REASONS="$FAIL_REASONS; $t: $file is missing or unreadable — $note"
    return 1
  fi
  if grep -Fq "$token" "$file"; then
    INFO_REASONS="$INFO_REASONS; $t: $file still carries $token"
    return 0
  fi
  FAIL_REASONS="$FAIL_REASONS; $t: $file no longer carries $token — $note"
  return 1
}

_gs020_assert() {
  local t="$1"
  case "$t" in
    gs020-refuses-sub-threshold) _gs020_refusal_check "$t"; return $? ;;
    gs020-target-untouched) _gs020_target_check "$t"; return $? ;;
    gs020-marker-parked) _gs020_marker_check "$t"; return $? ;;
    gs020-preflight-token-present)
      _gs020_source_token "$t" "$GS020_PREFLIGHT" "UPDATE_TARGET_NOT_WRITABLE" \
        "the sandboxed node's only operator-visible signal that it must hand off"
      return $? ;;
    gs020-helper-unit-rendered)
      _gs020_source_token "$t" "$GS020_HELPER_UNITS" "PathExists=" \
        "without the .path trigger the staged handoff is never consumed"
      return $? ;;
  esac
  FAIL_REASONS="$FAIL_REASONS; $t: unknown GS-020 assertion token"
  return 1
}

# ── standalone runner ───────────────────────────────────────────────────────
_gs020_trim() { printf '%s' "${1#; }"; }
_gs020_sq() { printf '%s' "$1" | sed "s/'/''/g"; }

# The worktree this may run from has no .omega/; the DB lives in the main checkout.
_gs020_db() {
  local db common
  db="${GS020_DB:-$_GS020_ROOT/.omega/memory.db}"
  [ -f "$db" ] && { printf '%s' "$db"; return 0; }
  common="$(git -C "$_GS020_ROOT" rev-parse --git-common-dir 2>/dev/null)" || return 1
  [ -n "$common" ] || return 1
  case "$common" in /*) ;; *) common="$_GS020_ROOT/$common" ;; esac
  db="$(cd "$common/.." 2>/dev/null && pwd)/.omega/memory.db"
  [ -f "$db" ] && { printf '%s' "$db"; return 0; }
  return 1
}

_gs020_log_run() {
  local status="$1" run="$2" ok="$3" dur="$4" db sha wf fj
  db="$(_gs020_db)" || return 0
  sha="$(git -C "$_GS020_ROOT" rev-parse --short HEAD 2>/dev/null)"
  wf="${WORKFLOW_RUN_ID:-}"; [ -n "$wf" ] || wf="NULL"
  fj="[]"
  [ "$status" = "fail" ] && fj="[{\"scenario\":\"GS-020\",\"reasons\":\"$(_gs020_trim "$FAIL_REASONS")\"}]"
  sqlite3 "$db" "INSERT INTO gauntlet_runs (run_id, mechanism_id, status, scenarios_run, scenarios_passed, failures, duration_seconds, git_sha) VALUES ($wf, 'GS-020', '$status', $run, $ok, '$(_gs020_sq "$fj")', $dur, '$sha');" 2>/dev/null \
    || echo "GS-020: could not write the gauntlet_runs row (DB error)" >&2
  return 0
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  set -uo pipefail
  trap '_gs020_cleanup' EXIT
  SKIP_REASONS=""; FAIL_REASONS=""; INFO_REASONS=""
  _gs020_start=$SECONDS
  _gs020_run=0; _gs020_pass=0; _gs020_skip=0; _gs020_fail=0
  for _gs020_t in gs020-refuses-sub-threshold gs020-target-untouched gs020-marker-parked \
    gs020-preflight-token-present gs020-helper-unit-rendered; do
    _gs020_run=$((_gs020_run + 1))
    _gs020_assert "$_gs020_t"
    case "$?" in
      0) _gs020_pass=$((_gs020_pass + 1)) ;;
      2) _gs020_skip=$((_gs020_skip + 1)) ;;
      *) _gs020_fail=$((_gs020_fail + 1)) ;;
    esac
  done
  if [ "$_gs020_fail" -gt 0 ]; then
    _gs020_status="fail"; _gs020_line="GS-020 fail($(_gs020_trim "$FAIL_REASONS"))"; _gs020_exit=1
  elif [ "$_gs020_skip" -gt 0 ]; then
    _gs020_status="pass"; _gs020_line="GS-020 skipped($(_gs020_trim "$SKIP_REASONS"))"; _gs020_exit=0
  else
    _gs020_status="pass"; _gs020_line="GS-020 pass"; _gs020_exit=0
  fi
  _gs020_log_run "$_gs020_status" "$_gs020_run" "$((_gs020_pass + _gs020_skip))" \
    "$((SECONDS - _gs020_start))"
  printf '%s\n' "$_gs020_line"
  exit "$_gs020_exit"
fi
