#!/usr/bin/env python3
# Procedural music composer for Standing Tank Defense.
#
# RENDER-ONLY assets: looping stems for audio.gd's two-deck vertical-mixing
# music system, plus the victory/defeat stingers (composed here, not curated
# from packs, so they share the score's key and instrument palette).
#
# Outputs (16-bit PCM WAV):
#   music/menu_theme.wav    38.4 s stereo loop  — calm menu bed (100 BPM)
#   music/match_base.wav    48.0 s stereo loop  — in-match base stem (120 BPM)
#   music/match_combat.wav  48.0 s mono loop    — combat overlay (drums + arp)
#   music/match_boss.wav    48.0 s mono loop    — boss overlay (toms + alarm)
#   sfx/victory.wav         ~3 s mono stinger
#   sfx/defeat.wav          ~3 s mono stinger
#
# The three match stems are SAMPLE-EXACTLY loop-length matched (1,058,400
# frames @ 22050 Hz) so set_music_layers() + set_intensity() stay phase-locked
# forever. Loops are seamless by construction: note tails and effect tails are
# folded circularly (events render into loop+tail buffers whose overhang wraps
# to the start; delays/reverbs process the loop twice and keep the steady-state
# second pass).
#
# Everything is seeded (SEED below) — byte-identical output on every run.
# Deps: numpy only (pip install numpy). All synthesis is band-limited additive
# (no naive-oscillator aliasing); music renders at 22050 Hz to keep the four
# checked-in WAV loops inside the repo's audio budget (pads/bass/percussion
# carry little energy above 10 kHz; swap to OGG later if sparkle is wanted).
#
# NOTE (Godot import): run `godot --headless --import` (or open the editor
# once) after regenerating so the WAVs become .sample resources. The .import
# sidecars written here pin stable uids and the forward-loop flag for music.
import hashlib
import os

import numpy as np

SR = 22050
SEED = 0x5747_0001
HERE = os.path.dirname(os.path.abspath(__file__))

A = 440.0


def hz(midi: float) -> float:
    return A * 2.0 ** ((midi - 69) / 12.0)


# --------------------------------------------------------------------------
# band-limited additive oscillators (vectorized over harmonics)
# --------------------------------------------------------------------------
def _additive(f0, n, amps_of_k, kmax_hz=9500.0, phase=0.0):
    t = np.arange(n) / SR
    kmax = max(1, int(kmax_hz / f0))
    out = np.zeros(n)
    for k in range(1, kmax + 1):
        a = amps_of_k(k)
        if a == 0.0:
            continue
        out += a * np.sin(2 * np.pi * f0 * k * t + phase * k)
    return out


def saw(f0, n, bright=9500.0, phase=0.0):
    return _additive(f0, n, lambda k: 1.0 / k, bright, phase) * (2 / np.pi)


def square(f0, n, bright=9500.0, phase=0.0):
    return _additive(f0, n, lambda k: (1.0 / k) if k % 2 else 0.0, bright, phase) * (4 / np.pi)


