# Resumen en directo – cómo funciona y cómo se configura

*En otros idiomas: [English](TICKER.md) · [Deutsch](TICKER.de.md) · [Français](TICKER.fr.md)*

![Cómo funciona el resumen en directo](img/ticker-flow.es.svg)

## 1. Qué es

Mientras `reco-minimap live` analiza el vídeo panorámico, detecta **cambios de jugadores** y **situaciones de juego** (saque inicial, saque de banda, córner, penalti, tiro libre, intentos de tiro – según el deporte). El resumen los convierte en un texto corto, lo más reciente arriba, con el tiempo del vídeo:

> 3:01 Cambio Sharks: sale #7, entra #12
> 1:35 Tiro de 3 puntos → Sharks (cerca de la línea)

Funciona para todos los deportes y se muestra en alemán, inglés, francés y español (cada visitante elige su idioma). Puedes mostrarlo en los móviles del pabellón, integrarlo en la web del club y decidir si muestra **números**, **nombres** o **ambos** – y si cada entrada espera antes **tu aprobación**.

## 2. Cómo funciona

Sigue la imagen desde arriba a la izquierda:

1. **Cámara → `reco-minimap live`.** El programa lee el vídeo panorámico y detecta jugadores, cambios y situaciones de juego. Todavía no se envía nada.
2. **Aprobación (opcional).** Con la *aprobación manual*, cada entrada nueva espera en una cola. Abres una página que **solo funciona en tu ordenador** y pulsas *Aprobar* o *Descartar* en cada entrada. Sin aprobación, las entradas se publican de inmediato.
3. **Resumen público.** Las entradas aprobadas forman el resumen. Los nombres solo van incluidos si los permites; con *números* nunca salen del programa.
4. **Dos formas de mostrarlo:**
   * **Móviles en la misma Wi-Fi** abren `http://<tu-ordenador>:8765/ticker`. Sencillo, sin alojamiento web.
   * **Tu web.** El *emisor* escribe el resumen en un archivo pequeño, `ticker.json`, y lo sube a tu alojamiento (SFTP / FTP / WebDAV).
5. **Tu alojamiento** solo guarda **archivos estáticos** (`ticker.json`, `ticker.js`, `ticker.html`). No hay nada que instalar en el servidor: ni PHP, ni base de datos, ni inicio de sesión.
6. **Los visitantes** abren tu página normal. Un pequeño widget de JavaScript (o un iframe) lee `ticker.json` cada 3 segundos y dibuja el resumen. Un script en el navegador de un visitante solo puede *leer*; por eso el envío lo hace el programa.

Las posiciones, el overlay en directo y la página de aprobación se quedan en tu ordenador. Solo las entradas aprobadas llegan al alojamiento.

## 3. Qué necesitas

