# DOLI

## Un Sistema de Dinero Electrónico Entre Pares

**Contacto:** doli@protonmail.com
**Web:** www.doli.network

---

## Resumen

Se propone un sistema de dinero electrónico puramente entre pares que permite enviar pagos directamente de una persona a otra sin pasar por una institución financiera. El problema del doble gasto se resuelve mediante una red de pares que ancla el consenso a trabajo secuencial verificable.

El recurso escaso no es energía ni capital permanente, sino tiempo. Las transacciones se ordenan en una cadena de bloques donde cada bloque requiere una Función de Retardo Verificable (VDF): un cómputo que toma tiempo fijo y no puede acelerarse mediante paralelización.

Para participar en la producción de bloques, los nodos deben registrarse mediante un proceso que consume tiempo secuencial y requiere un bond de activación. El tiempo limita la velocidad de creación de identidades; el bond limita su proliferación económica.

El bond decrece 30% por era mientras la recompensa decrece 50%. Esto hace que incorporarse como productor sea progresivamente más costoso en términos relativos, ya que el bond cae más lento que la recompensa.

El resultado es un sistema donde el cómputo por identidad no puede acelerarse mediante paralelización, donde la tasa de nuevas identidades está regulada por tiempo, y donde cada identidad tiene un costo económico irrecuperable ante comportamiento malicioso.

---

## 1. Introducción

El comercio en Internet depende casi exclusivamente de instituciones financieras que sirven como terceros de confianza para procesar pagos. Aunque el sistema funciona razonablemente bien para la mayoría de las transacciones, sufre de debilidades inherentes al modelo basado en confianza.

Las transacciones pueden revertirse, lo que aumenta los costos de mediación y limita el tamaño mínimo práctico de una transacción. La posibilidad de reversión genera la necesidad de que los comerciantes soliciten más información de la necesaria a sus clientes. Un cierto porcentaje de fraude se acepta como inevitable.

Estos costos e incertidumbres pueden evitarse usando dinero físico, pero no existe mecanismo para realizar pagos a través de un canal de comunicación sin depender de un tercero de confianza.

Lo que se necesita es un sistema de pago electrónico basado en prueba criptográfica en lugar de confianza, que permita a dos partes dispuestas realizar transacciones directamente entre sí sin necesidad de un tercero.

Este documento propone una solución al problema del doble gasto utilizando una red de pares que ancla el consenso al paso del tiempo mediante funciones de retardo verificable. El sistema es seguro mientras los participantes honestos controlen colectivamente más capacidad de cómputo secuencial que cualquier grupo de atacantes.

### 1.1. Contribución única

DOLI es el primer blockchain donde las Funciones de Retardo Verificable (VDF) constituyen el mecanismo primario de consenso, no un complemento.

**Comparación con sistemas existentes:**

| Sistema   | Recurso escaso    | Rol de VDF                        |
|-----------|-------------------|-----------------------------------|
| Bitcoin   | Energía           | No utiliza                        |
| Ethereum  | Capital (stake)   | No utiliza                        |
| Chia      | Espacio en disco  | VDF secundario para sincronizar   |
| Solana    | Capital (stake)   | VDF auxiliar para ordenar eventos |
| **DOLI**  | **Tiempo**        | **VDF es el consenso primario**   |

En Chia, la Proof of Space determina quién gana el bloque; la VDF solo sincroniza los tiempos. En Solana, el stake determina los validadores; la VDF (Proof of History) solo ordena eventos dentro de un slot. En ambos casos, la VDF es auxiliar.

En DOLI, el tiempo secuencial es el único recurso que determina el consenso. El bond económico es un filtro anti-Sybil, no un mecanismo de selección. Esto representa un cambio paradigmático: de "quien tiene más recursos gana" a "el tiempo pasa igual para todos".

---

## 2. Transacciones

Definimos una moneda electrónica como una cadena de firmas digitales. Cada propietario transfiere la moneda al siguiente firmando digitalmente un hash de la transacción anterior y la llave pública del siguiente propietario, y agregando estos al final de la moneda. Un beneficiario puede verificar las firmas para verificar la cadena de propiedad.

```
┌─────────────────────────────────┐
│         Transacción             │
├─────────────────────────────────┤
│  Hash de TX anterior            │
│  Llave pública del receptor     │
│  Firma del propietario          │
└─────────────────────────────────┘
                │
                ▼
┌─────────────────────────────────┐
│         Transacción             │
├─────────────────────────────────┤
│  Hash de TX anterior            │
│  Llave pública del receptor     │
│  Firma del propietario          │
└─────────────────────────────────┘
```

El problema, por supuesto, es que el beneficiario no puede verificar que uno de los propietarios no haya gastado la moneda dos veces. Una solución común es introducir una autoridad central de confianza que verifique cada transacción. Después de cada transacción, la moneda debe regresar a la autoridad para emitir una nueva, y solo las monedas emitidas directamente por la autoridad se consideran válidas.

El problema con esta solución es que el destino de todo el sistema monetario depende de la entidad que opera la autoridad, y cada transacción debe pasar por ella.

Necesitamos una forma de que el beneficiario sepa que los propietarios anteriores no firmaron transacciones previas. Para nuestros propósitos, la transacción más temprana es la que cuenta, así que no nos importan los intentos posteriores de doble gasto. La única forma de confirmar la ausencia de una transacción es estar al tanto de todas las transacciones. Para lograr esto sin una parte de confianza, las transacciones deben anunciarse públicamente, y necesitamos un sistema en el que los participantes acuerden un único historial del orden en que fueron recibidas.

### 2.1. Modelo de salidas no gastadas

El sistema utiliza un modelo donde cada transacción consume salidas de transacciones anteriores y crea nuevas salidas.

```
transacción = {
    versión: entero,
    tipo: entero,                    // 0 = transferencia, 1 = registro, 2 = salida,
                                     // 3 = reclamo_recompensa, 4 = reclamo_bond,
                                     // 5 = penalización_productor
    entradas: [
        {
            hash_tx_anterior: 32 bytes,
            índice_salida: entero,
            firma: 64 bytes
        },
        ...
    ],
    salidas: [
        {
            cantidad: entero,
            hash_llave_pública: 32 bytes
        },
        ...
    ],
    datos_adicionales: bytes
}
```

Una transacción es válida si:

1. Cada entrada referencia una salida existente y no gastada.
2. La firma corresponde a la llave pública de la salida referenciada.
3. La suma de las entradas es mayor o igual a la suma de las salidas.
4. Todas las cantidades son positivas.

La diferencia entre entradas y salidas constituye la comisión para el productor del bloque.

### 2.2. Firmas digitales

El sistema utiliza el algoritmo Ed25519 para firmas digitales. Este algoritmo proporciona firmas de 64 bytes con llaves públicas de 32 bytes.

El mensaje firmado es el hash de la transacción sin incluir las firmas.

### 2.3. Direcciones

Una dirección es el hash de una llave pública:

```
dirección = HASH(llave_pública)[0:20]
```

Para gastar una salida, el propietario debe revelar la llave pública completa junto con una firma válida.

### 2.4. Comisiones

```
comisión = suma(entradas) - suma(salidas)
```

La comisión mínima se calcula como:

```
comisión_mínima = tamaño_en_bytes × tarifa_base
```

Donde tarifa_base es un parámetro fijo del protocolo.

### 2.5. Definiciones de codificación

`HASH(x) = BLAKE3-256(x)`, salida de 32 bytes.

Las cadenas literales (por ejemplo "BLK", "REG", "SEED") se codifican en ASCII sin terminador NUL.

Los enteros se codifican en little-endian. Por defecto se usa uint32 (4 bytes) para ranura, época, índices y campos de control; y uint64 (8 bytes) para cantidad y otros campos monetarios.

Los mensajes de hash se construyen por concatenación exacta de bytes, sin separadores implícitos.

---

## 3. Servidor de tiempo

La solución que proponemos comienza con un servidor de tiempo distribuido. La red actúa como un servidor de tiempo: toma el hash de un bloque de elementos a marcar y publica ampliamente el hash. La marca de tiempo demuestra que los datos existían en ese momento para poder entrar en el hash.

```
                     ┌──────────────────┐
                     │      Bloque      │
                     ├──────────────────┤
                     │  Hash anterior   │
                     │  Marca de tiempo │
 Transacciones ───▶  │  Transacciones   │
                     │  Prueba VDF      │
                     └──────────────────┘
                              │
                              ▼
                     ┌──────────────────┐
                     │      Bloque      │
                     ├──────────────────┤
                     │  Hash anterior   │
                     │  Marca de tiempo │
                     │  Transacciones   │
                     │  Prueba VDF      │
                     └──────────────────┘
```

Cada marca de tiempo incluye la marca anterior en su hash, formando una cadena. Cada marca adicional refuerza las anteriores.

---

## 4. Prueba de Tiempo (Proof of Time)

> **Nota técnica:** Una VDF no prueba directamente que "pasó tiempo", sino que se ejecutaron N operaciones secuenciales. Sin embargo, dado que no existe forma conocida de acelerar esta computación mediante paralelización, el tiempo es el límite inferior efectivo. El término "Proof of Time" refleja esta propiedad.

Para implementar un servidor de tiempo distribuido entre pares, necesitamos un mecanismo que haga costoso producir bloques y que ese costo no pueda evadirse mediante paralelización o acumulación de recursos.

La solución es utilizar Funciones de Retardo Verificable (VDF). Una VDF es una función que:

1. Requiere un número fijo de operaciones secuenciales para computarse.
2. Produce una prueba que puede verificarse rápidamente.
3. No puede acelerarse significativamente mediante paralelización.

Para cada bloque, el productor debe calcular:

```
entrada = HASH("BLK" || hash_anterior || raíz_transacciones ||
               ranura || llave_productor)

salida_vdf = VDF(entrada, T)

prueba = π
```

Donde T es el parámetro de dificultad que determina cuánto tiempo toma el cómputo.

### 4.0.1. Implementación VDF: Hash-Chain VDF

DOLI utiliza una VDF basada en cadena de hashes (hash-chain) junto con **Epoch Lookahead** para prevenir grinding.

**Dos Tipos de Proof of Time:**

```
┌─────────────────────────────────────────────────────────────────┐
│  PROOF OF TIME: DOS DIMENSIONES TEMPORALES                      │
│                                                                 │
│  1. TIEMPO INMEDIATO (VDF ~700ms):                              │
│     - Heartbeat que prueba presencia AHORA                      │
│     - Latido continuo de actividad                              │
│                                                                 │
│  2. TIEMPO HISTÓRICO (Longevidad):                              │
│     - Derechos ganados a lo largo de semanas/meses/años         │
│     - Presencia demostrada a través de consistencia             │
│     - La verdadera "Proof of Sequence"                          │
└─────────────────────────────────────────────────────────────────┘
```

**Epoch Lookahead (Anti-Grinding):**

