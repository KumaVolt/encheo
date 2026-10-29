"""A pretend Plex Media Server for the demo library, enough for Encheo to browse it.

    python3 demo_server.py <port> <artwork folder>

Answers the endpoints the app uses with Plex-shaped JSON; artwork comes from make_art.py's
folder. Nothing plays: there are no media files.
"""

import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlparse

import library

ART = Path(".")


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


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        url = urlparse(self.path)
        query = parse_qs(url.query)
        parts = [p for p in url.path.split("/") if p]
        start = int(self.headers.get("X-Plex-Container-Start", 0))
        size = int(self.headers.get("X-Plex-Container-Size", 10_000))

        if url.path == "/photo/:/transcode":
            name = unquote(query.get("url", [""])[0]).split("/")[-1]
            return self.file(ART / name)
        if url.path == "/library/sections":
            return self.json(container(size=len(library.SECTIONS), Directory=library.SECTIONS))
        if parts[:2] == ["library", "sections"] and len(parts) == 4:
            section = parts[2]
            if parts[3] == "all":
                items = sorted((library.ITEMS[k] for k in library.SECTION_ITEMS.get(section, [])), key=sort_title)
                return self.json(metadata(items, start, size))
            if parts[3] == "recentlyAdded":
                return self.json(metadata(library.recently_added(section), start, size))
        if url.path == "/library/onDeck":
            return self.json(metadata(library.on_deck()))
        if parts[:2] == ["library", "metadata"] and len(parts) >= 3 and parts[2] in library.ITEMS:
            key = parts[2]
            if len(parts) == 3:
                return self.json(metadata([library.ITEMS[key]]))
            if parts[3] == "children":
                return self.json(metadata(sorted(library.children(key), key=lambda i: i.get("index", 0))))
            if parts[3] == "allLeaves":
                return self.json(metadata(sorted(library.all_leaves(key), key=lambda i: (i["parentIndex"], i["index"]))))
        if parts[:2] == ["hubs", "metadata"] and len(parts) == 4 and parts[2] in library.ITEMS:
            return self.json(related(parts[2]))
        if url.path == "/hubs/search":
            q = query.get("query", [""])[0].lower()
            found = [i for i in library.ITEMS.values() if q in i["title"].lower() and i["type"] in ("movie", "show", "album")]
            return self.json(container(Hub=[{"Metadata": found[:20]}]))
        if url.path.startswith("/:/"):  # progress reports, watched/unwatched
            return self.json(container())
        self.send_error(404)

    def json(self, body):
        data = json.dumps(body).encode()
        self.send_response(200)
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
