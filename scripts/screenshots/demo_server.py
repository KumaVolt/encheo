"""A pretend Plex Media Server for the demo library, enough for Encheo to browse it.

    python3 demo_server.py <port> <artwork folder>

Answers the endpoints the app uses with Plex-shaped JSON; artwork comes from make_art.py's
folder. There are no media files: films, episodes and trailers all play Big Buck Bunny
(Blender Foundation, CC BY 3.0), streamed from the Internet Archive's copy. Songs play a
generated tone.
"""

import io
import json
import math
import struct
import sys
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlparse

import library

ART = Path(".")
# Every film and episode: Big Buck Bunny (720p, 62 MB), from the Internet Archive.
DEMO_VIDEO = "https://archive.org/download/BigBuckBunny_124/Content/big_buck_bunny_720p_surround.mp4"


def container(**fields):
    return {"MediaContainer": fields}


def metadata(items, start=0, size=None):
    page = items[start:start + size] if size is not None else items
    return container(size=len(page), totalSize=len(items), offset=start, Metadata=page)


def sort_title(item):
    t = item["title"]
    for article in ("The ", "A "):
        if t.startswith(article):
            return t[len(article):]
    return t


def related(key):
    """Same kind, sharing a genre, best match first."""
    item = library.ITEMS[key]
    genres = {g["tag"] for g in item.get("Genre", [])}
    same = [i for i in library.ITEMS.values() if i["type"] == item["type"] and i["ratingKey"] != key]
    same.sort(key=lambda i: (-len(genres & {g["tag"] for g in i.get("Genre", [])}), i["ratingKey"]))
    return container(Hub=[{"title": "More Like This", "Metadata": same[:12]}])


def trailer(key):
    """Films have a trailer, so the details page shows its Trailer button."""
    item = library.ITEMS[key]
    if item["type"] != "movie":
        return []
    return [{
        "ratingKey": f"{key}-trailer", "key": f"/library/metadata/{key}-trailer", "type": "clip",
        "title": item["title"], "extraType": 1, "subtype": "trailer", "duration": 150000,
        "thumb": item.get("art"),
        "Media": [{"id": 1, "Part": [{"id": 1, "key": f"/library/parts/{key}-trailer/file.mp4"}]}],
    }]



_TONE = None


