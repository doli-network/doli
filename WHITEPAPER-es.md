# DOLI<sub>τ</sub>

## Ordenamiento Determinista mediante Iteraciones Lineales

### Un Sistema de Efectivo Electrónico Peer-to-Peer Basado en Tiempo Verificable

**E. Weil** · weil@doli.network

---

## Resumen

Proponemos un sistema de efectivo electrónico peer-to-peer donde el único recurso requerido para el consenso es el tiempo — el único recurso distribuido equitativamente entre todos los participantes.

La produccion de bloques sigue una rotacion determinista: un participante con un bond sabe exactamente cuando se producira su proximo bloque. El protocolo distribuye recompensas cada epoch a traves de un pool integrado — sin pools de mineria externos, sin operadores, sin comisiones. Las recompensas se reinvierten en stake productivo, creando un crecimiento exponencial predecible para cada participante sin importar su tamano.

Un nuevo productor que recibe 10 DOLI puede reinvertir las recompensas de bloques para duplicar su stake a intervalos regulares. La tasa de duplicacion es identica para todos los participantes — uno o tres mil bonds. La presencia continua se demuestra mediante attestations de actividad on-chain — los productores que estan en linea y siguiendo la cadena califican para su parte. Sin loteria. Sin varianza. Sin pools. Solo tiempo.

Las transacciones se ordenan mediante pruebas de retardo secuencial — computaciones de hash iteradas que no pueden paralelizarse. No se requiere hardware especial. Cualquier CPU puede participar en el consenso. El resultado es un sistema donde el peso del consenso emerge del tiempo en lugar de la confianza, el capital o la escala.

Demostramos que los NFTs, tokens fungibles y puentes entre cadenas sin confianza pueden implementarse como tipos de salida UTXO nativos con condiciones de gasto declarativas, sin una maquina virtual, sin medicion de gas y sin comites de confianza — logrando una expresividad equivalente a los enfoques basados en VM para estos casos de uso mientras se mantiene un costo de verificacion acotado y predecible.

---
## 1. Introduccion

Todo mecanismo de consenso jamas disenado comparte una suposicion: la seguridad requiere un recurso escaso que pueda acumularse. Bitcoin eligio la energia. Ethereum eligio el capital. Ambos crearon sistemas donde el participante mas grande tiene una ventaja estructural sobre el mas pequeno.

Esta suposicion es incorrecta. Existe un recurso que no puede acumularse, no puede paralelizarse y se distribuye equitativamente a cada participante en la Tierra: **el tiempo**.

Un segundo transcurre a la misma velocidad para un individuo operando un solo nodo como para un estado-nacion con presupuesto ilimitado. Ninguna cantidad de dinero puede comprar mas tiempo. Ninguna cantidad de hardware puede hacer que el tiempo pase mas rapido.

Proponemos un sistema de efectivo electronico donde el consenso se deriva de computacion secuencial verificable — prueba de que ha transcurrido tiempo real. El sistema es seguro mientras los participantes honestos mantengan colectivamente mas presencia de computacion secuencial que cualquier grupo cooperante de atacantes.

### 1.1. Por que ahora

El trilema blockchain asume tres propiedades en competencia: descentralizacion, seguridad y escalabilidad. Las soluciones propuestas intercambian una por otra — energia por seguridad (PoW), descentralizacion por escalabilidad (PoS), simplicidad por rendimiento (sharding). Estas concesiones surgen porque todo sistema previo ancla el consenso a un recurso que puede acumularse: poder de hash, stake o almacenamiento.

La formalizacion de las Verifiable Delay Functions por Boneh, Bonneau, Bunz y Fisch [2] en 2018 demostro que la computacion secuencial podria servir como primitiva de consenso — probando que ha transcurrido tiempo real sin confiar en el demostrador. Esta idea motivo nuestro enfoque, aunque DOLI usa una construccion mas simple (Seccion 5.1).

Al anclar el consenso a la computacion secuencial:

- **Descentralizacion:** No se requiere hardware especial. Cualquier CPU puede participar.
- **Seguridad:** Atacar requiere tiempo real, no recursos comprables.
- **Escalabilidad:** La produccion de bloques esta limitada por el tiempo, no por la competencia de recursos.

---

## 2. Transacciones

Una moneda es una cadena de firmas digitales. Para transferir la propiedad, el titular actual firma el hash de la transaccion junto con la clave publica del destinatario. El destinatario verifica la cadena de firmas para confirmar la procedencia.

```
┌─────────────────────────────────┐
│         Transaction             │
├─────────────────────────────────┤
│  Hash of previous TX            │
│  Recipient public key           │
│  Owner signature                │
└─────────────────────────────────┘
                │
                ▼
┌─────────────────────────────────┐
│         Transaction             │
├─────────────────────────────────┤
│  Hash of previous TX            │
│  Recipient public key           │
│  Owner signature                │
└─────────────────────────────────┘
```

El desafio fundamental es el doble gasto: sin una autoridad central, como sabe el destinatario que el remitente no ha gastado ya la misma moneda en otro lugar? Las soluciones centralizadas funcionan pero crean puntos unicos de falla y requieren confianza universal.

DOLI resuelve esto mediante el anuncio publico de todas las transacciones y el ordenamiento deterministico por tiempo. Cada transaccion se difunde a la red y se incluye en un bloque en un slot especifico. La naturaleza secuencial de los slots — cada uno anclado por una prueba de retardo secuencial — establece un ordenamiento inequivoco. La transaccion mas temprana en la secuencia temporal es la valida. Los intentos posteriores de gastar la misma salida son rechazados por cada nodo honesto.

### 2.1. Validez de transacciones

Una transaccion es valida si:

1. Cada entrada referencia una salida existente y no gastada.
2. La firma corresponde a la clave publica de la salida referenciada.
3. La suma de las entradas es mayor o igual a la suma de las salidas.
4. Todos los montos son positivos.

La diferencia entre entradas y salidas constituye la comision para el productor del bloque.

### 2.2. Estructura de salidas

Cada salida no gastada contiene cinco campos:

```
┌──────────────────────────────────────────────────────────┐
│                    Output (UTXO)                         │
├──────────────┬───────────────────────────────────────────┤
│ type         │ What kind of value (transfer, bond, ...)  │
│ amount       │ How much                                  │
│ owner        │ Who can spend (public key hash)           │
│ lock_until   │ When it becomes spendable                 │
│ extra_data   │ Extensible spending conditions            │
└──────────────┴───────────────────────────────────────────┘
```

Los primeros cuatro campos cubren todas las operaciones de efectivo. El quinto campo — `extra_data` — reserva espacio para condiciones de gasto arbitrarias sin requerir cambios en el formato de salida.

Para transferencias basicas, `extra_data` esta vacio. Para bonds, `extra_data` codifica el slot de creacion del bond (4 bytes, little-endian), permitiendo calculos de vesting FIFO por bond directamente desde el conjunto UTXO. Los nuevos tipos de salida definen como se interpreta `extra_data`, anadiendo reglas de validacion al protocolo mientras la estructura permanece fija desde el genesis.

---

## 3. Salidas programables

El campo `extra_data` en cada salida (Seccion 2.2) hace que las salidas de DOLI sean programables sin una maquina virtual, sin gas y sin un lenguaje de scripting Turing-completo.

### 3.1. Principios de diseno

Las plataformas de contratos inteligentes eligieron la generalidad: un computador universal en cada nodo, ejecutando codigo arbitrario en cada transaccion. El costo es complejidad, superficie de ataque y ejecucion impredecible.

DOLI elige lo opuesto: **condiciones declarativas**. Una salida no contiene codigo a ejecutar — contiene condiciones a verificar. La distincion importa:

| Propiedad | Ethereum (EVM) | Bitcoin Script | DOLI Conditions |
|----------|---------------|----------------|-----------------|
| Modelo | Account + VM | UTXO + Stack machine | UTXO + Native rules |
| Ejecucion | Turing-complete | Intencionalmente limitado | Verificacion declarativa |
| Gas/Comisiones | Impredecible (gas) | Fijo | Fijo (sin medicion) |
| Estado | Estado mutable compartido | Sin estado | Sin estado |
| Superficie de ataque | Ilimitada | Pequena | Minima |
| Tiempo de ejecucion | Interpretado (lento) | Interpretado | Compilado (nativo) |

