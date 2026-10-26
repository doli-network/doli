#!/usr/bin/env bash
# test_whitepaper_defense_text.sh — text check of the producer-defense rules in both whitepapers.
# covers: WHITEPAPER.md WHITEPAPER-es.md
# REQ-LDP-040 — Decision: a FAIL means §6.1 does not list the scheduled producer's block signature as a validity condition from its activation height.
# REQ-LDP-041 — Decision: a FAIL means §7.4 still describes a claim step, or not the locked unbonding output that a slash can burn.
# REQ-LDP-042 — Decision: a FAIL means §7 does not say Registration, AddBond and RequestWithdrawal need the named producer's own signature.
# REQ-LDP-043 — Decision: a FAIL means §11.3 keeps immediate exclusion or re-registration, or omits the locked-funds burn or the reporter rule.
# REQ-LDP-044 — Decision: a FAIL means ClaimWithdrawal or a two-step withdrawal survives in the type list, parameter table or summary list.
# REQ-LDP-045 — Decision: a FAIL means a changed passage lacks "activation height", or carries a height, a date or a network-activation claim.
# REQ-LDP-046 — Decision: RED on the pre-edit text and GREEN after it prove the check reads the edited passages.
# REQ-LDP-047 — Decision: a FAIL of a "passage found" check in one language means a section changed in EN but not ES (or back), so the per-section old→new report describes a change one reader never sees.
# REQ-LDP-048 — Decision: a FAIL means the §19.4 type count differs from the TxType variants in crates/core/src/transaction/types.rs minus the refused ClaimWithdrawal tombstone.
# REQ-LDP-061 — Decision: a FAIL means memory holds no active whitepaper/defense-rules decision saying the drift entries stay open until heights are pinned (SKIP without .omega/memory.db).
# OUTPUT CONTRACT: outputs = one `PASS|FAIL <lang> <check>` line per check, a `SUMMARY fails=<n>` line, exit code.
#   Per language (EN, ES) the passages read are: §6.1 list, §7 producer-auth paragraph, §7.4 withdrawal and
#   slashable paragraphs, §11.3 section, §19.4 type paragraph, summary-list line, unbonding table row, whole file.
#   Per passage: found · has "activation height" · no 4+ digit number but 60,480 · no `20NN-` date · no network named
#   except the unbonding parameter; per rule: required phrase present, retired phrase absent.
# INPUT PARTITIONS: pre-edit text (anchors missing, retired phrases present → exit 1) · aligned text (exit 0) ·
#   a missing file or a planted retired phrase via WP_EN / WP_ES (exit 1).
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EN="${WP_EN:-$ROOT/WHITEPAPER.md}"
ES="${WP_ES:-$ROOT/WHITEPAPER-es.md}"
FAILS=0

res() {
  if [ "$1" = 0 ]; then echo "PASS $2"; else echo "FAIL $2"; FAILS=$((FAILS + 1)); fi
}
has() { printf '%s' "$1" | grep -qF -- "$2"; }
lacks() { ! printf '%s' "$1" | grep -qF -- "$2"; }
check_has() { has "$2" "$3"; res $? "$1 has: $3"; }
check_lacks() { lacks "$2" "$3"; res $? "$1 lacks: $3"; }

section() {
  awk -v s="$2" -v e="$3" 'index($0, s) == 1 { f = 1 } f && index($0, e) == 1 { exit } f' "$1"
}
para() { awk -v a="$2" 'BEGIN { RS = "" } index($0, a) { print; exit }' "$1"; }
line() { grep -F -- "$2" "$1" | head -1; }

limits() {
  local tag="$1" text="$2" ah="$3" net_ok="$4" rest
  [ -n "$text" ]; res $? "$tag passage found"
  check_has "$tag" "$text" "$ah"
  rest="$(printf '%s' "$text" | sed "s/$net_ok//g")"
  ! printf '%s' "$rest" | sed 's/60,480//g' | grep -Eq '[0-9]{1,3}(,[0-9]{3})+|[0-9]{4,}'
  res $? "$tag no height or large number besides 60,480"
  ! printf '%s' "$rest" | grep -Eq '20[0-9]{2}-'; res $? "$tag no date"
  ! printf '%s' "$rest" | grep -Eiq 'mainnet|testnet|devnet'; res $? "$tag no network named"
}