El grinding se previene con selección determinista al inicio del epoch:

```
┌─────────────────────────────────────────────────────────────────┐
│  EPOCH LOOKAHEAD: SELECCIÓN DETERMINISTA                        │
│                                                                 │
│  líder_slot = slot % total_bonos                                │
│                                                                 │
│  - Selección basada en número de slot + distribución de bonos   │
│  - NO depende de prev_hash → NO grinding posible                │
│  - Líderes efectivamente determinados al inicio del epoch       │
│  - Atacante no puede influir en selección futura                │
└─────────────────────────────────────────────────────────────────┘
```

**Funcionamiento VDF:**

```
entrada = HASH("DOLI_HEARTBEAT_V1" || llave_productor || ranura || hash_anterior)
salida = HASH^n(entrada)  // ~10M iteraciones (~700ms)
```

Donde `HASH^n` significa aplicar la función hash n veces secuencialmente.

**Parámetros por Red:**

| Red      | Slot   | VDF Heartbeat | Propósito                    |
|----------|--------|---------------|------------------------------|
| Todas    | 60/10/5s | ~700ms      | Prueba de presencia continua |

**Flujo de Producción:**

```
┌─────────────────────────────────────────────────────────────────┐
│  SLOT N (10 segundos en mainnet)                                │
│                                                                 │
│  0s     Slot comienza, verificar si somos líder                 │
│         ↓                                                       │
│         INICIA VDF(prev_hash) como heartbeat                    │
│         │                                                       │
│  ~700ms VDF completo → construir bloque                         │
│  ~1s    Broadcast bloque                                        │
│         │  ← resto del slot para propagación                    │
│  60s    Slot termina, SLOT N+1 comienza...                      │
└─────────────────────────────────────────────────────────────────┘
```

**Requisitos de Hardware:**

El VDF heartbeat (~700ms) tiene requisitos mínimos:
- Cualquier CPU moderna (2015+)
- Intel i3+, AMD Ryzen 3+, Apple M1+
- Funciona incluso en hardware modesto

**Trade-offs:**
- ✓ Seguro contra grinding (Epoch Lookahead)
- ✓ Proof of Time basado en longevidad histórica
- ✓ Hardware accesible (VDF es solo heartbeat)
- ✓ Barrera de entrada baja (similar a PoS)

### 4.0.2. Calibración Dinámica de VDF

Para mantener tiempos consistentes (~700ms) independientemente del hardware, DOLI implementa calibración dinámica de iteraciones.

**Sistema de Calibración:**

```
ITERACIONES_DEFAULT = 10.000.000   // ~700ms
ITERACIONES_MIN = 100.000          // Mínimo seguro
ITERACIONES_MAX = 100.000.000      // Máximo permitido
TIEMPO_OBJETIVO = 700ms
TOLERANCIA = 10%
AJUSTE_MAX = 20% por ciclo
```

**Algoritmo:**

1. Al iniciar nodo productor: calibración inicial con 1M iteraciones de prueba
2. Medir tiempo real de VDF en cada bloque producido
3. Calcular tasa: `iteraciones / milisegundos`
4. Si desviación > 10%: ajustar iteraciones (máximo ±20% por ciclo)
5. Recalibrar cada 10 segundos mínimo

**Ejemplo:**

```
Hardware rápido: 10M iter = 500ms → aumentar a 14M iter
Hardware lento:  10M iter = 1400ms → reducir a 7M iter
```

Esto permite que hardware heterogéneo participe manteniendo tiempos de bloque consistentes.

La verificación requiere recomputar la cadena de hashes. Cualquier nodo puede verificar que la salida corresponde a la entrada repitiendo el cómputo secuencial.

El sistema utiliza VDF de cadena de hashes (SHA-256 iterado) en todas las redes con calibración dinámica para mantener un tiempo objetivo de ~700ms. Esta construcción está bien estudiada y proporciona fuertes garantías de secuencialidad.

### 4.1. Estructura temporal

**TIEMPO GÉNESIS:**

```
GENESIS_TIME = 2026-02-01T00:00:00Z (UTC)
             = 1769904000 (Unix timestamp)
```

**RANURA (slot):**
- Duración: 10 segundos
- Derivación: `ranura = floor((timestamp - GENESIS_TIME) / 10)`

La ranura NO es un campo libre. Se deriva determinísticamente del timestamp del bloque. Esto ancla el consenso al tiempo real.

**ÉPOCA (epoch):**
- Duración: 360 ranuras (1 hora)
- Se actualiza el conjunto de productores activos

| Unidad    | Ranuras     | Tiempo      |
|-----------|-------------|-------------|
| 1 ranura  | 1           | 10 segundos |
| 1 época   | 360         | 1 hora      |
| 1 día     | 8.640       | 24 horas    |
| 1 semana  | 60.480      | 7 días      |
| 1 año     | 3.153.600   | 365,25 días |
| 1 era     | 12.614.400  | ~4 años     |

### 4.2. Parámetro de dificultad

El parámetro T se calibra para que el cómputo tome aproximadamente 700ms (heartbeat VDF) en hardware de consumo común. Esto deja margen para propagación de red dentro de la ranura de 10 segundos.

A diferencia de sistemas donde la dificultad se ajusta dinámicamente, en DOLI el parámetro T está codificado en el protocolo con escalado automático por era.

**ESCALADO AUTOMÁTICO DE T:**

El hardware mejora ~20-50% por año. Para mantener los bloques como trabajo significativo, T aumenta automáticamente:

| Era | Años   | T (iteraciones) | Tiempo estimado* |
|-----|--------|-----------------|------------------|
| 0-1 | 0-8    | 55.000.000      | ~55 segundos     |
| 2-3 | 8-16   | 82.500.000      | ~55 segundos     |
| 4-5 | 16-24  | 110.000.000     | ~55 segundos     |
| 6-7 | 24-32  | 137.500.000     | ~55 segundos     |
| 36+ | 144+   | 550.000.000     | ~55 segundos (cap) |

*Con hardware promedio de cada época.

**FÓRMULA:**

```
T(H) = min(T_cap, T_base × (4 + era(H)) / 4)
```

Donde `era(H) = floor(H / 12.614.400)`.

Esto equivale a un aumento del 50% cada 2 eras (~8 años), alineado con la Ley de Moore histórica.

**JUSTIFICACIÓN:**

1. **Predecible:** El escalado está codificado desde génesis.
2. **Conservador:** El aumento es menor que la mejora histórica de hardware.
3. **Limitado:** T_cap previene tiempos de bloque excesivos.
4. **Sin gobernanza:** No requiere votación ni coordinación.

**VENTANA DE ACEPTACIÓN:**

Un bloque para ranura s es válido solo si su timestamp cae dentro de la ventana:

```
slot_start = GENESIS_TIME + (s × 10)

timestamp >= slot_start + (10 - MARGEN_RED)
timestamp <= slot_start + 10 + DERIVA
```

Donde `MARGEN_RED = 15 segundos` (parámetro del protocolo).

Esto impide que hardware acelerado adelante el reloj del consenso, y que bloques "tardíos" contaminen slots posteriores.

**POLÍTICA DE AJUSTE DE T:**

T puede aumentar (nunca disminuir) mediante activación programada basada únicamente en datos de la cadena:

1. Para cada bloque, calcular: `offset = timestamp - (GENESIS_TIME + ranura × 10)`
2. Al final de cada era, contar bloques donde `offset < UMBRAL_RAPIDO`.
3. Si más del 30% de bloques tienen `offset < UMBRAL_RAPIDO`, se activa una ventana de actualización.
4. Durante las siguientes 1.000 ranuras, los clientes aceptan `T_nuevo = T_actual × 1.2` (aumento del 20%).

Donde `UMBRAL_RAPIDO = 40 segundos`.

Esta regla es 100% determinista: depende solo de timestamps en bloques, no del reloj del validador. No requiere votación ni gobernanza.

### 4.3. Arranque de la red (bootstrapping)

El modo de arranque se define por altura de bloque y no por tiempo de reloj ni por número de participantes. Todas las reglas especiales de arranque aplican únicamente a bloques con altura inferior a BLOQUES_ARRANQUE.

```
BLOQUES_ARRANQUE = 10.080   (~1 semana si no hay ranuras vacías)
```

**Durante el modo arranque (H < BLOQUES_ARRANQUE):**

1. **PRODUCCIÓN ABIERTA:** Cualquier nodo puede producir bloques si completa la VDF correctamente. No se requiere registro previo ni bond.
2. **ANTI-SPAM:** Si múltiples bloques válidos llegan para el mismo slot, se acepta únicamente el de MENOR hash de encabezado. Los demás son inválidos.
3. **REGISTROS PARALELOS:** Durante este período, los nodos interesados completan sus VDF de registro y publican transacciones de registro con bond.
4. **CONSTRUCCIÓN DEL SET:** Al final de cada época, se actualiza el conjunto de productores activos con los registros confirmados.

**A partir del bloque H = BLOQUES_ARRANQUE:**

- Se activa el set de productores
- Se requiere registro válido con VDF
- Se requiere bond
- Se aplica selección determinista (sección 7)
- Solo productores registrados pueden producir

Este mecanismo permite que la red arranque sin depender de una lista inicial de productores "de confianza", manteniendo la filosofía de acceso abierto.

**TRADEOFF DE DISTRIBUCIÓN INICIAL:**

Durante el período de arranque, los primeros participantes obtienen una ventaja:

| Factor                        | Impacto                                    |
|-------------------------------|-------------------------------------------|
| Monedas minadas               | ~50,400 DOLI (10,080 bloques × 5)         |
| Valor de mercado              | Cercano a cero (no hay mercado aún)       |
| Duración de la ventaja        | ~1 semana                                 |

**ALTERNATIVAS CONSIDERADAS Y RECHAZADAS:**

| Método          | Problema                                        |
|-----------------|------------------------------------------------|
| Pre-mine        | Centralización, requiere confianza en fundadores|
| ICO/Venta       | Problemas legales, excluye a no-inversores     |
| Airdrop         | ¿A quién? Susceptible a sybil                  |
| Lista cerrada   | Centralización, requiere selección arbitraria  |

**JUSTIFICACIÓN:**

1. Bitcoin tuvo exactamente el mismo "problema" (Satoshi minó ~1M BTC).
2. Las monedas tempranas tienen valor especulativo mínimo sin mercado.
3. El bond de 1,000 DOLI limita cuántos productores pueden registrarse temprano.
4. El sistema es completamente transparente: todos saben las reglas desde el inicio.
5. Cualquiera puede participar desde el bloque 0 con el mismo costo (tiempo VDF).

Este es un tradeoff consciente: preferimos un arranque abierto y honesto sobre mecanismos que requieran confianza en una autoridad central.

---

## 5. Red

Los pasos para operar la red son los siguientes:

