#!/usr/bin/env bash
# INC-I-178 M8 outcome probe — REQ-BLS-015/REQ-BLS-016 (documentation-vs-code drift).
#
# OUTPUT CONTRACT:
#   O1  one "R<id>|file:line:<matched text>" line per surviving NEGATIVE hit
#       (a retired/false claim that still appears in a documentation file).
#   O2  one "P<id>|MISSING: <what>" line per UNSATISFIED positive check
#       (a claim the docs must gain, but don't yet).
#   O3  a final line that is EXACTLY the total integer count
#       (surviving negatives + unsatisfied positives + SCOPE-ERROR entries).
#   O4  exit code 0 always — this is a metric probe, not a gate.
#
# Read-only, cwd-independent (resolves repo root from BASH_SOURCE), re-runnable,
# no argv required. Target: before > 0 (drift exists today), after = 0 (aligned).
#
# SCOPE: greps only documentation files — never source, never
# specs/attestation-bls-architecture.md, never docs/redesigns/, never
# docs/improvements/ (deliberately-preserved historical analysis), never
# docs/.workflow/, never .omega/.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." >/dev/null 2>&1 && pwd)"

total=0

# --- scope file existence guard (false-negative protection) ----------------
SCOPE_FILES=(
  "specs/protocol.md"
  "specs/security_model.md"
  "WHITEPAPER.md"
  "WHITEPAPER-es.md"
  "docs/architecture.md"
  "docs/rpc_reference.md"
  "docs/troubleshooting.md"
  "docs/DOCS.md"
)

file_ok() {
  # $1 = path relative to ROOT
  [ -f "$ROOT/$1" ]
}

for f in "${SCOPE_FILES[@]}"; do
  if ! file_ok "$f"; then
    echo "SCOPE-ERROR: $f missing"
    total=$((total + 1))
  fi
done

skill_glob_ok=1
for f in "$ROOT"/.claude/skills/*/SKILL.md; do
  [ -e "$f" ] && skill_glob_ok=0 && break
done
if [ "$skill_glob_ok" -ne 0 ]; then
  echo "SCOPE-ERROR: .claude/skills/*/SKILL.md missing"
  total=$((total + 1))
fi

# --- helpers -----------------------------------------------------------------
# check_literal <id> <relfile> <fixed-string-pattern>
# Prints one "R<id>|relfile:line:<text>" line per match, adds to total.
check_literal() {
  local id="$1" relfile="$2" pattern="$3"
  file_ok "$relfile" || return 0
  local out
  out="$(grep -F -n -- "$pattern" "$ROOT/$relfile" 2>/dev/null || true)"
  [ -z "$out" ] && return 0
  local line lineno text
  while IFS= read -r line; do
    [ -z "$line" ] && continue
    lineno="${line%%:*}"
    text="${line#*:}"
    echo "${id}|${relfile}:${lineno}:${text}"
    total=$((total + 1))
  done <<<"$out"
}

# check_positive <id> <relfile> <fixed-string-pattern> <description>
# Prints "P<id>|MISSING: <description>" and adds to total if pattern absent.
check_positive() {
  local id="$1" relfile="$2" pattern="$3" desc="$4"
  file_ok "$relfile" || return 0
  if ! grep -F -q -- "$pattern" "$ROOT/$relfile" 2>/dev/null; then
    echo "${id}|MISSING: ${desc}"
    total=$((total + 1))
  fi
}

# --- R1-R7: WHITEPAPER.md / WHITEPAPER-es.md retired attestation claims ------
check_literal R1 "WHITEPAPER.md" "cryptographic proof that the bitfield is honest"
check_literal R2 "WHITEPAPER-es.md" "prueba criptografica de que el bitfield es honesto"
check_literal R3 "WHITEPAPER.md" "Sign(block_hash || slot)"
check_literal R3 "WHITEPAPER-es.md" "Sign(block_hash || slot)"
check_literal R4 "WHITEPAPER.md" 'bitfield** in the block header (`presence_root`)'
check_literal R5 "WHITEPAPER-es.md" 'bitfield** en la cabecera del bloque (`presence_root`)'
check_literal R6 "WHITEPAPER.md" "causes aggregate signature verification to fail"
check_literal R7 "WHITEPAPER-es.md" "causa que la verificacion de la firma agregada falle"

# --- R8-R9: specs/security_model.md — bls fields are MANDATORY, not optional -
check_literal R8 "specs/security_model.md" "(48 bytes, optional)"
check_literal R9 "specs/security_model.md" "(96 bytes, optional)"

