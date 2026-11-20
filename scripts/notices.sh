#!/bin/sh
# Collect the third-party notices into apps/encheo/notices.txt: THIRD-PARTY-NOTICES.md as plain
# text, the licence texts in licenses/, then every Rust crate's licence (cargo-about). The app
# shows that file in About, and the Windows and macOS downloads include it. Run it after changing
# dependencies.  Needs: cargo install cargo-about --features cli
set -e
cd "$(dirname "$0")/.."
command -v cargo-about >/dev/null || { echo "needs cargo-about: cargo install cargo-about --features cli" >&2; exit 1; }
crates=$(mktemp)
cargo about generate -m apps/encheo/Cargo.toml packaging/notices/crates.hbs -o "$crates" 2>/dev/null
python3 - "$crates" > apps/encheo/notices.txt <<'PY'
import re, sys, pathlib

def plain(md):
    out = []
    for line in md.splitlines():
        if re.match(r"^\|[-| ]+\|$", line):
            out.pop()  # the table's header row, then its rule
            continue
        if line.startswith("|"):
            cells = [c.strip() for c in line.strip("|").split("|")]
            line = "- " + " · ".join(c for c in cells if c)
        line = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", lambda m: m.group(1) if m.group(2).startswith("licenses/") or m.group(2) == "LICENSE" else f"{m.group(1)} ({m.group(2)})", line)
        line = re.sub(r"<(https?://[^>]+)>", r"\1", line)
        line = line.replace("`", "")
        m = re.match(r"^(#+) (.*)", line)
        if m:
            title = m.group(2).upper() if len(m.group(1)) == 1 else m.group(2)
            line = title + "\n" + ("=" if len(m.group(1)) == 1 else "-") * len(title)
        out.append(line)
    return "\n".join(out)

print(plain(pathlib.Path("THIRD-PARTY-NOTICES.md").read_text()))
print("\n\nLICENCE TEXTS\n=============")
for f in sorted(pathlib.Path("licenses").iterdir()):
    print("\n" + "-" * 78 + "\n" + f.stem + "\n" + "-" * 78 + "\n")
    print(f.read_text().strip())
print("\n\n" + pathlib.Path(sys.argv[1]).read_text().strip())
PY
rm -f "$crates"
echo "apps/encheo/notices.txt: $(wc -c < apps/encheo/notices.txt) bytes"
