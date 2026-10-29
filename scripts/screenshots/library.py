"""The demo library behind the README screenshots: invented films, shows and albums.

Shared by demo_server.py (which serves it in Plex's JSON shapes) and make_art.py (which draws
the artwork it refers to). Every image path is /art/<name>.jpg.
"""

import random

NOW = 1_761_000_000  # "today", in Unix seconds, so the order of Recently Added never changes

PEOPLE = [
    "Ines Calder", "Tomas Reyes", "Mira Holt", "Oskar Lindqvist", "Ada Okonkwo", "Leo Marchetti",
    "Sana Iqbal", "Felix Brandt", "Noor Haddad", "Elena Petrova", "Jonah Pike", "Priya Raman",
    "Theo Laurent", "Hana Sato", "Ruben Alves", "Clara Voss", "Dev Malhotra", "Yara Nilsen",
    "Sam Whitfield", "Lucia Ortega", "Bram de Wit", "Amara Diallo", "Kenji Mori", "Rosa Ferreira",
]

ROLES = [
    "Maren", "Detective Hale", "Jonas", "Dr. Ilse Varga", "The Ferryman", "Ana", "Captain Roy",
    "Lise", "Mr. Okafor", "Tilde", "Young Maren", "Pell", "Ruth", "The Stranger", "Kit", "Emil",
]


def person_art(name):
    return "/art/person-" + name.lower().replace(" ", "-").replace(".", "") + ".jpg"


# (title, year, genres, minutes, content rating, tagline, summary)
MOVIES = [
    ("Low Tide", 2025, ["Drama", "Mystery"], 128, "PG-13", "The sea gives back what it takes.",
     "When the tide pulls further out than anyone in Kessel Bay can remember, a ferry missing for "
     "thirty years rises from the sand. Marine archivist Maren Vos returns home to catalogue the "
     "wreck and finds her own name in its logbook."),
    ("Signal Lost", 2024, ["Science Fiction", "Thriller"], 117, "PG-13", "Somebody is still listening.",
     "A night-shift radio astronomer picks up a transmission that repeats her own voice, eleven "
     "minutes before she speaks."),
    ("The Glass Orchard", 2023, ["Fantasy", "Adventure"], 134, "PG", "Every fruit holds a year.",
     "Two siblings inherit an orchard whose glass apples each contain a single year of someone's life."),
    ("Kestrel", 2025, ["Action", "Thriller"], 112, "R", "Fly fast. Land never.",
     "A grounded courier pilot takes one last cross-border run with a passenger who won't say why "
     "three governments want her back."),
    ("Paper Moons", 2022, ["Comedy", "Romance"], 101, "PG-13", "Love, folded twice.",
     "A paper-crafts influencer and the grumpy printer who supplies her stock are forced to share a "
     "booth at the world's smallest trade fair."),
    ("Copper Sky", 2024, ["Western"], 141, "R", "The last town before the edge of the map.",
     "A widowed surveyor and a former outlaw race a mining company to the only water in the valley."),
    ("Orbit of Small Things", 2024, ["Animation", "Family"], 94, "G", "Even dust has somewhere to be.",
     "A lost screw aboard a space station sets out to find the machine it belongs to."),
    ("A House by the Fjord", 2021, ["Drama"], 122, "PG-13", "Some winters never end.",
     "Three estranged sisters spend one last winter in their late mother's house."),
    ("Midnight Diner on Route 9", 2023, ["Comedy", "Crime"], 99, "R", "Open all night. Questions extra.",
     "A roadside diner's night crew covers for a regular who may have robbed a bank."),
    ("The Cartographer", 2022, ["Adventure", "History"], 138, "PG-13", "Map the world. Lose yourself.",
     "In 1820, a mapmaker charting an unknown coast starts finding his own maps already there."),
    ("Static Bloom", 2025, ["Horror"], 97, "R", "Don't let it take root.",
     "A botanist's prize orchid starts broadcasting on the baby monitor."),
    ("Saltwater Hearts", 2023, ["Romance", "Drama"], 109, "PG-13", "Two shores. One summer.",
     "A lifeguard and a visiting marine biologist count seals and avoid the obvious."),
    ("Velvet Hour", 2021, ["Crime", "Mystery"], 118, "R", "Every alibi has a soundtrack.",
     "A jazz pianist plays the night a club owner disappears, and becomes the only witness."),
    ("Northbound", 2022, ["Documentary"], 88, "NR", "4,000 kilometres on foot.",
     "Following the caribou migration across the tundra, one season at a time."),
]

