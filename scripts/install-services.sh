#!/usr/bin/env bash
# install-services.sh — Generate and install standardized systemd service files
#
# Usage (run locally, deploys via SSH):
#   scripts/install-services.sh mainnet    # Install all mainnet services on ai2+ai4+ai1+ai3
#   scripts/install-services.sh testnet    # Install all testnet services on ai1+ai5+ai3
#   scripts/install-services.sh all        # Both networks
#   scripts/install-services.sh validate   # Validate deployed service files match expected
#
# Architecture v6 (2026-03-15):
#   ai1 = testnet NT1-NT5 + seeds, ai2 = mainnet N1-N5 + seeds + build
#   ai3 = seeds only + SANTIAGO, ai4 = mainnet N6-N12, ai5 = testnet NT6-NT12
#
# This is the ONLY way to create/update service files. Never hand-edit.
set -euo pipefail

# Capitalize first letter (bash 3.x compat)
ucfirst() { echo "$(echo "${1:0:1}" | tr '[:lower:]' '[:upper:]')${1:1}"; }

AI1="ilozada@72.60.228.233"    # Testnet NT1-NT5 + seeds
AI2="ilozada@187.124.95.188"   # Mainnet N1-N5 + seeds + build
AI3="ilozada@187.124.148.93"   # Seeds only + SANTIAGO (SSH port 50790)
AI4="ilozada@204.168.150.118"  # Mainnet N6-N12
AI5="ilozada@46.62.156.244"    # Testnet NT6-NT12

# ── Mainnet config ──────────────────────────────────────────────────────
MN_BINARY="/mainnet/bin/doli-node"
MN_BOOTSTRAP_1="/dns4/seed1.doli.network/tcp/30300"
MN_BOOTSTRAP_2="/dns4/seed2.doli.network/tcp/30300"

mn_p2p_port()     { echo $((30300 + $1)); }
mn_rpc_port()     { echo $((8500  + $1)); }
mn_metrics_port() { echo $((9000  + $1)); }

# ── Testnet config ─────────────────────────────────────────────────────
TN_BINARY="/testnet/bin/doli-node"
TN_BOOTSTRAP_1="/dns4/bootstrap1.testnet.doli.network/tcp/40300"
TN_BOOTSTRAP_2="/dns4/bootstrap2.testnet.doli.network/tcp/40300"

tn_p2p_port()     { echo $((40300 + $1)); }
tn_rpc_port()     { echo $((18500 + $1)); }
tn_metrics_port() { echo $((19000 + $1)); }

# ── Service file template ──────────────────────────────────────────────
generate_producer_service() {
  local net="$1" binary="$2" node_num="$3" data_dir="$4" key_file="$5"
  local p2p="$6" rpc="$7" metrics="$8" boot1="$9" boot2="${10}" log_file="${11}"

  local prefix="N"
  [[ "$net" == "testnet" ]] && prefix="NT"

  cat <<EOF
[Unit]
Description=Doli $(ucfirst "$net") Producer ${prefix}${node_num}
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=ilozada
Group=doliadmin
ExecStart=${binary} \\
  --network ${net} \\
  --data-dir ${data_dir} \\
  run \\
  --producer \\
  --producer-key ${key_file} \\
  --p2p-port ${p2p} \\
  --rpc-port ${rpc} --rpc-bind 0.0.0.0 \\
  --metrics-port ${metrics} \\
  --bootstrap ${boot1} \\
  --bootstrap ${boot2} \\
  --yes --force-start
Restart=always
RestartSec=10
StartLimitIntervalSec=600
StartLimitBurst=5
StandardOutput=append:${log_file}
StandardError=append:${log_file}
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOF
}

