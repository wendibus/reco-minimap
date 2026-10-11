# Spielsituationen („Szenen“) je Sportart

reco-minimap erkennt **Aufstellungen bei Spielunterbrechungen** – Neustarts und Standardsituationen, bei denen Spieler an charakteristischen Stellen stehen und sich kaum bewegen – und im Basketball **Wurfversuche**. Diese Seite listet für jede Sportart, welche Situationen leicht erkennbar sind, woran sie erkannt werden und wovon das abhängt.

Einschalten mit `--scenes` (Kommandozeile) oder dem Häkchen *Spielsituationen erkennen* in Schritt 4 / 5 der Oberfläche.

**Legende**

| | |
|---|---|
| ✅ | **Umgesetzt.** Regelbasiert auf den getrackten Positionen, an gestellten Szenen getestet. |
| 🟡 | **Umgesetzt, hängt vom Ball ab.** Funktioniert nur, wenn der Ball im richtigen Moment gefunden wird. |
| 🔧 | **Machbar, noch nicht umgesetzt.** Das Muster ist einfach; es muss geschrieben und abgestimmt werden. |
| ❌ | **Aus Positionen nicht erkennbar.** Braucht Anzeigetafel, Schiedsrichter-Zeichen oder Regelwissen. |

> **Ehrlicher Stand:** Alle ✅- und 🟡-Szenen wurden **nur an gestellten Aufstellungen** geprüft (positive Fälle und die typischen Verwechslungen, bei denen nichts auslösen darf). Sie sind nicht an echten Spielen abgestimmt. Rechne damit, Schwellen anzupassen, und mit Fehlalarmen bei den schwächeren (unten markiert).

## Deine Prioritätenliste

| Sportart | Situation | Stand |
|---|---|---|
| Basketball | Einwurf (Inbound) | ✅ |
| Basketball | Freiwurf | ✅ |
| Basketball | Wechsel | ✅ (Nummern oder Verlassen und Wiederbetreten an einer Seitenlinie) |
| Basketball | Startball (Sprungball) | ✅ |
| Basketball | 2er und 3er | 🟡 (braucht den Ball, der die Hände des Werfers verlässt; nicht, ob er getroffen hat) |
| Fußball | Anstoß | ✅ |
| Fußball | Wechsel | ✅ |
| Fußball | Elfmeter | ✅ |
| Fußball | Ecke | ✅ |
| Handball | Wechsel | ✅ |
| Handball | Einwurf | ✅ |
| Handball | 7 Meter | ✅ |

---

## So funktioniert es

1. Bei jedem verarbeiteten Bild schaut der Detektor auf die getrackten Spieler (Positionen in Metern, Geschwindigkeiten).
2. Eine **Aufstellung** ist ein Muster wie „ein Spieler steht an der Freiwurflinie, drei oder mehr in der Zone, fast alle stehen still“.
3. Das Muster muss **kurz bestehen** (1–2 s, je Szene), bevor das Ereignis auslöst, und es **löst einmal** pro Aufstellung aus. Erneut auslösen kann es erst, wenn sich die Aufstellung aufgelöst hat und eine Sperrzeit (6 s) vorbei ist.
4. Das Ereignis sagt, was, wo, welches Team ausführt, und mit welcher Sicherheit.

**Was dafür nötig ist**

* Eine Kalibrierung mit etwa **0,5 m Fehler oder weniger**. Die Toleranzen betragen 1–2 m.
* Die meisten Spieler getrackt (etwa 60 % oder mehr). Fehlt die halbe Mannschaft, wird die Aufstellung nicht erkannt.
* Bei ballabhängigen Szenen ein Ball, der im Moment des Abwurfs verfolgt wird.

**Typische Fehlerbilder**

* *Verpasst:* Spieler nicht erkannt (ferne Seite, kleines Bild), schlechte Kalibrierung, eine Aufstellung, die kürzer als die Haltezeit dauert.
* *Fehlalarm:* Ein Spieler, der im laufenden Spiel am Seitenaus still steht, sieht aus wie ein Einwerfer – deshalb haben Einwürfe nur Sicherheit 0,5, außer der Ball ist in seinen Händen (0,8). Ein still stehender Torwart ist für sich keine Szene; deshalb ist der Abstoß nicht umgesetzt.

## Ausgabe

Szenen erscheinen an drei Stellen:

* **Protokoll:** `SITUATION Freiwurf links (Heim) bei 12:34 (Sicherheit 0.85)` und eine Zusammenfassung am Ende eines Laufs. Der Wortlaut folgt der gewählten Sprache.
* **JSON** (`--json`): im Feld `events` des Bildes, in dem die Szene ausgelöst hat:

```json
{"type": "scene", "kind": "free_throw", "team": "home", "side": "l",
 "x": 5.8, "y": 7.5, "confidence": 0.85, "borderline": false, "t": 754.2}
```

`kind` ist eines von `tip_off`, `kickoff`, `free_throw`, `throw_in`, `corner`, `penalty`, `seven_meter`, `shot2`, `shot3`. `side` ist `l` oder `r` (das Feldende, zu dem die Szene gehört) oder `null`. `team` ist das Team, das den Neustart oder Wurf ausführt. `borderline` ist wahr bei Wurfversuchen, die innerhalb des Positionsfehlers der 3-Punkte-Linie abgegeben wurden.

* **Live-Overlay:** ein Banner für einige Sekunden, in der Sprache der Oberfläche.
* **`reco-minimap stats`:** listet jede Szene mit Zeit und Sicherheit.

---

## Basketball

| Situation | Stand | Woran erkannt | Hinweise |
|---|---|---|---|
| **Startball / Sprungball** | ✅ | Zwei Gegner im Mittelkreis (≤ 1,5 m), dicht beieinander, still; sonst niemand näher als 2,4 m am Mittelpunkt; die übrigen (≥ 6 getrackt) überwiegend ruhig. Hält 1 s. | Ein dritter Spieler im Kreis (Gerangel) hebt es auf. |
| **Freiwurf** | ✅ | Ein Spieler steht still an der Freiwurflinie (≤ 1,1 m), ≥ 3 Spieler in der Zone, höchstens jeder fünfte Spieler schnell in Bewegung. Hält 2 s. | Meldet das Feldende (`l`/`r`) und das Team des Werfers. |
| **Einwurf (Inbound)** | ✅ | Ein Spieler steht still an Seiten- oder Grundlinie (≤ 0,8 m vom Rand, innen oder knapp außerhalb), niemand sonst innerhalb von 1,2 m, ≥ 3 Spieler innerhalb von 9 m. Hält 1,2 s. | Sicherheit 0,8, wenn der Ball in seinen Händen oder daneben ist, sonst 0,5. Ein Spieler, der im Spiel an der Linie steht, kann es auslösen. |
| **Wechsel** | ✅ | Die bestätigte Trikotnummer eines Platzes ändert sich, oder ein Spieler, der zuletzt an einer Seitenlinie verfolgt wurde, ist mindestens 3 s weg und jemand taucht an einer Seitenlinie auf („möglich“). Siehe [README](../README.md#jersey-numbers-and-substitutions). | Am stärksten mit Nummern und Kaderliste. |
| **2er- oder 3er-Versuch** | 🟡 | Der Ball verlässt die Hände des Ballhalters (Zustand *gehalten* → *Flug*) und fliegt auf einen Korb zu (endet ≥ 2 m näher oder innerhalb von 2,5 m). Eingeordnet nach der Position des Werfers beim Abwurf: ≥ 6,8 m vom Korb oder im Eckstreifen zählt als 3. Freiwurf-Abwürfe sind ausgenommen. | `borderline: true`, wenn der Abwurf innerhalb von 0,8 m der Bogenlinie liegt – so groß ist etwa der Positionsfehler. Verpasst, wenn der Ball beim Abwurf nicht verfolgt wird; mit einem schwachen Ballmodell ist das häufig. |
| Treffer oder nicht | ❌ | Aus Positionen nicht sichtbar. | Braucht die Anzeigetafel. Auf einen Korb folgt meist ein Einwurf des *anderen* Teams an der Grundlinie – ein Hinweis, kein Beweis. |
| Auszeit | 🔧 | Alle stehen still und sammeln sich > 20 s auf der Bankseite. | |
| Fastbreak | 🔧 | Viele Spieler beider Teams überqueren schnell die Mittellinie in einer Richtung. | |
| Foul, Regelverstöße, Sprungball-Situation | ❌ | Regelwissen und Schiedsrichter-Zeichen. | |

## Fußball

| Situation | Stand | Woran erkannt | Hinweise |
|---|---|---|---|
| **Anstoß** | ✅ | Beide Teams in der eigenen Hälfte (je ≥ 85 %), alle stehen still, höchstens zwei Spieler eines Teams im Mittelkreis (9,15 m). Hält 1,5 s. | Das Team im Kreis führt aus. Löst zu Beginn jeder Halbzeit und nach einem Tor aus. |
| **Elfmeter** | ✅ | Ein Spieler steht still am Elfmeterpunkt (≤ 1,8 m), niemand sonst innerhalb von 8,5 m, höchstens ein weiterer Spieler (der Torwart) im Strafraum. Hält 2 s. | Meldet das Ende und das Team des Schützen. |
| **Ecke** | ✅ | Ein Spieler steht still an der Eckfahne (≤ 2,4 m), ≥ 5 Spieler im Strafraum an diesem Ende, die sich langsam bewegen. Hält 1,5 s. | |
| **Einwurf** | ✅ | Wie im Basketball, an der Seitenlinie, außerhalb der Eckzone (die Ecke übernimmt der Eck-Detektor). | Gleiche Sicherheitsregeln wie oben. |
| **Wechsel** | ✅ | Wie oben. Ein Einwechselspieler kommt meist an der Mittellinie nahe der Seitenlinie herein. | |
| Abstoß | 🔧 | Ausführender im Torraum, Gegner außerhalb des Strafraums. | Nicht umgesetzt: Ein still stehender Torwart sieht im normalen Spiel gleich aus. Braucht den Ball. |
| Freistoß (Mauer) | 🔧 | ≥ 3 Verteidiger in einer Linie etwa 10 m vom Ball, still. | |
| Tor | 🔧 | Ein Anstoß des *anderen* Teams folgt binnen einer Minute; die Anzeigetafel ist sicherer. | Ohne Uhr nicht vom Halbzeitbeginn zu unterscheiden. |
| Abseits, Foul, Karten | ❌ | | |

## Futsal

| Situation | Stand | Woran erkannt | Hinweise |
|---|---|---|---|
| **Anstoß** | ✅ | Wie im Fußball, Mittelkreis 3 m. | |
| **Einkick** | ✅ | Spieler steht still an der Seitenlinie (Einwurf-Detektor). | |
| **Ecke** | ✅ | Wie im Fußball, mit ≥ 3 Spielern im Strafraum. | |
| **Strafstoß (6 m)** | ✅ | Schütze am 6-m-Punkt (≤ 1,5 m), niemand innerhalb von 4,5 m, höchstens ein weiterer Spieler im Strafraum. | |
| **Wechsel** | ✅ | Wie oben. | Fliegende Wechsel sind häufig; erwarte viele *mögliche* Wechsel. |
| Zweiter Strafstoß (10 m) | 🔧 | | |
| Auszeit | 🔧 | | |

## Handball

| Situation | Stand | Woran erkannt | Hinweise |
|---|---|---|---|
| **Anwurf** | ✅ | Beide Teams in der eigenen Hälfte, still, der Ausführende in der Mitte. | In den Daten als „Anstoß“ (`kickoff`), in der Oberfläche als „Anwurf“. |
| **7-Meter** | ✅ | Ausführender steht still an der 7-m-Marke (≤ 1,2 m), niemand sonst innerhalb von 1,9 m, höchstens ein weiterer Spieler im 6-m-Raum. Hält 2 s. | |
| **Einwurf** | ✅ | Spieler steht still an der Seitenlinie. | Eckwürfe, an der Seitenlinie nahe der Ecke ausgeführt, sind eingeschlossen. |
| **Wechsel** | ✅ | Wie oben. | Fliegende Wechsel sind häufig; erwarte viele *mögliche* Wechsel. |
| Freiwurf | 🔧 | Eine Abwehrmauer aus ≥ 3 Spielern etwa 3 m vom Ausführenden. | |
| Abwurf | 🔧 | Torwart steht still im 6-m-Raum. | Braucht den Ball (gleiches Problem wie der Fußball-Abstoß). |
| 2-Minuten-Strafe | 🔧 | Ein Team spielt zwei Minuten mit einem Spieler weniger: getrackte Plätze des Teams zählen. | |
| Auszeit | 🔧 | | |

## Hockey (Feld)

| Situation | Stand | Woran erkannt | Hinweise |
|---|---|---|---|
| **Anspiel in der Mitte** | ✅ | Das Anstoß-Muster mit 5 m Mittelradius. | |
| **Strafstoß** | ✅ | Schütze am 6,4-m-Punkt (≤ 1,5 m), niemand innerhalb von 4,5 m, höchstens ein weiterer Spieler im Schusskreis. | |
| **Wechsel** | ✅ | Wie oben. | Rollierende Wechsel sind häufig. |
| Strafecke | 🔧 | Anspieler an der Torlinie, Angreifer außerhalb des Kreises, Verteidiger hinter der Linie. | |
| Lange Ecke, Freischlag | 🔧 | | |

## Rugby

| Situation | Stand | Woran erkannt | Hinweise |
|---|---|---|---|
| **Anstoß** | ✅ | Teams in der eigenen Hälfte, still, höchstens zwei Spieler eines Teams innerhalb von 10 m der Mitte. | |
| Gedränge | 🔧 | 14–16 Spieler gebunden in einem Kreis von etwa 4 m, kaum in Bewegung. | |
| Gasse (Lineout) | 🔧 | Zwei parallele Spielerreihen an der Seitenlinie. | |
| Straf- oder Erhöhungstritt | 🔧 | Ein einzelner Kicker, die anderen stehen zurück. | |
| Wechsel | 🔧 | Die Wechsel-Logik ist für 15 Plätze je Team abzustimmen. | |
| Versuch | ❌ | Braucht den abgelegten Ball und die Anzeigetafel. | |

## Lacrosse

| Situation | Stand | Woran erkannt | Hinweise |
|---|---|---|---|
| **Face-off** | ✅ | Zwei Gegner in der Mitte (≤ 1,2 m), die anderen stehen herum. | |
| Wechsel | 🔧 | Fliegende Wechsel über die Wechselzone. | |
| Tor, Überzahl | ❌ | | |

## American Football

| Situation | Stand | Woran erkannt | Hinweise |
|---|---|---|---|
| Scrimmage-Aufstellung / Snap | 🔧 | Zwei Reihen gegenüber in einem Band von etwa 10 Yards Breite, ≥ 5 s kaum in Bewegung. Ein klares Muster; ein guter nächster Schritt. | |
| Kick-off | 🔧 | Eine Reihe von Kickern quer über das Feld, Empfänger tief. | |
| Punt-, Field-Goal-Aufstellungen | 🔧 | | |
| Auszeit | 🔧 | | |
| Touchdown, First Down | ❌ | | |

## Volleyball

| Situation | Stand | Woran erkannt | Hinweise |
|---|---|---|---|
| Aufschlag | 🔧 | Ein Spieler allein hinter der Grundlinie, still. | |
| Wechsel | 🔧 | Spieler an der Wechselzone. | |
| Rotation | 🔧 | Positionswechsel im Uhrzeigersinn nach einem Seitenwechsel des Aufschlags. | |
| Punkt | ❌ | | |

## Andere Sportarten (freies Rechteck)

| Situation | Stand |
|---|---|
| Einwurf / Neustart an der Seitenlinie | ✅ |
| Wechsel | ✅ |
| Alles andere | ❌ (keine sportartspezifische Geometrie) |

---

## Für Entwickler: eine Szene ergänzen

Der Detektor liegt in [`src/scenes.rs`](../src/scenes.rs). Eine Szene ist eine Funktion, die die Spieler eines Bildes ansieht und einen Kandidaten ablegt (`kind`, `key`, `team`, `pos`, `side`, `confidence`). Der gemeinsame Zeitcode erledigt Haltezeit, einmaliges Auslösen, Sperrzeit und Wiederbereitschaft. Schritte:

1. Eine `SceneKind`-Variante samt `key()`-Text und `hold()`-Zeit ergänzen.
2. Die Musterfunktion schreiben (Vorlagen: `free_throws`, `corners`) und in `patterns()` für die passenden Sportarten aufrufen.
3. Einen Test in `tests/scenes.rs` ergänzen: eine positive Aufstellung und die nächste Verwechslung, bei der **nichts** auslösen darf.
4. Die Namen in `scripts/gen_i18n.py` (`scene.<kind>`) und in `scene_name` in `src/bin/reco-minimap.rs` in allen vier Sprachen ergänzen und das Skript ausführen.

Nützliche Bausteine: `profile.landmark("spot_l")`, `profile.zone_rect('l', 1)` (Strafraum / Zone), `profile.field.edge_dist(pos)`, `AgentOut.vel` und der Ballzustand im Bild.
