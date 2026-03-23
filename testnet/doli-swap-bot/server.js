/**
 * DOLI Swap Bot - BSC USDT (BEP-20) → DOLI Exchange
 * Matches payments by exact USDT amount (random suffix per order).
 */

require("dotenv").config();
const express = require("express");
const cors = require("cors");
const helmet = require("helmet");
const rateLimit = require("express-rate-limit");
const Database = require("better-sqlite3");
const { ethers } = require("ethers");
const { exec } = require("child_process");
const util = require("util");
const execAsync = util.promisify(exec);
const fs = require("fs");
const path = require("path");

// =============================================================================
// CONFIGURATION
// =============================================================================

const CONFIG = {
    PORT: process.env.PORT || 3000,
    RATE: parseInt(process.env.RATE) || 10,
    MIN_USDT: parseFloat(process.env.MIN_USDT) || 0.10,
    ORDER_EXPIRY_MINUTES: parseInt(process.env.ORDER_EXPIRY_MINUTES) || 30,
    REQUIRED_CONFIRMATIONS: parseInt(process.env.REQUIRED_CONFIRMATIONS) || 5,
    POOL_BSC: process.env.POOL_BSC,
    POOL_DOLI: process.env.POOL_DOLI || "doli17engd6utnqs4ag6l6xme7tdhvgh6rcd8ezay5qw0vssqxyw239ts9dygef",
    BSC_RPC: process.env.BSC_RPC || "https://bsc-dataseed.binance.org",
    USDT_CONTRACT: process.env.USDT_CONTRACT || "0x55d398326f99059fF775485246999027B3197955",
    USDT_DECIMALS: parseInt(process.env.USDT_DECIMALS) || 18,
    DOLI_RPC: process.env.DOLI_RPC || "http://127.0.0.1:8545",
    DOLI_CLI: process.env.DOLI_CLI || "/home/ilozada/repos/doli/target/release/doli",
    DOLI_WALLET: process.env.DOLI_WALLET || "/home/ilozada/.doli/mainnet/keys/producer_1.json",
    MAX_PENDING_ORDERS_PER_ADDRESS: 3,
    MAX_USDT_PER_ORDER: 10000,
    POLL_INTERVAL_MS: 5000,  // BSC blocks ~3s
};

const USDT_ABI = [
    "event Transfer(address indexed from, address indexed to, uint256 value)",
    "function balanceOf(address) view returns (uint256)"
];

// =============================================================================
// LOGGING
// =============================================================================

const LOG_FILE = path.join(__dirname, "swap.log");

function log(level, message, data = {}) {
    const timestamp = new Date().toISOString();
    const entry = JSON.stringify({ timestamp, level, message, ...data });
    console.log(`[${timestamp}] [${level}] ${message}`, Object.keys(data).length ? data : "");
    fs.appendFileSync(LOG_FILE, entry + "\n");
}

// =============================================================================
// DATABASE
// =============================================================================

const db = new Database(path.join(__dirname, "swap.db"));

db.exec(`
    CREATE TABLE IF NOT EXISTS orders (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        doli_address TEXT NOT NULL,
        usdt_amount REAL NOT NULL,
        usdt_exact REAL NOT NULL,
        doli_amount REAL NOT NULL,
        status TEXT DEFAULT 'pending',
        bsc_tx_hash TEXT,
        doli_tx_hash TEXT,
        error_message TEXT,
        created_at TEXT,
        expires_at TEXT,
        completed_at TEXT
    );

    CREATE INDEX IF NOT EXISTS idx_orders_status ON orders(status);
    CREATE INDEX IF NOT EXISTS idx_orders_usdt_exact ON orders(usdt_exact);
    CREATE INDEX IF NOT EXISTS idx_orders_doli_address ON orders(doli_address);

    CREATE TABLE IF NOT EXISTS processed_txs (
        tx_hash TEXT PRIMARY KEY,
        processed_at TEXT
    );

    CREATE TABLE IF NOT EXISTS state (
        key TEXT PRIMARY KEY,
        value TEXT
    );
`);

function getState(key, defaultValue) {
    const row = db.prepare("SELECT value FROM state WHERE key = ?").get(key);
    return row ? row.value : defaultValue;
}

function setState(key, value) {
    db.prepare("INSERT OR REPLACE INTO state (key, value) VALUES (?, ?)").run(key, value);
}

// =============================================================================
// UNIQUE EXACT AMOUNT GENERATION
// =============================================================================

function generateExactAmount(baseAmount) {
    // Add a random suffix 0.01-0.99 to make each order's USDT amount unique
    for (let attempts = 0; attempts < 100; attempts++) {
        const suffix = Math.floor(Math.random() * 99 + 1) / 100; // 0.01 to 0.99
        const exact = Math.round((baseAmount + suffix) * 100) / 100;

        const existing = db.prepare(
            "SELECT 1 FROM orders WHERE usdt_exact = ? AND status = 'pending'"
        ).get(exact);

        if (!existing) return exact;
    }
    // Fallback: extend to 3 decimals
    const suffix = Math.floor(Math.random() * 999 + 1) / 1000;
    return Math.round((baseAmount + suffix) * 1000) / 1000;
}