# --- R10-R16: specs/protocol.md — presence_root/bitfield/ZERO drift ----------
check_literal R10 "specs/protocol.md" "decode presence_root attestation bitfields"
check_literal R11 "specs/protocol.md" "decode_attestation_bitfield(block.presence_root"
check_literal R12 "specs/protocol.md" "presence commitments are not used for consensus"
check_literal R13 "specs/protocol.md" "Merkle root of RegionAggregates"
check_literal R14 "specs/protocol.md" 'stored in `header.presence_root` for v2+ blocks'
check_literal R15 "specs/protocol.md" "ZERO in deterministic model"
check_literal R16 "specs/protocol.md" "Hash::ZERO in deterministic scheduler model"

# --- R17: docs/rpc_reference.md ----------------------------------------------
check_literal R17 "docs/rpc_reference.md" "decodes presence_root bitfields"

# --- R18: skill lines conflating presence_root (commitment hash) with bitfield
if [ "$skill_glob_ok" -eq 0 ]; then
  out="$(grep -n "presence_root" "$ROOT"/.claude/skills/*/SKILL.md 2>/dev/null | grep -i "bitfield" || true)"
  if [ -n "$out" ]; then
    while IFS= read -r line; do
      [ -z "$line" ] && continue
      filepart="${line%%:*}"
      rest="${line#*:}"
      lineno="${rest%%:*}"
      text="${rest#*:}"
      relfilepart="${filepart#"$ROOT"/}"
      echo "R18|${relfilepart}:${lineno}:${text}"
      total=$((total + 1))
    done <<<"$out"
  fi
fi

# --- R19-R22: drift found by the M8 doc-drift survey, outside R1-R18 ---------
# Added after the survey landed; their `before` values were measured against a
# pristine b8a794e7 worktree (R19=1 R20=1 R21=2 R22=2), so the extended probe's
# honest before is 31, not 25.
# R19/R20 name symbols that were DELETED from the code. A line that SAYS so is a
# correction, not a drifted claim, so lines marked DELETED are not counted — a
# probe that flags its own fix cannot reach zero without deleting the correction.
check_literal_undeleted() {
  local id="$1" relfile="$2" pattern="$3"
  file_ok "$relfile" || return 0
  local out
  out="$(grep -F -n -- "$pattern" "$ROOT/$relfile" 2>/dev/null | grep -v -F 'DELETED' || true)"
  [ -z "$out" ] && return 0
  local line lineno text
  while IFS= read -r line; do
    [ -z "$line" ] && continue
    lineno="${line%%:*}"
    text="${line#*:}"
    echo "${id}|${relfile}:${lineno}:${text}"
    total=$((total + 1))
  done <<<"$out"
}
check_literal_undeleted R19 ".claude/skills/node/SKILL.md" "aggregate_bls_signatures()"
check_literal_undeleted R20 ".claude/skills/crypto/SKILL.md" "attestation_message(&block_hash, slot)"
check_literal R21 "specs/protocol.md" "(optional, default empty)"

# R22: BITFIELD_BODY_ACTIVATION_HEIGHT was DELETED in INC-I-178 M1; no skill may
# still describe block fields in terms of that gate.
for f in "$ROOT"/.claude/skills/*/SKILL.md; do
  [ -e "$f" ] || continue
  out="$(grep -F -n -- "BITFIELD_BODY" "$f" 2>/dev/null || true)"
  [ -z "$out" ] && continue
  while IFS= read -r line; do
    [ -z "$line" ] && continue
    lineno="${line%%:*}"
    text="${line#*:}"
    echo "R22|${f#"$ROOT"/}:${lineno}:${text}"
    total=$((total + 1))
  done <<<"$out"
done

# --- P1-P3: positive alignment checks (drift-until-satisfied) ---------------
check_positive P1 "specs/protocol.md" "inc_i_178_attestation_bls_activation_height" \
  "specs/protocol.md must name the inc_i_178_attestation_bls_activation_height gate"
check_positive P2 "docs/troubleshooting.md" "ATTESTATION_VERIFY_FAILED" \
  "docs/troubleshooting.md must document ATTESTATION_VERIFY_FAILED"
check_positive P3 "docs/DOCS.md" "attestation-bls-verification-improvement.md" \
  "docs/DOCS.md must index attestation-bls-verification-improvement.md (REQ-BLS-016)"

echo "$total"
exit 0
