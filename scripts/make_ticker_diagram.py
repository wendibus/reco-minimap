#!/usr/bin/env python3
"""Zeichnet docs/img/ticker-flow.<lang>.svg (Ablauf des Live-Tickers) in vier Sprachen.
Aufruf:  python3 scripts/make_ticker_diagram.py   (im Projektordner)"""
import os, html

W, H = 1100, 624
TXT = {
 "en": dict(band="On your computer (match day)", net="Internet", cam=("Camera", "Reco panorama video"),
   live=("reco-minimap live", "detects substitutions and game situations"),
   gate=("Release (optional)", "you approve every entry; this computer only"),
   hub=("Public feed", "numbers only, or names if you allow them"),
   wifi=("Phones on the same Wi-Fi", "http://<computer>:8765/ticker"),
   send=("Sender", "writes ticker.json and uploads it by SFTP / FTP / WebDAV"),
   web=("Your web space", "only static files: ticker.json, ticker.js, ticker.html"),
   vis=("Visitors", "your page shows the widget or an iframe; it reads ticker.json every 3 seconds"),
   up="one small file", open="visitors open your page",
   note=("Nothing to install on the server", "No PHP, no database, no login. Positions, the hub and the release page never leave your computer; only released entries reach the web space.")),
 "de": dict(band="Auf deinem Rechner (Spieltag)", net="Internet", cam=("Kamera", "Reco-Panoramavideo"),
   live=("reco-minimap live", "erkennt Wechsel und Spielsituationen"),
   gate=("Freigabe (optional)", "du gibst jeden Eintrag frei; nur dieser Rechner"),
   hub=("Öffentlicher Ticker", "nur Nummern, oder Namen, wenn du sie erlaubst"),
   wifi=("Handys im selben WLAN", "http://<Rechner>:8765/ticker"),
   send=("Sender", "schreibt ticker.json und lädt sie per SFTP / FTP / WebDAV hoch"),
   web=("Dein Webspace", "nur statische Dateien: ticker.json, ticker.js, ticker.html"),
   vis=("Besucher", "deine Seite zeigt das Widget oder einen iframe; es liest ticker.json alle 3 Sekunden"),
   up="eine kleine Datei", open="Besucher öffnen deine Seite",
   note=("Auf dem Server ist nichts zu installieren", "Kein PHP, keine Datenbank, kein Login. Positionen, der Hub und die Freigabe-Seite verlassen deinen Rechner nie; nur freigegebene Einträge erreichen den Webspace.")),
 "fr": dict(band="Sur votre ordinateur (jour de match)", net="Internet", cam=("Caméra", "vidéo panoramique Reco"),
   live=("reco-minimap live", "détecte changements et situations de jeu"),
   gate=("Validation (option)", "vous validez chaque entrée (cet ordinateur)"),
   hub=("Fil public", "numéros seulement, ou noms si vous les autorisez"),
   wifi=("Téléphones sur le même Wi-Fi", "http://<ordinateur>:8765/ticker"),
   send=("Émetteur", "écrit ticker.json et le téléverse par SFTP / FTP / WebDAV"),
   web=("Votre hébergement", "uniquement des fichiers statiques : ticker.json, ticker.js, ticker.html"),
   vis=("Visiteurs", "votre page affiche le widget ou un iframe ; il lit ticker.json toutes les 3 secondes"),
   up="un petit fichier", open="les visiteurs ouvrent votre page",
   note=("Rien à installer sur le serveur", "Pas de PHP, pas de base de données, pas de connexion. Les positions, le hub et la page de validation ne quittent jamais votre ordinateur ; seules les entrées validées arrivent sur l'hébergement.")),
 "es": dict(band="En tu ordenador (día de partido)", net="Internet", cam=("Cámara", "vídeo panorámico de Reco"),
   live=("reco-minimap live", "detecta cambios y situaciones de juego"),
   gate=("Aprobación (opcional)", "apruebas cada entrada (este ordenador)"),
   hub=("Resumen público", "solo números, o nombres si los permites"),
   wifi=("Móviles en la misma Wi-Fi", "http://<ordenador>:8765/ticker"),
   send=("Emisor", "escribe ticker.json y lo sube por SFTP / FTP / WebDAV"),
   web=("Tu alojamiento web", "solo archivos estáticos: ticker.json, ticker.js, ticker.html"),
   vis=("Visitantes", "tu página muestra el widget o un iframe; lee ticker.json cada 3 segundos"),
   up="un archivo pequeño", open="los visitantes abren tu página",
   note=("Nada que instalar en el servidor", "Sin PHP, sin base de datos, sin inicio de sesión. Las posiciones, el hub y la página de aprobación nunca salen de tu ordenador; solo las entradas aprobadas llegan al alojamiento.")),
}