Las condiciones no se interpretan en tiempo de ejecucion — se compilan en el binario del nodo como reglas de validacion nativas de Rust. Cada tipo de salida define que condiciones son validas y como se decodifica `extra_data`. Anadir un nuevo tipo de condicion es una actualizacion del protocolo, no un despliegue.

### 3.2. Lenguaje de condiciones

Las condiciones son predicados componibles. Cada condicion retorna verdadero o falso. Una salida es gastable cuando todas sus condiciones se satisfacen.

```
Condition := Signature(pubkey_hash)
           | Multisig(threshold, [pubkey_hash, ...])
           | Hashlock(hash)
           | Timelock(min_height)
           | TimelockExpiry(max_height)
           | And(Condition, Condition)
           | Or(Condition, Condition)
           | Threshold(n, [Condition, ...])
```

**Codificacion:** Las condiciones se serializan en `extra_data` como un formato binario compacto. Para transferencias basicas, `extra_data` esta vacio — la condicion por defecto es `Signature(owner)`.

**Costo de verificacion:** Cada condicion se resuelve en un numero fijo de operaciones criptograficas (verificaciones de firma, comparaciones de hash, comparaciones de altura). Sin bucles. Sin recursion. Sin computacion ilimitada. El costo de verificacion se conoce antes de la ejecucion.

### 3.3. Tipos de salida nativos

Cada tipo de salida es un patron nombrado sobre el lenguaje de condiciones:

| Tipo | Condiciones | Caso de uso |
|------|-----------|----------|
| Transfer | `Signature(owner)` | Pago estandar |
| Bond | `Signature(owner) AND Protocol(withdrawal)` | Stake del productor |
| Multisig | `Multisig(n, keys)` | Custodia compartida |
| Hashlock | `Signature(owner) AND Hashlock(h)` | Atomic swaps |
| HTLC | `(Hashlock(h) AND Timelock(t)) OR TimelockExpiry(t+d)` | Canales de pago |
| Escrow* | `Threshold(2, [buyer, seller, arbiter])` | Comercio sin confianza |
| Vesting | `Signature(owner) AND Timelock(unlock_height)` | Asignaciones con bloqueo temporal |
| UniqueAsset | `Condition + [token_id, content_hash]` | Tokens no fungibles |
| FungibleAsset | `Condition + [asset_id, supply, ticker]` | Tokens emitidos por usuarios |
| BridgeHTLC | `HTLC + [target_chain, target_address]` | Puentes entre cadenas |

*Escrow es un patron de composicion usando condiciones Multisig o Threshold, no un tipo de salida separado.

Estas no son implementaciones separadas — son composiciones de las mismas condiciones primitivas. Un desarrollador no escribe un contrato inteligente. Un desarrollador selecciona condiciones.

### 3.4. Tokens no fungibles (UniqueAsset)

Una salida UniqueAsset porta un token globalmente unico que representa la propiedad de un objeto digital singular. El campo `extra_data` almacena la condicion de gasto seguida de metadatos:

```
extra_data = [condition_bytes][version][token_id][content_hash_len][content_hash]
```

**Identidad del token.** El `token_id` es determinista: `BLAKE3("DOLI_NFT" || creator_pubkey_hash || nonce)`. Dos acunaciones con diferentes nonces siempre producen tokens diferentes. El hash de contenido puede ser un CID de IPFS, una URI HTTP o un digest BLAKE3 crudo — el protocolo almacena bytes sin interpretarlos.

**Condiciones de gasto.** El campo de condicion usa el mismo lenguaje componible que cualquier otra salida. El caso mas simple es `Signature(owner)` — solo el titular actual puede transferir el NFT. Pero nada impide una custodia Multisig, una revelacion protegida por Hashlock, o una subasta con Timelock donde el NFT se vuelve gastable por cualquiera despues de una fecha limite.

**Transferencia.** Transferir un NFT gasta el UTXO antiguo y crea una nueva salida UniqueAsset con el mismo `token_id` y `content_hash` pero un nuevo propietario y potencialmente nuevas condiciones. El token_id es la identidad permanente; el UTXO es el registro de propiedad actual.

**Sin registro, sin contrato, sin estado global.** El NFT existe enteramente dentro del UTXO que lo porta. La indexacion es responsabilidad del lector — el protocolo valida estructura y condiciones, nada mas.

### 3.5. Tokens emitidos por usuarios (FungibleAsset)

Una salida FungibleAsset representa un token emitido por un usuario con suministro fijo. El campo `extra_data` almacena la condicion de gasto seguida de metadatos del activo:

```
extra_data = [condition_bytes][version][asset_id][total_supply][ticker_len][ticker]
```

**Identidad del activo.** El `asset_id` se deriva de la transaccion genesis: `BLAKE3("DOLI_ASSET" || genesis_tx_hash || output_index)`. Esto lo hace unico por construccion — dos emisiones no pueden producir el mismo asset_id porque dos transacciones no comparten un hash.

**Suministro fijo.** El suministro total se establece en la emision y se codifica en cada UTXO que porta el token. El protocolo no impone invariantes de suministro entre UTXOs — esa es responsabilidad del indexador. Lo que el protocolo impone: la estructura de `extra_data` es valida, la condicion es satisfacible, y la salida sigue las reglas estandar de UTXO.

**Ticker.** Hasta 16 caracteres ASCII. `DOGEOLI`, `STBL`, `GOLD` — el ticker es metadatos para legibilidad humana, almacenado en cadena y consultable a traves del RPC.

**Lo que esto permite:** meme coins, stablecoins, puntos de lealtad, valores tokenizados, monedas de juegos — cualquier escenario donde se necesite un token fungible de suministro fijo. El token vive en la misma cadena que DOLI, validado por los mismos productores, a la misma velocidad. Sin sidechain, sin puente, sin wrapper.

**Estado:** El tipo de salida esta implementado y validado por el protocolo. No se han creado tokens emitidos por usuarios en mainnet al momento de esta publicacion.

### 3.6. Puentes entre cadenas (BridgeHTLC)

Una salida BridgeHTLC es un HTLC estandar con metadatos de enrutamiento para atomic swaps entre cadenas. El campo `extra_data` almacena la condicion HTLC seguida de metadatos del puente:

```
extra_data = [condition_bytes][version][target_chain][addr_len][target_address]
```

La condicion es siempre un HTLC: `(Hashlock(h) AND Timelock(t)) OR TimelockExpiry(t+d)`. Los metadatos indican a las contrapartes en que cadena bloquear y donde.

**Cadenas soportadas:**

| Cadena | ID | Formato de direccion | Soporte de Hashlock |
|--------|-----|---------------------|---------------------|
| Bitcoin | 1 | Base58/Bech32 | Nativo (OP_SHA256, OP_HASH160) |
| Ethereum | 2 | Hex con prefijo 0x | Contrato Solidity de 30 lineas |
| Monero | 3 | Estandar/Integrada | Nativo (firmas adaptoras Ed25519) |
| Litecoin | 4 | Base58/Bech32 | Nativo (igual que Bitcoin) |
| Cardano | 5 | Bech32 | Script Plutus |

**Protocolo de atomic swap:**

```
1. Alice (DOLI) genera secreto S, computa H = BLAKE3(S)
2. Alice bloquea X DOLI en BridgeHTLC(H, lock=L, expiry=E, chain=Bitcoin, to=Bob_BTC)
3. Bob ve el bloqueo en la cadena DOLI, verifica H
4. Bob bloquea Y BTC en Bitcoin HTLC con el mismo hash H, expiracion mas corta
5. Alice reclama los BTC de Bob revelando S en Bitcoin
6. Bob lee S de Bitcoin, reclama los DOLI de Alice revelando S en DOLI
7. Si Bob nunca bloquea → Alice reembolsa despues de E
8. Si Alice nunca reclama → Bob reembolsa despues de su expiracion en Bitcoin
```

Ambos lados estan protegidos. Ninguno puede perder fondos. La revelacion de la preimagen en una cadena habilita el reclamo en la otra. Este es el mismo mecanismo que asegura la Lightning Network — aplicado entre cadenas.

**Lo que esto no es.** Esto no es un puente con validadores, multisigs o custodios. No hay comite de puente. No hay token envuelto. No hay TVL que explotar. Cada swap es un UTXO independiente con un hash lock. La unica suposicion de confianza es que ambas cadenas incluiran transacciones antes de sus respectivas expiraciones — la misma suposicion subyacente a toda blockchain.

