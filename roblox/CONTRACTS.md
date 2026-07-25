# Roblox Fork — Interface Contracts

**These are the seams. They are owned centrally and must not be changed by a single component's author.** If a contract here is wrong or insufficient, say so in your report — do not silently widen it.

Design decisions these serve: [`docs/10-roblox-fork-design.md`](../docs/10-roblox-fork-design.md). Feasibility background: [`docs/09-roblox-port-assessment.md`](../docs/09-roblox-port-assessment.md).

---

## C1 — `content.json`

Generated from `sim-core/crates/sim/src/content.rs` by `cargo run -p sim --bin export-content` (or equivalent). **The Rust tables stay the single source of truth**; the JSON is a build artifact, checked in so the Roblox side has no Rust toolchain dependency.

Payload enums (`Attack`, `WeaponAbility`, `EnemyAbility`, `ModEffect`) are encoded with the **existing `words()` pattern already in `content.rs`** — a tag plus up to three `i64` operands:

```json
{ "tag": 3, "a": 250, "b": 0, "c": 0 }
```

Where a `words()`/`from_words()` pair does not yet exist for one of those enums, add it in the same style as `WeaponAbility::words` and `ModEffect::words`, and keep it lossless — `from_words(e.words()) == Some(e)` for every variant, proven by a round-trip test over every catalog entry.

Top-level shape:

```json
{
  "schema_version": 1,
  "content_hash": "<hex>",
  "damage_matrix": [[ /* [damage_type][armor_class] -> Fixed raw i64 */ ]],
  "weapons":   [ { "name","rarity","cost","damage","damage_type","attack":WORDS,
                   "cooldown_ticks","range","proj_speed",
                   "on_hit":{"poison_dps","poison_ticks","frost_stacks","fire_stacks","stun_ticks"},
                   "ability":WORDS } ],
  "enemies":   [ { "name","base_hp","move_speed","contact_damage","bounty",
                   "armor_class","archetype","ability":WORDS,"boss" } ],
  "modifiers": [ { "name","rarity","cost","effects":[WORDS],
                   "ramp": null | {"effect":WORDS,"interval_ticks":N} } ],
  "waves":     [ { "enemy","cadence_ticks","start_tick" } ],
  "timeline":  { "steam": { /* the constants as they are in content.rs today */ },
                 "roblox": { /* the F1 rescale */ } },
  "constants": { "frost_max_stacks": 25, "ramp_per_round": 900, "...": 0 }
}
```

Rules:

- **Array index is identity.** `weapons[i]` must correspond to weapon id `i` exactly as the Rust indexes it. Named ids (`STARTING_WEAPON`, `DEATH_ENGINE`, `BOSS`) go in `constants`.
- **All numbers are integers.** Ratios stay as `(num, den)` operand pairs inside `WORDS`; anything already `Fixed` is emitted as its **raw `i64`**. No floats anywhere in this file.
- `content_hash` is **FNV-1a-64 over the canonical (compact, insertion-ordered) serialization of the whole document with the `content_hash` member removed.** Reproducible by any consumer from the artifact alone. (The original wording said "everything above it", which read literally would have covered only `schema_version`.)
- `timeline.roblox` carries the F1 rescale: boss at tick 9000, 600-tick rounds, **ramp interval 900** — deliberately *not* the round length, so `ramp_intervals_to_boss` stays equal to Steam's 10 and the difficulty endpoint survives the rescale untouched (`docs/10` F1). Both timelines ship so the fork's divergence from Steam is visible in one place.

### C1 addenda (blessed centrally after the first export)

- **`boss_escort` and `spawn_ring` are part of the contract.** The boss phase and enemy placement are unreachable without them; both reuse existing shapes (`WaveSpawn` entries, `Fixed` raws).
- **`archetype` is an integer id** (0–6), with the mapping mirrored into `constants.archetype_*`. It is flavor/telemetry, not one of the four payload enums, so it does not use `WORDS`.
- **`enemies[].boss` is a JSON boolean.** The "all numbers are integers" rule governs numbers; a flag should be a flag.
- **`EnemyAbility::RangedAttack` packs four fields into three operands**: `b = cooldown_ticks | (damage_type << 32)`, lossless for all `(u32, u8)`. The shift ships as `constants.enemy_ranged_dtype_shift`; Luau unpacks with `b // 2^32` and `b % 2^32`. Accepted over burning tag space.
- **`Attack`'s tag is identical to `attack_scope_id`** (test-asserted), so a port carries one mapping rather than two.