def triangle(f0, n, bright=6000.0, phase=0.0):
    def amp(k):
        if k % 2 == 0:
            return 0.0
        return ((-1.0) ** ((k - 1) // 2)) / (k * k)
    return _additive(f0, n, amp, bright, phase) * (8 / np.pi**2)


def sine(f0, n, phase=0.0):
    return np.sin(2 * np.pi * f0 * np.arange(n) / SR + phase)


# --------------------------------------------------------------------------
# envelopes / filters / effects (loop-safe, numpy-only)
# --------------------------------------------------------------------------
def adsr(n, a, d, s, r):
    """Attack/decay/sustain-level/release, in seconds; release eats the tail."""
    a_n, d_n, r_n = (max(1, int(x * SR)) for x in (a, d, r))
    s_n = max(0, n - a_n - d_n - r_n)
    env = np.concatenate([
        np.linspace(0.0, 1.0, a_n, endpoint=False),
        np.linspace(1.0, s, d_n, endpoint=False),
        np.full(s_n, s),
        np.linspace(s, 0.0, r_n),
    ])
    return env[:n] if len(env) >= n else np.pad(env, (0, n - len(env)))


def expdec(n, tau):
    return np.exp(-np.arange(n) / (tau * SR))


def fir_lowpass(x, cutoff_hz, taps=127):
    t = np.arange(taps) - taps // 2
    h = np.sinc(2.0 * cutoff_hz / SR * t) * np.hamming(taps)
    h /= h.sum()
    m = 1 << int(np.ceil(np.log2(len(x) + taps)))
    return np.fft.irfft(np.fft.rfft(x, m) * np.fft.rfft(h, m), m)[
        taps // 2: taps // 2 + len(x)]


def fir_highpass(x, cutoff_hz, taps=127):
    return x - fir_lowpass(x, cutoff_hz, taps)


def comb(x, delay_s, g):
    """Feedback comb, block-vectorized (stride = delay)."""
    d = max(1, int(delay_s * SR))
    y = x.copy()
    for i in range(d, len(x), d):
        j = min(i + d, len(x))
        y[i:j] += g * y[i - d: i - d + (j - i)]
    return y


def allpass(x, delay_s, g):
    d = max(1, int(delay_s * SR))
    v = comb(x, delay_s, g)          # v[n] = x[n] + g v[n-d]
    y = -g * v
    y[d:] += v[:-d]                  # y[n] = -g v[n] + v[n-d]
    return y


def reverb(x, wet=0.18):
    """Small Schroeder room. Loop-safe when used via steady_state()."""
    combs = [(0.0297, 0.65), (0.0371, 0.61), (0.0411, 0.58), (0.0437, 0.55)]
    w = np.zeros(len(x))
    for dly, g in combs:
        w += comb(x, dly, g)
    w /= len(combs)
    w = allpass(w, 0.005, 0.7)
    w = allpass(w, 0.0017, 0.7)
    return x + wet * fir_lowpass(w, 5200.0)


def pingpong(xl, xr, delay_s, fb=0.34, taps=5):
    """Feedforward ping-pong echo (loop-safe via steady_state)."""
    d = int(delay_s * SR)
    yl, yr = xl.copy(), xr.copy()
    src = 0.5 * (xl + xr)
    for k in range(1, taps + 1):
        off = k * d
        if off >= len(xl):
            break
        g = fb ** k
        if k % 2:
            yr[off:] += g * src[:-off]
        else:
            yl[off:] += g * src[:-off]
    return yl, yr


def steady_state(fx, x):
    """Run linear effect `fx` over the loop twice; keep the settled 2nd pass."""
    n = len(x)
    return fx(np.concatenate([x, x]))[n:]


def fold(buf, n):
    """Wrap a loop+tail buffer circularly onto its first n samples."""
    out = buf[:n].copy()
    tail = buf[n:]
    i = 0
    while len(tail) > 0:
        m = min(n, len(tail))
        out[:m] += tail[:m]
        tail = tail[m:]
        i += 1
    return out


# --------------------------------------------------------------------------
# instruments (each returns a mono buffer for one note)
# --------------------------------------------------------------------------
def pad_note(midi, dur, bright=2400.0, detune=0.006):
    n = int(dur * SR)
    f = hz(midi)
    out = np.zeros(n)
    for det, ph in ((1.0 - detune, 0.0), (1.0, 1.3), (1.0 + detune, 2.6)):
        out += saw(f * det, n, bright=bright, phase=ph)
    out *= adsr(n, 0.45, 0.8, 0.75, min(1.2, dur * 0.4)) / 3.0
    return out


def bass_note(midi, dur, bright=1400.0):
    n = int(dur * SR)
    f = hz(midi)
    out = saw(f, n, bright=bright) * 0.8 + sine(f * 0.5, n) * 0.55
    out *= adsr(n, 0.004, 0.10, 0.62, min(0.12, dur * 0.3))
    return out


def pluck_note(midi, dur, bright=3800.0):
    n = int(dur * SR)
    f = hz(midi)
    out = triangle(f, n, bright=bright) * 0.8 + square(f, n, bright=2000.0) * 0.25
    out *= expdec(n, 0.14) * adsr(n, 0.002, 0.05, 0.8, 0.05)
    return out


def arp_note(midi, dur, vel=1.0):
    n = int(dur * SR)
    f = hz(midi)
    out = square(f, n, bright=5200.0) * 0.7 + saw(f * 2.0, n, bright=6400.0) * 0.22
    out *= expdec(n, 0.065) * vel
    return out


def lead_note(midi, dur, vel=1.0):
    n = int(dur * SR)
    f = hz(midi)
    vib = 4.0 * np.sin(2 * np.pi * 5.2 * np.arange(n) / SR) * \
        np.linspace(0.0, 1.0, n)
    t = np.arange(n) / SR
    out = np.sin(2 * np.pi * (f + vib) * t) + 0.35 * np.sin(2 * np.pi * (f + vib) * 2 * t)
    out *= adsr(n, 0.02, 0.1, 0.8, min(0.25, dur * 0.35)) * vel
    return out


def stab_note(midi, dur, vel=1.0):
    n = int(dur * SR)
    f = hz(midi)
    out = saw(f, n, bright=3000.0) + saw(f * 1.007, n, bright=3000.0, phase=0.9)
    out *= expdec(n, 0.09) * vel * 0.5
    return out


# --- percussion -----------------------------------------------------------
def drum_kick(vel=1.0):
    n = int(0.30 * SR)
    t = np.arange(n) / SR
    f = 42.0 + 110.0 * np.exp(-t / 0.028)
    ph = 2 * np.pi * np.cumsum(f) / SR
    body = np.sin(ph) * expdec(n, 0.10)
    click = fir_highpass(_rng.standard_normal(n) * expdec(n, 0.004), 1500.0)
    return (body + 0.35 * click) * vel


def drum_snare(vel=1.0):
    n = int(0.22 * SR)
    tone = sine(196.0, n) * expdec(n, 0.035) * 0.5
    nz = _rng.standard_normal(n) * expdec(n, 0.055)
    nz = fir_highpass(nz, 900.0)
    return (tone + 0.9 * nz) * vel * 0.8


def drum_hat(vel=1.0, open_=False):
    n = int((0.16 if open_ else 0.05) * SR)
    nz = _rng.standard_normal(n) * expdec(n, 0.045 if open_ else 0.012)
    return fir_highpass(nz, 6000.0) * vel * 0.5


def drum_tom(midi, vel=1.0):
    n = int(0.28 * SR)
    t = np.arange(n) / SR
    f0 = hz(midi)
    f = f0 * (1.0 + 0.6 * np.exp(-t / 0.03))
    ph = 2 * np.pi * np.cumsum(f) / SR
    return (np.sin(ph) * expdec(n, 0.09) +
            0.2 * fir_highpass(_rng.standard_normal(n) * expdec(n, 0.01), 2000.0)) * vel


def drum_ride(vel=1.0):
    n = int(0.30 * SR)
    nz = _rng.standard_normal(n) * expdec(n, 0.12)
    return fir_highpass(nz, 8000.0) * vel * 0.28


# --------------------------------------------------------------------------
# score assembly
# --------------------------------------------------------------------------
class Stem:
    """Loop buffer with circular tail folding. Stereo = (left, right)."""

    def __init__(self, n_loop, stereo):
        self.n = n_loop
        self.stereo = stereo
        pad = int(4.0 * SR)  # generous tail room; folded circularly at the end
        self.left = np.zeros(n_loop + pad)
        self.right = np.zeros(n_loop + pad) if stereo else None

    def add(self, buf, at_s, gain=1.0, pan=0.0):
        i = int(round(at_s * SR)) % self.n
        seg = buf * gain
        end = i + len(seg)
        if end > len(self.left):
            seg = seg[: len(self.left) - i]
            end = len(self.left)
        if self.stereo:
            gl = np.sqrt(0.5 * (1.0 - pan))
            gr = np.sqrt(0.5 * (1.0 + pan))
            self.left[i:end] += seg * gl
            self.right[i:end] += seg * gr
        else:
            self.left[i:end] += seg

    def mixdown(self):
        l = fold(self.left, self.n)
        if not self.stereo:
            return l
        return l, fold(self.right, self.n)


def saturate(x, drive=2.0):
    """Peak-preserving tanh crest taming (percussive overlays would otherwise
    hit the peak ceiling ~8 dB below their loudness target)."""
    peak = np.abs(x).max()
    if peak < 1e-9:
        return x
    return np.tanh(drive * x / peak) * (peak / np.tanh(drive))


def normalize_stem(ch, target_db, ceiling_db=-1.5):
    """Normalize (mono array or channel list) by overall RMS with a peak cap."""
    chans = [ch] if isinstance(ch, np.ndarray) else list(ch)
    mono = np.mean(chans, axis=0)
    rms = np.sqrt(np.mean(mono**2) + 1e-18)
    g = 10 ** (target_db / 20.0) / rms
    peak = max(np.abs(c).max() for c in chans) * g
    ceil = 10 ** (ceiling_db / 20.0)
    if peak > ceil:
        g *= ceil / peak
    out = [c * g for c in chans]
    return out[0] if isinstance(ch, np.ndarray) else out


def write_wav(path, data, sr=SR):
    import wave
    import struct
    chans = [data] if isinstance(data, np.ndarray) else list(data)
    n = len(chans[0])
    inter = np.empty(n * len(chans))
    for i, c in enumerate(chans):
        inter[i::len(chans)] = c
    pcm = np.clip(inter * 32767.0, -32768, 32767).astype("<i2")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with wave.open(path, "w") as w:
        w.setnchannels(len(chans))
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(pcm.tobytes())
    print(f"wrote {os.path.relpath(path, HERE):24s} {n} frames "
          f"({n / sr:.3f}s, {len(chans)}ch)")


# --- harmony ---------------------------------------------------------------
# A minor. Chords as midi note lists (root voicings chosen for voice-leading).
AM = [57, 60, 64]      # A3 C4 E4
F_ = [57, 60, 65]      # A3 C4 F4  (F/A keeps the common tones)
C_ = [55, 60, 64]      # G3 C4 E4  (C/G)
G_ = [55, 59, 62]      # G3 B3 D4
CHORD_BASS = {id(AM): 33, id(F_): 29, id(C_): 36, id(G_): 31}  # A1 F1 C2 G1
PENTA = [57, 60, 62, 64, 67, 69, 72]   # A minor pentatonic pool for leads

_rng = np.random.default_rng(SEED)


# --------------------------------------------------------------------------
# menu theme — 100 BPM, 16 bars (38.4 s), calm and spacious
# --------------------------------------------------------------------------
def build_menu():
    bpm = 100.0
    beat = 60.0 / bpm
    bar = 4 * beat
    n_loop = int(round(16 * bar * SR))          # 846,720 frames
    st = Stem(n_loop, stereo=True)
    prog = [AM, AM, F_, F_, C_, C_, G_, G_] * 2

    for b, ch in enumerate(prog):
        t0 = b * bar
        # slow warm pad, alternating slight pan per chord
        pan = 0.16 if b % 2 else -0.16
        for i, m in enumerate(ch):
            st.add(pad_note(m, bar * 1.05, bright=1900.0), t0, gain=0.30, pan=pan)
        # airy octave shimmer enters in the second half
        if b >= 8:
            st.add(pad_note(ch[-1] + 12, bar * 1.02, bright=2600.0, detune=0.004),
                   t0, gain=0.10, pan=-pan)
        # sub bass: soft root, one per bar
        st.add(bass_note(CHORD_BASS[id(ch)] + 12, bar * 0.94, bright=500.0),
               t0, gain=0.30)

    # sparse pluck melody: two-bar call/response phrases on chord tones
    phrase = [(0.0, 2), (1.0, 1), (1.5, 2), (3.0, 0), (5.0, 1), (6.5, 2)]
    for cyc in range(2):
        for (off, deg) in phrase:
            b = int(off // 4) + cyc * 8
            ch = prog[b]
            note = ch[deg % len(ch)] + 12
            if cyc == 1 and off == 6.5:
                note = ch[0] + 24
            st.add(pluck_note(note, beat * 2.0), cyc * 8 * bar + off * beat,
                   gain=0.16, pan=0.25 if int(off * 2) % 2 else -0.25)

    l, r = st.mixdown()
    # gentle glue: reverb + a whisper of ping-pong echo on the whole bed
    l = steady_state(lambda x: reverb(x, wet=0.16), l)
    r = steady_state(lambda x: reverb(x, wet=0.16), r)
    dl = steady_state(lambda x: pingpong(x, x, beat * 0.75, fb=0.22, taps=3)[0], l)
    dr = steady_state(lambda x: pingpong(x, x, beat * 0.75, fb=0.22, taps=3)[1], r)
    l, r = 0.85 * l + 0.15 * dl, 0.85 * r + 0.15 * dr
    return normalize_stem([l, r], target_db=-17.0)


# --------------------------------------------------------------------------
# match stems — 120 BPM, 24 bars (48.0 s), shared grid: |Am Am F F C C G G| x3
# --------------------------------------------------------------------------
MATCH_BPM = 120.0
M_BEAT = 60.0 / MATCH_BPM
M_BAR = 4 * M_BEAT
M_BARS = 24
M_LOOP = int(round(M_BARS * M_BAR * SR))        # 1,058,400 frames
M_PROG = [AM, AM, F_, F_, C_, C_, G_, G_] * 3


def build_match_base():
    st = Stem(M_LOOP, stereo=True)
    for b, ch in enumerate(M_PROG):
        t0 = b * M_BAR
        cycle = b // 8
        # dark mid pad: half notes, low-passed further than the menu pad
        for m in ch:
            st.add(pad_note(m, M_BAR * 0.55, bright=1500.0), t0,
                   gain=0.20, pan=-0.12)
            st.add(pad_note(m, M_BAR * 0.55, bright=1500.0), t0 + M_BAR * 0.5,
                   gain=0.16, pan=0.12)
        # driving eighth-note bass with a pickup accent pattern
        root = CHORD_BASS[id(ch)] + 12
        pattern = [0, 0, 7, 0, 0, 10, 0, 12] if cycle == 2 else [0, 0, 7, 0, 0, 0, 10, 0]
        for e, iv in enumerate(pattern):
            vel = 0.9 if e in (0, 4) else 0.62
            st.add(bass_note(root + iv, M_BEAT * 0.48, bright=1600.0),
                   t0 + e * M_BEAT * 0.5, gain=0.34 * vel)
        # heartbeat pulse: soft kick on 1 and 3 keeps the base stem alive alone
        st.add(drum_kick(0.5), t0, gain=0.5)
        st.add(drum_kick(0.38), t0 + 2 * M_BEAT, gain=0.5)
        # off-beat tick (very low, stereo right)
        st.add(drum_hat(0.30), t0 + M_BEAT * 1.5, gain=0.4, pan=0.3)
        st.add(drum_hat(0.30), t0 + M_BEAT * 3.5, gain=0.4, pan=0.3)
    # quiet lead motif in the final 8-bar cycle only (keeps the loop evolving)
    motif = [(0.0, 64), (0.75, 67), (1.5, 69), (3.0, 67), (4.0, 64), (6.0, 62),
             (8.0, 60), (10.0, 57)]
    for off, m in motif:
        st.add(lead_note(m + 12, M_BEAT * 1.5, vel=0.8), (16 * 4 + off * 2) * M_BEAT,
               gain=0.10, pan=0.2)
    l, r = st.mixdown()
    l = steady_state(lambda x: reverb(x, wet=0.12), l)
    r = steady_state(lambda x: reverb(x, wet=0.12), r)
    return normalize_stem([l, r], target_db=-16.5)


def build_match_combat():
    st = Stem(M_LOOP, stereo=False)
    for b, ch in enumerate(M_PROG):
        t0 = b * M_BAR
        last_of_phrase = b % 8 == 7
        # drums: four-on-floor kick, snare on 2/4, 16th hats with accents
        for beat_i in range(4):
            st.add(drum_kick(1.0), t0 + beat_i * M_BEAT, gain=0.9)
            if beat_i % 2 == 1:
                st.add(drum_snare(1.0), t0 + beat_i * M_BEAT, gain=0.85)
        for s in range(16):
            vel = 0.85 if s % 4 == 2 else (0.45 + 0.1 * float(_rng.random()))
            st.add(drum_hat(vel, open_=(s == 14)), t0 + s * M_BEAT * 0.25, gain=0.7)
        if last_of_phrase:  # snare fill into the next phrase
            for s in range(4):
                st.add(drum_snare(0.5 + 0.14 * s), t0 + (3.0 + s * 0.25) * M_BEAT,
                       gain=0.8)
        # 16th arp on chord tones, octave-hopping, velocity-grooved
        tones = [ch[0] + 12, ch[1] + 12, ch[2] + 12, ch[1] + 24]
        for s in range(16):
            note = tones[s % 4] + (12 if (s // 4) % 2 and b % 4 >= 2 else 0)
            vel = 0.9 if s % 4 == 0 else 0.5
            st.add(arp_note(note, M_BEAT * 0.24, vel), t0 + s * M_BEAT * 0.25,
                   gain=0.16)
    m = st.mixdown()
    # arp echo glues the 16ths; mono-summed ping-pong ≈ slapback
    d = steady_state(lambda x: pingpong(x, x, M_BEAT * 0.75, fb=0.26, taps=4)[1], m)
    m = 0.88 * m + 0.12 * d
    return normalize_stem(saturate(m), target_db=-17.0)


def build_match_boss():
    st = Stem(M_LOOP, stereo=False)
    for b, ch in enumerate(M_PROG):
        t0 = b * M_BAR
        # menacing eighth-note low pulse on the chord root + tritone color
        root = CHORD_BASS[id(ch)] + 24
        for e in range(8):
            iv = 6 if (b % 4 == 3 and e >= 6) else 0        # tritone bite at turns
            st.add(stab_note(root + iv, M_BEAT * 0.5, vel=0.9 if e % 2 == 0 else 0.6),
                   t0 + e * M_BEAT * 0.5, gain=0.30)
        # war toms: syncopated low pattern
        for (off, m, v) in ((0.0, 45, 1.0), (0.75, 45, 0.6), (1.5, 41, 0.8),
                            (2.5, 45, 0.9), (3.25, 38, 0.7), (3.75, 41, 0.5)):
            st.add(drum_tom(m, v), t0 + off * M_BEAT, gain=0.8)
        # ride shimmer keeps the top alive against the combat hats
        st.add(drum_ride(0.8), t0 + 2 * M_BEAT, gain=0.5)
        # alarm motif: minor-second oscillation, sparse (phrase heads only)
        if b % 8 == 0:
            for k in range(4):
                st.add(lead_note(81 if k % 2 == 0 else 80, M_BEAT * 0.4, vel=0.75),
                       t0 + k * M_BEAT * 0.5, gain=0.10)
        # phrase-start impact
        if b % 8 == 0:
            st.add(drum_kick(1.0), t0, gain=1.1)
            st.add(drum_tom(33, 1.0), t0, gain=0.9)
    m = st.mixdown()
    m = steady_state(lambda x: reverb(x, wet=0.10), m)
    return normalize_stem(saturate(m), target_db=-16.5)


# --------------------------------------------------------------------------
# victory / defeat stingers (same palette + key as the score)
# --------------------------------------------------------------------------
def build_victory():
    beat = M_BEAT
    # sized so every tail ends inside the buffer (nothing folds — not a loop)
    st = Stem(int(3.6 * SR), stereo=False)
    # picardy lift: A minor score resolves to A MAJOR fanfare
    seq = [57, 61, 64, 69]                       # A3 C#4 E4 A4
    for i, m in enumerate(seq):
        st.add(pluck_note(m + 12, beat * 1.2), i * beat * 0.25, gain=0.30)
        st.add(lead_note(m, beat * 1.5, vel=0.7), i * beat * 0.25, gain=0.12)
    for m in (57, 61, 64, 69, 73):               # held final chord + 9th sparkle
        st.add(pad_note(m, 2.0, bright=3000.0, detune=0.004), beat, gain=0.16)
    st.add(bass_note(33, 1.8, bright=900.0), beat, gain=0.30)
    m = st.mixdown()
    m = reverb(m, wet=0.20)
    env = np.ones(len(m))
    env[-int(0.35 * SR):] = np.linspace(1.0, 0.0, int(0.35 * SR))
    return normalize_stem(m * env, target_db=-15.0)


def build_defeat():
    beat = M_BEAT
    _ = beat
    # sized so every tail ends inside the buffer (nothing folds — not a loop)
    st = Stem(int(4.2 * SR), stereo=False)
    seq = [(0.0, 57), (0.6, 55), (1.2, 52), (1.8, 45)]   # A3 G3 E3 -> A2
    for off, m in seq:
        st.add(pad_note(m, 1.4, bright=1400.0), off, gain=0.30)
        st.add(pad_note(m - 12, 1.4, bright=900.0), off, gain=0.16)
    st.add(bass_note(33, 2.2, bright=500.0), 1.8, gain=0.34)
    st.add(drum_tom(31, 0.9), 1.8, gain=0.6)             # dark final thud
    m = st.mixdown()
    m = reverb(m, wet=0.22)
    env = np.ones(len(m))
    env[-int(0.45 * SR):] = np.linspace(1.0, 0.0, int(0.45 * SR))
    return normalize_stem(m * env, target_db=-15.5)


# --------------------------------------------------------------------------
# Godot .import sidecars (uids pinned deterministically; loop flag for music)
# --------------------------------------------------------------------------
IMPORT_TEMPLATE = """[remap]

importer="wav"
type="AudioStreamWAV"
uid="uid://{uid}"
path="res://.godot/imported/{fname}-{md5}.sample"

[deps]

source_file="{res}"
dest_files=["res://.godot/imported/{fname}-{md5}.sample"]

[params]

force/8_bit=false
force/mono=false
force/max_rate=false
force/max_rate_hz=44100
edit/trim=false
edit/normalize=false
edit/loop_mode={loop_mode}
edit/loop_begin=0
edit/loop_end=-1
compress/mode=0
"""


def godot_uid(res_path: str) -> str:
    h = hashlib.sha1(res_path.encode()).hexdigest()
    alphabet = "abcdefghijklmnopqrstuvwxyz0123456789"
    v = int(h, 16)
    out = []
    for _ in range(13):
        out.append(alphabet[v % 36])
        v //= 36
    out[0] = "abcdefghijklmnopqrstuvwxyz"[v % 26]
    return "".join(out)


def write_import(rel, loop):
    path = os.path.join(HERE, rel + ".import")
    if os.path.exists(path):
        return
    res = "res://audio/" + rel.replace(os.sep, "/")
    fname = os.path.basename(rel)
    with open(path, "w") as f:
        f.write(IMPORT_TEMPLATE.format(
            uid=godot_uid(res), fname=fname, res=res,
            md5=hashlib.md5(res.encode()).hexdigest(),
            loop_mode=1 if loop else 0))
    print("wrote", rel + ".import")


def main():
    global _rng
    _rng = np.random.default_rng(SEED)
    jobs = [
        ("music/menu_theme.wav", build_menu, True),
        ("music/match_base.wav", build_match_base, True),
        ("music/match_combat.wav", build_match_combat, True),
        ("music/match_boss.wav", build_match_boss, True),
        ("sfx/victory.wav", build_victory, False),
        ("sfx/defeat.wav", build_defeat, False),
    ]
    for rel, builder, loop in jobs:
        write_wav(os.path.join(HERE, rel), builder())
        write_import(rel, loop)


if __name__ == "__main__":
    main()