1. Las transacciones nuevas se difunden a todos los nodos.
2. Cada productor elegible recolecta transacciones en un bloque.
3. El productor asignado a la ranura calcula la prueba VDF.
4. El productor difunde el bloque a la red.
5. Los nodos aceptan el bloque si todas las transacciones son válidas y la prueba VDF es correcta.
6. Los nodos expresan su aceptación del bloque trabajando en crear el siguiente bloque, usando el hash del bloque aceptado como hash anterior.

Los nodos siempre consideran la cadena que cubre más tiempo como la correcta y continúan trabajando para extenderla. Si dos nodos difunden versiones diferentes del siguiente bloque simultáneamente, algunos nodos pueden recibir una u otra primero. En ese caso, trabajan en la primera que recibieron, pero guardan la otra rama en caso de que se vuelva más larga. El empate se rompe cuando se produce el siguiente bloque y una rama cubre más ranuras; los nodos que estaban trabajando en la otra rama cambian a la más larga.

### 5.1. Reglas de validez de bloque

Un bloque B es VÁLIDO si cumple:

1. **TIEMPO:** `B.timestamp > prev_block.timestamp` y `B.timestamp <= tiempo_red + DERIVA`
2. **RANURA ANCLADA:** `B.ranura = floor((B.timestamp - GENESIS_TIME) / 60)` y `B.ranura > prev_block.ranura`
3. **PRODUCTOR (solo en modo estable, H >= BLOQUES_ARRANQUE):** B.productor tiene rank válido para B.ranura según ventana de tiempo (ver sección 7.1)
4. **VDF:** `VDF_verify(preimage, B.salida_vdf, B.prueba_vdf, T_bloque) == true`
5. **CONTENIDO:** Todas las transacciones incluidas son válidas

Si cualquiera de estas condiciones falla, el bloque se rechaza.

### 5.2. Sincronización de reloj

El consenso depende de que los nodos tengan relojes razonablemente sincronizados. DOLI utiliza un mecanismo híbrido:

**FUENTES DE TIEMPO:**

1. **NTP:** Los nodos deben sincronizar con servidores NTP públicos.
2. **Mediana de peers:** Los nodos calculan el offset de tiempo basado en sus peers.

**CÁLCULO DEL TIEMPO DE RED:**

```
offset_peer[i] = timestamp_recibido[i] - reloj_local
offset_red = mediana(offset_peer[1..n])
tiempo_red = reloj_local + offset_red
```

**TOLERANCIAS:**

| Parámetro       | Valor       | Descripción                           |
|-----------------|-------------|---------------------------------------|
| DERIVA          | 120 segundos| Tolerancia máxima vs tiempo de red    |
| OFFSET_MAX_PEER | 30 segundos | Offset máximo aceptable de un peer    |
| MIN_PEERS_TIEMPO| 3           | Mínimo de peers para calcular mediana |

**REGLAS:**

1. Rechazar bloques con `|timestamp - tiempo_red| > DERIVA`.
2. Desconectar peers con `|offset_peer| > OFFSET_MAX_PEER` sostenido.
3. Si `peers_conectados < MIN_PEERS_TIEMPO`, usar solo NTP (modo degradado).

**PROTECCIÓN CONTRA ATAQUES:**

- Nodos maliciosos con timestamps extremos son excluidos del cálculo de mediana.
- La mediana es robusta: hasta 49% de peers pueden mentir sin afectar el resultado.
- Los bloques se validan contra el tiempo de red consensuado, no el reloj local puro.

---

## 6. Registro de productores

En una red abierta, cualquiera puede crear identidades sin costo monetario. Sin embargo, permitir la creación ilimitada y gratuita de identidades expondría a la red a ataques Sybil, donde un atacante inunda el sistema con nodos falsos.

Para prevenir esto, DOLI utiliza un mecanismo de "Costo de Tiempo Dinámico". Registrarse requiere resolver una VDF cuya dificultad se ajusta automáticamente según la congestión de la red.

```
registro = {
    tipo: 1,
    entradas: [...],                  // Para pagar comisión de red
    salidas: [...],                   // Cambio
    datos_adicionales: {
        llave_pública: 32 bytes,
        época: entero,
        salida_vdf: 32 bytes,
        prueba_vdf: bytes
    }
}
```

### 6.1. Dificultad dinámica de registro

El tiempo requerido para registrarse (T_registro) se ajusta por época como regulador de congestión basado en tiempo.

Sea R_E el número de transacciones tipo=1 (registro) válidas incluidas en bloques cuya ranura pertenece a la época E, en la cadena canónica.

Definimos una demanda suavizada:

```
D_E = (D_{E-1} + R_E) / 2
```

y el tiempo requerido para registros que declaran época E+1 como:

```
T_registro(E+1) = T_base × max( 1, D_E / R_objetivo )
```

### 6.2. Validación del registro

La prueba VDF se calcula sobre:

```
entrada = HASH("REG" || llave_pública || época)
salida_vdf = VDF(entrada, T_registro(época))
```

Un registro es válido si:

1. La prueba VDF verifica correctamente con T_registro(época).
2. La época es la actual o la anterior.
3. La llave pública no está ya registrada.
4. La comisión es suficiente.

### 6.3. Conjunto de productores activos

Un productor está ACTIVO en la época E si:

1. Su transacción de registro fue confirmada antes de la época E.
2. No está en período de exclusión por infracción.
3. Su bond de activación sigue bloqueado o en período de liberación.

### 6.3.1. Peso por seniority

Los productores acumulan peso de gobernanza basado en tiempo activo:

| Años activos | Peso | Justificación                    |
|--------------|------|----------------------------------|
| 0-1          | 1    | Nuevo, confianza mínima          |
| 1-2          | 2    | Probado, compromiso demostrado   |
| 2-3          | 3    | Establecido                      |
| 3-4          | 4    | Veterano, confianza máxima       |

El peso máximo es 4. Un productor que opera por 10 años tiene el mismo peso que uno de 4 años.

**CÁLCULO:**

```
años_activos = (altura_actual - altura_registro) / BLOQUES_POR_AÑO
peso = min(4, 1 + floor(años_activos))
```

Donde `BLOQUES_POR_AÑO = 3.153.600`.

**PROPIEDADES:**

1. **Resistencia a Sybil:** Un atacante que registra 100 nodos nuevos tiene peso total 100. Un grupo de 25 productores veteranos tiene peso total 100.
2. **Incentivo de permanencia:** Mayor peso requiere presencia sostenida en la red.
3. **Reinicio en salida:** Un productor que sale y re-registra comienza en peso 1. La seniority no es transferible.

Este peso se usa exclusivamente para votación de veto. No afecta la selección de ranuras ni las recompensas de bloque.

### 6.3.2. Bond Stacking (Apilamiento de Bonds)

Los productores pueden apostar múltiples bonds (1-100) para incrementar su participación en la producción de bloques:

| Parámetro | Valor | Notas |
|-----------|-------|-------|
| BOND_UNIT | 1.000 DOLI | 1 bond = 1.000 DOLI (Era 1) |
| MIN_BONDS | 1 | Mínimo para registrar |
| MAX_BONDS | 100 | Límite anti-ballena (100.000 DOLI máximo) |

**CÓMO FUNCIONA:**

DOLI usa **rotación round-robin determinística**, NO lotería probabilística:

```
Alice: 1 bond  → produce 1 bloque cada 10 ranuras
Bob:   5 bonds → produce 5 bloques cada 10 ranuras
Carol: 4 bonds → produce 4 bloques cada 10 ranuras
```

**DIFERENCIA CLAVE VS PoS:**

| Aspecto | PoS Lotería | DOLI Round-Robin |
|---------|-------------|------------------|
| Selección | Aleatoria ponderada | Rotación determinística |
| Varianza | Alta (Bob podría ganar 10 o 0) | Cero (Bob gana exactamente 5/10) |
| Justicia | Probabilística | Garantizada |
| ROI | Variable | Fijo, igual % para todos |

**ROI EQUITATIVO:**

| Productor | Inversión | Bloques/Ciclo | Recompensa/Ciclo | ROI % |
|-----------|-----------|---------------|------------------|-------|
| Alice | 1.000 DOLI | 1 | 1 DOLI | 0,1% |
| Bob | 5.000 DOLI | 5 | 5 DOLI | 0,1% |
| Carol | 4.000 DOLI | 4 | 4 DOLI | 0,1% |

Todos los productores ganan el **mismo porcentaje de retorno**. Más bonds = más retorno absoluto, mismo ROI %.

**LÍMITE ANTI-BALLENA:**

El límite de 100 bonds por identidad previene dominancia:

```
Escenario: Ballena vs 100 productores honestos (1 bond cada uno)

Estrategia A: 100 bonds en 1 identidad
  - Inversión: 100.000 DOLI
  - Participación: 100/200 = 50%
  - Sin ventaja sobre participación justa

Estrategia B: 1 bond en 100 identidades
  - Inversión: 100.000 DOLI + costo de tiempo (100 × VDF de registro)
  - Participación: 100/200 = 50%
  - Mismo resultado, mucho más esfuerzo
```

### 6.4. Bond de activación

Todo registro de productor debe bloquear un bond de activación: una cantidad de monedas inmovilizadas que se libera tras un período fijo. El bond cumple:

1. **Barrera de entrada:** filtra spam y actores casuales.
2. **Skin in the game:** el productor arriesga capital propio.
3. **Penalización económica:** las infracciones destruyen el bond.

**CANTIDAD DEL BOND:**

```
BLOQUES_POR_ERA = 12.614.400
ERA(H) = floor(H / BLOQUES_POR_ERA)

B(H) = B_inicial × 0.7^( ERA(H) )
```

| Era | Años   | Bond  | Recompensa | Equivalente en bloques |
|-----|--------|-------|------------|------------------------|
| 1   | 0-4    | 1.000 | 5,0        | 200                    |
| 2   | 4-8    | 700   | 2,5        | 280                    |
| 3   | 8-12   | 490   | 1,25       | 392                    |
| 4   | 12-16  | 343   | 0,625      | 549                    |
| 5   | 16-20  | 240   | 0,3125     | 768                    |

**PÉRDIDA DE BOND (SLASHING):**

El slashing se reserva exclusivamente para comportamiento inequívocamente malicioso:

| Comportamiento     | Consecuencia                              |
|--------------------|-------------------------------------------|
| Bloque inválido    | Rechazado por la red. Pierdes la ranura.  |
| Inactividad        | Removido del set activo. Bond intacto.    |
| Producción doble   | 100% del bond quemado.                    |

**JUSTIFICACIÓN:**

Un bloque inválido puede ocurrir por bugs de software, problemas de sincronización, o errores de configuración. La red simplemente lo rechaza. El productor pierde la oportunidad de esa ranura pero no su bond.

La producción doble requiere firmar activamente dos bloques diferentes para la misma ranura. Esto no puede ocurrir por accidente. Es fraude deliberado y merece la máxima penalización.

