# Adaptive Live-Verarbeitung (experimentell)

Die Offline-Analyse (`run`) bleibt unverändert. Im Live-Modus ist eine neue,
sportneutrale Kachelauswahl zuschaltbar: `--adaptive-live` oder das entsprechende
Kontrollkästchen unter Live in der Oberfläche. Sie verwendet ausschließlich
Bildpositionen kürzlich beobachteter Personen und Bälle, keine Basketballregeln.

Zwischen vollständigen Suchläufen deckt eine Greedy-Auswahl die letzten Boxen
mit einem Bewegungsrand ab. Zuschnitt und Modellauflösung der vorhandenen
Kacheln bleiben unverändert. Standardmäßig erfolgt alle zwei Sekunden wieder
eine Vollsuche (`--sweep-secs`, begrenzt auf 0,1–10 s), beim nächsten verfügbaren
Analysebild. Fehlende/ungültige Beobachtungen oder Punkte außerhalb des
Suchbereichs führen ebenfalls zur Vollsuche. Eine Vollsuche kann den gewählten
ROI nicht erweitern: hohe Ballflüge müssen schon bei dessen Wahl bedacht werden.

Wichtig: Die Auswahl kann andere überlappende Kacheln verwenden als diejenige,
die ein Objekt ursprünglich erkannt hat. Schnelle Bewegung, kleine Bälle und
Neueintritte können zwischen Vollsuchen verloren gehen. Dies ist keine Garantie
gleicher Erkennungsqualität. Für vollständige Auswertung weiterhin `run` nutzen.

Die bereits vorhandene Ein-Bild-Warteschlange behält nur das neueste dekodierte
Bild. Übersprungene Bilder sind nun in jeder JSON-Zeile als
`performance.dropped_frames` sichtbar, zusammen mit Kachelzahlen, Vollsuche und
Inferenzzeit. Die Browser-Debugansicht (`?debug=1`) zeigt diese Werte ebenfalls.
`age_ms` misst **ab Dekodierung**, nicht ab Kameraaufnahme: Netzwerk-, Decoder-
und Browserverzögerungen sind damit nicht vollständig erfasst.

Optionale Trikot-OCR läuft im Live-Modus in einem eigenen Hintergrundthread,
mit höchstens einem offenen Auftrag aus maximal 16 Torsoausschnitten, nicht
ganzen Panoramen. Neue Aufträge frühestens alle zwei Sekunden (`--ocr-secs`).
Identitätsschlüssel (Slot, Segment, Mannschaft) und Beobachtungsalter (maximal
drei Sekunden) werden vor der Übernahme erneut geprüft. Ergebnisse verlorener
Tracks werden verworfen. JSON `ocr` zeigt offene, angewandte, veraltete und
fehlerhafte Ergebnisse. Nach zwei Sekunden startet der Worker keine weiteren
OCR-Lesungen des Auftrags. Ein laufender ONNX-Aufruf kann nicht abgebrochen
werden, blockiert aber nicht die Erkennung. Offline-OCR bleibt unverändert.
`--duration 15 --realtime` erlaubt begrenzte Live-Dateitests.

Zusätzlich rotieren im adaptiven Modus standardmäßig vier Suchkacheln pro
Update durch den vollständigen kalibrierten Bereich (`--ball-search-tiles 4`,
mit `0` deaktivierbar). Dies sucht auch abseits bekannter Spieler und Ballboxen.
Es ist eine unabhängige Suchabdeckung, **kein separates Ballmodell** und keine
Garantie einer bestimmten Ball-Abtastrate. Sie nutzt denselben Modell-Workerpool
und denselben Frame-Zeitstempel, wodurch keine verspäteten Ballpositionen in
aktuelle Spielsituationen gemischt werden. Zusätzliche Suche kostet Durchsatz.

