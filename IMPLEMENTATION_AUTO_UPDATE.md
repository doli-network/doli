# Plan de Implementación: DOLI Auto-Update System

## Estado: PLAN DE IMPLEMENTACIÓN v1.0
## Fecha: 2026-02-01

---

## 1. Resumen Ejecutivo

Este documento presenta el plan de implementación para el sistema de auto-actualización de DOLI, basado en la especificación `AUTO_UPDATE_SYSTEM.md` v3.0.

### 1.1 Alcance

El sistema incluye:
- **Maintainer Bootstrap**: Los primeros 5 productores registrados se convierten automáticamente en maintainers
- **Gobernanza On-Chain**: Transacciones para agregar/remover maintainers (3/5 multisig)
- **Votación Ponderada por Seniority**: Los productores veteranos tienen hasta 4x peso de voto
- **Actualizaciones Automáticas**: Con período de veto de 7 días y rollback automático
- **Soporte Hard Fork**: Mecanismo upgrade-at-height para cambios de protocolo

### 1.2 Estado Actual del Código

| Componente | Estado | Ubicación |
|------------|--------|-----------|
| Constantes básicas | ✅ Parcial | `crates/updater/src/lib.rs` |
| VoteTracker con pesos | ✅ Implementado | `crates/updater/src/vote.rs` |
| Download/Verify binarios | ✅ Implementado | `crates/updater/src/download.rs` |
| Apply/Rollback básico | ✅ Implementado | `crates/updater/src/apply.rs` |
| Test keys para devnet | ✅ Implementado | `crates/updater/src/test_keys.rs` |
| Parámetros por red | ✅ Parcial | `crates/core/src/network.rs` |
| MaintainerSet | ❌ No existe | - |
| Seniority calculation | ❌ No existe | - |
| Watchdog/Crash detection | ❌ No existe | - |
| Hard fork support | ❌ No existe | - |
| CLI commands | ❌ No existe | - |
| RPC endpoints | ❌ No existe | - |
| Node integration | ❌ No existe | - |

---

## 2. Parametrización por Ambiente

**PRINCIPIO CLAVE**: Todos los tiempos deben ser configurables por red para permitir testing rápido en devnet.

### 2.1 Tabla de Parámetros por Red

```
┌────────────────────────────┬─────────────────┬─────────────────┬─────────────────┐
│ Parámetro                  │ Mainnet         │ Testnet         │ Devnet          │
├────────────────────────────┼─────────────────┼─────────────────┼─────────────────┤
│ VETO_PERIOD                │ 7 días          │ 7 días          │ 60 segundos     │
│ GRACE_PERIOD               │ 48 horas        │ 48 horas        │ 30 segundos     │
│ MIN_VOTING_AGE             │ 30 días         │ 30 días         │ 60 segundos     │
│ CHECK_INTERVAL             │ 6 horas         │ 6 horas         │ 10 segundos     │
│ CRASH_THRESHOLD            │ 3 crashes       │ 3 crashes       │ 3 crashes       │
│ CRASH_WINDOW               │ 1 hora          │ 1 hora          │ 60 segundos     │
│ SENIORITY_MATURITY_BLOCKS  │ 4 años (bloques)│ 4 años (bloques)│ 576 bloques     │
│ SENIORITY_STEP_BLOCKS      │ 1 año (bloques) │ 1 año (bloques) │ 144 bloques     │
└────────────────────────────┴─────────────────┴─────────────────┴─────────────────┘
```

### 2.2 Implementación en `network.rs`

Agregar los siguientes métodos a `Network`:

