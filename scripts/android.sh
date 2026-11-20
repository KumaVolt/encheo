#!/bin/sh
# Build Encheo for Android (arm64) with Gradle. cargo-ndk builds the Rust app (libencheo.so) into
# the Gradle project (apps/encheo/android) next to libmpv.so, then Gradle packages the APK.
#   scripts/android.sh          debug APK
#   scripts/android.sh run      debug APK, installed and started on the connected device/emulator
#   scripts/android.sh release  release APK and Play Store bundle (.aab); signed when
#                               ENCHEO_KEYSTORE (and its passwords, see android/app/build.gradle.kts) is set
set -e
cd "$(dirname "$0")/.."
mode=${1:-debug}
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-$(ls -d "$ANDROID_HOME"/ndk/*/ | sort -V | tail -1)}}"
# Slint compiles a small Java helper against android.jar; cargo-ndk's platform (26) has none here.
export ANDROID_JAR="${ANDROID_JAR:-$(ls -d "$ANDROID_HOME"/platforms/android-*/ | sort -V | tail -1)android.jar}"
[ -n "$JAVA_HOME" ] || export JAVA_HOME="$(/usr/libexec/java_home 2>/dev/null || true)"
command -v cargo-ndk >/dev/null || { echo "needs cargo-ndk: cargo install cargo-ndk" >&2; exit 1; }
[ -f apps/encheo/android-libs/arm64-v8a/libmpv.so ] || scripts/android-libs.sh
export MPV_LIB_DIR="$PWD/apps/encheo/android-libs/arm64-v8a"

jni=apps/encheo/android/app/src/main/jniLibs
flags=""
[ "$mode" = release ] && flags="--release"
cargo ndk -t arm64-v8a --platform 26 -o "$jni" build -p encheo --lib $flags
cp apps/encheo/android-libs/arm64-v8a/libmpv.so "$jni/arm64-v8a/"

cd apps/encheo/android
if [ "$mode" = release ]; then
  ./gradlew --quiet assembleRelease bundleRelease
  ls app/build/outputs/apk/release/*.apk app/build/outputs/bundle/release/*.aab
  exit 0
fi
./gradlew --quiet assembleDebug
apk=apps/encheo/android/app/build/outputs/apk/debug/app-debug.apk
echo "built $apk"
if [ "$mode" = run ]; then
  adb install -r --no-incremental app/build/outputs/apk/debug/app-debug.apk
  adb shell am start -n app.encheo/.EncheoActivity
fi
