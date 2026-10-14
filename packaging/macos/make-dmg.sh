#!/usr/bin/env bash
# Build reIDM-<version>.dmg from universal binaries.
# Usage: make-dmg.sh <version> <reidm> <reidm-host> <reidm-gui> <out.dmg>
#
# Signing: set MACOS_SIGN_IDENTITY to a "Developer ID Application: …" identity to sign for
# distribution, and MACOS_NOTARY_PROFILE to a notarytool keychain profile to notarize. With
# neither set, everything is ad-hoc signed, which is still *required*: `lipo` does not carry
# code signatures over, and the kernel refuses to execute an unsigned arm64 binary — the
# symptom is a bare "Killed: 9" with no dialog on Apple silicon.
set -euo pipefail
VER=$1; REIDM=$2; HOST=$3; GUI=$4; OUT=$5
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
STAGE=$(mktemp -d)/reIDM
APP="$STAGE/reIDM.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$STAGE/bin"

cp "$REIDM" "$STAGE/bin/reidm"; cp "$HOST" "$STAGE/bin/reidm-host"
cp "$GUI" "$APP/Contents/MacOS/reidm-gui"
chmod 755 "$STAGE/bin/reidm" "$STAGE/bin/reidm-host" "$APP/Contents/MacOS/reidm-gui"

# The daemon lives next to the app binary so `reidm-gui` can spawn it.
cp "$STAGE/bin/reidm" "$APP/Contents/MacOS/reidm"

# Re-sign what lipo produced; see the note at the top of this script.
for b in "$STAGE/bin/reidm" "$STAGE/bin/reidm-host" "$APP/Contents/MacOS/reidm" "$APP/Contents/MacOS/reidm-gui"; do
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

# reIDM's browser extension. The daemon finds it here (next to the app binary, in Resources)
# and keeps a copy the browser can load in ~/Library/Application Support.
cp -R "$ROOT/extension" "$APP/Contents/Resources/extension"
cp -R "$ROOT/extension" "$STAGE/browser-extension"

cat > "$APP/Contents/Info.plist" <<EOP
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>reIDM</string>
  <key>CFBundleDisplayName</key><string>reIDM</string>
  <key>CFBundleIdentifier</key><string>com.zektopic.reidm</string>
  <key>CFBundleVersion</key><string>$VER</string>
  <key>CFBundleShortVersionString</key><string>$VER</string>
  <key>CFBundleExecutable</key><string>reidm-gui</string>
  <key>CFBundleIconFile</key><string>reIDM.icns</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
EOP

cp "$ROOT/README.md" "$STAGE/README.md"; cp "$ROOT/CHANGELOG.md" "$STAGE/CHANGELOG.md"
mkdir -p "$STAGE/docs"; cp "$ROOT"/docs/*.md "$STAGE/docs/"

cat > "$STAGE/Install reIDM.command" <<'EOS'
#!/usr/bin/env bash
# Installs reIDM to ~/.local/bin, registers the native-messaging host and a login agent.
set -euo pipefail
cd "$(dirname "$0")"
DEST="$HOME/.local/bin"; mkdir -p "$DEST"
cp bin/reidm bin/reidm-host "$DEST/"
# The daemon in ~/.local/bin looks for the extension in ../share/reidm/extension.
mkdir -p "$HOME/.local/share/reidm"; rm -rf "$HOME/.local/share/reidm/extension"
cp -R browser-extension "$HOME/.local/share/reidm/extension"
# Keep the daemon beside the app too, so the window can start it itself.
cp bin/reidm "reIDM.app/Contents/MacOS/reidm"
xattr -dr com.apple.quarantine reIDM.app "$DEST/reidm" "$DEST/reidm-host" 2>/dev/null || true
"$DEST/reidm" install-host
PLIST="$HOME/Library/LaunchAgents/com.zektopic.reidm.plist"
mkdir -p "$(dirname "$PLIST")"
cat > "$PLIST" <<EOP
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>com.zektopic.reidm</string>
<key>ProgramArguments</key><array><string>$DEST/reidm</string><string>daemon</string></array>
<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
</dict></plist>
EOP
launchctl unload "$PLIST" 2>/dev/null || true
launchctl load "$PLIST"
echo
echo "reIDM installed."
echo "  * Drag reIDM.app to /Applications to keep it."
echo "  * Add \$HOME/.local/bin to PATH to use the 'reidm' command."
echo "  * Browser: load the extension once — see Help > Browser integration in the reIDM window."
EOS
chmod 755 "$STAGE/Install reIDM.command"

# Sign the bundle itself, last, once its contents are final.
sign "$APP"
codesign --verify --verbose=2 "$APP" 2>&1 | tail -2

# Launch the installer when the image is opened.
ln -s /Applications "$STAGE/Applications" 2>/dev/null || true

hdiutil create -volname "reIDM $VER" -srcfolder "$STAGE" -ov -format UDZO "$OUT"

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