```rust
impl Network {
    /// Grace period after update approval (seconds)
    pub fn grace_period_secs(&self) -> u64 {
        match self {
            Network::Mainnet => 48 * 3600,  // 48 hours
            Network::Testnet => 48 * 3600,  // Same as mainnet
            Network::Devnet => 30,          // 30 seconds
        }
    }

    /// Minimum producer age before voting is allowed (seconds)
    pub fn min_voting_age_secs(&self) -> u64 {
        match self {
            Network::Mainnet => 30 * 24 * 3600,  // 30 days
            Network::Testnet => 30 * 24 * 3600,  // Same as mainnet
            Network::Devnet => 60,               // 1 minute
        }
    }

    /// How often to check for updates (seconds)
    pub fn update_check_interval_secs(&self) -> u64 {
        match self {
            Network::Mainnet => 6 * 3600,   // 6 hours
            Network::Testnet => 6 * 3600,   // Same as mainnet
            Network::Devnet => 10,          // 10 seconds
        }
    }

    /// Crash window for rollback detection (seconds)
    pub fn crash_window_secs(&self) -> u64 {
        match self {
            Network::Mainnet => 3600,   // 1 hour
            Network::Testnet => 3600,   // Same as mainnet
            Network::Devnet => 60,      // 1 minute
        }
    }

    /// Blocks to reach full seniority (4x vote weight)
    pub fn seniority_maturity_blocks(&self) -> u64 {
        self.blocks_per_year() * 4  // 4 years in blocks
    }

    /// Blocks per seniority step (1 year)
    pub fn seniority_step_blocks(&self) -> u64 {
        self.blocks_per_year()  // 1 year in blocks
    }
}
```

---

## 3. Arquitectura de Componentes

### 3.1 Estructura de Archivos

```
crates/
├── core/src/
│   ├── maintainer.rs         # NEW: MaintainerSet, derivation logic
│   ├── transaction.rs        # MODIFY: Add RemoveMaintainer, AddMaintainer
│   ├── validation.rs         # MODIFY: Validate maintainer transactions
│   └── network.rs            # MODIFY: Add update parameters
│
├── updater/src/
│   ├── lib.rs                # MODIFY: Use network params, export new modules
│   ├── vote.rs               # EXISTS: Minor updates for seniority integration
│   ├── seniority.rs          # NEW: Calculate producer voting weights
│   ├── watchdog.rs           # NEW: Crash detection and auto-rollback
│   ├── hardfork.rs           # NEW: Upgrade-at-height mechanism
│   ├── apply.rs              # EXISTS: Add watchdog integration
│   ├── download.rs           # EXISTS: No changes needed
│   └── test_keys.rs          # EXISTS: No changes needed
│
├── storage/src/
│   ├── maintainer.rs         # NEW: Persist maintainer set
│   └── update.rs             # NEW: Persist update state, votes
│
├── rpc/src/
│   └── update.rs             # NEW: RPC endpoints for update system
│
├── network/src/
│   └── gossip.rs             # MODIFY: Add vote gossip topic
│
bins/
├── node/src/
│   └── updater.rs            # NEW: Node runtime integration
│
└── cli/src/
    ├── update.rs             # NEW: Update CLI commands
    └── maintainer.rs         # NEW: Maintainer CLI commands
```

### 3.2 Diagrama de Dependencias

```
                     ┌─────────────┐
                     │   doli-cli  │
                     └──────┬──────┘
                            │
         ┌──────────────────┼──────────────────┐
         │                  │                  │
         ▼                  ▼                  ▼
┌─────────────┐    ┌─────────────┐    ┌─────────────┐
│  doli-node  │    │   doli-rpc  │    │  doli-cli   │
└──────┬──────┘    └──────┬──────┘    │  commands   │
       │                  │           └─────────────┘
       ▼                  ▼
┌─────────────────────────────────────────────────┐
│                    updater                       │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐       │
│  │ seniority│  │ watchdog │  │ hardfork │       │
│  └──────────┘  └──────────┘  └──────────┘       │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐       │
│  │   vote   │  │  apply   │  │ download │       │
│  └──────────┘  └──────────┘  └──────────┘       │
└─────────────────────┬───────────────────────────┘
                      │
       ┌──────────────┼──────────────┐
       │              │              │
       ▼              ▼              ▼
┌───────────┐  ┌───────────┐  ┌───────────┐
│  storage  │  │   core    │  │  network  │
│           │  │maintainer │  │  gossip   │
└───────────┘  └───────────┘  └───────────┘
```

---

## 4. Milestones de Implementación

### Milestone 1: Parametrización por Red (Día 1)
**Objetivo**: Hacer todos los tiempos configurables por network

