#!/usr/bin/env bash
# ============================================================================
# gauntlet-gs015.sh — GS-015 "newest release published and signed" scenario.
#
# Sourced by scripts/gauntlet.sh. Replays INC-I-202: CI published v6.26.2 with
# the 0-entry SIGNATURES.json scaffold, so every fail-closed `doli upgrade`
# refused it with "Insufficient signatures: 0/3", and nothing in the repo
# noticed. OBSERVATIONAL, READ-ONLY, not opt-in, no confirm-var: it reads the
# GitHub release API and this repo, never the chain, a node, or a release.
#
#   gs015-newest-release-published-and-signed — delegates to
#     scripts/monitor-release-signed.sh, behind a preflight (below).
#   gs015-workflow-drafts-releases — the `draft: true` gate at
#     .github/workflows/release.yml:592 is the only thing keeping an unsigned CI
#     artifact unreachable, and a one-word revert of it is otherwise silent.
#
# Env: GS015_REPO_DIR, GS015_MONITOR, GS015_WORKFLOW.
# ============================================================================

_GS015_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GS015_REPO_DIR="${GS015_REPO_DIR:-$_GS015_ROOT}"
GS015_MONITOR="${GS015_MONITOR:-$_GS015_ROOT/scripts/monitor-release-signed.sh}"
GS015_WORKFLOW="${GS015_WORKFLOW:-$GS015_REPO_DIR/.github/workflows/release.yml}"

# Resolution order of monitor-release-signed.sh:28-38, one step STRICTER: a set but
# non-executable DOLI_CLI reaches the monitor, exits 127 there, and is reported as
# "signatures are missing or sub-threshold" — the INC-I-202 symptom, from a typo.
_gs015_doli_resolvable() {
  _GS015_DOLI_WHY="doli CLI not resolvable (DOLI_CLI, ./target/release/doli, PATH)"
  if [ -n "${DOLI_CLI:-}" ]; then
    [ -x "$DOLI_CLI" ] && return 0
    _GS015_DOLI_WHY="DOLI_CLI is set to '$DOLI_CLI', which is not executable"
    return 1
  fi
  [ -x "./target/release/doli" ] && return 0
  command -v doli >/dev/null 2>&1
}

_gs015_release_check() {
  local t="$1" out msg rc=0 tool
  # Preflight before the monitor: an absent, logged-out or offline `gh` makes
  # `gh release view` fail, which the monitor renders as "no GitHub release
  # found" (monitor:50) — a FAIL against a release that is fine. One false FAIL
  # is how a scenario earns a standing waiver and stops guarding anything.
  if ! command -v gh >/dev/null 2>&1; then
    SKIP_REASONS="$SKIP_REASONS; $t: gh not on PATH — GitHub release state unreadable here"
    return 2
  fi
  if ! gh auth status >/dev/null 2>&1; then
    SKIP_REASONS="$SKIP_REASONS; $t: gh is unauthenticated or offline — release state unreadable, not a release defect"
    return 2
  fi
  # The monitor's remaining runtime dependencies, each a silent false FAIL when absent:
  # jq exits 127 into a fail-closed IS_DRAFT=true (monitor:56), which prints "release is
  # still a DRAFT"; git missing reads as "no v* tag found". Both are facts about the host.
  for tool in jq git; do
    if ! command -v "$tool" >/dev/null 2>&1; then
      SKIP_REASONS="$SKIP_REASONS; $t: $tool not on PATH — release state unreadable here, not a release defect"
      return 2
    fi
  done
  if [ ! -r "$GS015_MONITOR" ]; then
    SKIP_REASONS="$SKIP_REASONS; $t: monitor missing or unreadable at $GS015_MONITOR"
    return 2
  fi
  if ! _gs015_doli_resolvable; then
    SKIP_REASONS="$SKIP_REASONS; $t: $_GS015_DOLI_WHY — signatures unverifiable"
    return 2
  fi
  # actions/checkout fetches no tags at the default fetch-depth: "no tags here" is an
  # environment fact. "Tags exist but the newest has no release" stays a FAIL.
  if [ -z "$(git -C "$GS015_REPO_DIR" tag --list 'v*' 2>/dev/null)" ]; then
    SKIP_REASONS="$SKIP_REASONS; $t: no v* tag in $GS015_REPO_DIR (tagless or shallow checkout) — nothing to check"
    return 2
  fi
  out="$(REPO_DIR="$GS015_REPO_DIR" bash "$GS015_MONITOR" 2>&1)" || rc=$?
  # The (UN)HEALTHY line is the monitor's contract; the CLI it wraps prints above it.
  msg="$(printf '%s\n' "$out" | grep 'HEALTHY' | tail -1)"
  [ -n "$msg" ] || msg="$(printf '%s' "$out" | tr '\n' ' ')"
  if [ "$rc" -ne 0 ]; then
    FAIL_REASONS="$FAIL_REASONS; $t: ${msg:-monitor exited $rc with no output}"
    return 1
  fi
  INFO_REASONS="$INFO_REASONS; $t: ${msg:-newest tag published and verified}"
  return 0
}

# The step block that carries the release-creation action, so the gate is read where it
# acts: a `draft: true` under a second release path (nightly, RC, mirror) must not stand
# in for the one at release.yml:592. Steps open with `- name:` or `- uses:`.
_gs015_release_step_block() {
  awk '
    /^[[:space:]]*-[[:space:]]+(name|uses):/ { if (hit) printf "%s", blk; blk=""; hit=0 }
    { blk = blk $0 "\n" }
    /uses:[[:space:]]*softprops\/action-gh-release/ { hit=1 }
    END { if (hit) printf "%s", blk }
  ' "$1"
}

_gs015_workflow_check() {
  local t="$1" block
  # A missing file means the gate was not checked, never "no revert found".
  if [ ! -r "$GS015_WORKFLOW" ]; then
    SKIP_REASONS="$SKIP_REASONS; $t: workflow missing or unreadable at $GS015_WORKFLOW — draft gate unchecked"
    return 2
  fi
  # A repo fact, not a host fact: the release path was rewritten and the gate now lives
  # somewhere this check cannot see. That is exactly what a human has to look at.
  block="$(_gs015_release_step_block "$GS015_WORKFLOW")"
  if [ -z "$block" ]; then
    FAIL_REASONS="$FAIL_REASONS; $t: no release-creation step (softprops/action-gh-release) in $GS015_WORKFLOW — the draft gate cannot be read where it acts"
    return 1
  fi
  # Anchored on the KEY at line start: the comment block above release.yml:592 and
  # the `prerelease:` neighbour also carry the word "draft", and a commented-out
  # `# draft: true` must not satisfy the gate.
  if grep -Eq '^[[:space:]]*draft:[[:space:]]*true[[:space:]]*(#.*)?$' <<<"$block"; then
    INFO_REASONS="$INFO_REASONS; $t: release.yml still sets draft: true — CI cannot reach installers unsigned"
    return 0
  fi
  FAIL_REASONS="$FAIL_REASONS; $t: the release-creation step of release.yml no longer sets 'draft: true' ($GS015_WORKFLOW) — an unsigned build would publish straight to Latest"
  return 1
}

_gs015_assert() {
  local t="$1"
  case "$t" in
    gs015-newest-release-published-and-signed) _gs015_release_check "$t"; return $? ;;
    gs015-workflow-drafts-releases) _gs015_workflow_check "$t"; return $? ;;
  esac
  FAIL_REASONS="$FAIL_REASONS; $t: unknown GS-015 assertion token"
  return 1
}
