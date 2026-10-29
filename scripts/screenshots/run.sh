#!/bin/sh
# Make the README screenshots: draw the demo library's artwork, serve the library as a pretend
# Plex server and let the app photograph its main screens (ENCHEO_SCREENSHOTS).
#
#   scripts/screenshots/run.sh [encheo binary] [output folder]
#
# Defaults: target/debug/encheo, docs/screenshots. Needs Python 3 with Pillow (set PYTHON to use
# another interpreter). Your own Encheo settings are left alone: the app gets a separate data folder.
set -e
cd "$(dirname "$0")/../.."
BIN="${1:-target/debug/encheo}"
OUT="${2:-docs/screenshots}"
PYTHON="${PYTHON:-python3}"
PORT="${PORT:-32499}"
WORK="$(mktemp -d)"
trap 'kill $SERVER 2>/dev/null || true; rm -rf "$WORK"' EXIT

"$PYTHON" scripts/screenshots/make_art.py "$WORK/art"
"$PYTHON" scripts/screenshots/demo_server.py "$PORT" "$WORK/art" &
SERVER=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do
    curl -fs "http://127.0.0.1:$PORT/library/sections" >/dev/null && break
    sleep 0.5
done

mkdir -p "$WORK/data"
cat > "$WORK/data/session.json" <<JSON
{
  "client_id": "encheo-screenshots",
  "token": "demo",
  "server": {
    "name": "Media Server",
    "client_identifier": "demo",
    "base_url": "http://127.0.0.1:$PORT",
    "token": "demo"
  }
}
JSON

ENCHEO_DATA_DIR="$WORK/data" ENCHEO_SCREENSHOTS="$OUT" "$BIN"