**Archivos a modificar**:
- `crates/core/src/network.rs` - Agregar métodos de parámetros
- `crates/updater/src/lib.rs` - Usar parámetros de Network en lugar de constantes

**Tareas**:
1. Agregar `grace_period_secs()` a Network
2. Agregar `min_voting_age_secs()` a Network
3. Agregar `update_check_interval_secs()` a Network
4. Agregar `crash_window_secs()` a Network
5. Agregar `seniority_maturity_blocks()` a Network
6. Agregar `seniority_step_blocks()` a Network
7. Refactorizar `updater/lib.rs` para recibir Network y usar sus parámetros
8. Tests unitarios

**Criterio de éxito**: `cargo test -p doli-core -p updater` pasa

---

### Milestone 2: Sistema de Seniority (Día 2)
**Objetivo**: Calcular peso de voto basado en antigüedad del productor

**Archivos nuevos**:
- `crates/updater/src/seniority.rs`

**Tareas**:
1. Crear struct `ProducerSeniority` con campos:
   - `pubkey: PublicKey`
   - `registration_height: u64`
   - `current_height: u64`
   - `network: Network`
2. Implementar `calculate_weight()`:
   ```rust
   // weight = 1.0 + min(years, 4) * 0.75
   // En bloques: years = (current - registration) / blocks_per_year
   pub fn calculate_weight(&self) -> f64 {
       let blocks_active = self.current_height.saturating_sub(self.registration_height);
       let years = (blocks_active as f64) / (self.network.blocks_per_year() as f64);
       let capped_years = years.min(4.0);
       1.0 + capped_years * 0.75
   }
   ```
3. Implementar `is_eligible_to_vote()` (min voting age check)
4. Crear `SeniorityCalculator` que toma storage y calcula pesos para todos los productores
5. Tests con diferentes antigüedades

**Criterio de éxito**: Tests verifican pesos correctos (1x a 4x)

---

### Milestone 3: MaintainerSet y Bootstrap (Día 3-4)
**Objetivo**: Sistema de maintainers derivado de la blockchain

**Archivos nuevos**:
- `crates/core/src/maintainer.rs`
- `crates/storage/src/maintainer.rs`

**Archivos a modificar**:
- `crates/core/src/transaction.rs` - Agregar TxType::RemoveMaintainer (13), AddMaintainer (14)
- `crates/core/src/lib.rs` - Exportar módulo maintainer

**Estructuras de datos**:
```rust
// core/src/maintainer.rs

pub const INITIAL_MAINTAINER_COUNT: usize = 5;
pub const MAINTAINER_THRESHOLD: usize = 3;
pub const MIN_MAINTAINERS: usize = 3;
pub const MAX_MAINTAINERS: usize = 5;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaintainerSet {
    pub members: Vec<PublicKey>,
    pub threshold: usize,
    pub last_updated: u64,  // Block height
}

impl MaintainerSet {
    pub fn is_maintainer(&self, pubkey: &PublicKey) -> bool;
    pub fn can_remove(&self) -> bool;  // members.len() > MIN_MAINTAINERS
    pub fn can_add(&self) -> bool;     // members.len() < MAX_MAINTAINERS
    pub fn verify_multisig(&self, signatures: &[Signature], message: &[u8]) -> bool;
    pub fn calculate_threshold(member_count: usize) -> usize;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaintainerChangeData {
    pub target: PublicKey,
    pub signatures: Vec<MaintainerSignature>,
    pub reason: Option<String>,
}

pub struct MaintainerSignature {
    pub pubkey: PublicKey,
    pub signature: Signature,
}
```

**Tareas**:
1. Crear `MaintainerSet` struct con métodos
2. Crear función `derive_maintainer_set(chain: &impl BlockchainReader)` que:
   - Escanea registrations desde genesis
   - Toma los primeros 5 como maintainers
   - Procesa transacciones Add/Remove posteriores
3. Agregar `TxType::RemoveMaintainer = 13` y `AddMaintainer = 14`
4. Crear structs de datos para las transacciones
5. Implementar validación de transacciones de maintainer
6. Crear storage para persistir MaintainerSet
7. Tests de bootstrap y modificación