Este modelo sigue la filosofía de Bitcoin: la penalización natural (perder la ranura, perder recompensa) es suficiente para errores honestos. El slashing existe solo para fraude.

### 6.5. Ciclo de vida del bond

El bond tiene un ciclo de vida de 4 años (período de compromiso) con opciones de salida.

```
T_compromiso = 12.614.400 bloques (~4 años, 1 era)
T_unbonding = 259.200 bloques (~30 días)
```

**DURANTE LOS 4 AÑOS (H < H_registro + T_compromiso):**

1. **Colateral activo:** El bond permanece bloqueado como garantía.
2. **Slashing:** Las infracciones destruyen el bond completamente.
3. **Producción:** El productor participa normalmente en la selección de ranuras.
4. **Salida anticipada:** El productor puede solicitar salida antes de T_compromiso, sujeto a penalización.

**SALIDA NORMAL (después de 4 años):**

Un productor que completa su período de compromiso puede salir sin penalización:

```
transacción_salida = {
    tipo: 2,  // EXIT
    datos_adicionales: {
        llave_productor: 32 bytes
    }
}
```

1. Se inicia período de unbonding (T_unbonding = 43.200 bloques).
2. Durante unbonding: el productor se remueve del set activo, slashing sigue aplicando.
3. Al finalizar unbonding: bond se libera completamente.

**SALIDA ANTICIPADA (antes de 4 años):**

Un productor puede solicitar salir antes de completar su compromiso, pero incurre en penalización proporcional al tiempo restante:

```
penalización_pct = (tiempo_restante × 100) / T_compromiso
retorno = bond × (100 - penalización_pct) / 100
penalización = bond - retorno
```

La penalización se recicla al pool de recompensas para distribución entre productores activos. Esto mantiene el incentivo en el sistema sin destruir valor.

| Tiempo completado | Penalización | Bond devuelto |
|-------------------|--------------|---------------|
| 0 años (0%)       | 100%         | 0 DOLI        |
| 1 año (25%)       | 75%          | 250 DOLI      |
| 2 años (50%)      | 50%          | 500 DOLI      |
| 3 años (75%)      | 25%          | 750 DOLI      |
| 4 años (100%)     | 0%           | 1.000 DOLI    |

**DESTINO DE PENALIZACIONES:**

| Tipo de salida    | Destino de penalización | Justificación                    |
|-------------------|------------------------|----------------------------------|
| Salida anticipada | Pool de recompensas    | Recicla valor al sistema         |
| Slashing          | Quemado                | Destruye valor como castigo      |

**AL CUMPLIR 4 AÑOS (H >= H_registro + T_compromiso):**

El productor tiene dos opciones:

| Opción   | Acción                                          | Resultado                        |
|----------|-------------------------------------------------|----------------------------------|
| RENOVAR  | Nuevo registro VDF + bond a tasa de era actual  | Continúa produciendo             |
| SALIR    | Transacción de salida                           | Unbonding → bond completo liberado|

**PROCESO DE RENOVACIÓN:**

```
1. Productor calcula nuevo VDF de registro (T_registro de la época actual)
2. Productor deposita nuevo bond (B(H) de la era actual, más bajo)
3. Bond original se libera inmediatamente
4. Nuevo ciclo de 4 años comienza
```

**PROCESO DE SALIDA:**

```
1. Productor envía transacción de salida
2. Inicia unbonding de T_unbonding bloques (30 días)
3. Durante unbonding: removido del set, slashing aplica si había infracción previa
4. Después de unbonding: bond se libera
```

**RECLAMO DE BOND:**

Una vez completado el período de unbonding, el productor debe reclamar su bond:

```
transacción_reclamo_bond = {
    tipo: 4,  // CLAIM_BOND
    entradas: [],
    salidas: [{
        cantidad: bond_a_devolver,
        hash_llave_pública: productor
    }],
    datos_adicionales: {
        llave_productor: 32 bytes
    }
}
```

**EJEMPLO NUMÉRICO:**

| Evento                | Bloque      | Bond bloqueado | Estado           |
|-----------------------|-------------|----------------|------------------|
| Registro Era 1        | 100.000     | 1.000 DOLI     | Activo           |
| Producción normal     | 100.001-... | 1.000 DOLI     | Activo           |
| Cumple 4 años         | 2.202.400   | 1.000 DOLI     | Decisión requerida|
| Opción A: Renueva     | 2.202.401   | 700 DOLI (Era 2)| Activo, 1.000 liberado|
| Opción B: Sale        | 2.202.401   | 1.000 DOLI     | Unbonding        |
| Fin unbonding         | 2.245.601   | 0 DOLI         | Inactivo, liberado|

**EJEMPLO SALIDA ANTICIPADA (2 años):**

| Evento                | Bloque      | Cálculo              | Resultado        |
|-----------------------|-------------|----------------------|------------------|
| Registro Era 1        | 100.000     | Bond inicial         | 1.000 DOLI       |
| Solicita salida       | 1.151.200   | 50% del compromiso   | Penalización 50% |
| Inicia unbonding      | 1.151.200   | -                    | Unbonding        |
| Fin unbonding         | 1.194.400   | 1.000 × 50%          | 500 DOLI devuelto|
| Penalización          | -           | 1.000 × 50%          | 500 DOLI → pool  |

**JUSTIFICACIÓN:**

- **Unbonding 30 días:** Período suficiente para detectar comportamiento malicioso después de anunciar salida.
- **Penalización proporcional:** Incentiva completar el compromiso. Quien sale antes asume un costo.
- **Reciclaje al pool:** Las penalizaciones no se destruyen, se redistribuyen a productores activos.
- **Renovación con bond reducido:** Incentiva a productores veteranos a continuar.
- **Sin vesting:** Evita complejidad de liberación gradual.

### 6.6. Mecánica detallada de renovación

La renovación de bonds requiere atención especial para garantizar continuidad operativa y seguridad económica.

**TRANSACCIÓN DE RENOVACIÓN:**

```
transacción_renovación = {
    tipo: 3,  // RENEWAL
    entradas: [
        // Bond nuevo (B(H) de la era actual)
        { tx_hash: ..., índice: ..., firma: ... }
    ],
    salidas: [
        // Bond bloqueado para nuevo ciclo
        { cantidad: B(H), hash_llave_pública: BOND_LOCK_HASH }
    ],
    datos_adicionales: {
        llave_productor: 32 bytes,
        época: entero,
        salida_vdf: 32 bytes,
        prueba_vdf: bytes,
        bond_original_tx: 32 bytes  // Referencia al bond anterior
    }
}
```

**VENTANA DE RENOVACIÓN:**

```
T_grace = 10.080 bloques (~1 semana)

ventana_renovación_inicio = H_registro + T_lock - T_grace
ventana_renovación_fin = H_registro + T_lock + T_grace
```

| Período | Bloque | Estado |
|---------|--------|--------|
| Antes de ventana | H < inicio | Sin acción permitida |
| Ventana temprana | inicio ≤ H < T_lock | Renovación sin interrupción |
| Ventana tardía | T_lock ≤ H ≤ fin | Renovación con gap potencial |
| Después de ventana | H > fin | Salida automática forzada |

**RENOVACIÓN TEMPRANA (RECOMENDADA):**

Si el productor renueva durante la ventana temprana (antes de cumplir 4 años):

1. Nueva VDF de registro se computa con anticipación
2. Nuevo bond se deposita mientras el anterior sigue activo
3. A la altura T_lock exacta:
   - Bond original se libera automáticamente
   - Nuevo bond toma efecto
   - Sin interrupción de elegibilidad

```
Línea de tiempo (renovación temprana):

|------ 4 años ------|
                   ↓ VDF computada
                    ↓ Nuevo bond depositado
                     | ← T_lock (transición automática)
                     |------ 4 años nuevos ------|
```

**RENOVACIÓN TARDÍA:**

Si el productor renueva después de cumplir 4 años pero dentro del período de gracia:

1. Productor entra en estado GRACE
2. Sigue en el set de productores pero con penalización de prioridad
3. Slashing sigue aplicando durante GRACE
4. Al confirmar renovación: transición completa

```
Estado GRACE:
- Elegible para producción: SÍ
- Prioridad en selección: rank += 1 (desventaja)
- Slashing activo: SÍ
- Bond original: Bloqueado hasta renovación o expiración
```

**ESCENARIOS DE FALLO:**

| Escenario | Resultado | Bond |
|-----------|-----------|------|
| No renueva, no solicita salida | Salida forzada al fin de gracia | Liberado tras unbonding |
| VDF de renovación inválida | Rechazo, puede reintentar | Sin cambio |
| Fondos insuficientes para nuevo bond | Rechazo, puede reintentar | Sin cambio |
| Slashing durante gracia | Pérdida según infracción | Destruido |
| Doble renovación | Segunda rechazada | Primera válida |

**RENOVACIÓN DURANTE CAMBIO DE ERA:**

Caso especial: la ventana de renovación cruza un cambio de era.

```
Era N: B(H) = 1.000 DOLI
Era N+1: B(H) = 700 DOLI

Si ventana cruza el límite:
- Renovación antes del límite: bond = 1.000 DOLI
- Renovación después del límite: bond = 700 DOLI
```

El productor se beneficia si espera al cambio de era para renovar con bond reducido, pero arriesga quedar en estado GRACE.

**MÚLTIPLES IDENTIDADES:**

Un operador puede registrar múltiples identidades de productor. Cada una:

- Tiene su propio ciclo de 4 años
- Requiere su propio bond
- Se gestiona independientemente

```
Identidad A: |-------- 4 años --------|-------- 4 años --------|
Identidad B:     |-------- 4 años --------|-------- 4 años --------|
Identidad C:         |-------- 4 años --------|
```

**ATOMICIDAD:**

La transacción de renovación es atómica:

1. Si la VDF es válida Y el nuevo bond es suficiente:
   - Bond original se libera
   - Nuevo bond se bloquea
   - Nuevo ciclo comienza
2. Si cualquier condición falla:
   - Estado no cambia
   - Bond original permanece

**NOTIFICACIÓN AL PRODUCTOR:**

El nodo debe alertar al operador:

```
Alertas recomendadas:
- 30 días antes de T_lock: "Bond expira pronto"
- 7 días antes de T_lock: "Inicio de ventana de renovación"
- Al entrar en GRACE: "URGENTE: Renovar o perder elegibilidad"
- 24 horas antes de fin de gracia: "CRÍTICO: Última oportunidad"
```

**EJEMPLO COMPLETO:**

