#!/usr/bin/env python3
# Procedural placeholder SFX generator for Standing Tank Defense.
#
# RENDER-ONLY assets: these WAVs are played by the Godot front-end in response to
# sim-state deltas the renderer already reads. They never touch the sim core.
#
# Uses only the Python stdlib (wave + math + struct + random). Every sound is
# synthesized, peak-normalized and soft-limited (tanh) so nothing clips, then
# written as a small 16-bit mono PCM WAV at 22050 Hz.
#
# Run:  python3 gen_sfx.py
# Output:  ./sfx/*.wav  and  ./music/ambient_bed.wav
import math
import os
import random
import struct
import wave

SR = 22050  # sample rate (Hz) — plenty for placeholder blips, keeps files tiny


# ---------------------------------------------------------------------------
# Tiny synth toolkit (all operate on plain python float lists in [-1, 1]-ish)
# ---------------------------------------------------------------------------
def _n(t):
    return int(t * SR)


def silence(dur):
    return [0.0] * _n(dur)


def add(buf, other, at=0):
    """Mix `other` into `buf` starting at sample index `at` (extends buf)."""
    end = at + len(other)
    if end > len(buf):
        buf.extend([0.0] * (end - len(buf)))
    for i, v in enumerate(other):
        buf[at + i] += v
    return buf


def sine(freq, dur, amp=1.0, phase=0.0):
    out = [0.0] * _n(dur)
    w = 2.0 * math.pi * freq / SR
    for i in range(len(out)):
        out[i] = amp * math.sin(w * i + phase)
    return out


def sweep(f0, f1, dur, amp=1.0, kind="sine"):
    """Frequency glide f0->f1 (linear) over dur seconds."""
    n = _n(dur)
    out = [0.0] * n
    ph = 0.0
    for i in range(n):
        f = f0 + (f1 - f0) * (i / max(1, n - 1))
        ph += 2.0 * math.pi * f / SR
        if kind == "square":
            out[i] = amp * (1.0 if math.sin(ph) >= 0 else -1.0)
        elif kind == "saw":
            out[i] = amp * (2.0 * ((ph / (2 * math.pi)) % 1.0) - 1.0)
        else:
            out[i] = amp * math.sin(ph)
    return out


def noise(dur, amp=1.0):
    return [amp * (random.random() * 2.0 - 1.0) for _ in range(_n(dur))]


def env_ad(buf, attack, decay, hold=0.0):
    """Apply an attack/hold/decay (exponential-ish) amplitude envelope in place."""
    n = len(buf)
    a = _n(attack)
    h = _n(hold)
    for i in range(n):
        if i < a:
            g = i / max(1, a)
        elif i < a + h:
            g = 1.0
        else:
            j = i - a - h
            d = max(1, n - a - h)
            g = max(0.0, 1.0 - j / d)
            g = g * g  # softer tail
        buf[i] *= g
    return buf


def lowpass(buf, alpha=0.25):
    """One-pole low-pass to take the edge off noise/harshness."""
    out = [0.0] * len(buf)
    prev = 0.0
    for i, v in enumerate(buf):
        prev = prev + alpha * (v - prev)
        out[i] = prev
    return out


def normalize(buf, peak=0.85):
    m = max((abs(v) for v in buf), default=0.0)
    if m < 1e-9:
        return buf
    g = peak / m
    return [v * g for v in buf]


def softclip(buf, drive=1.0):
    return [math.tanh(v * drive) for v in buf]


def write_wav(path, buf):
    buf = softclip(buf, 1.0)
    buf = normalize(buf, 0.9)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with wave.open(path, "w") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        frames = bytearray()
        for v in buf:
            s = int(max(-1.0, min(1.0, v)) * 32767)
            frames += struct.pack("<h", s)
        w.writeframes(bytes(frames))
    print("wrote", os.path.relpath(path), "(%d samples)" % len(buf))


