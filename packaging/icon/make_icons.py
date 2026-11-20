#!/usr/bin/env python3
"""Draw Encheo's icon ("Shelves": three library rows that read as an E, the top one playing) and
write it out for every platform. Run from anywhere: python3 packaging/icon/make_icons.py

Needs Pillow. macOS's iconutil makes the .icns (skipped elsewhere).

The design lives on a 100 x 100 grid (see encheo.svg): a dark rounded tile with three orange bars.
"""
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parents[2]
BG = (28, 28, 31)
ORANGE = (245, 163, 59)


def mix(a, b, t):
    return tuple(round(x * t + y * (1 - t)) for x, y in zip(a, b))


# The bars on the 100 grid: (x, y, width, height, colour). The lower two are the orange faded
# into the tile (75 % and 50 %), as opaque colours.
DX, DY = -2.75, 1.75  # centres the glyph on the tile
BARS = [
    (24, 24, 38, 12, ORANGE),
    (24, 44, 40, 12, mix(ORANGE, BG, 0.75)),
    (24, 64, 52, 12, mix(ORANGE, BG, 0.5)),
]
# The play arrow at the end of the top bar, its corners rounded by a 3-unit stroke.
ARROW = [(66, 22), (80, 30), (66, 38)]
ARROW_STROKE = 3


def draw_glyph(d, scale, ox, oy):
    """The bars and the arrow, 100-grid units scaled by `scale`, the grid's origin at (ox, oy)."""
    p = lambda x, y: (ox + (x + DX) * scale, oy + (y + DY) * scale)
    for x, y, w, h, c in BARS:
        d.rounded_rectangle([p(x, y), p(x + w, y + h)], radius=h / 2 * scale, fill=c)
    pts = [p(x, y) for x, y in ARROW]
    d.polygon(pts, fill=ORANGE)
    r = ARROW_STROKE / 2 * scale
    d.line(pts + [pts[0]], fill=ORANGE, width=round(ARROW_STROKE * scale), joint="curve")
    for x, y in pts:
        d.ellipse([x - r, y - r, x + r, y + r], fill=ORANGE)


def render(size, tile=1.0, corner=0.22, shadow=False, square=False):
    """The icon at `size` px. The tile fills `tile` of the canvas (centred) with corner radius
    `corner` of its side; `square` makes an opaque, uncornered tile (iOS masks it itself)."""
    ss = 4 if size >= 64 else 8  # supersampling
    big = size * ss
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    side = big * tile
    o = (big - side) / 2
    if shadow:
        sh = Image.new("RGBA", (big, big), (0, 0, 0, 0))
        ImageDraw.Draw(sh).rounded_rectangle(
            [o, o + side * 0.02, o + side, o + side * 1.02], radius=side * corner, fill=(0, 0, 0, 110)
        )
        img = Image.alpha_composite(img, sh.filter(ImageFilter.GaussianBlur(side * 0.02)))
    d = ImageDraw.Draw(img)
    if square:
        d.rectangle([0, 0, big, big], fill=BG)
    else:
        d.rounded_rectangle([o, o, o + side, o + side], radius=side * corner, fill=BG)
    draw_glyph(d, side / 100, o, o)
    return img.resize((size, size), Image.LANCZOS)


def save(img, path):
    path = ROOT / path
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path)
    print(path.relative_to(ROOT))


def macos():
    """Encheo.icns. Apple's grid: an 824 px tile on a 1024 px canvas, with a soft shadow."""
    out = ROOT / "packaging/macos/Encheo.icns"
    if not shutil.which("iconutil"):
        print("skipped", out.relative_to(ROOT), "(needs macOS)")
        return
    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "Encheo.iconset"
        iconset.mkdir()
        for pt in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                name = f"icon_{pt}x{pt}{'@2x' if scale == 2 else ''}.png"
                render(pt * scale, tile=824 / 1024, corner=0.2237, shadow=pt * scale >= 64).save(iconset / name)
        out.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["iconutil", "-c", "icns", iconset, "-o", out], check=True)
    print(out.relative_to(ROOT))


def windows():
    """encheo.ico: each size drawn on its own so the small ones stay crisp."""
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    imgs = [render(s, tile=0.94 if s >= 32 else 1.0) for s in sizes]
    path = ROOT / "packaging/windows/encheo.ico"
    imgs[-1].save(path, sizes=[(s, s) for s in sizes], append_images=imgs[:-1])
    print(path.relative_to(ROOT))


def window_icon():
    """The window's own icon (Linux and Windows title bars and taskbars), built into the app."""
    save(render(256, tile=0.94), "apps/encheo/ui/icon.png")


def ios():
    """Info.plist lists these; iOS rounds the corners itself, so they're square and opaque."""
    for name, size in [
        ("AppIcon60x60@2x.png", 120),
        ("AppIcon60x60@3x.png", 180),
        ("AppIcon76x76@2x~ipad.png", 152),
        ("AppIcon83.5x83.5@2x~ipad.png", 167),
    ]:
        save(render(size, square=True).convert("RGB"), f"apps/encheo/ios/icons/{name}")


