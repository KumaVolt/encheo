#!/bin/sh
# Wrap the debug binary in a minimal macOS app bundle (target/Encheo.app) and launch it,
# so it behaves like a real app (Dock icon, focus, accessibility/automation tools).
set -e
cd "$(dirname "$0")/.."
cargo build -p encheo
APP=target/Encheo.app
mkdir -p "$APP/Contents/MacOS"
# Replace, don't overwrite: macOS caches code signatures per file and kills a binary whose
# bytes changed under an existing signature ("Code Signature Invalid").
rm -f "$APP/Contents/MacOS/Encheo"
cp target/debug/encheo "$APP/Contents/MacOS/Encheo"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Encheo</string>
  <key>CFBundleDisplayName</key><string>Encheo</string>
  <key>CFBundleIdentifier</key><string>app.encheo.dev</string>
  <key>CFBundleExecutable</key><string>Encheo</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
codesign --force --sign - "$APP" 2>/dev/null
pkill -x Encheo 2>/dev/null || true
pkill -x encheo 2>/dev/null || true
LOG="${ENCHEO_LOG:-/tmp/encheo.log}"
ENCHEO_DEBUG_KEYS="${ENCHEO_DEBUG_KEYS:-}" RUST_BACKTRACE=1 nohup "$APP/Contents/MacOS/Encheo" >"$LOG" 2>&1 &
disown 2>/dev/null || true
echo "running, log: $LOG"