# ---------------------------------------------------------------------------
# Individual SFX (each returns a float buffer)
# ---------------------------------------------------------------------------
def sfx_fire_a():
    # short downward zap with a noisy transient
    b = sweep(1500, 420, 0.12, 0.9, "saw")
    env_ad(b, 0.002, 0.118)
    z = lowpass(noise(0.05, 0.5), 0.5)
    env_ad(z, 0.001, 0.049)
    add(b, z, 0)
    return b


def sfx_fire_b():
    # second variant: higher, snappier, square-ish
    b = sweep(1900, 700, 0.10, 0.8, "square")
    env_ad(b, 0.001, 0.099)
    add(b, sine(2400, 0.03, 0.4), 0)
    return b


def sfx_hit():
    # tight noisy thwack with a low body
    b = lowpass(noise(0.07, 1.0), 0.45)
    env_ad(b, 0.001, 0.069)
    add(b, env_ad(sine(180, 0.07, 0.7), 0.001, 0.069))
    return b


def sfx_enemy_death():
    # downward warble + noise burst (squish)
    b = sweep(700, 120, 0.22, 0.8, "saw")
    env_ad(b, 0.004, 0.216)
    nz = lowpass(noise(0.18, 0.6), 0.35)
    env_ad(nz, 0.002, 0.178)
    add(b, nz)
    return b


def sfx_boss_spawn():
    # bigger/longer: rising rumble + ominous detuned tones + impact
    dur = 1.1
    b = silence(dur)
    add(b, env_ad(sweep(50, 130, dur, 0.9), 0.25, 0.85), 0)
    add(b, env_ad(sine(82, dur, 0.5), 0.3, 0.8), 0)
    add(b, env_ad(sine(123, dur, 0.4), 0.3, 0.8), 0)  # ~fifth, detuned feel
    # low-end impact near the end
    imp = lowpass(noise(0.25, 1.0), 0.3)
    env_ad(imp, 0.002, 0.248)
    add(b, imp, _n(0.78))
    add(b, env_ad(sine(60, 0.3, 0.9), 0.005, 0.295), _n(0.78))
    return b


def sfx_tank_hit():
    # metallic clank: short metallic ring + noise
    b = silence(0.16)
    for f, a in [(330, 0.6), (495, 0.4), (740, 0.3)]:
        add(b, env_ad(sine(f, 0.16, a), 0.001, 0.159))
    nz = lowpass(noise(0.05, 0.8), 0.5)
    env_ad(nz, 0.001, 0.049)
    add(b, nz)
    return b


def sfx_tank_destroyed():
    # big descending explosion: low boom + noisy debris fall
    dur = 0.9
    b = silence(dur)
    add(b, env_ad(sweep(160, 40, dur, 1.0, "saw"), 0.003, 0.9))
    boom = lowpass(noise(dur, 1.0), 0.18)
    env_ad(boom, 0.003, dur)
    add(b, boom)
    add(b, env_ad(sine(55, 0.5, 0.9), 0.002, 0.498))
    return b


def sfx_buy():
    # pleasant two-note coin-ish ding (up)
    b = silence(0.22)
    add(b, env_ad(sine(880, 0.10, 0.6), 0.002, 0.098), 0)
    add(b, env_ad(sine(1318, 0.14, 0.6), 0.002, 0.138), _n(0.07))
    return b


def sfx_reroll():
    # quick shuffle/flutter sweep up
    b = sweep(500, 1200, 0.16, 0.6, "square")
    env_ad(b, 0.004, 0.156)
    # add a couple of ticks
    add(b, env_ad(sine(1500, 0.02, 0.4), 0.001, 0.019), _n(0.04))
    add(b, env_ad(sine(1700, 0.02, 0.4), 0.001, 0.019), _n(0.1))
    return b


def sfx_clear():
    # big sweeping shockwave whoosh (down) with bright onset
    dur = 0.5
    b = sweep(1400, 180, dur, 0.8, "saw")
    env_ad(b, 0.005, dur - 0.005)
    nz = lowpass(noise(dur, 0.7), 0.25)
    env_ad(nz, 0.005, dur - 0.005)
    add(b, nz)
    add(b, env_ad(sine(90, 0.3, 0.8), 0.002, 0.298))
    return b


