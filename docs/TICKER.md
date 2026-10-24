# Live ticker – how it works and how to set it up

*In other languages: [Deutsch](TICKER.de.md) · [Français](TICKER.fr.md) · [Español](TICKER.es.md)*

![How the live ticker works](img/ticker-flow.en.svg)

## 1. What it is

While `reco-minimap live` watches the panorama video, it notices **substitutions** and **game situations** (kick-off, throw-in, corner, penalty, free throw, shot attempts – which ones depends on the sport). The ticker turns these into a short text feed, newest first, with the video time:

> 3:01 Substitution Sharks: #7 off, #12 on
> 1:35 3-point attempt → Sharks (close to the line)

It works for every sport and shows in German, English, French and Spanish (each viewer picks the language). You can show it on phones in the hall, put it on your club's web site, and decide whether it shows **numbers**, **names** or **both** – and whether every entry waits for **your OK** first.

## 2. How it works

Follow the picture from top left:

1. **Camera → `reco-minimap live`.** The program reads the panorama video and detects players, substitutions and game situations. Nothing is sent anywhere yet.
2. **Release (optional).** With *Manual release* switched on, every new entry waits in a queue. You open a page that works **only on your computer**, and press *Release* or *Discard* per entry. Without it, entries are published at once.
3. **Public feed.** Released entries form the feed. Names are part of it only if you allowed them; with *numbers* they never leave the program.
4. **Two ways to show it:**
   * **Phones on the same Wi-Fi** open `http://<your-computer>:8765/ticker`. Simple, needs no web space.
   * **Your web site.** The *sender* writes the feed into one small file, `ticker.json`, and uploads it to your web space (SFTP / FTP / WebDAV).
5. **Your web space** only holds **static files** (`ticker.json`, `ticker.js`, `ticker.html`). There is nothing to install on the server: no PHP, no database, no login.
6. **Visitors** open your normal page. A small JavaScript widget (or an iframe) on it reads `ticker.json` every 3 seconds and draws the feed. A script in a visitor's browser can only *read*; that is why the program does the sending.

Positions, the live overlay and the release page stay on your computer. Only released entries reach the web space.

## 3. What you need

