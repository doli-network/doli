#!/usr/bin/env bash
# gauntlet-gs061.sh — GS-061 rpc-bind-and-token-census (INC-I-308; AUDIT-P1-RPC-003,
# AUDIT-P2-RPC-003, AUDIT-P3-RPC-005).
#
# Usage: bash scripts/gauntlet-gs061.sh
# OBSERVATIONAL. Reads ps, lsof, token FILE existence/length (never the value), the live launchd
# plists (~/Library/LaunchAgents/*doli*) and the repo installers. Sends no RPC, writes nothing.
#   gs061-proc-bind-token    per running doli-node: --rpc-bind, real listen address, token source
#                            (env | file | none) and length. FAIL: a non-loopback listen with no token.
#   gs061-no-token-on-argv   no running doli-node has the token (env-var form, flag form, or the
#                            token-file value) on its argv
#   gs061-units-no-token     no launchd plist and no repo installer carries a literal token value
#                            (DOLI_RPC_ADMIN_TOKEN=<value>, not a $variable). Their --rpc-bind values
#                            are listed; a non-loopback bind there is INFO: from INC-I-308 M1 the
#                            node creates <data_dir>/rpc-admin.token itself, so no unit runs tokenless.
set -uo pipefail
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/gauntlet-rpc-admin-lib.sh"
FAILS=""; fail(){ FAILS="$FAILS; $*"; echo "  ✗ $*"; }
echo "GS-061 rpc-bind-and-token-census"
PIDS=$(ga_fleet_pids)
N=0
for pid in $PIDS; do
  argv=$(ga_argv "$pid"); [ -n "$argv" ] || continue
  rpc=$(ga_flag "$argv" --rpc-port); bind=$(ga_flag "$argv" --rpc-bind); dd=$(ga_flag "$argv" --data-dir)
  N=$((N+1))
  addrs=""; [ -n "$rpc" ] && addrs=$(ga_listen_addrs "$pid" "$rpc" | tr '\n' ' ')
  src="none"; len=0
  [ -n "$(ga_env_has_token "$pid")" ] && src="env"
  if [ "$src" = none ] && [ -n "$dd" ] && [ -f "$dd/rpc-admin.token" ]; then
    len=$(tr -d '[:space:]' < "$dd/rpc-admin.token" 2>/dev/null | wc -c | tr -d ' ')
    [ "${len:-0}" -ge 32 ] && src="file"
  fi
  onargv=no
  printf '%s' "$argv" | grep -Eq 'DOLI_RPC_ADMIN_TOKEN=|--rpc-admin-token|--admin-token' && onargv=yes
  if [ "$src" = file ]; then
    t=$(tr -d '[:space:]' < "$dd/rpc-admin.token"); printf '%s' "$argv" | grep -qF -- "$t" && onargv=yes; t=""
  fi
  nonloop=""
  for a in $addrs; do ga_is_loopback "$a" || nonloop="$nonloop $a"; done
  ga_metric proc pid=$pid rpc_bind=${bind:-default} listen="${addrs% }" token_src=$src token_len=$len token_on_argv=$onargv
  [ -n "$nonloop" ] && [ "$src" = none ] && fail "gs061-proc-bind-token: pid=$pid listen=$nonloop token=none"
  [ "$onargv" = yes ] && fail "gs061-no-token-on-argv: pid=$pid"
done
ga_metric processes=$N

# Matches `X=<value>` (unit/env/shell) and `<key>X</key> <string><value>` (plist, newlines folded).
lit='DOLI_RPC_ADMIN_TOKEN(</key>)?[=[:space:]"'"'"']*(<string>)?[A-Za-z0-9_+/-]{32,}'
for f in "$HOME"/Library/LaunchAgents/*doli*.plist; do
  [ -f "$f" ] || continue
  bind=$(awk '/<string>--rpc-bind<\/string>/{getline; gsub(/.*<string>|<\/string>.*/,""); print; exit}' "$f")
  hastok=no; tr '\n' ' ' < "$f" | grep -Eq "$lit" && hastok=yes
  ga_metric plist "$(basename "$f")" rpc_bind=${bind:-default} literal_token=$hastok
  [ "$hastok" = yes ] && fail "gs061-units-no-token: $f carries a literal token"
done
for f in scripts/install-services.sh scripts/install-local-services.sh scripts/install.ps1 \
         testnetlinux/scripts/install-services.sh scripts/inc-i-300-repair-restart.sh; do
  [ -f "$GA_ROOT/$f" ] || continue
  binds=$(grep -oE -- '--rpc-bind[^A-Za-z0-9]*[0-9.:a-z]+' "$GA_ROOT/$f" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+|localhost' | sort | uniq -c | awk '{printf "%s×%s ",$2,$1}')
  [ -z "$binds" ] && grep -q -- '<string>--rpc-bind</string>' "$GA_ROOT/$f" && binds="plist-template"
  hastok=no; grep -Eq "$lit" "$GA_ROOT/$f" && hastok=yes
  ga_metric installer "$f" rpc_bind="${binds:-none}" literal_token=$hastok
  [ "$hastok" = yes ] && fail "gs061-units-no-token: $f carries a literal token"
done
[ "$N" -ge 1 ] || { echo "VERDICT: NOT_MADE (no running doli-node)"; exit 3; }
if [ -z "$FAILS" ]; then echo "VERDICT: PASS"; exit 0; fi
echo "VERDICT: FAIL$FAILS"; exit 1