**Lo que esto elimina.** Cada gran hackeo de puentes — Ronin ($624M), Wormhole ($326M), Nomad ($190M), Harmony ($100M) — exploto el mismo patron: un comite pequeno custodiando un pool grande. DOLI no tiene pool. Cada swap es punto a punto, financiado por los participantes, asegurado por matematicas. No hay nada que hackear porque no hay nada que custodiar.

**Estado:** El tipo de salida BridgeHTLC esta implementado y validado on-chain. No se ha ejecutado ningun atomic swap cross-chain todavia — esto requiere integracion de contraparte en cada cadena objetivo.

### 3.7. Separacion de testigos (estilo SegWit)

Gastar una salida condicionada requiere un testigo — los datos que satisfacen las condiciones. Un Hashlock requiere la preimagen. Una condicion Signature requiere una firma de la clave correspondiente. Un Multisig requiere N firmas.

Los testigos se almacenan en el campo `extra_data` de la transaccion, separados del hash de firma. El mensaje de firma cubre entradas y salidas pero excluye los datos de testigo — la misma separacion que Bitcoin SegWit introdujo para resolver la maleabilidad de transacciones.

```
signing_hash = BLAKE3(version || tx_type || inputs || outputs)
    ↑ excluye extra_data (testigos)

tx_hash = BLAKE3(version || tx_type || inputs || outputs || extra_data)
    ↑ incluye extra_data (compromiso inmutable)
```

Esto previene un problema circular: un testigo de Signature debe firmar un hash que no incluya el testigo mismo. El testigo se compromete en el `tx_hash` completo para inmutabilidad pero se excluye del `signing_hash` para constructabilidad.

### 3.8. Lo que esto permite

**Sin una maquina virtual:**

- **Intercambios descentralizados:** Atomic swaps entre DOLI y cualquier cadena que soporte hash locks. Sin intermediario, sin custodia, sin riesgo de contraparte.
- **Canales de pago:** Transacciones fuera de cadena con liquidacion en cadena. Los HTLCs permiten una red equivalente a Lightning de forma nativa.
- **Custodia multipartita:** Tesorias corporativas, DAOs, herencias — cualquier escenario que requiera autorizacion N-de-M.
- **Deposito en garantia sin confianza:** Comprador, vendedor y arbitro cada uno posee una clave. Cualquier par puede liberar los fondos.
- **Calendarios de vesting:** Salidas con bloqueo temporal para asignaciones de equipo, subvenciones u obligaciones contractuales.
- **NFTs nativos:** Arte digital, tokens de identidad, certificados — activos unicos con condiciones de gasto componibles, sin despliegue de contratos.
- **Tokens emitidos por usuarios:** Meme coins, stablecoins, puntos de lealtad — tokens de suministro fijo en la capa base, sin necesidad de sidechain.
- **Puentes entre cadenas:** Atomic swaps sin confianza con Bitcoin, Ethereum, Monero, Litecoin y Cardano. Sin comite de puente, sin tokens envueltos, sin riesgo custodial.

**Sin estado mutable compartido:**

Cada salida es independiente. Gastar una salida no puede afectar a otra. No hay reentrancia, no hay front-running, no hay MEV. Las transacciones son completamente paralelizables — la validacion escala linealmente con los nucleos.

### 3.9. Lo que esto no permite

Las salidas de DOLI no pueden mantener estado persistente entre transacciones. No hay almacenamiento en cadena, no hay bucles, no hay computacion arbitraria. Esto es deliberado.

Las aplicaciones que requieren estado compartido — creadores de mercado automatizados, protocolos de prestamo, gobernanza en cadena con votacion compleja — pertenecen a la Capa 2 o a cadenas especificas de aplicacion que liquidan en DOLI.

La capa base proporciona: **transferencia de valor, ordenamiento anclado al tiempo, condiciones de gasto programables, activos nativos y liquidacion entre cadenas sin confianza.** Todo lo demas se construye encima.

---

## 4. Servidor de marcas de tiempo

La solucion comienza con un servidor de marcas de tiempo distribuido. La red actua como un servidor de marcas de tiempo tomando un hash de un bloque de elementos a ser marcados temporalmente y publicando ampliamente el hash. La marca de tiempo prueba que los datos debieron haber existido en ese momento para entrar en el hash.

```
                     ┌──────────────────┐
                     │      Block       │
                     ├──────────────────┤
                     │  Previous hash   │
                     │  Timestamp       │
 Transactions ───▶   │  Transactions    │
                     │  Prueba de Retardo │
                     └──────────────────┘
                              │
                              ▼
                     ┌──────────────────┐
                     │      Block       │
                     ├──────────────────┤
                     │  Previous hash   │
                     │  Timestamp       │
                     │  Transactions    │
                     │  Prueba de Retardo │
                     └──────────────────┘
```

Cada marca de tiempo incluye la marca de tiempo anterior en su hash, formando una cadena. Cada marca de tiempo adicional refuerza las anteriores.

---

## 5. Prueba de Tiempo

Para implementar un servidor de marcas de tiempo distribuido sobre una base peer-to-peer, necesitamos un mecanismo que haga costosa la produccion de bloques y prevenga que ese costo sea evadido mediante paralelizacion o acumulacion de recursos.

La solucion es usar **pruebas de retardo secuencial** — funciones que imponen un tiempo minimo de reloj de pared por bloque mediante computacion inherentemente serial. La construccion esta inspirada en las Verifiable Delay Functions [2, 3] pero utiliza un primitivo mas simple (Seccion 5.1). Las propiedades esenciales son:

1. Requiere un numero fijo de operaciones secuenciales para computarse.
2. No puede acelerarse significativamente mediante paralelizacion.
3. Puede ser verificada por cualquier nodo (por recomputacion).

> **Nota:** La prueba de retardo demuestra que *N* operaciones secuenciales fueron ejecutadas — el tiempo es el limite inferior efectivo ya que no se conoce ninguna tecnica que acelere la computacion secuencial de hashes mediante paralelizacion. La prueba sirve como latido (prueba de presencia), no como fuente de aleatoriedad. La seleccion de productor es una funcion pura de `(slot, ActiveSet(epoch))`, fijada al inicio del epoch, independiente de la velocidad de la prueba. Hardware mas rapido no proporciona ventaja de programacion.

Para cada bloque, el productor debe calcular:

```
input  = HASH(prefix || previous_hash || tx_root || slot || producer_key)
output = HASH^n(input)
```

Donde *n* es el parametro de dificultad que determina cuanto tiempo toma la computacion.

### 5.1. Construccion de la Prueba de Retardo

DOLI utiliza una **cadena de hash iterada** (BLAKE3), no una VDF algebraica sobre grupos de orden desconocido (Wesolowski [3], Pietrzak). La distincion importa:

| Propiedad | VDF algebraica (Wesolowski) | Cadena de hash iterada (DOLI) |
|----------|---------------------------|---------------------------|
| Verificacion | *O(log T)* — casi constante | *O(T)* — debe recomputarse |
| Configuracion confiable | Requerida (grupo RSA) | Ninguna |
| Resistencia cuantica | Incierta | Basada en hash (conservadora) |
| Implementacion | Compleja (GMP/enteros grandes) | Simple (~10 lineas) |

Las VDFs algebraicas ofrecen verificacion *O(log T)*, lo cual es critico cuando el parametro de retardo *T* es grande (minutos a horas). La prueba de retardo de bloque de DOLI requiere solo *T* = 800,000 iteraciones (~55ms), haciendo aceptable la verificacion *O(T)* — cada nodo recomputa la cadena en los mismos ~55ms.

La concesion es deliberada: DOLI gana simplicidad, auditabilidad y ausencia de configuracion confiable al costo de verificacion lineal. Para una prueba de latido donde *T* es pequeno, esta es la eleccion de ingenieria correcta.

```
Input: prev_hash ∥ slot ∥ producer_key
         │
         ▼
    ┌─────────┐
    │ BLAKE3  │ ◄──┐
    └────┬────┘    │
         │         │
         └─────────┘  × T iterations (T = 800,000)
         │
         ▼
      Output: h_T = H^T(input)
```

**Verificacion:** Un verificador recomputa *h_T = H^T(input)* y comprueba que *h_T == salida_declarada*. La dependencia secuencial *h_{i+1} = H(h_i)* previene la paralelizacion. No se conoce ningun atajo para computar *H^T* mas rapido que *T* evaluaciones secuenciales para BLAKE3 o cualquier funcion hash criptografica — esta es una suposicion estandar en criptografia basada en hash, no un limite inferior demostrado. La seguridad de la prueba de retardo descansa sobre esta suposicion, que compartimos con todas las construcciones de hash iterado incluyendo la Proof of History de Solana [4].

