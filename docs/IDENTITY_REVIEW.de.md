# Kader und manuelle Identitätskorrektur

Der Befehl `review-identities` ergänzt Namen und zeitlich begrenzte Zuordnungen
in einer vorhandenen JSONL-Analyse. Er läuft lokal, ohne Modellinferenz. Er ist
noch kein grafischer Editor und erzeugt kein neu gerendertes Video.

## Vorgehen

1. Analyse mit der aktuellen Version erzeugen; Spieler benötigen `id` und `segment`.
2. Prüfsumme ermitteln, unter macOS z. B. `shasum -a 256 states.jsonl`.
3. `review.json` erstellen und den Platzhalter durch die Prüfsumme ersetzen:

```json
{
  "schema_version": 1,
  "source_sha256": "HIER_DIE_64_STELLIGE_SHA256",
  "roster": [
    {"id": "home-7", "name": "Alex", "team": "home", "number": 7},
    {"id": "away-7", "name": "Sam", "team": "away", "number": 7}
  ],
  "corrections": [
    {"slot": 3, "segment": 1, "start": 10.0, "end": 20.0, "player_id": "home-7"},
    {"slot": 3, "segment": 1, "start": 20.0, "end": 22.0, "player_id": null}
  ]
}
```

```sh
reco-minimap review-identities states.jsonl --annotations review.json --out reviewed.jsonl
```

Die Zeiten sind Sekunden der Analyse: Start inklusive, Ende exklusive. `null`
bedeutet ausdrücklich unbekannt. Leere `corrections` erlauben reine Kaderanreicherung.
Nummern sind derzeit auf 0–99 beschränkt (wie die bestehende OCR); `00` und `0`
können noch nicht unterschieden werden. Namen und IDs sind frei wählbar, IDs aber
nur innerhalb dieser Analyse gültig. Mannschaften sind `home` und `away`.

## Sicherheit und Bedeutung

- Original und Korrekturdatei bleiben unverändert; bestehende Ausgaben werden
  nicht überschrieben. Bei Fehlern wird die neu erzeugte unvollständige Ausgabe
  entfernt. Die Verarbeitung hält nur jeweils einen Frame im Speicher.
- Die SHA-256 bindet Korrekturen an die genaue Eingabedatei. Vor der Bearbeitung
  muss eine laufende Analyse abgeschlossen sein; Eingabe währenddessen nicht ändern.
- Eine Korrektur gilt nur für den angegebenen Tracking-Abschnitt. Überlappende
  Korrekturen desselben Abschnitts, unbekannte Kader-IDs und doppelte Mannschafts-
  nummern werden abgelehnt. Korrekturen ohne passenden Frame führen zu einem Fehler.
- Eine Identitätskorrektur verändert nicht die Mannschaftserkennung. Bei abweichender
  Mannschaft wird abgebrochen; eine Team-Korrektur ist noch nicht implementiert.
- Manuelle Zuordnungen erhalten `identity.status: manual`. Kadernamen allein machen
  aus einem automatischen `candidate` keine sichere Identität. Gleichzeitige Ansprüche
  auf denselben Spieler werden weiterhin als `conflict` markiert, auch manuelle.
- `identity_before_review` bewahrt ursprüngliche Identität und Trikotnummer.
  `identity_review` enthält Quell- und Korrekturdateiprüfsumme. Die manuellen Einträge
  bewahren Zeitraum und Herkunft. Für neue Revisionen immer die Originalanalyse
  verwenden, nicht bereits korrigierte Ausgaben.
- Ballbesitz, Positionen und Ereignisse bleiben unverändert. Eine Identitätskorrektur
  bestätigt insbesondere keinen automatisch erkannten Spielerwechsel.

## Grafische Review-Ansicht

`reco-minimap ui` starten und oben **Review** öffnen (Route `/review`).
Video und Original-JSONL über die Dateiauswahl laden. Die Dateien bleiben im
Browser; sie werden nicht hochgeladen. Sportart und Zeitversatz explizit prüfen.
Die FieldMap übernimmt Linien und Maße aus den Rust-Sportprofilen. Sie skaliert
proportional; Fenstergröße verändert keine gespeicherten Feldkoordinaten.

Die gemeinsame Zeitleiste folgt der Videozeit. Ereignisse lassen sich nach Typ
filtern und springen mit zwei Sekunden Vorlauf ins Video. Ein Klick auf einen
Spieler **auf der Karte** pausiert das Video und wählt Platz und Abschnitt aus.
Kader hinzufügen, Zeitraum und Spieler wählen, dann **Zuordnen**. Die Karte zeigt
die Korrektur als Vorschau; Konflikte bleiben rot und ohne sichere Nummer.

**review.json herunterladen** speichert das mit dem CLI kompatible Dokument.
Falls der eingebettete Browser Downloads blockiert, den sichtbaren Exporttext
kopieren und als `review.json` speichern. Bestehende Review-Dateien können wieder
geöffnet werden; ihre Quellprüfsumme wird geprüft. Der Browser speichert Änderungen
nicht automatisch und warnt beim Verlassen vor möglicherweise ungesicherten Daten.

Grenzen dieser ersten Oberfläche: Deutsch, höchstens 64 MiB bzw. 100.000 Frames
im Speicher, höchstens 500 Ereignistreffer gleichzeitig, Standard-Sportfelder,
browserabhängige Videocodecs. Datenlücken über einer Sekunde zeigen keine Punkte.
Keine automatische Ermittlung des Zeitversatzes und keine projektgebundene
Videozuordnung. Korrektur gilt anhand der Framezeit. Kein Overlay im Panoramabild,
kein Neurendern und keine manuelle Ereigniskorrektur. Die CLI bleibt der Weg zur
dauerhaft korrigierten JSONL-Datei.

Prüfung: `node --test scripts/review_test.js` plus Rust-Suite. Ein synthetischer
Browser-Smoke-Test mit Testvideo und `tests/fixtures/review-demo.jsonl` prüft Laden,
Zeitleiste, Ereignissprung, Kartenklick, Kader, Zuordnung und JSON-Exportvorschau.
Dies ersetzt keinen Genauigkeits- oder Langzeittest mit realen Spielen.

Keine neuen Modelle, Fremdcode oder Abhängigkeiten; weiterhin AGPL-3.0-only.
