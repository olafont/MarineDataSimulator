#!/usr/bin/env bash
# Builds "Marine Data Simulator.app" and a DMG in target/macos/.
#
#   scripts/build-macos-app.sh              # for this Mac's architecture
#   scripts/build-macos-app.sh --universal  # Apple Silicon + Intel in one app
#   scripts/build-macos-app.sh --no-dmg     # skip the disk image
#
# Signing, from the environment:
#
# - MACOS_SIGN_IDENTITY unset: the app is signed ad hoc. It runs on this Mac, and on other Macs
#   after "Open Anyway" in System Settings the first time (see docs/USER_MANUAL.md, "Install").
# - MACOS_SIGN_IDENTITY set (e.g. "Developer ID Application: Name (TEAMID)"): the app and the
#   disk image are signed with it, with the hardened runtime and a secure timestamp.
# - Also notarized and stapled (the disk image, so other Macs open it without a warning) when
#   either MACOS_NOTARY_PROFILE names credentials saved with `xcrun notarytool store-credentials`,
#   or MACOS_NOTARY_APPLE_ID, MACOS_NOTARY_PASSWORD (an app-specific password) and
#   MACOS_NOTARY_TEAM_ID are set.
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
# The executable carries the app's name: the menu bar's "About", "Hide" and "Quit" items show it.
cp "$EXE" "$APP/Contents/MacOS/$NAME"
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>                <string>$NAME</string>
  <key>CFBundleDisplayName</key>         <string>$NAME</string>
  <key>CFBundleIdentifier</key>          <string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key>          <string>$NAME</string>
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

# 4. Sign: with a Developer ID when one is given, else ad hoc (required on Apple Silicon; seals
# the bundle).
IDENTITY="${MACOS_SIGN_IDENTITY:-}"
if [ -n "$IDENTITY" ]; then
  codesign --force --options runtime --timestamp --sign "$IDENTITY" "$APP"
  SIGNED="signed by $IDENTITY"
else
  codesign --force --sign - --timestamp=none "$APP"
  SIGNED="signed ad hoc"
fi
codesign --verify --strict "$APP"
echo "built $APP ($ARCH, version $VERSION, $SIGNED)"

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

  # 6. Sign, notarize and staple the disk image.
  if [ -n "$IDENTITY" ]; then
    codesign --force --timestamp --sign "$IDENTITY" "$DMG_PATH"
    NOTARY=()
    if [ -n "${MACOS_NOTARY_PROFILE:-}" ]; then
      NOTARY=(--keychain-profile "$MACOS_NOTARY_PROFILE")
    elif [ -n "${MACOS_NOTARY_APPLE_ID:-}" ] && [ -n "${MACOS_NOTARY_PASSWORD:-}" ] && [ -n "${MACOS_NOTARY_TEAM_ID:-}" ]; then
      NOTARY=(--apple-id "$MACOS_NOTARY_APPLE_ID" --password "$MACOS_NOTARY_PASSWORD" --team-id "$MACOS_NOTARY_TEAM_ID")
    fi
    if [ ${#NOTARY[@]} -gt 0 ]; then
      echo "notarizing $DMG_PATH (a few minutes)…"
      RESULT=$(xcrun notarytool submit "$DMG_PATH" "${NOTARY[@]}" --wait 2>&1) || true
      echo "$RESULT"
      if ! grep -q "status: Accepted" <<<"$RESULT"; then
        # Apple's log says what it rejected.
        ID=$(sed -n 's/^ *id: //p' <<<"$RESULT" | head -1)
        [ -n "$ID" ] && xcrun notarytool log "$ID" "${NOTARY[@]}" || true
        echo "notarization failed" >&2
        exit 1
      fi
      xcrun stapler staple "$DMG_PATH"
      spctl --assess --type open --context context:primary-signature -vv "$DMG_PATH"
      echo "notarized $DMG_PATH"
    else
      echo "not notarized: no notarization credentials given" >&2
    fi
  fi
fi
