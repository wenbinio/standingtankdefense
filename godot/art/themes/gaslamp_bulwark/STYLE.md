# STYLE.md — Standing Tank Defense Art Bible (LOCKED)

This is the single source of truth for the game's visual language. Every sprite,
icon, and UI element copies the hexes and obeys the rules below **verbatim**.

## Art direction — "Gaslamp Bulwark"

A dark-fantasy battlefield lit by gaslamp ember and arcane frost: heroic
**brass-and-steel steampunk war-machines** holding the line against a tide of
**rot-green, bone-and-sinew horror** — rendered as adorable, big-headed **chibi**
characters. The look is **clean, bold, arcade-readable vector** — thick dark
outlines, flat two-tone fills with a single rim light and one inner shadow, juicy
saturated accents. Think modern flat-but-juicy mobile strategy art with a cute
**chibi** cast, never gritty photorealism and never thin corporate flat-icons.

## Chibi proportions (LOCKED — every CHARACTER & creature obeys)

All characters/creatures/war-machines (player tank, enemies, the boss) are drawn
**heavily chibi** — exaggerated cute, toy-like proportions:

- **Big top, tiny bottom.** The head / main mass dominates: roughly **60–70% of
  the sprite's height and width**. The body/base/legs are small and stubby —
  total height ≈ **1.5–2 "heads"**. Machines read like chunky toys, not scale
  models.
- **Oversized features.** Huge expressive eye(s) / lens / core (a single big eye
  or one dominant cluster), oversized cannon-mouth or maw. Few BIG features beat
  many small ones.
- **Round & bulbous.** Soft, fat, rounded silhouettes — bubble bodies, pudgy
  cheeks, rounded shoulders. Sharp bits (spikes, tusks, barrels) are short, blunt
  and stubby, used as accents on an otherwise round form.
- **Stubby limbs.** Tiny arms/legs/treads — short, fat, minimal; little feet
  peeking out under a big body. Weapons are small and held close.
- **Less detail, more charm.** Simplify: a few chunky shapes, generous outlines,
  no fussy filigree. Cuteness and silhouette read over realism.
- **Keep everything else.** Same palette, same `#101218` chunky outlines + one
  rim + one inner shadow, same team-read (blue=ally / red-green=enemy), same
  facing-UP and centered-with-margin rules, same ground shadow. Chibi changes
  **proportions**, not the rendering language.

The boss stays the biggest, most menacing silhouette — but a chibi menace:
oversized angry eye, fat round body bristling with stubby bone spikes, tiny feet.

## Camera & facing convention (LOCKED — every sprite obeys)

- **High 3/4 top-down**: the camera looks down at the arena from a steep
  three-quarter angle (slightly tilted, not pure orthographic top). Subjects show
  their **top surfaces dominantly** plus a sliver of the front/forward face.
- **Default facing = UP (north, toward −Y).** Every sprite is authored pointing
  toward the top of its viewBox. The engine rotates sprites around center for
  other headings; do not bake rotation into the art.
- **Centered in a square viewBox `0 0 S S`.** Keep a transparent margin of
  **~8% of S** on all sides so glows/outlines/rim never clip.
- **Ground anchor:** standing structures cast a soft elliptical **ground shadow**
  centered under the pivot (use `--shadow` at low opacity), so they read as
  planted on the field, not floating.

## PALETTE (copy these hexes verbatim)

| Token | Name | Hex | Group |
|-------|------|-----|-------|
| `--ground-deep`   | Deep Battlefield   | `#171a24` | background / ground |
| `--ground`        | Ash Soil           | `#262b3a` | background / ground |
| `--shadow`        | Cast Shadow        | `#0b0d14` | background / ground |
| `--steel-dark`    | Gun Steel          | `#3a4a63` | player faction |
| `--steel`         | Heroic Steel       | `#6f8db3` | player faction |
| `--steel-light`   | Frost Rim          | `#bcd6f2` | player faction |
| `--brass`         | Warm Brass         | `#caa24a` | player faction (accent) |
| `--ally-glow`     | Aether Blue        | `#3fa9ff` | player faction (energy) |
| `--rot-dark`      | Rot Shadow         | `#1f3322` | enemy faction |
| `--rot`           | Plague Flesh       | `#5e8c3a` | enemy faction |
| `--rot-bright`    | Sick Bile          | `#a6d65e` | enemy faction (accent) |
| `--bone`          | Old Bone           | `#d9d2b0` | enemy faction |
| `--menace`        | Hellfire Maw       | `#ff5a3c` | enemy faction (energy) |
| `--ui-line`       | Outline Ink        | `#101218` | neutral / UI |
| `--ui-text`       | Parchment          | `#e8e3d2` | neutral / UI |
| `--rarity-common`   | Common Grey   | `#9aa3ad` | rarity |
| `--rarity-uncommon` | Uncommon Green| `#4fc36a` | rarity |
| `--rarity-rare`     | Rare Blue     | `#3f8cff` | rarity |
| `--rarity-epic`     | Epic Violet   | `#b04dff` | rarity |

## Line & shading rules

- **Outline:** every readable shape gets a `--ui-line` (`#101218`) stroke.
  Weight scales with canvas: **outer silhouette 4–5 units**, internal divisions
  **2.5–3.5 units**, at the authored scale (S=192 → ~4u outer; S=320 → ~5u outer).
  Use `stroke-linejoin="round"` for a forged, chunky feel.
- **Fills are flat two-tone.** Each major form = one base fill + ONE inner-shadow
  shape (the base color stepped one rank darker, ~70% coverage on the lower/away
  side) + ONE rim-light sliver (the base's light token) along the top/forward
  edge facing the camera. No multi-stop gradient soup.
- **Glow / energy:** emissive cores (`--ally-glow`, `--menace`, `--rot-bright`)
  use a soft blurred halo (`feGaussianBlur`) UNDER a crisp solid core. Halo only
  on energy, never on metal.
- **Team-read at a glance:**
  - **Player / friendly** = cool steel-blue body + warm brass trim + **blue
    aether glow**. Symmetrical, architectural, riveted, planted.
  - **Enemy** = rot-green flesh + bone + **orange/red hellfire glow**. Asymmetric,
    organic, jagged, leaning forward.
  - When in doubt: blue glow = ours, red/green = theirs.

## Rarity usage

Shop/item frames and drop beams use the four rarity hexes as the dominant border
and glow color. Keep item interiors neutral so the rarity color reads instantly.

## Do / Don't

**Do**
- Keep silhouettes bold and instantly recognizable at 48–96px.
- Use the named tokens; step a color one rank for shadow instead of inventing one.
- Round chunky joins; thick dark outlines; one rim + one inner shadow per form.
- Center in the square; leave the 8% margin; add the ground shadow.

**Don't**
- No raster, no external fonts/images, no `<image>` links — self-contained SVG.
- No baked rotation (author facing UP only).
- No gradient soup, no photoreal noise/textures, no hairline 1px detail.
- Don't mix team energy colors (no blue glow on enemies, no red glow on allies).
- Don't let glow halos touch the silhouette edge so they survive the margin.
