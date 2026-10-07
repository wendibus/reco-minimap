# Sportübergreifende Analysearchitektur

## Umgesetzter erster Schritt

Die bestehenden Rust-Module bleiben in einem Crate. Eine Aufteilung in mehrere
Crates ist dafür noch nicht nötig. Kamera-/Panoramaprojektion (`geom::Projector`),
Detektion, Tracking und Ausgabe bleiben unabhängig von Sportregeln.

- `sport.rs`: Geometrie, Spielfeldmarkierungen, Referenzpunkte und Größen.
- `sport/rules.rs`: explizite Auswahl von Szenen- und Besitzregeln.
- `scenes.rs`: gemeinsames Ereignisformat, zeitliche Bestätigung und Wiederfreigabe.
- `scenes/basketball.rs`: Basketball-Freiwürfe und Wurfversuche.
- `scenes/formations.rs`: übrige bisherige Formationsheuristiken mit expliziter Auswahl.
- `ball.rs`: Ballverfolgung mit konfigurierbarer Interpretation von Handkontrolle.

Die bisherigen Konstruktoren bleiben verfügbar. `Config::for_sport` und
`SceneDetector::new` wählen eingebaute Regeln anhand des Profilnamens.
Für eigene Profile sind `Config::for_sport_with_rules` und
`SceneDetector::with_rules` zu verwenden; beide erhalten dieselbe Regelkonfiguration.
So muss eine individuell vermessene Halle nicht anhand ihrer Maße erraten werden.

## Bewusste Verhaltensänderungen

Volleyball, American Football und unbekannte Sportarten erben keine generischen
Einwurf-Szenen mehr. Ohne implementierte Szenenregeln werden keine Szenen erzeugt;
Video, Tracking und Karte funktionieren weiter.

Die bisherige Überlappung von Ball und Spielerbox wird nur noch bei Basketball,
Handball, Rugby und American Football als Kandidat für Handkontrolle verwendet.
Fuß-/Schlägerkontrolle und Volleyballkontakte benötigen eigene Evidenzmodelle.
Bis dahin bleibt individueller Besitz dort unbestimmt; freie Ballpositionen werden
weiter verfolgt. Dies ist eine konservative Einschränkung, keine neue Erkennung
dieser Sportarten.

Fehlt der Ball länger als standardmäßig 1,5 Sekunden, wird angenommene Handkontrolle
unbekannt. Kurze Verdeckungen bleiben überbrückt. Der Ablauf wird in Videozeit
gemessen, nicht in analysierten Frames. Der Wert ist ein abstimmbarer Startwert,
keine sportwissenschaftlich validierte Grenze. Ablauf allein bedeutet keinen Wurf.

## Nächste Ausbaustufen – noch nicht implementiert

1. Tracking-ID, dauerhafte Spieleridentität und zeitabhängige Zuordnung trennen.
   Feste Team-Slots sind derzeit weiterhin vorhanden und keine Identitätsgarantie.
2. Mannschaftsbesitz, individuelle Kontrolle und Kontakte getrennt modellieren.
3. Ereignisse um Intervalle, Beteiligte, Evidenz und manuelle Korrekturen erweitern.
4. Synchronen Analyseplayer mit Ereignisnavigation und wiederverwendbaren
   Detektionsdaten aufbauen; Modellaufrufe bei reiner Neukalibrierung vermeiden.
5. Sportmodule mit realen annotierten Sequenzen prüfen und danach gezielt
   Verarbeitungskosten optimieren.

Spielregeln sind nicht aus Feldgeometrie abzuleiten. Höhenpunkte (z. B. Korbringe)
sind keine Bodenreferenzen. Modellfähigkeiten und Klassenmapping bleiben von
Sportprofilen getrennt. Fehlende Fähigkeiten sollen einzelne Analysen deaktivieren.

## Prüfung und Lizenz

Regressionstests prüfen bestehende Szenen, das Ausbleiben sportfremder Ereignisse,
explizite Regeln für eigene Profile sowie Ballbesitzablauf bei verschiedenen
Abtastraten. Synthetische Tests belegen keine Genauigkeit in realen Spielen.

Diese Änderung ist eigenständig implementiert, ohne übernommenen Fremdcode oder
neue Modellgewichte und ohne zusätzliche Abhängigkeiten. Es gilt weiterhin die
AGPL-3.0-only des Repositories. Bei künftigen Integrationen sind Code-, Modell- und
Datensatzlizenzen getrennt mit Quelle und Version zu dokumentieren.
