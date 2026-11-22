#!/usr/bin/env bash
# INC-I-308 send-back 1 (D3) — scripts/install-services.sh re-run keeps a unit's admin token.
# Decision: a FAIL means a re-run of the fleet installer drops the DOLI_RPC_ADMIN_TOKEN entry of an
# existing unit (the node restarts with a new random file token and every tool holding the old one is
# refused), adds an entry a unit never had, or prints the token.
# OUTPUT CONTRACT: rewritten unit text (token line present once, inside [Service], byte-equal) and
# installer stdout/stderr (no token) for `install-services.sh mainnet` and `validate`.
# INPUT PARTITIONS: old unit in {quoted Environment="...", unquoted Environment=..., no token line}.
# Runs the real installer with fake ssh/sudo/systemctl that act on a temp dir.
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 1
FAILS=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1${2:+ — $2}"; FAILS=$((FAILS + 1)); }

T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
mkdir -p "$T/fake" "$T/units" "$T/localbin"
cat > "$T/fake/ssh" <<EOF
#!/usr/bin/env bash
while [ \$# -gt 0 ] && [ "\${1#-}" != "\$1" ]; do shift 2; done
shift
cmd="\$*"; cmd="\${cmd//\/etc\/systemd\/system/$T/units}"; cmd="\${cmd//\/usr\/local\/bin/$T/localbin}"
exec bash -c "\$cmd"
EOF
printf '#!/usr/bin/env bash\nexec "$@"\n' > "$T/fake/sudo"
printf '#!/usr/bin/env bash\nexit 0\n' > "$T/fake/systemctl"
chmod +x "$T/fake/"*

TOK_Q="a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0"
TOK_U="d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0"
LINE_Q="Environment=\"DOLI_RPC_ADMIN_TOKEN=$TOK_Q\""
LINE_U="Environment=DOLI_RPC_ADMIN_TOKEN=$TOK_U"
old_unit() { printf '[Unit]\nDescription=old\n\n[Service]\nType=simple\n%s\nExecStart=/old/doli-node run\n\n[Install]\nWantedBy=multi-user.target\n' "$1"; }
old_unit "$LINE_Q" > "$T/units/doli-mainnet-n1.service"
old_unit "$LINE_U" > "$T/units/doli-mainnet-seed.service"
old_unit "" > "$T/units/doli-mainnet-n2.service"

export DOLI_AI1=u@h1 DOLI_AI2=u@h2 DOLI_AI3=u@h3 DOLI_AI4=u@h4 DOLI_AI5=u@h5
PATH="$T/fake:$PATH" bash scripts/install-services.sh mainnet > "$T/out" 2>&1
PATH="$T/fake:$PATH" bash scripts/install-services.sh validate >> "$T/out" 2>&1

# check <unit> <line|""> <label>
check() {
  local f="$T/units/$1.service" n svc
  grep -q 'StartLimitBurst' "$f" || { fail "$3 unit rewritten" "old content left in $1"; return; }
  n=$(grep -c 'DOLI_RPC_ADMIN_TOKEN' "$f")
  if [ -z "$2" ]; then
    [ "$n" = 0 ] && pass "$3 unit without a token gets none" || fail "$3 unit without a token gets none" "$n lines"
    return
  fi
  [ "$n" = 1 ] && grep -qxF "$2" "$f" && pass "$3 token entry carried byte-equal once" \
    || fail "$3 token entry carried byte-equal once" "count=$n"
  svc=$(awk '/^\[/{s=$0} s=="[Service]"' "$f")
  printf '%s\n' "$svc" | grep -qxF "$2" && pass "$3 token entry is inside [Service]" || fail "$3 token entry is inside [Service]"
}
check doli-mainnet-n1 "$LINE_Q" "a quoted"
check doli-mainnet-seed "$LINE_U" "b unquoted"
check doli-mainnet-n2 "" "c none"
if grep -qE "$TOK_Q|$TOK_U" "$T/out"; then fail "d installer output never shows a token"; else pass "d installer output never shows a token"; fi
grep -q 'MISMATCH.*doli-mainnet-n1 ' "$T/out" && fail "e validate ignores the carried token line" || pass "e validate ignores the carried token line"

echo "---"
[ "$FAILS" -eq 0 ] && echo "ALL PASS" || { echo "FAILS: $FAILS"; exit 1; }
