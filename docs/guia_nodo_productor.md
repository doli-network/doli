# Guia: Nodo Productor DOLI Mainnet

Guia paso a paso para configurar un nodo productor en DOLI mainnet.

---

## Requisitos

| Componente | Minimo | Recomendado |
|------------|--------|-------------|
| CPU | 4 cores | 8+ cores |
| RAM | 8 GB | 16+ GB |
| Disco | 100 GB SSD | 500+ GB NVMe |
| Red | 50 Mbps | 100+ Mbps |
| Bond | 10 DOLI (1 bond) | - |

---

## Paso 1: Clonar y compilar

```bash
# Instalar Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Dependencias del sistema (Ubuntu/Debian)
sudo apt install build-essential pkg-config libssl-dev libgmp-dev librocksdb-dev

# Clonar y compilar
git clone https://github.com/e-weil/doli.git
cd doli
cargo build --release
```

Los binarios quedan en `target/release/`:
- `doli-node` — nodo completo
- `doli` — CLI de wallet

---

## Paso 2: Crear wallet del productor

```bash
# IMPORTANTE: -w es flag global, va ANTES del subcomando
./target/release/doli -w ~/.doli/mainnet/producer.json new
```

> **Nota:** `-w` (wallet path) es un flag global del CLI. Siempre va antes del subcomando (`new`, `info`, `balance`, etc.), nunca despues.

Verificar que se creo correctamente:

```bash
./target/release/doli -w ~/.doli/mainnet/producer.json info
```

La salida muestra tres valores:
- **Address (20-byte)** — NO usar para envios
- **Pubkey Hash (32-byte)** — USAR ESTE para recibir fondos
- **Public Key** — solo verificacion

Respaldar el archivo de wallet:

```bash
cp ~/.doli/mainnet/producer.json ~/backup/
```

---

## Paso 3: Fondear la wallet

Obtener la direccion correcta (Pubkey Hash de 32 bytes):

```bash
./target/release/doli -w ~/.doli/mainnet/producer.json info
```

Desde una wallet con fondos, enviar al Pubkey Hash del nuevo productor:

```bash
./target/release/doli -w ~/.doli/mainnet/funded_wallet.json \
    --rpc http://127.0.0.1:8545 \
    send <PUBKEY_HASH_DEL_PRODUCTOR> 15
```

Necesitas: bond (10 DOLI) + fee de registro + margen operativo.

Verificar balance:

```bash
./target/release/doli -w ~/.doli/mainnet/producer.json \
    --rpc http://127.0.0.1:8545 \
    balance
```

---

## Paso 4: Registrar como productor

```bash
./target/release/doli -w ~/.doli/mainnet/producer.json \
    --rpc http://127.0.0.1:8545 \
    producer register
```

Esto inicia el VDF de registro (~10 minutos) y envia la transaccion de registro con 1 bond (10 DOLI).

Para registrar con mas bonds:

```bash
./target/release/doli -w ~/.doli/mainnet/producer.json \
    --rpc http://127.0.0.1:8545 \
    producer register --bonds 5
```

---

## Paso 5: Iniciar el nodo productor

```bash
./target/release/doli-node run \
    --producer \
    --producer-key ~/.doli/mainnet/producer.json \
    --no-auto-update \
    --p2p-port 30303 \
    --rpc-port 8545
```

Con nodo bootstrap (para unirse a red existente):

```bash
./target/release/doli-node run \
    --producer \
    --producer-key ~/.doli/mainnet/producer.json \
    --no-auto-update \
    --bootstrap /ip4/<IP_BOOTSTRAP>/tcp/30303
```

> **Nota:** `--no-auto-update` se recomienda durante la fase inicial de mainnet mientras el sistema de actualizacion usa claves de bootstrap. Cuando las claves de mantenedores esten derivadas on-chain, se puede remover este flag.

### Servicio systemd (produccion)

```bash
mkdir -p ~/.config/systemd/user/
```

Crear `~/.config/systemd/user/doli-producer.service`:

```ini
[Unit]
Description=DOLI Producer Node
After=network.target

[Service]
Type=simple
ExecStart=%h/repos/doli/target/release/doli-node run \
    --producer \
    --producer-key %h/.doli/mainnet/producer.json \
    --no-auto-update \
    --p2p-port 30303 \
    --rpc-port 8545
Restart=always
RestartSec=10
LimitNOFILE=65536

[Install]
WantedBy=default.target
```

Activar:

```bash
systemctl --user daemon-reload
systemctl --user enable doli-producer
systemctl --user start doli-producer
systemctl --user status doli-producer
```

---

## Paso 6: Verificar produccion

```bash
# Estado del productor
./target/release/doli -w ~/.doli/mainnet/producer.json \
    --rpc http://127.0.0.1:8545 \
    producer status

# Balance (debe incrementar con recompensas)
./target/release/doli -w ~/.doli/mainnet/producer.json \
    --rpc http://127.0.0.1:8545 \
    balance

# Info de la cadena
./target/release/doli -w ~/.doli/mainnet/producer.json \
    --rpc http://127.0.0.1:8545 \
    chain
```

---

## Firewall

Abrir solo el puerto P2P:

```bash
sudo ufw allow 30303/tcp
```

NO exponer el puerto RPC (8545) a internet.

---

## Respaldo

| Archivo | Prioridad | Nota |
|---------|-----------|------|
| `~/.doli/mainnet/producer.json` | Critica | Clave del productor — perderla = perder el bond |
| `~/.doli/mainnet/node.key` | Alta | Identidad del nodo — sin ella cambia el PeerId |
| `~/.doli/mainnet/db/` | Baja | Se puede resincronizar |

---

## Advertencias

- **NUNCA** ejecutar dos nodos con la misma clave de productor simultaneamente. Esto causa slashing (100% del bond quemado).
- **NUNCA** compartir el archivo `producer.json` — contiene la clave privada.
- Las recompensas de coinbase tienen un periodo de madurez de 100 bloques antes de poder gastarlas.
- El bond se bloquea por 4 anos. Retiro anticipado tiene penalidades: 75% (ano 0-1), 50% (ano 1-2), 25% (ano 2-3), 0% (ano 3+).

---

*Ultima actualizacion: Febrero 2026*
