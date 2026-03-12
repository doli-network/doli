#!/usr/bin/env bash
# install-services.sh — Generate and install standardized systemd service files
#
# Usage (run locally, deploys via SSH):
#   scripts/install-services.sh mainnet    # Install all mainnet services on ai1+ai2
#   scripts/install-services.sh testnet    # Install all testnet services on ai1+ai2
#   scripts/install-services.sh all        # Both networks
#   scripts/install-services.sh validate   # Dry-run: show what would be installed
#
# This is the ONLY way to create/update service files. Never hand-edit.
set -euo pipefail

AI1="ilozada@72.60.228.233"
AI2="ilozada@187.124.95.188"
AI3="ilozada@187.124.148.93"

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

  cat <<EOF
[Unit]
Description=Doli ${net^} Producer N${node_num}
After=doli-${net}-seed.service
Wants=doli-${net}-seed.service

[Service]
Type=simple
User=ilozada
Group=doliadmin
ExecStart=${binary} \\
  --data-dir ${data_dir} \\
  run \\
  --producer \\
  --producer-key ${key_file} \\
  --p2p-port ${p2p} \\
  --rpc-port ${rpc} \\
  --metrics-port ${metrics} \\
  --bootstrap ${boot1} \\
  --bootstrap ${boot2} \\
  --yes --force-start
Restart=on-failure
RestartSec=10
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
    archive_flag="--archive-to ${archive_dir} \\\\"$'\n'"  "
  fi

  local bootstrap_flags=""
  if [[ -n "$boot1" ]]; then
    bootstrap_flags="--bootstrap ${boot1} \\\\"$'\n'"  --bootstrap ${boot2} \\\\"$'\n'"  "
  fi

  cat <<EOF
[Unit]
Description=Doli ${net^} Seed (Archive+Relay)
After=network.target

[Service]
Type=simple
User=ilozada
Group=doliadmin
ExecStart=${binary} \\
  --data-dir ${data_dir} \\
  run \\
  --rpc-bind 0.0.0.0 \\
  --relay-server \\
  --p2p-port ${p2p} \\
  --rpc-port ${rpc} \\
  --metrics-port ${metrics} \\
  ${archive_flag}${bootstrap_flags}--yes
Restart=on-failure
RestartSec=10
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

  echo "$content" | ssh -o ConnectTimeout=10 "$server" "sudo tee /etc/systemd/system/${service_name}.service > /dev/null"
  echo "  Installed ${service_name} on ${server}"
}

# ── Mainnet installation ───────────────────────────────────────────────
install_mainnet() {
  local dry_run="${1:-false}"
  echo "=== Mainnet Services ==="

  # Seed on ai1
  local svc
  svc=$(generate_seed_service "mainnet" "$MN_BINARY" "/mainnet/seed/data" \
    30300 8500 9000 "" "" "/var/log/doli/mainnet/seed.log" "/mainnet/seed/blocks")
  install_remote "$AI1" "doli-mainnet-seed" "$svc" "$dry_run"

  # Seed on ai2 (bootstraps from ai1 seed)
  svc=$(generate_seed_service "mainnet" "$MN_BINARY" "/mainnet/seed/data" \
    30300 8500 9000 "$MN_BOOTSTRAP_1" "" "/var/log/doli/mainnet/seed.log" "/mainnet/seed/blocks")
  install_remote "$AI2" "doli-mainnet-seed" "$svc" "$dry_run"

  # Producers: odd on ai1, even on ai2
  for N in $(seq 1 12); do
    local server
    if (( N % 2 == 1 )); then server="$AI1"; else server="$AI2"; fi

    local prefix="n"
    svc=$(generate_producer_service "mainnet" "$MN_BINARY" "$N" \
      "/mainnet/${prefix}${N}/data" "/mainnet/${prefix}${N}/keys/producer.json" \
      "$(mn_p2p_port $N)" "$(mn_rpc_port $N)" "$(mn_metrics_port $N)" \
      "$MN_BOOTSTRAP_1" "$MN_BOOTSTRAP_2" "/var/log/doli/mainnet/${prefix}${N}.log")
    install_remote "$server" "doli-mainnet-${prefix}${N}" "$svc" "$dry_run"
  done

  # Reload systemd on both servers
  if [[ "$dry_run" != "true" ]]; then
    ssh -o ConnectTimeout=10 "$AI1" "sudo systemctl daemon-reload" && echo "  daemon-reload on ai1"
    ssh -o ConnectTimeout=10 "$AI2" "sudo systemctl daemon-reload" && echo "  daemon-reload on ai2"
  fi
  echo ""
}

# ── Testnet installation ───────────────────────────────────────────────
install_testnet() {
  local dry_run="${1:-false}"
  echo "=== Testnet Services ==="

  # Seed on ai1
  local svc
  svc=$(generate_seed_service "testnet" "$TN_BINARY" "/testnet/seed/data" \
    40300 18500 19000 "" "" "/var/log/doli/testnet/seed.log" "/testnet/seed/blocks")
  install_remote "$AI1" "doli-testnet-seed" "$svc" "$dry_run"

  # Seed on ai2
  svc=$(generate_seed_service "testnet" "$TN_BINARY" "/testnet/seed/data" \
    40300 18500 19000 "$TN_BOOTSTRAP_1" "" "/var/log/doli/testnet/seed.log" "/testnet/seed/blocks")
  install_remote "$AI2" "doli-testnet-seed" "$svc" "$dry_run"

  # Producers: odd on ai1, even on ai2
  for N in $(seq 1 12); do
    local server
    if (( N % 2 == 1 )); then server="$AI1"; else server="$AI2"; fi

    local prefix="nt"
    svc=$(generate_producer_service "testnet" "$TN_BINARY" "$N" \
      "/testnet/${prefix}${N}/data" "/testnet/${prefix}${N}/keys/producer.json" \
      "$(tn_p2p_port $N)" "$(tn_rpc_port $N)" "$(tn_metrics_port $N)" \
      "$TN_BOOTSTRAP_1" "$TN_BOOTSTRAP_2" "/var/log/doli/testnet/${prefix}${N}.log")
    install_remote "$server" "doli-testnet-${prefix}${N}" "$svc" "$dry_run"
  done

  if [[ "$dry_run" != "true" ]]; then
    ssh -o ConnectTimeout=10 "$AI1" "sudo systemctl daemon-reload" && echo "  daemon-reload on ai1"
    ssh -o ConnectTimeout=10 "$AI2" "sudo systemctl daemon-reload" && echo "  daemon-reload on ai2"
  fi
  echo ""
}

# ── Main ───────────────────────────────────────────────────────────────
case "${1:-}" in
  mainnet)  install_mainnet ;;
  testnet)  install_testnet ;;
  all)      install_mainnet; install_testnet ;;
  validate) install_mainnet "true"; install_testnet "true" ;;
  *)        echo "Usage: $0 [mainnet|testnet|all|validate]"; exit 1 ;;
esac

echo "Done. Services installed but NOT restarted."
echo "To apply: restart nodes in order (seeds first, wait 10s, then producers)."
