# sim-core

The **deterministic simulation core** for Standing Tank Defense — engine-independent, fixed-tick, fixed-point. Everything the netcode relies on lives here (`docs/03`, `docs/05`). No Godot/engine types; the renderer will only ever *read* this state.

## Milestone status: **M0 + M1 + M2 complete** ✅

- **M0** — single-arena deterministic sim + determinism gate. Same `(seed, input log)` ⇒ **bit-identical** `state_checksum` every tick, verified same-machine and (golden-checksum test) across the CI OS matrix.
- **M1** — in-process shadow-sim + byte snapshots. `sim::snapshot` round-trips `ArenaState` to portable bytes; `harness::shadow::ShadowRunner` detects client/shadow divergence at digest boundaries and corrects from a snapshot; `replay_from_snapshot` is the reconnect path. Injected divergence is detected and corrected within one interval and reconverges.
- **M2** — authoritative match director + transport (`crates/net`). Transport-abstracted (deterministic `Hub` now; Steam `ISteamNetworkingSockets` adapter drops in behind the same interface). `Director` owns the clock, per-player shadow-sims, input ordering/acks, digest→snapshot correction, and death/placement; thin non-predictive `Client`; `wire` codec + shared `Schedule`. **Exit test proves the thesis**: one client's stall can't perturb another client's arena.

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
│   ├── harness/       # determinism gate + shadow-sim (M0/M1)
│   └── net/           # M2: match director, thin client, wire codec, schedule,
│                      #     and a deterministic Hub (latency/stall injection)
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

## Next: M3

8-player lobby, reconnect, and host migration over Steam (`docs/06`/`docs/07`): full 8 slots via Steam lobby + SDR relay; reconnect (re-seed + replay, already proven in M1/M2); director-role migration to a hot standby on host loss. Then M4 wires the real `ISteamNetworkingSockets` adapter behind the M2 transport interface and imports the full content catalog.
