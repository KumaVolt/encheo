"""Draw placeholder artwork for the demo library: posters, backdrops, episode stills, album covers
and cast photos. Landscapes of gradients and silhouettes, no real films or people.

    python3 make_art.py <output folder>

Needs Pillow. The same title always gets the same picture.
"""

import math
import random
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

import library

# (sky top, sky bottom, sun, far layer, near layer)
PALETTES = [
    ("#0b1d3a", "#e9826b", "#ffd9a8", "#5a3d63", "#1a1530"),  # dusk over water
    ("#071a2b", "#2f7f8f", "#e8f4f0", "#1c4a5a", "#081822"),  # northern teal
    ("#2a0f2e", "#f2a65a", "#fff1c9", "#8a3b4a", "#2b0f24"),  # desert sunset
    ("#0f2c25", "#9cc9a5", "#f7f3d6", "#3e6b55", "#0d1f1a"),  # forest morning
    ("#1b1033", "#6c4bc2", "#ffc6e0", "#3a2a78", "#120a24"),  # violet night
    ("#3a1606", "#e3612b", "#ffe2a1", "#8c2d14", "#2a0d05"),  # copper
    ("#0a0f1f", "#3c5b9e", "#dfe8ff", "#26365e", "#070b16"),  # moonlight
    ("#14322f", "#e9c46a", "#fff6da", "#2a6f63", "#0b1f1c"),  # gold and green
    ("#250d1a", "#d65a7a", "#ffe0e8", "#6e2240", "#1a0812"),  # rose
    ("#11161c", "#8a9aa8", "#f4f1ea", "#4a5866", "#0c1014"),  # fog
    ("#062436", "#47b7c9", "#f0fbff", "#14637a", "#041a27"),  # lagoon
    ("#2b1a0b", "#c9a66b", "#fff4dc", "#6b4a26", "#1d1208"),  # sepia
]


def rng(key):
    return random.Random(f"art-{key}")


def hex_rgb(h):
    h = h.lstrip("#")
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))


