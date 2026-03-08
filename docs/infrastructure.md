# DOLI Infrastructure

## Servers

| Server | IP | Alias | Role | SSH |
|--------|-----|-------|------|-----|
| omegacortex (ai1) | 72.60.228.233 | ai1 | Primary: N1, N2, N6, Archiver, NT1-NT5, Archive-T | `ssh ilozada@72.60.228.233` |
| omegacortex.ai2 | 187.124.95.188 | ai2 | Mirror of ai1 + Web + Swap Bot | `ssh ilozada@omegacortex.ai2` |
| N3 | 147.93.84.44 | — | N3, NT6-NT8 | `ssh -p 50790 ilozada@147.93.84.44` |
| N4 | 72.60.115.209 | — | N4, N8-N12 | `ssh -p 50790 ilozada@72.60.115.209` |
| N5 | 72.60.70.166 | — | N5, N7, NT9-NT12 | `ssh -p 50790 ilozada@72.60.70.166` |

## ai2 Mirror Architecture

ai2 (187.124.95.188) is a full redundancy mirror of ai1, deployed 2026-03-08. It runs non-producing copies of the same mainnet and testnet services, plus web infrastructure.

### Services

| Service | Type | Ports (P2P/RPC) |
|---------|------|-----------------|
| `doli-mainnet-node1` | Mainnet relay mirror | 30303 / 8545 |
| `doli-mainnet-node2` | Mainnet mirror | 30304 / 8546 |
| `doli-mainnet-node6` | Mainnet mirror | 30305 / 8547 |
| `doli-mainnet-archiver` | Mainnet archive mirror | 30306 / 8548 |
| `doli-testnet-nt1` | Testnet relay mirror | 40303 / 18545 |
| `doli-testnet-nt2` | Testnet mirror | 40304 / 18546 |
| `doli-testnet-nt3` | Testnet mirror | 40305 / 18547 |
| `doli-testnet-nt4` | Testnet mirror | 40306 / 18548 |
| `doli-testnet-nt5` | Testnet mirror | 40307 / 18549 |
| `doli-testnet-archiver` | Testnet archive mirror | 40308 / 18550 |
| `nginx` | Web server | 80 / 443 |
| `doli-swap` | Swap bot | 3000 |

### Binaries

| Binary | Path |
|--------|------|
| Mainnet doli-node | `~/repos/doli/target/release/doli-node` |
| Mainnet doli CLI | `~/repos/doli/target/release/doli` |
| Testnet doli-node | `/opt/doli/testnet/doli-node` |
| Testnet doli CLI | `/opt/doli/testnet/doli` |

### Keys

| Network | Path |
|---------|------|
| Mainnet | `~/.doli/mainnet/keys/producer_{1,2,6}.json` |
| Testnet | `~/doli-test/keys/nt{1-5}.json` |

## DNS Round-Robin

DNS records use round-robin for redundancy between ai1 and ai2.

| Record | Type | IPs | Purpose |
|--------|------|-----|---------|
| `doli.network` | A | 72.60.228.233 + 187.124.95.188 | Website |
| `www.doli.network` | CNAME | → doli.network | Website alias |
| `seed1.doli.network` | A | 72.60.228.233 + 187.124.95.188 | P2P seed node |
| `seed2.doli.network` | A | 72.60.228.233 + 187.124.95.188 | P2P seed node |
| `archive.doli.network` | A | 72.60.228.233 + 187.124.95.188 | Block archive RPC |
| `testnet.doli.network` | A | 187.124.95.188 | Testnet web (ai2 only) |
| `bootstrap1.testnet.doli.network` | A | 72.60.228.233 | Testnet P2P bootstrap |
| `bootstrap2.testnet.doli.network` | A | 72.60.228.233 | Testnet P2P bootstrap |

## SSL Certificates

Managed by certbot with auto-renewal on ai2:

| Domain | Server | Expiry |
|--------|--------|--------|
| `doli.network` | ai2 | 2026-06-06 |
| `www.doli.network` | ai2 | 2026-06-06 |
| `testnet.doli.network` | ai2 | 2026-06-06 |

ai1 also has its own SSL certificates for doli.network.

## Mainnet Node Distribution

### Genesis Producers (N1-N6) — producing

| Node | Server | P2P | RPC | Key |
|------|--------|-----|-----|-----|
| N1 | ai1 | 30303 | 8545 | `producer_1.json` |
| N2 | ai1 | 30304 | 8546 | `producer_2.json` |
| N3 | N3 | 30303 | 8545 | `producer_3.json` |
| N4 | N4 | 30303 | 8545 | `producer_5.json` (swapped) |
| N5 | N5 | 30303 | 8545 | `producer_4.json` (swapped) |
| N6 | ai1 | 30305 | 8547 | `producer_6.json` |

### Registered Producers (N7-N12) — producing

| Node | Server | P2P | RPC |
|------|--------|-----|-----|
| N7 | N5 | 30304 | 8546 |
| N8-N12 | N4 | 30304-30308 | 8546-8550 |

### Archivers — non-producing

| Node | Server | P2P | RPC |
|------|--------|-----|-----|
| Archiver | ai1 | 30306 | 8548 |
| Archiver-m | ai2 | 30306 | 8548 |

## Testnet Node Distribution

### Genesis Producers (NT1-NT6) — producing

| Node | Server |
|------|--------|
| NT1-NT5 | ai1 |
| NT6-NT8 | N3 |

### Registered Producers (NT9-NT12) — producing

| Node | Server |
|------|--------|
| NT9-NT12 | N5 |

### Mirrors (non-producing)

| Node | Server |
|------|--------|
| NT1m-NT5m | ai2 |

N4 has no testnet nodes.