def tone(seconds=40, rate=22050):
    """A soft 220 Hz tone as a WAV file (made once)."""
    global _TONE
    if _TONE is None:
        buf = io.BytesIO()
        with wave.open(buf, "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(rate)
            w.writeframes(b"".join(struct.pack("<h", int(2000 * math.sin(2 * math.pi * 220 * i / rate)))
                                   for i in range(seconds * rate)))
        _TONE = buf.getvalue()
    return _TONE


def section_hubs(section):
    """Plex's suggested rows for a library (/hubs/sections/<key>): Top Rated and a genre row."""
    kind = {"1": "movie", "2": "show"}.get(section)
    if not kind:
        return []
    found = [i for i in library.ITEMS.values() if i["type"] == kind]
    by_rating = sorted(found, key=lambda i: -i.get("audienceRating", 0))
    genre = found[0]["Genre"][0]["tag"]
    in_genre = [i for i in found if any(g["tag"] == genre for g in i.get("Genre", []))]
    word = "Movies" if kind == "movie" else "TV"
    return [
        {"hubIdentifier": f"{kind}.recentlyadded.{section}", "title": "Recently Added", "Metadata": found},
        {"hubIdentifier": f"{kind}.toprated.{section}", "title": f"Top Rated {word}", "Metadata": by_rating},
        {"hubIdentifier": f"{kind}.moreingenre.{section}", "title": f"More in {genre}", "Metadata": in_genre or found},
    ]

# Plex Home people, for "Who's watching?" (the app talks to this server as plex.tv when started
# with ENCHEO_PLEX_TV / debug.encheo.plextv). Kids has the PIN 1234.
HOME_USERS = [
    {"id": 1, "uuid": "demo-alex", "title": "Alex", "admin": True, "protected": False},
    {"id": 2, "uuid": "demo-sam", "title": "Sam", "admin": False, "protected": False},
    {"id": 3, "uuid": "demo-kids", "title": "Kids", "admin": False, "protected": True, "restricted": True},
]
HOME_PINS = {"demo-kids": "1234"}


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        url = urlparse(self.path)
        query = parse_qs(url.query)
        parts = [p for p in url.path.split("/") if p]
        if parts[:3] == ["api", "v2", "home"] and len(parts) == 6 and parts[5] == "switch":
            uuid = parts[4]
            user = next((u for u in HOME_USERS if u["uuid"] == uuid), None)
            if user is None:
                return self.send_error(404)
            if uuid in HOME_PINS and query.get("pin", [""])[0] != HOME_PINS[uuid]:
                return self.json({"errors": [{"code": 1041, "message": "Invalid PIN"}]}, status=403)
            return self.json({**user, "authToken": "demo-" + uuid}, status=201)
        if url.path.startswith("/:/"):
            return self.json(container())
        self.send_error(404)

    def do_GET(self):
        url = urlparse(self.path)
        query = parse_qs(url.query)
        parts = [p for p in url.path.split("/") if p]
        start = int(self.headers.get("X-Plex-Container-Start", 0))
        size = int(self.headers.get("X-Plex-Container-Size", 10_000))

        # plex.tv stand-in: the Home's people, the account, and this server as its only server.
        if url.path == "/api/v2/home/users":
            host = self.headers.get("Host", "127.0.0.1")
            users = [dict(u) for u in HOME_USERS]
            users[0]["thumb"] = f"http://{host}/photo/:/transcode?url=/art/album-3011.jpg"
            return self.json({"subscription": {}, "users": users})
        if url.path == "/api/v2/user":
            return self.json({"id": 1, "uuid": "demo-alex", "username": "alex", "title": "Alex", "profile": {"defaultSubtitleLanguage": "en"}})
        if url.path == "/api/v2/resources":
            host = self.headers.get("Host", "127.0.0.1")
            address, _, port = host.partition(":")
            return self.json([{
                "name": "Demo", "clientIdentifier": "demo-server", "provides": "server", "owned": True,
                "accessToken": self.headers.get("X-Plex-Token", "demo"),
                "connections": [{"uri": f"http://{host}", "protocol": "http", "address": address, "port": int(port or 80), "local": True, "relay": False, "IPv6": False}],
            }])
        if url.path == "/identity":
            return self.json(container(machineIdentifier="demo-server"))

        if url.path == "/photo/:/transcode":
            name = unquote(query.get("url", [""])[0]).split("/")[-1]
            return self.file(ART / name)
        if url.path == "/library/sections":
            return self.json(container(size=len(library.SECTIONS), Directory=library.SECTIONS))
        if parts[:2] == ["library", "sections"] and len(parts) == 4:
            section = parts[2]
            if parts[3] == "all":
                kind = library.MUSIC_TYPES.get(query.get("type", [""])[0])
                if kind and section == "3":
                    items = sorted((i for i in library.ITEMS.values() if i["type"] == kind), key=sort_title)
                    return self.json(metadata(items, start, size))
                items = sorted((library.ITEMS[k] for k in library.SECTION_ITEMS.get(section, [])), key=sort_title)
                return self.json(metadata(items, start, size))
            if parts[3] == "collections":
                found = list(library.COLLECTIONS.values()) if section == "1" else []
                return self.json(metadata(found))
            if parts[3] == "recentlyAdded":
                return self.json(metadata(library.recently_added(section), start, size))
        if parts[:2] == ["library", "collections"] and len(parts) == 4 and parts[3] == "children":
            films = library.COLLECTION_ITEMS.get(parts[2], [])
            return self.json(metadata(sorted((library.ITEMS[k] for k in films), key=lambda i: i["year"])))
        if url.path == "/playlists":
            return self.json(metadata(library.PLAYLISTS))
        if parts[:1] == ["playlists"] and len(parts) == 3 and parts[2] == "items":
            return self.json(metadata([library.ITEMS[k] for k in library.PLAYLIST_TRACKS]))
        if url.path == "/library/onDeck":
            return self.json(metadata(library.on_deck()))
        if parts[:2] == ["library", "metadata"] and len(parts) >= 3 and parts[2] in library.ITEMS:
            key = parts[2]
            if len(parts) == 3:
                return self.json(metadata([library.ITEMS[key]]))
            if parts[3] == "children":
                return self.json(metadata(sorted(library.children(key), key=lambda i: i.get("index", 0))))
            if parts[3] == "extras":
                return self.json(metadata(trailer(key)))
            if parts[3] == "allLeaves":
                return self.json(metadata(sorted(library.all_leaves(key), key=lambda i: (i["parentIndex"], i["index"]))))
        if parts[:2] == ["hubs", "metadata"] and len(parts) == 4 and parts[2] in library.ITEMS:
            return self.json(related(parts[2]))
        if url.path == "/hubs/search":
            q = query.get("query", [""])[0].lower()
            found = [i for i in library.ITEMS.values() if q in i["title"].lower() and i["type"] in ("movie", "show", "album", "episode")]  # Plex lists episodes too
            return self.json(container(Hub=[{"Metadata": found[:20]}]))
        if parts[:2] == ["hubs", "sections"] and len(parts) == 3:
            return self.json(container(Hub=section_hubs(parts[2])))
        if url.path.startswith("/:/"):  # progress reports, watched/unwatched
            return self.json(container())
        if parts[:2] == ["library", "streams"] and len(parts) == 3 and parts[2].startswith("lyrics-"):
            return self.json(container(size=1, Lyrics=[library.lyrics(parts[2].removeprefix("lyrics-"))]))
        if parts[:2] == ["library", "parts"] and url.path.endswith(".mp3"):
            return self.audio()
        if parts[:2] == ["library", "parts"]:
            self.send_response(302)
            self.send_header("Location", DEMO_VIDEO)
            self.send_header("Content-Length", "0")
            return self.end_headers()
        self.send_error(404)

    def audio(self):
        """Every song is the same quiet 40-second tone, so music actually plays (and can be
        paused, skipped and controlled from Android's lock screen) in tests."""
        data = tone()
        self.send_response(200)
        self.send_header("Content-Type", "audio/wav")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def json(self, body, status=200):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def file(self, path):
        if not path.is_file():
            return self.send_error(404)
        data = path.read_bytes()
        self.send_response(200)
        self.send_header("Content-Type", "image/jpeg")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


def main():
    global ART
    port, ART = int(sys.argv[1]), Path(sys.argv[2])
    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    print(f"demo server on http://127.0.0.1:{port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
