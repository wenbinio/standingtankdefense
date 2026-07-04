# sim-core

The **deterministic simulation core** for Standing Tank Defense — engine-independent, fixed-tick, fixed-point. Everything the netcode relies on lives here (`docs/03`, `docs/05`). No Godot/engine types; the renderer will only ever *read* this state.

## Milestone status: **M0–M4 complete ✅ · M5 partial ◐**

(Per-milestone proof pointers and the open live-Steam items are annotated in `docs/06 §6.1`.)

- **M0** — single-arena deterministic sim + determinism gate. Same `(seed, input log)` ⇒ **bit-identical** `state_checksum` every tick, verified same-machine and (golden-checksum test) across the CI OS matrix.
- **M1** — in-process shadow-sim + byte snapshots. `sim::snapshot` round-trips `ArenaState` to portable bytes; `harness::shadow::ShadowRunner` detects client/shadow divergence at digest boundaries and corrects from a snapshot; `replay_from_snapshot` is the reconnect path. Injected divergence is detected and corrected within one interval and reconverges.
- **M2** — authoritative match director + transport (`crates/net`). Transport-abstracted (deterministic `Hub` now; Steam `ISteamNetworkingSockets` adapter drops in behind the same interface). `Director` owns the clock, per-player shadow-sims, input ordering/acks, digest→snapshot correction, and death/placement; thin non-predictive `Client`; `wire` codec + shared `Schedule`. **Exit test proves the thesis**: one client's stall can't perturb another client's arena.
- **M3** — 8 players, reconnect, host migration. A mid-match `Client::reconnecting` adopts an authoritative snapshot and tracks the arena tick; **`eight_players_reconnect`**: one of 8 drops and rejoins on the canonical trajectory, the other 7 unaffected. **`host_migration`**: a hot-standby director (fed identical inbound, bit-identical at the handoff) takes over on host loss with clients continuing in sync.
- **M4** — content depth (systems **and** catalog complete). All gameplay systems are in and deterministic: the **modifier/stacking engine** (`sim::modifiers`, incl. defensive), **status effects** (`sim::status` — Poison/Frost/Fire/Stun), **all attack types** (Single/Splash/Barrage/Area/Wave/Bounce), the **defensive layer** (`sim::defense` — dodge→armor→mana-shield→HP + regen), **wave scaling** on the restored source arc — a smooth per-minute ramp, a +20% step at 10:00, and **The Hippocrate** boss at the 15-min tick (`BOSS_SPAWN_TICK` = 27000; Clear-only) with a post-15:00 "swift end" escalation, shop close, and ramp stop — and **Steam** results/adapter scaffolding (`net::results`, `net::steam`). A `full_arc` test drives the whole 11-phase pipeline to the boss byte-deterministically. The shipped catalog is **96 weapons / 110 modifiers / a 12-entry enemy roster** (`sim/src/content.rs`), gated against the docs by `sim/tests/doc_sync.rs`.
- **M5** — hardening (partial). Chaos/loss/reorder/latency injection (`net/tests/chaos.rs`), 8-player full-match soak (`net/tests/soak.rs`), and clock sync (`net/tests/clock_sync.rs`) are done and CI-gated. **Open:** everything needing a live Steam environment (real SDR transport + lobby, stats, release) — see `docs/06` + `docs/07a` and the adapter at `../adapters/steam-transport/`.

## Layout

```
sim-core/
├── crates/
│   ├── determinism/   # Fixed (Q47.16), SplitMix64 Rng + purpose streams, FNV-1a Checksum, isqrt. ZERO deps.
│   ├── sim/           # the simulation
│   │   ├── lib.rs      step() order-of-operations + checksum()  [central seam]
│   │   ├── ids.rs      ids, Purpose, Input                       [central seam]
│   │   ├── state.rs    ArenaState data model + Vec2 math         [central seam]
│   │   ├── content.rs  full catalog: 96 weapons / 110 modifiers, [central seam]
│   │   │               12 enemies incl. the boss, ramp/arc constants
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
cargo test --workspace      # full suite, incl. the cross-platform golden checksum
cargo run -p harness        # the M0 DETERMINISM GATE (prints PASS/FAIL)
cargo run -p harness --example liveness   # sanity: shows the sim actually simulates

# Watch the game play itself — a self-driving, full-loop preview over the real
# sim, rendered as text (the same read-only state a Godot front-end will draw):
cargo run -p preview              # headless: a few spaced frames + a summary line
cargo run -p preview -- --watch   # live at ~30 Hz (clears the screen each frame)
cargo run -p preview -- --watch --speed 8   # fast-forward to the 15-min boss
```

The `preview` crate is the first **integration seam**: it constructs a real
`ArenaState`, drives `sim::step` with a bot's `Input`s, and renders only public
sim state (tank, enemies, projectiles, economy, shop, arsenal). It proves the
whole pipeline composes into a playable match and is the reference for what the
engine front-end consumes.

## Invariants (enforced; see `CLAUDE.md`, `docs/05 §5.6`)

- Fixed 30 Hz tick; **no floats** in anything feeding `state_checksum` — `Fixed`/`i64` only.
- Deterministic PRNG, drawn from the correct **purpose stream** (`spawn`/`targeting`/`shop`/`reroll`/`proc`); never system RNG, never wall-clock.
- **Stable iteration order** (by id); never iterate a `HashMap`.
- `step()` phase order is part of the spec — client and shadow-sim must match it.
- `overflow-checks = true` in every profile (no silent wrapping).

## Golden checksum re-baseline (the runbook)

The M0 gate pins a **golden checksum** — the `state_checksum` of the scripted reference trace — as a constant in `crates/harness/tests/determinism.rs`. Any sim-behavior change (balance retune, new checksummed field, new step phase) legitimately shifts it; anything else shifting it is a **determinism bug**. This has been done six times in this repo's history; the procedure is:

1. **Confirm the shift is deliberate and reviewed.** Only an intentional, reviewed sim-behavior change may re-baseline. If you didn't mean to change sim behavior, stop — you've found a bug, not a re-baseline.
2. **Get the new value:** `cd sim-core && cargo run -p harness` — the gate prints `final_checksum = 0x…` (it checks run-vs-run identity, so it still passes; the *golden test* `cargo test -p harness --test determinism` is what fails until you update it).
3. **Update the golden constant** in `crates/harness/tests/determinism.rs` to the new value (and only that — if other tests break, your change did more than you think).
4. **Re-run the full suite:** `cargo test --workspace` from `sim-core/`, plus `cargo run -p harness` (must print PASS).
5. **Let the CI OS matrix confirm it** (Win/Linux/Mac must all agree on the new value — that cross-platform agreement is the actual guarantee; a value that differs per-OS is a determinism bug, not a re-baseline).
6. **Say so in the commit message** (what behavior change moved it), so the history of goldens stays auditable.

**The rule: the golden value only ever changes together with a deliberate, reviewed sim-behavior change — never to "make CI green."** (Referenced from `docs/06 §6.3`.)

## Remaining work: the live-Steam half of M5

The catalog and all gameplay systems are done. What's left needs a **real Steam environment**: wiring the `ISteamNetworkingSockets`/SDR adapter (`../adapters/steam-transport/`, runbook `docs/07a`) into a live 2-machine match, a real `ISteamMatchmaking` lobby, Steam stats/auth, a real `content_hash` (`docs/05 §5.7`), and the release pack. See `docs/06 §6.1` (M5) and `docs/09 §9.4-P6`.
