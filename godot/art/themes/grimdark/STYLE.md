# Grimdark Theme — STYLE GUIDE (LOCKED)

> The authoritative visual contract for the **grimdark** theme of *Standing Tank Defense*.
> Other agents: copy hex values **verbatim**. Do not invent colors or re-derive shading rules.

## Direction: "Ashen Vigil"

A grim, painterly dark-fantasy battlefield seen from a high 3/4 top-down. The world is a
cold, ash-choked dusk: nearly-black ground swallows the screen while a single warm focal
light carves out lit subjects with **volume and meat** — sculpted lit and shadowed sides
that blend, never flat cel bands. It is **visceral** (weight, grime, dried blood, ember
glow, grounded menacing silhouettes) but **easy to look at**: a calm dark stage, a tightly
limited desaturated palette, and one crisp readable silhouette per subject that pops out of
the murk. Natural / heroic proportions — deliberately NOT chibi.

---

## PALETTE (copy hex verbatim)

### Background / Ground (dark, calm stage — the eye rests here)
| Name             | Hex       | Use                                              |
|------------------|-----------|--------------------------------------------------|
| `bg_void`        | `#0A0B0F` | Deepest background, viewBox backdrop, void edges |
| `bg_ash`         | `#15171E` | Ash-field ground base, dark fills                |
| `ground_loam`    | `#23252E` | Lit ground / dirt mid-tone                       |
| `shadow_cast`    | `#06070A` | Ground shadows, deep-crevice ambient occlusion   |

### Player faction — cold iron & steel + dim brass + a COLD glow (ally read)
| Name             | Hex       | Use                                              |
|------------------|-----------|--------------------------------------------------|
| `iron_dark`      | `#1C212B` | Steel shadow side, silhouette rim                |
| `iron_steel`     | `#3B4655` | Cold steel mid-tone (base metal)                 |
| `iron_light`     | `#6E7C8C` | Steel lit side / rim light                       |
| `brass_dim`      | `#8A6A33` | Dim brass fittings, bolts, reinforcing bands     |
| `glow_cold`      | `#5BC8E6` | COLD ally glow — core/lens, ally team-read       |

### Enemy faction — diseased flesh / bone + ember & blood glow (enemy read)
| Name             | Hex       | Use                                              |
|------------------|-----------|--------------------------------------------------|
| `flesh_diseased` | `#5C6B4A` | Sickly green-grey diseased flesh mid-tone        |
| `flesh_light`    | `#8A9468` | Lit flesh / sinflamed highlight                  |
| `bone_pale`      | `#C9C1A8` | Exposed bone, claws, teeth                       |
| `blood_dried`    | `#6E1F1C` | Dried-blood accents, gore, wounds                |
| `glow_ember`     | `#FF7A2E` | ENEMY ember glow — eyes, energy, enemy team-read |

### Neutral / UI
| Name             | Hex       | Use                                              |
|------------------|-----------|--------------------------------------------------|
| `ui_parch`       | `#D8CFB8` | Label text, UI ink, readable light on dark       |
| `ui_iron`        | `#2A2E38` | UI panel/frame base                              |

### Rarity (loot tiers — fixed)
| Name              | Hex       | Tier                                            |
|-------------------|-----------|-------------------------------------------------|
| `rarity_common`   | `#9CA3AE` | Common — dull pewter                             |
| `rarity_uncommon` | `#5E9C6B` | Uncommon — muted moss green                      |
| `rarity_rare`     | `#3E78C8` | Rare — cold steel blue                           |
| `rarity_epic`     | `#9A4FD0` | Epic — bruised violet                            |

---

## Painterly shading rules (this theme's signature)

1. **Volume, not cel bands.** Every major form is filled with a soft `radialGradient` or
   `linearGradient`: a **lit side** (toward the warm key light) blending into a **shadowed
   side**. Use 2–4 gradient stops. Forms should read as "meat" / sculpted mass, never flat.
   Key light convention: a warm overhead-front key (upper area), so tops/fronts catch light,
   undersides fall into shadow.
2. **Ambient occlusion.** Tuck low-opacity (`0.25–0.5`) blurred `shadow_cast` shapes into
   crevices, seams, under overhangs and where forms meet. Use a Gaussian-blur filter.
3. **One warm rim/ember highlight per major form.** A single thin rim catch on the lit edge
   (steel → `iron_light`; flesh → `flesh_light`). Do not over-light; one accent per form.
4. **Crisp silhouette outside, painterly inside.** Wrap the outer form in a dark edge — a
   `iron_dark`/`shadow_cast` deep-shadow rim **or** a thin dark stroke (~3–5px at native S) —
   so the shape reads at small sizes. Painterly gradients live *inside* that edge.
5. **Glow only on emissive cores.** Soft blurred glow (`feGaussianBlur`) is reserved for
   emissive elements: ally `glow_cold` cores/lenses, enemy `glow_ember` eyes/energy.
   No gradient-soup rainbows. Glow stays within the palette.
6. **Limited & desaturated.** Stay inside this palette. Backgrounds dark and calm; saturation
   and brightness are spent on the lit subject and its single glow, not spread everywhere.

---

## Camera / facing / centering convention (HARD CONTRACT)

- **Camera:** high 3/4 top-down.
- **Facing:** author every sprite facing **UP (north, −Y)**. Bake **NO rotation** — the
  engine rotates the sprite around its center.
- **Canvas:** centered in a **square** viewBox `0 0 S S`, with **~8% transparent margin** on
  all sides so glow/shadow never clips.
- **Ground shadow:** standing subjects get a soft elliptical dark (`shadow_cast`) ground
  shadow under the pivot.
- **Canvas sizes (S):** player tank = **192**; Samwise (boss) = **320**; (full set later:
  normal enemies 96, projectiles 48, fx 128, ui icons 48–64, frames 96, panel 320×96,
  env 1024).
- **Self-contained SVG only.** No raster, no external fonts/images/links. Must be well-formed
  XML.

## Team-read rule (non-negotiable)

- **Ally = COLD glow** (`glow_cold`, cyan-blue). Player tank, ally projectiles, ally FX.
- **Enemy = WARM glow** (`glow_ember`, orange; gore uses `blood_dried`). Enemies, enemy eyes,
  enemy energy/FX.
- A player must distinguish friend from foe by glow temperature alone, at a glance.

## Do / Don't

**DO**
- Build forms from gradient-filled shapes with a lit + shadow side that blend.
- Keep backgrounds near-black and uncluttered; let the lit subject pop.
- Give every standing subject weight: a grounded base, a ground shadow, a heavy silhouette.
- Use brass/bone/blood/ember as sparse accents, not fields of color.
- Keep one clean readable silhouette per subject.

**DON'T**
- No flat cel-shading bands (this is the painterly theme — distinct from chibi).
- No chibi / big-head proportions; use natural / heroic proportions.
- No busy texture, noise, clutter, or rainbow gradient-soup.
- No glow on non-emissive surfaces; no rotation baked into sprites.
- No floats-into-bright backgrounds; the stage stays dark and calm.
- Never mix team glow temperatures (ally cold / enemy warm only).
