# Escenario Extremo: Devnet con 600 Productores

Este documento analiza el comportamiento de DOLI bajo condiciones extremas con 600 validadores activos en devnet, aplicando todas las fórmulas del whitepaper.

## Resumen Ejecutivo

| Metric | Devnet Normal | Devnet Extremo (600) |
|--------|---------------|----------------------|
| Productores activos | 1-10 | 600 |
| Probabilidad de producir por slot | 10-100% | 0.17% |
| Bloques por productor por hora | 720-72 | 1.2 |
| Tiempo entre bloques propios | 5-50s | ~50 min |
| Total bond bloqueado | 1-10 DOLI | 600 DOLI |
| Recompensas por hora | 360,000 DOLI | 360,000 DOLI |
| ROI por productor por hora | 36,000-360,000 DOLI | 600 DOLI |

---

## 1. Parámetros Base del Devnet

Del whitepaper y `network.rs`:

```
SLOT_DURATION      = 5 segundos
VDF_ITERATIONS     = 1,000,000 (~1 segundo)
INITIAL_BOND       = 1 DOLI (100,000,000 satoshis)
BLOCK_REWARD       = 500 DOLI (50,000,000,000 satoshis)
BOOTSTRAP_BLOCKS   = 100
SLOTS_PER_EPOCH    = 60
VETO_PERIOD        = 60 segundos
```

---

## 2. Cálculos de Selección de Productor

### 2.1. Semilla de Selección (Sección 7)

```
semilla = HASH("SEED" || hash_anterior || ranura)
puntuación(llave, ranura) = HASH(semilla || llave)
```

El productor con MENOR puntuación es el primario (rank 0).

### 2.2. Distribución de Probabilidad con 600 Productores

Con 600 productores uniformemente distribuidos:

| Rank | Probabilidad por slot | Tiempo promedio entre asignaciones |
|------|----------------------|-----------------------------------|
| 0 (primario) | 1/600 = 0.167% | 600 slots = 50 minutos |
| 1 (fallback 1) | 1/600 = 0.167% | 600 slots = 50 minutos |
| 2 (fallback 2) | 1/600 = 0.167% | 600 slots = 50 minutos |
| Top 3 | 3/600 = 0.5% | 200 slots = 16.7 minutos |

### 2.3. Ventanas de Producción (Sección 7.1)

Con slot de 5 segundos, las ventanas se escalan proporcionalmente:

| Tiempo en slot (mainnet 60s) | Tiempo en slot (devnet 5s) | Elegibles |
|------------------------------|---------------------------|-----------|
| 0-30s | 0-2.5s | Solo rank 0 |
| 30-45s | 2.5-3.75s | rank 0 o 1 |
| 45-60s | 3.75-5s | rank 0, 1 o 2 |

**Fórmula escalada para devnet:**
```
ventana_rank_N = slot_duration × (N × 15) / 60

rank 0: válido desde t=0
rank 1: válido desde t=1.25s
rank 2: válido desde t=1.875s
```

### 2.4. Simulación de Bloques Perdidos

Si el productor primario (rank 0) está offline:

| Escenario | Probabilidad con 600 prod | Tiempo extra |
|-----------|--------------------------|--------------|
| Rank 0 online | 99%+ (asumiendo) | 0s |
| Rank 0 offline, rank 1 online | ~99% | +1.25s |
| Rank 0,1 offline, rank 2 online | ~99% | +1.875s |
| Top 3 offline | (1/600)³ ≈ 0 | Slot vacío |

---

## 3. Registro Dinámico de Productores (Sección 6.1)

### 3.1. Dificultad Dinámica de Registro

Del whitepaper:
```
D_E = (D_{E-1} + R_E) / 2
T_registro(E+1) = T_base × max(1, D_E / R_objetivo)
```

Donde:
- `R_E` = registros confirmados en época E
- `R_objetivo` = registros objetivo por época (asumiendo 10)
- `T_base` = 600,000,000 iteraciones (~10 minutos en mainnet)

### 3.2. Escenario: 600 Registros Simultáneos

Si 600 productores intentan registrarse en la época 1:

```
Época 0: D_0 = 0, R_0 = 0
Época 1: R_1 = 600, D_1 = (0 + 600) / 2 = 300
Época 2: T_registro = T_base × max(1, 300/10) = T_base × 30

En devnet (T_base escalado a ~10 segundos):
T_registro = 10s × 30 = 300 segundos = 5 minutos por registro
```

