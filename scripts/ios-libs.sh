#!/usr/bin/env bash
# Fetch libmpv and its dependencies for iOS: MPVKit's LGPL xcframeworks (github.com/mpvkit),
# checked against the checksums in MPVKit's Package.swift. Into apps/encheo/ios-libs (not in git).
set -euo pipefail
cd "$(dirname "$0")/.."
version=1.0.0
out=apps/encheo/ios-libs
mkdir -p "$out/zips"
manifest=$(curl -fsSL "https://raw.githubusercontent.com/mpvkit/MPVKit/$version/Package.swift")
# Pairs of url + checksum for the non-GPL binary targets.
echo "$manifest" | tr -d ' \n' | grep -oE 'url:"[^"]+",checksum:"[0-9a-f]+"' | grep -v -- '-GPL' |
while IFS= read -r line; do
  url=$(echo "$line" | sed -E 's/url:"([^"]+)".*/\1/')
  sum=$(echo "$line" | sed -E 's/.*checksum:"([0-9a-f]+)"/\1/')
  zip=$out/zips/$(basename "$url")
  name=$(basename "$url" .zip)
  [ -d "$out/$name" ] && continue
  [ -f "$zip" ] || curl -fsSL -o "$zip" "$url"
  got=$(shasum -a 256 "$zip" | cut -d' ' -f1)
  [ "$got" = "$sum" ] || { echo "checksum mismatch: $zip" >&2; rm -f "$zip"; exit 1; }
  unzip -q -o "$zip" -d "$out"
  rm -f "$zip"
  echo "ok $name"
done