* `reco-minimap` **0.8.0 or newer** ([installation](../README.md#install); update with `reco-minimap update --yes`). Check with `reco-minimap --version`.
* A working live setup: a model, `court.json` and `teams.json` (see the [walkthrough](WALKTHROUGH.md)).
* **Game situations** need `--scenes`; **substitutions** need jersey numbers (`--numbers`, ideally with squad lists).
* For your web site: a web space with **SFTP or FTP access** (nearly every hoster has it) and the program `curl` (already on macOS, Linux and Windows 10/11; check with `curl --version`).

## 4. Set up – choose your level

### A. Only a link (5 minutes, no web space)

Start live as usual with the situation and number options. In the interface that is step **5 · Live** (the checkbox *Detect game situations* sits in step 4 · Process and counts for live as well); on the command line:

```bash
reco-minimap live <source> --model <models-folder> --court court.json --teams teams.json \
    --size 1920x1080 --scenes --numbers
```

The ticker is at **`http://127.0.0.1:8765/ticker`**. For phones on the same Wi-Fi add `--host 0.0.0.0` (interface: *Reachable from other devices on the network*) and send `http://<address-of-your-computer>:8765/ticker`. The interface shows the finished link; on the command line find the address with `ipconfig` (Windows), `hostname -I` (Linux) or *System Settings → Wi-Fi → Details* (macOS). The *Share* button on the page hands the link to the phone's share sheet.

> `--host 0.0.0.0` also makes the live overlay and the position data reachable for everyone in that network. Use it on networks you trust (not a hall's public guest Wi-Fi).

### B. On your own web site

1. **Get your access data** from your hoster: server name, user, password and the folder of your web site (often `htdocs` or `public_html`). Prefer **SFTP** (`sftp://`) or `ftps://`; plain `ftp://` sends the password unencrypted.
2. **Create the files once.** Start live (or a test, see below) with `--ticker-export out/ticker`, or tick *Export for a web site* in the interface. The folder `out/ticker` now contains `ticker.html`, `ticker.js`, `index.html`, `.htaccess` and, as soon as live runs, `ticker.json`.
3. **Upload the folder once** into a folder on your web space, for example `htdocs/ticker/`, with any FTP program (FileZilla, Cyberduck, WinSCP). Keep all files, including `.htaccess`.
4. **Look at it:** open `https://your-domain/ticker/index.html`. It shows the snippets to copy and a test widget (it says *offline* until a match runs).
5. **Embed it** in your page (section 5), for example in a *Custom HTML* block.
6. **On match day** let the program upload `ticker.json` itself:

   ```bash
   reco-minimap live <source> --model <models-folder> --court court.json --teams teams.json \
       --size 1920x1080 --scenes --numbers \
       --ticker-export out/ticker \
       --ticker-push "sftp://USER:PASSWORD@SERVER/htdocs/ticker/ticker.json"
   ```

   In the interface: *Export for a web site* and, in *Upload automatically to*, the same address. The program overwrites only that one file, under a temporary name first and then renamed, so nobody ever reads a half-written file. Missing folders are created.

   Special characters in the password are written percent-encoded: `@` → `%40`, `:` → `%3A`, `/` → `%2F`, `#` → `%23`, `%` → `%25`, space → `%20`.

**Other upload targets:** WebDAV (Nextcloud, IONOS HiDrive …), S3/R2 or a Cloudflare Worker work too: `--ticker-push https://…/ticker.json [--ticker-key TOKEN]` sends the file with HTTP `PUT` (`--ticker-push-method POST` if needed). If you prefer to sync the folder yourself (rsync, Syncthing, Cloudflare Pages, GitHub Pages), use only `--ticker-export`.

**Test without a match:** play a recorded game as if it were live:

```bash
reco-minimap live game.mp4 --realtime --size 1920x1080 --model … --court court.json --teams teams.json --scenes --numbers --ticker-export out/ticker --ticker-push "sftp://…"
```

### C. With your OK before anything is shown

Add `--ticker-approve` (interface: *Manual release*). Open **`http://127.0.0.1:8765/ticker/admin`** on the same computer. Waiting entries appear at the top: *Release* or *Discard* each, or *Release all*. A published entry can be *taken back* (visitors' pages drop it at their next refresh). The switch *Manual release* turns the queue on and off during the match. The page works only from this computer – not over Wi-Fi, not from other web pages. It also shows names even when the public ticker is set to numbers, so you can see who an entry means. Entries you discard never reach the feed or the web space.

## 5. Embedding

**iframe – works everywhere you can paste HTML:**

```html
<iframe src="https://your-domain/ticker/ticker.html?embed=1&lang=en"
        style="width:100%;height:480px;border:0" loading="lazy"></iframe>
```

| Address option | Meaning |
|---|---|
| `embed=1` | only the list (no header, no language switch) – for embedding |
| `lang=de` `en` `fr` `es` | language (without it the visitor's browser language) |
| `show=numbers` `names` `both` | what is shown (only as far as you allowed names) |
| `theme=dark` `light` | colours (default: follows the visitor's device) |
| `max=20` | show at most 20 entries |

To make the frame as high as its content, add this to your page once:

```html
<script>addEventListener('message',function(e){if(e.data&&e.data.type==='reco-ticker-height'){document.querySelector('iframe[src*="ticker.html"]').style.height=e.data.height+'px'}})</script>
```

**Widget – takes your page's font and style (files on the same domain):**

```html
<script src="https://your-domain/ticker/ticker.js"
        data-feed="https://your-domain/ticker/ticker.json" data-lang="en" data-max="20"></script>
```

or as an element: `<reco-ticker feed="…/ticker.json" lang="en" max="20" theme="light"></reco-ticker>`. Colours can be changed with CSS variables on the element:

```css
reco-ticker { --rt-home:#c8102e; --rt-away:#005bbb; --rt-acc:#c8102e;
              --rt-card:#fff; --rt-fg:#222; --rt-line:#ddd; --rt-dim:#666; }
```

If the files are on a *different* domain than your page, that server must allow reading `ticker.json` from your page (CORS); the supplied `.htaccess` does it on Apache. The iframe needs no such setting.

## 6. Names, numbers and privacy

`--ticker-names numbers|names|both` sets what the public sees; the default is **numbers**.

* **numbers:** `#7`. Names are **never sent** to any device, not even into `ticker.json`. This is the safe setting, especially for youth teams.
* **names / both:** the page offers a switch *Numbers · Names · Both*, so each visitor can show **less** than you allow, never more. `show=numbers` in a link presets it.

Names come from the squad lists, written after the number with `:` or `=`: `--roster-home "7:Anna Müller, 12:Ben, 1g:Tom"` (the goalkeeper mark still works; names may contain spaces, not commas). In the interface use the squad fields of *Jersey numbers* (on the Live step). Someone without a name is shown as `#7`; an unknown number as *a player*. `--home-name` and `--away-name` set the team names in the header.

Think about whether names of minors belong on a public page, and about permissions from parents and the club, before you switch to *names* or *both*.

## 7. Match-day checklist

1. Computer on mains power, no sleep mode; internet connection if you upload.
2. Start `live` about 10 minutes before the start; check the status line for *Ticker upload* messages (a failed upload is retried and reported once, and again when it works).
3. Open your web page on a phone: the status reads **live**. After about 45 seconds without a sign of life it reads **offline**.
4. With manual release: keep `…/ticker/admin` open on the computer.
5. After the match stop the program. The page keeps the last entries; starting `live` again begins an empty feed and overwrites `ticker.json`.

## 8. Option reference

| Command line | Interface (step 5 · Live) | Default | Meaning |
|---|---|---|---|
| `--scenes` | Detect game situations (step 4 · Process) | off | detect restarts and shot attempts |
| `--numbers` | Jersey numbers (also on the Live step) | off | read jersey numbers (needed for substitutions) |
| `--roster-home/away "7:Anna, 12:Ben"` | squad fields | – | squad numbers, optional names |
| `--ticker-names numbers\|names\|both` | Show | `numbers` | what the public may see |
| `--home-name` / `--away-name` | Home / Away team name | – | team names in the header |
| `--ticker-approve` | Manual release | off | entries wait for your OK |
| `--host 0.0.0.0` | Reachable from other devices | off | phones on the same Wi-Fi |
| `--ticker-export DIR` | Export for a web site | off | write the static files into a folder |
| `--ticker-push URL` | Upload automatically to | off | `sftp://`, `ftps://`, `ftp://`, `https://` (WebDAV, S3, Worker) |
| `--ticker-key TOKEN` | Key | – | sent as `Authorization: Bearer` (http/https only) |
| `--ticker-push-method` | – | `PUT` | `PUT` or `POST` (http/https only) |

## 9. Troubleshooting

| Problem | Likely cause and fix |
|---|---|
| Page says **offline** | The program is not running, or the upload fails. Look at the program's console for *Ticker upload failed (…)*. Check address, password (percent-encoding!) and that the folder exists. |
| **Nothing appears** | Nothing was detected yet. Situations need `--scenes`, substitutions need `--numbers`. With manual release, entries wait in `…/ticker/admin`. |
| **Names do not show** | `--ticker-names` is still `numbers`, or the squad list has no `7:Name` entries. |
| **Phones cannot open the link** | Missing `--host 0.0.0.0`, a firewall, or a Wi-Fi that isolates devices (guest networks). |
| **Widget empty on my site** | Files on another domain: CORS (use the `.htaccess` or the iframe). Check that `ticker.json` opens in the browser. |
| **Old entries after reload** | A cache in between. The page asks for a fresh file every time and `.htaccess` sets `no-store`; for other servers disable caching for `*.json`. |
| `--ticker-push needs the curl program` | Install curl, or use `--ticker-export` and sync the folder yourself. |
| Special characters break the upload | Write them percent-encoded in the address (section 4 B). |

## 10. Limits

* The ticker is built from **automatic detection** from the camera image. It can miss things, be late, or be wrong; a *possible* substitution is marked *unconfirmed*. It is not a referee's report and **does not show the score**.
* Time in the feed is the **video time**.
* The feed keeps the latest 600 entries; a visitor who joins late sees them all.
* Without a web server of your own, a link on your computer is reachable from outside your network only through a tunnel or service you set up yourself – the live hub has no login. That is why the web-site route uploads a file instead.
