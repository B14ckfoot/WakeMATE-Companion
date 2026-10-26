#!/usr/bin/env bash
# Build and package the macOS developer preview. Run the quality gate first.
# This creates an unsigned, non-notarized headless app; menu-bar pairing and
# real-macOS validation are still required before a production release.
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
  echo 'error: macOS packaging requires macOS and hdiutil.' >&2
  exit 1
fi
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"
CARGO_VERSION="$(sed -nE 's/^version = "([0-9]+\.[0-9]+\.[0-9]+)"$/\1/p' Cargo.toml | head -1)"
VERSION="${1:-$CARGO_VERSION}"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ || "$VERSION" != "$CARGO_VERSION" ]]; then
  echo 'error: package version must match the numeric Cargo package version.' >&2
  exit 1
fi

source_manifest() {
  shasum -a 256 Cargo.toml Cargo.lock build.rs
  find src assets -type f -exec shasum -a 256 {} \; | LC_ALL=C sort
}
SOURCE_BEFORE="$(source_manifest)"
cargo build --release --locked
SOURCE_AFTER="$(source_manifest)"
if [[ "$SOURCE_BEFORE" != "$SOURCE_AFTER" ]]; then
  echo 'error: sources changed during the build; retry after edits finish.' >&2
  exit 1
fi
BIN_PATH="$REPO_ROOT/target/release/wakemate-companion"
DIST_ROOT="$REPO_ROOT/dist/macos"
mkdir -p "$DIST_ROOT"
DIST_DIR="$(mktemp -d "$DIST_ROOT/$VERSION-$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX")"
APP_DIR="$DIST_DIR/WakeMATE Companion.app"
DMG_PATH="$DIST_DIR/WakeMATE-Companion-$VERSION-unsigned.dmg"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cp "$BIN_PATH" "$APP_DIR/Contents/MacOS/wakemate-companion"
chmod +x "$APP_DIR/Contents/MacOS/wakemate-companion"
sed "s/__VERSION__/$VERSION/g" "$SCRIPT_DIR/Info.plist.template" > "$APP_DIR/Contents/Info.plist"

ICON_SOURCE="$REPO_ROOT/assets/LOGO.Brand.png"
if [[ -f "$ICON_SOURCE" ]] && command -v sips >/dev/null 2>&1 && command -v iconutil >/dev/null 2>&1; then
  ICONSET_DIR="$DIST_DIR/AppIcon.iconset"
  mkdir -p "$ICONSET_DIR"
  for size in 16 32 64 128 256 512; do
    sips -z "$size" "$size" "$ICON_SOURCE" --out "$ICONSET_DIR/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" "$ICON_SOURCE" --out "$ICONSET_DIR/icon_${size}x${size}@2x.png" >/dev/null
  done
  iconutil -c icns "$ICONSET_DIR" -o "$APP_DIR/Contents/Resources/AppIcon.icns"
else
  echo 'warning: app icon conversion is unavailable.' >&2
fi

hdiutil create -volname 'WakeMATE Companion' -srcfolder "$APP_DIR" -format UDZO "$DMG_PATH"
SOURCES_PATH="$DMG_PATH.sources.sha256"
printf '%s\n' "$SOURCE_AFTER" > "$SOURCES_PATH"
DMG_HASH="$(shasum -a 256 "$DMG_PATH" | cut -d ' ' -f 1)"
BIN_HASH="$(shasum -a 256 "$BIN_PATH" | cut -d ' ' -f 1)"
SOURCE_HASH="$(shasum -a 256 "$SOURCES_PATH" | cut -d ' ' -f 1)"
CHECKSUM_PATH="$DMG_PATH.sha256"
printf '%s  %s\n' "$DMG_HASH" "$(basename "$DMG_PATH")" > "$CHECKSUM_PATH"
PROVENANCE_PATH="$DMG_PATH.provenance.json"
REVISION="$(git rev-parse HEAD)"
DIRTY=false
if [[ -n "$(git status --porcelain)" ]]; then DIRTY=true; fi
printf '{"schema_version":1,"version":"%s","git_revision":"%s","worktree_dirty":%s,"artifact_sha256":"%s","executable_sha256":"%s","source_manifest_sha256":"%s","signing":"unsigned, non-notarized developer preview"}\n' \
  "$VERSION" "$REVISION" "$DIRTY" "$DMG_HASH" "$BIN_HASH" "$SOURCE_HASH" > "$PROVENANCE_PATH"
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  printf 'artifact_path=%s\nchecksum_path=%s\nprovenance_path=%s\nsources_path=%s\n' \
    "$DMG_PATH" "$CHECKSUM_PATH" "$PROVENANCE_PATH" "$SOURCES_PATH" >> "$GITHUB_OUTPUT"
fi
printf 'Packaged unsigned macOS preview: %s\n' "$DMG_PATH"
