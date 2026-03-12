# BUY_DOLI - DOLI/USDT Exchange System

Internal documentation for the automated DOLI/USDT exchange system at doli.network.

---

## Overview

The exchange system allows users to purchase DOLI tokens using USDT (Tether) on Ethereum. The system consists of:

1. **Frontend** - Web interface for creating orders (MetaMask integration)
2. **API** - REST endpoints for order management
3. **Bot** - Automated Ethereum watcher that detects payments and sends DOLI

---

## Architecture

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│   Frontend      │────▶│   API Server    │────▶│   SQLite DB     │
│   (buy.html)    │     │   (Express)     │     │   (swap.db)     │
└─────────────────┘     └────────┬────────┘     └─────────────────┘
                                 │
                    ┌────────────┴────────────┐
                    │                         │
              ┌─────▼─────┐           ┌───────▼───────┐
              │ Ethereum  │           │  DOLI Node    │
              │ (polling) │           │  (RPC)        │
              └───────────┘           └───────────────┘
```

---

## Pool Addresses

| Asset | Address | Description |
|-------|---------|-------------|
| ETH (USDT) | `0x4cc9Ea41Dec9bF5d7e38D6a216e861243d93bb6D` | Receives USDT from buyers |
| DOLI | `doli1grww5ppnvz2phncskf0zq5tq3hxc87c5` | Sends DOLI to buyers (producer_1) |

---

## Exchange Parameters

| Parameter | Value |
|-----------|-------|
| Exchange Rate | 1 USDT = 10 DOLI |
| Minimum Purchase | 0.10 USDT (1 DOLI) |
| Maximum Purchase | 10,000 USDT (100,000 DOLI) |
| Order Expiration | 30 minutes |
| Required Confirmations | 2 blocks |
| Polling Interval | 15 seconds |
| Max Pending Orders/User | 3 |

---

## User Flow

```
┌──────────────────────────────────────────────────────────────────┐
│                         BUYER FLOW                               │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  1. Visit doli.network/buy.html                                  │
│                                                                  │
│  2. Connect MetaMask                                             │
│     └─▶ Frontend obtains: 0xBuyer...                             │
│                                                                  │
│  3. Enter DOLI address                                           │
│     └─▶ doli1abc...                                              │
│                                                                  │
│  4. Select USDT amount                                           │
│     └─▶ 100 USDT = 1,000 DOLI                                    │
│                                                                  │
│  5. Click "Buy DOLI"                                             │
│     └─▶ POST /api/order creates order in DB                      │
│     └─▶ Returns pool ETH address                                 │
│                                                                  │
│  6. Send USDT via MetaMask                                       │
│     └─▶ 0xBuyer... ──▶ 0x4cc9Ea41...                             │
│                                                                  │
│  7. Bot detects payment (polling every 15s)                      │
│     └─▶ Waits for 2 confirmations                                │
│     └─▶ Matches payment to pending order                         │
│     └─▶ Sends DOLI: doli1grww5pp... ──▶ doli1abc...              │
│                                                                  │
│  8. Order completed                                              │
│     └─▶ Frontend polls /api/order/:id until status=completed     │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

---

## API Reference

Base URL: `https://doli.network`

### GET /health

Health check endpoint.

**Response:**
```json
{
    "status": "ok",
    "watching": true,
    "lastBlock": 24345157
}
```

| Field | Description |
|-------|-------------|
| `status` | Always "ok" if server is running |
| `watching` | true if Ethereum polling is active |
| `lastBlock` | Last processed Ethereum block number |

---

### GET /api/liquidity

Returns available liquidity and exchange parameters.

**Response:**
```json
{
    "doli": 50000,
    "rate": 10,
    "min_usdt": 0.1,
    "pool_eth": "0x4cc9Ea41Dec9bF5d7e38D6a216e861243d93bb6D"
}
```

| Field | Description |
|-------|-------------|
| `doli` | Available DOLI in pool |
| `rate` | DOLI per USDT (10 = 10 DOLI per 1 USDT) |
| `min_usdt` | Minimum USDT purchase |
| `pool_eth` | ETH address to receive USDT payments |

---

### GET /api/user/:eth_address

Check user's order history.

**Response:**
```json
{
    "first_time": true,
    "completed_orders": 0,
    "pending_orders": 0
}
```

| Field | Description |
|-------|-------------|
| `first_time` | true if user has no completed orders |
| `completed_orders` | Count of completed orders |
| `pending_orders` | Count of pending orders |

---

### POST /api/order

Create a new exchange order.

**Request:**
```json
{
    "eth_address": "0xBuyer...",
    "doli_address": "doli1abc...",
    "usdt_amount": 100
}
```

**Response (success):**
```json
{
    "success": true,
    "order_id": 123,
    "usdt_amount": 100,
    "doli_amount": 1000,
    "pool_address": "0x4cc9Ea41Dec9bF5d7e38D6a216e861243d93bb6D",
    "expires_at": "2026-01-30T04:30:00.000Z"
}
```