def hexc(c):
    return "#%02X%02X%02X" % c


def android():
    """An adaptive launcher icon as vector drawables (Android 8+, the app's minimum): the dark
    background and the glyph, which the launcher masks to its own shape. The 100 grid maps onto
    the 72 dp visible area of the 108 dp canvas."""
    res = ROOT / "apps/encheo/android/app/src/main/res"
    s, o = 0.72, 18
    f = lambda v: f"{v:.2f}".rstrip("0").rstrip(".")
    paths = []
    for x, y, w, h, c in BARS:
        x0, y0 = o + (x + DX) * s, o + (y + DY) * s
        x1, y1, r = x0 + w * s, y0 + h * s, h / 2 * s
        d = (f"M{f(x0 + r)},{f(y0)} H{f(x1 - r)} A{f(r)},{f(r)} 0 0 1 {f(x1 - r)},{f(y1)} "
             f"H{f(x0 + r)} A{f(r)},{f(r)} 0 0 1 {f(x0 + r)},{f(y0)} Z")
        paths.append(f'    <path android:fillColor="{hexc(c)}" android:pathData="{d}" />')
    pts = [(o + (x + DX) * s, o + (y + DY) * s) for x, y in ARROW]
    d = "M" + " L".join(f"{f(x)},{f(y)}" for x, y in pts) + " Z"
    paths.append(
        f'    <path android:fillColor="{hexc(ORANGE)}" android:strokeColor="{hexc(ORANGE)}" '
        f'android:strokeWidth="{f(ARROW_STROKE * s)}" android:strokeLineJoin="round" android:pathData="{d}" />'
    )
    vector = ('<?xml version="1.0" encoding="utf-8"?>\n'
              '<!-- Generated by packaging/icon/make_icons.py -->\n'
              '<vector xmlns:android="http://schemas.android.com/apk/res/android"\n'
              '    android:width="108dp" android:height="108dp"\n'
              '    android:viewportWidth="108" android:viewportHeight="108">\n{}\n</vector>\n')
    # The notification's small icon: the glyph alone in white (Android tints it), cropped to it.
    white = [re.sub(r'(fill|stroke)Color="#[0-9A-F]{6}"', r'\1Color="#FFFFFF"', p) for p in paths]
    status = vector.format("\n".join(
        ['    <group android:translateX="-31" android:translateY="-31">'] + ["    " + p for p in white] + ["    </group>"]
    )).replace(
        'android:width="108dp" android:height="108dp"', 'android:width="24dp" android:height="24dp"').replace(
        'android:viewportWidth="108" android:viewportHeight="108"', 'android:viewportWidth="48" android:viewportHeight="48"')
    files = {
        "drawable/ic_stat_encheo.xml": status,
        "drawable/ic_launcher_foreground.xml": vector.format("\n".join(paths)),
        "drawable/ic_launcher_background.xml": vector.format(
            f'    <path android:fillColor="{hexc(BG)}" android:pathData="M0,0 H108 V108 H0 Z" />'),
        "mipmap-anydpi-v26/ic_launcher.xml": (
            '<?xml version="1.0" encoding="utf-8"?>\n'
            '<!-- Generated by packaging/icon/make_icons.py -->\n'
            '<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n'
            '    <background android:drawable="@drawable/ic_launcher_background" />\n'
            '    <foreground android:drawable="@drawable/ic_launcher_foreground" />\n'
            '</adaptive-icon>\n'),
    }
    for name, text in files.items():
        path = res / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        print(path.relative_to(ROOT))


def tv_banner():
    """The Android TV home screen's 320 x 180 banner: the icon and the name."""
    w, h, ss = 320, 180, 4
    img = Image.new("RGBA", (w * ss, h * ss), BG + (255,))
    d = ImageDraw.Draw(img)
    tile = 96 * ss
    x0, y0 = 34 * ss, (h * ss - tile) // 2
    draw_glyph(d, tile / 100, x0, y0)
    font = None
    for path, index in [("/System/Library/Fonts/HelveticaNeue.ttc", 1), ("/System/Library/Fonts/Helvetica.ttc", 1),
                        ("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 0)]:
        try:
            font = ImageFont.truetype(path, 46 * ss, index=index)
            break
        except OSError:
            continue
    font = font or ImageFont.load_default(46 * ss)
    text = "Encheo"
    box = d.textbbox((0, 0), text, font=font)
    tx = x0 + tile + 4 * ss
    ty = (h * ss - (box[3] - box[1])) // 2 - box[1]
    d.text((tx, ty), text, font=font, fill=(242, 242, 242))
    save(img.resize((w, h), Image.LANCZOS).convert("RGB"), "apps/encheo/android/app/src/main/res/drawable-xhdpi/banner.png")


def preview():
    """A 512 px PNG for the README and for looking at."""
    save(render(512, tile=0.94), "packaging/icon/encheo-512.png")


if __name__ == "__main__":
    macos()
    windows()
    window_icon()
    ios()
    android()
    tv_banner()
    preview()