### 5.2. Estructura temporal

La red define el tiempo de la siguiente manera:

```
GENESIS_TIME = 2026-03-10T23:54:33Z (UTC)
```

Un slot es 10 segundos. Un numero de slot se deriva deterministicamente de la marca de tiempo:

```
slot = floor((timestamp - GENESIS_TIME) / 10)
```

Un epoch es 360 slots (1 hora). En los limites de epoch, el conjunto activo de productores se actualiza.

| Unidad | Slots      | Duracion  |
|--------|------------|-----------|
| Slot   | 1          | 10 seg    |
| Epoch  | 360        | 1 hora    |
| Dia    | 8,640      | 24 horas  |
| Era    | 12,614,400 | ~4 anos   |

### 5.3. Parametros de iteracion

Cada red define un conteo fijo de iteraciones calibrado para ~55ms en CPUs modernas:

```
T_BLOCK = 800,000 iterations (~55ms)
```

Con slots de 10 segundos, la prueba de retardo toma ~55ms, dejando el resto para la construccion y propagacion del bloque. El conteo fijo de iteraciones asegura que todos los nodos computen pruebas identicas — no se necesita calibracion por nodo ni ajuste dinamico.

Todo sistema de consenso impone un recurso escaso. En DOLI, ese recurso es el tiempo secuencial.

---

## 6. Red

Los pasos para operar la red son los siguientes:

1. Las nuevas transacciones se difunden a todos los nodos.
2. Cada productor elegible recolecta nuevas transacciones en un bloque.
3. El productor asignado al slot computa la prueba de retardo.
4. El productor difunde el bloque a la red.
5. Los nodos aceptan el bloque solo si todas las transacciones en el son validas y la prueba de retardo es correcta.
6. Los nodos expresan su aceptacion del bloque trabajando en crear el siguiente bloque, usando el hash del bloque aceptado como el hash previo.

Los nodos siempre consideran la cadena que cubre mas tiempo como la correcta y continuaran trabajando en extenderla. Si dos nodos difunden versiones diferentes del siguiente bloque simultaneamente, algunos nodos pueden recibir una u otra primero. En ese caso, trabajan en la primera que recibieron pero guardan la otra rama en caso de que se vuelva mas larga. El empate se rompe cuando el siguiente bloque es producido y una rama cubre mas slots; los nodos que estaban trabajando en la otra rama entonces cambian a la mas larga.

### 6.1. Validez de bloques

Un bloque *B* es valido si:

1. `B.timestamp > prev_block.timestamp`
2. `B.timestamp <= network_time + DRIFT`
3. `B.slot` se deriva correctamente de `B.timestamp`
4. `B.slot > prev_block.slot`
5. `B.producer` tiene un rango valido para `B.slot` (despues del periodo de arranque)
6. `verify_hash_chain(preimage, B.delay_output, T) == true`
7. Todas las transacciones en el bloque son validas

### 6.2. Sincronizacion de reloj

El consenso depende de que los nodos tengan relojes razonablemente sincronizados. Los nodos se sincronizan mediante:

- Servidores NTP
- Desplazamiento mediano de los pares conectados

```
network_time = local_clock + median(peer_offsets)
```

Los bloques con marcas de tiempo fuera de la ventana aceptable son rechazados.

### 6.3. Rendimiento

Con tiempos de bloque de 10 segundos y un tamano base de bloque de 2 MB (duplicandose cada era, con tope en 32 MB):

| Metrica              | Era 1          | Era 2          | Era 4 (tope)   |
|----------------------|----------------|----------------|----------------|
| Tiempo de bloque     | 10 segundos    | 10 segundos    | 10 segundos    |
| Tamano maximo de bloque | 2 MB        | 4 MB           | 32 MB          |
| Transaccion promedio | ~250 bytes     | ~250 bytes     | ~250 bytes     |
| TPS maximo teorico   | ~800           | ~1,600         | ~12,800        |
| TPS practico         | 200-400        | 400-800        | 3,000-6,000    |

DOLI no compite en rendimiento bruto. Compite en accesibilidad:

| Sistema      | TPS       | Hardware minimo para participar |
|--------------|-----------|--------------------------------|
| Bitcoin      | ~7        | ASIC ($5,000+)                 |
| Ethereum PoS | ~30       | 32 ETH + servidor ($100K+)     |
| Solana       | ~4,000    | Servidor 256GB RAM ($10K+)     |
| DOLI         | ~400      | Cualquier CPU ($5/mes VPS)     |

400 TPS en un VPS de $5/mes es una proposicion diferente a 4,000 TPS en hardware que la mayoria de la gente no puede costear. El rendimiento es suficiente para un sistema de efectivo; la accesibilidad es suficiente para la participacion global. El calendario de duplicacion por era asegura que la capacidad crezca con la madurez de la red.

---

## 7. Registro de productores

En una red abierta, cualquiera puede crear identidades sin costo. Permitir la creacion ilimitada y gratuita de identidades expondria a la red a ataques Sybil donde un atacante inunda el sistema con nodos falsos.

Para prevenir esto, el registro requiere completar una prueba de retardo secuencial cuya dificultad impone un tiempo minimo de reloj de pared por identidad.

```
input  = HASH(prefix || public_key || epoch)
output = HASH^T(input)    where T = T_REGISTER_BASE = 5,000,000 iterations (~30 seconds)
```

Un registro es valido si:

1. La prueba de retardo se verifica correctamente con `T_REGISTER_BASE` iteraciones.
2. El epoch es el actual o el anterior.
3. La clave publica no esta ya registrada.
4. El bond de activacion esta incluido.

### 7.1. Dificultad de registro

La dificultad de registro es fija:

```
T_registration = T_REGISTER_BASE = 5,000,000 iterations (~30 seconds)
```

Esto es deliberadamente constante. El costo de capital del bond de activacion (10 DOLI) es el principal disuasivo Sybil; la prueba de retardo agrega un piso temporal que previene el registro masivo instantaneo independientemente del capital. Un atacante con *M* maquinas puede registrar *M* identidades en paralelo, pero cada una aun requiere ~30 segundos de computacion secuencial mas el capital del bond.

### 7.2. Bond de activacion

Cada registro de productor debe bloquear un bond de activacion de 10 DOLI (1 unidad de bond). La unidad de bond es fija y no cambia entre eras.

```
BOND_UNIT = 10 DOLI (fixed across all eras)
```

Las recompensas de bloque se reducen a la mitad cada era (~4 anos), haciendo la participacion temprana mas gratificante:

| Era | Anos  | Bond    | Recompensa | Bloques para recuperar bond |
|-----|-------|---------|------------|----------------------------|
| 1   | 0-4   | 10 DOLI | 1.0        | 10                         |
| 2   | 4-8   | 10 DOLI | 0.5        | 20                         |
| 3   | 8-12  | 10 DOLI | 0.25       | 40                         |
| 4   | 12-16 | 10 DOLI | 0.125      | 80                         |
| 5   | 16-20 | 10 DOLI | 0.0625     | 160                        |

### 7.3. Apilamiento de bonds

Los productores pueden aumentar su stake hasta 3,000 veces la unidad de bond base.

```
BOND_UNIT = 10 DOLI
MIN_STAKE = 1 × BOND_UNIT (10 DOLI)
MAX_STAKE = 3,000 × BOND_UNIT (30,000 DOLI)
```

La seleccion utiliza round-robin deterministico, no loteria probabilistica. Cada unidad de bond otorga un ticket en la rotacion:

**Ejemplo (3 productores, 10 unidades de bond totales):**

```
Alice: 1 bond unit  (10 DOLI)   → 1 block every 10 slots
Bob:   5 bond units (50 DOLI)   → 5 blocks every 10 slots
Carol: 4 bond units (40 DOLI)   → 4 blocks every 10 slots
```

A 1 DOLI de recompensa por bloque:

- Alice gana 1 DOLI por cada 10 slots (10% ROI por ciclo)
- Bob gana 5 DOLI por cada 10 slots (10% ROI por ciclo)
- Carol gana 4 DOLI por cada 10 slots (10% ROI por ciclo)

**Todos los productores obtienen un porcentaje de ROI identico independientemente del tamano de su stake.**

