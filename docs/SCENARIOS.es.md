# Situaciones de juego («escenas») por deporte

reco-minimap puede reconocer las **formaciones a balón parado** – reanudaciones y jugadas a balón parado en las que los jugadores ocupan lugares característicos y apenas se mueven – y, en baloncesto, los **intentos de tiro**. Esta página indica, para cada deporte, qué situaciones son fáciles de reconocer, cómo se reconocen y de qué dependen.

Se activa con `--scenes` (línea de comandos) o con la casilla *Detectar situaciones de juego* en los pasos 4 / 5 de la interfaz.

**Leyenda de estados**

| | |
|---|---|
| ✅ | **Implementado.** Reglas sobre las posiciones seguidas, probado con escenas construidas. |
| 🟡 | **Implementado, depende del balón.** Solo funciona si el balón se encuentra en el momento justo. |
| 🔧 | **Factible, aún no implementado.** El patrón es sencillo; falta escribirlo y ajustarlo. |
| ❌ | **No detectable a partir de posiciones.** Necesita marcador, señales del árbitro o conocimiento del reglamento. |

> **Estado real:** todas las escenas ✅ y 🟡 se han verificado **solo con formaciones construidas** (casos positivos y los parecidos típicos que *no* deben dispararse). No se han ajustado con partidos reales. Cuenta con ajustar umbrales y con falsos positivos en las más débiles (señaladas abajo).

## Lista de prioridades

| Deporte | Situación | Estado |
|---|---|---|
| Baloncesto | Saque de banda | ✅ |
| Baloncesto | Tiro libre | ✅ |
| Baloncesto | Cambio | ✅ (dorsales, o salida y entrada por la línea de banda) |
| Baloncesto | Salto inicial | ✅ |
| Baloncesto | Tiros de 2 y 3 puntos | 🟡 (hay que ver el balón salir de las manos del tirador; no si entró) |
| Fútbol | Saque inicial | ✅ |
| Fútbol | Cambio | ✅ |
| Fútbol | Penalti | ✅ |
| Fútbol | Córner | ✅ |
| Balonmano | Cambio | ✅ |
| Balonmano | Saque de banda | ✅ |
| Balonmano | Lanzamiento de 7 m | ✅ |

---

## Cómo funciona

1. En cada imagen procesada, el detector observa a los jugadores seguidos (posiciones en metros, velocidades).
2. Una **formación** es un patrón como «un jugador parado en la línea de tiros libres, tres o más en la zona, casi todos quietos».
3. El patrón debe **mantenerse un momento** (1–2 s según la escena) antes de emitir el evento, y se **emite una sola vez** por formación. Solo puede volver a emitirse cuando la formación se ha deshecho y han pasado 6 s.
4. El evento indica qué, dónde, qué equipo lo ejecuta y una confianza.

**Qué necesita para funcionar**

* Una calibración con un error de unos **0,5 m o menos**. Las tolerancias son de 1–2 m.
* La mayoría de los jugadores seguidos (alrededor del 60 % o más). Una formación no se reconoce si falta la mitad de los jugadores.
* Para las escenas que dependen del balón, un balón seguido en el momento de soltarlo.

**Fallos típicos**

* *No detectada:* jugadores sin detectar (lado lejano, imagen pequeña), mala calibración, una formación que dura menos que el tiempo de mantenimiento.
* *Falsos positivos:* un jugador parado en la línea de banda durante el juego se parece a quien va a sacar – por eso los saques de banda tienen una confianza menor (0,5) salvo que el balón esté en sus manos (0,8). Un portero parado no es una escena por sí solo, y por eso los saques de puerta no están implementados.

## Salida

Las escenas aparecen en varios sitios:

* **Registro:** `SITUACIÓN tiro libre izquierda (local) en 12:34 (confianza 0.85)` y un resumen al final del proceso. El texto sigue el idioma elegido.
* **JSON** (`--json`): en la lista `events` de la imagen en la que se emitió la escena:

```json
{"type": "scene", "kind": "free_throw", "team": "home", "side": "l",
 "x": 5.8, "y": 7.5, "confidence": 0.85, "borderline": false, "t": 754.2}
```

`kind` es `tip_off`, `kickoff`, `free_throw`, `throw_in`, `corner`, `penalty`, `seven_meter`, `shot2` o `shot3`. `side` es `l` o `r` (el extremo del campo al que pertenece la escena) o `null`. `team` es el equipo que ejecuta la reanudación o el tiro. `borderline` es verdadero para los tiros soltados a menos del error de posición de la línea de 3 puntos.

* **Superposición en directo:** un rótulo durante unos segundos, en el idioma de la interfaz.
* **`reco-minimap stats`:** lista cada escena con hora y confianza.

---

## Baloncesto

| Situación | Estado | Cómo se reconoce | Notas |
|---|---|---|---|
| **Salto inicial** | ✅ | Dos rivales dentro del círculo central (≤ 1,5 m), juntos y parados; nadie más a menos de 2,4 m del centro; los demás (≥ 6 seguidos) más bien quietos. Mantenimiento 1 s. | Un tercer jugador en el círculo (una pelea por el balón) lo anula. |
| **Tiro libre** | ✅ | Un jugador parado en la línea de tiros libres (≤ 1,1 m), ≥ 3 jugadores en la zona, como mucho uno de cada cinco jugadores moviéndose rápido. Mantenimiento 2 s. | Indica el lado (`l`/`r`) y el equipo del tirador. |
| **Saque de banda** | ✅ | Un jugador parado en la línea de banda o de fondo (≤ 0,8 m del borde, dentro o justo fuera), nadie más a menos de 1,2 m, ≥ 3 jugadores a menos de 9 m. Mantenimiento 1,2 s. | Confianza 0,8 si el balón está en sus manos o a su lado, si no 0,5. Un jugador parado en la línea durante el juego puede dispararlo. |
| **Cambio** | ✅ | El dorsal confirmado de un puesto cambia, o un jugador seguido por última vez junto a una línea de banda desaparece al menos 3 s y alguien aparece junto a una línea de banda («posible»). Ver el [README](../README.md#jersey-numbers-and-substitutions). | Lo más fiable es con dorsales y plantilla. |
| **Tiro de 2 o 3 puntos** | 🟡 | El balón sale de las manos del poseedor (estado *retenido* → *en vuelo*) y va hacia una canasta (termina ≥ 2 m más cerca, o a menos de 2,5 m). Se clasifica por la posición del tirador al soltar: ≥ 6,8 m de la canasta, o en la franja de la esquina, es de 3. Se excluyen los tiros libres. | `borderline: true` si el lanzamiento está a menos de 0,8 m del arco – el error de posición es de ese orden. Se pierde si el balón no está seguido al soltarlo; con un modelo de balón débil es habitual. |
| Anotado o fallado | ❌ | No se ve a partir de las posiciones. | Necesita el marcador. A una canasta suele seguirle, en segundos, un saque de fondo del *otro* equipo – un indicio, no una prueba. |
| Tiempo muerto | 🔧 | Todos parados y agrupados junto al banquillo durante > 20 s. | |
| Contraataque | 🔧 | Muchos jugadores de ambos equipos cruzando el medio campo en el mismo sentido a gran velocidad. | |
| Falta, violaciones, balón retenido | ❌ | Conocimiento del reglamento y señales del árbitro. | |

## Fútbol

| Situación | Estado | Cómo se reconoce | Notas |
|---|---|---|---|
| **Saque inicial** | ✅ | Ambos equipos en su mitad (≥ 85 % cada uno), todos parados, como mucho dos jugadores de un mismo equipo dentro del círculo central (9,15 m). Mantenimiento 1,5 s. | Lo ejecuta el equipo que está en el círculo. Se emite al inicio de cada parte y tras un gol. |
| **Penalti** | ✅ | Un jugador parado en el punto de penalti (≤ 1,8 m), nadie más a menos de 8,5 m, como mucho otro jugador (el portero) en el área. Mantenimiento 2 s. | Indica el lado y el equipo del lanzador. |
| **Córner** | ✅ | Un jugador parado junto a un banderín de córner (≤ 2,4 m), ≥ 5 jugadores en el área de ese lado, y esos jugadores moviéndose despacio. Mantenimiento 1,5 s. | |
| **Saque de banda** | ✅ | Como en baloncesto, en una línea de banda, fuera de la zona de córner (el córner queda para el detector de córner). | Mismas reglas de confianza que arriba. |
| **Cambio** | ✅ | Como arriba. Un suplente suele entrar a la altura del medio campo, junto a la línea de banda. | |
| Saque de puerta | 🔧 | Lanzador dentro del área de meta, rivales fuera del área. | No implementado: un portero parado se ve igual durante el juego. Necesita el balón. |
| Tiro libre (barrera) | 🔧 | ≥ 3 defensas en línea a unos 10 m del balón, parados. | |
| Gol | 🔧 | Le sigue un saque inicial del *otro* equipo en un minuto; un marcador es más seguro. | No se distingue del inicio de una parte sin el reloj. |
| Fuera de juego, falta, tarjetas | ❌ | | |

## Fútbol sala

| Situación | Estado | Cómo se reconoce | Notas |
|---|---|---|---|
| **Saque inicial** | ✅ | Como en fútbol, círculo central de 3 m. | |
| **Saque de banda con el pie** | ✅ | Jugador parado en una línea de banda (detector de saque de banda). | |
| **Córner** | ✅ | Como en fútbol, con ≥ 3 jugadores en el área. | |
| **Penalti (6 m)** | ✅ | Lanzador en el punto de 6 m (≤ 1,5 m), nadie a menos de 4,5 m, como mucho otro jugador en el área. | |
| **Cambio** | ✅ | Como arriba. | Los cambios volantes son frecuentes; cuenta con muchos cambios *posibles*. |
| Doble penalti (10 m) | 🔧 | | |
| Tiempo muerto | 🔧 | | |

## Balonmano

| Situación | Estado | Cómo se reconoce | Notas |
|---|---|---|---|
| **Saque de centro** | ✅ | Ambos equipos en su mitad, parados, el lanzador en el centro. | Figura como «kick-off» en los datos (`kickoff`) y como «Saque de centro» en la interfaz. |
| **Lanzamiento de 7 m** | ✅ | Lanzador parado en la marca de 7 m (≤ 1,2 m), nadie más a menos de 1,9 m, como mucho otro jugador en el área de 6 m. Mantenimiento 2 s. | |
| **Saque de banda** | ✅ | Jugador parado en una línea de banda. | Incluye los saques de esquina, que se ejecutan en la línea de banda junto a la esquina. |
| **Cambio** | ✅ | Como arriba. | Los cambios volantes son frecuentes; cuenta con muchos cambios *posibles*. |
| Golpe franco | 🔧 | Una barrera defensiva de ≥ 3 jugadores a unos 3 m del lanzador. | |
| Saque de portería | 🔧 | Portero parado en el área de 6 m. | Necesita el balón (el mismo problema que el saque de puerta en fútbol). |
| Exclusión de 2 minutos | 🔧 | Un equipo juega con un jugador menos durante dos minutos: contar los puestos seguidos de ese equipo. | |
| Tiempo muerto | 🔧 | | |

## Hockey (hierba)

| Situación | Estado | Cómo se reconoce | Notas |
|---|---|---|---|
| **Pase de centro** | ✅ | El patrón del saque inicial con un radio central de 5 m. | |
| **Penalti stroke** | ✅ | Lanzador en el punto de 6,4 m (≤ 1,5 m), nadie a menos de 4,5 m, como mucho otro jugador en el círculo. | |
| **Cambio** | ✅ | Como arriba. | Los cambios rotativos son frecuentes. |
| Penalti córner | 🔧 | Sacador en la línea de fondo, atacantes fuera del círculo, defensas tras la línea. | |
| Córner largo, golpe franco | 🔧 | | |

## Rugby

| Situación | Estado | Cómo se reconoce | Notas |
|---|---|---|---|
| **Saque inicial** | ✅ | Equipos en su mitad, parados, como mucho dos jugadores de un mismo equipo a menos de 10 m del centro. | |
| Melé | 🔧 | 14–16 jugadores agarrados en un círculo de unos 4 m, casi parados. | |
| Touch (saque de lateral) | 🔧 | Dos filas paralelas de jugadores junto a la línea de banda. | |
| Golpe de castigo o transformación | 🔧 | Un pateador solo con los demás retirados. | |
| Cambio | 🔧 | La lógica de cambios debe ajustarse para 15 puestos por equipo. | |
| Ensayo | ❌ | Necesita el balón apoyado y el marcador. | |

## Lacrosse

| Situación | Estado | Cómo se reconoce | Notas |
|---|---|---|---|
| **Saque neutral (face-off)** | ✅ | Dos rivales en el centro (≤ 1,2 m), los demás alrededor, parados. | |
| Cambio | 🔧 | Cambios sobre la marcha por la zona de cambios. | |
| Gol, superioridad numérica | ❌ | | |

## Fútbol americano

| Situación | Estado | Cómo se reconoce | Notas |
|---|---|---|---|
| Línea de scrimmage / formación de snap | 🔧 | Dos filas de jugadores enfrentadas en una franja de unas 10 yardas, casi paradas durante ≥ 5 s. Un patrón claro; un buen siguiente paso. | |
| Saque inicial | 🔧 | Una fila de pateadores a lo ancho del campo, receptores al fondo. | |
| Formaciones de despeje, de gol de campo | 🔧 | | |
| Tiempo muerto | 🔧 | | |
| Touchdown, primer down | ❌ | | |

## Voleibol

| Situación | Estado | Cómo se reconoce | Notas |
|---|---|---|---|
| Saque | 🔧 | Un jugador solo detrás de la línea de fondo, parado. | |
| Cambio | 🔧 | Jugadores en la zona de cambios. | |
| Rotación | 🔧 | Cambio de posición en el sentido de las agujas del reloj tras recuperar el saque. | |
| Punto | ❌ | | |

## Otros deportes (rectángulo libre)

| Situación | Estado |
|---|---|
| Saque de banda / reanudación en la línea de banda | ✅ |
| Cambio | ✅ |
| Todo lo demás | ❌ (sin geometría propia del deporte) |

---

## Para desarrolladores: añadir una escena

El detector está en [`src/scenes.rs`](../src/scenes.rs). Una escena es una función que observa a los jugadores de una imagen y propone un candidato (`kind`, `key`, `team`, `pos`, `side`, `confidence`). El código común de temporización gestiona el tiempo de mantenimiento, la emisión única, la espera y el rearme. Pasos:

1. Añadir una variante de `SceneKind`, su cadena `key()` y su tiempo `hold()`.
2. Escribir la función del patrón (ver `free_throws` o `corners` como plantillas) y llamarla para los deportes adecuados en `patterns()`.
3. Añadir una prueba en `tests/scenes.rs`: una formación positiva y el parecido más cercano que **no** debe dispararse.
4. Añadir los nombres en `scripts/gen_i18n.py` (`scene.<kind>`) y en `scene_name` de `src/bin/reco-minimap.rs`, en los cuatro idiomas, y ejecutar el script.

Piezas útiles: `profile.landmark("spot_l")`, `profile.zone_rect('l', 1)` (área / zona), `profile.field.edge_dist(pos)`, `AgentOut.vel`, y el estado del balón en la imagen.