# (title, year, genres, content rating, summary, seasons, episodes per season)
SHOWS = [
    ("Harbour Lights", 2021, ["Drama", "Crime"], "TV-14",
     "In a fishing town where everyone keeps a secret, a harbour master's daughter becomes the town's "
     "first detective in forty years.", 3, 8),
    ("The Quiet Engineers", 2023, ["Science Fiction", "Drama"], "TV-14",
     "The maintenance crew of a generation ship discovers the voyage has been over for a century.", 2, 10),
    ("Kitchen Brigade", 2022, ["Comedy"], "TV-14",
     "A three-star kitchen, one terrible new owner and a staff with nothing left to lose.", 2, 8),
    ("Field Notes", 2024, ["Documentary"], "TV-PG",
     "Naturalists spend a year in one square kilometre of wilderness.", 1, 6),
    ("Deep Station", 2025, ["Thriller", "Mystery"], "TV-MA",
     "Six researchers, one underwater lab, and a seventh set of footprints.", 1, 8),
    ("Wildflower County", 2020, ["Drama"], "TV-PG",
     "Three generations of a farming family weather drought, debt and each other.", 2, 10),
]

EPISODE_TITLES = [
    "The Long Way Round", "Undertow", "What the Gulls Saw", "Low Water", "Old Debts",
    "Lantern", "The Crossing", "Night Shift", "Small Hours", "Breakwater", "Salt", "First Light",
    "Fog Bank", "The Visitor", "Ballast", "Dead Reckoning", "Slack Tide", "Homecoming", "Northern Light",
]

# (artist, [(album, year, genre, [tracks])])
MUSIC = [
    ("Marlow & The Tides", [
        ("Undertow", 2025, "Indie Rock", ["Harbour Song", "Undertow", "Glass Bottles", "Paper Boats",
         "Low Light", "Second Wind", "Kessel Bay", "The Long Way Home", "Salt in the Wound", "Lighthouse"]),
        ("Lantern Season", 2022, "Indie Rock", ["Lantern", "North Wall", "Cold Hands", "Driftwood",
         "Seventeen", "Tidal", "Wintering", "After the Storm"]),
    ]),
    ("Juno Vale", [
        ("Glasshouse", 2024, "Electronic", ["Prism", "Glasshouse", "Chroma", "Night Garden", "Afterimage",
         "Soft Focus", "Halation", "Bloom"]),
    ]),
    ("Ostinato Ensemble", [
        ("Night Studies", 2023, "Classical", ["Nocturne I", "Nocturne II", "Étude in Blue", "Rain Study",
         "Nocturne III", "Lullaby for a City", "Coda"]),
    ]),
    ("The Paper Planes", [
        ("Weekend Static", 2024, "Pop", ["Static", "Weekend", "Radio Heart", "Polaroid", "Summer Again",
         "Neon Pool", "Overdrive", "Last Call", "Sunday"]),
    ]),
    ("Kai Okafor", [
        ("Low Sun", 2025, "Soul", ["Low Sun", "Golden Hour", "Easy", "Slow Burn", "Hold On", "Amber",
         "Late Bloom", "Morning"]),
    ]),
]


def _rng(key):
    return random.Random(f"encheo-demo-{key}")


def _cast(key, n=12):
    r = _rng(key)
    names = r.sample(PEOPLE, n)
    roles = r.sample(ROLES, n)
    return [{"tag": p, "role": q, "thumb": person_art(p)} for p, q in zip(names, roles)]


def _ratings(key):
    r = _rng(key)
    imdb = round(r.uniform(6.6, 8.7), 1)
    critics = round(min(9.9, imdb + r.uniform(-0.6, 1.4)), 1)
    audience = round(min(9.9, imdb + r.uniform(-0.4, 1.0)), 1)
    return [
        {"image": "imdb://image.rating", "value": imdb, "type": "audience"},
        {"image": "rottentomatoes://image.rating.ripe", "value": critics, "type": "critic"},
        {"image": "rottentomatoes://image.rating.upright", "value": audience, "type": "audience"},
    ]