generate_seed_service() {
  local net="$1" binary="$2" data_dir="$3" p2p="$4" rpc="$5" metrics="$6"
  local boot1="$7" boot2="$8" log_file="$9" archive_dir="${10:-}"

  local archive_flag=""
  if [[ -n "$archive_dir" ]]; then
    archive_flag="--archive-to ${archive_dir} \\"$'\n'"  "
  fi

  local bootstrap_flags=""
  if [[ -n "$boot1" ]]; then
    bootstrap_flags="--bootstrap ${boot1} \\"$'\n'"  "
    if [[ -n "$boot2" ]]; then
      bootstrap_flags+="--bootstrap ${boot2} \\"$'\n'"  "
    fi
  fi

  cat <<EOF
[Unit]
Description=Doli $(ucfirst "$net") Seed (Archive+Relay)
After=network.target

[Service]
Type=simple
User=ilozada
Group=doliadmin
ExecStart=${binary} \\
  --network ${net} \\
  --data-dir ${data_dir} \\
  run \\
  --rpc-bind 0.0.0.0 \\
  --relay-server \\
  --p2p-port ${p2p} \\
  --rpc-port ${rpc} \\
  --metrics-port ${metrics} \\
  ${archive_flag}${bootstrap_flags}--yes
Restart=always
RestartSec=10
StartLimitIntervalSec=600
StartLimitBurst=5
StandardOutput=append:${log_file}
StandardError=append:${log_file}
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOF
}

# ── Install a service on a remote server ───────────────────────────────
install_remote() {
  local server="$1" service_name="$2" content="$3" dry_run="$4"

  if [[ "$dry_run" == "true" ]]; then
    echo "  [DRY-RUN] Would install ${service_name} on ${server}"
    return
  fi

  echo "$content" | do_ssh "$server" "sudo tee /etc/systemd/system/${service_name}.service > /dev/null"
  echo "  Installed ${service_name} on ${server}"
}

# ── SSH wrapper (all servers use port 50790) ──────────────────────────
do_ssh() {
  local server="$1"; shift
  ssh -p 50790 -o ConnectTimeout=10 "$server" "$@"
}

reload_daemon() {
  local server="$1" dry_run="$2" label="$3"
  if [[ "$dry_run" == "true" ]]; then return; fi
  do_ssh "$server" "sudo systemctl daemon-reload" && echo "  daemon-reload on ${label}"
}

# ── Mainnet installation ───────────────────────────────────────────────
install_mainnet() {
  local dry_run="${1:-false}"
  echo "=== Mainnet Services ==="

  # Seed on ai1 (bootstraps from ai2 seed)
  local svc
  svc=$(generate_seed_service "mainnet" "$MN_BINARY" "/mainnet/seed/data" \
    30300 8500 9000 "$MN_BOOTSTRAP_1" "" "/var/log/doli/mainnet/seed.log" "/mainnet/seed/blocks")
  install_remote "$AI1" "doli-mainnet-seed" "$svc" "$dry_run"

  # Seed on ai2 (primary — no bootstrap needed)
  svc=$(generate_seed_service "mainnet" "$MN_BINARY" "/mainnet/seed/data" \
    30300 8500 9000 "" "" "/var/log/doli/mainnet/seed.log" "/mainnet/seed/blocks")
  install_remote "$AI2" "doli-mainnet-seed" "$svc" "$dry_run"

  # Seed on ai3 (bootstraps from ai2 seed)
  svc=$(generate_seed_service "mainnet" "$MN_BINARY" "/mainnet/seed/data" \
    30300 8500 9000 "$MN_BOOTSTRAP_1" "" "/var/log/doli/mainnet/seed.log" "/mainnet/seed/blocks")
  install_remote "$AI3" "doli-mainnet-seed" "$svc" "$dry_run"

  # N1-N5 on ai2
  for N in 1 2 3 4 5; do
    svc=$(generate_producer_service "mainnet" "$MN_BINARY" "$N" \
      "/mainnet/n${N}/data" "/mainnet/n${N}/keys/producer.json" \
      "$(mn_p2p_port $N)" "$(mn_rpc_port $N)" "$(mn_metrics_port $N)" \
      "$MN_BOOTSTRAP_1" "$MN_BOOTSTRAP_2" "/var/log/doli/mainnet/n${N}.log")
    install_remote "$AI2" "doli-mainnet-n${N}" "$svc" "$dry_run"
  done

  # N6-N12 on ai4
  for N in 6 7 8 9 10 11 12; do
    svc=$(generate_producer_service "mainnet" "$MN_BINARY" "$N" \
      "/mainnet/n${N}/data" "/mainnet/n${N}/keys/producer.json" \
      "$(mn_p2p_port $N)" "$(mn_rpc_port $N)" "$(mn_metrics_port $N)" \
      "$MN_BOOTSTRAP_1" "$MN_BOOTSTRAP_2" "/var/log/doli/mainnet/n${N}.log")
    install_remote "$AI4" "doli-mainnet-n${N}" "$svc" "$dry_run"
  done

  # Reload systemd
  reload_daemon "$AI1" "$dry_run" "ai1"
  reload_daemon "$AI2" "$dry_run" "ai2"
  reload_daemon "$AI3" "$dry_run" "ai3"
  reload_daemon "$AI4" "$dry_run" "ai4"
  echo ""
}

