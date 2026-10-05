#!/usr/bin/env bash
# Ganzes Spiel in Abschnitten (z.B. Viertel) verarbeiten und die Minimap ins Spielvideo einblenden.
#
#   MODEL=... COURT=court.json TEAMS=teams.json scripts/run_game.sh <panorama.mp4> <segmente.txt> <ausgabeordner>
#
# segmente.txt: eine Zeile je Abschnitt:  <name> <start-sekunden> <dauer-sekunden>
#     q1 305 600
#     q2 1210 600
#
# Optionale Umgebungsvariablen:
#   BROADCAST   Spielvideo, in das die Minimap eingeblendet wird (gleiche Zeitachse wie das Panorama).
#               Ohne BROADCAST entstehen nur die Overlay-Dateien (<name>.mov mit Transparenz) und <name>.jsonl.
#   SYNC        Sekunden, um die das BROADCAST-Video gegenueber dem Panorama versetzt beginnt (Standard 0)
#   OVERLAY_W   Breite der eingeblendeten Minimap in Pixeln (Standard 480)
#   OVERLAY_POS ffmpeg-Position (Standard: unten rechts)   z.B. "30:30" fuer oben links
#   BG          Deckkraft der dunklen Minimap-Unterlage 0..1 (Standard 0.6 mit BROADCAST, sonst 0 = transparent)
#   EXTRA       weitere reco-minimap-Optionen, z.B. EXTRA='--ref-color "#000000" --draw-home "#ffffff" --draw-away "#2f6bff"'
#   EVERY       jedes N-te Bild erkennen (Standard 5)
#   TILE        Kachelung (Standard auto, braucht scales.json im aktuellen Ordner oder SCALES=<datei>)
#   EP          Ausfuehrung: coreml (Standard) oder cpu
set -euo pipefail
PANO=${1:?Panorama-Video fehlt}; SEG=${2:?segmente.txt fehlt}; OUT=${3:?Ausgabeordner fehlt}
: "${MODEL:?MODEL=<modellordner> setzen}" "${COURT:?COURT=court.json setzen}" "${TEAMS:?TEAMS=teams.json setzen}"
EVERY=${EVERY:-5}; EP=${EP:-coreml}; TILE=${TILE:-auto}; SCALES_ARG=(); [[ -n "${SCALES:-}" ]] && SCALES_ARG=(--scales "$SCALES"); OVERLAY_W=${OVERLAY_W:-480}; OVERLAY_POS=${OVERLAY_POS:-W-w-30:H-h-30}; SYNC=${SYNC:-0}
BG=${BG:-$([[ -n "${BROADCAST:-}" ]] && echo 0.6 || echo 0)}
mkdir -p "$OUT"
while read -r name start dur; do
  [[ -z "${name:-}" || "$name" == \#* ]] && continue
  echo "=== $name: ab ${start}s, ${dur}s ==="
  eval reco-minimap run '"$PANO"' --model '"$MODEL"' --court '"$COURT"' --teams '"$TEAMS"' --tile "$TILE" "${SCALES_ARG[@]+${SCALES_ARG[@]}}" --ep "$EP" \
       --start "$start" --duration "$dur" --every "$EVERY" \
       --alpha '"$OUT/$name.mov"' --alpha-bg "$BG" --json '"$OUT/$name.jsonl"' ${EXTRA:-} < /dev/null
  if [[ -n "${BROADCAST:-}" ]]; then
    bstart=$(python3 -c "print(max(0, $start + $SYNC))")
    ffmpeg -nostdin -y -v error -ss "$bstart" -t "$dur" -i "$BROADCAST" -i "$OUT/$name.mov" \
      -filter_complex "[1:v]scale=$OVERLAY_W:-1[m];[0:v][m]overlay=$OVERLAY_POS:format=auto" \
      -c:v libx264 -crf 18 -preset medium -pix_fmt yuv420p -c:a copy "$OUT/${name}_mit_minimap.mp4"
    echo "  -> $OUT/${name}_mit_minimap.mp4"
  fi
done < "$SEG"
echo "Fertig. Zustandsdaten je Abschnitt: $OUT/*.jsonl (Auswertung: reco-minimap stats <datei>)"
