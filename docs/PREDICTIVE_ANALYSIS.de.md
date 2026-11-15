# Prädiktive Analyse und reproduzierbarer Qualitätsvergleich

Diese Erweiterung baut auf 0.15.0 auf. Das Ziel ist eine Vollanalyse in höchstens
20 % der bisherigen Zeit bei mindestens gleicher Qualität. Das ist ein
**Abnahmekriterium, kein bereits gemessenes Ergebnis**.

## Vorhandenes Modell weiterverwenden

```sh
cargo build --release --features tools,coreml
target/release/reco-minimap run spiel.mp4 --model model --court court.json \
  --teams teams.json --scales scales.json --tile auto --ep coreml \
  --adaptive --every 2 --json lauf.jsonl --dets evidenz.jsonl --metrics zeit.json
```

`--adaptive` und `--adaptive-live` benutzen jetzt Bildbewegung und wachsende
Unsicherheitsbereiche. Die Suche deckt die letzte Position und die vorhergesagte
Position ab. Personen bleiben nach fehlenden Beobachtungen bis zu einer Sekunde
Suchziele, Bälle bis zu 0,4 Sekunden. Diese Suchhypothesen sind keine zusätzlichen
Detektionen und keine Personenidentitäten. Regelmäßige Vollsuchen sowie rotierende
Recovery-Kacheln bleiben aktiv. Rücksprünge und Lücken über drei Sekunden löschen
den Bewegungskontext.

Die Suche mit nur einem Modell bleibt auf den geplanten ROI begrenzt. Vollsuchen
erweitern diesen nicht: hohe Ballflüge müssen bei der ROI-Wahl berücksichtigt
werden. Mehrere Spieler im selben Bereich, viele Fehlalarme oder große
Unsicherheit können weiterhin fast alle Kacheln erfordern.

Die Vorverarbeitung verwendet höchstens acht Threads und wiederverwendbare
Tensorpuffer. Normalisierung, Abtastung und Kachel-NMS-Reihenfolge bleiben erhalten.

In der Oberfläche findest du unter **Erweiterte Analyse** die Wahl eines
separaten Ballmodells und die maximale Personenfrequenz. Unter Analyse lässt
sich dort auch das Speichern der Erkennungsdaten samt Laufzeitmessung aktivieren.
Die vorhandenen Modelle werden angeboten; es wird kein Modell automatisch geladen.

## Optional getrennte Frequenzen für Personen und Ball

```sh
target/release/reco-minimap run spiel.mp4 --model personenmodell \
  --ball-model ballmodell --person-hz 10 --ball-full-secs 1 \
  --ball-recovery-tiles 2 --ball-workers 1 --ball-batch 4 \
  --ball-model-threshold 0.2 --court court.json --teams teams.json \
  --tile auto --scales scales.json --ep coreml --adaptive \
  --every 1 --json lauf.jsonl --dets evidenz.jsonl --metrics zeit.json
```

Das optionale Ballmodell muss bereits lokal vorliegen, dem bestehenden RF-DETR-
oder YOLO-End-to-end-Vertrag entsprechen und eine korrekt gemappte Ballklasse
besitzen. Keine neuen Gewichte werden automatisch heruntergeladen oder trainiert.
`--person-hz` ohne Ballmodell wird abgelehnt. Standard ohne Ballmodell: bisherige
gemeinsame Detektionsfrequenz. Die Option funktioniert auch bei `live`.

Das Personenmodell läuft höchstens mit der gewählten Frequenz, bei kürzlich
verlorenen Tracks gegebenenfalls früher. Das Ballmodell untersucht bei jedem
**analysierten** Zeitstempel native Bildausschnitte. Unabhängige Vollsuchen decken
das ganze Bild einschließlich hoher Flüge ab; dazwischen helfen letzte
Ballpositionen, Handbereiche und rotierende Suchkacheln. `--every` begrenzt beide
Frequenzen: aus sechs analysierten Bildern pro Sekunde werden keine 60 Ballmessungen.