def _media(key, minutes, uhd):
    r = _rng(key)
    size = int(r.uniform(28, 62) * 1e9) if uhd else int(r.uniform(6, 14) * 1e9)
    video = {"id": int(key) * 10 + 2, "index": 0, "streamType": 1, "codec": "hevc" if uhd else "h264", "displayTitle": "4K" if uhd else "1080p"}
    if uhd:
        video["colorTrc"] = "smpte2084"
    return [{
        "id": int(key) * 10,
        "duration": minutes * 60_000,
        "bitrate": int(size * 8 / (minutes * 60) / 1000),
        "width": 3840 if uhd else 1920,
        "height": 2160 if uhd else 1080,
        "videoResolution": "4k" if uhd else "1080",
        "videoCodec": "hevc" if uhd else "h264",
        "audioCodec": "truehd" if uhd else "eac3",
        "audioChannels": 8 if uhd else 6,
        "container": "mkv",
        "videoFrameRate": "24p",
        "Part": [{
            "id": int(key) * 10 + 1,
            "key": f"/library/parts/{key}/file.mkv",
            "file": f"/media/{key}.mkv",
            "size": size,
            "container": "mkv",
            "Stream": [video, {"id": int(key) * 10 + 3, "index": 1, "streamType": 2, "codec": "truehd" if uhd else "eac3", "channels": 8 if uhd else 6,
                               "languageCode": "eng", "displayTitle": "English"}],
        }],
    }]


def build():
    """Everything, as Plex metadata dicts, plus the library sections and which ids are in each."""
    items = {}
    sections = {"1": [], "2": [], "3": []}

    # Films: newest additions first, a few watched, one half-way through.
    for n, (title, year, genres, minutes, rating, tagline, summary) in enumerate(MOVIES):
        key = str(101 + n)
        cast = _cast(key)
        item = {
            "ratingKey": key, "key": f"/library/metadata/{key}", "guid": f"demo://movie/{key}", "type": "movie",
            "title": title, "year": year, "summary": summary, "tagline": tagline, "contentRating": rating,
            "audienceRating": _ratings(key)[2]["value"], "duration": minutes * 60_000,
            "thumb": f"/art/poster-{key}.jpg", "art": f"/art/backdrop-{key}.jpg",
            "addedAt": NOW - n * 86_400 * 3,
            "Genre": [{"tag": g} for g in genres],
            "Director": [{"tag": cast[0]["tag"], "thumb": cast[0]["thumb"]}],
            "Writer": [{"tag": cast[1]["tag"], "thumb": cast[1]["thumb"]}],
            "Role": cast[2:],
            "Rating": _ratings(key),
            "Media": _media(key, minutes, uhd=n % 3 != 2),
        }
        if n in (5, 7, 9, 12):
            item["viewCount"] = 1
        if title in ("Signal Lost", "The Glass Orchard"):
            item["viewOffset"] = int(minutes * 60_000 * (0.62 if n == 1 else 0.3))
        items[key] = item
        sections["1"].append(key)

    # Shows, their seasons and episodes.
    for n, (title, year, genres, rating, summary, seasons, per_season) in enumerate(SHOWS):
        key = str(201 + n)
        show_thumb, show_art = f"/art/poster-{key}.jpg", f"/art/backdrop-{key}.jpg"
        cast = _cast(key)
        r = _rng(key)
        leaves = seasons * per_season
        items[key] = {
            "ratingKey": key, "key": f"/library/metadata/{key}/children", "guid": f"demo://show/{key}", "type": "show",
            "title": title, "year": year, "summary": summary, "contentRating": rating,
            "thumb": show_thumb, "art": show_art, "childCount": seasons, "leafCount": leaves,
            "viewedLeafCount": 0, "addedAt": NOW - n * 86_400 * 5,
            "Genre": [{"tag": g} for g in genres], "Role": cast, "Rating": _ratings(key),
        }
        sections["2"].append(key)
        titles = r.sample(EPISODE_TITLES, len(EPISODE_TITLES))
        watched_through = {0: 11, 1: 2, 4: 5}.get(n, 0)  # Harbour Lights: into season 2
        for s in range(1, seasons + 1):
            skey = f"{key}{s}"
            items[skey] = {
                "ratingKey": skey, "key": f"/library/metadata/{skey}/children", "type": "season", "title": f"Season {s}",
                "index": s, "parentRatingKey": key, "parentTitle": title, "thumb": show_thumb, "art": show_art,
                "leafCount": per_season,
            }
            for e in range(1, per_season + 1):
                ekey = f"{skey}{e:02d}"
                number = (s - 1) * per_season + e
                minutes = r.randint(42, 58)
                ep = {
                    "ratingKey": ekey, "key": f"/library/metadata/{ekey}", "type": "episode",
                    "title": titles[(number - 1) % len(titles)] if number > 1 else "Pilot",
                    "index": e, "parentIndex": s, "parentRatingKey": skey, "parentTitle": f"Season {s}",
                    "grandparentRatingKey": key, "grandparentTitle": title,
                    "grandparentThumb": show_thumb, "grandparentArt": show_art,
                    "thumb": f"/art/still-{ekey}.jpg", "art": show_art,
                    "summary": r.choice([
                        "An old favour comes due at the worst possible moment.",
                        "A storm traps everyone in one place for the night, and tempers run out before the power does.",
                        "A stranger arrives with a story nobody wants to believe.",
                        "The investigation turns up a name no one has said aloud in years.",
                        "Plans fall apart, then come together in the strangest way.",
                        "Someone finally tells the truth. It doesn't help.",
                    ]),
                    "duration": minutes * 60_000, "addedAt": NOW - n * 86_400 * 5 - number * 600,
                    "Media": _media(ekey, minutes, uhd=False),
                }
                if number <= watched_through:
                    ep["viewCount"] = 1
                elif number == watched_through + 1 and watched_through:
                    ep["viewOffset"] = int(minutes * 60_000 * (0.35 if n == 0 else 0.7))
                items[ekey] = ep
        items[key]["viewedLeafCount"] = watched_through

    # Artists, albums, tracks.
    for a, (artist, albums) in enumerate(MUSIC):
        akey = str(301 + a)
        items[akey] = {
            "ratingKey": akey, "key": f"/library/metadata/{akey}/children", "type": "artist", "title": artist,
            "thumb": f"/art/album-{akey}1.jpg", "art": f"/art/backdrop-{akey}.jpg",
            "summary": f"{artist} make records in a converted boathouse and tour mostly by train.",
            "Genre": [{"tag": albums[0][2]}], "addedAt": NOW - a * 86_400 * 4,
        }
        sections["3"].append(akey)
        for b, (album, year, genre, tracks) in enumerate(albums):
            bkey = f"{akey}{b + 1}"
            r = _rng(bkey)
            items[bkey] = {
                "ratingKey": bkey, "key": f"/library/metadata/{bkey}/children", "type": "album", "title": album,
                "year": year, "parentRatingKey": akey, "parentTitle": artist,
                "thumb": f"/art/album-{bkey}.jpg", "art": f"/art/backdrop-{akey}.jpg",
                "summary": f"{album} is the {'debut' if b else 'latest'} album by {artist}.",
                "Genre": [{"tag": genre}], "leafCount": len(tracks), "addedAt": NOW - a * 86_400 * 4 - b * 86_400 * 200,
            }
            for t, name in enumerate(tracks):
                tkey = f"{bkey}{t + 1:02d}"
                seconds = r.randint(150, 330)
                items[tkey] = {
                    "ratingKey": tkey, "key": f"/library/metadata/{tkey}", "type": "track", "title": name,
                    "index": t + 1, "parentIndex": 1, "parentRatingKey": bkey, "parentTitle": album,
                    "parentThumb": f"/art/album-{bkey}.jpg", "grandparentRatingKey": akey, "grandparentTitle": artist,
                    "thumb": f"/art/album-{bkey}.jpg", "duration": seconds * 1000,
                }

    return items, sections