**Response (error):**
```json
{
    "error": "Invalid DOLI address"
}
```

| Error | Cause |
|-------|-------|
| "Invalid ETH address" | eth_address is not valid |
| "Invalid DOLI address" | doli_address doesn't start with "doli1" |
| "Minimum is 0.1 USDT" | usdt_amount < 0.10 |
| "Maximum is 10000 USDT" | usdt_amount > 10000 |
| "Too many pending orders" | User has 3+ pending orders |
| "Insufficient liquidity" | Pool doesn't have enough DOLI |

---

### GET /api/order/:id

Get order status.

**Response:**
```json
{
    "id": 123,
    "eth_address": "0xbuyer...",
    "doli_address": "doli1abc...",
    "usdt_amount": 100,
    "doli_amount": 1000,
    "status": "completed",
    "eth_tx_hash": "0xabc123...",
    "doli_tx_hash": "def456...",
    "created_at": "2026-01-30T04:00:00.000Z",
    "expires_at": "2026-01-30T04:30:00.000Z",
    "completed_at": "2026-01-30T04:05:00.000Z"
}
```

| Status | Description |
|--------|-------------|
| `pending` | Waiting for USDT payment |
| `completed` | DOLI sent successfully |
| `no_liquidity` | Failed due to insufficient pool balance |
| `doli_failed` | Failed to send DOLI |
| `eth_error` | Ethereum confirmation error |

---

### GET /api/admin/orders

List recent orders (admin endpoint).

**Response:**
```json
{
    "orders": [
        { "id": 123, "status": "completed", ... },
        { "id": 122, "status": "pending", ... }
    ]
}
```

---

## Database Schema

```sql
-- Orders table
CREATE TABLE orders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    eth_address TEXT NOT NULL,        -- Buyer's ETH address (lowercase)
    doli_address TEXT NOT NULL,       -- Buyer's DOLI address
    usdt_amount REAL NOT NULL,        -- USDT amount
    doli_amount REAL NOT NULL,        -- DOLI amount (usdt * rate)
    status TEXT DEFAULT 'pending',    -- Order status
    eth_tx_hash TEXT,                 -- USDT payment transaction hash
    doli_tx_hash TEXT,                -- DOLI send transaction hash
    error_message TEXT,               -- Error details if failed
    created_at TEXT,                  -- ISO timestamp
    expires_at TEXT,                  -- ISO timestamp (created + 30min)
    completed_at TEXT                 -- ISO timestamp when completed
);

CREATE INDEX idx_orders_eth ON orders(eth_address);
CREATE INDEX idx_orders_status ON orders(status);

-- Processed transactions (prevent double-processing)
CREATE TABLE processed_txs (
    tx_hash TEXT PRIMARY KEY,
    processed_at TEXT
);

-- State persistence (last processed block)
CREATE TABLE state (
    key TEXT PRIMARY KEY,
    value TEXT
);
```

---

## Bot Implementation

The bot runs **continuously as a background service**, independent of the frontend. It actively polls Ethereum every 15 seconds looking for USDT transfers to the pool address. The frontend only creates orders in the database; the bot handles payment detection and DOLI delivery automatically.

The bot uses **block polling** (not WebSocket events) for compatibility with public RPC endpoints.

### Polling Logic

```
1. Get current Ethereum block number
2. Calculate safe block (current - required_confirmations)
3. Query USDT Transfer events from last_processed_block to safe_block
4. For each transfer TO our pool:
   a. Check if already processed (processed_txs table)
   b. Find matching pending order by sender address
   c. Verify pool has sufficient DOLI
   d. Send DOLI to buyer's address
   e. Update order status to completed
5. Store safe_block as last_processed_block
6. Sleep 15 seconds
7. Repeat
```

### Key Design Decisions

1. **Polling vs WebSockets**: Public RPCs (PublicNode, Infura free) don't reliably support `eth_getFilterChanges`. Polling with `eth_getLogs` is more robust.

2. **Block Persistence**: Last processed block is stored in SQLite. On restart, the bot resumes from where it left off (minus a small buffer for safety).

3. **Confirmation Wait**: Bot only processes blocks with 2+ confirmations to avoid reorg issues.

4. **Idempotency**: Each ETH transaction hash is recorded in `processed_txs` to prevent double-processing.

---

## Production Deployment

### Server Location

```
Server: omegacortex.ai (axiomrx)
User: ilozada
```

### File Locations

```
/home/ilozada/repos/doli-swap-bot/
├── server.js           # Main application
├── swap.db             # SQLite database
├── swap.log            # Application logs
├── .env                # Configuration
├── package.json        # Dependencies
└── node_modules/       # Installed packages

/var/www/doli.network/
├── buy.html            # English frontend
└── espanol/
    └── comprar.html    # Spanish frontend
```

### Environment Variables (.env)

