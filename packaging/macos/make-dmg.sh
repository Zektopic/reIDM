#!/usr/bin/env bash
# Build reIDM-<version>.dmg from universal binaries. Usage: make-dmg.sh <version> <reidm> <reidm-host> <out.dmg>
set -euo pipefail
VER=$1; REIDM=$2; HOST=$3; OUT=$4
STAGE=$(mktemp -d)/reIDM
mkdir -p "$STAGE"
cp "$REIDM" "$STAGE/reidm"; cp "$HOST" "$STAGE/reidm-host"
chmod 755 "$STAGE/reidm" "$STAGE/reidm-host"
cp README.md "$STAGE/README.md"; cp docs/EXTENSION.md "$STAGE/EXTENSION.md"
cat > "$STAGE/Install reIDM.command" <<'EOS'
#!/usr/bin/env bash
# Installs reIDM to ~/.local/bin, registers the native-messaging host and a login agent.
set -euo pipefail
cd "$(dirname "$0")"
DEST="$HOME/.local/bin"; mkdir -p "$DEST"
cp reidm reidm-host "$DEST/"
xattr -d com.apple.quarantine "$DEST/reidm" "$DEST/reidm-host" 2>/dev/null || true
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
echo "reIDM installed. Add \$HOME/.local/bin to PATH to use the 'reidm' command."
EOS
chmod 755 "$STAGE/Install reIDM.command"
hdiutil create -volname "reIDM $VER" -srcfolder "$STAGE" -ov -format UDZO "$OUT"
