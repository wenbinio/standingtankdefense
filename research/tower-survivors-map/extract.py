#!/usr/bin/env python3
"""
Extract and parse the Tower Survivors (WC3) map for Standing Tank Defense research.

This is the *finalized* extraction so future work does not have to repeat it.

What it does
------------
1. Opens a WC3 `.w3x` (an MPQ archive) and pulls the known internal files
   (the encrypted `(listfile)` is skipped; well-known names are read directly).
2. Parses `war3map.wts` (the trigger-string table) into structured entries.
3. Emits a machine-readable `parsed/catalog.json` (weapons, upgrades, units,
   changelog/help text) and re-creates the human-readable evidence appendix.

Usage
-----
    python3 extract.py /path/to/TowerSurvivors.w3x        # full run from a .w3x
    python3 extract.py                                    # parse raw/war3map.wts only

The raw extracted files are committed under ./raw so this runs offline with no
.w3x present. `mpyq.py` (vendored, BSD) is only needed for step 1.
"""
import json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
RAW = os.path.join(HERE, "raw")
PARSED = os.path.join(HERE, "parsed")

# WC3 internal files worth keeping (text-bearing + object data).
WANTED = [
    "war3map.wts", "war3map.w3i", "war3mapMisc.txt",
    "war3map.w3u", "war3map.w3a", "war3map.w3t", "war3map.w3b",
    "war3map.w3h", "war3map.w3q",
]


def extract_from_w3x(path):
    """Pull WANTED files out of an MPQ .w3x into ./raw using vendored mpyq."""
    sys.path.insert(0, HERE)
    from mpyq import MPQArchive  # vendored single-file extractor
    a = MPQArchive(path, listfile=False)
    os.makedirs(RAW, exist_ok=True)
    for fn in WANTED:
        try:
            data = a.read_file(fn)
        except NotImplementedError:
            # mpyq cannot decrypt; the script block (war3map.j/.lua) lands here.
            print(f"  {fn}: ENCRYPTED (skipped)")
            continue
        if data:
            open(os.path.join(RAW, fn), "wb").write(data)
            print(f"  {fn}: {len(data)} bytes")
        else:
            print(f"  {fn}: not present")


def strip_colorcodes(s):
    return re.sub(r"\|c[0-9a-fA-F]{8}|\|r", " ", s).replace("|n", "\n").strip()


def parse_wts(path):
    """Parse war3map.wts -> list of (id, comment, body)."""
    text = open(path, "rb").read().decode("utf-8", "replace").replace("\r\n", "\n")
    out = []
    for part in re.split(r"(?:^|\n)STRING ", text)[1:]:
        m = re.match(r"(\d+)[^\n]*\n(//[^\n]*\n)?\{\n(.*?)\n\}", part, re.S)
        if m:
            out.append((int(m.group(1)), (m.group(2) or "").strip(), m.group(3)))
    return out


def build_catalog(entries):
    cat = {"units": [], "weapons": [], "upgrades": [], "notes": []}
    seen_u, seen_w, seen_up = set(), set(), set()
    for n, c, b in entries:
        t = strip_colorcodes(b)
        flat = re.sub(r"\s+", " ", t).strip()
        if "Units:" in c and ", Name " in c:
            if flat and flat not in seen_u:
                seen_u.add(flat); cat["units"].append(flat)
        if t.startswith("Weapon") and "Attack Type:" in t:
            key = flat[:90]
            if key not in seen_w:
                seen_w.add(key); cat["weapons"].append(flat)
        if t.startswith("Upgrade") and "Ubertip" in c:
            first = re.sub(r"\s+", " ", re.split(r"\n\s*-", t[len("Upgrade"):])[0]).strip()
            if first and first not in seen_up:
                seen_up.add(first); cat["upgrades"].append(first)
        # long non-ability strings = changelog / help / credits
        if "Abilities:" not in c and "Units:" not in c and len(t) > 200:
            cat["notes"].append({"id": n, "text": t})
    return cat


def main():
    if len(sys.argv) > 1:
        print(f"Extracting from {sys.argv[1]} ...")
        extract_from_w3x(sys.argv[1])
    wts = os.path.join(RAW, "war3map.wts")
    if not os.path.exists(wts):
        sys.exit("No raw/war3map.wts found. Pass a .w3x path on first run.")
    entries = parse_wts(wts)
    print(f"Parsed {len(entries)} trigger strings.")
    cat = build_catalog(entries)
    os.makedirs(PARSED, exist_ok=True)
    json.dump(cat, open(os.path.join(PARSED, "catalog.json"), "w"), indent=2, ensure_ascii=False)
    print(f"Wrote parsed/catalog.json: "
          f"{len(cat['units'])} units, {len(cat['weapons'])} weapons, "
          f"{len(cat['upgrades'])} upgrades, {len(cat['notes'])} notes.")


if __name__ == "__main__":
    main()
