#!/usr/bin/env bash
#
# Packages the already-built macOS binary into a WakeMATE Companion.app
# bundle and a .dmg. Must run on macOS (uses hdiutil).
#
# IMPORTANT LIMITATIONS -- read before trusting this artifact:
#   - This produces an UNSIGNED, NON-NOTARIZED .dmg. Gatekeeper will block
#     it on any Mac other than the one that built it unless the user right
#     clicks -> Open, or signing/notarization is added (see
#     docs/MACOS_BUILD.md).
#   - The binary it packages is today's cross-platform build target, which
#     runs as a headless local HTTP/UDP service. There is no menu-bar icon,
#     pairing-QR window, or login-item toggle yet on macOS -- see
#     docs/MACOS_BUILD.md for the concrete porting plan for that parity work.
#
# Usage:
#   ./installer/macos/package-macos.sh [version]
#
# Expects target/release/wakemate-companion to already exist (run
# `cargo build --release` first).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
VERSION="${1:-$(grep -m1 '^version' "$REPO_ROOT/Cargo.toml" | sed -E 's/version = "(.*)"/\1/')}"

BIN_PATH="$REPO_ROOT/target/release/wakemate-companion"
DIST_DIR="$REPO_ROOT/dist/macos"
APP_NAME="WakeMATE Companion.app"
APP_DIR="$DIST_DIR/$APP_NAME"
DMG_PATH="$DIST_DIR/WakeMATE-Companion-$VERSION-unsigned.dmg"

if [[ ! -f "$BIN_PATH" ]]; then
  echo "error: $BIN_PATH not found. Run 'cargo build --release' first." >&2
  exit 1
fi

echo "Packaging WakeMATE Companion $VERSION for macOS (unsigned)..."

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"

cp "$BIN_PATH" "$APP_DIR/Contents/MacOS/wakemate-companion"
chmod +x "$APP_DIR/Contents/MacOS/wakemate-companion"

sed "s/__VERSION__/$VERSION/g" "$SCRIPT_DIR/Info.plist.template" > "$APP_DIR/Contents/Info.plist"

# Best-effort icon conversion; do not fail packaging if iconutil/sips or the
# source artwork aren't available (e.g. in a minimal CI image).
ICON_SOURCE="$REPO_ROOT/assets/LOGO.Brand.png"
if [[ -f "$ICON_SOURCE" ]] && command -v sips >/dev/null 2>&1 && command -v iconutil >/dev/null 2>&1; then
  ICONSET_DIR="$(mktemp -d)/AppIcon.iconset"
  mkdir -p "$ICONSET_DIR"
  for size in 16 32 64 128 256 512; do
    sips -z "$size" "$size" "$ICON_SOURCE" --out "$ICONSET_DIR/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" "$ICON_SOURCE" --out "$ICONSET_DIR/icon_${size}x${size}@2x.png" >/dev/null
  done
  iconutil -c icns "$ICONSET_DIR" -o "$APP_DIR/Contents/Resources/AppIcon.icns"
  echo "Generated AppIcon.icns from $ICON_SOURCE"
else
  echo "warning: skipping app icon (need assets/LOGO.Brand.png plus sips/iconutil)" >&2
fi

echo "Building $DMG_PATH ..."
rm -f "$DMG_PATH"
hdiutil create -volname "WakeMATE Companion" -srcfolder "$APP_DIR" -ov -format UDZO "$DMG_PATH"

echo ""
echo "Done: $DMG_PATH"
echo "This .dmg is UNSIGNED and NOT NOTARIZED -- see docs/MACOS_BUILD.md before distributing it."