# ── Testnet installation ───────────────────────────────────────────────
install_testnet() {
  local dry_run="${1:-false}"
  echo "=== Testnet Services ==="

  # Seed on ai1 (primary — no bootstrap needed)
  local svc
  svc=$(generate_seed_service "testnet" "$TN_BINARY" "/testnet/seed/data" \
    40300 18500 19000 "" "" "/var/log/doli/testnet/seed.log" "/testnet/seed/blocks")
  install_remote "$AI1" "doli-testnet-seed" "$svc" "$dry_run"

  # Seed on ai2 (bootstraps from ai1 seed)
  svc=$(generate_seed_service "testnet" "$TN_BINARY" "/testnet/seed/data" \
    40300 18500 19000 "$TN_BOOTSTRAP_1" "" "/var/log/doli/testnet/seed.log" "/testnet/seed/blocks")
  install_remote "$AI2" "doli-testnet-seed" "$svc" "$dry_run"

  # Seed on ai3 (bootstraps from ai1 seed)
  svc=$(generate_seed_service "testnet" "$TN_BINARY" "/testnet/seed/data" \
    40300 18500 19000 "$TN_BOOTSTRAP_1" "" "/var/log/doli/testnet/seed.log" "/testnet/seed/blocks")
  install_remote "$AI3" "doli-testnet-seed" "$svc" "$dry_run"

  # NT1-NT5 on ai1
  for N in 1 2 3 4 5; do
    svc=$(generate_producer_service "testnet" "$TN_BINARY" "$N" \
      "/testnet/nt${N}/data" "/testnet/nt${N}/keys/producer.json" \
      "$(tn_p2p_port $N)" "$(tn_rpc_port $N)" "$(tn_metrics_port $N)" \
      "$TN_BOOTSTRAP_1" "$TN_BOOTSTRAP_2" "/var/log/doli/testnet/nt${N}.log")
    install_remote "$AI1" "doli-testnet-nt${N}" "$svc" "$dry_run"
  done

  # NT6-NT12 on ai5
  for N in 6 7 8 9 10 11 12; do
    svc=$(generate_producer_service "testnet" "$TN_BINARY" "$N" \
      "/testnet/nt${N}/data" "/testnet/nt${N}/keys/producer.json" \
      "$(tn_p2p_port $N)" "$(tn_rpc_port $N)" "$(tn_metrics_port $N)" \
      "$TN_BOOTSTRAP_1" "$TN_BOOTSTRAP_2" "/var/log/doli/testnet/nt${N}.log")
    install_remote "$AI5" "doli-testnet-nt${N}" "$svc" "$dry_run"
  done

  # Reload systemd
  reload_daemon "$AI1" "$dry_run" "ai1"
  reload_daemon "$AI2" "$dry_run" "ai2"
  reload_daemon "$AI3" "$dry_run" "ai3"
  reload_daemon "$AI5" "$dry_run" "ai5"
  echo ""
}

# ── Validate deployed vs expected ─────────────────────────────────────
VALIDATE_ERRORS=0

validate_service() {
  local server="$1" service_name="$2" expected="$3" label="$4"

  local deployed
  deployed=$(do_ssh "$server" "cat /etc/systemd/system/${service_name}.service 2>/dev/null") || deployed=""

  if [[ -z "$deployed" ]]; then
    echo -e "  \033[0;31mMISSING\033[0m ${label} — ${service_name} not found on ${server}"
    VALIDATE_ERRORS=$((VALIDATE_ERRORS+1))
    return
  fi

  local diff_output
  diff_output=$(diff <(echo "$expected") <(echo "$deployed") 2>/dev/null) || true

  if [[ -z "$diff_output" ]]; then
    echo -e "  \033[0;32mOK\033[0m ${label}"
  else
    echo -e "  \033[0;31mMISMATCH\033[0m ${label} — ${service_name} on ${server}"
    echo "$diff_output" | head -20 | sed 's/^/    /'
    VALIDATE_ERRORS=$((VALIDATE_ERRORS+1))
  fi
}

