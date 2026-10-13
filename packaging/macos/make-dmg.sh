#!/usr/bin/env bash
# Build reIDM-<version>.dmg from universal binaries.
# Usage: make-dmg.sh <version> <reidm> <reidm-host> <reidm-gui> <out.dmg>
set -euo pipefail
VER=$1; REIDM=$2; HOST=$3; GUI=$4; OUT=$5
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
STAGE=$(mktemp -d)/reIDM
APP="$STAGE/reIDM.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$STAGE/bin"

cp "$REIDM" "$STAGE/bin/reidm"; cp "$HOST" "$STAGE/bin/reidm-host"
cp "$GUI" "$APP/Contents/MacOS/reidm-gui"
chmod 755 "$STAGE/bin/reidm" "$STAGE/bin/reidm-host" "$APP/Contents/MacOS/reidm-gui"

# The daemon lives next to the app binary so `reidm-gui` can spawn it.
cp "$STAGE/bin/reidm" "$APP/Contents/MacOS/reidm"

# Icon: assemble an .icns from the PNGs we ship.
ICONSET=$(mktemp -d)/reidm.iconset
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s "$ROOT/packaging/icon/reidm-512.png" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s*2)) $((s*2)) "$ROOT/packaging/icon/reidm-512.png" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/reIDM.icns"

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

cp "$ROOT/README.md" "$STAGE/README.md"; cp "$ROOT/docs/EXTENSION.md" "$STAGE/EXTENSION.md"; cp "$ROOT/CHANGELOG.md" "$STAGE/CHANGELOG.md"

cat > "$STAGE/Install reIDM.command" <<'EOS'
#!/usr/bin/env bash
# Installs reIDM to ~/.local/bin, registers the native-messaging host and a login agent.
set -euo pipefail
cd "$(dirname "$0")"
DEST="$HOME/.local/bin"; mkdir -p "$DEST"
cp bin/reidm bin/reidm-host "$DEST/"
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
EOS
chmod 755 "$STAGE/Install reIDM.command"

# Launch the installer when the image is opened.
ln -s /Applications "$STAGE/Applications" 2>/dev/null || true

hdiutil create -volname "reIDM $VER" -srcfolder "$STAGE" -ov -format UDZO "$OUT"