def mix(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def gradient(w, h, top, bottom):
    img = Image.new("RGB", (1, h))
    for y in range(h):
        img.putpixel((0, y), mix(top, bottom, (y / max(1, h - 1)) ** 1.4))
    return img.resize((w, h))


def ridge(r, w, h, base, rough, points=9):
    """A mountain/hill outline: midpoint displacement across the width."""
    ys = [base + r.uniform(-rough, rough) for _ in range(2)]
    for _ in range(points):
        nxt = []
        for a, b in zip(ys, ys[1:]):
            nxt += [a, (a + b) / 2 + r.uniform(-rough, rough)]
        nxt.append(ys[-1])
        ys = nxt
        rough *= 0.55
        if len(ys) > w // 2:
            break
    step = w / (len(ys) - 1)
    return [(i * step, y) for i, y in enumerate(ys)] + [(w, h), (0, h)]


def waves(r, w, h, base, amp):
    phase, freq = r.uniform(0, 6.3), r.uniform(1.5, 3.5)
    pts = [(x, base + amp * math.sin(phase + freq * x / w * math.tau) + amp * 0.4 * math.sin(phase * 2 + 7 * x / w * math.tau))
           for x in range(0, w + 8, 8)]
    return pts + [(w, h), (0, h)]


def skyline(r, w, h, base):
    pts, x = [(0, h)], 0
    while x < w:
        bw = r.uniform(0.03, 0.09) * w
        top = base - r.uniform(0, 0.22) * h
        pts += [(x, top), (x + bw, top)]
        x += bw
    return pts + [(w, h)]


def trees(draw, r, w, h, base, color):
    x = -20
    while x < w + 20:
        th = r.uniform(0.08, 0.2) * h
        tw = th * r.uniform(0.3, 0.45)
        y = base + r.uniform(-0.02, 0.03) * h
        draw.polygon([(x, y - th), (x - tw / 2, y), (x + tw / 2, y)], fill=color)
        x += tw * r.uniform(0.35, 0.7)
    draw.rectangle([0, base + 0.02 * h, w, h], fill=color)


def scene(key, w, h, palette_key=None):
    """A landscape for `key`; `palette_key` (a show) keeps episode stills in the show's colours."""
    r = rng(key)
    pr = rng(palette_key or key)
    top, bottom, sun, far, near = (hex_rgb(c) for c in pr.choice(PALETTES))
    s = 2  # draw at twice the size and scale down, for smooth edges
    W, H = w * s, h * s
    img = gradient(W, H, top, bottom)

    # Decide the composition first, so a title's poster and backdrop show the same scene.
    kind = r.choice(["mountains", "mountains", "sea", "hills", "city", "forest"])
    starry = sum(top) < 120 and r.random() < 0.5
    stars = random.Random(f"stars-{key}")

    if starry:
        d = ImageDraw.Draw(img)
        for _ in range(int(W * H / 9000)):
            x, y = stars.uniform(0, W), stars.uniform(0, H * 0.5)
            d.ellipse([x, y, x + s * 1.6, y + s * 1.6], fill=mix(top, (255, 255, 255), stars.uniform(0.3, 1)))

    # Sun or moon with a soft glow.
    cx, cy = r.uniform(0.2, 0.8) * W, r.uniform(0.25, 0.55) * H
    rad = r.uniform(0.06, 0.13) * max(W, H)
    glow = Image.new("RGB", (W, H), (0, 0, 0))
    ImageDraw.Draw(glow).ellipse([cx - rad * 2.6, cy - rad * 2.6, cx + rad * 2.6, cy + rad * 2.6], fill=mix(sun, bottom, 0.5))
    glow = glow.filter(ImageFilter.GaussianBlur(rad * 1.2))
    img = Image.blend(img, Image.composite(glow, img, glow.convert("L")), 0.55)
    d = ImageDraw.Draw(img)
    d.ellipse([cx - rad, cy - rad, cx + rad, cy + rad], fill=sun)

    # Three to four layers, far to near.
    layers = r.randint(3, 4)
    for i in range(layers):
        t = i / (layers - 1)
        color = mix(mix(far, bottom, 0.35), near, t)
        base = H * (0.55 + 0.12 * t + r.uniform(-0.04, 0.04))
        if kind == "sea" and i > 0:
            pts = waves(r, W, H, base + H * 0.05, H * 0.012 * (1 + t))
        elif kind == "city" and i == layers - 1:
            pts = skyline(r, W, H, base)
        elif kind == "forest" and i >= layers - 2:
            trees(d, r, W, H, base, color)
            continue
        else:
            pts = ridge(r, W, H, base, H * (0.16 if kind == "mountains" else 0.06) * (1 - 0.5 * t))
        d.polygon(pts, fill=color)
        # Haze between layers.
        haze = Image.new("RGB", (W, H), bottom)
        mask = gradient(W, H, (0, 0, 0), (60, 60, 60)).convert("L")
        img = Image.composite(haze, img, mask.point(lambda v: v * (1 - t)))
        d = ImageDraw.Draw(img)

    img = img.resize((w, h), Image.LANCZOS)
    return grain(vignette(img), r)


def vignette(img):
    w, h = img.size
    mask = Image.new("L", (w, h), 0)
    ImageDraw.Draw(mask).ellipse([-w * 0.25, -h * 0.2, w * 1.25, h * 1.2], fill=255)
    mask = mask.filter(ImageFilter.GaussianBlur(min(w, h) * 0.18))
    return Image.composite(img, Image.new("RGB", (w, h), (0, 0, 0)), mask.point(lambda v: 90 + v * 165 // 255))


def grain(img, r):
    noise = Image.effect_noise(img.size, 18).convert("RGB")
    return Image.blend(img, Image.composite(noise, img, Image.new("L", img.size, 40)), 0.25)


def font(size):
    try:
        return ImageFont.load_default(size)
    except TypeError:  # Pillow < 10.1
        return ImageFont.load_default()


def spaced(draw, xy, text, f, fill, spacing, anchor_center=True):
    widths = [draw.textlength(c, font=f) for c in text]
    total = sum(widths) + spacing * (len(text) - 1)
    x, y = xy
    if anchor_center:
        x -= total / 2
    for c, cw in zip(text, widths):
        draw.text((x, y), c, font=f, fill=fill)
        x += cw + spacing


def poster(key, title):
    w, h = 640, 960
    img = scene(key, w, h)
    # Darken the bottom for the title.
    mask = Image.linear_gradient("L").resize((w, h)).point(lambda v: max(0, v - 128) * 3 // 2)
    img = Image.composite(Image.new("RGB", (w, h)), img, mask)
    d = ImageDraw.Draw(img)
    words = title.upper().split()
    lines, line = [], ""
    for word in words:  # wrap at ~14 characters
        if line and len(line) + 1 + len(word) > 14:
            lines.append(line)
            line = word
        else:
            line = f"{line} {word}".strip()
    lines.append(line)
    size = 64 if max(len(l) for l in lines) <= 10 else 50
    f = font(size)
    y = h - 90 - len(lines) * (size + 10)
    for l in lines:
        spaced(d, (w / 2, y), l, f, (250, 246, 238), size * 0.12)
        y += size + 10
    spaced(d, (w / 2, h - 66), "A FILM BY THE DEMO COLLECTIVE", font(15), (210, 205, 196), 3)
    return img


def album(key, title, artist):
    r = rng(key)
    top, bottom, sun, far, near = (hex_rgb(c) for c in r.choice(PALETTES))
    w = 600
    s = 2
    img = gradient(w * s, w * s, top, bottom)
    d = ImageDraw.Draw(img)
    style = r.choice(["rings", "stripes", "grid", "sun"])
    W = w * s
    if style == "rings":
        cx, cy = r.uniform(0.3, 0.7) * W, r.uniform(0.3, 0.7) * W
        for i in range(14, 0, -1):
            rad = i * W / 16
            d.ellipse([cx - rad, cy - rad, cx + rad, cy + rad], fill=mix(near, sun, i % 2 * 0.7 * (1 - i / 16)))
    elif style == "stripes":
        n = r.randint(6, 12)
        for i in range(n):
            y = W * (0.25 + 0.5 * i / n)
            d.rectangle([0, y, W, y + W * 0.5 / n * 0.55], fill=mix(sun, far, i / n))
    elif style == "grid":
        n = r.randint(4, 7)
        cell = W / n
        for i in range(n):
            for j in range(n):
                if r.random() < 0.55:
                    pad = cell * 0.12
                    shape = d.ellipse if r.random() < 0.5 else d.rectangle
                    shape([i * cell + pad, j * cell + pad, (i + 1) * cell - pad, (j + 1) * cell - pad], fill=mix(sun, far, r.random()))
    else:
        rad = W * 0.28
        d.ellipse([W / 2 - rad, W * 0.42 - rad, W / 2 + rad, W * 0.42 + rad], fill=sun)
        d.polygon(waves(r, W, W, W * 0.62, W * 0.015), fill=near)
    img = img.resize((w, w), Image.LANCZOS)
    d = ImageDraw.Draw(img)
    d.text((34, w - 76), artist.upper(), font=font(18), fill=(250, 246, 238))
    d.text((34, w - 52), title, font=font(26), fill=(250, 246, 238))
    return grain(img, r)


def person(key):
    r = rng(key)
    top, bottom, sun, far, near = (hex_rgb(c) for c in r.choice(PALETTES))
    w, s = 300, 2
    W = w * s
    img = gradient(W, W, mix(bottom, sun, 0.3), mix(far, top, 0.3))
    d = ImageDraw.Draw(img)
    tone = mix(near, far, r.uniform(0.1, 0.5))
    cx = W / 2 + r.uniform(-0.04, 0.04) * W
    d.ellipse([cx - W * 0.4, W * 0.68, cx + W * 0.4, W * 1.35], fill=tone)  # shoulders
    d.rectangle([cx - W * 0.07, W * 0.5, cx + W * 0.07, W * 0.75], fill=tone)  # neck
    d.ellipse([cx - W * 0.17, W * 0.2, cx + W * 0.17, W * 0.62], fill=tone)  # head
    img = img.resize((w, w), Image.LANCZOS).filter(ImageFilter.GaussianBlur(0.6))
    return grain(img, r)


def main(out):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    items = library.ITEMS
    for path, kind in sorted(library.images().items()):
        name = path.split("/")[-1]
        target = out / name
        if target.exists():
            continue
        key = name.split("-", 1)[1].rsplit(".", 1)[0]
        if kind == "poster":
            img = poster(key, items[key]["title"])
        elif kind == "backdrop":
            img = scene(key, 1920, 1080)
        elif kind == "still":
            ep = items[key]
            img = scene(key, 712, 400, palette_key=ep["grandparentRatingKey"])
        elif kind == "album":
            a = items[key]
            img = album(key, a["title"], a["parentTitle"])
        else:
            img = person(key)
        img.save(target, quality=88)
    print(f"artwork in {out}")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "art")
