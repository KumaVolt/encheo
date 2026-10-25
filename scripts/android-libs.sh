#!/bin/sh
# Fetch prebuilt libmpv for Android (media-kit/libmpv-android-video-build, "full" variant: all
# codecs) into apps/encheo/android-libs/<abi>/, where cargo-apk packs it into the APK.
set -e
cd "$(dirname "$0")/.."
VERSION="${LIBMPV_ANDROID_VERSION:-v1.1.11}"
OUT=apps/encheo/android-libs
TMP=$(mktemp -d)
for ABI in ${ANDROID_ABIS:-arm64-v8a}; do
  gh release download "$VERSION" --repo media-kit/libmpv-android-video-build \
    --pattern "full-$ABI.jar" --dir "$TMP" --clobber
  unzip -o -q "$TMP/full-$ABI.jar" "lib/$ABI/libmpv.so" -d "$TMP/$ABI"
  mkdir -p "$OUT/$ABI"
  cp "$TMP/$ABI/lib/$ABI/libmpv.so" "$OUT/$ABI/"
  echo "libmpv for $ABI -> $OUT/$ABI/libmpv.so"
done
rm -rf "$TMP"