### 3.3. Tiempo Total de Registro de 600 Productores

**Escenario secuencial (imposible en práctica):**
```
600 × 5 minutos = 3,000 minutos = 50 horas
```

**Escenario paralelo (realista):**
- Cada productor computa su VDF independientemente
- El cuello de botella es inclusión en bloques
- Con blocks de ~5s y ~100 TX por bloque:
  - 600 registros / 100 = 6 bloques = 30 segundos

**Tiempo real de onboarding con 600 productores:**
```
Fase 1: VDF individual = 5 minutos (paralelo para todos)
Fase 2: Confirmación en cadena = ~30 segundos
Total: ~5.5 minutos para tener 600 productores activos
```

---

## 4. Economía con 600 Productores

### 4.1. Distribución de Recompensas

```
BLOCK_REWARD = 500 DOLI
SLOT_DURATION = 5 segundos
BLOCKS_PER_HOUR = 3600 / 5 = 720 bloques
REWARDS_PER_HOUR = 720 × 500 = 360,000 DOLI
```

**Por productor:**
```
SLOTS_TO_PRODUCE = PRODUCERS × AVERAGE_WAIT
AVERAGE_WAIT = 600 slots
EXPECTED_BLOCKS_PER_HOUR = 720 / 600 = 1.2 bloques/hora
EXPECTED_REWARD_PER_HOUR = 1.2 × 500 = 600 DOLI
```

### 4.2. ROI del Bond

```
BOND = 1 DOLI
REWARD_PER_HOUR = 600 DOLI
ROI_PER_HOUR = 600 / 1 = 60,000%
ROI_PER_DAY = 600 × 24 = 14,400 DOLI = 1,440,000%
```

**Nota:** Estos números son absurdos porque devnet está diseñado para testing, no economía real.

### 4.3. Suministro Total

```
TOTAL_SUPPLY = 21,024,000 DOLI
EMISSION_RATE = 360,000 DOLI/hora
TIME_TO_EXHAUST_ERA_1 = 10,512,000 / 360,000 = 29.2 horas

Con 600 productores a máxima capacidad:
- Era 1 completa en ~29 horas
- 50% del suministro distribuido en ~1.2 días
```

---

## 5. Carga de Red

### 5.1. Mensajes GossipSub

Del Apéndice B del whitepaper:

```
mesh_n = 6 peers por nodo en el mesh
mesh_n_high = 12 máximo

Con 600 nodos:
- Cada nodo mantiene ~6 conexiones mesh
- Total conexiones = 600 × 6 / 2 = 1,800 conexiones
```

**Propagación de bloques:**
```
Cada bloque se propaga via gossip
Hop count promedio = log_6(600) ≈ 3.6 hops
Latencia estimada = 3.6 × 100ms = 360ms para llegar a toda la red
```

### 5.2. Sincronización de Mempool

```
MAX_MEMPOOL_TX = 5,000 (mainnet) / 10,000 (testnet)
TX_SIZE_AVG = 250 bytes
MEMPOOL_SIZE_MAX = 10,000 × 250 = 2.5 MB

Sincronización completa entre 600 nodos:
- Cada nodo envía estado a 6 peers
- Convergencia en ~4 ciclos de gossip
- Tiempo: 4 × 1s heartbeat = 4 segundos
```

### 5.3. Ancho de Banda por Nodo

```
BLOCK_SIZE_AVG = 100 KB (estimado para devnet)
BLOCKS_PER_HOUR = 720
BLOCK_TRAFFIC = 720 × 100 KB = 72 MB/hora outbound

Con factor de gossip (6 peers):
BANDWIDTH = 72 MB × 6 = 432 MB/hora = 120 KB/s

Incluyendo overhead de protocolo (+30%):
TOTAL_BANDWIDTH ≈ 156 KB/s por nodo
```

---

## 6. Escenarios de Estrés

### 6.1. Todos los Productores Online

**Situación ideal:**
- 600 productores compitiendo por cada slot
- Solo 1 puede ganar (rank 0)
- Los otros 599 desperdician VDF

**Eficiencia de red:**
```
VDF_COMPUTATIONS_PER_SLOT = 600
USEFUL_VDF = 1
EFFICIENCY = 1/600 = 0.17%
```

### 6.2. 50% de Productores Offline