def sfx_round_start():
    # rising 3-note fanfare, bright
    b = silence(0.5)
    for i, f in enumerate([523, 659, 784]):  # C E G
        add(b, env_ad(sine(f, 0.18, 0.5), 0.003, 0.177), _n(0.10 * i))
        add(b, env_ad(sine(f * 2, 0.12, 0.2), 0.003, 0.117), _n(0.10 * i))
    return b


def sfx_victory():
    # triumphant ascending arpeggio + final chord
    b = silence(1.2)
    seq = [(523, 0.0), (659, 0.12), (784, 0.24), (1046, 0.36)]  # C E G C
    for f, t in seq:
        add(b, env_ad(sine(f, 0.28, 0.45), 0.003, 0.277), _n(t))
        add(b, env_ad(sine(f * 2, 0.2, 0.18), 0.003, 0.197), _n(t))
    # held final chord
    for f in (523, 659, 784, 1046):
        add(b, env_ad(sine(f, 0.5, 0.3), 0.01, 0.49), _n(0.55))
    return b


def sfx_defeat():
    # somber descending tones
    b = silence(1.0)
    seq = [(440, 0.0), (392, 0.18), (330, 0.36), (262, 0.54)]  # A G E C
    for f, t in seq:
        add(b, env_ad(sine(f, 0.4, 0.45), 0.005, 0.395), _n(t))
        add(b, env_ad(sine(f * 0.5, 0.4, 0.2), 0.005, 0.395), _n(t))
    return b


def sfx_ui_move():
    # very short soft blip for menu navigation
    b = sine(1200, 0.04, 0.5)
    env_ad(b, 0.001, 0.039)
    return b


def music_ambient_bed():
    # ~8s seamless low drone pad (loops): a slow detuned chord + airy noise wash.
    # Frequencies are snapped so an integer number of cycles fits in `dur`, so the
    # buffer starts and ends in phase and can loop with no audible seam.
    dur = 8.0
    n = _n(dur)
    out = [0.0] * n
    base = 110.0  # A2
    partials = [
        (base, 0.30),
        (base * 1.5, 0.16),    # fifth
        (base * 2.0, 0.12),    # octave
        (base * 2.51, 0.06),   # slightly detuned for movement
    ]
    for f, a in partials:
        cycles = max(1, round(f * dur))
        ff = cycles / dur
        for i in range(n):
            # slow tremolo for life (one full cycle over the loop -> seamless)
            trem = 0.85 + 0.15 * math.sin(2 * math.pi * (1.0 / dur) * i)
            out[i] += a * trem * math.sin(2 * math.pi * ff * i / SR)
    # gentle airy wash (looped noise low-passed) at low level
    wash = lowpass(noise(dur, 0.18), 0.02)
    for i in range(n):
        out[i] += wash[i] * 0.4
    return out


SFX = {
    "fire": sfx_fire_a,
    "fire_b": sfx_fire_b,
    "hit": sfx_hit,
    "enemy_death": sfx_enemy_death,
    "boss_spawn": sfx_boss_spawn,
    "tank_hit": sfx_tank_hit,
    "tank_destroyed": sfx_tank_destroyed,
    "buy": sfx_buy,
    "reroll": sfx_reroll,
    "clear": sfx_clear,
    "round_start": sfx_round_start,
    "victory": sfx_victory,
    "defeat": sfx_defeat,
    "ui_move": sfx_ui_move,
}


def main():
    random.seed(1234)  # stable output for reproducible assets
    here = os.path.dirname(os.path.abspath(__file__))
    for name, fn in SFX.items():
        write_wav(os.path.join(here, "sfx", name + ".wav"), fn())
    write_wav(os.path.join(here, "music", "ambient_bed.wav"), music_ambient_bed())


if __name__ == "__main__":
    main()
