# sim-core

The **deterministic simulation core** for Standing Tank Defense — engine-independent, fixed-tick, fixed-point. Everything the netcode relies on lives here (`docs/03`, `docs/05`). No Godot/engine types; the renderer will only ever *read* this state.

## Milestone status: **M0 complete** ✅

Single-arena deterministic sim + the determinism gate (`docs/06` M0). Exit test holds: same `(seed, input log)` ⇒ **bit-identical** `state_checksum` every tick, verified same-machine and (via the golden-checksum test) across the CI OS matrix.

## Layout

```
sim-core/
├── crates/
│   ├── determinism/   # Fixed (Q47.16), SplitMix64 Rng + purpose streams, FNV-1a Checksum, isqrt. ZERO deps.
│   ├── sim/           # the simulation
│   │   ├── lib.rs      step() order-of-operations + checksum()  [central seam]
│   │   ├── ids.rs      ids, Purpose, Input                       [central seam]
│   │   ├── state.rs    ArenaState data model + Vec2 math         [central seam]
│   │   ├── content.rs  M0 catalog (from the extracted map data)  [central seam]
│   │   ├── combat.rs   weapons / projectiles / enemy movement
│   │   ├── waves.rs    spawning
│   │   ├── economy.rs  income / bounty / death
│   │   ├── shop.rs     offer generation / reroll
│   │   └── input.rs    apply one player action
│   └── harness/       # determinism gate: run a scenario, compare checksum traces
└── ...
```

## Run it

```bash
cd sim-core
cargo test --workspace      # 48 tests, incl. the cross-platform golden checksum
cargo run -p harness        # the M0 DETERMINISM GATE (prints PASS/FAIL)
cargo run -p harness --example liveness   # sanity: shows the sim actually simulates
```

## Invariants (enforced; see `CLAUDE.md`, `docs/05 §5.6`)

- Fixed 30 Hz tick; **no floats** in anything feeding `state_checksum` — `Fixed`/`i64` only.
- Deterministic PRNG, drawn from the correct **purpose stream** (`spawn`/`targeting`/`shop`/`reroll`/`proc`); never system RNG, never wall-clock.
- **Stable iteration order** (by id); never iterate a `HashMap`.
- `step()` phase order is part of the spec — client and shadow-sim must match it.
- `overflow-checks = true` in every profile (no silent wrapping).

## Next: M1

In-process shadow-sim + snapshot serialize/replay + deliberate-divergence correction (`docs/06`). The core is already shaped for it: `ArenaState` is `Clone`/`PartialEq` and all RNG is cursor-serializable.
