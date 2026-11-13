#!/bin/zsh
# Builds the Reco Minimap "payload": the part of the Mac app that updates without a new app
# download - the reco-minimap program (with the React interface built in) and ONNX Runtime.
# The launcher runs it from ~/Library/Application Support/Reco Minimap/payloads/<version>/.
#
#   mac/scripts/build-payload.sh [output-dir]
#
# Writes <output-dir>/payload/ (a ready folder) and Reco.Minimap.Payload.<version>.zip.
# Environment:
#   RECO_SIGN_IDENTITY        "Developer ID Application: ..." (hardened runtime); unset or "-" = ad hoc
#   RECO_ORT_DYLIB            ONNX Runtime library (1.24+, with CoreML); default ORT_DYLIB_PATH
#   RECO_PAYLOAD_MIN_LAUNCHER oldest launcher that may run this payload (default 0.10.0)
#   RECO_SKIP_NPM=1           use the interface already built in src/tools/webui
set -euo pipefail

ROOT_DIR="${0:A:h}/../.."
OUTPUT_DIR="${1:-$ROOT_DIR/mac/dist/payload-build}"
SIGN_IDENTITY="${RECO_SIGN_IDENTITY:--}"
ORT_DYLIB="${RECO_ORT_DYLIB:-${ORT_DYLIB_PATH:-}}"
MIN_LAUNCHER="${RECO_PAYLOAD_MIN_LAUNCHER:-0.10.0}"

cd "$ROOT_DIR"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
if [[ -z "${RECO_SKIP_NPM:-}" ]]; then
  (cd web && npm ci --no-audit --no-fund >/dev/null && npm run build >/dev/null)
fi
[[ -f src/tools/webui/index.html ]] || { echo "Built interface not found: src/tools/webui" >&2; exit 1; }
# Nothing of this computer may end up in the program: compiled-in source paths (panic messages)
# would show the home folder and the user name, so they are rewritten to neutral ones.
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME/.cargo/registry/src=/cargo --remap-path-prefix=$HOME/.cargo/git/checkouts=/cargo-git --remap-path-prefix=$HOME/.rustup=/rustup --remap-path-prefix=$PWD=/reco-minimap"
cargo build --release --features tools,coreml >&2
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
LEAK="$(strings -a "$TARGET_DIR/release/reco-minimap" | grep -E "$HOME|/Users/|/Volumes/|${USER:-nouser}" | head -3 || true)"
if [[ -n "$LEAK" ]]; then
  echo "The program still contains paths of this computer - not packaged:" >&2
  echo "$LEAK" >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="${OUTPUT_DIR:A}"
PAYLOAD_DIR="$OUTPUT_DIR/payload"
ZIP_PATH="$OUTPUT_DIR/Reco.Minimap.Payload.$VERSION.zip"
rm -rf "$PAYLOAD_DIR" "$ZIP_PATH" "$ZIP_PATH.sig"
mkdir -p "$PAYLOAD_DIR/bin" "$PAYLOAD_DIR/lib" "$PAYLOAD_DIR/licenses"

cp "$TARGET_DIR/release/reco-minimap" "$PAYLOAD_DIR/bin/reco-minimap"
chmod 755 "$PAYLOAD_DIR/bin/reco-minimap"
cp LICENSE "$PAYLOAD_DIR/licenses/reco-minimap-LICENSE"
if [[ -n "$ORT_DYLIB" ]]; then
  [[ -f "$ORT_DYLIB" ]] || { echo "ONNX Runtime library not found: $ORT_DYLIB" >&2; exit 2; }
  cp -L "$ORT_DYLIB" "$PAYLOAD_DIR/lib/libonnxruntime.dylib"
  chmod 644 "$PAYLOAD_DIR/lib/libonnxruntime.dylib"
  # its licence travels with it (MIT); the Python wheel keeps the files two folders up
  for notice in LICENSE ThirdPartyNotices.txt; do
    [[ -f "${ORT_DYLIB:A:h}/../$notice" ]] && cp "${ORT_DYLIB:A:h}/../$notice" "$PAYLOAD_DIR/licenses/onnxruntime-$notice"
  done
else
  rmdir "$PAYLOAD_DIR/lib"
  echo "Note: no ONNX Runtime in the payload (set RECO_ORT_DYLIB); reco-minimap then looks for one from Homebrew." >&2
fi
printf '{\n  "version": "%s",\n  "minLauncher": "%s"\n}\n' "$VERSION" "$MIN_LAUNCHER" > "$PAYLOAD_DIR/payload.json"

# Sign the program and the library: an update payload is unpacked outside the app bundle, so its
# code needs its own Developer ID signature (same team, so the program may load the library).
xattr -cr "$PAYLOAD_DIR"
for code in "$PAYLOAD_DIR/lib/libonnxruntime.dylib" "$PAYLOAD_DIR/bin/reco-minimap"; do
  [[ -f "$code" ]] || continue
  if [[ "$SIGN_IDENTITY" == "-" ]]; then
    codesign --force --sign - "$code"
  else
    codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$code"
  fi
done
find "$PAYLOAD_DIR" -name '.DS_Store' -delete

# Zip the folder's contents (payload.json at the archive root), deterministic order.
(cd "$PAYLOAD_DIR" && find . -type f | sed 's#^\./##' | LC_ALL=C sort | zip -q -X "$ZIP_PATH" -@)
echo "$ZIP_PATH"