Primäre Ballkandidaten bleiben zusätzlich erhalten und werden per NMS mit dem
zweiten Modell zusammengeführt. Neue Modellaufrufe ersetzen keine menschliche
Validierung. Ein großes zweites Modell kann erheblich **langsamer** sein; für
einen Durchsatzgewinn ist ein geeignetes kleines Ballmodell erforderlich.

JSON `performance.people_observed`, `person_tiles` und `ball_tiles` unterscheiden
echte Personenmessungen von Zwischenbildern. Der Tracker sagt in Zwischenbildern
Positionen zeitbasiert voraus; alte Boxen werden nicht als neue Messung eingespeist.

## OCR und Cache

Offline-OCR priorisiert kontrastreiche Torsoausschnitte mit Bildkanten statt nur
großer Boxen. Flache Ausschnitte werden verworfen. Unbekannte Nummern werden
frühestens nach 0,25 s erneut gelesen, Kandidaten nach 0,8 s, gefestigte Nummern
nach 2 s. Die Zeiten sind Heuristiken und müssen mit echten Trikotbildern geprüft
werden. Live-OCR behält seinen vorhandenen begrenzten Hintergrundworker.

Ballkontakte, Freigaben und Flugbestätigungen benötigen weiterhin mehrere
Beobachtungen und standardmäßig mindestens 20 ms zusammenhängende Evidenz.
Fehlende Beobachtungen oder Abstände über 0,5 s löschen laufende Zähler.
Eine konfigurierte Ein-Beobachtungs-Bestätigung bleibt sofortig. Diese Grenzen
sind abstimmbare Startwerte, keine sportwissenschaftlich kalibrierten Konstanten.

`--dets` bleibt im bisherigen JSONL-Grundformat lesbar. Der zusätzliche
`evidence_schema: 1` dokumentiert Modell-SHA256, Kacheln, Suchparameter, getrenntes
Ballmodell, OCR-Modellhash, Kader und Detektionsschwellen. Zeilen enthalten die
OCR-Wahrscheinlichkeiten und Informationen über die Messfrequenzen. Alte Caches
ohne Zusatzdaten bleiben abspielbar; unbekannte zukünftige Evidenzversionen
werden abgelehnt.

```sh
target/release/reco-minimap replay evidenz.jsonl --court court.json \
  --teams teams.json --json neu.jsonl --scenes
```

Replay braucht für die aufgezeichnete OCR kein OCR-Modell. Es benutzt die
aufgezeichneten Schwellen und Kader, sofern keine Überschreibung angegeben wird.
OCR wird nur bei identischem sichtbarem Slot, Segment, Team und Bildausschnitt
übernommen. Andere Trackingparameter, ein neuer Startzeitpunkt oder Teamfarben
können Evidenz deshalb bewusst verwerfen. Angewandte und verworfene Lesungen
werden gezählt. Es wird keine Identitätsgleichheit über Trackabbrüche behauptet.

Die vorhandene Nummernergänzung entlang eines kontinuierlichen Tracklets läuft
nach `run` und entsprechendem Replay. `--no-backfill` deaktiviert sie. Ungültige
JSON-Zeilen werden bei der Nachbearbeitung jetzt als Fehler gemeldet, statt beim
Neuschreiben still zu verschwinden. Neue automatische Verknüpfung verschiedener
Tracklets, Re-ID-Netze, 3D-Ballrekonstruktion und bildbasierte Trefferbestätigung
sind mit dieser Erweiterung nicht implementiert.

## Laufzeit messen

```sh
python3 scripts/benchmark_analysis.py --binary target/release/reco-minimap \
  --out vergleich --repeats 3 -- spiel.mp4 --model model --court court.json \
  --teams teams.json --scales scales.json --tile auto --ep coreml \
  --start 30 --duration 60 --every 2 --numbers --scenes
```

