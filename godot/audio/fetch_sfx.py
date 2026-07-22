#!/usr/bin/env python3
# CC0 SFX curation pipeline for Standing Tank Defense.
#
# RENDER-ONLY assets: these WAVs are played by the Godot front-end in response
# to sim-state deltas the renderer already reads. They never touch the sim core.
#
# Sources: Kenney's CC0 audio packs (https://kenney.nl/assets — CC0 1.0, no
# attribution required; credited anyway in /CREDITS.md). This script:
#   1. downloads the pinned pack zips into ./_kenney_cache/ (skipped if present),
#   2. decodes the selected OGGs straight out of the zips,
#   3. processes each one consistently: mono downmix, silence trim, short
#      anti-click fades, optional 2:1 downsample for long stingers, and
#      active-RMS loudness normalization to a per-category target with a
#      -1 dBFS peak ceiling,
#   4. writes 16-bit PCM WAVs into ./sfx/ and a matching Godot .import file
#      for any output that doesn't already have one.
#
# Deps beyond the stdlib: numpy + soundfile (pip install numpy soundfile).
# The OUTPUT WAVs are checked in; running this script is only needed to
# regenerate them. Victory/defeat stingers + all music are NOT made here —
# they are composed by gen_music.py so they share the score's key and palette.
#
# NOTE (Godot import): WAVs still need Godot's import step to become .sample
# resources — run `godot --headless --import` (or open the editor once) after
# regenerating. The .import sidecars written here pin stable uids/params.
import hashlib
import io
import os
import sys
import urllib.request
import zipfile

import numpy as np

try:
    import soundfile as sf
except ImportError:  # pragma: no cover
    sys.exit("fetch_sfx.py needs the `soundfile` package (pip install soundfile)")

HERE = os.path.dirname(os.path.abspath(__file__))
CACHE = os.path.join(HERE, "_kenney_cache")
OUT = os.path.join(HERE, "sfx")

# Pinned pack URLs (kenney.nl media URLs carry a content hash — stable).
PACKS = {
    "scifi": "https://kenney.nl/media/pages/assets/sci-fi-sounds/6b296f9ecf-1677589334/kenney_sci-fi-sounds.zip",
    "impact": "https://kenney.nl/media/pages/assets/impact-sounds/87b4ddecda-1677589768/kenney_impact-sounds.zip",
    "interface": "https://kenney.nl/media/pages/assets/interface-sounds/fa43c1dd4d-1677589452/kenney_interface-sounds.zip",
    "ui": "https://kenney.nl/media/pages/assets/ui-audio/490d233f68-1677590494/kenney_ui-audio.zip",
    "digital": "https://kenney.nl/media/pages/assets/digital-audio/216eac4753-1677590265/kenney_digital-audio.zip",
}

# Loudness categories: (active-RMS target dBFS, output sample rate).
# UI sits lowest, frequent combat a bit louder, one-shot stingers loudest.
# Long stingers drop to 22050 Hz (they carry little content above 10 kHz).
CAT = {
    "ui":      (-20.0, 44100),
    "economy": (-19.0, 44100),
    "combat":  (-17.5, 44100),
    "accent":  (-16.0, 44100),
    "stinger": (-14.5, 22050),
}

# out_name -> (pack, member suffix inside the zip, category)
MAPPING = {
    # weapons / combat (frequent; short + light)
    "fire":           ("scifi", "laserSmall_000.ogg", "combat"),
    "fire_b":         ("scifi", "laserSmall_002.ogg", "combat"),
    "fire_c":         ("scifi", "laserSmall_004.ogg", "combat"),
    "hit":            ("impact", "impactGeneric_light_001.ogg", "combat"),
    "hit_b":          ("impact", "impactGeneric_light_003.ogg", "combat"),
    "enemy_death":    ("scifi", "slime_000.ogg", "combat"),
    "enemy_death_b":  ("impact", "impactSoft_heavy_004.ogg", "combat"),
    "tank_hit":       ("impact", "impactMetal_medium_000.ogg", "accent"),
    "tank_hit_b":     ("impact", "impactMetal_medium_002.ogg", "accent"),
    # big one-shots
    "boss_spawn":     ("scifi", "forceField_000.ogg", "stinger"),
    "boss_death":     ("scifi", "explosionCrunch_004.ogg", "stinger"),
    "tank_destroyed": ("scifi", "explosionCrunch_000.ogg", "stinger"),
    "elimination":    ("scifi", "lowFrequency_explosion_000.ogg", "stinger"),
    "clear":          ("digital", "powerUp8.ogg", "accent"),
    "round_start":    ("digital", "threeTone2.ogg", "accent"),
    "achievement":    ("digital", "powerUp3.ogg", "accent"),
    # economy
    "buy":            ("interface", "confirmation_001.ogg", "economy"),
    "sell":           ("interface", "drop_003.ogg", "economy"),
    "coin":           ("interface", "glass_002.ogg", "economy"),
    "reroll":         ("interface", "scroll_001.ogg", "economy"),
    # UI
    "ui_move":        ("interface", "select_001.ogg", "ui"),
    "ui_click":       ("interface", "click_001.ogg", "ui"),
    "ui_back":        ("interface", "back_003.ogg", "ui"),
    "ui_hover":      ("ui", "rollover2.ogg", "ui"),
    "ui_deny":        ("interface", "error_004.ogg", "ui"),
}

IMPORT_TEMPLATE = """[remap]

importer="wav"
type="AudioStreamWAV"
uid="uid://{uid}"
path="res://.godot/imported/{name}.wav-{md5}.sample"

[deps]

source_file="res://audio/sfx/{name}.wav"
dest_files=["res://.godot/imported/{name}.wav-{md5}.sample"]

[params]

force/8_bit=false
force/mono=false
force/max_rate=false
force/max_rate_hz=44100
edit/trim=false
edit/normalize=false
edit/loop_mode=0
edit/loop_begin=0
edit/loop_end=-1
compress/mode=0
"""