| Parametro             | Valor                    |
|-----------------------|--------------------------|
| Unidad de bond        | 10 DOLI                  |
| Stake minimo          | 10 DOLI (1 bond)         |
| Stake maximo          | 30,000 DOLI (3,000 bonds) |
| Recompensa de bloque (Era 1) | 1 DOLI            |

#### Accesibilidad a escala

En la madurez de la red (18,000 bonds totales entre todos los productores):

| Tu stake   | Bonds | Bloques/Semana | Ingreso/Semana | Hardware     |
|-----------|-------|----------------|----------------|--------------|
| 10 DOLI   | 1     | ~3             | ~3 DOLI        | Cualquier CPU|
| 100 DOLI  | 10    | ~34            | ~34 DOLI       | Cualquier CPU|
| 1,000 DOLI| 100   | ~336           | ~336 DOLI      | Cualquier CPU|

Sin equipos de mineria. Sin staking pools. Sin requisitos minimos de hardware. Un VPS de $5/mes es suficiente.

### 7.4. Ciclo de vida del bond

El bond tiene un periodo de compromiso de 4 anos con seguimiento FIFO por bond:

```
T_commitment = 12,614,400 blocks (~4 years)
```

Cada bond rastrea su propio tiempo de creacion. El retiro utiliza orden FIFO (los bonds mas antiguos primero), con penalizacion calculada individualmente por bond segun su edad.

**El pago del retiro es instantaneo** — los fondos se devuelven en el mismo bloque. Sin demora de 7 dias. Sin paso de reclamacion separado. La eliminacion del bond del conjunto activo toma efecto en el siguiente limite de epoch.

El retiro anticipado incurre en una penalizacion escalonada basada en la edad individual del bond:

| Edad del bond | Penalizacion | Devuelto |
|---------------|-------------|----------|
| < 1 ano       | 75%         | 25%      |
| 1-2 anos      | 50%         | 50%      |
| 2-3 anos      | 25%         | 75%      |
| 3+ anos       | 0%          | 100%     |

Un productor con bonds de edades mixtas puede retirar selectivamente. Los bonds mas antiguos (menor penalizacion) se retiran primero. Esto recompensa el compromiso a largo plazo mientras permite una salida flexible.

Todas las penalizaciones se queman permanentemente, removiendo monedas de la circulacion.

---

## 8. Seleccion de productores

Para cada slot, una funcion determinista selecciona al productor de bloques. Sea *P* = {*p_1*, ..., *p_n*} el conjunto activo ordenado por clave publica, y *b(p_i)* el conteo de bonds del productor *p_i*. Definimos *B* = Sigma *b(p_i)*.

```
producer(s) = p_j  where j = min{j : Σ_{i=1}^{j} b(p_i) > s mod B}
```

La funcion es pura: `producer(s) = f(s, ActiveSet(epoch(s)))`. No depende de ningun valor que el productor actual pueda influenciar — ni `prev_hash`, ni el ordenamiento de transacciones, ni marcas de tiempo dentro de la ventana de deriva. **Grinding es imposible porque el calendario es una funcion solo del tiempo, fijado al inicio del epoch.**

### 8.1. Mecanismo de respaldo

Para evitar slots vacios cuando el productor primario esta fuera de linea, 5 rangos de respaldo se activan en ventanas secuenciales de 2 segundos:

| Tiempo en slot | Productor elegible |
|----------------|-------------------|
| 0s - 2s        | solo rango 0      |
| 2s - 4s        | solo rango 1      |
| 4s - 6s        | solo rango 2      |
| 6s - 8s        | solo rango 3      |
| 8s - 10s       | solo rango 4      |

Cada rango tiene una ventana exclusiva de 2 segundos. Un bloque del rango *N* es valido solo si `timestamp >= slot_start + N x 2s`. Si llegan multiples bloques validos para el mismo slot, el de menor rango gana.

### 8.2. Comparacion con sistemas existentes

Los pools existen en PoW y PoS porque las recompensas son probabilisticas — la varianza obliga a los pequenos participantes a delegar el control a operadores centralizados. El round-robin deterministico de DOLI elimina la varianza por completo (ver Seccion 7.3), y el pool de recompensas por epoch integrado (Seccion 10.5) distribuye recompensas directamente en cadena a todos los productores calificados. Los pools externos no pueden ofrecer un mejor trato.

| Sistema      | Seleccion                  | Varianza | Pools | Energia        | Hardware minimo     |
|--------------|----------------------------|----------|-------|----------------|---------------------|
| Bitcoin      | Loteria (hashpower)        | Alta     | Si    | ~150 TWh/ano   | ASIC ($5,000+)      |
| Ethereum PoS | Loteria (stake)            | Media    | Si    | ~2.6 GWh/ano   | 32 ETH ($100K+)     |
| Solana PoH   | Calendario (stake)         | Baja     | Si    | ~4 GWh/ano     | Servidor $10,000+   |
| DOLI PoT     | Round-robin deterministico | **Cero** | **Integrado** | **Despreciable** | **Cualquier CPU ($5/mes)** |

Solana usa Proof of History como reloj, pero la seleccion de lider sigue siendo ponderada por stake con elementos probabilisticos y requiere hardware de alto rendimiento. DOLI usa la prueba de retardo puramente como latido — la seleccion de lider es una funcion pura de `(slot, ActiveSet(epoch))`. No existe ventaja de hardware.

---

## 9. Seleccion de cadena

Cuando existen multiples cadenas validas, los nodos deben acordar cual seguir.

### 9.1. Eleccion de fork basada en peso

La cadena canonica es la que tiene el mayor peso acumulado de productores:

```
accumulated_weight(block) = accumulated_weight(parent) + producer_weight
```

El peso del productor se deriva de la antiguedad:

| Anos activo | Peso |
|-------------|------|
| 0           | 1.00 |
| 1           | 1.75 |
| 2           | 2.50 |
| 3           | 3.25 |
| 4+          | 4.00 |

El peso sigue una formula continua: `peso = 1.0 + min(anos, 4) × 0.75`.

Esto previene ataques donde un atacante crea muchos bloques de nuevos productores para superar una cadena construida por productores establecidos.

---

## 10. Incentivo

Las recompensas no se distribuyen por bloque. En cambio, el protocolo acumula recompensas de bloque en un **pool de epoch** y las distribuye una vez por epoch (cada 360 bloques, ~1 hora) a todos los productores que demostraron presencia continua durante el epoch. El protocolo actua como un **pool integrado** — sin pools de mineria externos, sin operadores, sin comisiones, sin confianza.

### 10.1. Emision

| Parametro        | Valor                     |
|------------------|---------------------------|
| Recompensa inicial | 1 DOLI/bloque           |
| Tiempo de bloque | 10 segundos               |
| Intervalo de halving | 12,614,400 bloques (~4 anos) |
| Suministro total | 25,228,800 DOLI           |

| Era | Anos  | Recompensa | Acumulado   | % del total |
|-----|-------|------------|-------------|-------------|
| 1   | 0-4   | 1.0        | 12,614,400  | 50.00%      |
| 2   | 4-8   | 0.5        | 18,921,600  | 75.00%      |
| 3   | 8-12  | 0.25       | 22,075,200  | 87.50%      |
| 4   | 12-16 | 0.125      | 23,652,000  | 93.75%      |
| 5   | 16-20 | 0.0625     | 24,440,400  | 96.88%      |
| 6   | 20-24 | 0.03125    | 24,834,600  | 98.44%      |

### 10.2. Distribucion de recompensas por epoch

En el primer bloque de cada nuevo epoch, el productor de bloques emite una unica transaccion de recompensa distribuyendo el pool acumulado a todos los productores calificados, proporcionalmente a su conteo de bonds:

```
epoch_pool = Σ block_reward(h) for h in [epoch_start, epoch_end)
reward(i)  = epoch_pool × bonds(i) / Σ qualifying_bonds
```

Solo los productores que atestiguaron en el 90% o mas de las ventanas de attestation de 1 minuto del epoch califican. Los no calificados no reciben nada; su parte se redistribuye a los productores calificados.

Esto produce un UTXO por productor por epoch en lugar de uno por bloque — eliminando el polvo de recompensas mientras se mantiene la misma emision total.

### 10.3. Attestation de actividad

La produccion de bloques demuestra que un productor estuvo en linea durante sus slots asignados. Pero con programacion determinista, un productor sabe exactamente cuales slots son suyos y puede estar fuera de linea el resto del tiempo.

