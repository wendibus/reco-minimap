# Live-Ticker – Funktionsweise und Einrichtung

*In anderen Sprachen: [English](TICKER.md) · [Français](TICKER.fr.md) · [Español](TICKER.es.md)*

![So funktioniert der Live-Ticker](img/ticker-flow.de.svg)

## 1. Was das ist

Während `reco-minimap live` das Panoramavideo auswertet, erkennt es **Wechsel** und **Spielsituationen** (Anstoß, Einwurf, Ecke, Strafstoß, Freiwurf, Wurfversuche – welche, hängt von der Sportart ab). Der Ticker macht daraus einen kurzen Text-Feed, neueste Meldung oben, mit der Videozeit:

> 3:01 Wechsel Sharks: #7 raus, #12 rein
> 1:35 Wurfversuch 3 Punkte → Sharks (knapp an der Linie)

Er funktioniert für jede Sportart und erscheint auf Deutsch, Englisch, Französisch und Spanisch (jeder Besucher wählt seine Sprache). Du kannst ihn auf Handys in der Halle zeigen, auf der Vereinswebseite einbauen und festlegen, ob **Nummern**, **Namen** oder **beides** erscheinen – und ob jede Meldung erst auf **dein OK** wartet.

## 2. Funktionsweise

Folge dem Bild von oben links:

1. **Kamera → `reco-minimap live`.** Das Programm liest das Panoramavideo und erkennt Spieler, Wechsel und Spielsituationen. Noch wird nichts irgendwohin gesendet.
2. **Freigabe (optional).** Bei eingeschalteter *Manueller Freigabe* wartet jede neue Meldung in einer Warteschlange. Du öffnest eine Seite, die **nur auf deinem Rechner** funktioniert, und drückst je Meldung *Freigeben* oder *Verwerfen*. Ohne Freigabe erscheinen Meldungen sofort.
3. **Öffentlicher Ticker.** Freigegebene Meldungen bilden den Feed. Namen sind nur dabei, wenn du sie erlaubst; bei *Nummern* verlassen sie das Programm nie.
4. **Zwei Wege, ihn zu zeigen:**
   * **Handys im selben WLAN** öffnen `http://<dein-Rechner>:8765/ticker`. Einfach, kein Webspace nötig.
   * **Deine Webseite.** Der *Sender* schreibt den Feed in eine kleine Datei, `ticker.json`, und lädt sie auf deinen Webspace hoch (SFTP / FTP / WebDAV).
5. **Dein Webspace** enthält nur **statische Dateien** (`ticker.json`, `ticker.js`, `ticker.html`). Auf dem Server ist nichts zu installieren: kein PHP, keine Datenbank, kein Login.
6. **Besucher** öffnen deine normale Seite. Ein kleines JavaScript-Widget (oder ein iframe) liest alle 3 Sekunden `ticker.json` und zeichnet den Feed. Ein Script im Browser eines Besuchers kann nur *lesen*; deshalb sendet das Programm.

Positionen, das Live-Overlay und die Freigabe-Seite bleiben auf deinem Rechner. Nur freigegebene Meldungen erreichen den Webspace.

## 3. Was du brauchst

