# DOLI Node - Docker Guide

Run a DOLI node using Docker for easy deployment and management.

---

## Quick Start

### Pull and Run (Recommended)

```bash
# Pull the latest image
docker pull ghcr.io/e-weil/doli-node:latest

# Run a mainnet node
docker run -d \
  --name doli-node \
  -p 30303:30303 \
  -p 8545:8545 \
  -v doli-data:/data \
  ghcr.io/e-weil/doli-node:latest
```

### Build and Run Locally

```bash
# Clone the repository
git clone https://github.com/e-weil/doli.git
cd doli

# Build the image
docker build -t doli-node .

# Run a mainnet node
docker run -d \
  --name doli-node \
  -p 30303:30303 \
  -p 8545:8545 \
  -v doli-data:/data \
  doli-node
```

---

## Docker Compose

For production deployments, use Docker Compose:

```bash
# Start mainnet node
docker compose up -d

# Start testnet node
docker compose -f docker-compose.testnet.yml up -d

# Start devnet node (local development)
docker compose -f docker-compose.devnet.yml up -d

# View logs
docker compose logs -f

# Stop node
docker compose down
```

### With Monitoring Stack

Run with Prometheus and Grafana for metrics visualization:

```bash
# Start node with monitoring
docker compose --profile monitoring up -d

# Access Grafana at http://localhost:3000
# Default credentials: admin / doli
```

---

## Configuration

### Environment Variables

Configure the node using environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `DOLI_NETWORK` | Network to join: `mainnet`, `testnet`, `devnet` | `mainnet` |
| `DOLI_DATA_DIR` | Data directory inside container | `/data` |
| `DOLI_LOG_LEVEL` | Log level: `error`, `warn`, `info`, `debug`, `trace` | `info` |
| `DOLI_P2P_PORT` | Override P2P listen port | Network default |
| `DOLI_RPC_PORT` | Override RPC listen port | Network default |
| `DOLI_METRICS_PORT` | Metrics server port | `9090` |
| `DOLI_BOOTSTRAP` | Bootstrap node multiaddr | Network defaults |
| `DOLI_PRODUCER` | Enable producer mode: `true` | `false` |
| `DOLI_PRODUCER_KEY_FILE` | Path to producer key file (enables producer mode) | None |
| `DOLI_NO_AUTO_UPDATE` | Disable auto-updates: `true` | `false` |
| `DOLI_NO_DHT` | Disable DHT discovery: `true` | `false` |
| `DOLI_CHAINSPEC` | Path to custom chainspec JSON | None |

### Network Ports

| Network | P2P Port | RPC Port | Metrics |
|---------|----------|----------|---------|
| Mainnet | 30303 | 8545 | 9090 |
| Testnet | 40303 | 18545 | 9090 |
| Devnet | 50303 | 28545 | 9090 |

### Example: Custom Configuration

```bash
docker run -d \
  --name doli-testnet \
  -e DOLI_NETWORK=testnet \
  -e DOLI_LOG_LEVEL=debug \
  -e DOLI_EXTERNAL_IP=203.0.113.50 \
  -p 40303:40303 \
  -p 18545:18545 \
  -v doli-testnet-data:/data \
  doli-node
```

---

## Volume Management

### Data Persistence

Blockchain data is stored in `/data` inside the container. Mount a volume to persist data:

```bash
# Named volume (recommended)
docker run -v doli-data:/data doli-node

# Host directory
docker run -v /path/to/data:/data doli-node
```

### Backup and Restore

```bash
# Backup
docker run --rm \
  -v doli-data:/data \
  -v $(pwd):/backup \
  alpine tar czf /backup/doli-backup.tar.gz -C /data .

# Restore
docker run --rm \
  -v doli-data:/data \
  -v $(pwd):/backup \
  alpine tar xzf /backup/doli-backup.tar.gz -C /data
```

### Clean Start

```bash
# Remove container and data
docker compose down -v

# Or manually
docker stop doli-node
docker rm doli-node
docker volume rm doli-data
```

---

## Running a Producer Node

To run a block producer, you need to provide your producer key.

### Option 1: Key File (Recommended)

```bash
# Create keys directory
mkdir -p keys

# Generate a key (using doli-cli)
doli-cli wallet new --output keys/producer.key

# Run with key file mounted
docker run -d \
  --name doli-producer \
  -e DOLI_PRODUCER_KEY_FILE=/keys/producer.key \
  -v $(pwd)/keys:/keys:ro \
  -v doli-data:/data \
  -p 30303:30303 \
  -p 8545:8545 \
  doli-node
```

