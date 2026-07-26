# R3 — Luau Simulation Core: Module Spec

Central seam document for the R3 transcription (`docs/10` F7). Owned centrally; a single module's author does not change it. If it is wrong or insufficient, **report that** — do not widen it locally.

Prerequisites already landed and verified: `Fixed.luau`, `Rng.luau`, `Json.luau` (R0, parity + differential green), `content.json` (R1).

---

## S1 — The governing principle

**This is a transcription, not a redesign.** The Rust in `sim-core/crates/sim/src/` is the specification. Every behavioral question is answered by reading it. Do not improve, simplify, reorder, or "fix" anything — a behavioral difference is a *bug*, even when the Luau version looks better, because it breaks the oracle that makes this whole approach cheap.

If you believe the Rust is wrong, report it. Do not act on it.

## S2 — Module map (mirrors the Rust 1:1)

| Luau module | Rust source | Owner |
| --- | --- | --- |
| `src/shared/Checksum.luau` | `determinism::Checksum` | Foundation |
| `src/shared/Content.luau` | `content.rs` (via `content.json`) | Foundation |
| `src/shared/sim/State.luau` | `state.rs`, `ids.rs` | Schema |
| `src/shared/sim/Input.luau` | `input.rs` | Schema |
| `src/shared/sim/Combat.luau` | `combat.rs` | Combat |
| `src/shared/sim/Modifiers.luau` | `modifiers.rs` | Economy |
| `src/shared/sim/Economy.luau` | `economy.rs` | Economy |
| `src/shared/sim/Status.luau` | `status.rs` | Status |
| `src/shared/sim/Defense.luau` | `defense.rs` | Status |
| `src/shared/sim/Waves.luau` | `waves.rs` | Waves |
| `src/shared/sim/Shop.luau` | `shop.rs` | Waves |
| `src/shared/sim/View.luau` | `view.rs` | Runner |
| `src/shared/sim/Arena.luau` | `lib.rs` (`step`, `checksum`) | Runner |
| `src/shared/sim/Bot.luau` | `bot.rs` | Runner |

`snapshot.rs` is **not** ported: F6 makes the Roblox build server-authoritative, so there is no reconnect-by-replay and no shadow-sim to serialize for.

## S3 — Naming and schema conventions (non-negotiable)

- **State field names are byte-identical to the Rust**, in `snake_case`: `tank_hit_this_tick`, `pending_kills`, `next_entity_id`, `rng_spawn`. This is not stylistic — identical names make oracle trace diffs readable and make transcription errors visible.
- **Struct shapes mirror `state.rs` exactly** — same fields, same nesting. `State.luau` is a direct reading of that file.
- **Functions are `camelCase`** and mirror the Rust function names: `combat::fire_weapons` → `Combat.fireWeapons`.
- Modules **mutate the state table in place**, matching Rust's `&mut ArenaState`. No module returns a new state.
- `Option<T>` becomes `nil`. `Vec<T>` becomes a 1-indexed array table — but **iteration order must match the Rust's**, and any Rust code that sorts or iterates by id must sort or iterate by id in Luau too.
- **1-indexing is a transcription hazard.** Rust `Vec` indices, `def: u16` content indices, and `slot` inputs are all **0-based**. Content ids stay 0-based (they index `content.json`, where array position *is* identity per C1). Only Luau array storage is 1-indexed. Be explicit at every boundary; this is where silent off-by-ones will live.

## S4 — Numeric rules

- **Anything the Rust types as `Fixed`, you hold as a `Fixed` value** from `Fixed.luau`. Anything typed `i64`/`u32`/`u16` is a plain Luau number (all such quantities sit far inside 2^53).
- **Never index into a `Fixed` value.** It is opaque. Its internal representation is a central concern and may change — treating it as opaque is what keeps the F5 escape hatch and any future storage optimization available. Use only the `Fixed.luau` API.
- Integer division and shifts in the Rust floor toward **negative infinity**. Luau's `/` is float division and `//` floors — neither is automatically right. Match the Rust semantics deliberately at every site.
- **RNG stream discipline is sacred.** Each stream must be drawn from in exactly the same order, the same number of times, with the same number of consumed words as the Rust. A single extra or skipped draw desynchronizes everything downstream and the oracle will catch it — but far from the cause. Where the Rust conditionally draws, the Luau must conditionally draw identically.