Para demostrar presencia **continua**, cada productor firma una attestation de actividad cada minuto usando ambas claves Ed25519 y BLS12-381:

```
attestation = Sign(block_hash || slot)
```

El hash del bloque demuestra que el productor no solo esta vivo sino que activamente sigue y valida la cadena. Las attestations se difunden por gossip a la red. Cada productor de bloques registra:

1. Un **bitfield** en la cabecera del bloque (`presence_root`) — un bit por productor (atestiguado o no), soportando hasta 256 productores (32 bytes)
2. Una **firma BLS agregada** en el cuerpo del bloque — prueba criptografica de que el bitfield es honesto

La firma BLS agregada comprime todas las firmas individuales de attestation en una sola verificacion. Un bit falso — afirmando que un productor atestiguó cuando no lo hizo — causa que la verificacion de la firma agregada falle. El bloque es rechazado.

En el limite de epoch, cada nodo escanea los bitfields registrados en los bloques del epoch y cuenta los minutos de attestation por productor. Cada epoch abarca 60 minutos de attestation (uno por cada 6 slots). El umbral es 90%: un productor debe atestiguar en al menos 54 de 60 minutos para calificar para recompensas. Deterministico: cada nodo lee la misma cadena, computa los mismos conteos, coincide en la misma calificacion.

### 10.4. Diseno de doble clave

Los productores poseen dos pares de claves:

| Clave | Curva | Proposito |
|-------|-------|-----------|
| Ed25519 | Curve25519 | Transacciones, firma de bloques |
| BLS | BLS12-381 | Agregacion de attestation de actividad |

Ed25519 es mas rapido para operaciones de firma unica (firma de bloques, firma de transacciones). BLS se usa unicamente para attestation porque es el unico esquema que soporta agregacion de firmas — comprimiendo N firmas en una para eficiencia en cadena.

Ambas claves se registran en cadena en el registro del productor. La clave publica BLS se almacena en la transaccion de registro.

### 10.5. Pool integrado

Los pools de mineria tradicionales existen porque las recompensas en PoW y la mayoria de los sistemas PoS son probabilisticas — los pequenos participantes experimentan alta varianza y deben delegar a operadores centralizados para suavizar ingresos.

DOLI elimina la necesidad de pools externos por completo. El protocolo mismo es el pool:

| Propiedad | Pool de mineria externo | Recompensas de epoch DOLI |
|-----------|------------------------|---------------------------|
| Operador | Tercero (cobra comision) | Protocolo (sin comision) |
| Confianza | Confiar en el operador | Sin confianza (matematica en cadena) |
| Distribucion | El pool decide las divisiones | Determinista: bonds x calificacion |
| Centralizacion | Concentra poder | Cada productor opera su propio nodo |
| Varianza de recompensa | Suavizada por el pool | Cero — determinista por diseno |

Cada productor participa automaticamente. Las recompensas se distribuyen proporcionalmente por peso de bonds a todos los productores calificados. Ningun intermediario puede ofrecer un mejor trato que el protocolo mismo.

### 10.6. Comisiones

```
fee = sum(inputs) - sum(outputs)
```

La comision va al productor del bloque. Una tarifa minima previene el spam.

### 10.7. Madurez de recompensas

Las salidas de transacciones de recompensa de epoch requieren 6 confirmaciones (~1 minuto) antes de poder gastarse.

### 10.8. Crecimiento compuesto

Las recompensas de DOLI se componen en capital productivo. Cada DOLI ganado por produccion de bloques puede reinvertirse como unidades de bond adicionales, aumentando las asignaciones futuras de bloques proporcionalmente.

**Definicion.** Sea *b* = conteo de bonds de un productor, *B* = total de bonds de la red, *R* = recompensa de bloque, *S* = slots por semana (60,480). Las ganancias semanales del productor y el tiempo de duplicacion son:

```
E(b) = S · R · b / B          (weekly earnings)
D    = BOND_UNIT · B / (S · R) (doubling time in weeks)
```

*D* es independiente de *b*. Un productor con 1 bond y un productor con 1,000 bonds ambos duplican su stake en *D* semanas. La tasa de crecimiento es uniforme; solo la magnitud absoluta difiere.

**Ejemplo (Era 1, *B* = 18,000, *R* = 1 DOLI):**

*D* = 10 x 18,000 / (60,480 x 1) ≈ 3 semanas.

```
Week 0:   1 bond     →   3.3 DOLI/week
Week 3:   2 bonds    →   6.6 DOLI/week
Week 6:   4 bonds    →    13 DOLI/week
Week 12:  16 bonds   →    53 DOLI/week
Week 24:  256 bonds  →   853 DOLI/week
```

Comenzando con 10 DOLI, un productor que reinvierte todas las recompensas alcanza el tope de 3,000 bonds en meses. Esta trayectoria es calculable antes de que se produzca el primer bloque.

**Autorregulacion:** A medida que *B* crece, *D* aumenta proporcionalmente. El crecimiento rapido inicial converge naturalmente hacia una distribucion estable sin intervencion de gobernanza. Los entrantes tardios enfrentan tiempos de duplicacion mas largos pero se benefician de una red mas segura y valiosa.

---

## 11. Infracciones

### 11.1. Bloques invalidos

La red rechaza bloques que:

- Tienen pruebas de retardo invalidas
- Son producidos por el productor incorrecto para ese slot
- Tienen marcas de tiempo fuera de la ventana valida
- Contienen transacciones invalidas

El productor pierde la oportunidad del slot. El bond permanece intacto.

### 11.2. Inactividad

Si un productor falla en producir cuando es seleccionado durante 50 slots consecutivos:

- Es removido del conjunto activo
- El bond permanece bloqueado (sin penalizacion por inactividad)
- Puede reactivarse con una nueva prueba de retardo de registro

La inactividad no se castiga — se tolera. Un productor que se desconecta pierde ingresos (recompensas de bloque perdidas) pero no capital (el bond permanece intacto). Esta es una eleccion de diseno deliberada: penalizar el tiempo de inactividad desalentaria a los operadores pequenos con infraestructura menos confiable.

### 11.3. Doble produccion

Si un productor crea dos bloques diferentes para el mismo slot, cualquiera puede construir una prueba de esta infraccion.

**Penalizacion:**

- 100% del bond quemado permanentemente
- Exclusion inmediata del conjunto de productores
- Para reactivarse: nuevo registro con `T_registration x 2`

Esta es la unica infraccion que resulta en slashing porque es la unica que es inequivocamente intencional.

---

## 12. Seguridad

Si el atacante controla menos capacidad de computacion secuencial que la red honesta, la probabilidad de alcanzar la cadena honesta disminuye rapidamente con la diferencia en numero de slots.

En este sistema, un atacante no puede "acelerar" la produccion de una cadena alternativa anadiendo hardware paralelo, porque cada bloque requiere una computacion secuencial de duracion fija.

### 12.1. Costo de ataque

Para dominar la red, un atacante necesitaria:

1. Registrar mas productores que los honestos (costo en tiempo por identidad)
2. Bloquear mas bonds que los participantes honestos (costo economico)
3. Mantener esos productores activos (costo operativo)
4. Arriesgar la perdida total de bonds si se detecta doble produccion

El protocolo regula automaticamente la tasa a la que nuevas identidades pueden unirse.

### 12.2. Probabilidad de ataque

**Suposicion (Dureza secuencial).** Para una funcion hash criptografica *H*, computar *H^T(x)* requiere al menos *T* evaluaciones secuenciales de *H*. Ningun algoritmo puede producir *H^T(x)* en menos de *T* pasos, independientemente de los recursos paralelos. Esta es una suposicion estandar en criptografia basada en hash — no existe ningun contraejemplo para ninguna funcion hash considerada segura, pero tampoco existe una demostracion formal de este limite inferior.

**Teorema (Deficit secuencial).** Bajo la Suposicion de Dureza Secuencial, sea *T* el tiempo secuencial fijo por bloque. Un atacante que comienza una cadena alternativa con deficit *d* >= 1 bloques no puede reducir *d* independientemente de los recursos computacionales paralelos.

**Demostracion.** Sea *t_0* el tiempo en que el atacante comienza a bifurcar. Definimos:

- *H(t)* = longitud de la cadena honesta en el tiempo *t*
- *A(t)* = longitud de la cadena del atacante en el tiempo *t*
- *d(t) = H(t) - A(t)* = deficit en el tiempo *t*