**300 productores activos:**
```
PROBABILITY_SLOT_FILLED = 1 - (0)³ ≈ 100%
EXPECTED_BLOCKS_PER_PRODUCER = 720/300 = 2.4/hora
REWARD_PER_PRODUCER = 2.4 × 500 = 1,200 DOLI/hora
```

### 6.3. Partición de Red (Split Brain)

**Escenario:** Red se divide en 2 particiones de 300 nodos cada una.

```
Partición A: 300 productores, produce cadena A
Partición B: 300 productores, produce cadena B

Ambas cadenas avanzan a misma velocidad.
Al reconectarse, gana la cadena con MAYOR slot (sección 8):

1. Mayor número de ranura
2. Si empatan: mayor altura
3. Si empatan: menor hash

Una partición descarta ~50% de bloques producidos.
```

### 6.4. Ataque del 51% con 600 Productores

Para controlar la red, un atacante necesita:

```
PRODUCERS_NEEDED = 301 (mayoría simple)
BOND_NEEDED = 301 × 1 DOLI = 301 DOLI
VDF_TIME_PER_REGISTRATION = 5 minutos
TOTAL_VDF_TIME = 301 × 5 min = 1,505 minutos = 25 horas

Costo total:
- Tiempo: 25 horas de VDF (paralelo reduce esto)
- Capital: 301 DOLI bloqueados
```

**En devnet esto es trivial porque:**
1. Los DOLI no tienen valor
2. El VDF es rápido (~10 segundos)
3. No hay consecuencias reales

---

## 7. Inactividad y Slashing

### 7.1. Regla de Inactividad (Sección 7.1)

```
MAX_FALLOS = 50 slots consecutivos sin producir cuando asignado

Con 600 productores:
- Probabilidad de ser asignado = 1/600 por slot
- Slots para 50 asignaciones = 50 × 600 = 30,000 slots
- Tiempo para exclusión por inactividad = 30,000 × 5s = 150,000s = 41.7 horas
```

**Nota:** Un productor puede estar offline 41+ horas antes de ser excluido.

### 7.2. Producción Doble

```
PENALTY = 100% del bond = 1 DOLI
EXCLUSION_IMMEDIATE = Sí

Detección:
- Cualquier nodo puede construir prueba
- Prueba = (hash_bloque_1, hash_bloque_2, ranura, firma)
- TX tipo 5 (SLASH_PRODUCER) incluida en bloque
```

---

## 8. Pool de Recompensas

### 8.1. Acumulación (Sección 9.3)

```
Fuente: Penalizaciones por salida anticipada
Destino: Distribuido entre productores activos

recompensa_total = recompensa_base + (pool / productores_activos)
recompensa_total = 500 + (pool / 600)
```

### 8.2. Escenario: 100 Productores Salen Anticipadamente

Si 100 productores salen después de 1 año (25% del compromiso):

```
Penalización por productor = 1 DOLI × 75% = 0.75 DOLI
Total al pool = 100 × 0.75 = 75 DOLI
Productores restantes = 500
Bonus por bloque = 75 / 500 = 0.15 DOLI extra
```

---

## 9. Configuración Recomendada

### 9.1. Script de Lanzamiento de 600 Nodos

```bash
#!/bin/bash
# launch_600_nodes.sh

BASE_P2P_PORT=50303
BASE_RPC_PORT=28545
BASE_DATA_DIR=~/.doli/devnet-stress

# Nodo semilla (bootstrap)
./doli-node --network devnet run \
    --data-dir $BASE_DATA_DIR-0 \
    --p2p-port $BASE_P2P_PORT \
    --rpc-port $BASE_RPC_PORT \
    --producer &

BOOTSTRAP="/ip4/127.0.0.1/tcp/$BASE_P2P_PORT"

# Lanzar 599 nodos adicionales
for i in $(seq 1 599); do
    P2P_PORT=$((BASE_P2P_PORT + i))
    RPC_PORT=$((BASE_RPC_PORT + i))

    ./doli-node --network devnet run \
        --data-dir $BASE_DATA_DIR-$i \
        --p2p-port $P2P_PORT \
        --rpc-port $RPC_PORT \
        --bootstrap $BOOTSTRAP \
        --producer &

    # Pequeña pausa para no saturar
    if [ $((i % 50)) -eq 0 ]; then
        echo "Lanzados $i nodos..."
        sleep 1
    fi
done

echo "600 nodos lanzados"
wait
```

