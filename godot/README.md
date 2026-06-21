# Standing Tank Defense — Godot 4 front-end

A thin graphical shell over the deterministic Rust core (`sim-core`). Godot does
**rendering, UI, and input only**; all game logic runs in `sim` and is reached
through the `StSim` GDExtension node (`rust/src/lib.rs`). This is the locked
architecture (`docs/08`): engine on top, decoupled deterministic sim underneath.

> This folder is intentionally **outside** the `sim-core` Cargo workspace, so the
> core stays engine-independent and its zero-dependency CI build is unaffected.
> Building here fetches the `godot` crate and needs a Godot 4 editor — do it on
> your own machine, not in the sandbox.

## Prerequisites
- **Rust** (stable) and **Godot 4.3+**.

## Build & run
```bash
# 1. Build the GDExtension binding (from this folder):
cd rust
cargo build                 # debug → rust/target/debug/libstanding_tank_gdext.*
cd ..

# 2. Open this folder in Godot 4.3+ and press Play (F5).
#    The .gdextension points at the debug artifact you just built; Godot loads
#    StSim automatically. (Release: `cargo build --release` + run the exported
#    game, which reads the release path.)
```
If Godot can't find `StSim`, confirm the library exists at the path in
`standing_tank_defense.gdextension` for your platform, then reopen the project.

## Controls
- **1 / 2 / 3** — buy shop slot 1/2/3
- **R** — reroll the shop
- **Space** — Clear (board wipe; the only thing that hurts the boss)
- **Esc** — quit

## How it's wired
- `rust/src/lib.rs` — `StSim`: owns an `ArenaState`, `step(code, slot)` advances
  one 30 Hz tick, and the getters return flat `Packed*Array`s built from
  `sim::view` (the engine-agnostic render contract).
- `main.gd` — drives exactly one `sim.step()` per `_physics_process` (physics is
  pinned to 30 Hz in `project.godot`) and draws whatever the sim reports.
- `Main.tscn` / `project.godot` — entry scene and config.

Because the sim is deterministic, the same seed (set in `main.gd`'s
`new_match`) always produces the same match — the foundation the netcode
(`sim-core/crates/net`) builds on.

## Versions (verified)
Built and run headlessly against **Godot 4.4.1** with **godot-rust 0.5.3**.
The binding pins the `api-4-3` feature, so the compiled extension loads on
**Godot 4.3 and every newer 4.x** (forward-compatible). If you target an even
older editor, lower the feature and `compatibility_minimum` to match; if your
toolchain is older, gdext 0.5 still applies.
