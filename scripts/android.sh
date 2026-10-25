#!/bin/sh
# Build the Android APK (arm64). `scripts/android.sh run` also installs and starts it on the
# connected device or emulator.
set -e
cd "$(dirname "$0")/.."
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export ANDROID_NDK_ROOT="${ANDROID_NDK_ROOT:-$(ls -d "$ANDROID_HOME"/ndk/*/ | sort -V | tail -1)}"
[ -n "$JAVA_HOME" ] || export JAVA_HOME="$(/usr/libexec/java_home 2>/dev/null || true)"
[ -f apps/encheo/android-libs/arm64-v8a/libmpv.so ] || scripts/android-libs.sh
export MPV_LIB_DIR="$PWD/apps/encheo/android-libs/arm64-v8a"
cd apps/encheo
if [ "$1" = "run" ]; then
  cargo apk run --lib --target aarch64-linux-android
else
  cargo apk build --lib --target aarch64-linux-android
fi