validate_mainnet() {
  echo "=== Validating Mainnet Services ==="
  local svc

  # Seeds on ai1, ai2, ai3
  svc=$(generate_seed_service "mainnet" "$MN_BINARY" "/mainnet/seed/data" \
    30300 8500 9000 "$MN_BOOTSTRAP_1" "" "/var/log/doli/mainnet/seed.log" "/mainnet/seed/blocks")
  validate_service "$AI1" "doli-mainnet-seed" "$svc" "Seed1 (ai1)"

  svc=$(generate_seed_service "mainnet" "$MN_BINARY" "/mainnet/seed/data" \
    30300 8500 9000 "" "" "/var/log/doli/mainnet/seed.log" "/mainnet/seed/blocks")
  validate_service "$AI2" "doli-mainnet-seed" "$svc" "Seed2 (ai2)"

  svc=$(generate_seed_service "mainnet" "$MN_BINARY" "/mainnet/seed/data" \
    30300 8500 9000 "$MN_BOOTSTRAP_1" "" "/var/log/doli/mainnet/seed.log" "/mainnet/seed/blocks")
  validate_service "$AI3" "doli-mainnet-seed" "$svc" "Seed3 (ai3)"

  # N1-N5 on ai2
  for N in 1 2 3 4 5; do
    svc=$(generate_producer_service "mainnet" "$MN_BINARY" "$N" \
      "/mainnet/n${N}/data" "/mainnet/n${N}/keys/producer.json" \
      "$(mn_p2p_port $N)" "$(mn_rpc_port $N)" "$(mn_metrics_port $N)" \
      "$MN_BOOTSTRAP_1" "$MN_BOOTSTRAP_2" "/var/log/doli/mainnet/n${N}.log")
    validate_service "$AI2" "doli-mainnet-n${N}" "$svc" "N${N} (ai2)"
  done

  # N6-N12 on ai4
  for N in 6 7 8 9 10 11 12; do
    svc=$(generate_producer_service "mainnet" "$MN_BINARY" "$N" \
      "/mainnet/n${N}/data" "/mainnet/n${N}/keys/producer.json" \
      "$(mn_p2p_port $N)" "$(mn_rpc_port $N)" "$(mn_metrics_port $N)" \
      "$MN_BOOTSTRAP_1" "$MN_BOOTSTRAP_2" "/var/log/doli/mainnet/n${N}.log")
    validate_service "$AI4" "doli-mainnet-n${N}" "$svc" "N${N} (ai4)"
  done
  echo ""
}

validate_testnet() {
  echo "=== Validating Testnet Services ==="
  local svc

  # Seeds on ai1, ai2, ai3
  svc=$(generate_seed_service "testnet" "$TN_BINARY" "/testnet/seed/data" \
    40300 18500 19000 "" "" "/var/log/doli/testnet/seed.log" "/testnet/seed/blocks")
  validate_service "$AI1" "doli-testnet-seed" "$svc" "SeedT1 (ai1)"

  svc=$(generate_seed_service "testnet" "$TN_BINARY" "/testnet/seed/data" \
    40300 18500 19000 "$TN_BOOTSTRAP_1" "" "/var/log/doli/testnet/seed.log" "/testnet/seed/blocks")
  validate_service "$AI2" "doli-testnet-seed" "$svc" "SeedT2 (ai2)"

  svc=$(generate_seed_service "testnet" "$TN_BINARY" "/testnet/seed/data" \
    40300 18500 19000 "$TN_BOOTSTRAP_1" "" "/var/log/doli/testnet/seed.log" "/testnet/seed/blocks")
  validate_service "$AI3" "doli-testnet-seed" "$svc" "SeedT3 (ai3)"

  # NT1-NT5 on ai1
  for N in 1 2 3 4 5; do
    svc=$(generate_producer_service "testnet" "$TN_BINARY" "$N" \
      "/testnet/nt${N}/data" "/testnet/nt${N}/keys/producer.json" \
      "$(tn_p2p_port $N)" "$(tn_rpc_port $N)" "$(tn_metrics_port $N)" \
      "$TN_BOOTSTRAP_1" "$TN_BOOTSTRAP_2" "/var/log/doli/testnet/nt${N}.log")
    validate_service "$AI1" "doli-testnet-nt${N}" "$svc" "NT${N} (ai1)"
  done

  # NT6-NT12 on ai5
  for N in 6 7 8 9 10 11 12; do
    svc=$(generate_producer_service "testnet" "$TN_BINARY" "$N" \
      "/testnet/nt${N}/data" "/testnet/nt${N}/keys/producer.json" \
      "$(tn_p2p_port $N)" "$(tn_rpc_port $N)" "$(tn_metrics_port $N)" \
      "$TN_BOOTSTRAP_1" "$TN_BOOTSTRAP_2" "/var/log/doli/testnet/nt${N}.log")
    validate_service "$AI5" "doli-testnet-nt${N}" "$svc" "NT${N} (ai5)"
  done
  echo ""
}

