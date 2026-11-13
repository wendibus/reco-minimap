#!/usr/bin/env bash
# Builds the Windows or Linux download: a zip with reco-minimap, a start script that opens the
# interface in the browser, ONNX Runtime (if given) and the licences.
#
#   scripts/package-platform.sh [output-dir]          (Linux, or Git Bash / MSYS on Windows)
#
# Environment: RECO_ORT_LIB  onnxruntime.dll / libonnxruntime.so.* to put into the zip
#              RECO_FEATURES cargo features (default: tools)
#              RECO_SKIP_NPM=1 use the interface already built in src/tools/webui
# ffmpeg and ffprobe are not inside; they must be on PATH (the start script says so if they are missing).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="${1:-$ROOT_DIR/dist}"
cd "$ROOT_DIR"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*|Windows_NT) OS=windows; EXE=reco-minimap.exe ;;
  Linux) OS=linux; EXE=reco-minimap ;;
  *) echo "Use mac/scripts/package-app.sh on macOS." >&2; exit 2 ;;
esac
ARCH="$(uname -m)"; [[ "$ARCH" == "AMD64" ]] && ARCH=x86_64

if [[ -z "${RECO_SKIP_NPM:-}" ]]; then
  (cd web && npm ci --no-audit --no-fund && npm run build)
fi
# keep paths of the build machine out of the program (see mac/scripts/build-payload.sh)
CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$CARGO_HOME_DIR/registry/src=/cargo --remap-path-prefix=$CARGO_HOME_DIR/git/checkouts=/cargo-git --remap-path-prefix=$PWD=/reco-minimap"
cargo build --release --features "${RECO_FEATURES:-tools}"
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"

NAME="Reco-Minimap-$VERSION-$OS-$ARCH"
STAGE="$OUT_DIR/$NAME"
rm -rf "$STAGE" "$OUT_DIR/$NAME.zip"
mkdir -p "$STAGE/licenses"
cp "$TARGET_DIR/release/$EXE" "$STAGE/"
cp LICENSE "$STAGE/licenses/reco-minimap-LICENSE"
if [[ -n "${RECO_ORT_LIB:-}" ]]; then
  if [[ "$OS" == windows ]]; then cp "$RECO_ORT_LIB" "$STAGE/onnxruntime.dll"; else mkdir -p "$STAGE/lib" && cp -L "$RECO_ORT_LIB" "$STAGE/lib/libonnxruntime.so"; fi
  for notice in LICENSE ThirdPartyNotices.txt; do
    f="$(dirname "$RECO_ORT_LIB")/../$notice"; [[ -f "$f" ]] && cp "$f" "$STAGE/licenses/onnxruntime-$notice"
  done
fi

if [[ "$OS" == windows ]]; then
  cat > "$STAGE/Start Reco Minimap.bat" <<'BAT'
@echo off
rem Starts Reco Minimap and opens it in the browser. Close this window to stop it.
cd /d "%~dp0"
if exist "%~dp0onnxruntime.dll" set "ORT_DYLIB_PATH=%~dp0onnxruntime.dll"
where ffmpeg >nul 2>nul || echo ffmpeg was not found. Install it (for example: winget install Gyan.FFmpeg) and start again.
"%~dp0reco-minimap.exe" ui
pause
BAT
  printf 'Reco Minimap %s\r\n\r\nDouble-click "Start Reco Minimap.bat". The interface opens in your browser;\r\nclosing the black window stops the program. ffmpeg must be installed\r\n(winget install Gyan.FFmpeg). Guide: in the interface, top right.\r\n' "$VERSION" > "$STAGE/README.txt"
  (cd "$OUT_DIR" && powershell -NoProfile -Command "Compress-Archive -Path '$NAME' -DestinationPath '$NAME.zip' -Force")
else
  cat > "$STAGE/start-reco-minimap.sh" <<'SH'
#!/usr/bin/env bash
# Starts Reco Minimap and opens it in the browser. Ctrl+C (or closing the terminal) stops it.
DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"
[[ -f "$DIR/lib/libonnxruntime.so" ]] && export ORT_DYLIB_PATH="$DIR/lib/libonnxruntime.so"
command -v ffmpeg >/dev/null || echo "ffmpeg was not found - install it (e.g. sudo apt install ffmpeg) and start again."
exec "$DIR/reco-minimap" ui
SH
  chmod 755 "$STAGE/start-reco-minimap.sh" "$STAGE/reco-minimap"
  printf 'Reco Minimap %s\n\nRun ./start-reco-minimap.sh (or double-click it and choose "Run"). The interface opens in\nyour browser; Ctrl+C stops the program. ffmpeg must be installed (sudo apt install ffmpeg).\n' "$VERSION" > "$STAGE/README.txt"
  (cd "$OUT_DIR" && zip -qr "$NAME.zip" "$NAME")
fi
echo "$OUT_DIR/$NAME.zip"