check_lang() {
  local L="$1" F="$2" AH NET ROW SUM P61 P74A P74B PAUTH P113 P194 PSUM PROW
  [ -f "$F" ]; res $? "$L file present"
  if [ "$L" = EN ]; then
    AH="activation height"; NET="60,480 blocks on mainnet"
    ROW="| Unbonding period |"; SUM="- Bond lifecycle:"
    P74A="$(para "$F" "single locked unbonding output")"
    P74B="$(para "$F" "a slash can still burn it")"
    PAUTH="$(para "$F" "signature of the producer it names")"
    P194="$(para "$F" "explicit transaction types")"
  else
    AH="altura de activacion"; NET="60,480 bloques en mainnet"
    ROW="| Periodo de desvinculacion |"; SUM="- Ciclo de vida de bonds:"
    P74A="$(para "$F" "unica salida de desvinculacion bloqueada")"
    P74B="$(para "$F" "un slashing todavia puede quemarla")"
    PAUTH="$(para "$F" "firma del productor que nombra")"
    P194="$(para "$F" "tipos de transaccion explicitos")"
  fi
  P61="$(section "$F" "### 6.1." "### 6.2.")"
  P113="$(section "$F" "### 11.3." "---")"
  PSUM="$(line "$F" "$SUM")"
  PROW="$(line "$F" "$ROW")"

  limits "$L §6.1" "$P61" "$AH" "$NET"
  limits "$L §7.4 withdrawal" "$P74A" "$AH" "$NET"
  limits "$L §7.4 slashable" "$P74B" "$AH" "$NET"
  limits "$L §7 producer auth" "$PAUTH" "$AH" "$NET"
  limits "$L §11.3" "$P113" "$AH" "$NET"
  limits "$L §19.4" "$P194" "$AH" "$NET"
  limits "$L summary" "$PSUM" "$AH" "$NET"

  if [ "$L" = EN ]; then
    check_has "$L §6.1" "$P61" "signature of the scheduled producer"
    check_has "$L §7.4" "$P74A" "no separate claim step"
    check_has "$L §7.4" "$P74A" "next epoch boundary"
    check_has "$L §7.4" "$P74A" "60,480 blocks"
    check_has "$L §7" "$PAUTH" "\`Registration\`, an \`AddBond\` or a \`RequestWithdrawal\`"
    check_has "$L §11.3" "$P113" "including withdrawn bond still locked"
    check_has "$L §11.3" "$P113" "Exclusion from the producer set at the next epoch boundary"
    check_has "$L §11.3" "$P113" "No part of the burned bond goes to the reporter"
    check_has "$L §11.3" "$P113" "can never register again"
    check_lacks "$L §11.3" "$P113" "Immediate exclusion"
    check_lacks "$L §11.3" "$P113" "new registration required"
    check_has "$L §19.4" "$P194" "$NTYPES explicit transaction types"
    check_has "$L summary" "$PSUM" "one-step locked withdrawal"
    check_has "$L table" "$PROW" "60,480 blocks"
    for r in "two-step" "ClaimWithdrawal" "Immediate exclusion" "new registration required"; do
      check_lacks "$L file" "$(cat "$F" 2>/dev/null)" "$r"
    done
    ! grep -Eiq 'reporter (receives|is paid|gets|earns|is rewarded)|rewards? (goes |go )?to the reporter|reporter reward' "$F"
    res $? "$L file grants no reporter reward"
  else
    check_has "$L §6.1" "$P61" "firma del productor asignado"
    check_has "$L §7.4" "$P74A" "no hay un paso de reclamo separado"
    check_has "$L §7.4" "$P74A" "siguiente limite de epoch"
    check_has "$L §7.4" "$P74A" "60,480 bloques"
    check_has "$L §7" "$PAUTH" "\`Registration\`, un \`AddBond\` o un \`RequestWithdrawal\`"
    check_has "$L §11.3" "$P113" "incluido el bond retirado que sigue bloqueado"
    check_has "$L §11.3" "$P113" "Exclusion del conjunto de productores en el siguiente limite de epoch"
    check_has "$L §11.3" "$P113" "Ninguna parte del bond quemado va al denunciante"
    check_has "$L §11.3" "$P113" "nunca puede registrarse de nuevo"
    check_lacks "$L §11.3" "$P113" "Exclusion inmediata"
    check_lacks "$L §11.3" "$P113" "nuevo registro requerido"
    check_has "$L §19.4" "$P194" "$NTYPES tipos de transaccion explicitos"
    check_has "$L summary" "$PSUM" "retiro en un paso con bloqueo"
    check_has "$L table" "$PROW" "60,480 bloques"
    for r in "dos pasos" "ClaimWithdrawal" "Exclusion inmediata" "nuevo registro requerido"; do
      check_lacks "$L file" "$(cat "$F" 2>/dev/null)" "$r"
    done
    ! grep -Eiq 'denunciante (recibe|cobra|gana)|recompensa (para|al) (el )?denunciante' "$F"
    res $? "$L file grants no reporter reward"
  fi
}

TYPES="$ROOT/crates/core/src/transaction/types.rs"
VARIANTS="$(awk '/^pub enum TxType/ { f = 1 } f && /^}/ { exit } f' "$TYPES" | grep -cE '^    [A-Z][A-Za-z]* = [0-9]+,')"
TOMBS="$(awk '/^pub enum TxType/ { f = 1 } f && /^}/ { exit } f' "$TYPES" | grep -cE '^    ClaimWithdrawal = 9,')"
NTYPES=$((VARIANTS - TOMBS))
[ "$VARIANTS" -gt 0 ] && [ "$TOMBS" = 1 ]; res $? "code TxType accepted count read: $NTYPES (variants $VARIANTS, ClaimWithdrawal tombstone $TOMBS)"

check_lang EN "$EN"
check_lang ES "$ES"

MEM="${MEMORY_DB:-$ROOT/.omega/memory.db}"
if [ -f "$MEM" ]; then
  [ "$(sqlite3 "$MEM" "SELECT COUNT(*) FROM decisions WHERE domain='whitepaper/defense-rules' AND status='active' AND decision||rationale LIKE '%until%activation heights are pinned%' AND decision||rationale LIKE '%INC-I-236%INC-I-255%INC-I-257%INC-I-182%INC-I-259%';" 2>/dev/null)" -ge 1 ]
  res $? "memory has the whitepaper/defense-rules decision row"
else
  echo "SKIP memory decision row (no $MEM)"
fi
echo "SUMMARY fails=$FAILS"
[ "$FAILS" = 0 ]