// =============================================================================
// DOLI FUNCTIONS
// =============================================================================

let doliBalance = 50000;

async function getDoliBalance() {
    try {
        const cmd = `${CONFIG.DOLI_CLI} -w ${CONFIG.DOLI_WALLET} -r ${CONFIG.DOLI_RPC} balance 2>/dev/null`;
        const { stdout } = await execAsync(cmd);
        const match = stdout.match(/([0-9]+\.?[0-9]*)/);
        if (match) doliBalance = parseFloat(match[1]);
    } catch (err) {
        // Silently use cached balance if DOLI node not available
    }
    return doliBalance;
}

async function sendDoli(toAddress, amount) {
    log("INFO", "Sending DOLI", { to: toAddress, amount });
    try {
        const cmd = `${CONFIG.DOLI_CLI} -w ${CONFIG.DOLI_WALLET} -r ${CONFIG.DOLI_RPC} send ${toAddress} ${amount} 2>&1`;
        const { stdout } = await execAsync(cmd);
        const txMatch = stdout.match(/[a-f0-9]{64}/i);
        return txMatch ? txMatch[0] : `tx_${Date.now()}`;
    } catch (err) {
        throw new Error(`Failed to send DOLI: ${err.message}`);
    }
}

// =============================================================================
// BSC WATCHER (POLLING-BASED)
// =============================================================================

let provider = null;
let usdtContract = null;
let isWatching = false;
let lastProcessedBlock = 0;

async function initBscWatcher() {
    try {
        provider = new ethers.JsonRpcProvider(CONFIG.BSC_RPC);
        usdtContract = new ethers.Contract(CONFIG.USDT_CONTRACT, USDT_ABI, provider);

        const currentBlock = await provider.getBlockNumber();
        lastProcessedBlock = parseInt(getState("last_block", currentBlock - 10));

        log("INFO", "BSC connected", { currentBlock, startingFrom: lastProcessedBlock });

        isWatching = true;
        pollForTransfers();

        return true;
    } catch (err) {
        log("ERROR", "Failed to connect to BSC", { error: err.message });
        return false;
    }
}

async function pollForTransfers() {
    while (isWatching) {
        try {
            const currentBlock = await provider.getBlockNumber();
            const safeBlock = currentBlock - CONFIG.REQUIRED_CONFIRMATIONS;

            if (safeBlock > lastProcessedBlock) {
                const fromBlock = lastProcessedBlock + 1;
                // BSC public RPCs limit range to ~5000 blocks
                const toBlock = Math.min(safeBlock, fromBlock + 4999);

                const filter = {
                    address: CONFIG.USDT_CONTRACT,
                    topics: [
                        ethers.id("Transfer(address,address,uint256)"),
                        null, // any from
                        ethers.zeroPadValue(CONFIG.POOL_BSC, 32) // to our pool
                    ],
                    fromBlock,
                    toBlock,
                };

                const logs = await provider.getLogs(filter);

                for (const logEntry of logs) {
                    await processTransferLog(logEntry);
                }

                lastProcessedBlock = toBlock;
                setState("last_block", toBlock.toString());
            }
        } catch (err) {
            log("WARN", "Poll error", { error: err.message });
        }

        await new Promise(r => setTimeout(r, CONFIG.POLL_INTERVAL_MS));
    }
}

async function processTransferLog(logEntry) {
    const txHash = logEntry.transactionHash;

    // Check if already processed
    const processed = db.prepare("SELECT 1 FROM processed_txs WHERE tx_hash = ?").get(txHash);
    if (processed) return;

    // Decode the log
    const iface = new ethers.Interface(USDT_ABI);
    const decoded = iface.parseLog({ topics: logEntry.topics, data: logEntry.data });

    const from = decoded.args[0];
    const amount = parseFloat(ethers.formatUnits(decoded.args[2], CONFIG.USDT_DECIMALS));

    log("INFO", "USDT Transfer detected", { from, amount, txHash });

    db.prepare("INSERT INTO processed_txs (tx_hash, processed_at) VALUES (?, ?)")
        .run(txHash, new Date().toISOString());

    // Match by exact amount (rounded to cents for comparison)
    const rounded = Math.round(amount * 100) / 100;
    const order = db.prepare(`
        SELECT * FROM orders
        WHERE usdt_exact = ?
        AND status = 'pending'
        AND expires_at > datetime('now')
        ORDER BY created_at ASC LIMIT 1
    `).get(rounded);

    if (!order) {
        log("WARN", "Payment without matching order", { from, amount: rounded });
        return;
    }

    // Check liquidity
    const balance = await getDoliBalance();
    if (balance < order.doli_amount) {
        db.prepare("UPDATE orders SET status = ?, error_message = ? WHERE id = ?")
            .run("no_liquidity", "Insufficient liquidity", order.id);
        return;
    }

    // Send DOLI
    try {
        const doliTxHash = await sendDoli(order.doli_address, order.doli_amount);
        db.prepare(`
            UPDATE orders SET status = 'completed', bsc_tx_hash = ?, doli_tx_hash = ?, completed_at = ?
            WHERE id = ?
        `).run(txHash, doliTxHash, new Date().toISOString(), order.id);
        log("INFO", "Order completed", { orderId: order.id, doliTx: doliTxHash });
    } catch (err) {
        db.prepare("UPDATE orders SET status = ?, error_message = ? WHERE id = ?")
            .run("doli_failed", err.message, order.id);
    }
}

