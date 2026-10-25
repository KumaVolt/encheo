#!/usr/bin/env bash
# Build Encheo for iOS. `scripts/ios.sh sim [device]` runs it in a headless simulator and saves a
# screenshot to target/ios/screen.png; `scripts/ios.sh device` builds for iPhones/iPads.
set -euo pipefail
cd "$(dirname "$0")/.."
mode=${1:-sim}
device=${2:-iPhone 17 Pro}
case $mode in
  sim) target=aarch64-apple-ios-sim ;;
  device) target=aarch64-apple-ios ;;
  *) echo "usage: $0 sim|device [device name]" >&2; exit 1 ;;
esac
profile=${PROFILE:-debug}
flags=(); [ "$profile" = release ] && flags=(--release)
cargo build -p encheo --bin encheo --target "$target" ${flags[@]+"${flags[@]}"}

app=target/ios/$mode/Encheo.app
rm -rf "$app" && mkdir -p "$app"
cp "target/$target/$profile/encheo" "$app/encheo"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
sed "s/VERSION/$version/g" apps/encheo/ios/Info.plist > "$app/Info.plist"
codesign --force --sign - "$app" >/dev/null

[ "$mode" = sim ] || { echo "built $app (sign it with your team to install)"; exit 0; }
udid=$(xcrun simctl list devices available | grep -F "$device (" | head -1 | grep -oE '[0-9A-F-]{36}')
# Boot without opening the Simulator app: it runs headless.
xcrun simctl boot "$udid" 2>/dev/null || true
xcrun simctl bootstatus "$udid" >/dev/null
xcrun simctl install "$udid" "$app"
xcrun simctl terminate "$udid" app.encheo 2>/dev/null || true
SIMCTL_CHILD_ENCHEO_PLAY="${PLAY:-}" xcrun simctl launch "$udid" app.encheo
sleep "${WAIT:-6}"
xcrun simctl io "$udid" screenshot target/ios/screen.png >/dev/null
echo "screenshot: target/ios/screen.png"
