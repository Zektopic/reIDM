#!/usr/bin/env bash
# Build reIDM-<version>.dmg from universal binaries: a drag-to-Applications disk image whose
# window shows reIDM.app, an arrow, and the Applications folder.
# Usage: make-dmg.sh <version> <reidm> <reidm-host> <reidm-gui> <out.dmg>
#
# Needs dmgbuild (`python3 -m pip install dmgbuild`), which lays out the window without
# scripting Finder. Everything reIDM needs is inside the app: the service and the browser's
# native host beside the window's binary in Contents/MacOS, the extension in Resources. The
# service registers the native host and copies the extension out each time it starts, so
# dragging the app to Applications and opening it once is the whole installation.
#
# Signing: set MACOS_SIGN_IDENTITY to a "Developer ID Application: …" identity to sign for
# distribution, and MACOS_NOTARY_PROFILE to a notarytool keychain profile to notarize. With
# neither set, everything is ad-hoc signed, which is still *required*: `lipo` does not carry
# code signatures over, and the kernel refuses to execute an unsigned arm64 binary — the
# symptom is a bare "Killed: 9" with no dialog on Apple silicon.
set -euo pipefail
VER=$1; REIDM=$2; HOST=$3; GUI=$4; OUT=$5
# Info.plist's version fields are dotted integers (a release candidate's "-rc" is not one);
# the file name keeps the full version, and About shows the binary's own version string.
SHORT=${VER%%-*}
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

SIGN_ID="${MACOS_SIGN_IDENTITY:--}"
NOTARY_PROFILE="${MACOS_NOTARY_PROFILE:-}"

sign() {  # sign <path> [--deep]
  local path=$1; shift
  if [ "$SIGN_ID" = "-" ]; then
    codesign --force --sign - "$@" "$path"
  else
    codesign --force --options runtime --timestamp "$@" --sign "$SIGN_ID" "$path"
  fi
}
command -v dmgbuild >/dev/null || { echo "dmgbuild not found: python3 -m pip install dmgbuild" >&2; exit 1; }

WORK=$(mktemp -d)
APP="$WORK/reIDM.app"
BIN="$APP/Contents/MacOS"
mkdir -p "$BIN" "$APP/Contents/Resources"

# The window's binary, the service it starts, and the native host the browser starts, side by
# side: each finds the others next to itself.
cp "$GUI" "$BIN/reidm-gui"; cp "$REIDM" "$BIN/reidm"; cp "$HOST" "$BIN/reidm-host"
chmod 755 "$BIN"/*

# Re-sign what lipo produced; see the note at the top of this script.
for b in "$BIN/reidm" "$BIN/reidm-host" "$BIN/reidm-gui"; do
  sign "$b"
done
echo "code signatures:"
codesign -dv "$APP/Contents/MacOS/reidm-gui" 2>&1 | grep -E "^Identifier|^Signature|^TeamIdentifier" || true

# Icon: assemble an .icns from the PNGs we ship. Each slot gets the size drawn for it rather
# than a resampled large one: the small sizes are drawn differently (bolder arrow, no letters).
ICONSET=$(mktemp -d)/reidm.iconset
mkdir -p "$ICONSET"
I="$ROOT/packaging/icon"
cp "$I/reidm-16.png"   "$ICONSET/icon_16x16.png"
cp "$I/reidm-32.png"   "$ICONSET/icon_16x16@2x.png"
cp "$I/reidm-32.png"   "$ICONSET/icon_32x32.png"
cp "$I/reidm-64.png"   "$ICONSET/icon_32x32@2x.png"
cp "$I/reidm-128.png"  "$ICONSET/icon_128x128.png"
cp "$I/reidm-256.png"  "$ICONSET/icon_128x128@2x.png"
cp "$I/reidm-256.png"  "$ICONSET/icon_256x256.png"
cp "$I/reidm-512.png"  "$ICONSET/icon_256x256@2x.png"
cp "$I/reidm-512.png"  "$ICONSET/icon_512x512.png"
cp "$I/reidm-1024.png" "$ICONSET/icon_512x512@2x.png"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/reIDM.icns"

# reIDM's browser extension. The service finds it here (next to the app binary, in Resources)
# and keeps a copy the browser can load in ~/Library/Application Support.
cp -R "$ROOT/extension" "$APP/Contents/Resources/extension"

cat > "$APP/Contents/Info.plist" <<EOP
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>reIDM</string>
  <key>CFBundleDisplayName</key><string>reIDM</string>
  <key>CFBundleIdentifier</key><string>com.zektopic.reidm</string>
  <key>CFBundleVersion</key><string>$SHORT</string>
  <key>CFBundleShortVersionString</key><string>$SHORT</string>
  <key>CFBundleExecutable</key><string>reidm-gui</string>
  <key>CFBundleIconFile</key><string>reIDM.icns</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
EOP

# Sign the bundle itself, last, once its contents are final.
sign "$APP"
codesign --verify --verbose=2 "$APP" 2>&1 | tail -2

# The window's background, at 1x and 2x in one file so Retina screens get the sharp one.
# Drawn by tools/make_dmg_background.py.
tiffutil -cathidpicheck "$ROOT/packaging/macos/background.png" "$ROOT/packaging/macos/background@2x.png" \
  -out "$WORK/background.tiff"

rm -f "$OUT"
dmgbuild -s "$ROOT/packaging/macos/dmg-settings.py" \
  -D app="$APP" -D background="$WORK/background.tiff" -D volume_icon="$APP/Contents/Resources/reIDM.icns" \
  "reIDM $VER" "$OUT"

# The image itself carries a signature too when there is a real identity to sign it with.
if [ "$SIGN_ID" != "-" ]; then
  codesign --force --timestamp --sign "$SIGN_ID" "$OUT"
fi

if [ -n "$NOTARY_PROFILE" ]; then
  echo "notarizing (this can take a few minutes)…"
  xcrun notarytool submit "$OUT" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$OUT"
  xcrun stapler validate "$OUT"
  echo "notarized and stapled"
else
  echo
  echo "NOTE: this build is ad-hoc signed, not notarized."
  echo "  It runs locally, but macOS will warn on other machines until it is signed with a"
  echo "  Developer ID certificate and notarized. See docs/PACKAGING.md."
fi
