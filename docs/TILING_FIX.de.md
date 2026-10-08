# Vollständige automatische Kachelplanung

## Korrektur vom 8. Oktober 2026

`plan_auto` begrenzte die Planung auf sechs Entfernungszonen. Bei steiler
Größenkalibrierung blieb dadurch die untere Spielfeldhälfte teilweise ungesucht.
Die Planung läuft jetzt bis zum angegebenen Ende. Ein Mindestfortschritt von
zehn Pixeln verhindert endlose Schleifen ohne eine feste Zonenzahl.

`plan_auto_roi` unterstützt zusätzlich einen expliziten Suchbereich. Der CLI-
Parameter `--roi x0,y0,x1,y1` wird nun auch mit `--tile auto` berücksichtigt.
Koordinaten müssen endlich, geordnet und im Bereich 0–1 liegen. Ungültige Daten
oder ein leerer Auto-Suchbereich erzeugen einen Fehler. Kleine Restbereiche
unter zehn Pixeln führen nicht mehr zu einem ungültigen `clamp`-Intervall.

Keine automatische Ableitung aus Basketballmaßen: Suchbereich und Skalierung
gehören zur Aufnahme, nicht zur Sportart. Der Bereich muss Platz für Körper,
Einwürfe und hohe Ballflüge enthalten. Die Standardeinstellung bleibt das ganze
Bild; die Oberfläche setzt noch keinen projektspezifischen ROI automatisch.

## Messung an der bereitgestellten Aufnahme

M4 Pro, CoreML, Release-Build, 2560×1440. Zwei Stichproben bei 7,5 und 22,5 s.
Unverändertes Modell und unveränderte Erkennungsschwellen:

| Korrigierte Planung | Kacheln | Modell-Wandzeit/Bild | Spieler je Stichprobe | Ballkandidaten |
|---|---:|---:|---|---|
| Gesamte Breite | 44 | 833 ms | 16 / 21 | 2 / 1 |
| ROI `0.12,0.38,0.86,0.73` | 35 | 663 ms | 16 / 21 | 1 / 1 |

Der ROI spart gegenüber der **vollständigen** Planung etwa 20 % Modellzeit.
Ein anschließender Fünf-Sekunden-Lauf mit ROI, `--every 15`, ohne OCR und mit
MP4-/JSON-Ausgabe verarbeitete 20 Analysebilder in 13,5 Sekunden (1,49 Bilder/s).
Die alte fehlerhafte Planung hatte ebenfalls 35 Kacheln, ließ aber unten Bereiche
aus. Gleiche Kachelzahl bedeutet deshalb nicht gleiche Abdeckung.

Diese Messungen sind keine Precision-/Recall- oder Identitätsbewertung. Hohe
Personenzahlen und fehlende Schiedsrichterklassifikation bleiben offene Probleme.
Der ROI ist auf diese Aufnahme zugeschnitten und darf nicht pauschal auf andere
Videos oder Sportarten übertragen werden. CoreML und geringere zeitliche
Abtastung bleiben die größeren Geschwindigkeitshebel.

## Prüfung

`cargo test --features tools --all-targets` enthält Regressionstests mit dem
steilen Kalibrierprofil der Aufnahme: lückenlose Abdeckung, ROI-Grenzen,
kleine Restbänder, ungültige Parameter und leere Bilder.

Ein separater CoreML-Release-Build wurde getestet; das installierte Programm
und die bestehenden Projekteinstellungen wurden nicht ersetzt.

Eigenständige Rust-Korrektur, keine neuen Abhängigkeiten; AGPL-3.0-only.