```
Registro inicial:
  Bloque: 100.000
  Era: 1
  Bond: 1.000 DOLI

Ventana de renovación:
  Inicio: 2.192.320 (T_lock - T_grace)
  Fin: 2.212.480 (T_lock + T_grace)

Escenario A: Renovación temprana (bloque 2.195.000)
  1. Productor computa VDF de registro
  2. Deposita 700 DOLI (bond Era 2)
  3. TX confirmada en bloque 2.195.005
  4. En bloque 2.202.400: 1.000 DOLI liberados automáticamente
  5. Nuevo ciclo: 2.202.400 → 4.304.800

Escenario B: Renovación tardía (bloque 2.205.000)
  1. En bloque 2.202.400: estado cambia a GRACE
  2. Productor computa VDF (con desventaja de prioridad)
  3. Deposita 700 DOLI
  4. TX confirmada en bloque 2.205.010
  5. Inmediatamente: 1.000 DOLI liberados
  6. Nuevo ciclo: 2.205.010 → 4.307.410

Escenario C: Expiración (no actúa)
  1. En bloque 2.202.400: estado cambia a GRACE
  2. Sin acción del productor
  3. En bloque 2.212.481: salida forzada
  4. Inicia cooldown
  5. En bloque 2.222.561: 1.000 DOLI liberados
```

### 6.7. Estados de actividad

El protocolo distingue tres estados de actividad basados en producción de bloques reciente:

| Estado            | Condición                           | Poder de gobernanza |
|-------------------|-------------------------------------|---------------------|
| Activo            | Produjo bloque en últimos 7 días    | Completo            |
| Inactivo reciente | 7-14 días sin producir              | Ninguno (gracia)    |
| Dormido           | >= 14 días sin producir             | Ninguno             |

**PRINCIPIO: EL SILENCIO NO BLOQUEA**

Los productores inactivos no cuentan para el quórum de veto. Si un productor deja de participar, pierde su poder de gobernanza pero no puede bloquear decisiones de la red por omisión.

```
peso_efectivo_veto = suma(peso[p] para cada p en productores_activos_que_votaron_veto)
peso_total_gobernanza = suma(peso[p] para cada p en productores_activos)

// Solo productores Activos cuentan. Inactivos recientes y Dormidos
// no suman al denominador ni pueden votar.
```

**CONSTANTES:**

| Parámetro               | Valor    | Descripción                    |
|-------------------------|----------|--------------------------------|
| INACTIVITY_THRESHOLD    | 10.080   | ~7 días sin producir → inactivo|
| REACTIVATION_THRESHOLD  | 1.440    | ~1 día produciendo → reactiva  |

**PENALIZACIÓN DE PESO POR INACTIVIDAD:**

Los productores con gaps de actividad sufren penalización de peso efectivo:

```
penalización_pct = min(50%, semanas_inactivo × 10%)
peso_efectivo = peso_base × (100% - penalización_pct)
```

Un productor dormido por 5+ semanas tiene solo 50% de su peso de seniority para propósitos de gobernanza.

**REACTIVACIÓN:**

Un productor inactivo puede reactivarse produciendo bloques consistentemente:

1. Produce 1.440 bloques (~1 día de actividad normal)
2. Estado cambia de Dormido/Inactivo a Activo
3. Recupera poder de gobernanza completo
4. Penalización de peso se elimina gradualmente

Este mecanismo permite recuperación de productores que tuvieron problemas técnicos temporales, pero penaliza a quienes abandonan la red sin salir formalmente.

### 6.8. Cancelación de salida

Un productor en período de unbonding puede cancelar su salida y volver a estado Activo.

**TRANSACCIÓN DE CANCELACIÓN:**

```
transacción_cancelar_salida = {
    tipo: 6,  // CANCEL_EXIT
    entradas: [],
    salidas: [],
    datos_adicionales: {
        llave_productor: 32 bytes
    }
}
```

**REGLAS:**

1. Solo válido durante período de unbonding (30 días post-solicitud de salida)
2. El productor vuelve inmediatamente a estado Activo
3. El bond permanece bloqueado
4. La seniority acumulada se preserva completamente

**JUSTIFICACIÓN:**

Los errores ocurren. Un productor puede solicitar salida por error o cambiar de opinión. Permitir cancelación durante unbonding proporciona una oportunidad de corrección sin penalización, manteniendo el incentivo de largo plazo.

**RESTRICCIONES:**

| Estado del productor | ¿Puede cancelar? |
|---------------------|------------------|
| Activo              | No (no está saliendo) |
| Unbonding           | Sí               |
| Exited              | No (ya salió)    |
| Slashed             | No (fue penalizado) |

---

## 7. Selección de productor

Para cada ranura, se determina qué productor debe crear el bloque mediante un proceso determinista de **rotación round-robin basado en bonds**.

### 7.0. Rotación Determinística por Tickets

A diferencia de sistemas PoS con lotería ponderada, DOLI usa asignación determinística:

```python
def seleccionar_productor(ranura, productores_activos):
    """
    Rotación determinística por tickets de bond.

    Ejemplo con Alice:1, Bob:5, Carol:4 bonds (total 10):
      Tickets: [Alice, Bob, Bob, Bob, Bob, Bob, Carol, Carol, Carol, Carol]
      Ranura 0 → Alice, Ranuras 1-5 → Bob, Ranuras 6-9 → Carol

    Bob SIEMPRE produce exactamente 5 de cada 10 bloques.
    Sin varianza, sin suerte.
    """
    # Ordenar por llave pública para orden determinístico
    productores_ordenados = sorted(productores_activos, key=lambda p: p.llave_publica)

    # Calcular total de tickets (suma de bond_count)
    total_tickets = sum(p.bond_count for p in productores_ordenados)

    # Selección determinística: ranura mod total_tickets
    indice_ticket = ranura % total_tickets

    # Encontrar dueño del ticket
    acumulado = 0
    for productor in productores_ordenados:
        acumulado += productor.bond_count
        if indice_ticket < acumulado:
            return productor.llave_publica
```

**PROPIEDADES:**

| Propiedad | Descripción |
|-----------|-------------|
| DETERMINISTA | Todos los nodos calculan exactamente el mismo resultado |
| SIN VARIANZA | Cada productor recibe exactamente su proporción de ranuras |
| EQUITATIVO | Mismo ROI % para todos, independiente de cantidad de bonds |
| VERIFICABLE | Cualquier nodo puede comprobar quién debía producir |

**EJEMPLO:**

```
Total: 10 bonds distribuidos entre 3 productores

Productor  Bonds  Tickets       Ranuras asignadas
─────────────────────────────────────────────────
Alice        1    [0]           0, 10, 20, 30...
Bob          5    [1,2,3,4,5]   1-5, 11-15, 21-25...
Carol        4    [6,7,8,9]     6-9, 16-19, 26-29...

En 100 ranuras:
- Alice produce EXACTAMENTE 10 bloques
- Bob produce EXACTAMENTE 50 bloques
- Carol produce EXACTAMENTE 40 bloques
```

### 7.1. Fallback de productor

Para evitar ranuras vacías en cascada cuando el productor primario está offline, se implementa un mecanismo de fallback:

```
productor_primario   = menor puntuación (rank 0)
productor_fallback_1 = segunda menor puntuación (rank 1)
productor_fallback_2 = tercera menor puntuación (rank 2)
```

**VENTANAS DE PRODUCCIÓN:**

| Tiempo en ranura | Productor elegible |
|------------------|-------------------|
| 0s - 30s         | Solo rank 0       |
| 30s - 45s        | rank 0 o rank 1   |
| 45s - 60s        | rank 0, 1 o 2     |

**REGLAS:**

1. Un bloque de rank N solo es válido si `timestamp >= slot_start + (N × 15)`.
2. Si llegan múltiples bloques válidos para la misma ranura, se prefiere el de menor rank.
3. El productor fallback recibe la recompensa completa si produce el bloque.
4. El productor primario NO es penalizado por inactividad ocasional (ver regla de inactividad).

Este mecanismo garantiza que la red no se detenga aunque varios productores estén offline simultáneamente.

**REGLA DE INACTIVIDAD:**

Si `fallos_en_ventana >= FALLOS_MAX` (por defecto: 50), el productor pasa a estado INACTIVO:

1. Se remueve del conjunto de productores activos.
2. Su bond permanece bloqueado (no se pierde por inactividad).
3. Para reactivarse: debe completar un nuevo registro con VDF.

---

## 8. Regla de selección de cadena

Cuando existen múltiples cadenas válidas, los nodos deben acordar cuál seguir.

### 8.1. Fork Choice por peso acumulado

DOLI implementa una regla de selección basada en **peso acumulado de productores**. La cadena con mayor peso acumulado es la cadena canónica.

**PESO ACUMULADO:**

```
peso_acumulado(bloque) = peso_acumulado(padre) + peso_efectivo(productor)
```

Donde `peso_efectivo` del productor se deriva de su seniority y actividad (ver sección 6.3.1).

**REGLA DE FORK CHOICE:**

```
si peso_acumulado(cadena_nueva) > peso_acumulado(cadena_actual):
    reorganizar a cadena_nueva
sino:
    mantener cadena_actual
```

**PROPIEDADES:**

| Propiedad | Descripción |
|-----------|-------------|
| Determinista | Todos los nodos calculan el mismo resultado |
| Resistente a Sybil | Muchos productores nuevos (peso=1) no superan a pocos veteranos (peso=4) |
| Sin manipulación | El peso se determina por historia on-chain, no por contenido del bloque |

**EJEMPLO:**

```
Cadena A: genesis → bloque1(peso=4) → bloque2(peso=3) → bloque3(peso=2)
          Peso acumulado: 4 + 3 + 2 = 9

Cadena B: genesis → bloque1(peso=4) → fork1(peso=1) → fork2(peso=1) → fork3(peso=1)
          Peso acumulado: 4 + 1 + 1 + 1 = 7

Cadena A gana (9 > 7), aunque Cadena B tiene más bloques.
```

### 8.2. Manejo de reorganizaciones

Cuando se detecta una cadena más pesada, el nodo ejecuta una reorganización:

1. **Detectar fork:** Recibir bloque que no extiende la punta actual
2. **Calcular pesos:** Comparar peso acumulado de ambas cadenas
3. **Encontrar ancestro común:** Retroceder hasta el punto de bifurcación
4. **Rollback:** Revertir bloques de la cadena actual hasta el ancestro
5. **Aplicar:** Aplicar bloques de la nueva cadena en orden

**LÍMITES:**

| Parámetro | Valor | Descripción |
|-----------|-------|-------------|
| MAX_REORG_DEPTH | 100 bloques | Profundidad máxima de reorganización |
| TRACKED_BLOCKS | 10,000 | Bloques recientes en memoria para fork choice |

**RESULTADO DE REORGANIZACIÓN:**

```
ReorgResult {
    rollback: [bloques a revertir],
    common_ancestor: hash del ancestro común,
    new_blocks: [bloques a aplicar],
    weight_delta: diferencia de peso (positivo = nueva cadena más pesada)
}
```

Esta regla previene ataques triviales donde un atacante crea muchos bloques con productores de bajo peso.

---

## 9. Incentivo

