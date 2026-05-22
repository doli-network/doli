# DOLI<sub>τ</sub>

## Ordenamiento Determinista mediante Iteraciones Lineales

### Un Sistema de Efectivo Electrónico Peer-to-Peer Basado en Tiempo Verificable

**I. Lozada** · ivan@doli.network | **A. Lozada** · antonio@doli.network

---

## Resumen

Proponemos un sistema de efectivo electrónico peer-to-peer donde el único recurso requerido para el consenso es el tiempo — el único recurso distribuido equitativamente entre todos los participantes.

La produccion de bloques sigue un scheduling determinista ponderado por bonds: cada productor activo recibe asignaciones de bloques proporcionales a su conteo de bonds. El protocolo distribuye recompensas cada epoch a traves de un pool integrado — proporcional a los bonds, sin pools de mineria externos, sin operadores, sin comisiones. Las recompensas se reinvierten en stake productivo, creando un crecimiento exponencial predecible para cada participante sin importar su tamano.

Un nuevo productor que recibe 10 DOLI puede reinvertir las recompensas de bloques para duplicar su stake a intervalos regulares. La tasa de duplicacion es identica para todos los participantes — uno o tres mil bonds. La presencia continua se demuestra mediante attestations de actividad on-chain — los productores que estan en linea y siguiendo la cadena califican para su parte. Sin loteria. Sin varianza. Sin pools. Solo tiempo.

Las transacciones se ordenan mediante pruebas de retardo secuencial — computaciones de hash iteradas que no pueden paralelizarse. No se requiere hardware especial. Cualquier CPU puede participar en el consenso. El resultado es un sistema donde el peso del consenso emerge del tiempo en lugar de la confianza, el capital o la escala.

Demostramos que los NFTs, tokens fungibles y puentes entre cadenas sin confianza pueden implementarse como tipos de salida UTXO nativos con condiciones de gasto declarativas, sin una maquina virtual, sin medicion de gas y sin comites de confianza — logrando una expresividad equivalente a los enfoques basados en VM para estos casos de uso mientras se mantiene un costo de verificacion acotado y predecible.

El protocolo fue disenado durante la transicion a la era de los agentes. Los errores llevan codigos estables y campos estructurados, el estado es explicito a traves del modelo UTXO, el scheduling es determinista y consultable, y los eventos de la cadena en vivo se transmiten por WebSocket. El tooling autonomo — agentes de IA que envian transacciones, monitorean el estado y se autocorrigen sin supervision — es un cliente de primera clase, no un anadido posterior (Seccion 19).

---