En *t_0*: *d(t_0) = d* >= 1.

Cada bloque requiere exactamente *T* de computacion secuencial. El atacante produce a lo sumo un bloque por *T* segundos por cadena (dependencia secuencial: el bloque *i+1* requiere el hash del bloque *i*). La red honesta tambien produce a lo sumo un bloque por *T* segundos.

Despues de un tiempo transcurrido *Delta_t*:

```
H(t₀ + Δt) ≤ H(t₀) + ⌊Δt / T⌋
A(t₀ + Δt) ≤ A(t₀) + ⌊Δt / T⌋
```

Por lo tanto:

```
d(t₀ + Δt) = H(t₀ + Δt) − A(t₀ + Δt) ≥ d(t₀) = d
```

El deficit es monotonamente no decreciente. Anadir hardware paralelo permite computar multiples cadenas *independientes*, pero cada cadena es secuencial — el atacante no puede fusionar cadenas paralelas en una sola mas larga. QED

**Contraste con Proof of Work:** En PoW, un atacante con >50% de hashpower reduce el deficit probabilisticamente porque los intentos de hash son paralelizables. En Prueba de Tiempo, la dependencia secuencial *h_{i+1} = H(h_i)* hace que cada cadena sea inherentemente serial. El deficit del atacante esta acotado inferiormente por su valor inicial, independientemente del presupuesto.

El unico vector de ataque es controlar >50% de los slots ponderados por bonds, lo que requiere:

1. *T_registration* de tiempo secuencial por identidad (no puede paralelizarse por identidad)
2. *BOND_UNIT* de capital por identidad
3. 100% de riesgo de perdida de bond si se detecta doble produccion

### 12.3. La objecion de acumulacion de CPUs

Una objecion natural: "El tiempo no puede acumularse, pero la capacidad de probar retardos si — mas CPUs permiten mas identidades paralelas."

Esto es correcto y es por diseno. Un atacante con *M* maquinas puede registrar *M* identidades en paralelo, cada una completando *T_registration* independientemente. Sin embargo, cada identidad aun requiere:

1. **Tiempo secuencial:** *T_registration* segundos de reloj de pared (no puede reducirse anadiendo nucleos)
2. **Capital:** *BOND_UNIT* bloqueado por identidad (costo lineal en *M*)
3. **Presencia continua:** Un latido de prueba de retardo por slot por identidad (costo operativo lineal en *M*)

El costo de capital es *O(M)* — identico a Proof of Stake. DOLI no escapa de esto. Un atacante con recursos suficientes que puede costear *M* bonds enfrenta el mismo costo lineal de capital que en cualquier sistema PoS.

Lo que DOLI agrega es un **piso temporal** que PoS no tiene: incluso con capital ilimitado, registrar *M* identidades toma al menos *T_registration* de tiempo de reloj de pared por identidad. El registro requiere un *T_REGISTER_BASE* fijo (~30 segundos) de computacion secuencial por identidad mas *BOND_UNIT* de capital. Un ataque estilo PoS de "comprar el 51% del stake de la noche a la manana" requiere tanto tiempo como capital — ninguno de los dos es suficiente por si solo.

Comparemos con PoW: un atacante con *M* ASICs obtiene *Mx* hashpower inmediatamente, sin demora temporal por identidad. En DOLI, las mismas *M* maquinas producen *M* identidades, pero el pipeline de registro impone un cuello de botella secuencial por identidad y el requisito de capital escala linealmente.

El sistema no reclama inmunidad ante adversarios adinerados — ningun sistema puede. Reclama dos cosas: (1) el capital solo no puede eludir el piso temporal, y (2) una vez registrado, el costo operativo por identidad del atacante es permanente, no un gasto unico.

### 12.4. Teorema de seguridad

**Teorema.** Sea *n* = total de slots ponderados por bonds por epoch. Un atacante que controla *f* < *n/2* slots ponderados por bonds no puede producir una cadena mas pesada que la red honesta sobre cualquier intervalo de *k* >= 1 epochs.

**Demostracion.** Definimos por epoch *e*:

- *S_h(e)* = conjunto de slots asignados a productores honestos
- *S_a(e)* = conjunto de slots asignados a productores atacantes
- *w(p)* = peso de antiguedad del productor *p* perteneciente a [1.0, 4.0]

El calendario es una funcion pura de `(slot, ActiveSet(epoch))` — ningun contenido de bloque lo influencia.

El peso acumulado de la cadena sobre *k* epochs:

```
W_h(k) = Σ_{e=1}^{k} Σ_{s ∈ S_h(e)} w(producer(s))
W_a(k) = Σ_{e=1}^{k} Σ_{s ∈ S_a(e)} w(producer(s))
```

Dado que *f < n/2*, tenemos *|S_a(e)| < |S_h(e)|* para todo *e*. Adicionalmente, la ponderacion por antiguedad (Seccion 9.1) penaliza las nuevas identidades: *w(nuevo) = 1* mientras *w(establecido) <= 4*. Por lo tanto *W_h(k) > W_a(k)* para todo *k* >= 1.

Por el Teorema del Deficit Secuencial (12.2), el atacante no puede compensar computando mas rapido — la dependencia secuencial de la cadena de hash previene la aceleracion paralela. QED

**Corolario.** Un atacante que parte de cero necesita ~3 anos de presencia sostenida antes de que su peso de antiguedad iguale al de un productor honesto establecido, incluso con conteo de bonds igual. La ventana de ataque esta por lo tanto acotada no solo por capital sino por tiempo calendario.

**Limitacion.** La antiguedad protege contra atacantes *tardios* — aquellos que intentan unirse y dominar una red establecida. No protege contra atacantes *tempranos y pacientes* que se registran durante la infancia de la red y acumulan antiguedad legitimamente junto a los productores honestos. Esto se mitiga por: (1) el costo de capital sigue escalando linealmente con el conteo de bonds, (2) riesgo de perdida del 100% del bond por doble produccion, y (3) el requisito de attestation — mantener *M* identidades al 90% de uptime durante anos tiene un costo operativo compuesto. Ningun sistema de consenso puede distinguir un adversario paciente de un participante legitimo; la defensa es hacer que la deshonestidad sostenida sea costosa, no imposible.

---

## 13. Recuperacion de espacio en disco

Una vez que la ultima transaccion en una moneda esta enterrada bajo suficientes bloques, las transacciones gastadas anteriores pueden descartarse para ahorrar espacio en disco. Las transacciones se hashean en un arbol Merkle, con solo la raiz incluida en el hash del bloque.

Una cabecera de bloque sin transacciones es aproximadamente 340 bytes. Con bloques cada 10 segundos, eso es ~1 GB por ano solo para cabeceras.

---

## 14. Verificacion simplificada de pagos

Es posible verificar pagos sin ejecutar un nodo completo. Un usuario solo necesita mantener una copia de las cabeceras de bloque de la cadena mas larga y obtener la rama Merkle que vincula la transaccion al bloque en el que fue marcada temporalmente. Esta capacidad esta soportada por las estructuras de datos del protocolo pero aun no esta implementada como una funcionalidad de cliente.

---

## 15. Privacidad

DOLI adopta el modelo de privacidad pseudonimo descrito por Nakamoto [1]: las transacciones son publicas, pero las identidades detras de las claves no lo son. Los usuarios pueden generar multiples direcciones para reducir la vinculacion. Esto proporciona privacidad equivalente a las divulgaciones de bolsas de valores publicas — los montos y flujos son visibles, los participantes no lo son.

---

## 16. Distribucion

No hay preminado, ICO, tesoro ni asignaciones especiales. Cada moneda en circulacion proviene de recompensas de bloques.

### 16.1. Bloque genesis

El bloque genesis contiene una unica transaccion coinbase con el mensaje:

> *"Time is the only fair currency."*

Esto sirve como declaracion de la filosofia del sistema. El timestamp del genesis esta embebido en el bloque, probando que no se minaron bloques antes de ese momento.

El bloque genesis contiene exactamente:

- Una transaccion coinbase con 1 DOLI (recompensa estandar)
- Cero transacciones adicionales

**No existen asignaciones ocultas.**

### 16.2. Arranque de la Red

Una cadena basada en Prueba de Tiempo enfrenta una dependencia circular en el lanzamiento: los productores necesitan bonds para producir bloques, pero los bonds requieren DOLI, y DOLI solo existe a traves de la produccion de bloques.

El protocolo resuelve esto en tres fases dentro de un unico epoch (360 bloques, ~1 hora):