Das Skript vergleicht Vollsuche und adaptive Suche mit demselben aktuellen Binary.
Es wechselt die Reihenfolge, misst den vollständigen Prozess einschließlich
Modellstart, OCR und Nachbearbeitung und prüft gleiche Bildzahlen und Zeitspannen.
Es ersetzt keinen Vergleich mit einem älteren Binary. Der Test schreibt nur JSON;
Videoexporte benötigen einen zusätzlichen Vergleich mit identischen Ausgaben.

`--metrics` meldet auch Initialisierung, Dekodierung, Vorverarbeitung, Inferenz,
OCR, Tracking und Ausgabe. Bei zwei Modellen werden Ballinferenz und Kachelzahlen
separat ausgewiesen. Nicht überlappende Phasenzeiten sind keine Hardwareauslastung.
Der vollständige Prozess wird vom Python-Skript einschließlich Dateischluss gemessen.

Erster kurzer Smoke-Test vom 15.11.2026: M4 Pro, Basketball-Small, zwei CoreML-
GPU-Worker, NBV-Panorama 3840×1480, 34 Kacheln, zwei Videosekunden, jedes zehnte
Bild, ohne OCR/Videoexport, eine Wiederholung. Vollsuche 53,08 s, adaptiv 48,82 s
einschließlich Prozessstart: **1,09×**, nicht 5×. Dies ist kein belastbarer
Langzeitbenchmark und enthält keine manuelle Qualitätsbewertung. Die optionale
Zwei-Modell-Architektur wurde bei dieser Messung nicht verwendet.

## Qualität gegen manuelle Referenz prüfen

JSONL-Beispiel für sparsame Annotationen, mit den tatsächlichen Analysezeitstempeln:

```json
{"t":30.0,"players":[{"id":"home-4","team":"home","number":4,"x":8.2,"y":6.0}],"ball":{"state":"held","x":8.2,"y":6.0,"holder_player_id":"home-4"}}
{"t":30.2,"players":[],"ball":{"present":false}}
{"t":31.0,"event_window":[30.0,35.0],"events":[{"t":31.0,"type":"scene","kind":"shot3","team":"home"}]}
```

`players`, sofern vorhanden, muss alle im betrachteten Courtabschnitt relevanten
Spieler enthalten. Leere Listen sind explizite Negativbeispiele. Nicht annotierte
Felder werden nicht bewertet. Für frei fliegende Bälle dürfen reale Bodenpunkte
nicht mit der homographisch projizierten Ballposition verwechselt werden; dort
zunächst Ballstatus und Ereignisse annotieren. `present:false` bedeutet, dass im
Analyseergebnis keine Ballposition behauptet werden soll, nicht bloß unsichtbarer Ball.

```sh
python3 scripts/evaluate_analysis.py --reference referenz.jsonl \
  --baseline vergleich/full-1.jsonl --candidate vergleich/adaptive-1.jsonl \
  --out qualitaet.json
```

Die Auswertung verwendet globale Positionszuordnung, standardmäßig 2 m Distanz-
und 20 ms Zeitfenster. Fehlende Analysebilder zählen als Fehler. Gemeldet werden
Spielerwiederfindung, Positionsabweichungen, korrekte Nummern, ID-Wechsel zwischen
nahen annotierten Frames, Ballstatus, Ballpositionen und individueller Besitz.
Für Ereignisse müssen vollständige `event_window`-Intervalle annotiert sein;
Zuordnung erfolgt einzeln mit einer Sekunde Toleranz. Außerhalb dieser Fenster
werden keine Ereignis-Fehlalarme behauptet. Das sind eigene operative Metriken,
keine HOTA/MOTA-Implementierung. Sparse Labels übersehen Fehler zwischen den Labels.

Für eine Freigabe müssen sowohl der Zeitfaktor als auch festgelegte Qualitätswerte
auf mehreren repräsentativen Sequenzen bestehen. Fehlende Annotationen werden
nicht als erfolgreiche Qualitätsprüfung ausgegeben.
