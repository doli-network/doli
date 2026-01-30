# BUY_DOLI.md

Sistema de intercambio DOLI/USDT para doli.network.

---

## Pool Addresses

| Asset | Address | Description |
|-------|---------|-------------|
| ETH (USDT) | `0x4cc9Ea41Dec9bF5d7e38D6a216e861243d93bb6D` | Recibe USDT de compradores |
| DOLI | `doli1grww5ppnvz2phncskf0zq5tq3hxc87c5` | Envía DOLI a compradores (producer_1) |

---

## Exchange Rate

```
1 USDT = 10 DOLI
```

Mínimo: 0.10 USDT (1 DOLI)

---

## User Flow

```
┌─────────────────────────────────────────────────────────────┐
│                    COMPRADOR                                │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  1. Visita doli.network/buy.html                            │
│                                                             │
│  2. Conecta MetaMask                                        │
│     → Obtenemos: 0xComprador...                             │
│                                                             │
│  3. Ingresa su dirección DOLI                               │
│     → doli1abc...                                           │
│                                                             │
│  4. Selecciona cantidad USDT                                │
│     → 100 USDT = 1,000 DOLI                                 │
│                                                             │
│  5. Click "Buy DOLI"                                        │
│     → Se crea orden en DB                                   │
│     → Se muestra dirección del pool ETH                     │
│                                                             │
│  6. Envía USDT desde MetaMask                               │
│     → 0xComprador... → 0x4cc9Ea41...                        │
│                                                             │
│  7. Bot detecta el pago                                     │
│     → Verifica orden en DB                                  │
│     → Envía DOLI: doli1grww5pp... → doli1abc...             │
│                                                             │
│  8. Comprador recibe DOLI ✅                                │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

---

## Database Schema

```sql
CREATE TABLE orders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    eth_address TEXT NOT NULL,        -- 0xComprador...
    doli_address TEXT NOT NULL,       -- doli1abc...
    usdt_amount INTEGER NOT NULL,     -- En centavos (100 = 1 USDT)
    doli_amount INTEGER NOT NULL,     -- En unidades
    status TEXT DEFAULT pending,    -- pending, completed, expired, failed
    eth_tx_hash TEXT,                 -- TX de pago USDT
    doli_tx_hash TEXT,                -- TX de envío DOLI
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP,             -- created_at + 30 min
    completed_at TIMESTAMP
);

CREATE INDEX idx_orders_eth ON orders(eth_address);
CREATE INDEX idx_orders_status ON orders(status);
```

---

## Bot Logic

```javascript
// Configuración
const POOL_ETH = "0x4cc9Ea41Dec9bF5d7e38D6a216e861243d93bb6D";
const POOL_DOLI = "doli1grww5ppnvz2phncskf0zq5tq3hxc87c5";
const RATE = 10; // 1 USDT = 10 DOLI

// Escuchar transferencias USDT al pool
usdt.on(Transfer, async (from, to, amount) => {
    if (to.toLowerCase() !== POOL_ETH.toLowerCase()) return;
    
    // Buscar orden pendiente
    const order = await db.get(`
        SELECT * FROM orders 
        WHERE eth_address = ? 
        AND status = pending
        AND expires_at > datetime(now)
        ORDER BY created_at DESC
        LIMIT 1
    `, [from]);
    
    if (!order) {
        console.log(`⚠️ Pago sin orden: ${from} → ${amount} USDT`);
        return;
    }
    
    // Verificar liquidez
    const balance = await getDoliBalance(POOL_DOLI);
    if (balance < order.doli_amount) {
        await db.run(`UPDATE orders SET status = no_liquidity WHERE id = ?`, [order.id]);
        console.log(`❌ Sin liquidez para orden ${order.id}`);
        return;
    }
    
    // Enviar DOLI
    const txHash = await sendDoli(POOL_DOLI, order.doli_address, order.doli_amount);
    
    // Actualizar orden
    await db.run(`
        UPDATE orders 
        SET status = completed, 
            eth_tx_hash = ?,
            doli_tx_hash = ?,
            completed_at = datetime(now)
        WHERE id = ?
    `, [txHash, doliTxHash, order.id]);
    
    console.log(`✅ Orden ${order.id}: ${order.doli_amount} DOLI → ${order.doli_address}`);
});
```

---

## API Endpoints

### GET /api/liquidity

Retorna liquidez disponible.

```json
{
    "doli": 50000,
    "rate": 10
}
```

### GET /api/user/:eth_address

Verifica si es primera compra.

```json
{
    "first_time": true,
    "completed_orders": 0
}
```

### POST /api/order

Crea nueva orden.

**Request:**
```json
{
    "eth_address": "0xComprador...",
    "doli_address": "doli1abc...",
    "usdt_amount": 100
}
```

**Response:**
```json
{
    "success": true,
    "order_id": 123,
    "pool_address": "0x4cc9Ea41Dec9bF5d7e38D6a216e861243d93bb6D",
    "expires_at": "2026-01-30T03:30:00Z"
}
```

### GET /api/order/:id

Estado de una orden.

```json
{
    "id": 123,
    "status": "completed",
    "doli_tx_hash": "abc123..."
}
```

---

## Frontend Pages

| URL | Language | File |
|-----|----------|------|
| /buy.html | English | `buy.html` |
| /espanol/comprar.html | Spanish | `espanol/comprar.html` |

---

## Security Considerations

1. **Rate Limiting**
   - Máx 3 órdenes pendientes por ETH address
   - Cooldown de 1 minuto entre órdenes

2. **Order Expiration**
   - Órdenes expiran en 30 minutos
   - Cron job limpia órdenes expiradas

3. **Confirmations**
   - Esperar 2-3 confirmaciones ETH antes de enviar DOLI

4. **First-Time Warning**
   - Sugerir compra de prueba de 1 DOLI
   - Verificar dirección antes de montos grandes

5. **Liquidity Check**
   - Verificar antes de crear orden
   - Verificar antes de enviar DOLI

---

## Files Structure

```
doli.network/
├── buy.html              # Frontend English
├── espanol/
│   └── comprar.html      # Frontend Spanish
└── BUY_DOLI.md           # This document

doli-swap-bot/ (backend)
├── server.js             # API + Bot
├── db.sqlite             # Database
├── .env                  # Private keys
└── package.json
```

---

## Environment Variables (.env)

```bash
# Ethereum
ETH_RPC=https://mainnet.infura.io/v3/YOUR_KEY
ETH_PRIVATE_KEY=xxx  # Para monitorear, no necesita enviar

# DOLI
DOLI_RPC=http://127.0.0.1:8545
DOLI_WALLET_PATH=/path/to/producer_1.json

# USDT Contract (Ethereum Mainnet)
USDT_CONTRACT=0xdAC17F958D2ee523a2206206994597C13D831ec7

# Pool Addresses
POOL_ETH=0x4cc9Ea41Dec9bF5d7e38D6a216e861243d93bb6D
POOL_DOLI=doli1grww5ppnvz2phncskf0zq5tq3hxc87c5

# Config
RATE=10
MIN_USDT=0.10
ORDER_EXPIRY_MINUTES=30
```

---

## Quick Start (Backend)

```bash
# 1. Crear directorio
mkdir doli-swap-bot && cd doli-swap-bot

# 2. Inicializar
npm init -y
npm install express better-sqlite3 ethers dotenv

# 3. Crear .env con las variables arriba

# 4. Crear server.js (ver Bot Logic arriba)

# 5. Ejecutar
node server.js
```

---

## Contact

- **Website:** https://doli.network
- **Email:** weil@doli.network
- **GitHub:** https://github.com/e-weil/doli
