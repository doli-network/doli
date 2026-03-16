# DOLI Launch Checklist - INTERNO

> **NO SUBIR A GIT** - Contiene información sensible de infraestructura

Última actualización: 2026-01-25

---

## Estado Actual

| Componente | Estado | Notas |
|------------|--------|-------|
| Código | ✅ Listo | Tests pasan, security fix aplicado |
| Bootstrap node #1 | ✅ Corriendo | omegacortex.ai |
| Bootstrap node #2 | ❌ Pendiente | Necesita segundo servidor |
| Maintainer keys | ❌ Pendiente | 5 keys Ed25519 |
| Genesis config | ⚠️ Parcial | Timestamp y recipients pendientes |
| Dominio | ❌ Pendiente | doli.network no registrado |

---

## Infraestructura

### Bootstrap Node #1 (ACTIVO)
```
Hostname: omegacortex.ai / axiomrx
IP: 72.60.228.233
Puerto P2P: 40303 (testnet)
Puerto RPC: 18545 (localhost only)
SSH: ssh ilozada@omegacortex.ai
Servicio: systemctl status doli-testnet
Logs: journalctl -u doli-testnet -f
Data: /home/ilozada/doli-data-testnet
Binary: /home/ilozada/doli-node/target/release/doli-node
```

### Bootstrap Node #2 (PENDIENTE)
```
Hostname: [TBD]
IP: [TBD]
Requisitos: Ubuntu 22+, 4GB RAM, 100GB disco
```

---

## Maintainer Keys (5 requeridas)

Las keys firman actualizaciones de software. Se requieren 3/5 para aprobar.

| # | Responsable | Public Key | Ubicación Private Key |
|---|-------------|------------|----------------------|
| 1 | [TBD] | [PENDIENTE] | [offline/hardware wallet] |
| 2 | [TBD] | [PENDIENTE] | [offline/hardware wallet] |
| 3 | [TBD] | [PENDIENTE] | [offline/hardware wallet] |
| 4 | [TBD] | [PENDIENTE] | [offline/hardware wallet] |
| 5 | [TBD] | [PENDIENTE] | [offline/hardware wallet] |

### Generar key (instrucciones)
```bash
# En máquina offline/segura
cd doli-node
cargo run --release --bin doli-cli -- keygen --output maintainer_key_N.json

# Output:
# Public key: <64 hex chars>
# Private key saved to: maintainer_key_N.json (GUARDAR OFFLINE)
```

### Actualizar en código
Archivo: `doli-updater/src/lib.rs` línea 77
```rust
pub const MAINTAINER_KEYS: [&str; 5] = [
    "<public_key_1>",
    "<public_key_2>",
    "<public_key_3>",
    "<public_key_4>",
    "<public_key_5>",
];
```

---

## Genesis Configuration

### Testnet (activo)
```
GENESIS_TIME: 1748736000 (2025-06-01 00:00:00 UTC)
Estado: Ya pasó, testnet puede generar bloques
```

### Mainnet (pendiente)
```
GENESIS_TIME actual: 1769904000 (2026-02-01 00:00:00 UTC)
Estado: ~1 semana en el futuro
```

**Decisión requerida:**
- [ ] Confirmar fecha de lanzamiento mainnet
- [ ] Definir distribución inicial (genesis recipients)

### Genesis Recipients
Archivo: `doli-core/src/genesis.rs`

```rust
// Mainnet - DEFINIR ANTES DE LANZAMIENTO
GenesisConfig {
    network: Network::Mainnet,
    recipients: vec![
        // (pubkey_hash, amount_in_units)
        // Total debe ser <= TOTAL_SUPPLY (21B coins = 2.1e18 units)
    ],
}
```

---

## Dominio doli.network

### Registros DNS necesarios

| Tipo | Nombre | Valor | Propósito |
|------|--------|-------|-----------|
| A | seed1.doli.network | 72.60.228.233 | Bootstrap mainnet #1 |
| A | seed2.doli.network | [IP #2] | Bootstrap mainnet #2 |
| A | testnet-seed1.doli.network | 72.60.228.233 | Bootstrap testnet #1 |
| A | releases.doli.network | [CDN IP] | Binarios de actualización |

### Registradores sugeridos
- Namecheap (~$10/año)
- Cloudflare Registrar (~$9/año)
- Porkbun (~$9/año)

---

## Checklist Pre-Genesis

### Crítico (bloqueante)
- [ ] 5 maintainer keys generadas y guardadas offline
- [ ] Public keys actualizadas en `doli-updater/src/lib.rs`
- [ ] Segundo bootstrap node corriendo
- [ ] Genesis recipients definidos
- [ ] GENESIS_TIME confirmado

### Importante (recomendado)
- [ ] Dominio doli.network registrado
- [ ] DNS configurado para seeds
- [ ] releases.doli.network con binarios
- [ ] Testnet corriendo 72h+ sin fallos

### Opcional (puede esperar)
- [ ] Auditoría externa
- [ ] Website informativo
- [ ] Block explorer
- [ ] Wallet GUI

---

## Comandos Útiles

### Servidor (omegacortex.ai)
```bash
# Estado del nodo
ssh ilozada@omegacortex.ai "systemctl status doli-testnet"

# Logs en tiempo real
ssh ilozada@omegacortex.ai "journalctl -u doli-testnet -f"

# Reiniciar nodo
ssh ilozada@omegacortex.ai "sudo systemctl restart doli-testnet"

# Actualizar código
ssh ilozada@omegacortex.ai "cd ~/doli-node && git pull && source ~/.cargo/env && cargo build --release --package doli-node && sudo systemctl restart doli-testnet"
```

### Local
```bash
# Correr tests
cargo test --workspace

# Conectar a testnet
./target/release/doli-node --network testnet run

# Generar keypair
./target/release/doli-cli keygen
```

---

## Contactos

| Rol | Persona | Contacto |
|-----|---------|----------|
| Lead dev | ilozada | [email/signal] |
| Maintainer 2 | [TBD] | [TBD] |
| Maintainer 3 | [TBD] | [TBD] |
| Maintainer 4 | [TBD] | [TBD] |
| Maintainer 5 | [TBD] | [TBD] |

---

## Historial de Cambios

| Fecha | Cambio |
|-------|--------|
| 2026-01-25 | Documento creado |
| 2026-01-25 | Bootstrap node #1 desplegado en omegacortex.ai |
| 2026-01-25 | Security fix: VDF validation usa t_block(height) |
| 2026-01-25 | Grinding risk documentado en SECURITY.md |