* `reco-minimap` **0.8.0 o más reciente** ([instalación](../README.md#install); actualiza con `reco-minimap update --yes`). Comprueba con `reco-minimap --version`.
* Una configuración en directo que funcione: modelo, `court.json` y `teams.json` (ver la [guía](WALKTHROUGH.es.md)).
* Las **situaciones de juego** necesitan `--scenes`, los **cambios** necesitan los dorsales (`--numbers`, idealmente con las plantillas).
* Para tu web: un alojamiento con **acceso SFTP o FTP** (casi todos lo tienen) y el programa `curl` (ya incluido en macOS, Linux y Windows 10/11; comprueba con `curl --version`).

## 4. Configuración – elige tu nivel

### A. Solo un enlace (5 minutos, sin alojamiento)

Inicia live como siempre con situaciones y dorsales. En la interfaz es el paso **5 · En vivo** (la casilla *Detectar situaciones de juego* está en el paso 4 · Procesar y vale también para el directo); en la línea de comandos:

```bash
reco-minimap live <fuente> --model <carpeta-modelos> --court court.json --teams teams.json \
    --size 1920x1080 --scenes --numbers
```

El resumen está en **`http://127.0.0.1:8765/ticker`**. Para móviles en la misma Wi-Fi añade `--host 0.0.0.0` (interfaz: *Accesible desde otros dispositivos de la red*) y envía `http://<dirección-de-tu-ordenador>:8765/ticker`. La interfaz muestra el enlace listo; en la línea de comandos encuentra la dirección con `ipconfig` (Windows), `hostname -I` (Linux) o *Ajustes del Sistema → Wi-Fi → Detalles* (macOS). El botón *Compartir* de la página pasa el enlace al menú de compartir del móvil.

> `--host 0.0.0.0` hace también accesibles el overlay en directo y las posiciones para todos los de esa red. Úsalo en redes de confianza (no en la Wi-Fi pública de invitados de un pabellón).

### B. En tu propia web

1. **Consigue tus datos de acceso** de tu proveedor: nombre del servidor, usuario, contraseña y la carpeta de tu web (a menudo `htdocs` o `public_html`). Prefiere **SFTP** (`sftp://`) o `ftps://`; `ftp://` simple envía la contraseña sin cifrar.
2. **Crea los archivos una vez.** Inicia live (o una prueba, ver abajo) con `--ticker-export out/ticker`, o marca *Exportar para una web* en la interfaz. La carpeta `out/ticker` contiene entonces `ticker.html`, `ticker.js`, `index.html`, `.htaccess` y, en cuanto live funciona, `ticker.json`.
3. **Sube la carpeta una vez** a una carpeta de tu alojamiento, por ejemplo `htdocs/ticker/`, con cualquier programa FTP (FileZilla, Cyberduck, WinSCP). Conserva todos los archivos, también `.htaccess`.
4. **Míralo:** abre `https://tu-dominio/ticker/index.html`. Muestra los fragmentos para copiar y un widget de prueba (dice *sin conexión* hasta que haya un partido).
5. **Intégralo** en tu página (sección 5), por ejemplo en un bloque de *HTML personalizado*.
6. **El día del partido** deja que el programa suba `ticker.json` por sí mismo:

   ```bash
   reco-minimap live <fuente> --model <carpeta-modelos> --court court.json --teams teams.json \
       --size 1920x1080 --scenes --numbers \
       --ticker-export out/ticker \
       --ticker-push "sftp://USUARIO:CONTRASEÑA@SERVIDOR/htdocs/ticker/ticker.json"
   ```

   En la interfaz: *Exportar para una web* y, en *Subir automáticamente a*, la misma dirección. El programa sobrescribe solo ese archivo, primero con un nombre temporal y luego renombrado: nadie lee nunca un archivo a medias. Las carpetas que falten se crean.

   Los caracteres especiales de la contraseña se escriben con codificación porcentual: `@` → `%40`, `:` → `%3A`, `/` → `%2F`, `#` → `%23`, `%` → `%25`, espacio → `%20`.

**Otros destinos:** también sirven WebDAV (Nextcloud, IONOS HiDrive …), S3/R2 o un Cloudflare Worker: `--ticker-push https://…/ticker.json [--ticker-key TOKEN]` envía el archivo con HTTP `PUT` (`--ticker-push-method POST` si hace falta). Si prefieres sincronizar la carpeta tú mismo (rsync, Syncthing, Cloudflare Pages, GitHub Pages), usa solo `--ticker-export`.

**Prueba sin partido:** reproduce un partido grabado como si fuera en directo:

```bash
reco-minimap live partido.mp4 --realtime --size 1920x1080 --model … --court court.json --teams teams.json --scenes --numbers --ticker-export out/ticker --ticker-push "sftp://…"
```

### C. Con tu visto bueno antes de mostrar nada

Añade `--ticker-approve` (interfaz: *Aprobación manual*). Abre **`http://127.0.0.1:8765/ticker/admin`** en el mismo ordenador. Las entradas en espera aparecen arriba: *Aprobar* o *Descartar* cada una, o *Aprobar todo*. Una entrada publicada puede *retirarse* (a los visitantes les desaparece en la siguiente actualización). El interruptor *Aprobación manual* activa y desactiva la cola durante el partido. La página solo funciona desde este ordenador – no por Wi-Fi, ni desde otras páginas web. También muestra nombres aunque el resumen público esté en números, para que veas a quién se refiere cada entrada. Las entradas descartadas no llegan ni al resumen ni al alojamiento.

## 5. Integración

**iframe – funciona donde se pueda pegar HTML:**

```html
<iframe src="https://tu-dominio/ticker/ticker.html?embed=1&lang=es"
        style="width:100%;height:480px;border:0" loading="lazy"></iframe>
```

| Opción en la dirección | Significado |
|---|---|
| `embed=1` | solo la lista (sin cabecera ni selector de idioma) – para incrustar |
| `lang=de` `en` `fr` `es` | idioma (sin indicar: idioma del navegador del visitante) |
| `show=numbers` `names` `both` | qué se muestra (solo hasta donde hayas permitido nombres) |
| `theme=dark` `light` | colores (por defecto: según el dispositivo del visitante) |
| `max=20` | como máximo 20 entradas |

Para que el marco tenga la altura de su contenido, añade esto una vez a tu página:

```html
<script>addEventListener('message',function(e){if(e.data&&e.data.type==='reco-ticker-height'){document.querySelector('iframe[src*="ticker.html"]').style.height=e.data.height+'px'}})</script>
```

**Widget – toma la fuente y el estilo de tu página (archivos en el mismo dominio):**

```html
<script src="https://tu-dominio/ticker/ticker.js"
        data-feed="https://tu-dominio/ticker/ticker.json" data-lang="es" data-max="20"></script>
```

o como elemento: `<reco-ticker feed="…/ticker.json" lang="es" max="20" theme="light"></reco-ticker>`. Los colores se cambian con variables CSS en el elemento:

```css
reco-ticker { --rt-home:#c8102e; --rt-away:#005bbb; --rt-acc:#c8102e;
              --rt-card:#fff; --rt-fg:#222; --rt-line:#ddd; --rt-dim:#666; }
```

Si los archivos están en un dominio *distinto* al de tu página, ese servidor debe permitir leer `ticker.json` desde tu página (CORS); el `.htaccess` incluido lo hace en Apache. El iframe no necesita este ajuste.

## 6. Nombres, números y privacidad

`--ticker-names numbers|names|both` define lo que ve el público; por defecto **numbers**.

* **numbers:** `#7`. Los nombres **no se envían a ningún dispositivo**, ni siquiera dentro de `ticker.json`. Es el ajuste seguro, sobre todo en equipos juveniles.
* **names / both:** la página ofrece un interruptor *Números · Nombres · Ambos*; cada visitante puede mostrar **menos** de lo que permites, nunca más. `show=numbers` en un enlace lo preajusta.

Los nombres salen de las listas de plantilla, escritos tras el número con `:` o `=`: `--roster-home "7:Anna Müller, 12:Ben, 1g:Tom"` (la marca de portero sigue funcionando; los nombres pueden llevar espacios, no comas). En la interfaz usa los campos de plantilla de *Dorsales* (en el paso En vivo). Quien no tiene nombre aparece como `#7`; un número desconocido como *un jugador*. `--home-name` y `--away-name` fijan los nombres de los equipos en la cabecera.

Piensa si los nombres de menores deben estar en una página pública y consigue el permiso de padres y club antes de pasar a *names* o *both*.

## 7. Lista de comprobación del día del partido

1. Ordenador enchufado, sin suspensión; conexión a internet si subes archivos.
2. Inicia `live` unos 10 minutos antes del comienzo; vigila los mensajes de *subida del resumen* (una subida fallida se reintenta y se avisa una vez, y otra vez cuando vuelve a funcionar).
3. Abre tu web en un móvil: el estado dice **en vivo**. Tras unos 45 segundos sin señal de vida dice **sin conexión**.
4. Con aprobación manual: deja abierta `…/ticker/admin` en el ordenador.
5. Tras el partido, detén el programa. La página conserva las últimas entradas; iniciar `live` de nuevo empieza con un resumen vacío y sobrescribe `ticker.json`.

## 8. Resumen de opciones

| Línea de comandos | Interfaz (paso 5 · En vivo) | Por defecto | Significado |
|---|---|---|---|
| `--scenes` | Detectar situaciones de juego (paso 4 · Procesar) | no | detectar saques e intentos de tiro |
| `--numbers` | Dorsales (también en el paso En vivo) | no | leer dorsales (necesario para los cambios) |
| `--roster-home/away "7:Anna, 12:Ben"` | campos de plantilla | – | números de la plantilla, nombres opcionales |
| `--ticker-names numbers\|names\|both` | Mostrar | `numbers` | lo que puede ver el público |
| `--home-name` / `--away-name` | Nombre local / visitante | – | nombres de equipo en la cabecera |
| `--ticker-approve` | Aprobación manual | no | las entradas esperan tu aprobación |
| `--host 0.0.0.0` | Accesible desde otros dispositivos | no | móviles en la misma Wi-Fi |
| `--ticker-export CARPETA` | Exportar para una web | no | escribir los archivos estáticos en una carpeta |
| `--ticker-push URL` | Subir automáticamente a | no | `sftp://`, `ftps://`, `ftp://`, `https://` (WebDAV, S3, Worker) |
| `--ticker-key TOKEN` | Clave | – | enviada como `Authorization: Bearer` (solo http/https) |
| `--ticker-push-method` | – | `PUT` | `PUT` o `POST` (solo http/https) |

## 9. Si algo no funciona

| Problema | Causa probable y solución |
|---|---|
| La página dice **sin conexión** | El programa no está en marcha, o la subida falla. Busca *Falló la subida del resumen (…)* en la consola del programa. Revisa la dirección, la contraseña (¡codificación porcentual!) y la carpeta. |
| **No aparece nada** | Aún no se ha detectado nada. Las situaciones necesitan `--scenes`, los cambios `--numbers`. Con aprobación manual, las entradas esperan en `…/ticker/admin`. |
| **No salen los nombres** | `--ticker-names` sigue en `numbers`, o la lista de plantilla no tiene entradas como `7:Nombre`. |
| **Los móviles no llegan al enlace** | Falta `--host 0.0.0.0`, un cortafuegos, o una Wi-Fi que aísla los dispositivos (redes de invitados). |
| **El widget sigue vacío** | Archivos en otro dominio: CORS (usa el `.htaccess` o el iframe). Comprueba que `ticker.json` se abre en el navegador. |
| **Entradas antiguas al recargar** | Hay una caché de por medio. La página pide un archivo nuevo cada vez y `.htaccess` pone `no-store`; en otros servidores desactiva la caché de `*.json`. |
| `--ticker-push necesita el programa curl` | Instala curl, o usa solo `--ticker-export` y sincroniza la carpeta tú mismo. |
| Caracteres especiales rompen la subida | Escríbelos con codificación porcentual en la dirección (sección 4 B). |

## 10. Límites

* El resumen nace de una **detección automática** a partir de la imagen de la cámara. Puede pasar cosas por alto, llegar tarde o equivocarse; un cambio *posible* se marca como *sin confirmar*. No es un acta arbitral y **no muestra el marcador**.
* El tiempo del resumen es el **tiempo del vídeo**.
* El resumen conserva las últimas 600 entradas; quien llega tarde las ve todas.
* Sin un servidor web propio, un enlace en tu ordenador solo es accesible desde fuera de tu red mediante un túnel o servicio que configures tú mismo: el hub en directo no tiene inicio de sesión. Por eso la vía «web» sube un archivo.