**Criterio de éxito**: Tests verifican derivación correcta y cambios de maintainer

---

### Milestone 4: Watchdog y Auto-Rollback (Día 5)
**Objetivo**: Detectar crashes post-update y hacer rollback automático

**Archivos nuevos**:
- `crates/updater/src/watchdog.rs`

**Estructuras de datos**:
```rust
// updater/src/watchdog.rs

pub struct UpdateWatchdog {
    last_update_version: Option<String>,
    last_update_time: Option<u64>,
    crash_count: u32,
    crash_timestamps: Vec<u64>,
    network: Network,
}

impl UpdateWatchdog {
    pub fn new(network: Network) -> Self;
    pub fn record_crash(&mut self, timestamp: u64);
    pub fn record_update(&mut self, version: String, timestamp: u64);
    pub fn should_rollback(&self) -> bool;
    pub fn clear_crash_history(&mut self);

    fn within_crash_window(&self) -> bool {
        // Usa self.network.crash_window_secs()
    }
}
```

**Tareas**:
1. Crear `UpdateWatchdog` struct
2. Implementar detección de crashes dentro de ventana
3. Implementar lógica de rollback:
   - 3+ crashes en crash_window → trigger rollback
   - Guardar backup antes de update
   - Restaurar backup en rollback
4. Integrar con `apply.rs`
5. Persistir estado del watchdog
6. Tests de detección y rollback

**Criterio de éxito**: Tests simulan crashes y verifican rollback

---

### Milestone 5: Hard Fork Support (Día 6)
**Objetivo**: Soporte para upgrades que requieren altura de activación

**Archivos nuevos**:
- `crates/updater/src/hardfork.rs`

**Estructuras de datos**:
```rust
// updater/src/hardfork.rs

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HardForkInfo {
    pub activation_height: u64,
    pub min_version: String,
    pub consensus_changes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HardForkRelease {
    pub version: String,
    pub binary_sha256: String,
    pub changelog: String,
    pub hard_fork: HardForkInfo,
    pub signatures: Vec<MaintainerSignature>,
}

impl HardForkRelease {
    pub fn is_hard_fork(&self) -> bool;
    pub fn blocks_until_activation(&self, current_height: u64) -> u64;
    pub fn is_activated(&self, current_height: u64) -> bool;
}
```

**Tareas**:
1. Crear `HardForkInfo` y `HardForkRelease` structs
2. Extender `Release` para soportar campo `hard_fork: Option<HardForkInfo>`
3. Implementar lógica de activación por altura
4. Agregar hook para recalcular scheduler en activación
5. CLI para mostrar info de hard fork pendiente
6. Tests de activación

**Criterio de éxito**: Tests verifican activación en altura correcta

---

### Milestone 6: Storage para Updates (Día 7)
**Objetivo**: Persistir estado de actualizaciones y votos

**Archivos nuevos**:
- `crates/storage/src/update.rs`

**Tareas**:
1. Crear `UpdateStore` con:
   - Releases pendientes
   - Votos por versión
   - Estado de enforcement
   - Historial de updates aplicados
2. Column families en RocksDB:
   - `cf_pending_releases`
   - `cf_votes`
   - `cf_update_history`
   - `cf_maintainer_set`
3. Integrar con `doli-storage` existente
4. Tests de persistencia

**Criterio de éxito**: Tests verifican persistencia y recuperación

---

### Milestone 7: Gossip para Votos (Día 8)
**Objetivo**: Propagar votos entre nodos via gossipsub

**Archivos a modificar**:
- `crates/network/src/gossip.rs`

**Tareas**:
1. Agregar topic `update_votes` para gossip de votos
2. Crear handlers para recibir/enviar votos
3. Validar votos recibidos (firma, elegibilidad)
4. Deduplicar votos (solo el más reciente por productor)
5. Tests de propagación

**Criterio de éxito**: Votos se propagan entre nodos de test

---

### Milestone 8: RPC Endpoints (Día 9)
**Objetivo**: Exponer funcionalidad via JSON-RPC

**Archivos nuevos**:
- `crates/rpc/src/update.rs`