# ── Main ───────────────────────────────────────────────────────────────
case "${1:-}" in
  mainnet)  install_mainnet ;;
  testnet)  install_testnet ;;
  all)      install_mainnet; install_testnet ;;
  validate)
    validate_mainnet; validate_testnet
    echo "============================="
    if (( VALIDATE_ERRORS > 0 )); then
      echo -e "\033[0;31mFAILED: ${VALIDATE_ERRORS} service file(s) differ from expected\033[0m"
      echo "Run '$0 all' to regenerate all service files."
      exit 1
    else
      echo -e "\033[0;32mALL SERVICE FILES MATCH\033[0m"
      exit 0
    fi
    ;;
  *)        echo "Usage: $0 [mainnet|testnet|all|validate]"; exit 1 ;;
esac

echo "Done. Services installed but NOT restarted."
echo "To apply: restart nodes in order (seeds first, wait 10s, then producers)."

# ── Install watchdog on all servers ───────────────────────────────────
install_watchdog() {
  local server="$1" label="$2"

  # Copy watchdog script
  cat "$(dirname "$0")/doli-watchdog.sh" | do_ssh "$server" "sudo tee /usr/local/bin/doli-watchdog.sh > /dev/null && sudo chmod +x /usr/local/bin/doli-watchdog.sh"

  # Install timer + service
  do_ssh "$server" "sudo tee /etc/systemd/system/doli-watchdog.service > /dev/null" <<'WDSVC'
[Unit]
Description=Doli Node Watchdog — RPC health check
After=network.target

[Service]
Type=oneshot
ExecStart=/usr/local/bin/doli-watchdog.sh
WDSVC

  do_ssh "$server" "sudo tee /etc/systemd/system/doli-watchdog.timer > /dev/null" <<'WDTIMER'
[Unit]
Description=Run Doli Watchdog every 2 minutes

[Timer]
OnBootSec=120
OnUnitActiveSec=120
AccuracySec=10

[Install]
WantedBy=timers.target
WDTIMER

  do_ssh "$server" "sudo systemctl daemon-reload && sudo systemctl enable --now doli-watchdog.timer"
  echo "  Watchdog installed and enabled on ${label}"
}

if [[ "${1:-}" != "validate" ]]; then
  echo ""
  echo "=== Installing Watchdog ==="
  case "${1:-}" in
    mainnet)
      install_watchdog "$AI2" "ai2"
      install_watchdog "$AI3" "ai3"
      install_watchdog "$AI4" "ai4"
      ;;
    testnet)
      install_watchdog "$AI1" "ai1"
      install_watchdog "$AI3" "ai3"
      install_watchdog "$AI5" "ai5"
      ;;
    all)
      for srv_pair in "$AI1:ai1" "$AI2:ai2" "$AI3:ai3" "$AI4:ai4" "$AI5:ai5"; do
        install_watchdog "${srv_pair%%:*}" "${srv_pair##*:}"
      done
      ;;
  esac
fi