def wrap(text, width, fs):
    """Zeilenumbruch nach geschaetzter Zeichenbreite (grobe Sans-Serif-Metrik)."""
    per = max(8, int((width - 20) / (fs * 0.56)))
    lines, cur = [], ""
    for w in text.split():
        if cur and len(cur) + 1 + len(w) > per:
            lines.append(cur); cur = w
        else:
            cur = (cur + " " + w).strip()
    if cur: lines.append(cur)
    return lines

def box(x, y, w, h, title, sub, fill, stroke):
    o = f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="12" fill="{fill}" stroke="{stroke}" stroke-width="2"/>'
    sl = wrap(sub, w, 13)
    tl = wrap(title, w, 16)
    total = len(tl) * 20 + len(sl) * 17
    ty = y + (h - total) / 2 + 15
    for t in tl:
        o += f'<text x="{x + w/2}" y="{ty:.0f}" text-anchor="middle" font-size="16" font-weight="700" fill="#10202e">{html.escape(t)}</text>'; ty += 20
    for t in sl:
        o += f'<text x="{x + w/2}" y="{ty:.0f}" text-anchor="middle" font-size="13" fill="#2b3f50">{html.escape(t)}</text>'; ty += 17
    assert total + 16 <= h, (title, total, h)
    return o

def arrow(pts, label=None, lx=0, ly=0, dashed=False):
    p = " ".join(f"{a},{b}" for a, b in pts)
    d = ' stroke-dasharray="7 6"' if dashed else ''
    o = f'<polyline points="{p}" fill="none" stroke="#3b5a73" stroke-width="2.5"{d} marker-end="url(#ah)"/>'
    if label:
        o += f'<text x="{lx}" y="{ly}" font-size="12.5" fill="#3b5a73" font-style="italic">{html.escape(label)}</text>'
    return o

def make(lang):
    t = TXT[lang]
    o = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" font-family="system-ui,-apple-system,Segoe UI,Helvetica,Arial,sans-serif" role="img" aria-label="Live ticker">',
         '<defs><marker id="ah" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#3b5a73"/></marker></defs>',
         f'<rect width="{W}" height="{H}" fill="#ffffff"/>',
         f'<rect x="10" y="22" width="1080" height="316" rx="18" fill="#f1f6fb" stroke="#c5d6e6" stroke-width="2"/>',
         f'<text x="30" y="48" font-size="14" font-weight="700" fill="#4a6b86">{html.escape(t["band"])}</text>']
    # row 1
    o.append(box(30, 66, 170, 104, *t["cam"], "#e8f0f8", "#7da2c4"))
    o.append(box(240, 66, 200, 104, *t["live"], "#e3f3ea", "#6fb48a"))
    o.append(box(480, 66, 200, 104, *t["gate"], "#fff3d6", "#d9a93a"))
    o.append(box(720, 66, 340, 104, *t["hub"], "#e3f3ea", "#6fb48a"))
    for a, b in [(200, 240), (440, 480), (680, 720)]:
        o.append(arrow([(a + 2, 118), (b - 2, 118)]))
    # row 2
    o.append(box(480, 232, 200, 84, *t["wifi"], "#e8f0f8", "#7da2c4"))
    o.append(box(720, 232, 340, 84, *t["send"], "#e3f3ea", "#6fb48a"))
    o.append(arrow([(760, 172), (760, 202), (580, 202), (580, 230)]))
    o.append(arrow([(890, 172), (890, 230)]))
    # internet divider
    o.append('<line x1="10" y1="372" x2="1090" y2="372" stroke="#8aa3b8" stroke-width="2" stroke-dasharray="3 8"/>')
    o.append(f'<text x="1085" y="365" text-anchor="end" font-size="13" font-weight="700" fill="#8aa3b8">{html.escape(t["net"])}</text>')
    o.append(arrow([(890, 318), (890, 410)], t["up"], 902, 366))
    # row 3, 4
    o.append(box(720, 412, 340, 80, *t["web"], "#fff3d6", "#d9a93a"))
    o.append(box(720, 522, 340, 88, *t["vis"], "#e8f0f8", "#7da2c4"))
    o.append(arrow([(890, 494), (890, 520)], t["open"], 902, 513))
    # note
    o.append(box(30, 412, 640, 198, *t["note"], "#f6f9fc", "#c5d6e6"))
    o.append("</svg>")
    return "\n".join(o)

here = os.path.dirname(os.path.abspath(__file__))
for lang in TXT:
    p = os.path.join(here, "..", "docs", "img", f"ticker-flow.{lang}.svg")
    open(p, "w", encoding="utf-8").write(make(lang))
    print("geschrieben:", os.path.normpath(p))