**Endpoints**:
```rust
// getMaintainerSet
// getUpdateStatus
// submitVote
// submitMaintainerChange
// getVoteStatus
```

**Tareas**:
1. Implementar `getMaintainerSet` - retorna maintainers actuales
2. Implementar `getUpdateStatus` - retorna release pendiente y estado
3. Implementar `submitVote` - acepta voto firmado
4. Implementar `submitMaintainerChange` - acepta cambio de maintainer
5. Implementar `getVoteStatus` - retorna votos por versión
6. Integrar con router Axum existente
7. Tests de cada endpoint

**Criterio de éxito**: Tests de integración pasan para cada endpoint

---

### Milestone 9: CLI Commands (Día 10-11)
**Objetivo**: Comandos para gestionar updates y maintainers

**Archivos nuevos**:
- `bins/cli/src/update.rs`
- `bins/cli/src/maintainer.rs`

**Comandos Update**:
```bash
doli-node update check          # Verificar updates disponibles
doli-node update status         # Estado detallado
doli-node update apply          # Aplicar update aprobado
doli-node update rollback       # Rollback manual
doli-node update verify --version X.Y.Z  # Verificar firmas
doli-node update vote --veto --version X.Y.Z --key <path>
doli-node update vote --approve --version X.Y.Z --key <path>
doli-node update votes --version X.Y.Z  # Ver votos actuales
```

**Comandos Maintainer**:
```bash
doli-node maintainer list       # Listar maintainers
doli-node maintainer verify --pubkey <key>  # Verificar si es maintainer
doli-node maintainer remove --target <key> --key <signer>
doli-node maintainer add --target <key> --key <signer>
doli-node maintainer sign --proposal-id <id> --key <signer>
```

**Tareas**:
1. Crear subcomando `update` con opciones
2. Crear subcomando `maintainer` con opciones
3. Implementar cada comando
4. Formatear salida con banners informativos
5. Tests de CLI

**Criterio de éxito**: Cada comando funciona correctamente

---

### Milestone 10: Node Integration (Día 12-13)
**Objetivo**: Integrar todo en el runtime del nodo

**Archivos nuevos**:
- `bins/node/src/updater.rs`

**Tareas**:
1. Crear `UpdateManager` que:
   - Inicializa watchdog
   - Carga maintainer set
   - Inicia check periódico
   - Maneja votos recibidos
2. Integrar con `Node` struct existente
3. Hook en block_applied para:
   - Actualizar seniority cache
   - Verificar activación de hard forks
   - Calcular resultados de votación
4. Banner de notificación en CLI
5. Logging apropiado
6. Tests de integración

**Criterio de éxito**: Nodo arranca con sistema de updates funcional

---

### Milestone 11: Testing End-to-End (Día 14-15)
**Objetivo**: Validar flujo completo en devnet

**Scripts de test**:
- `scripts/test_update_veto.sh` - Test de período de veto
- `scripts/test_maintainer_bootstrap.sh` - Test de bootstrap
- `scripts/test_rollback.sh` - Test de rollback automático
- `scripts/test_hard_fork.sh` - Test de activación de hard fork

**Tareas**:
1. Crear script de test de veto con múltiples nodos
2. Crear script de test de bootstrap de maintainers
3. Crear script de test de rollback
4. Crear script de test de hard fork
5. Documentar proceso de testing

**Criterio de éxito**: Todos los scripts pasan en devnet

---

### Milestone 12: Documentación (Día 16)
**Objetivo**: Documentar sistema para usuarios y desarrolladores

**Archivos a crear/modificar**:
- `docs/auto_update.md` - Guía de usuario
- `docs/rpc_reference.md` - Agregar endpoints
- `docs/cli.md` - Agregar comandos
- `specs/protocol.md` - Agregar sección de governance

**Tareas**:
1. Escribir guía de usuario para productores
2. Documentar endpoints RPC
3. Documentar comandos CLI
4. Actualizar spec de protocolo
5. Review de documentación

**Criterio de éxito**: Documentación completa y clara

---

## 5. Estimación de Esfuerzo