### 9.2. Requisitos de Hardware

Para 600 nodos en una sola máquina (solo para pruebas extremas):

| Recurso | Mínimo | Recomendado |
|---------|--------|-------------|
| CPU | 64 cores | 128 cores |
| RAM | 128 GB | 256 GB |
| Disco | 500 GB SSD | 1 TB NVMe |
| Red | 1 Gbps | 10 Gbps |

**Nota:** En producción real, cada nodo estaría en hardware separado.

### 9.3. Configuración de Recursos por Nodo

```toml
# config.toml para nodo de stress test
[resources]
max_memory_mb = 200          # 200 MB por nodo
max_open_files = 100         # Limitar file descriptors
vdf_threads = 1              # Un thread de VDF por nodo

[network]
max_peers = 12               # Reducir para 600 nodos
gossip_mesh_n = 4            # Mesh más pequeño
connection_timeout_ms = 5000

[mempool]
max_count = 1000             # Mempool reducido
max_size_bytes = 1048576     # 1 MB
```

---

## 10. Métricas a Monitorear

### 10.1. Prometheus Queries

```promql
# Bloques producidos por nodo
rate(doli_blocks_produced_total[5m])

# Distribución de tiempo de VDF
histogram_quantile(0.99, doli_vdf_duration_seconds_bucket)

# Peers conectados
doli_peers_connected

# Mempool size
doli_mempool_transactions

# Latencia de propagación de bloques
doli_block_propagation_seconds
```

### 10.2. Alertas Críticas

```yaml
# Alerta si producción de bloques cae
- alert: BlockProductionLow
  expr: rate(doli_chain_height[1m]) < 10
  for: 5m
  annotations:
    summary: "Block production below 10/minute"

# Alerta si hay muchas reorganizaciones
- alert: HighReorgRate
  expr: rate(doli_chain_reorgs_total[5m]) > 1
  for: 2m
  annotations:
    summary: "High reorganization rate detected"
```

---

## 11. Conclusiones

### 11.1. Límites del Protocolo con 600 Productores

| Aspecto | Comportamiento | Límite |
|---------|---------------|--------|
| Selección de productor | Funciona correctamente | Sin límite teórico |
| Propagación de bloques | ~360ms para red completa | Aceptable |
| VDF redundante | 99.83% desperdiciado | Ineficiente pero funcional |
| Economía | Recompensas muy diluidas | Diseño intencional |
| Seguridad | Requiere 301+ para atacar | Fuerte |

### 11.2. Qué Probar con 600 Productores

1. **Convergencia**: ¿Todos los nodos acuerdan la misma cadena?
2. **Fallbacks**: ¿Los rank 1 y 2 producen cuando rank 0 falla?
3. **Reorganizaciones**: ¿Cómo maneja la red splits temporales?
4. **Memoria**: ¿Los nodos permanecen estables en uso de RAM?
5. **Latencia**: ¿La propagación se mantiene bajo 1 segundo?

### 11.3. Escenarios NO Recomendados

- **10,000+ productores**: Gossip se degrada significativamente
- **VDF < 100ms**: Riesgo de forks frecuentes
- **Slot < 1s**: Propagación insuficiente

---

## Apéndice A: Fórmulas del Whitepaper Aplicadas

### A.1. Selección de Productor

```
semilla = HASH("SEED" || prev_hash || slot)
score(key, slot) = HASH(semilla || key)
producer = argmin(score) over all active producers
```

### A.2. Dificultad de Registro

```
D_E = (D_{E-1} + R_E) / 2
T_reg(E+1) = T_base × max(1, D_E / R_target)
```

### A.3. Bond por Era

```
B(H) = B_initial × 0.7^(era(H))
era(H) = floor(H / 2,102,400)
```

### A.4. Recompensa por Era

```
R(H) = R_initial × 0.5^(era(H))
R_initial = 5 DOLI (mainnet), 500 DOLI (devnet)
```

### A.5. Penalización por Salida Anticipada

```
penalty_pct = (time_remaining × 100) / T_commitment
return = bond × (100 - penalty_pct) / 100
```

### A.6. Regla de Selección de Cadena

```
1. Preferir mayor slot
2. Si empatan: preferir mayor altura
3. Si empatan: preferir menor hash
```

---

*Documento generado para stress testing del protocolo DOLI*
*No usar estos parámetros en redes con valor real*