Por convención, la primera transacción en un bloque es una transacción especial que crea monedas nuevas pertenecientes al productor del bloque. Esto añade un incentivo para que los nodos apoyen la red y proporciona una forma de distribuir monedas inicialmente.

### 9.1. Emisión

- **Recompensa inicial:** 1 moneda por bloque
- **Reducción a la mitad:** cada 12.614.400 bloques (~4 años)
- **Suministro total:** ~25.228.800 monedas

| Era | Años   | Recompensa | Acumulado    | % del total |
|-----|--------|------------|--------------|-------------|
| 1   | 0-4    | 1,0000     | 12.614.400   | 50,00%      |
| 2   | 4-8    | 0,5000     | 18.921.600   | 75,00%      |
| 3   | 8-12   | 0,2500     | 22.075.200   | 87,50%      |
| 4   | 12-16  | 0,1250     | 23.652.000   | 93,75%      |
| 5   | 16-20  | 0,0625     | 24.440.400   | 96,88%      |
| 6   | 20-24  | 0,0313     | 24.834.600   | 98,44%      |

### 9.2. Madurez de recompensa

Las salidas de la transacción de recompensa requieren 100 confirmaciones antes de poder gastarse.

### 9.3. Pool de recompensas

El protocolo mantiene un pool de recompensas que acumula penalizaciones por salida anticipada de productores.

**FUENTES DEL POOL:**

| Fuente | Cantidad |
|--------|----------|
| Penalización por salida anticipada | Proporcional al tiempo restante |

**DISTRIBUCIÓN:**

El pool se distribuye proporcionalmente entre productores activos como parte de las recompensas de bloque.

```
recompensa_total = recompensa_base + (pool_recompensas / productores_activos)
```

Cuando un productor sale anticipadamente, su penalización se añade al pool. Cuando un productor produce un bloque, recibe su porción del pool.

**DISTINCIÓN IMPORTANTE:**

| Tipo de penalización | Destino | Efecto económico |
|---------------------|---------|------------------|
| Salida anticipada | Pool de recompensas | Recicla valor |
| Slashing (infracciones) | Quemado | Reduce suministro |

Esta distinción es deliberada:
- **Salida anticipada:** No es maliciosa, solo incumplimiento de compromiso. El valor se recicla.
- **Slashing:** Comportamiento malicioso. El valor se destruye como castigo y desincentivo.

**RECLAMO DE RECOMPENSAS:**

Los productores reclaman recompensas mediante transacciones tipo 3:

```
transacción_reclamo = {
    tipo: 3,  // CLAIM_REWARD
    entradas: [],
    salidas: [{
        cantidad: recompensa_acumulada,
        hash_llave_pública: productor
    }],
    datos_adicionales: {
        llave_productor: 32 bytes
    }
}
```

---

## 10. Escalabilidad

El tamaño máximo de bloque aumenta automáticamente cada era, permitiendo que la red procese más transacciones a medida que la tecnología mejora:

| Era | Años   | Tamaño máximo | TPS estimado* |
|-----|--------|---------------|---------------|
| 1   | 0-4    | 1 MB          | ~66           |
| 2   | 4-8    | 2 MB          | ~132          |
| 3   | 8-12   | 4 MB          | ~264          |
| 4   | 12-16  | 8 MB          | ~528          |
| 5   | 16-20  | 16 MB         | ~1,056        |
| 6+  | 20+    | 32 MB (cap)   | ~2,112        |

*Asumiendo transacciones promedio de 250 bytes.

**FÓRMULA:**

```
max_block_size(H) = min(32 MB, 1 MB × 2^era(H))
```

Donde `era(H) = floor(H / 12.614.400)`.

**JUSTIFICACIÓN:**

Este crecimiento está codificado en el protocolo desde génesis. No requiere hard forks ni votación.

El crecimiento de ~19% anual en requisitos de nodo es inferior al crecimiento histórico de capacidad de hardware:
- Almacenamiento: ~40% anual
- Ancho de banda: ~50% anual
- Capacidad de procesamiento: ~20% anual

**IMPACTO EN NODOS:**

| Era | Tamaño/día | Tamaño/año | Requisito de ancho de banda |
|-----|------------|------------|----------------------------|
| 1   | ~1.4 GB    | ~500 GB    | ~170 Kbps                  |
| 2   | ~2.8 GB    | ~1 TB      | ~340 Kbps                  |
| 3   | ~5.6 GB    | ~2 TB      | ~680 Kbps                  |
| 4   | ~11.2 GB   | ~4 TB      | ~1.4 Mbps                  |
| 5   | ~22.4 GB   | ~8 TB      | ~2.8 Mbps                  |
| 6+  | ~44.8 GB   | ~16 TB     | ~5.6 Mbps                  |

Estos requisitos están bien dentro de las capacidades de hardware de consumo proyectadas para cada período.

---

## 11. Infracciones

### 11.1. Bloques inválidos

Tipos de bloques que la red rechaza:

- **VDF_INVALIDA:** la prueba VDF no verifica
- **PRODUCTOR_INCORRECTO:** el productor no es el seleccionado para esa ranura
- **TIMESTAMP_INVALIDO:** timestamp fuera de ventana permitida
- **RANURA_INCONSISTENTE:** ranura no deriva correctamente del timestamp

**CONSECUENCIA:** El bloque es rechazado. El productor pierde la ranura y su recompensa. El bond permanece intacto.

Esta es la penalización natural, siguiendo el modelo de Bitcoin: intentaste algo inválido, la red lo ignora, perdiste tu tiempo.

### 11.2. Inactividad

Si un productor falla repetidamente en producir bloques cuando es seleccionado:

```
si fallos_consecutivos >= MAX_FALLOS (50):
    estado = INACTIVO
    // Removido del set de productores
    // Bond permanece bloqueado
    // Puede reactivarse con nuevo VDF de registro
```

**CONSECUENCIA:** Remoción del set activo. Bond intacto. Puede re-registrarse.

### 11.3. Producción doble (única infracción con slashing)

Si un productor crea dos bloques diferentes para la misma ranura, cualquier nodo puede construir una prueba de esta infracción.

#### 11.3.1. Detección automática de equivocación

DOLI implementa detección automática de producción doble en cada nodo. El sistema rastrea bloques recientes por par `(productor, ranura)` y detecta cuando un productor firma dos bloques diferentes para la misma ranura.

**ESTRUCTURA DEL DETECTOR:**

```
EquivocationDetector {
    seen_blocks: Map<(productor, ranura), hash_bloque>,
    max_tracked: 1.000 ranuras,
    pending_proofs: [EquivocationProof]
}
```

**ALGORITMO DE DETECCIÓN:**

```
función check_block(bloque):
    key = (bloque.productor, bloque.ranura)
    hash = hash(bloque)

    si seen_blocks contiene key:
        hash_existente = seen_blocks[key]
        si hash != hash_existente:
            // EQUIVOCACIÓN DETECTADA
            prueba = EquivocationProof {
                productor: bloque.productor,
                hash_bloque_1: hash_existente,
                hash_bloque_2: hash,
                ranura: bloque.ranura
            }
            emitir prueba
    sino:
        seen_blocks[key] = hash
```

**GENERACIÓN AUTOMÁTICA DE TRANSACCIÓN:**

Cuando se detecta equivocación, el nodo puede generar automáticamente una transacción de penalización:

```
proof.to_slash_transaction(reporter_keypair) → Transaction
```

El reportero firma la evidencia para probar que fue testigo de la equivocación.

#### 11.3.2. Transacción de penalización

**TRANSACCIÓN DE PENALIZACIÓN:**

```
transacción_penalización = {
    tipo: 5,  // SLASH_PRODUCER
    entradas: [],
    salidas: [],  // Bond se quema, no se redistribuye
    datos_adicionales: {
        llave_productor: 32 bytes,
        evidencia: {
            hash_bloque_1: 32 bytes,
            hash_bloque_2: 32 bytes,
            ranura: entero
        },
        firma_reportero: 64 bytes
    }
}
```

**VERIFICACIÓN:**

1. Ambos bloques tienen la misma ranura.
2. Ambos bloques están firmados por el mismo productor.
3. Los hashes de los bloques son diferentes.

**PENALIDAD:**

1. **PÉRDIDA DE BOND:** 100% del bond se quema permanentemente.
2. **EXCLUSIÓN:** Inmediata del set de productores.
3. **Para reactivarse:** nuevo registro con nuevo bond y T_registro × 2.

**JUSTIFICACIÓN:**

La producción doble requiere firmar activamente dos bloques diferentes. Esto no puede ocurrir por accidente, bug, o mala configuración. Es la única infracción que merece slashing porque es la única que es inequívocamente intencional.

### 11.4. Reincidencia

Si un productor reincide en producción doble después de reactivarse:

```
T_req = min( T_cap, T_registro(E) × 2^(infracciones_previas) )
```

El tiempo requerido para re-registrarse se duplica con cada infracción.

---

## 12. Recuperación de espacio

Una vez que la última transacción de una moneda está enterrada bajo suficientes bloques, las transacciones gastadas anteriores pueden descartarse para ahorrar espacio. Las transacciones se organizan en un árbol de Merkle, con solo la raíz incluida en el hash del bloque.

Un encabezado de bloque sin transacciones ocupa aproximadamente 340 bytes. Asumiendo que se genera un bloque cada minuto, eso equivale a ~178 MB por año.

---

## 13. Verificación simplificada

Es posible verificar pagos sin ejecutar un nodo completo. Un usuario solo necesita mantener una copia de los encabezados de bloque de la cadena más larga y obtener la rama de Merkle que enlaza la transacción al bloque en que está marcada con tiempo.

---

## 14. Privacidad

El público puede ver que alguien está enviando una cantidad a alguien más, pero sin información que vincule la transacción a nadie. Esto es similar al nivel de información liberado por las bolsas de valores.

Como cortafuego adicional, se debería usar un nuevo par de llaves para cada transacción para evitar que se vinculen a un propietario común.

---

## 15. Seguridad

Si el atacante controla menos capacidad de cómputo secuencial que la red honesta, la probabilidad de que alcance a la cadena honesta disminuye rápidamente con el número de ranuras de diferencia.

En este sistema, un atacante no puede "acelerar" la producción de una cadena alternativa añadiendo hardware en paralelo, porque cada bloque requiere un cómputo secuencial de duración fija.

### 15.1. Costo de ataque

Para dominar la red, un atacante necesitaría:

1. Registrar más productores que los honestos.
2. Mantener esos productores activos durante las ranuras asignadas.
3. Arriesgar exclusión si es detectado haciendo producción doble.

El protocolo regula automáticamente la tasa a la que pueden incorporarse nuevas identidades.

---

## 16. Distribución

DOLI no tiene premine, ICO, tesorería, ni asignaciones especiales. Toda moneda en circulación proviene de recompensas de bloque.

**POLÍTICA DE DISTRIBUCIÓN:**