| Milestone | Días | Complejidad | Riesgo |
|-----------|------|-------------|--------|
| 1. Parametrización | 1 | Baja | Bajo |
| 2. Seniority | 1 | Media | Bajo |
| 3. MaintainerSet | 2 | Alta | Medio |
| 4. Watchdog | 1 | Media | Bajo |
| 5. Hard Fork | 1 | Media | Medio |
| 6. Storage | 1 | Media | Bajo |
| 7. Gossip | 1 | Media | Medio |
| 8. RPC | 1 | Media | Bajo |
| 9. CLI | 2 | Media | Bajo |
| 10. Node Integration | 2 | Alta | Alto |
| 11. E2E Testing | 2 | Alta | Medio |
| 12. Documentación | 1 | Baja | Bajo |
| **Total** | **16 días** | | |

---

## 6. Criterios de Aceptación Globales

### 6.1 Funcionalidad
- [ ] Maintainers se derivan correctamente de los primeros 5 registros
- [ ] Cambios de maintainer requieren 3/5 firmas
- [ ] Votos se ponderan correctamente por seniority
- [ ] Período de veto es configurable por red
- [ ] Rollback automático funciona tras 3 crashes
- [ ] Hard forks se activan en altura correcta
- [ ] CLI muestra banners informativos
- [ ] RPC endpoints funcionan correctamente

### 6.2 Testing
- [ ] Cobertura de tests unitarios > 80%
- [ ] Tests de integración pasan
- [ ] Scripts E2E pasan en devnet
- [ ] Tests de regresión no fallan

### 6.3 Documentación
- [ ] Guía de usuario completa
- [ ] RPC documentado
- [ ] CLI documentado
- [ ] Spec actualizado

### 6.4 Código
- [ ] `cargo clippy` sin warnings
- [ ] `cargo fmt --check` pasa
- [ ] `cargo test` pasa
- [ ] Sin dependencias nuevas innecesarias

---

## 7. Riesgos y Mitigaciones

| Riesgo | Impacto | Probabilidad | Mitigación |
|--------|---------|--------------|------------|
| Deriva de maintainer set entre nodos | Alto | Medio | Derivación determinística desde genesis |
| Ataques Sybil en votación | Alto | Bajo | Seniority weighting + min voting age |
| Rollback infinito | Medio | Bajo | Límite de intentos, notificación a operador |
| Hard fork incompleto | Alto | Medio | Testing exhaustivo en devnet |
| Vulnerabilidad en verificación de firmas | Crítico | Bajo | Usar crypto crate existente, tests exhaustivos |

---

## 8. Decisiones de Diseño Clave

### 8.1 ¿Por qué derivar maintainers de la chain?
- **Verificabilidad**: Cualquier nodo puede verificar independientemente
- **Sin hardcoding**: No hay configuración externa que pueda divergir
- **Auditabilidad**: Historial completo on-chain

### 8.2 ¿Por qué seniority en lugar de stake para votos?
- **Separación de concerns**: Stake → producción de bloques, Tiempo → gobernanza
- **Anti-plutocracy**: Los ricos no dominan la gobernanza inmediatamente
- **Convergencia**: Después de 4 años, todos son iguales

### 8.3 ¿Por qué 40% para veto en lugar de 33%?
- **Costo de ataque**: 40% requiere más nodos para bloquear
- **Balance**: Suficientemente alto para evitar Sybil, suficientemente bajo para democracia

### 8.4 ¿Por qué parámetros diferentes en devnet?
- **Testing rápido**: 7 días de veto es impráctico para desarrollo
- **Misma lógica**: El código es idéntico, solo cambian los tiempos
- **Realismo**: Devnet usa 1s slots pero misma estructura

---

## 9. Próximos Pasos

1. **Revisión del plan**: Validar con stakeholders
2. **Comenzar Milestone 1**: Parametrización por red
3. **CI/CD**: Agregar tests de update system a CI
4. **Comunicación**: Documentar progreso en cada milestone

---

*Documento generado: 2026-02-01*
*Autor: Claude Opus 4.5*
*Basado en: AUTO_UPDATE_SYSTEM.md v3.0*