### Option 2: Environment Variable

```bash
# WARNING: Less secure - key visible in process list and docker inspect
docker run -d \
  --name doli-producer \
  -e DOLI_PRODUCER_KEY=your_private_key_hex \
  -v doli-data:/data \
  -p 30303:30303 \
  -p 8545:8545 \
  doli-node
```

### Docker Compose Producer

Edit `docker-compose.yml`:

```yaml
services:
  doli-node:
    environment:
      - DOLI_PRODUCER_KEY_FILE=/keys/producer.key
    volumes:
      - doli-mainnet-data:/data
      - ./keys:/keys:ro
```

---

## Networking

### Port Exposure

For full network participation, expose the P2P port:

```bash
# Required for inbound connections
-p 30303:30303

# Optional: RPC (only if needed externally)
-p 8545:8545

# Optional: Metrics
-p 9090:9090
```

### Firewall Configuration

```bash
# UFW (Ubuntu)
sudo ufw allow 30303/tcp

# firewalld (RHEL/CentOS)
sudo firewall-cmd --permanent --add-port=30303/tcp
sudo firewall-cmd --reload
```

### Behind NAT

If running behind NAT, set your external IP:

```bash
docker run -d \
  -e DOLI_EXTERNAL_IP=your.public.ip.address \
  -p 30303:30303 \
  doli-node
```

---

## Monitoring

### Health Check

The container includes a built-in health check. Check status:

```bash
docker inspect --format='{{.State.Health.Status}}' doli-node
```

### View Logs

```bash
# All logs
docker logs doli-node

# Follow logs
docker logs -f doli-node

# Last 100 lines
docker logs --tail 100 doli-node
```

### RPC Queries

```bash
# Get chain info
curl -s http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}'

# Get peer count
curl -s http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getPeerCount","params":[],"id":1}'
```

### Prometheus Metrics

Metrics are exposed at `http://localhost:9090/metrics`. Start with monitoring:

```bash
docker compose --profile monitoring up -d
# Access Grafana at http://localhost:3000
```

---

## Troubleshooting

### Container Won't Start

```bash
# Check logs for errors
docker logs doli-node

# Common issues:
# - Port already in use: Change port mapping
# - Permission denied: Check volume permissions
# - Out of memory: Increase Docker memory limit
```

### Node Not Syncing

```bash
# Check peer connections
curl -s localhost:8545 -d '{"jsonrpc":"2.0","method":"getPeerCount","params":[],"id":1}'

# Verify P2P port is accessible
nc -zv your-ip 30303

# Check firewall rules
sudo iptables -L -n | grep 30303
```

### High Memory Usage

The node uses RocksDB which can consume significant memory. Adjust limits:

```yaml
# docker-compose.yml
deploy:
  resources:
    limits:
      memory: 4G
    reservations:
      memory: 1G
```

### Data Corruption

If you experience data corruption:

```bash
# Stop the container
docker stop doli-node

# Remove and recreate
docker rm doli-node
docker volume rm doli-data

# Start fresh
docker compose up -d
```

---

## Upgrading

### Standard Upgrade

```bash
# Pull latest image
docker pull ghcr.io/e-weil/doli-node:latest

# Recreate container with new image
docker compose up -d --force-recreate
```

### Specific Version

```bash
# Pull specific version
docker pull ghcr.io/e-weil/doli-node:v1.2.0

# Update compose file or run directly
docker run -d \
  --name doli-node \
  ... \
  ghcr.io/e-weil/doli-node:v1.2.0
```

---

## Security Best Practices

1. **Don't expose RPC publicly** - Only expose port 8545 if needed, and use a reverse proxy with authentication
2. **Use key files** - Never pass keys via environment variables in production
3. **Run as non-root** - The container runs as user `doli` (UID 1000) by default
4. **Keep updated** - Regularly pull the latest image for security fixes
5. **Limit resources** - Use Docker's resource limits to prevent DoS

---

## Command Reference

```bash
# Start node
docker compose up -d

# Stop node
docker compose down

# View logs
docker compose logs -f

# Check status
docker compose ps

# Restart node
docker compose restart

# Update to latest
docker compose pull && docker compose up -d

# Shell into container
docker compose exec doli-node bash

# Run CLI commands
docker compose exec doli-node doli-node info
```

---

## See Also

- [Running a Node](./running_a_node.md) - General node operation guide
- [Becoming a Producer](./becoming_a_producer.md) - Producer setup guide
- [RPC Reference](./rpc_reference.md) - API documentation
- [Troubleshooting](./troubleshooting.md) - Common issues and solutions