> **Esta red esta en produccion.** El sistema descrito en este documento esta operativo desde marzo de 2026. El codigo fuente es [abierto](https://github.com/doli-network/doli), el estado de la cadena es verificable publicamente a traves del [explorador de bloques](https://doli.network/explorer.html), y productores externos operan de forma independiente. Esto no es una propuesta — es la documentacion de un sistema en funcionamiento en su fase inicial de crecimiento. Agradecemos el escrutinio, la retroalimentacion y la colaboracion.

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
           | AmountGuard(min_amount, output_index)
           | OutputTypeGuard(expected_type, output_index)
           | RecipientGuard(pubkey_hash, output_index)
```

**Codificacion:** Las condiciones se serializan en `extra_data` como un formato binario compacto. Para transferencias basicas, `extra_data` esta vacio — la condicion por defecto es `Signature(owner)`.

**Costo de verificacion:** Cada condicion se resuelve en un numero fijo de operaciones criptograficas (verificaciones de firma, comparaciones de hash, comparaciones de altura). Sin bucles. Sin recursion. Sin computacion ilimitada. El costo de verificacion se conoce antes de la ejecucion.

**Introspeccion de transacciones (guards).** Las ultimas tres primitivas — `AmountGuard`, `OutputTypeGuard`, `RecipientGuard` — son las unicas condiciones que leen la transaccion que *gasta* (no solo el UTXO que se gasta). Cada una inspecciona exactamente una salida de la tx gastadora, identificada por `output_index`, y verifica una propiedad: monto minimo, tipo de salida, o pubkey_hash del destinatario. Habilitan patrones que la logica de firma pura no puede: ordenes limite (`AmountGuard` exige un minimo recibido), anti-redireccion en reclamos de bridge (`OutputTypeGuard` fuerza los fondos a una forma especifica de salida), y pagos condicionales (`RecipientGuard` fija el destino). Preservan la propiedad de costo fijo sin bucles porque cada guard es una sola comparacion de campo.

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
| BSC | 6 | Hex con prefijo 0x | Contrato HTLC BEP-20 |

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

Cada salida es independiente. Gastar una salida no puede afectar a otra. No hay reentrancia. Los ataques sandwich son estructuralmente imposibles (ver Seccion 3.9). Las transacciones son completamente paralelizables — la validacion escala linealmente con los nucleos.

### 3.9. Primitivos DeFi nativos

DOLI implementa operaciones DeFi centrales como tipos de transaccion nativos en lugar de contratos ejecutados en una VM. Los creadores de mercado automatizados (CreatePool, AddLiquidity, RemoveLiquidity, Swap), prestamos (CreateLoan, RepayLoan, LiquidateLoan, LendingDeposit, LendingWithdraw) y fraccionalizacion de NFTs (FractionalizeNft, RedeemNft) estan compilados en el binario del nodo como transiciones de estado validadas.

**Resistencia a sandwich por construccion.** Un swap consume un UTXO de pool atomicamente — dos swaps contra el mismo pool son mutuamente excluyentes por semantica UTXO, haciendo imposible el sandwich clasico de 3 transacciones. Otras formas de MEV (front-running, censura por el productor, arbitraje entre pools) siguen siendo posibles y se mitigan con metodos estandar: tolerancia de deslizamiento (slippage) en los swaps, multiples productores via distribucion de stake, y liquidacion L2 para mercados de alta frecuencia.

**Limitaciones conocidas.**
- *Techo de rendimiento por pool.* Solo un Swap contra un UTXO de pool dado puede entrar por bloque. Los pools con trafico alto se serializan en ~8,640 swaps/dia por pool en la Era 0. El sharding mediante multiples UTXOs de pool por par o la liquidacion L2 es la respuesta arquitectonica.
- *Sin replace-by-fee.* Una transaccion atascada no puede ser reemplazada por una version con tarifa mas alta. Child-pays-for-parent (CPFP) funciona como solucion parcial.

**Liquidacion L2.** Para aplicaciones que requieren computacion arbitraria, DOLI soporta liquidacion de rollups L2 sin permisos mediante transacciones `ZKSettle` que verifican pruebas de conocimiento cero contra una `verifying_key` comprometida en una salida `ZKRollup`. Cada rollup es su propio dominio de confianza — sin registro gobernado por maintainers.

### 3.10. Lo que esto no permite

Las salidas de DOLI no pueden mantener estado compartido persistente entre transacciones. No hay almacenamiento en cadena, no hay bucles, no hay computacion arbitraria. Esto es deliberado. Las aplicaciones que requieren estado compartido complejo mas alla de los primitivos nativos anteriores pertenecen a cadenas L2 que liquidan en DOLI mediante el mecanismo ZKSettle.

La capa base proporciona: **transferencia de valor, ordenamiento anclado al tiempo, condiciones de gasto programables, activos nativos, liquidacion entre cadenas sin confianza, AMM/prestamos nativos y verificacion de rollups L2.** Todo lo demas se construye encima.

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

> **Nota:** La prueba de retardo demuestra que *N* operaciones secuenciales fueron ejecutadas — el tiempo es el limite inferior efectivo ya que no se conoce ninguna tecnica que acelere la computacion secuencial de hashes mediante paralelizacion. La prueba sirve como latido (prueba de presencia), no como fuente de aleatoriedad. La seleccion de productor es una funcion pura de `(slot, ActiveSet(epoch), LivenessFilter)`, independiente de la velocidad de la prueba. Hardware mas rapido no proporciona ventaja de programacion.

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

Las VDFs algebraicas ofrecen verificacion *O(log T)*, lo cual es critico cuando el parametro de retardo *T* es grande (minutos a horas). La prueba de retardo de bloque de DOLI requiere solo *T* = 1,000 iteraciones (tiempo de computacion despreciable), haciendo la verificacion *O(T)* trivialmente rapida — cada nodo recomputa la cadena en microsegundos.

La concesion es deliberada: DOLI gana simplicidad, auditabilidad y ausencia de configuracion confiable al costo de verificacion lineal. Para una barrera anti-grinding liviana donde *T* es pequeno, esta es la eleccion de ingenieria correcta. El requisito de bond (10 DOLI por productor) es la defensa principal contra Sybil; la prueba de retardo sirve como latido a nivel de protocolo y barrera anti-flash, no como una prueba de trabajo intensiva en tiempo.

```
Input: prev_hash ∥ slot ∥ producer_key
         │
         ▼
    ┌─────────┐
    │ BLAKE3  │ ◄──┐
    └────┬────┘    │
         │         │
         └─────────┘  × T iterations (T = 1,000)
         │
         ▼
      Output: h_T = H^T(input)
```

**Verificacion:** Un verificador recomputa *h_T = H^T(input)* y comprueba que *h_T == salida_declarada*. La dependencia secuencial *h_{i+1} = H(h_i)* previene la paralelizacion. No se conoce ningun atajo para computar *H^T* mas rapido que *T* evaluaciones secuenciales para BLAKE3 o cualquier funcion hash criptografica — esta es una suposicion estandar en criptografia basada en hash, no un limite inferior demostrado. La seguridad de la prueba de retardo descansa sobre esta suposicion, que compartimos con todas las construcciones de hash iterado incluyendo la Proof of History de Solana [4].

**Verificacion paralela de multiples pruebas:** Cuando un bloque contiene multiples transacciones que requieren verificacion VDF (ej. varios registros de productores), el nodo verifica cada prueba en un hilo separado usando `thread::scope`. Cada prueba individual permanece secuencial, pero pruebas independientes se verifican concurrentemente. Esto asegura que bloques con muchos registros no creen un cuello de botella de verificacion.

### 5.2. Estructura temporal

La red define el tiempo de la siguiente manera:

```
GENESIS_TIME = 2026-04-22T05:58:30Z (UTC)
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

Cada red define un conteo fijo de iteraciones:

```
T_BLOCK = 1,000 iterations (tiempo de computacion despreciable)
```

Con slots de 10 segundos, la prueba de retardo se completa en microsegundos, dejando el resto para la construccion y propagacion del bloque. El conteo fijo de iteraciones asegura que todos los nodos computen pruebas identicas — no se necesita calibracion por nodo ni ajuste dinamico.

La prueba de retardo es deliberadamente liviana. El requisito de bond (Seccion 7.2) es el costo principal de participacion; la VDF sirve como medida anti-grinding que previene que un productor precompute bloques trivialmente sin conocimiento del hash del bloque anterior. Todo sistema de consenso impone un recurso escaso. En DOLI, ese recurso es capital en bond anclado por tiempo secuencial.

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
5. `B.producer` es la seleccion ponderada por bonds correcta para `B.slot` dado el conjunto activo y el filtro de actividad
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

El TPS practico asume un tamano promedio de transaccion de 500-1000 B (tipico 2-in/2-out con witnesses); el techo teorico asume ~250 B (transferencias simples de 1-in/1-out).

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

Para prevenir esto, el registro requiere tanto una prueba de retardo secuencial como un bond de activacion (Seccion 7.2). El bond es el principal disuasivo Sybil; la prueba de retardo agrega una barrera anti-grinding liviana.

```
input  = HASH(prefix || public_key || epoch)
output = HASH^T(input)    where T = T_REGISTER_BASE = 1,000 iterations
```

Un registro es valido si:

1. La prueba de retardo se verifica correctamente con `T_REGISTER_BASE` iteraciones.
2. El epoch es el actual o el anterior.
3. La clave publica no esta ya registrada.
4. El bond de activacion esta incluido.

### 7.1. Dificultad de registro

La dificultad de registro es fija:

```
T_registration = T_REGISTER_BASE = 1,000 iterations (tiempo de computacion despreciable)
```

Esto es deliberadamente liviano. El costo de capital del bond de activacion (10 DOLI) es el principal disuasivo Sybil. La prueba de retardo sirve como medida anti-grinding — vincula el registro a un epoch especifico y una clave publica, previniendo la precomputacion de pruebas de registro. Un atacante con *M* maquinas puede registrar *M* identidades, pero cada una requiere *BOND_UNIT* de capital, haciendo el costo de un ataque Sybil *O(M)* en capital en bond.

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

La produccion de bloques utiliza scheduling ponderado por bonds — cada productor recibe asignaciones de bloques proporcionales a su conteo de bonds. Un productor con 5 bonds recibe 5 slots consecutivos por ciclo de rotacion; un productor con 1 bond recibe 1. Tanto la frecuencia de produccion como la distribucion de recompensas por epoch (Seccion 10.2) escalan linealmente con los bonds.

**Ejemplo (3 productores, 10 bonds totales):**

```
Alice: 1 bond unit  (10 DOLI)   → 1 slot por ciclo  (10% de los bloques)
Bob:   5 bond units (50 DOLI)   → 5 slots por ciclo (50% de los bloques)
Carol: 4 bond units (40 DOLI)   → 4 slots por ciclo (40% de los bloques)
```

La frecuencia de produccion y las recompensas de epoch escalan con los bonds:

- Alice gana 10% de los bloques + 10% de las recompensas del epoch
- Bob gana 50% de los bloques + 50% de las recompensas del epoch
- Carol gana 40% de los bloques + 40% de las recompensas del epoch

**Todos los productores obtienen un porcentaje de ROI identico independientemente del tamano de su stake** — porque tanto la produccion como las recompensas escalan linealmente con los bonds, cada DOLI bondeado genera el mismo retorno.

| Parametro             | Valor                          |
|-----------------------|--------------------------------|
| Unidad de bond        | 10 DOLI                        |
| Stake minimo          | 10 DOLI (1 bond)               |
| Stake maximo          | 30,000 DOLI (3,000 bonds)      |
| Recompensa de bloque (Era 1) | 1 DOLI                  |
| Frecuencia de produccion | Proporcional a bonds (determinista) |
| Distribucion de recompensas | Proporcional a bonds (pool de epoch) |

#### Accesibilidad a escala

En la madurez de la red (500 productores, 18,000 bonds totales):

| Tu stake   | Bonds | Bloques/Semana | Ingreso/Semana | Hardware     |
|-----------|-------|----------------|----------------|--------------|
| 10 DOLI   | 1     | ~3             | ~3 DOLI        | Cualquier CPU|
| 100 DOLI  | 10    | ~34            | ~34 DOLI       | Cualquier CPU|
| 1,000 DOLI| 100   | ~336           | ~336 DOLI      | Cualquier CPU|

Tanto las asignaciones de bloques como las recompensas de epoch escalan linealmente con los bonds — cada DOLI bondeado genera el mismo retorno sin importar el tamano total del stake. Sin equipos de mineria. Sin staking pools. Sin requisitos minimos de hardware. Un VPS de $5/mes es suficiente.

### 7.4. Ciclo de vida del bond

El bond tiene un periodo de compromiso de 4 anos con seguimiento FIFO por bond:

```
T_commitment = 12,614,400 blocks (~4 years)
```

Cada bond rastrea su propio tiempo de creacion. El retiro utiliza orden FIFO (los bonds mas antiguos primero), con penalizacion calculada individualmente por bond segun su edad.

**El retiro utiliza un proceso de dos pasos con un periodo de desvinculacion de 7 dias** (60,480 bloques). Un productor envia una transaccion `RequestWithdrawal`, que inicia la cuenta regresiva de desvinculacion. Despues de 60,480 bloques (~7 dias), el productor envia una transaccion `ClaimWithdrawal` para recibir los fondos. La eliminacion del bond del conjunto activo toma efecto en el siguiente limite de epoch despues de la solicitud.

El periodo de desvinculacion previene ataques de corto alcance donde un productor se retira inmediatamente despues de portarse mal, y asegura que la red retenga la capacidad de slashing durante la ventana de disputa.

El retiro anticipado incurre en una penalizacion escalonada FIFO basada en la edad individual del bond:

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

Para cada slot, una funcion determinista selecciona al productor de bloques. Sea *P* = {*p_1*, ..., *p_n*} el conjunto activo congelado en el limite de epoch, ordenado por clave publica. Cada productor *p_i* recibe *bonds(p_i)* tickets consecutivos en la rotacion. El espacio total de tickets es *T = Σ bonds(p_i)*.

```
ticket(s) = s mod T
producer(s) = p_i donde ticket(s) cae en el rango de tickets de p_i
```

El scheduler es un `DeterministicScheduler`: una funcion pura de `(slot, EpochBondSnapshot)`. No depende de ningun valor que el productor actual pueda influenciar — ni `prev_hash`, ni el ordenamiento de transacciones, ni marcas de tiempo dentro de la ventana de deriva. **Grinding es imposible porque el calendario es una funcion del tiempo y el snapshot de bonds congelado del epoch.**

El conteo de bonds influye en la frecuencia de produccion proporcionalmente — un productor con 5 bonds recibe 5 slots consecutivos por ciclo de rotacion, un productor con 1 bond recibe 1. Tanto la frecuencia de produccion como la distribucion de recompensas por epoch (Seccion 10.2) escalan con los bonds. Esto asegura que cada DOLI bondeado genera un retorno identico sin importar el tamano total del stake del productor — el porcentaje de ROI es uniforme.

### 8.1. Filtro de actividad

El protocolo utiliza un filtro de actividad basado en attestations aplicado en los limites de epoch. En lugar de exclusion mid-epoch (que creaba divergencia entre nodos con diferente estado local), la lista activa de productores se congela en cada limite de epoch basada en una ventana de lookback de 3 epochs.

**Filtrado por attestation.** En el limite de epoch, los productores que atestiguaron en cualquiera de los 3 epochs anteriores se retienen en el conjunto activo. Los productores con cero attestations en los 3 epochs son excluidos de la rotacion del siguiente epoch.

**Piso de seguridad anti-deadlock.** Si el filtro de attestation dejaria menos de 2/3 de los productores activos, el filtro se omite y todos los productores se incluyen — previniendo la muerte de la cadena durante eventos masivos (caida de red, deploy coordinado).

**Promocion por tier.** Cuando el conjunto activo excede el `ACTIVE_PRODUCERS_CAP` (50), los productores se ordenan por tiempo de registro (mas antiguos primero). Los productores por debajo del umbral de attestation para el epoch recien completado son degradados y reemplazados por attestors calificados.

| Evento | Efecto |
|--------|--------|
| Limite de epoch | Lista activa reconstruida desde lookback de attestation de 3 epochs |
| Productor atestigua en cualquiera de 3 epochs | Retenido en conjunto activo |
| Cero attestations en 3 epochs | Excluido del siguiente epoch |
| Filtro deja < 2/3 productores | Bypass: incluir todos (piso de seguridad) |

El filtro de actividad es determinista: cada nodo computa el mismo `EpochState` en la misma altura de limite. El `EpochState` — incluyendo la lista activa, el snapshot de bonds y los acumuladores de attestation — se persiste atomicamente y se incluye en el state root para verificacion cross-nodo.

### 8.2. Comparacion con sistemas existentes

Los pools existen en PoW y PoS porque las recompensas son probabilisticas — la varianza obliga a los pequenos participantes a delegar el control a operadores centralizados. El scheduling determinista ponderado por bonds de DOLI elimina la varianza de produccion por completo — cada productor recibe asignaciones de bloques garantizadas proporcionales a sus bonds. El pool de recompensas por epoch integrado (Seccion 10.5) distribuye recompensas ponderadas por bonds a todos los productores calificados. Los pools externos no pueden ofrecer un mejor trato.

| Sistema      | Seleccion                  | Varianza | Pools | Energia        | Hardware minimo     |
|--------------|----------------------------|----------|-------|----------------|---------------------|
| Bitcoin      | Loteria (hashpower)        | Alta     | Si    | ~150 TWh/ano   | ASIC ($5,000+)      |
| Ethereum PoS | Loteria (stake)            | Media    | Si    | ~2.6 GWh/ano   | 32 ETH ($100K+)     |
| Solana PoH   | Calendario (stake)         | Baja     | Si    | ~4 GWh/ano     | Servidor $10,000+   |
| DOLI PoT     | Ponderado por bonds deterministico | **Cero** | **Integrado** | **Despreciable** | **Cualquier CPU ($5/mes)** |

Solana usa Proof of History como reloj, pero la seleccion de lider sigue siendo ponderada por stake con elementos probabilisticos y requiere hardware de alto rendimiento. DOLI usa la prueba de retardo puramente como latido — la seleccion de lider es una funcion pura de `(slot, ActiveSet(epoch), LivenessFilter)`. No existe ventaja de hardware. No existe ventaja de stake para la produccion — solo para las recompensas.

**Ruta de escalamiento por niveles.** El protocolo define una arquitectura de dos niveles para crecimiento futuro: hasta 500 validadores de Nivel 1 (productores de bloques con participacion completa en el consenso) y hasta 15,000 attestors de Nivel 2 (attestation de actividad sin produccion de bloques). La delegacion permite a participantes de Nivel 3 hacer stake sin operar infraestructura, con recompensas divididas 10% al delegado (operador de nodo Nivel 1/2) y 90% al staker. Este modelo por niveles preserva la accesibilidad del protocolo base mientras escala la participacion en el consenso mas alla del conjunto activo de productores.

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

**Nota sobre la emision a largo plazo.** El halving se implementa como un desplazamiento a la derecha (right-shift) entero sobre la recompensa por bloque expresada en unidades base (`reward >> era`). Como las unidades base son indivisibles, el bit menos significativo se descarta en cada halving. A partir de la era 9 esto introduce una pequena perdida de precision por bloque (unas pocas unidades base), y una vez que `initial_reward >> era` se trunca a cero la recompensa por bloque se vuelve exactamente cero para siempre — la emision termina como un acantilado discreto en lugar de continuar como una cola geometrica. Por lo tanto, el suministro total emitido se topa ligeramente por debajo del valor de suma geometrica pura de 25,228,800 DOLI mostrado arriba (diferencia despreciable a la precision de la tabla), y la emision finaliza unas pocas eras antes de lo que sugeriria una serie geometrica ilimitada. Esta es una propiedad deterministica del protocolo — cada nodo calcula el mismo desplazamiento, por lo que no hay implicacion alguna para el consenso — y se menciona aqui por exactitud de especificacion respecto al suministro a largo plazo.

### 10.2. Distribucion de recompensas por epoch

En el primer bloque de cada nuevo epoch, el productor de bloques emite una unica transaccion de recompensa distribuyendo el pool acumulado a todos los productores calificados, proporcionalmente a su conteo de bonds:

```
epoch_pool = Σ block_reward(h) for h in [epoch_start, epoch_end)
reward(i)  = epoch_pool × bonds(i) / Σ qualifying_bonds
```

Solo los productores que atestiguaron en el 90% o mas de las ventanas de attestation de 1 minuto del epoch califican. Los no calificados no reciben nada; su parte se redistribuye a los productores calificados.

Esto produce un UTXO por productor por epoch en lugar de uno por bloque — eliminando el polvo de recompensas mientras se mantiene la misma emision total.

### 10.3. Attestation de actividad

La produccion de bloques demuestra que un productor estuvo en linea durante sus slots asignados. Pero con programacion determinista, un productor podria anticipar sus slots y estar fuera de linea el resto del tiempo.

Para demostrar presencia **continua**, cada productor firma una attestation de actividad cada minuto usando ambas claves Ed25519 y BLS12-381:

```
attestation = Sign(block_hash || slot)
```

El hash del bloque demuestra que el productor no solo esta vivo sino que activamente sigue y valida la cadena. Las attestations se difunden por gossip a la red. Cada productor de bloques registra:

1. Un **bitfield** en la cabecera del bloque (`presence_root`) — un bit por productor (atestiguado o no), soportando hasta 256 productores (32 bytes)
2. Una **firma BLS agregada** en el cuerpo del bloque — prueba criptografica de que el bitfield es honesto

La firma BLS agregada comprime todas las firmas individuales de attestation en una sola verificacion. Un bit falso — afirmando que un productor atestiguó cuando no lo hizo — causa que la verificacion de la firma agregada falle. El bloque es rechazado.

En el limite de epoch, cada nodo escanea los bitfields registrados en los bloques del epoch y cuenta los minutos de attestation por productor. Cada epoch abarca 60 minutos de attestation (uno por cada 6 slots). Dos umbrales distintos aplican:

1. **Calificacion para recompensas de epoch (90%):** Un productor debe atestiguar en al menos 54 de 60 minutos para calificar para la distribucion de recompensas de ese epoch. Los no calificados no reciben nada; su parte se redistribuye a los productores calificados.
2. **Retencion en el conjunto activo (50%):** Un productor debe mantener una tasa de presencia de al menos 50% para permanecer en el conjunto activo de productores. Los productores que caen por debajo de este umbral son removidos del conjunto y deben re-registrarse para reincorporarse.

Ambos umbrales son deterministas: cada nodo lee la misma cadena, computa los mismos conteos, coincide en las mismas decisiones de calificacion y retencion.

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
- Para reactivarse: nuevo registro requerido

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

El unico vector de ataque es controlar >50% de los productores en el conjunto activo, lo que requiere:

1. *BOND_UNIT* de capital bloqueado por identidad (costo lineal, sujeto a slashing)
2. Prueba de retardo *T_registration* por identidad (anti-grinding, vinculada al epoch)
3. 100% de riesgo de perdida de bond si se detecta doble produccion

### 12.3. La objecion de acumulacion de capital

Una objecion natural: "Si el registro requiere solo 1,000 iteraciones de hash (tiempo despreciable), que impide que un atacante inunde la red con identidades?"

La respuesta es **capital en bond**. Cada identidad requiere *BOND_UNIT* (10 DOLI) bloqueado como bond de activacion. Un atacante que registra *M* identidades debe bloquear *M x BOND_UNIT* de capital — el costo es *O(M)*, identico a Proof of Stake. DOLI no escapa de la economia fundamental de la resistencia Sybil: prevenir la inundacion de identidades requiere un recurso escaso que escale linealmente con el numero de identidades.

Lo que DOLI agrega mas alla de PoS puro:

1. **Capital en riesgo:** *BOND_UNIT* bloqueado por identidad, con slashing del 100% por doble produccion y penalizaciones de vesting por retiro anticipado.
2. **Anti-grinding:** La VDF de registro (1,000 iteraciones) vincula la prueba a un epoch especifico y una clave publica, previniendo la precomputacion de pruebas de registro entre epochs futuros.
3. **Costo operativo continuo:** Cada identidad requiere attestation de actividad continua (90% de uptime por epoch). Mantener *M* identidades a este umbral tiene un costo operativo compuesto — una maquina por identidad, indefinidamente.
4. **Desventaja de antiguedad:** Las nuevas identidades reciben peso 1.0; los productores establecidos acumulan hasta 4.0. Un atacante que parte de cero necesita ~3 anos antes de que su peso de antiguedad iguale al de los participantes honestos establecidos.

Comparemos con PoW: un atacante con *M* ASICs obtiene *Mx* hashpower inmediatamente, sin bloqueo de capital por identidad. En DOLI, cada identidad requiere capital bloqueado que puede ser destruido ante mal comportamiento.

El sistema no reclama inmunidad ante adversarios adinerados — ningun sistema puede. Reclama que (1) la creacion de identidades tiene costo de capital lineal con riesgo de slashing, (2) la VDF anti-grinding previene ataques de precomputacion, y (3) una vez registrado, el costo operativo por identidad del atacante es permanente, no un gasto unico.

### 12.4. Teorema de seguridad

**Teorema.** Sea *n* = total de productores en el conjunto activo. Un atacante que controla *f* < *n/2* productores no puede producir una cadena mas pesada que la red honesta sobre cualquier intervalo de *k* >= 1 epochs.

**Demostracion.** Definimos por epoch *e*:

- *S_h(e)* = conjunto de slots asignados a productores honestos
- *S_a(e)* = conjunto de slots asignados a productores atacantes
- *w(p)* = peso de antiguedad del productor *p* perteneciente a [1.0, 4.0]

El calendario es una funcion pura de `(slot, ActiveSet(epoch), LivenessFilter)` — ningun contenido de bloque lo influencia. Bajo scheduling ponderado por bonds, cada unidad de bond recibe asignaciones de slots iguales: *|S_h(e)| + |S_a(e)| = total slots*, distribuidos proporcionalmente a los bonds.

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

Es posible verificar pagos sin ejecutar un nodo completo. Un usuario solo necesita mantener una copia de las cabeceras de bloque de la cadena mas larga y obtener la rama Merkle que vincula la transaccion al bloque en el que fue marcada temporalmente.

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

**Fase 4 — Participacion abierta.** En el bloque 26,979 (~3 dias despues del genesis), los productores fundadores financiaron un faucet publico con sus propias recompensas ganadas — 250 DOLI cada uno, 1,500 DOLI en total. El faucet distribuye 10.01 DOLI (1 unidad de bond + comisiones de transaccion) a cualquier nuevo participante que lo solicite. La barrera de entrada es cero capital — solo un VPS de $5/mes y la voluntad de operar un nodo. Todas las transacciones del faucet estan en cadena y son verificables. Hoy, los nuevos participantes pueden reclamar su airdrop de DOLI para comenzar a minar uniendose al [Discord de DOLI](https://discord.gg/hB3mjQmv) y mencionando a @dolifather o @isudoajl.

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

## 19. Disenado para agentes

Una nueva clase de consumidor esta leyendo nuestras APIs. Los agentes de IA ahora envian transacciones, consultan el estado y reaccionan a errores sin un humano en el ciclo. Son reconocedores de patrones, no expertos en protocolos. No pueden leer prosa; parsean campos. No pueden adivinar; necesitan estado explicito.

DOLI fue disenado durante esta transicion. La claridad de los errores, el timing determinista y el estado legible por maquina fueron tratados como propiedades del protocolo, no como anadidos posteriores. Adaptar estas propiedades a una cadena madura es extremadamente dificil — cada cambio arriesga romper contratos e indexers existentes. Construirlas desde el genesis no cuesta nada.

### 19.1. La transicion agentica

La mayoria de las cadenas fueron disenadas para desarrolladores humanos leyendo documentacion. Sus errores son strings cripticos. Su estado es implicito. Su scheduling es opaco. Un agente que falla con `Error: revert` y sin mas informacion no puede autocorregirse. Un agente que recibe `INSUFFICIENT_FUNDS` con `inputs=500, outputs=1000` calcula el deficit y reintenta.

La diferencia se acumula. Una cadena que habla con agentes mediante campos estructurados atrae mas tooling autonomo, lo que produce mas volumen de transacciones, lo que fortalece la red. Una cadena que requiere expresiones regulares sobre prosa en ingles no.

### 19.2. Errores estructurados con codigos estables

Cada error retornado por un nodo DOLI lleva:

- Un `code` numerico estable (compatible con JSON-RPC).
- Un `message` legible por humanos.
- Un campo `data` estructurado con `error_code`, `stage` y los valores especificos involucrados.

Donde la mayoria de las cadenas retorna:

```json
{"code": -32002, "message": "validation failed: insufficient funds: inputs=500, outputs=1000"}
```

— forzando al agente a aplicar regex sobre el string del mensaje — DOLI retorna:

```json
{
  "code": -32002,
  "message": "validation failed: insufficient funds: inputs=500, outputs=1000",
  "data": {
    "error_code": "INSUFFICIENT_FUNDS",
    "stage": "mempool_validation",
    "inputs": 500,
    "outputs": 1000
  }
}
```

El agente lee `error_code`, ve `INSUFFICIENT_FUNDS`, compara `inputs` con `outputs`, calcula el deficit, selecciona UTXOs adicionales y reintenta. Sin parseo de strings. Sin adivinanzas.

Los codigos RPC especificos del dominio son estables y estan documentados:

| Codigo | Significado | Accion del agente |
|--------|-------------|-------------------|
| `-32000` | Bloque no encontrado | Consultar con otro identificador |
| `-32001` | Transaccion no encontrada | Esperar confirmacion |
| `-32002` | Transaccion invalida | Leer `data.error_code` para detalles |
| `-32003` | Ya en mempool | No reintentar — la tx esta pendiente |
| `-32004` | Mempool lleno | Hacer back off y reintentar mas tarde |
| `-32005` | UTXO no encontrado | Refrescar el conjunto UTXO |
| `-32006` | Productor no encontrado | Verificar formato de pubkey |
| `-32007` | Pool no encontrado | Verificar que el pool existe |
| `-32008` | No autorizado | Proveer token de admin |

Los errores de validacion exponen codigos como `INSUFFICIENT_FUNDS`, `OUTPUT_NOT_FOUND`, `OUTPUT_LOCKED`, `INSUFFICIENT_FEE`, `DOUBLE_SPEND`, `MISSING_PUBLIC_KEY` — cada uno con los outpoints, alturas, montos o indices de input especificos que un agente necesita para autocorregirse.

### 19.3. Modelo UTXO: estado explicito

En una cadena basada en cuentas, el balance que ves depende de cuyas transacciones pendientes han sido incluidas desde la ultima vez que verificaste. Un agente debe simular la transicion de estado completa para predecir resultados. No hay una moneda individual a la cual apuntar.

El modelo UTXO de DOLI hace que cada moneda sea un objeto direccionable individualmente:

- `getUtxos` retorna el conjunto exacto de salidas no gastadas que controla una direccion.
- Cada UTXO tiene una identidad estable: `(tx_hash, output_index)`.
- Una transaccion consume un UTXO especifico o no — sin estado mutable compartido.
- Cuando la validacion falla, la respuesta nombra el outpoint especifico que causo la falla.

No hay estimacion de gas. No hay race de nonce. No hay reordenamiento MEV que cambie el balance efectivo del agente entre la presentacion y la inclusion. El estado que un agente lee es el estado contra el cual transacciona.

### 19.4. Transacciones tipadas

DOLI expone 27 tipos de transaccion explicitos — `Transfer`, `Registration`, `AddBond`, `DelegateBond`, `RequestWithdrawal`, `ClaimWithdrawal`, `Swap`, `AddLiquidity`, `NftMint`, `BridgeLock`, entre otros — en lugar de un campo `data` opaco que lleva calldata codificado en ABI.

Cada tipo tiene sus propias reglas de validacion, sus propias respuestas de error estructuradas y un modelo de costo fijo. Un agente no necesita un ABI de contrato para interpretar una transaccion; el tipo mismo describe lo que la transaccion hace.

### 19.5. Scheduling determinista

La produccion de bloques es una funcion pura de `(slot, bond_snapshot, liveness_filter)`. Dos metodos RPC permiten a los agentes predecir el timing exactamente:

- `getSlotSchedule(start, end)` retorna el productor asignado a cada slot en un rango.
- `getProducerSchedule(pubkey, lookahead)` retorna los proximos slots a los que un productor especifico sera asignado.

No hay subasta de lider, no hay race MEV y no hay seleccion probabilistica. Un agente que quiere saber cuando aterrizara su transaccion — o el bloque de quien escuchar — lo lee de la cadena.

### 19.6. Introspeccion de estado

El nodo expone 47 metodos JSON-RPC que cubren estado de la cadena, mempool, peers, schedules, productores, bonds, pools DeFi, prestamos, NFTs y storage. Varios estan orientados especificamente al diagnostico impulsado por agentes:

- `getStateRootDebug` retorna los componentes que forman el state root actual, permitiendo a un agente localizar una divergencia en un subsistema especifico.
- `getUtxoDiff` superficia la diferencia exacta del conjunto UTXO contra un nodo peer.
- `getStateSnapshot` expone los `chain_state`, `utxo_set`, `producer_set`, `state_root`, `epoch_bond_snapshot` y `epoch_accumulators` serializados para comparacion determinista entre nodos.
- `verifyChainIntegrity` recorre el commitment rodante de la cadena y reporta la primera inconsistencia.

Un agente que detecta una anomalia puede localizarla sin ayuda de un operador.

### 19.7. Flujo de eventos en vivo

Un endpoint WebSocket en `/ws` emite eventos etiquetados a medida que ocurren:

```json
{"type": "NewBlock", "hash": "...", "height": 19372, "slot": 193720, "timestamp": ..., "producer": "...", "tx_count": 4}
{"type": "NewTx",    "hash": "...", "tx_type": "Transfer", "size": 312, "fee": 100}
```

Sin polling. Sin race entre confirmacion y actualizacion de balance. Los agentes que necesitan reaccionar a eventos de la cadena lo hacen en decenas de milisegundos desde la propagacion del bloque.

### 19.8. Superficie de observabilidad

Cada nodo expone metricas Prometheus en `--metrics-port`: `doli_chain_height`, `doli_current_slot`, `doli_blocks_processed_total`, `doli_blocks_by_status_total`, `doli_block_processing_seconds` (histograma), `doli_transactions_by_type_total`, `doli_mempool_size`, `doli_peers_connected` y docenas mas. Los logs de la aplicacion se emiten a traves del framework `tracing` como pares clave-valor estructurados — no prosa libre.

Un agente de monitoreo autonomo puede responder *"esta este nodo saludable?"*, *"esta este nodo en la cadena canonica?"* y *"esta este nodo retrasandose?"* leyendo numeros, no parseando logs.

### 19.9. Diagnostico reproducible

Una blockchain es tan depurable como los snapshots y rollbacks que soporta. DOLI provee cuatro primitivos de reproducibilidad que un agente puede usar:

- **Rollback basado en undo.** Cada bloque aplicado escribe un registro undo estructurado. Revertir un bloque no requiere replay desde el genesis.
- **Checkpoints en caliente de RocksDB.** `createCheckpoint` produce un snapshot consistente a la altura actual via hard-links, sin detener el nodo.
- **Checkpoint embebido en el binario.** Cada binario de release contiene un `CHECKPOINT_HEIGHT`, `CHECKPOINT_HASH` y `CHECKPOINT_STATE_ROOT`. Los nodos nuevos verifican que estan en la cadena canonica antes de procesar transacciones.
- **Recuperacion determinista de forks.** Cuando un nodo detecta que esta en una cadena minoritaria, una maquina de estados interna planifica y ejecuta el reorg sin intervencion del operador. Cada paso queda registrado con los datos que un agente necesita para reconstruir lo que ocurrio.

### 19.10. CLI scriptable

El CLI `doli` esta disenado para ser conducido de forma no interactiva. Cada comando que requiere confirmacion acepta `--yes`. Los flujos de NFT, bridge y gobernanza escriben su estado intermedio como archivos JSON que pueden ser inspeccionados, firmados y combinados por otras herramientas. El CLI es la misma superficie usada por tests automatizados, productores externos y arneses de agentes — no hay una "API para maquinas" separada.

### 19.11. Superficie de gobernanza

Las actualizaciones de protocolo ocurren a traves de votos firmados broadcasteados sobre la misma superficie RPC usada para transacciones:

- Un `Vote {Approve | Veto}` lleva `version`, `vote`, `producer_id`, `timestamp` y una firma sobre el mensaje canonico de votacion.
- `submitVote` acepta estos votos de cualquier cliente, incluyendo agentes que actuan en nombre de un productor.
- Un umbral de veto ponderado por antiguedad (40% del stake ponderado) bloquea actualizaciones que el conjunto de productores rechaza.
- Un `HardForkSchedule` lista cada cambio de consenso con su altura de activacion — los agentes pueden predecir exactamente cuando cambiara el comportamiento.
- El `UpdateWatchdog` del nodo hace rollback automaticamente si un release nuevo crashea mas del umbral configurado dentro de su ventana.

Un agente operando un productor puede monitorear `getUpdateStatus`, evaluar un release pendiente y votar — todo sin intervencion humana.

### 19.12. Comparacion

| Dimension | Ethereum | DOLI |
|-----------|----------|------|
| Formato de error | Bytes codificados ABI que requieren ABI de contrato para decodificar | JSON estructurado con codigos estables e informacion de stage |
| Codigos de error | Ninguno estandar — cada contrato inventa el suyo | Codigos estables por dominio en toda la superficie RPC |
| Informacion de stage | Ninguna — el revert podria estar en cualquier llamada anidada | Explicita: `deserialization`, `mempool`, `mempool_validation` |
| Modelo de estado | Basado en cuentas (implicito, requiere simulacion) | UTXO (explicito, direccionable individualmente) |
| Scheduling | Probabilistico (MEV, subastas de gas) | Determinista (slot schedule consultable) |
| Formato de transaccion | Bytes de calldata arbitrarios | 27 transacciones tipadas con validacion fija |
| Flujo de eventos en vivo | Filtros de logs especificos del proveedor | Eventos WebSocket etiquetados en cada nodo |
| Adaptable retroactivamente? | Extremadamente dificil — miles de contratos desplegados | Construido desde el genesis |

### 19.13. Por que esto se acumula

El buen diseno de errores es una de las pocas propiedades de protocolo que se vuelve mas dificil de agregar con el tiempo. Cada contrato, indexer y cliente existente que depende del formato actual de error restringe lo que una cadena madura puede cambiar. Ethereum ha estado trabajando en razones de revert estructuradas durante anos; el costo de romper contratos desplegados domina el espacio de diseno.

DOLI esta en su fase de crecimiento. La taxonomia de errores estructurada, las transacciones tipadas, el scheduling determinista, los RPCs de introspeccion y el flujo de eventos en vivo son comportamiento base — no feature flags apilados encima. A medida que el tooling autonomo madura, la cadena que ya habla el lenguaje que los agentes entienden tiene una ventaja estructural que no depende del marketing.

Esto no es una funcionalidad. Es una posicion.

---

## 20. Red en produccion

DOLI no es una propuesta. La red descrita en este documento esta operativa.

A mayo de 2026, la red principal esta en su **fase de crecimiento** — operativa con 38 productores registrados en 11 servidores geograficamente distribuidos. El codigo fuente es abierto, el estado de la cadena es verificable publicamente, y productores externos operan de forma independiente. La cadena paso por multiples resets de genesis durante el arranque; las metricas a continuacion reflejan la cadena actual (genesis: 2026-04-22).

| Metrica | Valor |
|---------|-------|
| Tiempo de bloque | 10 segundos |
| Computacion de prueba de retardo | Despreciable (~1,000 iteraciones) |
| Propagacion de bloques | < 500ms |
| Hardware de nodos | VPS estandar, cualquier CPU |
| Bond minimo | 10 DOLI |
| Filtro de actividad | Exclusion/re-inclusion dinamica |
| Periodo de desvinculacion | 7 dias (60,480 bloques) |

El conteo actual de productores refleja el crecimiento temprano. Las propiedades de seguridad del protocolo se fortalecen a medida que productores independientes se unen — cada operador adicional aumenta el costo de un ataque >50% del conteo de productores y reduce la dependencia del conjunto fundador.

**Evolucion post-lanzamiento del protocolo.** Los siguientes mecanismos fueron agregados despues del lanzamiento mediante hard forks activados forward-only (sin resets de genesis):

- **EpochState en state root** (h=2750): El state root ahora incluye el estado completo del scheduler de epoch (snapshot de bonds, lista de productores, acumuladores de attestation). La divergencia entre nodos es detectable en 10 segundos en lugar de esperar al limite de epoch.
- **Chain commitment incremental**: Cada bloque actualiza un commitment BLAKE3 rodante — `commitment[h] = BLAKE3(commitment[h-1] || block_hash[h])`. La verificacion de integridad es O(1) por bloque en lugar de un scan O(n) de toda la cadena.
- **Recuperacion de bloques en tres capas**: (1) Gossip entrega bloques via push, (2) ORPHAN_CHASE solicita bloques faltantes directamente al peer que envio un bloque dependiente — causal, determinista, sin heuristicas, (3) Silence pull solicita bloques proactivamente cuando gossip esta en silencio por 30 segundos.
- **Entrega directa de attestations**: Las attestations se envian punto-a-punto al productor del siguiente slot via el protocolo sync, evitando dependencias de timing del mesh gossip.
- **Fingerprints de estado**: Cada bloque registra hashes de 7 componentes de estado para diagnostico instantaneo de divergencia entre nodos.

```
Genesis:    2026-04-22 (cadena actual)
Consensus:  Proof of Time (delay proof heartbeat + deterministic bond-weighted scheduling)
Status:     Live
Source:     https://github.com/doli-network/doli
Explorer:   https://doli.network
```

### 20.1. Estado actual y limitaciones conocidas

Creemos que la honestidad sobre lo que funciona y lo que no es mas valiosa que una narrativa pulida. Aqui esta donde se encuentra DOLI hoy.

**Funcionando en produccion:**

- Produccion de bloques, pruebas de retardo y scheduling determinista
- Distribucion de recompensas por epoch con agregacion de attestation BLS
- Ciclo de vida de bonds: apilamiento, retiro en dos pasos con penalizaciones de vesting FIFO
- Atomic swaps entre cadenas (HTLC) con BSC (BEP-20 USDT)
- Filtro de actividad con lookback de 3 epochs y piso de seguridad
- Seleccion de fork basada en peso con antiguedad
- Registro de productores con bonds de activacion
- Explorador de bloques y panel de monitoreo de red
- Marketplace para swaps entre cadenas
- Faucet para nuevos participantes (via Discord)
- Taxonomia de errores JSON-RPC estructurada con codigos estables (Seccion 19)
- Flujo de eventos en vivo via WebSocket en `/ws`
- Checkpoints en caliente de RocksDB via `createCheckpoint`
- Metricas Prometheus y logs `tracing` estructurados

**Descrito en este documento pero aun no en produccion:**

- NFTs nativos (tipo de salida UniqueAsset) — implementado en el nodo, aun no utilizado en produccion
- Tokens emitidos por usuarios (FungibleAsset) — implementado, aun no utilizado en produccion
- Pools AMM nativos (CreatePool, Swap, AddLiquidity) — implementado, aun no activado
- Primitivos de prestamo (CreateLoan, RepayLoan, LiquidateLoan) — implementado, aun no activado
- Liquidacion de rollups ZK (ZKSettle) — disenado, aun no implementado
- Escalamiento Tier 2/3 (attestors y delegacion) — disenado, aun no implementado
- Escrow via condiciones Multisig/Threshold — el lenguaje de condiciones existe, el patron escrow aun no ha sido ejercido
- Swaps entre cadenas con Bitcoin, Monero, Litecoin, Cardano — el protocolo los soporta, el tooling de contraparte aun no esta construido

**Limitaciones conocidas:**

- **Conjunto pequeno de productores.** 38 productores son suficientes para operar pero estan por debajo del umbral donde las garantias de seguridad se vuelven robustas contra atacantes bien financiados. Necesitamos mas operadores independientes.
- **Concentracion geografica.** La mayoria de los nodos operan en un numero reducido de proveedores de hosting. La diversidad geografica y de infraestructura es una prioridad.
- **Sin auditoria formal.** El codigo no ha sido auditado por una firma de seguridad externa. El codigo fuente esta abierto para que cualquiera lo revise.
- **Datos de una sola era.** La red esta en la Era 1. El comportamiento de halving, las transiciones de era y la economia a largo plazo no han sido probados en mainnet.
- **Marketplace en etapa temprana.** El marketplace de swaps entre cadenas funciona pero tiene limitaciones de UX conocidas. El CLI sigue siendo la ruta mas confiable para swaps.

No listamos esto como advertencias sino como invitaciones. Cada limitacion es un problema que agradeceriamos ayuda para resolver.

---

## 21. Preguntas frecuentes

**"En que se diferencia de la Proof of History de Solana?"**

Solana usa hash iterado SHA-256 como reloj — un registro verificable del paso del tiempo. Pero la seleccion de lider de Solana es ponderada por stake y probabilistica, y ejecutar un validador requiere hardware de gama alta (256 GB RAM, almacenamiento NVMe, conectividad de alto ancho de banda). DOLI usa la prueba de retardo puramente como latido; la seleccion de lider es una funcion determinista de `(slot, bond_snapshot)`. Cualquier CPU puede participar. La diferencia filosofica: Solana optimiza para rendimiento a costa de accesibilidad. DOLI optimiza para accesibilidad a costa de rendimiento.

**"En que se diferencia de Ethereum?"**

Ethereum usa Proof of Stake — los validadores bloquean 32 ETH (~$100K+) y son seleccionados probabilisticamente para proponer bloques. La ejecucion ocurre en la EVM, una maquina virtual Turing-completa que ejecuta contratos inteligentes arbitrarios con medicion de gas. DOLI usa Proof of Time con un bond de 10 DOLI (~$1 equivalente al lanzamiento) y scheduling determinista ponderado por bonds. No hay maquina virtual — las salidas portan condiciones de gasto declarativas compiladas en el binario del nodo (Seccion 3). Las diferencias practicas: Ethereum requiere capital significativo e infraestructura especializada para validar; DOLI corre en un VPS de $5/mes. Los contratos inteligentes de Ethereum permiten computacion arbitraria pero introducen superficie de ataque ilimitada (reentrancia, MEV, manipulacion de gas); las condiciones declarativas de DOLI tienen costo de verificacion fijo y sin estado mutable compartido. Ethereum tiene un ecosistema maduro con miles de aplicaciones; DOLI esta en su fase de crecimiento con un conjunto de funcionalidades enfocado. Resuelven problemas diferentes a escalas diferentes.

**"Que impide que un atacante rico compre el 51% de los productores?"**

Capital. Cada identidad de productor requiere 10 DOLI bloqueados como bond (Seccion 12.3). Controlar el 51% de los productores significa bloquear el 51% del capital en bonds — y arriesgar una perdida del 100% si se detecta doble produccion. Ademas, las nuevas identidades comienzan con peso de antiguedad 1.0 mientras los productores establecidos acumulan hasta 4.0, lo que significa que un atacante necesita ~3 anos antes de que el peso de su cadena iguale al de los participantes honestos establecidos. Ningun sistema de consenso es inmune a un adversario suficientemente rico, pero DOLI hace que el ataque sea costoso, lento y detectable.

**"Por que deberia confiar en una red con solo 38 productores?"**

No deberias — no ciegamente. Deberias verificar. El codigo fuente es abierto. El estado de la cadena es consultable publicamente. Cada bloque, cada transaccion, cada attestation esta en cadena y es auditable. 38 productores es un numero de etapa temprana, y somos transparentes al respecto (Seccion 19.1). Las propiedades de seguridad se fortalecen con cada productor independiente que se une. Si esto te preocupa, la respuesta mas productiva es ejecutar un nodo y convertirte en el productor numero 39.

**"Hay preminado o asignacion para insiders?"**

No. Cero. El bloque genesis contiene una unica transaccion coinbase de 1 DOLI (Seccion 16.1). Los productores fundadores financiaron sus propios bonds con recompensas de bloques ganadas durante el primer epoch — pagaron su participacion con trabajo, no con privilegio. Cada moneda en circulacion provino del calendario de emision estandar. Todas las transacciones estan en cadena y son verificables.

**"Ha sido auditado?"**

No por una firma externa. El codigo es open source y ha sido revisado internamente y por contribuidores de la comunidad, pero no se ha completado una auditoria de seguridad formal. Lo consideramos una prioridad y agradecemos ofertas de auditoria de firmas calificadas o investigadores independientes.

**"Que pasa si los fundadores desaparecen?"**

El protocolo continua. La produccion de bloques es determinista — no requiere intervencion humana una vez que los productores estan funcionando. Las recompensas de epoch se distribuyen automaticamente. El filtro de actividad remueve productores inactivos y los reincluye cuando regresan. Los fundadores no tienen privilegios especiales a nivel de protocolo. Si todos los nodos fundadores se desconectaran, los productores restantes continuarian la cadena. El codigo es open source; cualquiera puede compilarlo y ejecutarlo.

**"Como puedo participar?"**

Ejecuta un nodo en cualquier VPS ($5/mes es suficiente). Reclama tus 10 DOLI iniciales del faucet en [Discord](https://discord.gg/hB3mjQmv). Registrate como productor. Comienza a ganar recompensas de bloques. El proceso completo toma menos de una hora. Consulta la [guia de instalacion](https://doli.network/guide.html) para instrucciones paso a paso.

**"Es DOLI adecuado para agentes de IA y tooling autonomo?"**

Si — por diseno (Seccion 19). Cada error retornado por el nodo lleva un codigo numerico estable, un campo `stage` y datos estructurados. El estado es explicito a traves del modelo UTXO: cada moneda es un objeto direccionable individualmente. La produccion de bloques es determinista — `getSlotSchedule` le dice a un agente exactamente que productor maneja cada slot proximo. Un endpoint WebSocket emite eventos etiquetados a medida que llegan bloques y transacciones. El CLI es totalmente no interactivo. Las actualizaciones de protocolo pasan por votos firmados sobre la misma superficie RPC. Construir estas propiedades desde el genesis no cuesta nada; adaptarlas a una cadena madura es extremadamente dificil, razon por la cual Ethereum ha estado trabajando en razones de revert estructuradas durante anos.

**"Que pasa si encuentro un bug o no estoy de acuerdo con una decision de diseno?"**

Dinoslo. Abre un issue en [GitHub](https://github.com/doli-network/doli), inicia una discusion en [Discord](https://discord.gg/hB3mjQmv), o envia un correo directamente a los mantenedores. Construimos esto en publico porque creemos que los buenos sistemas emergen de la retroalimentacion honesta, no del desarrollo cerrado. Cada critica que lleva a una mejora hace la red mas fuerte para todos.

---

## 22. Contribuir y retroalimentacion

DOLI esta en su fase de crecimiento. No afirmamos que todo funciona perfectamente — afirmamos que todo es verificable. El codigo fuente, el estado de la cadena, el calendario de emision, el conjunto de productores — todo es publico y auditable.

Estamos buscando activamente:

- **Productores independientes** — cada nuevo operador fortalece la seguridad y descentralizacion de la red
- **Revisores de seguridad** — el codigo es abierto y no auditado; agradecemos la revision adversarial
- **Retroalimentacion del protocolo** — si ves una falla en nuestro razonamiento, un mejor enfoque o una suposicion no declarada, queremos saberlo
- **Constructores de aplicaciones** — los tipos de salida nativos (Seccion 3) estan disenados para composicion; nos interesa lo que la gente construya con ellos

Este no es un producto terminado. Es un sistema en funcionamiento que mejora a traves de retroalimentacion honesta y colaboracion abierta. La peor respuesta ante una falla es el silencio.

- GitHub: [github.com/doli-network/doli](https://github.com/doli-network/doli)
- Discord: [discord.gg/hB3mjQmv](https://discord.gg/hB3mjQmv)
- Email: ivan@doli.network / antonio@doli.network

---

## 23. Alcance

DOLI optimiza para mover valor con finalidad determinista, temporalidad predecible y condiciones de gasto extensibles. La capa base es intencionalmente minima — pero extensible por diseno.

Esta restriccion es una caracteristica. Un sistema que hace una cosa bien es mas seguro, mas auditable y mas resistente a la captura de gobernanza que un sistema que intenta ser un computador universal. Bitcoin demostro que un protocolo enfocado puede sostener una red de un billon de dolares. La complejidad no es un prerequisito para el valor.

La diferencia: el formato de salida de Bitcoin fue fijado en 2009. El formato de salida de DOLI fue disenado en 2026 con diecisiete anos de retrospectiva. El campo `extra_data` existe desde el genesis — sin SegWit, sin Taproot, sin hacks de compatibilidad hacia atras requeridos.

---

## 24. Conclusion

Hemos propuesto un sistema para transacciones electronicas que no requiere confianza en instituciones, ni un gasto masivo de energia, ni acumulacion de capital para participar en el consenso.

Comenzamos con el marco habitual de monedas hechas de firmas digitales, que proporciona un fuerte control de propiedad. Esto es incompleto sin una forma de prevenir el doble gasto. Para resolver esto, propusimos una red peer-to-peer que usa pruebas de retardo secuencial para anclar el consenso al tiempo.

**Los nodos votan con su tiempo.** La red no puede acelerarse con riqueza ni paralelizarse con hardware. Una hora de computacion secuencial es una hora, ya sea realizada por un individuo o un estado-nacion.

**Las recompensas son deterministas, no probabilisticas.** Cada productor recibe asignaciones de bloques garantizadas proporcionales a sus bonds mediante scheduling determinista. El protocolo actua como un pool integrado, distribuyendo recompensas de epoch ponderadas por bonds en cadena a todos los productores que demuestran presencia continua mediante attestations de actividad on-chain. Los pools externos son innecesarios. Cada DOLI bondeado genera el mismo porcentaje de retorno sin importar el tamano total del stake.

La red es robusta en su simplicidad. Los nodos trabajan con poca coordinacion. No necesitan ser identificados, ya que los mensajes no se enrutan a ningun lugar particular y solo necesitan ser entregados con el mejor esfuerzo posible. Los nodos pueden irse y reincorporarse a la red a voluntad, aceptando la cadena mas pesada como prueba de lo que ocurrio mientras estuvieron ausentes.

**Las reglas se fijan en el genesis. La emision es predecible.**

Cualquier regla e incentivo necesario puede aplicarse con este mecanismo de consenso.

---

**DOLI v6.21.18**

*"El tiempo es la unica moneda justa."*

**I. Lozada** · ivan@doli.network | **A. Lozada** · antonio@doli.network

*Ultima actualizacion: mayo 2026*

---
## Referencias

1. Nakamoto, S. (2008). *Bitcoin: A Peer-to-Peer Electronic Cash System.*

2. Boneh, D., Bonneau, J., Bunz, B., & Fisch, B. (2018). *Verifiable Delay Functions.* In Advances in Cryptology – CRYPTO 2018.

3. Wesolowski, B. (2019). *Efficient Verifiable Delay Functions.* In Advances in Cryptology – EUROCRYPT 2019. (Citado por contraste — DOLI usa cadenas de hash iteradas, no VDFs algebraicas. Ver Seccion 5.1.)

4. Yakovenko, A. (2018). *Solana: A new architecture for a high performance blockchain.* Utiliza hash iterado SHA-256 para Proof of History bajo la misma suposicion de dureza secuencial.