* `reco-minimap` **0.8.0 oder neuer** ([Installation](../README.md#install); aktualisieren mit `reco-minimap update --yes`). Prüfen mit `reco-minimap --version`.
* Eine funktionierende Live-Einrichtung: Modell, `court.json` und `teams.json` (siehe [Anleitung](WALKTHROUGH.de.md)).
* **Spielsituationen** brauchen `--scenes`, **Wechsel** brauchen Trikotnummern (`--numbers`, am besten mit Kaderlisten).
* Für deine Webseite: ein Webspace mit **SFTP- oder FTP-Zugang** (fast jeder Anbieter hat ihn) und das Programm `curl` (auf macOS, Linux und Windows 10/11 schon vorhanden; prüfen mit `curl --version`).

## 4. Einrichten – such dir die Stufe aus

### A. Nur ein Link (5 Minuten, kein Webspace)

Starte Live wie gewohnt mit Situationen und Nummern. In der Oberfläche ist das Schritt **5 · Live** (das Häkchen *Spielsituationen erkennen* steht in Schritt 4 · Verarbeiten und gilt auch für Live); auf der Kommandozeile:

```bash
reco-minimap live <quelle> --model <modelle-ordner> --court court.json --teams teams.json \
    --size 1920x1080 --scenes --numbers
```

Der Ticker liegt unter **`http://127.0.0.1:8765/ticker`**. Für Handys im selben WLAN ergänze `--host 0.0.0.0` (Oberfläche: *Für andere Geräte im Netzwerk erreichbar*) und schicke `http://<adresse-deines-rechners>:8765/ticker`. Die Oberfläche zeigt den fertigen Link; auf der Kommandozeile findest du die Adresse mit `ipconfig` (Windows), `hostname -I` (Linux) oder *Systemeinstellungen → WLAN → Details* (macOS). Der Knopf *Teilen* auf der Seite gibt den Link an das Teilen-Menü des Handys.

> `--host 0.0.0.0` macht auch das Live-Overlay und die Positionsdaten für alle in diesem Netz erreichbar. Nutze es nur in Netzen, denen du traust (nicht im öffentlichen Gast-WLAN einer Halle).

### B. Auf deiner eigenen Webseite

1. **Zugangsdaten holen** bei deinem Anbieter: Servername, Benutzer, Passwort und den Ordner deiner Webseite (oft `htdocs` oder `public_html`). Nimm bevorzugt **SFTP** (`sftp://`) oder `ftps://`; einfaches `ftp://` sendet das Passwort unverschlüsselt.
2. **Dateien einmal erzeugen.** Starte Live (oder einen Test, siehe unten) mit `--ticker-export out/ticker` oder setze in der Oberfläche das Häkchen *Für die Webseite exportieren*. Der Ordner `out/ticker` enthält dann `ticker.html`, `ticker.js`, `index.html`, `.htaccess` und, sobald Live läuft, `ticker.json`.
3. **Den Ordner einmal hochladen** in einen Ordner deines Webspace, zum Beispiel `htdocs/ticker/`, mit einem beliebigen FTP-Programm (FileZilla, Cyberduck, WinSCP). Alle Dateien behalten, auch `.htaccess`.
4. **Ansehen:** Öffne `https://deine-domain/ticker/index.html`. Sie zeigt die Schnipsel zum Kopieren und ein Test-Widget (es steht auf *offline*, bis ein Spiel läuft).
5. **Einbauen** in deine Seite (Abschnitt 5), zum Beispiel in einen Block *Benutzerdefiniertes HTML*.
6. **Am Spieltag** lädt das Programm `ticker.json` selbst hoch:

   ```bash
   reco-minimap live <quelle> --model <modelle-ordner> --court court.json --teams teams.json \
       --size 1920x1080 --scenes --numbers \
       --ticker-export out/ticker \
       --ticker-push "sftp://BENUTZER:PASSWORT@SERVER/htdocs/ticker/ticker.json"
   ```

   In der Oberfläche: *Für die Webseite exportieren* und bei *Automatisch hochladen nach* dieselbe Adresse. Das Programm überschreibt nur diese eine Datei, erst unter einem Zwischennamen, dann umbenannt – niemand liest je eine halbe Datei. Fehlende Ordner werden angelegt.

   Sonderzeichen im Passwort schreibst du prozentcodiert: `@` → `%40`, `:` → `%3A`, `/` → `%2F`, `#` → `%23`, `%` → `%25`, Leerzeichen → `%20`.

**Andere Ziele:** WebDAV (Nextcloud, IONOS HiDrive …), S3/R2 oder ein Cloudflare Worker gehen auch: `--ticker-push https://…/ticker.json [--ticker-key TOKEN]` sendet die Datei per HTTP `PUT` (bei Bedarf `--ticker-push-method POST`). Wenn du den Ordner lieber selbst abgleichst (rsync, Syncthing, Cloudflare Pages, GitHub Pages), nutze nur `--ticker-export`.

**Test ohne Spiel:** Spiele ein aufgezeichnetes Spiel, als wäre es live:

```bash
reco-minimap live spiel.mp4 --realtime --size 1920x1080 --model … --court court.json --teams teams.json --scenes --numbers --ticker-export out/ticker --ticker-push "sftp://…"
```

### C. Mit deinem OK, bevor etwas erscheint

Ergänze `--ticker-approve` (Oberfläche: *Manuelle Freigabe*). Öffne **`http://127.0.0.1:8765/ticker/admin`** auf demselben Rechner. Wartende Meldungen stehen oben: je Meldung *Freigeben* oder *Verwerfen*, oder *Alle freigeben*. Eine veröffentlichte Meldung lässt sich *zurücknehmen* (bei den Besuchern verschwindet sie beim nächsten Aktualisieren). Der Schalter *Manuelle Freigabe* schaltet die Warteschlange während des Spiels ein und aus. Die Seite funktioniert nur von diesem Rechner – nicht über WLAN, nicht von anderen Webseiten aus. Sie zeigt auch dann Namen, wenn der öffentliche Ticker auf Nummern steht, damit du siehst, wer gemeint ist. Verworfene Meldungen erreichen weder den Feed noch den Webspace.

## 5. Einbauen

**iframe – geht überall, wo man HTML einfügen kann:**

```html
<iframe src="https://deine-domain/ticker/ticker.html?embed=1&lang=de"
        style="width:100%;height:480px;border:0" loading="lazy"></iframe>
```

| Option in der Adresse | Bedeutung |
|---|---|
| `embed=1` | nur die Liste (ohne Kopf und Sprachwahl) – zum Einbetten |
| `lang=de` `en` `fr` `es` | Sprache (ohne Angabe: Sprache des Besucher-Browsers) |
| `show=numbers` `names` `both` | Anzeige (nur so weit, wie du Namen erlaubt hast) |
| `theme=dark` `light` | Farben (Standard: folgt dem Gerät des Besuchers) |
| `max=20` | höchstens 20 Meldungen |

Damit der Rahmen so hoch wird wie sein Inhalt, füge dies einmal auf deiner Seite ein:

```html
<script>addEventListener('message',function(e){if(e.data&&e.data.type==='reco-ticker-height'){document.querySelector('iframe[src*="ticker.html"]').style.height=e.data.height+'px'}})</script>
```

**Widget – übernimmt Schrift und Stil deiner Seite (Dateien auf derselben Domain):**

```html
<script src="https://deine-domain/ticker/ticker.js"
        data-feed="https://deine-domain/ticker/ticker.json" data-lang="de" data-max="20"></script>
```

oder als Element: `<reco-ticker feed="…/ticker.json" lang="de" max="20" theme="light"></reco-ticker>`. Farben änderst du mit CSS-Variablen am Element:

```css
reco-ticker { --rt-home:#c8102e; --rt-away:#005bbb; --rt-acc:#c8102e;
              --rt-card:#fff; --rt-fg:#222; --rt-line:#ddd; --rt-dim:#666; }
```

Liegen die Dateien auf einer *anderen* Domain als deine Seite, muss dieser Server das Lesen von `ticker.json` von deiner Seite aus erlauben (CORS); die mitgelieferte `.htaccess` macht das bei Apache. Der iframe braucht diese Einstellung nicht.

## 6. Namen, Nummern und Datenschutz

`--ticker-names numbers|names|both` legt fest, was die Öffentlichkeit sieht; Standard ist **numbers**.

* **numbers:** `#7`. Namen werden **an kein Gerät gesendet**, auch nicht in `ticker.json`. Das ist die sichere Einstellung, besonders bei Jugendmannschaften.
* **names / both:** Die Seite bietet einen Schalter *Nummern · Namen · Beides*, jeder Besucher kann **weniger** anzeigen als du erlaubst, nie mehr. `show=numbers` im Link stellt es voraus.

Namen kommen aus den Kaderlisten, nach der Nummer mit `:` oder `=`: `--roster-home "7:Anna Müller, 12:Ben, 1g:Tom"` (die Torwart-Markierung funktioniert weiter; Namen dürfen Leerzeichen enthalten, keine Kommas). In der Oberfläche nutzt du die Kaderfelder bei *Trikotnummern* (im Schritt Live). Wer keinen Namen hat, erscheint als `#7`; eine unbekannte Nummer als *ein Spieler*. `--home-name` und `--away-name` setzen die Teamnamen in der Kopfzeile.

Überlege, ob Namen Minderjähriger auf eine öffentliche Seite gehören, und kläre Einverständnis von Eltern und Verein, bevor du auf *names* oder *both* stellst.

## 7. Checkliste für den Spieltag

1. Rechner am Netzteil, kein Ruhezustand; Internetverbindung, wenn du hochlädst.
2. `live` etwa 10 Minuten vor Beginn starten; in der Statuszeile auf *Ticker-Upload*-Meldungen achten (ein fehlgeschlagener Upload wird wiederholt und einmal gemeldet, und noch einmal, wenn er wieder klappt).
3. Deine Webseite auf einem Handy öffnen: Der Status zeigt **live**. Nach etwa 45 Sekunden ohne Lebenszeichen zeigt er **offline**.
4. Bei manueller Freigabe: `…/ticker/admin` auf dem Rechner offen lassen.
5. Nach dem Spiel das Programm beenden. Die Seite behält die letzten Meldungen; ein neuer Start von `live` beginnt mit leerem Feed und überschreibt `ticker.json`.

## 8. Optionen im Überblick

| Kommandozeile | Oberfläche (Schritt 5 · Live) | Standard | Bedeutung |
|---|---|---|---|
| `--scenes` | Spielsituationen erkennen (Schritt 4 · Verarbeiten) | aus | Anstöße, Einwürfe, Wurfversuche erkennen |
| `--numbers` | Trikotnummern (auch im Schritt Live) | aus | Trikotnummern lesen (nötig für Wechsel) |
| `--roster-home/away "7:Anna, 12:Ben"` | Kaderfelder | – | Kadernummern, optional Namen |
| `--ticker-names numbers\|names\|both` | Anzeige | `numbers` | was die Öffentlichkeit sehen darf |
| `--home-name` / `--away-name` | Name Heim / Gast | – | Teamnamen in der Kopfzeile |
| `--ticker-approve` | Manuelle Freigabe | aus | Meldungen warten auf dein OK |
| `--host 0.0.0.0` | Für andere Geräte erreichbar | aus | Handys im selben WLAN |
| `--ticker-export ORDNER` | Für die Webseite exportieren | aus | statische Dateien in einen Ordner schreiben |
| `--ticker-push URL` | Automatisch hochladen nach | aus | `sftp://`, `ftps://`, `ftp://`, `https://` (WebDAV, S3, Worker) |
| `--ticker-key TOKEN` | Schlüssel | – | als `Authorization: Bearer` gesendet (nur http/https) |
| `--ticker-push-method` | – | `PUT` | `PUT` oder `POST` (nur http/https) |

## 9. Wenn etwas nicht klappt

| Problem | Wahrscheinliche Ursache und Lösung |
|---|---|
| Seite zeigt **offline** | Das Programm läuft nicht, oder der Upload scheitert. In der Konsole des Programms nach *Ticker-Upload fehlgeschlagen (…)* suchen. Adresse, Passwort (Prozentcodierung!) und Ordner prüfen. |
| **Nichts erscheint** | Es wurde noch nichts erkannt. Situationen brauchen `--scenes`, Wechsel `--numbers`. Bei manueller Freigabe warten Meldungen unter `…/ticker/admin`. |
| **Namen erscheinen nicht** | `--ticker-names` steht noch auf `numbers`, oder die Kaderliste hat keine Einträge wie `7:Name`. |
| **Handys erreichen den Link nicht** | `--host 0.0.0.0` fehlt, eine Firewall, oder ein WLAN, das Geräte voneinander trennt (Gast-Netze). |
| **Widget bleibt leer** | Dateien auf anderer Domain: CORS (`.htaccess` nutzen oder den iframe). Prüfen, ob sich `ticker.json` im Browser öffnen lässt. |
| **Nach Neuladen alte Meldungen** | Ein Cache dazwischen. Die Seite fordert jedes Mal eine frische Datei an und `.htaccess` setzt `no-store`; bei anderen Servern Caching für `*.json` abschalten. |
| `--ticker-push braucht das Programm curl` | curl installieren, oder nur `--ticker-export` nutzen und den Ordner selbst abgleichen. |
| Sonderzeichen brechen den Upload | Prozentcodiert in die Adresse schreiben (Abschnitt 4 B). |

## 10. Grenzen

* Der Ticker entsteht durch **automatische Erkennung** aus dem Kamerabild. Er kann etwas übersehen, zu spät kommen oder falsch liegen; ein *möglicher* Wechsel ist als *unbestätigt* markiert. Er ist kein Spielbericht und **zeigt den Spielstand nicht**.
* Die Zeit im Feed ist die **Videozeit**.
* Der Feed behält die letzten 600 Meldungen; wer später dazukommt, sieht alle.
* Ohne eigenen Webserver ist ein Link auf deinem Rechner von außerhalb deines Netzes nur über einen Tunnel oder Dienst erreichbar, den du selbst einrichtest – der Live-Hub hat kein Login. Deshalb lädt der Weg über die Webseite eine Datei hoch.