## S5 — Tick order

`Arena.step(state, input)` implements `lib.rs::step` **in exactly this order**. The order is part of the spec (`docs/05 §5.6.1`); it is not an implementation detail:

1. Dead short-circuit: if `dead`, increment `tick` and return (traces stay length-aligned).
2. Round boundary → `Shop.generateOffers`, `Economy.onRoundStart`, reset `tank.spikes_stacks`.
3. `Input.apply`
4. `Modifiers.applyRamps`
5. `Waves.spawn`
6. `Status.pulse`
7. `Combat.fireWeapons`
8. `Combat.advanceProjectiles`
9. `Combat.moveEnemies`
10. `Combat.enemyRangedAttacks`
11. `Defense.spikes`
12. `Defense.shieldBreakStun`
13. `Combat.tickHazards`
14. `Combat.tickAura`
15. `Combat.tickMinions`
16. `Status.tick`
17. `Economy.collectBounties`
18. `Economy.tickIncome`
19. `Defense.regen`
20. `Economy.resolveDeaths`
21. `tick += 1`

Read `lib.rs` for the comments attached to each phase — several encode real constraints (e.g. `shieldBreakStun` runs after `spikes` because `spikes` consumes the hit flag).

## S6 — The done-condition: checksum parity against the Rust

`Arena.checksum(state)` ports `lib.rs::checksum` exactly, over `Checksum.luau` (a port of `determinism::Checksum`).

**R3 is done when the Luau per-tick checksum trace is identical to the Rust's, for every seed in the trace corpus, for the full run length.** Not "looks right", not "plays fine" — byte-identical `u64` per tick.

Trace format (`roblox/test/traces/`), one JSON document per seed:

```json
{ "schema_version": 1, "seed": 12345, "content_hash": "<hex>",
  "inputs": [ {"tick": 0, "code": 0}, ... ],
  "checksums": [ "<16-hex u64>", ... ] }
```

- `checksums[i]` is the checksum **after** tick `i` completes.
- `inputs` is the scripted input log — a fixed, seeded `Bot` script, so the Luau side replays identical decisions rather than reimplementing bot judgment as a prerequisite.
- `content_hash` must match `content.json`'s, so a stale trace fails loudly instead of silently.
- `u64` values are 16-digit lowercase hex, per `CONTRACTS.md` C4.

A `--verbose` mode additionally dumps every checksummed field for a given tick, so that when a trace diverges the first differing field is one command away. Without that, debugging a checksum mismatch across 9,000 ticks is miserable — build it.

## S7 — Working rules for module authors

- **Read your Rust source completely before writing.** These modules carry accumulated fixes and source-fidelity notes that are not derivable from first principles.
- **Preserve the Rust's comments** where they explain *why*. They are the record of decisions and edge cases.
- You may write a **minimal local stub** for a module another agent owns, purely to run your own tests — **delete it before reporting**. Never commit a stub for someone else's file.
- Stay strictly inside your file list. Everything is being built concurrently; touching a neighbour's file loses their work.
- Run `luau-analyze` on your files and leave it clean.
- The interpreter and analyzer are at:
  `/tmp/claude-0/-home-user-standingtankdefense/d7ede231-09e9-5d42-8af3-755591abd79a/scratchpad/luau/build/{luau,luau-analyze}`
- Standalone Luau has **no file I/O**. Tests take data via argv after `-a` — see the header of `roblox/test/parity.luau` for the established pattern.
- `src/shared/` may reference **no Roblox global**. That invariant is what lets all of this be tested under a bare interpreter.

## S8 — Report contract

Report: the exact public API you exposed; which Rust behaviors were subtle enough to be worth calling out; anything in this spec that was wrong or underspecified (**flag, do not widen**); your test output; and any place you knowingly deviated from the Rust, with the reason. A silent deviation is the one outcome that makes the oracle worthless.
