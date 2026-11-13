#!/bin/zsh
set -euo pipefail

ROOT_DIR="${0:A:h}/.."
FINAL_DIST_DIR="${RECO_DIST_DIR:-$ROOT_DIR/dist}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT_DIR/../Cargo.toml" | head -1)"
WORK_DIR="$(mktemp -d /tmp/reco-minimap-dmg.XXXXXX)"
APP_DIR="$WORK_DIR/Reco Minimap.app"
STAGING_DIR="$WORK_DIR/dmg-stage"
DMG_PATH="$FINAL_DIST_DIR/Reco-Minimap-Mac-$VERSION.dmg"
WORK_DMG_PATH="$WORK_DIR/Reco-Minimap-Mac-$VERSION.dmg"
RW_DMG_PATH="$WORK_DIR/Reco-Minimap-Mac-$VERSION-rw.dmg"
MOUNT_DIR="$WORK_DIR/dmg-mount"

cleanup() {
  hdiutil detach "$MOUNT_DIR" -force >/dev/null 2>&1 || true
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

RECO_DIST_DIR="$WORK_DIR" "$ROOT_DIR/scripts/package-app.sh"
rm -rf "$STAGING_DIR"
mkdir -p "$STAGING_DIR"
ditto --noextattr --noqtn "$APP_DIR" "$STAGING_DIR/Reco Minimap.app"
xattr -cr "$STAGING_DIR/Reco Minimap.app"
codesign --verify --deep --strict --verbose=2 "$STAGING_DIR/Reco Minimap.app"
ln -s /Applications "$STAGING_DIR/Applications"
mkdir -p "$FINAL_DIST_DIR"
rm -f "$DMG_PATH" "$RW_DMG_PATH" "$WORK_DMG_PATH"
rm -rf "$MOUNT_DIR"
mkdir -p "$MOUNT_DIR"
hdiutil create -volname "Reco Minimap" -srcfolder "$STAGING_DIR" -ov -format UDRW "$RW_DMG_PATH"
hdiutil attach "$RW_DMG_PATH" -nobrowse -mountpoint "$MOUNT_DIR" >/dev/null
xattr -cr "$MOUNT_DIR/Reco Minimap.app"
if [[ "${RECO_SIGN_IDENTITY:--}" == "-" ]]; then
  codesign --force --deep --sign - "$MOUNT_DIR/Reco Minimap.app"
else
  codesign --force --deep --options runtime --timestamp --sign "$RECO_SIGN_IDENTITY" "$MOUNT_DIR/Reco Minimap.app"
fi
codesign --verify --deep --strict --verbose=2 "$MOUNT_DIR/Reco Minimap.app"
hdiutil detach "$MOUNT_DIR" >/dev/null
hdiutil convert "$RW_DMG_PATH" -format UDZO -o "$WORK_DMG_PATH"
codesign --force --sign "${RECO_SIGN_IDENTITY:--}" "$WORK_DMG_PATH"
ditto --noextattr --noqtn "$WORK_DMG_PATH" "$DMG_PATH"
echo "$DMG_PATH"