| Método | DOLI |
|--------|------|
| Premine | ❌ No |
| ICO/Venta | ❌ No |
| Tesorería | ❌ No |
| Fundación | ❌ No |
| Recompensas de desarrollo | ❌ No |
| Minería justa | ✅ Sí |

**JUSTIFICACIÓN:**

1. **Sin privilegios:** Ningún participante tiene ventaja estructural sobre otros.
2. **Sin conflictos:** Los desarrolladores no tienen incentivo para modificar reglas en su favor.
3. **Sin dependencia:** El protocolo no requiere financiamiento continuo.
4. **Verificable:** Cualquiera puede auditar que no existen monedas pre-minadas.

**BLOQUE GÉNESIS:**

El bloque génesis contiene exactamente:
- Una transacción coinbase con 1 DOLI (recompensa estándar)
- Cero transacciones adicionales

No hay "monedas fundacionales" ocultas ni asignaciones especiales.

---

## 17. Gobernanza

DOLI no tiene gobernanza on-chain ni mecanismos de votación para cambios de protocolo. Las reglas del protocolo se congelan tras el lanzamiento de mainnet.

**POLÍTICA DE CAMBIOS:**

| Tipo de cambio | Permitido |
|----------------|-----------|
| Corrección de bugs críticos | ✅ Sí (soft fork) |
| Optimizaciones de rendimiento | ✅ Sí (sin cambiar reglas) |
| Nuevas características | ❌ No |
| Cambios de parámetros | ❌ No |
| Hard forks | ❌ No |

### 17.1. Sistema de actualizaciones de software

El cliente oficial incluye un sistema de auto-actualización con poder de veto para los productores.

**REGLAS (sin excepciones):**

```
T_veto = 7 días
UMBRAL_VETO = 40% (peso ponderado por seniority)
FIRMAS_REQUERIDAS = 3 de 5 mantenedores
```

**FLUJO DE ACTUALIZACIÓN:**

1. **Publicación:** Nueva versión firmada por al menos 3 de 5 mantenedores.
2. **Período de veto:** 7 días durante los cuales los productores pueden votar.
3. **Evaluación:** Si >= 40% del peso efectivo vota VETO: RECHAZADA.
4. **Aplicación:** Si < 40% del peso vota VETO: APROBADA y aplicada automáticamente.

**VERIFICACIÓN DE RELEASE:**

```
mensaje_firmado = "versión:hash_sha256_binario"
```

Cada release contiene:
- Versión semántica
- Hash SHA-256 del binario
- Changelog
- Firmas de mantenedores

**VOTACIÓN:**

```
voto = {
    versión: string,
    voto: VETO | APROBAR,
    id_productor: string,
    firma: bytes
}
```

Solo productores activos pueden votar. El poder de voto se pondera por seniority (1-4 según años activos). Los votos se propagan por gossip.

**CÁLCULO DEL RESULTADO:**

El veto se calcula por peso, no por conteo. Esto protege contra ataques Sybil donde un atacante registra muchos nodos nuevos.

```
peso_veto = suma(peso[p] para cada p en productores_que_votaron_veto)
peso_total = suma(peso[p] para cada p en productores_activos)
porcentaje_veto = (peso_veto × 100) / peso_total

si porcentaje_veto >= 40%:
    actualización RECHAZADA
sino:
    actualización APROBADA
```

Un productor veterano (4 años, peso 4) tiene más poder de veto que cuatro productores nuevos (peso 1 cada uno). Esto dificulta que un atacante bloquee actualizaciones legítimas mediante registro masivo de nodos.

**JUSTIFICACIÓN:**

1. **Transparencia:** Las actualizaciones son públicas y auditables.
2. **Veto ponderado:** 40% del peso efectivo protege contra Sybil y permite minoría significativa detener cambios controversiales.
3. **Multi-firma:** 3 de 5 mantenedores previene compromiso de una sola llave.
4. **7 días:** Tiempo suficiente para revisión de código.
5. **Sin urgencia:** No hay mecanismo de actualización de emergencia que bypass el veto.

**CONFIGURACIÓN DEL NODO:**

| Opción | Efecto |
|--------|--------|
| `--auto-update=true` (default) | Actualizaciones automáticas con veto |
| `--auto-update=false` | Solo notificaciones, sin aplicar |
| `--auto-update-url=URL` | Mirror personalizado |

**PROCESO PARA BUGS CRÍTICOS:**

1. Publicación de la vulnerabilidad (después de mitigación)
2. Release firmado por 3/5 mantenedores
3. Período de veto de 7 días (sin excepciones)
4. Si pasa el veto: aplicación automática

**JUSTIFICACIÓN:**

1. **Predecibilidad:** Las reglas no cambian arbitrariamente.
2. **Resistencia a captura:** No hay mecanismo para grupos de interés tomar control.
3. **Simplicidad:** Menos superficie de ataque en la gobernanza.
4. **Contrato social:** Los usuarios saben exactamente qué están usando.

**CONTROVERSIAS PREVISTAS:**

El protocolo está diseñado para evitar las "guerras de criptomonedas" comunes:

| Controversia | Resolución DOLI |
|--------------|-----------------|
| Tamaño de bloque | Escalado automático por era (1MB → 32MB) |
| Dificultad VDF | Escalado automático por era |
| Emisión monetaria | Fija, halving cada 4 años |
| Consenso | VDF, sin cambios posibles |
| Bond mínimo | Decae 30% por era, el mercado decide |

Estos parámetros están codificados en el protocolo y no requieren decisiones futuras.

---

## 18. Inmutabilidad

DOLI es inmutable. No existen mecanismos para revertir transacciones, recuperar fondos, o modificar el historial.

**POLÍTICA DE NO-REVERSIÓN:**

| Situación | Respuesta del protocolo |
|-----------|-------------------------|
| Pérdida de llaves privadas | Fondos perdidos permanentemente |
| Transacción errónea | No reversible |
| Hackeo de exchange | No reversible |
| Orden judicial | No cumplible |
| "Robo" de fondos | No reversible |

**JUSTIFICACIÓN:**

1. **Certeza:** Las transacciones confirmadas son finales.
2. **Neutralidad:** El protocolo no distingue entre transacciones "buenas" y "malas".
3. **Descentralización:** Nadie tiene poder para modificar el historial.
4. **Precedente:** Cualquier excepción crea expectativa de excepciones futuras.

**RESPONSABILIDAD DEL USUARIO:**

- Respaldar llaves privadas de forma segura
- Verificar direcciones antes de enviar
- Usar montos pequeños para transacciones de prueba
- Entender que las transacciones son irreversibles

**NOTA HISTÓRICA:**

Este principio se basa en la experiencia de otras criptomonedas:

- **TheDAO (2016):** Ethereum revirtió un hackeo, creando Ethereum Classic
- **BitFinex (2016):** Bitcoin no revirtió, los fondos se perdieron

DOLI sigue el precedente de Bitcoin: el código es ley, las transacciones son finales.

---

## 19. Conclusión

Se ha propuesto un sistema para transacciones electrónicas sin depender de confianza. La red utiliza pruebas de tiempo para registrar un historial público de transacciones. El sistema es seguro mientras los nodos honestos controlen colectivamente más capacidad de cómputo secuencial que cualquier grupo de atacantes cooperantes.

La red es robusta en su simplicidad no estructurada. Los nodos trabajan todos a la vez con poca coordinación. Votan con su capacidad de cómputo secuencial, expresando su aceptación de bloques válidos al trabajar en extenderlos y rechazando bloques inválidos al negarse a trabajar en ellos.

**El tiempo es el recurso más democrático. DOLI lo convierte en dinero.**

---

## Referencias

1. D. Boneh, J. Bonneau, B. Bünz, B. Fisch, "Verifiable Delay Functions," Advances in Cryptology – CRYPTO 2018.
2. B. Wesolowski, "Efficient Verifiable Delay Functions," Advances in Cryptology – EUROCRYPT 2019.
3. D. Bernstein, N. Duif, T. Lange, P. Schwabe, B. Yang, "High-speed high-security signatures," Journal of Cryptographic Engineering, 2012.
4. R. Merkle, "Protocols for public key cryptosystems," IEEE Symposium on Security and Privacy, 1980.

---

## Parámetros del protocolo

| Parámetro              | Valor                    |
|------------------------|--------------------------|
| **SUMINISTRO**         |                          |
| Total                  | ~25.228.800 monedas      |
| Decimales              | 8                        |
| Unidad mínima          | 0,00000001               |
| **TIEMPO**             |                          |
| GENESIS_TIME           | 2026-02-01T00:00:00Z     |
| Duración de ranura     | 10 segundos              |
| Ranuras por época      | 360                      |
| Bloques por reducción  | 12.614.400               |
| BLOQUES_ARRANQUE       | 60.480                   |
| **VDF (Hash-Chain)**   |                          |
| Iteraciones (default)  | 10.000.000               |
| Iteraciones (min)      | 100.000                  |
| Iteraciones (max)      | 100.000.000              |
| Tiempo objetivo        | 700 ms                   |
| Tolerancia calibración | 10%                      |
| Ajuste máximo/ciclo    | 20%                      |
| T_registro (base)      | 600.000.000 iteraciones  |
| **BOND**               |                          |
| B_inicial              | 1.000 monedas            |
| Factor de reducción    | 0.7 por era (30%)        |
| T_compromiso           | 2.102.400 bloques (~4 años) |
| T_unbonding            | 259.200 bloques (~30 días) |
| Penalización salida anticipada | Proporcional al tiempo restante |
| Destino penalización   | Pool de recompensas      |
| Destino slashing       | Quemado                  |
| **ACTIVIDAD**          |                          |
| INACTIVITY_THRESHOLD   | 60.480 bloques (~7 días) |
| REACTIVATION_THRESHOLD | 8.640 bloques (~1 día)   |
| MAX_GAP_PENALTY        | 50%                      |
| GAP_PENALTY_RATE       | 10% por semana inactivo  |
| **SENIORITY**          |                          |
| BLOQUES_POR_AÑO        | 3.153.600                |
| MIN_WEIGHT             | 1                        |
| MAX_WEIGHT             | 4                        |
| **ACTUALIZACIONES**    |                          |
| T_veto                 | 7 días                   |
| UMBRAL_VETO            | 40% (peso ponderado)     |
| FIRMAS_REQUERIDAS      | 3 de 5 mantenedores      |
| **RED**                |                          |
| DERIVA                 | 120 segundos             |
| OFFSET_MAX_PEER        | 30 segundos              |
| MIN_PEERS_TIEMPO       | 3                        |
| **FALLBACK**           |                          |
| Ventana rank 0         | 0s - 5s                  |
| Ventana rank 1         | 5s - 7.5s                |
| Ventana rank 2         | 7.5s - 10s               |
| **BLOQUES**            |                          |
| Tamaño base            | 1.000.000 bytes          |
| Tamaño máximo (cap)    | 32.000.000 bytes         |
| Factor de crecimiento  | ×2 por era               |
| **CRIPTOGRAFÍA**       |                          |
| Hash                   | BLAKE3-256               |
| Firmas                 | Ed25519                  |
| VDF                    | Hash-chain (SHA-256 iterado) |

