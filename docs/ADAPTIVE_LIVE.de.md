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

Optionale Trikot-OCR wird im Live-Modus höchstens alle zwei Sekunden versucht
(`--ocr-secs`), und nur wenn das Bild nach der Erkennung jünger als eine Sekunde
ist. Ein einzelner OCR-Aufruf hat noch keine harte Laufzeitgrenze. Offline-OCR
bleibt unverändert. `--duration 15 --realtime` erlaubt begrenzte Live-Dateitests.

Ein 30-Sekunden-Puffer, synchrone verzögerte Videoausgabe, ein separates
Ballmodell sowie hardwarebeschleunigtes Dekodieren sind hier **nicht** umgesetzt.
Ein Puffer allein löst dauerhaft zu geringen Durchsatz nicht. Der Live-Modus
priorisiert aktuelle Ergebnisse und überspringt dafür Quellbilder.

Eigenständige Rust-Implementierung unter der Projektlizenz AGPL-3.0-only;
keine zusätzlichen Bibliotheken oder übernommenen Fremdquellen.

## Vorläufige Messung vom 8. Oktober 2026

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