```bash
PORT=3000
RATE=10
MIN_USDT=0.10
ORDER_EXPIRY_MINUTES=30
REQUIRED_CONFIRMATIONS=2
POOL_ETH=0x4cc9Ea41Dec9bF5d7e38D6a216e861243d93bb6D
POOL_DOLI=doli1grww5ppnvz2phncskf0zq5tq3hxc87c5
ETH_RPC=https://ethereum.publicnode.com
DOLI_RPC=http://127.0.0.1:18500
DOLI_CLI=/home/ilozada/repos/doli/target/release/doli
DOLI_WALLET=/home/ilozada/.doli/testnet/producer_keys/producer_1.json
```

### Systemd Service

```ini
# /etc/systemd/system/doli-swap.service
[Unit]
Description=DOLI Swap Bot
After=network.target

[Service]
Type=simple
User=ilozada
WorkingDirectory=/home/ilozada/repos/doli-swap-bot
ExecStart=/usr/bin/node server.js
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
```

**Service Behavior:**
- **Auto-start on reboot**: Yes (`WantedBy=multi-user.target` + `enabled`)
- **Auto-restart on crash**: Yes (`Restart=on-failure`, waits 10s)
- **Runs continuously**: 24/7 active polling, not triggered by frontend

**Commands:**
```bash
sudo systemctl start doli-swap      # Start
sudo systemctl stop doli-swap       # Stop
sudo systemctl restart doli-swap    # Restart
sudo systemctl status doli-swap     # Status
sudo systemctl enable doli-swap     # Enable auto-start on boot
sudo systemctl disable doli-swap    # Disable auto-start on boot
journalctl -u doli-swap -f          # View logs
```

### Nginx Configuration

```nginx
# In /etc/nginx/sites-available/doli.network

# API proxy
location /api {
    proxy_pass http://127.0.0.1:3000;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
}

# Health check proxy
location /health {
    proxy_pass http://127.0.0.1:3000;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
}
```

---

## Security Considerations

### Rate Limiting

- **API**: 30 requests per minute per IP (express-rate-limit)
- **Orders**: Maximum 3 pending orders per ETH address
- **Helmet**: Security headers enabled

### Order Validation

- ETH address validated with ethers.js `isAddress()`
- DOLI address must start with "doli1"
- Amount bounds enforced (0.10 - 10,000 USDT)
- Liquidity checked before order creation AND before sending

### Transaction Safety

- 2 block confirmations required before processing
- Each ETH tx hash recorded to prevent double-processing
- Order expiration (30 min) prevents stale orders

### First-Time Buyers

Frontend shows warning for first-time buyers suggesting a test purchase of 1 DOLI to verify the address is correct.

---

## Monitoring

### Health Check

```bash
curl https://doli.network/health
# {"status":"ok","watching":true,"lastBlock":24345157}
```

If `watching: false`, the Ethereum connection failed. Check:
1. ETH_RPC endpoint is accessible
2. Service is running (`systemctl status doli-swap`)
3. Logs for errors (`journalctl -u doli-swap -n 50`)

### Liquidity Check

```bash
curl https://doli.network/api/liquidity
# {"doli":50000,"rate":10,"min_usdt":0.1,"pool_eth":"0x..."}
```

### Recent Orders

```bash
curl https://doli.network/api/admin/orders
```

### Logs

```bash
# Systemd logs
journalctl -u doli-swap -f

# Application log
tail -f /home/ilozada/repos/doli-swap-bot/swap.log
```

---

## Frontend Pages

| URL | Language |
|-----|----------|
| https://doli.network/buy.html | English |
| https://doli.network/espanol/comprar.html | Spanish |

Both pages:
- Use MetaMask for wallet connection
- Show real-time liquidity from API
- Poll order status until completion
- Display first-time buyer warning
- Match doli.network design aesthetic

---

## Dependencies

```json
{
  "dependencies": {
    "better-sqlite3": "^9.x",
    "cors": "^2.x",
    "dotenv": "^16.x",
    "ethers": "^6.x",
    "express": "^4.x",
    "express-rate-limit": "^7.x",
    "helmet": "^7.x"
  }
}
```

---

## Troubleshooting

### Bot not detecting payments

1. Check `watching` status: `curl https://doli.network/health`
2. Verify ETH_RPC is working: `curl -X POST $ETH_RPC -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'`
3. Check logs: `journalctl -u doli-swap -n 100`

### Order stuck in pending

1. Verify USDT was sent to correct pool address
2. Check sender address matches order's eth_address (case-insensitive)
3. Wait for 2+ confirmations (~30 seconds)
4. Check bot logs for errors

### DOLI not sent

1. Verify DOLI node is running on configured RPC port
2. Check wallet file exists and has balance
3. Test manually: `doli send <address> 1 --wallet <path> --rpc <url>`

### Database issues

```bash
# Backup
cp /home/ilozada/repos/doli-swap-bot/swap.db swap.db.backup

# Query directly
sqlite3 /home/ilozada/repos/doli-swap-bot/swap.db "SELECT * FROM orders ORDER BY id DESC LIMIT 10;"
```

---

## Contact

- **Website:** https://doli.network
- **Email:** weil@doli.network
- **GitHub:** https://github.com/e-weil/doli
