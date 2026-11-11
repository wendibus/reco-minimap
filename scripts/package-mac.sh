#!/bin/zsh
# Builds the Mac app: "Reco Minimap.app" with the one Rust program inside. Started from the
# Finder it serves the interface, opens it in the browser and stays in the Dock.
#
#   scripts/package-mac.sh [output-dir]          default: dist/
#
# Environment:
#   RECO_SIGN_IDENTITY    "Developer ID Application: ..." for a signed app with hardened runtime;
#                         unset or "-" signs ad hoc (runs on this Mac only).
#   RECO_NOTARY_PROFILE   notarytool keychain profile; if set (with a Developer ID), the zipped
#                         app is sent to Apple for notarization and the ticket is stapled.
#   RECO_ORT_DYLIB        ONNX Runtime library to put into the app (1.24 or newer, with CoreML:
#                         the one from the Python wheel). Default: ORT_DYLIB_PATH. Without it the
#                         app looks for a Homebrew ONNX Runtime at run time.
#
# ffmpeg and ffprobe are not inside the app; they are taken from Homebrew (/opt/homebrew/bin).
set -euo pipefail

ROOT_DIR="${0:A:h}/.."
OUT_DIR="${1:-$ROOT_DIR/dist}"
mkdir -p "$OUT_DIR"
OUT_DIR="${OUT_DIR:A}"
APP_DIR="$OUT_DIR/Reco Minimap.app"
SIGN_IDENTITY="${RECO_SIGN_IDENTITY:--}"
ORT_DYLIB="${RECO_ORT_DYLIB:-${ORT_DYLIB_PATH:-}}"

cd "$ROOT_DIR"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
cargo build --release --features tools,coreml
cargo build --release --example app_icon
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources" "$APP_DIR/Contents/Frameworks"
cp "$TARGET_DIR/release/reco-minimap" "$APP_DIR/Contents/MacOS/reco-minimap"
sed "s/__VERSION__/$VERSION/g" mac/Info.plist > "$APP_DIR/Contents/Info.plist"
cp LICENSE "$APP_DIR/Contents/Resources/LICENSE"

# icon: drawn by examples/app_icon.rs, scaled into an .icns
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
"$TARGET_DIR/release/examples/app_icon" "$WORK/icon.png" >/dev/null
mkdir "$WORK/AppIcon.iconset"
for size in 16 32 128 256 512; do
  sips -z $size $size "$WORK/icon.png" --out "$WORK/AppIcon.iconset/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) "$WORK/icon.png" --out "$WORK/AppIcon.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$WORK/AppIcon.iconset" -o "$APP_DIR/Contents/Resources/AppIcon.icns"

if [[ -n "$ORT_DYLIB" ]]; then
  [[ -f "$ORT_DYLIB" ]] || { echo "ONNX Runtime library not found: $ORT_DYLIB" >&2; exit 2; }
  cp -L "$ORT_DYLIB" "$APP_DIR/Contents/Frameworks/libonnxruntime.dylib"
  chmod 644 "$APP_DIR/Contents/Frameworks/libonnxruntime.dylib"
  # its licence travels with it (MIT); the wheel keeps the files two folders up
  for notice in LICENSE ThirdPartyNotices.txt; do
    [[ -f "${ORT_DYLIB:A:h}/../$notice" ]] && cp "${ORT_DYLIB:A:h}/../$notice" "$APP_DIR/Contents/Resources/onnxruntime-$notice"
  done
else
  rmdir "$APP_DIR/Contents/Frameworks"
  echo "Note: no ONNX Runtime inside the app (set RECO_ORT_DYLIB); it will use one from Homebrew." >&2
fi

xattr -cr "$APP_DIR"
if [[ "$SIGN_IDENTITY" == "-" ]]; then
  [[ -d "$APP_DIR/Contents/Frameworks" ]] && codesign --force --sign - "$APP_DIR/Contents/Frameworks/libonnxruntime.dylib"
  codesign --force --sign - "$APP_DIR"
  echo "Note: signed ad hoc; set RECO_SIGN_IDENTITY for an app other Macs accept." >&2
else
  # inside out: the library first, then the app with its main program
  [[ -d "$APP_DIR/Contents/Frameworks" ]] && codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP_DIR/Contents/Frameworks/libonnxruntime.dylib"
  codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP_DIR"
fi
codesign --verify --deep --strict --verbose=2 "$APP_DIR"

ZIP_PATH="$OUT_DIR/Reco-Minimap-Mac-$VERSION.zip"
# Zip without resource forks and extended attributes: on a volume that keeps them in "._" files
# (exFAT) they would otherwise end up inside the app and break the signature for other unzip tools.
zip_app() {
  rm -f "$ZIP_PATH"
  ditto -c -k --norsrc --noextattr --keepParent "$APP_DIR" "$ZIP_PATH"
}
zip_app

if [[ "$SIGN_IDENTITY" != "-" && -n "${RECO_NOTARY_PROFILE:-}" ]]; then
  xcrun notarytool submit "$ZIP_PATH" --keychain-profile "$RECO_NOTARY_PROFILE" --wait
  xcrun stapler staple "$APP_DIR"
  zip_app
  spctl --assess --type execute --verbose=2 "$APP_DIR"
fi

echo "$APP_DIR"
echo "$ZIP_PATH"