**Fase 1 — Trabajo sin recompensa.** Cinco productores genesis reciben un placeholder temporal de planificacion (una entrada bond con hash cero) que permite al scheduler asignar slots. Este placeholder no tiene valor — existe unicamente para que el algoritmo round-robin tenga una entrada. Durante este primer epoch, cada recompensa de bloque va directamente al pool de recompensas. Los productores genesis no reciben nada.

**Fase 2 — Conversion automatica.** En el bloque 361 (primer bloque despues del epoch 0), el protocolo ejecuta `consume_genesis_bond_utxos`: recolecta todos los UTXOs acumulados en el pool, crea un bond real (10 DOLI) por cada productor genesis financiado enteramente desde el pool, y devuelve el sobrante al pool para la distribucion del epoch 1. Los placeholders temporales son reemplazados por UTXOs de bond reales respaldados por trabajo ya realizado.

**Fase 3 — Reglas iguales.** A partir del bloque 361, los productores genesis operan bajo las mismas reglas que cualquier participante futuro. Sus bonds maduran en el mismo calendario, ganan las mismas recompensas y enfrentan las mismas penalizaciones por retiro.

El resultado: los productores fundadores pagaron sus propios bonds con produccion real de bloques. No se crearon monedas fuera del calendario de emision estandar. No se otorgo ventaja alguna que cualquier productor futuro no reciba tambien a traves del mismo mecanismo.

**Los fundadores no recibieron privilegio alguno — pagaron el costo del arranque con trabajo.**

---

## 17. Inmutabilidad

Las transacciones son finales. No existen mecanismos para revertir transacciones, recuperar fondos o modificar el historial.

| Situacion            | Respuesta del protocolo |
|----------------------|------------------------|
| Claves privadas perdidas | Fondos perdidos permanentemente |
| Transaccion erronea  | No reversible          |
| Hackeo de exchange   | No reversible          |
| Orden judicial       | No ejecutable          |

**El codigo es ley. Las transacciones son finales.**

---

## 18. Actualizaciones del protocolo

El software requiere mantenimiento. Los errores deben corregirse. La pregunta es: quien decide?

En sistemas centralizados, el operador decide. En Bitcoin, el consenso informal entre desarrolladores, mineros y usuarios determina que cambios se adoptan. Esto funciona pero es lento y contencioso.

DOLI formaliza el proceso. Las actualizaciones son firmadas por mantenedores y revisadas por productores.

### 18.1. Firma de versiones

Cada version requiere firmas de 3 de 5 mantenedores. Una sola clave comprometida no puede impulsar codigo malicioso.

### 18.2. Periodo de veto

Cuando se publica una nueva version, los productores tienen 7 dias para revisarla. Cualquier productor puede votar en contra. Si el 40% o mas vota en contra, la actualizacion es rechazada.

| Votos de veto | Resultado |
|---------------|-----------|
| < 40%         | Aprobada  |
| >= 40%        | Rechazada |

El umbral esta ponderado por stake y antiguedad (bonds x multiplicador de antiguedad). Un atacante no puede crear muchos nodos nuevos para forzar la aprobacion de una actualizacion.

### 18.3. Adopcion

Despues de la aprobacion, los productores tienen 1 hora para actualizar. Los nodos que ejecutan versiones obsoletas no pueden producir bloques. Esto no es un castigo — es proteccion. Una vulnerabilidad en codigo antiguo afecta a toda la red.

La eleccion es simple: participar en el consenso con software actual, o no participar.

---

## 19. Red en produccion

DOLI no es una propuesta. La red descrita en este documento esta operativa.

A marzo de 2026, la red principal esta en su **fase de arranque** — operativa y produciendo bloques, pero con un conjunto reducido de productores operado principalmente por el equipo fundador en servidores geograficamente distribuidos. El codigo fuente es abierto, el estado de la cadena es verificable publicamente, y productores externos han comenzado a unirse.

| Metrica | Valor |
|---------|-------|
| Tiempo de bloque | 10 segundos |
| Computacion de prueba de retardo | ~55ms por bloque |
| Propagacion de bloques | < 500ms |
| Forks desde genesis | 0 |
| Tasa de slots perdidos | < 10% (mecanismo de respaldo) |
| Hardware de nodos | VPS estandar, cualquier CPU |
| Bond minimo | 10 DOLI |
| Productores activos | 14 (fase de arranque) |
| Productores externos | Incorporacion en progreso |

El conteo actual de productores refleja la fase de arranque descrita en la Seccion 16.2. Las propiedades de seguridad del protocolo se fortalecen a medida que productores independientes se unen — cada operador adicional aumenta el costo de un ataque >50% ponderado por bonds y reduce la dependencia del conjunto fundador. El objetivo es un conjunto de productores lo suficientemente grande como para que ninguna entidad unica controle una fraccion significativa de los slots ponderados por bonds.

```
Genesis:    March 2026
Consensus:  Proof of Time (delay proof heartbeat + deterministic round-robin)
Status:     Live
Source:     https://github.com/e-weil/doli
Explorer:   https://doli.network
```

---

## 20. Alcance

DOLI optimiza para mover valor con finalidad determinista, temporalidad predecible y condiciones de gasto extensibles. La capa base es intencionalmente minima — pero extensible por diseno.

Esta restriccion es una caracteristica. Un sistema que hace una cosa bien es mas seguro, mas auditable y mas resistente a la captura de gobernanza que un sistema que intenta ser un computador universal. Bitcoin demostro que un protocolo enfocado puede sostener una red de un billon de dolares. La complejidad no es un prerequisito para el valor.

La diferencia: el formato de salida de Bitcoin fue fijado en 2009. El formato de salida de DOLI fue disenado en 2026 con diecisiete anos de retrospectiva. El campo `extra_data` existe desde el genesis — sin SegWit, sin Taproot, sin hacks de compatibilidad hacia atras requeridos.

---

## 21. Conclusion

Hemos propuesto un sistema para transacciones electronicas que no requiere confianza en instituciones, ni un gasto masivo de energia, ni acumulacion de capital para participar en el consenso.

Comenzamos con el marco habitual de monedas hechas de firmas digitales, que proporciona un fuerte control de propiedad. Esto es incompleto sin una forma de prevenir el doble gasto. Para resolver esto, propusimos una red peer-to-peer que usa pruebas de retardo secuencial para anclar el consenso al tiempo.

**Los nodos votan con su tiempo.** La red no puede acelerarse con riqueza ni paralelizarse con hardware. Una hora de computacion secuencial es una hora, ya sea realizada por un individuo o un estado-nacion.

**Las recompensas son deterministas, no probabilisticas.** Un participante sabe exactamente cuando se producira su proximo bloque. El protocolo actua como un pool integrado, distribuyendo recompensas de epoch en cadena a todos los productores que demuestran presencia continua mediante attestations de actividad on-chain. Los pools externos son innecesarios. El participante mas pequeno recibe el mismo porcentaje de retorno que el mas grande.

La red es robusta en su simplicidad. Los nodos trabajan con poca coordinacion. No necesitan ser identificados, ya que los mensajes no se enrutan a ningun lugar particular y solo necesitan ser entregados con el mejor esfuerzo posible. Los nodos pueden irse y reincorporarse a la red a voluntad, aceptando la cadena mas pesada como prueba de lo que ocurrio mientras estuvieron ausentes.

**Las reglas se fijan en el genesis. La emision es predecible.**

Cualquier regla e incentivo necesario puede aplicarse con este mecanismo de consenso.

---

**DOLI v3.4.1**

*"El tiempo es la unica moneda justa."*

**E. Weil** · contacto: weil@doli.network

---
## Referencias

1. Nakamoto, S. (2008). *Bitcoin: A Peer-to-Peer Electronic Cash System.*

2. Boneh, D., Bonneau, J., Bunz, B., & Fisch, B. (2018). *Verifiable Delay Functions.* In Advances in Cryptology – CRYPTO 2018.

3. Wesolowski, B. (2019). *Efficient Verifiable Delay Functions.* In Advances in Cryptology – EUROCRYPT 2019. (Citado por contraste — DOLI usa cadenas de hash iteradas, no VDFs algebraicas. Ver Seccion 5.1.)

4. Yakovenko, A. (2018). *Solana: A new architecture for a high performance blockchain.* Utiliza hash iterado SHA-256 para Proof of History bajo la misma suposicion de dureza secuencial.