## C2 — `Fixed.luau`

Exact emulation of `determinism::Fixed` (`i64` Q48.16, `i128` intermediates, **saturating** narrowing to the `i64` boundary). Luau numbers are doubles with a 53-bit integer-safe mantissa, so raw values must be carried as split limbs — a plain `a*b` on raw Q16.16 values overflows the mantissa and silently loses precision.

Required API, semantics matching the Rust function-for-function:

```
Fixed.FRAC_BITS = 16 · Fixed.ONE · Fixed.ZERO
Fixed.fromInt(i) · Fixed.fromRaw(raw) · Fixed.raw(x) · Fixed.fromRatio(num, den)
Fixed.floorToInt(x)          -- arithmetic shift, floors toward -inf (NOT truncation)
Fixed.add/sub/neg            -- saturating
Fixed.mul(a, b)              -- (a*b) >> 16 via exact i128 intermediate, then saturate
Fixed.div(a, b)              -- (a << 16) / b  via exact intermediate, then saturate
Fixed.scaleI64(x, v)         -- (v * x) >> 16, saturating
Fixed.sqrt(x)                -- floor integer sqrt; errors on negative
```

Non-negotiable: `floorToInt` floors toward negative infinity (Rust `>>`), **not** toward zero. Division and shift semantics on negatives are the most likely place a port silently diverges.

## C3 — `Rng.luau`

Exact emulation of `determinism::Rng`:

```
Rng.fromSeed(seed) · Rng.state(r)
Rng.derive(master, player, purpose, round)
Rng.nextU64(r) · Rng.nextU32(r) · Rng.below(r, n) · Rng.chance(r, num, den)
```

`below` must reproduce the Rust rejection/modulo strategy **exactly** — an "equivalent" unbiased method that draws a different number of words desynchronizes every downstream stream. Read the Rust before writing it.

## C4 — Parity vectors

The correctness oracle (`[09] §1.5`). The Rust side emits `roblox/test/vectors/*.json`; the Luau side asserts against them.

- `fixed_vectors.json` — operand pairs and expected raw results for every `Fixed` op, **including** negatives, zero, saturation boundaries (`i64::MAX/MIN`), and `floorToInt` on negatives.
- `rng_vectors.json` — for several seeds and `derive` tuples, the first N outputs of each of `nextU64`/`nextU32`/`below`/`chance`.

Expected values come from **running the Rust**, never from hand-derivation.

### C4 addenda (blessed centrally after the first export)

- **Every `i64`/`u64` in both vector files is a 16-digit lowercase hex string, two's complement** — each file carries an `encoding` field saying so. This is forced: `i64::MAX`/`MIN` cannot survive as JSON numbers for a Luau consumer. Decoders must go from hex straight into the split-limb representation, never through a Lua `number` intermediate, or the boundary cases the vectors exist to test are lost on the way in. `content.json` is unaffected — every catalog value sits far inside the safe range and stays a plain integer.
- **`from_int` and `from_ratio` truncate on narrowing; `mul`, `div` and `scale_i64` saturate.** This asymmetry is real Rust behavior and a prime divergence risk, so the truncating cases ship in separate `from_int_wrapping` / `from_ratio_wrapping` arrays rather than mixed into the main lists. A port must handle wrapping narrowing bit-for-bit, not "sensibly".
- **Not covered by vectors:** `div` by zero (panics in Rust) and `sqrt` of a negative (asserts). A Luau port should error on both; the behavior is deliberately unspecified beyond that.

## C5 — Module layout

```
roblox/
  default.project.json        Rojo
  README.md                   (owned by the scaffold task)
  CONTRACTS.md                (this file — central)
  src/shared/Fixed.luau  Rng.luau  Content.luau  content.json
  src/server/                 Arena, Director        (later milestone)
  src/client/                 render + UI            (later milestone)
  bench/                      throughput harness
  test/                       parity runner + vectors
```

Luau modules are plain `ModuleScript`-style returns and must **not** reference any Roblox global (`game`, `workspace`, `task`, `Instance`) in `src/shared/` — that layer runs unchanged under a standalone `luau` interpreter, which is how it gets tested in CI. This is the Luau restatement of the standing invariant that the sim core stays engine-independent.