ITEMS, SECTION_ITEMS = build()

SECTIONS = [
    {"key": "1", "title": "Movies", "type": "movie"},
    {"key": "2", "title": "TV Shows", "type": "show"},
    {"key": "3", "title": "Music", "type": "artist"},
]


def children(key):
    return [i for i in ITEMS.values() if i.get("parentRatingKey") == key]


def all_leaves(show_key):
    return [i for i in ITEMS.values() if i["type"] == "episode" and i.get("grandparentRatingKey") == show_key]


def on_deck():
    """In progress: films and episodes with a resume point."""
    started = [i for i in ITEMS.values() if i.get("viewOffset") and i["type"] in ("movie", "episode")]
    return sorted(started, key=lambda i: (i["type"] == "movie", i["ratingKey"]))


def recently_added(section):
    if section == "1":
        found = [ITEMS[k] for k in SECTION_ITEMS["1"]]
    elif section == "2":
        # Plex lists new episodes for a TV library; the app groups them by show.
        found = [i for i in ITEMS.values() if i["type"] == "episode" and i["parentIndex"] == SHOWS_LAST_SEASON[i["grandparentRatingKey"]]]
    else:
        found = [i for i in ITEMS.values() if i["type"] == "album"]
    return sorted(found, key=lambda i: -i["addedAt"])


SHOWS_LAST_SEASON = {i["ratingKey"]: i["childCount"] for i in ITEMS.values() if i["type"] == "show"}


def images():
    """Every artwork path the library refers to, with what kind of picture it is."""
    out = {}
    for i in ITEMS.values():
        for field in ("thumb", "art", "parentThumb", "grandparentThumb", "grandparentArt"):
            p = i.get(field)
            if p:
                out[p] = p.split("/")[-1].split("-")[0]
        for r in i.get("Role", []) + i.get("Director", []) + i.get("Writer", []):
            out[r["thumb"]] = "person"
    return out