`--workers 2` parallelisiert die Kachelinferenz über zwei Modellsitzungen.
Die Oberfläche bietet unter Analyse und Live 1–4 Worker an; bestehende Projekte
bleiben bei 1, bis man die Einstellung ändert. Mehr Worker benötigen mehr RAM
und bringen nicht auf jeder Hardware einen Vorteil. Die NMS-Eingabereihenfolge
bleibt unabhängig von der Workerzahl nach Kacheln geordnet.

Vergleich am M4 Pro mit dem Projektmodell: zwei Stichproben (7,5 und 22,5 s),
35 Auto-ROI-Kacheln, anschließend drei warme Wandzeitmessungen am letzten Bild:
ein CoreML-Worker **651 ms**, zwei **518 ms** (jeweils Median, rund 20 % weniger
Wandzeit). Das ist ein Inferenzbenchmark, keine Aussage zur Nummernqualität.

Kombinierter 15-Sekunden-Smoke-Test mit zwei Workern, Recovery-Budget 4 und
Hintergrund-OCR (`--ocr-min-h 20`, absichtlich niedrig zum Testen der Übergabe):
82 Updates, 5,64 Updates/s, 189 ms mittleres Bildalter, im Mittel 11,02 Kacheln.
38 Updates enthielten Ballkandidaten; Ballstatus `unknown` und `loose`. Keine
akzeptierte Trikotnummer, keine gemeldeten OCR-Fehler. Ballkandidaten sind keine
bestätigten richtigen Ballpositionen. Das ist **kein** direkter OCR-Speedvergleich
zum früheren Lauf: Suchbudget, Workerzahl und OCR-Einstellung wurden verändert.

Ein 30-Sekunden-Puffer, synchrone verzögerte Videoausgabe, ein separates
Ballmodell sowie hardwarebeschleunigtes Dekodieren sind hier **nicht** umgesetzt.
Ein Puffer allein löst dauerhaft zu geringen Durchsatz nicht. Der Live-Modus
priorisiert aktuelle Ergebnisse und überspringt dafür Quellbilder.

Eigenständige Rust-Implementierung unter der Projektlizenz AGPL-3.0-only;
keine zusätzlichen Bibliotheken oder übernommenen Fremdquellen.

## Vorläufige Messung vom 8. Oktober 2026 (vor zusätzlicher Ballsuche)

M4 Pro, CoreML, 15 Sekunden derselben Aufnahme (`stitch.mp4`), native
2560×1440, Modell Basketball Small, Projekt-`scales.json`, Auto-Kacheln,
ROI `0.12,0.38,0.86,0.73`, `--realtime`, ohne OCR oder MP4-Ausgabe.
Modellinitialisierung ist nicht in den Updatezeiten enthalten.

| Messwert | Vollsuche | Adaptiv |
|---|---:|---:|
| Updates/s über die Messspanne | 1,39 | 12,12 |
| Kacheln je Update, Mittel | 35 | 4,23 |
| Dekodiertes Bild → Ergebnis, Mittel | 745 ms | 95 ms |
| Dekodiertes Bild → Ergebnis, 95. Perzentil | 890 ms | 670 ms |
| Updates / Vollsuchen | 21 / 21 | 175 / 11 |
| Übersprungene Quellbilder | 879 | 725 |

**Qualitätsvorbehalt:** Adaptiv blieb der Ballstatus durchgehend `unknown`;
im Vergleich gab es auch `loose`. Ohne annotierte Referenz ist unklar, ob dort
ein echter Ball oder ein Fehlalarm vorlag. Gleiche Ballqualität ist damit nicht
nachgewiesen. Die erfassten Spielerpositionen sind ebenfalls nicht manuell
validiert. Zeitbasierte Live-Abtastung führt zu unterschiedlichen Analyseframes.
Dies ist ein kurzer Durchsatztest, keine Freigabe für verlässliche Spielanalyse
oder Beleg für lückenlose Echtzeitverarbeitung. Die Option bleibt standardmäßig
deaktiviert. Ein erster Test mit einer anderen `scales.json` wurde verworfen.