def fetch(pack: str) -> str:
    os.makedirs(CACHE, exist_ok=True)
    path = os.path.join(CACHE, pack + ".zip")
    if not os.path.exists(path):
        url = PACKS[pack]
        print("downloading", url)
        with urllib.request.urlopen(url, timeout=120) as r, open(path, "wb") as f:
            f.write(r.read())
    return path


def load_member(zpath: str, suffix: str):
    with zipfile.ZipFile(zpath) as z:
        for nm in z.namelist():
            if nm.endswith(suffix):
                data, sr = sf.read(io.BytesIO(z.read(nm)))
                return np.atleast_2d(data.T).mean(axis=0), sr  # mono downmix
    raise KeyError(f"{suffix} not found in {zpath}")


def trim_silence(x: np.ndarray, sr: int, thresh_db=-55.0, pre_ms=4.0, post_ms=30.0):
    a = np.abs(x)
    thresh = 10 ** (thresh_db / 20.0)
    idx = np.nonzero(a > thresh)[0]
    if idx.size == 0:
        return x
    lo = max(0, idx[0] - int(sr * pre_ms / 1000.0))
    hi = min(len(x), idx[-1] + int(sr * post_ms / 1000.0))
    return x[lo:hi]


def fade(x: np.ndarray, sr: int, in_ms=2.0, out_ms=10.0):
    n_in = min(len(x), int(sr * in_ms / 1000.0))
    n_out = min(len(x), int(sr * out_ms / 1000.0))
    if n_in > 0:
        x[:n_in] *= np.linspace(0.0, 1.0, n_in)
    if n_out > 0:
        x[-n_out:] *= np.linspace(1.0, 0.0, n_out)
    return x


def downsample2(x: np.ndarray) -> np.ndarray:
    """2:1 decimation behind a windowed-sinc lowpass (anti-aliasing)."""
    taps = 129
    t = np.arange(taps) - taps // 2
    h = np.sinc(t * 0.45) * np.hamming(taps)
    h /= h.sum()
    y = np.convolve(x, h, mode="same")
    return y[::2].copy()


def active_rms_db(x: np.ndarray) -> float:
    """RMS over 23 ms frames within 30 dB of the loudest frame (ignores tails)."""
    n = 1024
    m = len(x) // n
    if m == 0:
        return 20.0 * np.log10(np.sqrt(np.mean(x**2)) + 1e-12)
    fr = np.sqrt(np.mean(x[: m * n].reshape(m, n) ** 2, axis=1) + 1e-18)
    keep = fr > fr.max() * 10 ** (-30 / 20)
    return 20.0 * np.log10(np.sqrt(np.mean(fr[keep] ** 2)) + 1e-12)


def saturate(x: np.ndarray, drive=1.8) -> np.ndarray:
    """Peak-preserving tanh crest taming: spiky sources (explosions, slime)
    otherwise hit the -1 dBFS ceiling far below their loudness target."""
    peak = np.abs(x).max()
    if peak < 1e-9:
        return x
    return np.tanh(drive * x / peak) * (peak / np.tanh(drive))


def normalize(x: np.ndarray, target_db: float) -> np.ndarray:
    gain = 10 ** ((target_db - active_rms_db(x)) / 20.0)
    x = x * gain
    peak = np.abs(x).max()
    ceil = 10 ** (-1.0 / 20.0)  # -1 dBFS
    if peak > ceil:
        x *= ceil / peak
    return x


def godot_uid(res_path: str) -> str:
    """Deterministic 13-char [a-z0-9] uid derived from the res path."""
    h = hashlib.sha1(res_path.encode()).hexdigest()
    alphabet = "abcdefghijklmnopqrstuvwxyz0123456789"
    v = int(h, 16)
    out = []
    for _ in range(13):
        out.append(alphabet[v % 36])
        v //= 36
    out[0] = "abcdefghijklmnopqrstuvwxyz"[v % 26]  # first char: letter
    return "".join(out)


def write_import(name: str) -> None:
    path = os.path.join(OUT, name + ".wav.import")
    if os.path.exists(path):
        return  # keep the existing uid/params (already imported once)
    res = f"res://audio/sfx/{name}.wav"
    with open(path, "w") as f:
        f.write(IMPORT_TEMPLATE.format(
            uid=godot_uid(res), name=name,
            md5=hashlib.md5(res.encode()).hexdigest()))
    print("wrote", os.path.relpath(path, HERE))


def main() -> None:
    os.makedirs(OUT, exist_ok=True)
    zips = {p: fetch(p) for p in sorted({m[0] for m in MAPPING.values()})}
    for name in sorted(MAPPING):
        pack, member, cat = MAPPING[name]
        target_db, out_sr = CAT[cat]
        x, sr = load_member(zips[pack], member)
        assert sr == 44100, f"{member}: unexpected rate {sr}"
        x = trim_silence(x, sr)
        if out_sr == 22050:
            x = downsample2(x)
            sr = 22050
        x = fade(x.astype(np.float64), sr)
        if cat in ("combat", "stinger"):
            x = saturate(x)
        x = normalize(x, target_db)
        out_path = os.path.join(OUT, name + ".wav")
        sf.write(out_path, x, sr, subtype="PCM_16")
        write_import(name)
        print(f"wrote sfx/{name}.wav  {len(x)/sr:5.2f}s mono {sr} Hz "
              f"({pack}/{member}, {cat} @ {target_db} dB)")


if __name__ == "__main__":
    main()
