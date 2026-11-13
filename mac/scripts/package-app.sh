#!/bin/zsh
# Builds "Reco Minimap.app": the Swift launcher (window around the web interface, signed updates)
# with the current payload inside.
#
#   mac/scripts/package-app.sh                 -> mac/dist/Reco Minimap.app
#
# Environment: RECO_SIGN_IDENTITY ("Developer ID Application: ..."; unset = ad hoc),
#              RECO_ORT_DYLIB / ORT_DYLIB_PATH (ONNX Runtime for the payload), RECO_DIST_DIR.
set -euo pipefail

MAC_DIR="${0:A:h}/.."
ROOT_DIR="$MAC_DIR/.."
DIST_DIR="${RECO_DIST_DIR:-$MAC_DIR/dist}"
APP_DIR="$DIST_DIR/Reco Minimap.app"
BUILD_DIR="${RECO_BUILD_DIR:-${TMPDIR:-/tmp}/reco-minimap-mac-release-build}"
SIGN_IDENTITY="${RECO_SIGN_IDENTITY:--}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -1)"

cd "$MAC_DIR"
swift build -c release --disable-sandbox --scratch-path "$BUILD_DIR" >&2
BIN_DIR="$(swift build -c release --disable-sandbox --scratch-path "$BUILD_DIR" --show-bin-path)"

mkdir -p "$DIST_DIR"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cp "$BIN_DIR/RecoMinimapMac" "$APP_DIR/Contents/MacOS/RecoMinimapMac"
sed "s/__VERSION__/$VERSION/g" "$MAC_DIR/Packaging/Info.plist" > "$APP_DIR/Contents/Info.plist"
cp "$ROOT_DIR/LICENSE" "$APP_DIR/Contents/Resources/LICENSE"

# icon: drawn by examples/app_icon.rs, scaled into an .icns
(cd "$ROOT_DIR" && cargo build --release --example app_icon >&2)
TARGET_DIR="$(cd "$ROOT_DIR" && cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
"$TARGET_DIR/release/examples/app_icon" "$WORK/icon.png" >/dev/null
mkdir "$WORK/AppIcon.iconset"
for size in 16 32 128 256 512; do
  sips -z $size $size "$WORK/icon.png" --out "$WORK/AppIcon.iconset/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) "$WORK/icon.png" --out "$WORK/AppIcon.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$WORK/AppIcon.iconset" -o "$APP_DIR/Contents/Resources/AppIcon.icns"

# The payload ships inside the app and is copied to ~/Library/Application Support/Reco Minimap/payloads
# on first start; later versions arrive through the in-app updater without a new app download.
"$MAC_DIR/scripts/build-payload.sh" "$WORK/payload-build" >/dev/null
cp -R "$WORK/payload-build/payload" "$APP_DIR/Contents/Resources/payload"

xattr -cr "$APP_DIR"
if [[ "$SIGN_IDENTITY" == "-" ]]; then
  codesign --force --deep --sign - "$APP_DIR"
  echo "Note: signed ad hoc (this Mac only); set RECO_SIGN_IDENTITY for a notarized release." >&2
else
  codesign --force --deep --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP_DIR"
fi
codesign --verify --deep --strict --verbose=2 "$APP_DIR" >&2
echo "$APP_DIR"
