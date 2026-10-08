# Basketball: Aufstellung und zeitlicher Kontext

Mit `--scenes` werden Freiwurfaufstellung und nachfolgender Wurfversuch getrennt
gemeldet. Ein Werfer an der Linie genügt nicht: mindestens drei ruhende Spieler
müssen an beiden Zonenseiten warten, die Zonenmitte muss frei bleiben und die
Formation zwei Sekunden bestehen. Nur aktuell sichtbare Basketballtracks zählen.
Ein unbekannter Ball ist fehlende Evidenz, kein Nachweis einer Unterbrechung.
Ballflug oder Besitz eines anderen Spielers widerspricht der Aufstellung.

Eine beobachtete Freigabe Richtung Korb nahe der Freiwurflinie wird nur dann als
Freiwurfversuch bezeichnet, wenn am selben Korb in den letzten zehn Sekunden
eine Freiwurfaufstellung erkannt wurde. Ohne diesen Kontext bleibt ein Wurf
von dieser Stelle ein gewöhnlicher Zweipunkt-Wurfkandidat. Keine Trefferwertung.

JSON-Szenen enthalten `phase` (`formation`/`attempt`), `started_at`, `evidence`
und `related_substitutions`. Ein Einwurf verweist auf Wechselereignisse aus den
vorangehenden 30 Sekunden, auch der anderen Mannschaft. Das ist ausdrücklich
nur zeitliche Nachbarschaft: kein Nachweis einer gemeinsamen Unterbrechung,
keine erhöhte Wechsel-Konfidenz und keine Hochstufung von `possible`. Rücksprünge
oder Datenlücken über drei Sekunden löschen den Sequenzkontext. Ein naher
Verteidiger verhindert den Basketball-Einwurfkandidaten nicht pauschal, wenn
Ballbesitz/-nähe zusätzliche Evidenz liefert.

## Grenzen und nächste Validierung

Die Regeln sind an synthetischen positiven und negativen Sequenzen getestet,
nicht an annotiertem Spielvideo vermessen. Konfidenzwerte sind Heuristiken,
keine kalibrierten Wahrscheinlichkeiten. Kamerafehler und falsche Ballpositionen
können falsche Situationen erzeugen. Freiwürfe ohne Rebound-Aufstellung werden
noch nicht zuverlässig erkannt. Teamreihenfolge an den Reboundplätzen,
Schiedsrichtergesten, tatsächliches Betreten/Verlassen der Wechselzone,
Freiwurfserien und vollständige 30-Sekunden-Nachbewertung sind noch offen.
JSON-Verbraucher müssen Aufstellungen von Versuchen unterscheiden und dürfen
nicht beide als ausgeführte Freiwürfe zählen. Die Review-Oberfläche wertet die
neuen Evidenzfelder noch nicht gesondert aus.

Basketballspezifische Geometrie bleibt im Basketballmodul; Zeitfenster,
Ereignisdaten und Rücksprungbehandlung bleiben gemeinsam nutzbar.

Regelfachliche Referenz: [FIBA Official Basketball Rules 2024, Artikel 19 und 43](https://assets.fiba.basketball/image/upload/documents-corporate-fiba-official-rules-2024-v10a.pdf).
Die Regeln erlauben auch Freiwurfsituationen ohne besetzte Reboundplätze; deshalb
ist die hier implementierte Variante bewusst nicht vollständig. Keine Texte,
Grafiken oder Software aus der Quelle übernommen; eigene Implementierung unter
der Projektlizenz AGPL-3.0-only, keine neuen Abhängigkeiten.
