// DOLI Local Explorer Server
// Serves static HTML + proxies /api/mainnet/X and /rpc to local RPC ports
//
// Usage: node server.js
// Then open http://localhost:8080
//
// Port layout (matches launchd services):
//   Seed: RPC=8500, N{i}: RPC=8500+i

const http = require("http");
const fs = require("fs");
const path = require("path");

const PORT = 8080;
const STATIC_DIR = __dirname;

// Map API paths to local RPC ports
function resolveRpcPort(urlPath) {
  // /api/mainnet/seed -> 8500
  if (urlPath.match(/^\/api\/mainnet\/seed/)) return 8500;
  // /api/mainnet/n{i} -> 8500+i
  const match = urlPath.match(/^\/api\/mainnet\/n(\d+)$/);
  if (match) return 8500 + parseInt(match[1]);
  // /rpc -> seed RPC
  if (urlPath === "/rpc") return 8500;
  return null;
}

function proxyRpc(req, res, targetPort) {
  let body = "";
  req.on("data", (chunk) => (body += chunk));
  req.on("end", () => {
    const options = {
      hostname: "127.0.0.1",
      port: targetPort,
      path: "/",
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "Content-Length": Buffer.byteLength(body),
      },
      timeout: 5000,
    };

    const proxy = http.request(options, (proxyRes) => {
      res.setHeader("Access-Control-Allow-Origin", "*");
      res.setHeader("Access-Control-Allow-Methods", "POST, OPTIONS");
      res.setHeader("Access-Control-Allow-Headers", "Content-Type");
      res.setHeader("Content-Type", "application/json");
      res.writeHead(proxyRes.statusCode);
      proxyRes.pipe(res);
    });

    proxy.on("error", () => {
      res.writeHead(502, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: "Node not reachable on port " + targetPort }));
    });

    proxy.on("timeout", () => {
      proxy.destroy();
      res.writeHead(504, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: "Timeout reaching port " + targetPort }));
    });

    proxy.write(body);
    proxy.end();
  });
}

function proxyWebSocket(req, socket, head, targetPort) {
  const net = require("net");
  const upstream = net.createConnection({ host: "127.0.0.1", port: targetPort }, () => {
    // Forward the original HTTP upgrade request
    const reqLine = `${req.method} /ws HTTP/${req.httpVersion}\r\n`;
    let headers = "";
    for (let i = 0; i < req.rawHeaders.length; i += 2) {
      headers += `${req.rawHeaders[i]}: ${req.rawHeaders[i + 1]}\r\n`;
    }
    upstream.write(reqLine + headers + "\r\n");
    if (head.length > 0) upstream.write(head);
    // Bidirectional pipe
    socket.pipe(upstream);
    upstream.pipe(socket);
  });
  upstream.on("error", () => socket.destroy());
  socket.on("error", () => upstream.destroy());
}

const MIME = {
  ".html": "text/html",
  ".js": "application/javascript",
  ".css": "text/css",
  ".json": "application/json",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
};

const server = http.createServer((req, res) => {
  const url = new URL(req.url, `http://localhost:${PORT}`);

  // CORS preflight
  if (req.method === "OPTIONS") {
    res.setHeader("Access-Control-Allow-Origin", "*");
    res.setHeader("Access-Control-Allow-Methods", "POST, OPTIONS");
    res.setHeader("Access-Control-Allow-Headers", "Content-Type");
    res.writeHead(204);
    res.end();
    return;
  }

  // Discovery endpoint: scan ports 8500-8612 for running nodes
  if (url.pathname === "/api/discover" && req.method === "GET") {
    const nodes = [];
    let pending = 0;
    const scanStart = 8500;
    const scanEnd = 8712; // seed + n1-n212 (covers batches 1-4)
    pending = scanEnd - scanStart + 1;

    for (let port = scanStart; port <= scanEnd; port++) {
      const body = JSON.stringify({ jsonrpc: "2.0", method: "getChainInfo", params: {}, id: 1 });
      const opts = {
        hostname: "127.0.0.1", port, path: "/", method: "POST",
        headers: { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(body) },
        timeout: 1000,
      };
      const probe = http.request(opts, (probeRes) => {
        let data = "";
        probeRes.on("data", (c) => (data += c));
        probeRes.on("end", () => {
          try {
            const r = JSON.parse(data).result;
            if (r && r.bestHeight !== undefined) {
              const offset = port - 8500;
              const name = offset === 0 ? "Seed" : "N" + offset;
              nodes.push({ name, server: "local", api: "/api/mainnet/" + (offset === 0 ? "seed" : "n" + offset), port });
            }
          } catch (e) {}
          if (--pending === 0) {
            nodes.sort((a, b) => a.port - b.port);
            res.setHeader("Content-Type", "application/json");
            res.setHeader("Access-Control-Allow-Origin", "*");
            res.end(JSON.stringify(nodes));
          }
        });
      });
      probe.on("error", () => { if (--pending === 0) { res.setHeader("Content-Type", "application/json"); res.end(JSON.stringify(nodes)); } });
      probe.on("timeout", () => { probe.destroy(); });
      probe.write(body);
      probe.end();
    }
    return;
  }

  // API proxy
  const rpcPort = resolveRpcPort(url.pathname);
  if (rpcPort && req.method === "POST") {
    proxyRpc(req, res, rpcPort);
    return;
  }

  // Static files
  let filePath = url.pathname;
  if (filePath === "/") filePath = "/index.html";

  const fullPath = path.join(STATIC_DIR, filePath);
  // Security: prevent directory traversal
  if (!fullPath.startsWith(STATIC_DIR)) {
    res.writeHead(403);
    res.end("Forbidden");
    return;
  }

  const ext = path.extname(fullPath);
  const contentType = MIME[ext] || "application/octet-stream";

  fs.readFile(fullPath, (err, data) => {
    if (err) {
      res.writeHead(404);
      res.end("Not Found");
      return;
    }
    res.writeHead(200, { "Content-Type": contentType });
    res.end(data);
  });
});

// WebSocket upgrade
server.on("upgrade", (req, socket, head) => {
  if (req.url === "/ws") {
    proxyWebSocket(req, socket, head, 8500);
  } else {
    socket.destroy();
  }
});

server.listen(PORT, () => {
  console.log(`DOLI Explorer running at http://localhost:${PORT}`);
  console.log(`  Explorer:  http://localhost:${PORT}/`);
  console.log(`  Network:   http://localhost:${PORT}/network.html`);
  console.log(`  RPC proxy: /rpc → 127.0.0.1:8500`);
  console.log(`  WS proxy:  /ws  → 127.0.0.1:8500/ws`);
  console.log(`  API proxy: /api/mainnet/seed → :8500, /api/mainnet/n{i} → :8500+i`);
});