// =============================================================================
// EXPIRY CLEANUP
// =============================================================================

function cleanupExpiredOrders() {
    const expired = db.prepare(`
        UPDATE orders SET status = 'expired'
        WHERE status = 'pending' AND expires_at < datetime('now')
    `).run();
    if (expired.changes > 0) {
        log("INFO", "Expired orders cleaned", { count: expired.changes });
    }
}

// =============================================================================
// EXPRESS API
// =============================================================================

const app = express();
app.use(helmet());
app.use(cors());
app.use(express.json());
app.use(rateLimit({ windowMs: 60000, max: 30 }));

app.get("/health", (req, res) => {
    res.json({ status: "ok", watching: isWatching, lastBlock: lastProcessedBlock });
});

app.get("/api/liquidity", async (req, res) => {
    const doli = await getDoliBalance();
    res.json({
        doli,
        rate: CONFIG.RATE,
        min_usdt: CONFIG.MIN_USDT,
        pool_bsc: CONFIG.POOL_BSC,
        chain: "BSC (BEP-20)",
    });
});

app.post("/api/order", async (req, res) => {
    const { doli_address, usdt_amount } = req.body;

    if (!doli_address || !doli_address.startsWith("doli1"))
        return res.status(400).json({ error: "Invalid DOLI address. Must start with doli1" });

    const usdt = parseFloat(usdt_amount);
    if (isNaN(usdt) || usdt < CONFIG.MIN_USDT)
        return res.status(400).json({ error: `Minimum is ${CONFIG.MIN_USDT} USDT` });
    if (usdt > CONFIG.MAX_USDT_PER_ORDER)
        return res.status(400).json({ error: `Maximum is ${CONFIG.MAX_USDT_PER_ORDER} USDT` });

    // Rate limit by doli_address
    const pending = db.prepare(
        "SELECT COUNT(*) as c FROM orders WHERE doli_address = ? AND status = 'pending'"
    ).get(doli_address);
    if (pending.c >= CONFIG.MAX_PENDING_ORDERS_PER_ADDRESS)
        return res.status(400).json({ error: "Too many pending orders for this address" });

    const doliAmount = usdt * CONFIG.RATE;
    const balance = await getDoliBalance();
    if (balance < doliAmount)
        return res.status(400).json({ error: "Insufficient liquidity", available_doli: balance });

    const usdtExact = generateExactAmount(usdt);
    const now = new Date();
    const expires = new Date(now.getTime() + CONFIG.ORDER_EXPIRY_MINUTES * 60000);

    const result = db.prepare(`
        INSERT INTO orders (doli_address, usdt_amount, usdt_exact, doli_amount, created_at, expires_at)
        VALUES (?, ?, ?, ?, ?, ?)
    `).run(doli_address, usdt, usdtExact, doliAmount, now.toISOString(), expires.toISOString());

    log("INFO", "Order created", {
        orderId: result.lastInsertRowid,
        doli: doli_address,
        usdt,
        usdtExact,
        doliAmount,
    });

    res.json({
        success: true,
        order_id: result.lastInsertRowid,
        usdt_amount: usdtExact,
        doli_amount: doliAmount,
        pool_address: CONFIG.POOL_BSC,
        expires_at: expires.toISOString(),
    });
});

app.get("/api/order/:id", (req, res) => {
    const order = db.prepare("SELECT * FROM orders WHERE id = ?").get(req.params.id);
    if (!order) return res.status(404).json({ error: "Order not found" });
    res.json(order);
});

app.get("/api/admin/orders", (req, res) => {
    const orders = db.prepare("SELECT * FROM orders ORDER BY id DESC LIMIT 50").all();
    res.json({ orders });
});

// =============================================================================
// START
// =============================================================================

async function start() {
    log("INFO", "DOLI Swap Bot Starting (BSC Mode)");
    log("INFO", "Config", {
        rate: CONFIG.RATE,
        pool_bsc: CONFIG.POOL_BSC,
        pool_doli: CONFIG.POOL_DOLI,
        chain: "BSC",
    });

    await initBscWatcher();
    await getDoliBalance();

    // Cleanup expired orders every 60s
    setInterval(cleanupExpiredOrders, 60000);

    app.listen(CONFIG.PORT, () => {
        log("INFO", `API running on port ${CONFIG.PORT}`);
    });
}

process.on("SIGINT", () => { isWatching = false; db.close(); process.exit(0); });
start();
