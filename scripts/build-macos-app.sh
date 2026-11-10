#!/usr/bin/env bash
# Builds "Marine Data Simulator.app" and a DMG in target/macos/.
#
#   scripts/build-macos-app.sh              # for this Mac's architecture
#   scripts/build-macos-app.sh --universal  # Apple Silicon + Intel in one app
#   scripts/build-macos-app.sh --no-dmg     # skip the disk image
#
# The app is signed ad hoc (no Apple Developer ID): it runs on this Mac, and on other Macs
# after right-click → Open the first time (see docs/USER_MANUAL.md, "macOS app").
set -euo pipefail

cd "$(dirname "$0")/.."
UNIVERSAL=0
DMG=1
for arg in "$@"; do
  case "$arg" in
    --universal) UNIVERSAL=1 ;;
    --no-dmg) DMG=0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

NAME="Marine Data Simulator"
BIN="marine-sim"
BUNDLE_ID="io.github.olafont.marine-data-simulator"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
OUT="target/macos"
APP="$OUT/$NAME.app"

# 1. Build.
if [ "$UNIVERSAL" = 1 ]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --locked -p app --target "$t"
  done
  EXE=$(mktemp -d)/$BIN
  lipo -create -output "$EXE" \
    "target/aarch64-apple-darwin/release/$BIN" "target/x86_64-apple-darwin/release/$BIN"
  ARCH="universal"
else
  cargo build --release --locked -p app
  EXE="target/release/$BIN"
  ARCH=$(uname -m)
fi

# 2. Icon: every size macOS asks for, from the 1024 px master.
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
ICONSET=$(mktemp -d)/AppIcon.iconset
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) assets/icon.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

# 3. Bundle.
cp "$EXE" "$APP/Contents/MacOS/$BIN"
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>                <string>$NAME</string>
  <key>CFBundleDisplayName</key>         <string>$NAME</string>
  <key>CFBundleIdentifier</key>          <string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key>          <string>$BIN</string>
  <key>CFBundleIconFile</key>            <string>AppIcon</string>
  <key>CFBundlePackageType</key>         <string>APPL</string>
  <key>CFBundleShortVersionString</key>  <string>$VERSION</string>
  <key>CFBundleVersion</key>             <string>$VERSION</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>LSApplicationCategoryType</key>   <string>public.app-category.developer-tools</string>
  <key>LSMinimumSystemVersion</key>      <string>11.0</string>
  <key>NSHighResolutionCapable</key>     <true/>
  <key>NSHumanReadableCopyright</key>    <string>© 2026 Olivier Lafont. GPL-3.0. Map data © OpenStreetMap and OpenSeaMap contributors</string>
</dict>
</plist>
EOF
plutil -lint "$APP/Contents/Info.plist" >/dev/null

# The licence and the notices travel with the app (GPL-3.0 section 4).
cp LICENSE THIRD_PARTY_NOTICES.md "$APP/Contents/Resources/"

# 4. Sign ad hoc (required on Apple Silicon; seals the bundle).
codesign --force --sign - --timestamp=none "$APP"
codesign --verify --strict "$APP"
echo "built $APP ($ARCH, version $VERSION)"

# 5. Disk image with an Applications shortcut.
if [ "$DMG" = 1 ]; then
  STAGE=$(mktemp -d)
  cp -R "$APP" "$STAGE/"
  ln -s /Applications "$STAGE/Applications"
  cp LICENSE "$STAGE/LICENSE.txt"
  cp THIRD_PARTY_NOTICES.md "$STAGE/"
  DMG_PATH="$OUT/MarineDataSimulator-$VERSION-macos-$ARCH.dmg"
  rm -f "$DMG_PATH"
  hdiutil create -quiet -volname "$NAME" -srcfolder "$STAGE" -fs HFS+ -format UDZO "$DMG_PATH"
  echo "built $DMG_PATH"
fi