---

## Apéndice A: Constantes del protocolo

### A.1. Hashes especiales

**BURN_HASH:**

Hash destino para quemar monedas (slashing, destrucción de bond):

```
BURN_HASH = 0xBBBBBBBB...BBBBBBBB (32 bytes de 0xBB)
```

Las salidas enviadas a BURN_HASH son no gastables y se consideran destruidas permanentemente.

**GENESIS_HASH:**

Hash del bloque génesis, calculado como:

```
GENESIS_HASH = HASH("DOLI Genesis")
```

---

## Apéndice B: Protocolo P2P

### B.1. Capa de transporte

El protocolo de red utiliza libp2p con la siguiente configuración:

| Componente     | Protocolo           | Descripción                       |
|----------------|---------------------|-----------------------------------|
| Transporte     | TCP                 | Conexiones TCP estándar           |
| Cifrado        | Noise               | Cifrado autenticado punto a punto |
| Multiplexado   | Yamux               | Múltiples flujos por conexión     |

### B.2. Descubrimiento de pares

**Protocolo:** Kademlia DHT

**Identificador:** `/doli/kad/1.0.0`

Los nodos participan en una tabla hash distribuida para descubrir nuevos pares. Cada nodo tiene un PeerId derivado de su identidad criptográfica.

**Nodos semilla (bootstrap):**

```
/dns4/seed1.doli.network/tcp/9000/p2p/<peer_id>
/dns4/seed2.doli.network/tcp/9000/p2p/<peer_id>
```

### B.3. Propagación de mensajes

**Protocolo:** GossipSub v1.1

**Temas (Topics):**

| Tema                | Contenido                    |
|---------------------|------------------------------|
| `/doli/blocks/1`    | Bloques nuevos               |
| `/doli/txs/1`       | Transacciones no confirmadas |

**Parámetros GossipSub:**

| Parámetro              | Valor |
|------------------------|-------|
| mesh_n                 | 6     |
| mesh_n_low             | 4     |
| mesh_n_high            | 12    |
| gossip_lazy            | 6     |
| heartbeat_interval     | 1s    |
| history_length         | 5     |
| history_gossip         | 3     |

### B.4. Sincronización

**Protocolo:** Request-Response

**Identificador:** `/doli/sync/1.0.0`

**Mensajes de solicitud:**

| Tipo               | Parámetros                        | Descripción                    |
|--------------------|-----------------------------------|--------------------------------|
| GetHeaders         | start_hash, max_count             | Solicitar encabezados de bloque|
| GetBodies          | hashes[]                          | Solicitar cuerpos de bloque    |
| GetBlockByHeight   | height                            | Solicitar bloque por altura    |
| GetBlockByHash     | hash                              | Solicitar bloque por hash      |

**Mensajes de respuesta:**

| Tipo     | Contenido                  |
|----------|----------------------------|
| Headers  | BlockHeader[]              |
| Bodies   | Block[]                    |
| Block    | Option<Block>              |

### B.5. Protocolo de estado

**Identificador:** `/doli/status/1.0.0`

Al conectarse, los pares intercambian su estado actual:

```
StatusRequest {
    version: u32,
    network_id: u32,
    genesis_hash: Hash,
    best_height: u64,
    best_hash: Hash,
    best_slot: u64,
}
```

Los nodos rechazan conexiones con:
- `network_id` diferente
- `genesis_hash` diferente
- `version` incompatible

### B.6. Estados de sincronización

```
enum SyncState {
    Idle,                                    // Sin sincronización activa
    DownloadingHeaders { target_slot: u64 }, // Descargando encabezados
    DownloadingBodies { pending: usize },    // Descargando cuerpos
    Processing { height: u64 },              // Procesando bloques
    Synchronized,                            // Sincronizado con la red
}
```

---

## Apéndice C: API JSON-RPC

### C.1. Protocolo

El servidor RPC utiliza JSON-RPC 2.0 sobre HTTP POST.

**Endpoint:** `http://localhost:8545/`

**Formato de solicitud:**

```json
{
    "jsonrpc": "2.0",
    "method": "<nombre_método>",
    "params": { ... },
    "id": 1
}
```

**Formato de respuesta exitosa:**

```json
{
    "jsonrpc": "2.0",
    "result": { ... },
    "id": 1
}
```

**Formato de error:**

```json
{
    "jsonrpc": "2.0",
    "error": {
        "code": -32600,
        "message": "Invalid Request"
    },
    "id": 1
}
```

### C.2. Métodos disponibles

#### getBlockByHash

Obtiene un bloque por su hash.

**Parámetros:**
```json
{ "hash": "abc123..." }
```

**Respuesta:**
```json
{
    "hash": "abc123...",
    "height": 12345,
    "slot": 67890,
    "prev_hash": "def456...",
    "merkle_root": "789abc...",
    "timestamp": 1700000000,
    "producer": "pubkey_hex",
    "tx_count": 5,
    "transactions": ["tx_hash_1", "tx_hash_2", ...]
}
```

#### getBlockByHeight

Obtiene un bloque por su altura.

**Parámetros:**
```json
{ "height": 12345 }
```

**Respuesta:** Igual que `getBlockByHash`.

#### getTransaction

Obtiene una transacción por su hash.

**Parámetros:**
```json
{ "hash": "tx_hash..." }
```

**Respuesta:**
```json
{
    "hash": "tx_hash...",
    "version": 1,
    "tx_type": 0,
    "inputs": [
        {
            "tx_hash": "prev_tx...",
            "output_index": 0
        }
    ],
    "outputs": [
        {
            "amount": 100000000,
            "pubkey_hash": "recipient..."
        }
    ],
    "fee": 1000
}
```

#### sendTransaction

Envía una transacción firmada a la red.

**Parámetros:**
```json
{ "tx": "hex_encoded_transaction" }
```

**Respuesta:**
```json
"tx_hash..."
```

**Errores posibles:**
- `-32001`: Transacción ya conocida
- `-32002`: Transacción inválida
- `-32003`: Mempool lleno

#### getBalance

Obtiene el balance de una dirección.

**Parámetros:**
```json
{ "address": "pubkey_hash_hex" }
```

**Respuesta:**
```json
{
    "confirmed": 100000000,
    "unconfirmed": 5000000,
    "total": 105000000
}
```

#### getUtxos

Obtiene las salidas no gastadas de una dirección.

**Parámetros:**
```json
{
    "address": "pubkey_hash_hex",
    "spendable_only": true
}
```

**Respuesta:**
```json
[
    {
        "tx_hash": "abc...",
        "output_index": 0,
        "amount": 50000000,
        "output_type": "normal",
        "lock_until": 0,
        "height": 1000,
        "spendable": true
    }
]
```

#### getMempoolInfo

Obtiene estadísticas del mempool.

**Parámetros:** Ninguno.

**Respuesta:**
```json
{
    "tx_count": 150,
    "total_size": 45000,
    "min_fee_rate": 1,
    "max_size": 10485760,
    "max_count": 5000
}
```

#### getNetworkInfo

Obtiene información de la red.

**Parámetros:** Ninguno.

**Respuesta:**
```json
{
    "peer_id": "12D3Koo...",
    "peer_count": 8,
    "syncing": false,
    "sync_progress": null
}
```

#### getChainInfo

Obtiene información de la cadena.

**Parámetros:** Ninguno.

**Respuesta:**
```json
{
    "network": "mainnet",
    "best_hash": "abc123...",
    "best_height": 12345,
    "best_slot": 67890,
    "genesis_hash": "genesis..."
}
```

### C.3. Códigos de error

| Código  | Mensaje               | Descripción                        |
|---------|-----------------------|------------------------------------|
| -32700  | Parse error           | JSON inválido                      |
| -32600  | Invalid Request       | Solicitud JSON-RPC inválida        |
| -32601  | Method not found      | Método no existe                   |
| -32602  | Invalid params        | Parámetros inválidos               |
| -32603  | Internal error        | Error interno del servidor         |
| -32001  | Tx already known      | Transacción ya existe en mempool   |
| -32002  | Invalid transaction   | Transacción no válida              |
| -32003  | Mempool full          | Mempool está lleno                 |
| -32004  | Block not found       | Bloque no encontrado               |
| -32005  | Tx not found          | Transacción no encontrada          |

---

## Apéndice D: Políticas del Mempool

### D.1. Límites

| Parámetro           | Mainnet     | Testnet     |
|---------------------|-------------|-------------|
| max_count           | 5.000 tx    | 10.000 tx   |
| max_size            | 10 MB       | 20 MB       |
| max_tx_size         | 100 KB      | 100 KB      |
| min_fee_rate        | 1 sat/byte  | 0 sat/byte  |
| expiry_time         | 72 horas    | 24 horas    |

### D.2. Cálculo de comisión

```
fee_rate = fee / tx_size_bytes
```

Donde:
- `fee` = suma(entradas) - suma(salidas), en satoshis
- `tx_size_bytes` = tamaño serializado de la transacción

### D.3. Política de evicción

Cuando el mempool está lleno:

1. Se ordenan las transacciones por `fee_rate` ascendente.
2. Se eliminan las transacciones con menor `fee_rate` hasta hacer espacio.
3. Si una transacción tiene descendientes, se calcula el `fee_rate` agregado (CPFP).

### D.4. CPFP (Child Pays For Parent)

Una transacción hija puede "pagar" por su padre:

```
fee_rate_agregado = (fee_padre + fee_hijo) / (size_padre + size_hijo)
```

Las transacciones se evalúan como paquetes, no individualmente.

### D.5. Validación de transacciones

Una transacción se acepta en el mempool si:

1. **Formato válido:** Deserializa correctamente.
2. **Sin duplicados:** No existe ya en mempool ni confirmada.
3. **Tamaño:** `tx_size <= max_tx_size`.
4. **Comisión:** `fee_rate >= min_fee_rate`.
5. **UTXOs válidos:** Todas las entradas referencian UTXOs existentes y no gastados.
6. **Firmas válidas:** Todas las firmas verifican correctamente.
7. **Sin conflictos:** No gasta UTXOs ya gastados por otra tx en mempool.
8. **Locktime:** Si aplica, el tiempo de bloqueo ha pasado.

### D.6. Expiración

Las transacciones se eliminan automáticamente después de `expiry_time` si no han sido confirmadas.

---

**DOLI v1.2**

*Actualizado enero 2026: Fork choice por peso acumulado, detección automática de equivocación, VDF hash-chain con calibración dinámica.*

*"El tiempo es el recurso más democrático."*

www.doli.network | doli@protonmail.com
