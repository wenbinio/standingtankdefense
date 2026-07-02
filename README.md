# Standing Tank Defense

> A multiplayer, last‑man‑standing, randomized tower‑defense / survival game built around **one tank you cannot move** — you only choose how to arm it. Inspired by the Warcraft III custom map **Tower Survivors** (itself inspired by *Vampire Survivors* / *Halls of Torment*), reimagined as a standalone title with a **network architecture designed for it from day one**.

This repository holds a **playable game**: the **specification package** (`docs/`), the finished **deterministic simulation core** (`sim-core/`) with the netcode milestones (M0–M4, plus most of M5's hardening) and the full content catalog implemented and test-gated, a **Godot 4 front-end** (`godot/`) with menus, lobby, challenges, localization, and an 8-arena net demo, and a prebuilt **Windows demo build** (`demos/`). The spec came first on purpose — networking is expensive to retrofit — and the core was then built netcode-first against it. What remains open is the *live-Steam* half of the ship track (real SDR transport, real lobbies, Steam stats) — see [`docs/06`](docs/06-roadmap-risks-testing.md) and [`docs/09`](docs/09-rebuild-plan.md).

**See it run now:** the whole game loop plays itself headlessly over the real sim —
```bash
cd sim-core && cargo run -p preview            # text preview (spaced frames + summary)
cd sim-core && cargo run -p preview -- --watch # live at ~30 Hz
```
The full **Godot 4 front-end** over the same core (via a Rust GDExtension binding) lives in [`godot/`](godot/) — see [`TUTORIAL.md`](TUTORIAL.md) to build and play it, or grab the prebuilt Windows build from [`demos/`](demos/).

**Shipping target:** a **free Steam app**, built on **Steamworks P2P / Steam Datagram Relay** — no server budget, the authority runs on a host player over Valve's relay backbone. See [`docs/07-steamworks-integration.md`](docs/07-steamworks-integration.md).

**The inspiration map was extracted and analyzed first-hand** (`TowerSurvivors v1.58.w3x`); the raw data and a re-runnable extractor live in [`research/tower-survivors-map/`](research/tower-survivors-map/) so future work doesn't repeat it, with the full catalog written up in [`docs/appendix-A-map-extraction.md`](docs/appendix-A-map-extraction.md).

---

## The pitch in one paragraph

Up to **8 players** each defend their **own** lane with a **single stationary tank**. You don't move and you don't aim — the tank auto‑fires. The entire game is **resource and build decisions**: as gold trickles in and random upgrade offers appear, you decide which weapons, modifiers, and economy upgrades to stack against an enemy tide that ramps in 3-minute steps for **30 minutes**, then the end boss — **The Hippocrate**. (The WC3 source map ran a 15-minute arc to its "Samwise" boss; see `docs/01`.) Everyone fights their *own* swarm in parallel; the multiplayer layer is a **race to outlast everyone else**. Die in the first half of the lobby and you lose; finish in the surviving half and you win; be the last tank standing and you score a **Last Stand**.

## Why this design is interesting for networking

The Warcraft III original ran on WC3's **peer‑to‑peer deterministic lockstep**: every machine simulates the *entire* shared world and the simulation can't advance a tick until **every** player's input has arrived. That is the right tool for a shared‑battlefield RTS, but it is the *wrong* tool here and it caused the classic problems players complained about — one laggy player stutters the whole lobby, a single desync ends the match for everyone, and there is no reconnect.

The key realization that drives this whole spec: **the arenas are independent.** Your enemies, projectiles, and tank never touch another player's arena. The only things that are genuinely *shared* are tiny and slow‑changing: the round clock, who is still alive, and the leaderboard. So instead of one giant lockstepped world, we run **N sharded, independently‑advancing simulations** coordinated by a small **server‑authoritative match director**. Your game advances on *your* inputs plus a server‑issued RNG seed — it can never stall waiting on someone else, a local desync self‑heals from a server snapshot, and reconnection is just "replay my input log." See [`docs/03-network-architecture.md`](docs/03-network-architecture.md).

---

## Code & document index

| Path | What's in it |
| --- | --- |
| [`sim-core/`](sim-core/) | The **deterministic Rust simulation core** + harness + netcode (`crates/{determinism,sim,harness,net}`) and a self-playing **`preview`** crate. Engine-independent, fixed-point, checksum-gated. |
| [`godot/`](godot/) | The **Godot 4 front-end** over the core via a Rust **GDExtension** binding (`rust/`). Built and run locally. |
| [`TUTORIAL.md`](TUTORIAL.md) | **Playtest guide** — how to build & run the game, every screen and control, and how to play well. |
| [`demos/`](demos/) | **Prebuilt binaries** (currently a Windows x86-64 zip) for playing without building from source. |
| [`adapters/steam-transport/`](adapters/steam-transport/) | The **Steamworks transport adapter** (`ISteamNetworkingSockets`/SDR behind the `net` `Transport` trait) — kept outside the `sim-core` workspace so the offline CI gate stays dependency-free. Live bring-up runbook: `docs/07a`. |
| [`CREDITS.md`](CREDITS.md) | Attribution & originality statement (all shipped content is original; no WC3 assets). |
| [`docs/01-source-analysis.md`](docs/01-source-analysis.md) | Analysis of the WC3 **Tower Survivors** map, what the community said about it, and the networking lessons we're carrying forward. Includes a note on the download attempt. |
| [`docs/02-game-design.md`](docs/02-game-design.md) | The game design itself: pillars, the single tank, rounds & waves, weapons/upgrades/synergies/rarities, economy, bosses, and the last‑man‑standing win logic. |
| [`docs/03-network-architecture.md`](docs/03-network-architecture.md) | **The centerpiece.** Topology, authority model, the sharded‑simulation thesis, tick/determinism model, the explicit comparison vs WC3 lockstep, reconnection, host migration, anti‑cheat, and scaling/bandwidth budgets. |
| [`docs/04-protocol-and-messages.md`](docs/04-protocol-and-messages.md) | Wire protocol: transport & channels, the full message catalog, time synchronization, the match state machine, and sequence diagrams. |
| [`docs/05-data-model.md`](docs/05-data-model.md) | Simulation entity model and the content‑data schemas (weapons, modifiers, enemies, wave tables) plus the RNG‑stream design that makes randomization deterministic. |
| [`docs/06-roadmap-risks-testing.md`](docs/06-roadmap-risks-testing.md) | Milestones / vertical slices, the risk register (networking‑weighted), the determinism & netcode test plan, and open questions. |
| [`docs/07-steamworks-integration.md`](docs/07-steamworks-integration.md) | **Steam deployment.** Host‑authoritative‑over‑SDR for a free game, the Steamworks API mapping (sockets, lobbies, auth, stats, cloud), host migration on Steam, and the honest anti‑cheat trade‑offs of free P2P. |
| [`docs/07a-steam-bringup.md`](docs/07a-steam-bringup.md) | **Steam bring-up runbook** — the §7.7 checklist item-by-item: what's implemented in `adapters/steam-transport/`, and exactly what still needs a real Steam environment. |
| [`docs/08-engine-choice.md`](docs/08-engine-choice.md) | Engine decision: decouple the deterministic sim core from the renderer; recommended **Godot 4 + Rust sim core** (with the alternatives and the one factor that flips it). |
| [`docs/09-rebuild-plan.md`](docs/09-rebuild-plan.md) | **Rebuild program.** Audit-driven plan for the presentation overhaul (art, VFX/particles, audio, UX), the sim→render event-stream contract, confirmed defect list, docs truth pass, and the Steam ship track — with phase gates and a delegation map. |
| [`docs/appendix-A-map-extraction.md`](docs/appendix-A-map-extraction.md) | Evidence appendix: the full extracted catalog (118 units, 79 weapons, 87 upgrades in `catalog.json` — 89 counting two HP‑for‑gold trades the parser split off) from `TowerSurvivors v1.58.w3x`. |
| [`research/tower-survivors-map/`](research/tower-survivors-map/) | The finalized map extraction: raw WC3 files, a re‑runnable `extract.py`, and `parsed/catalog.json`. |

## Status

- **Implemented and test-gated:** milestones **M0–M4** are shipped (deterministic sim, shadow-sim/snapshots, match director + transport abstraction, 8-player reconnect + host migration, full content systems and the 86-weapon / 91-modifier catalog), and **M5 is partial** (chaos/soak/replay/clock-sync hardening done). See the per-milestone status annotations in [`docs/06`](docs/06-roadmap-risks-testing.md) — each points at the test that proves it.
- **Open:** everything that needs a *live Steam environment* — the real SDR transport and `ISteamMatchmaking` lobby (the Steam halves of M2/M3), and Steam stats/leaderboards (M4's meta bullet). The adapter code exists (`adapters/steam-transport/`, `docs/07a`); it has never run between two real machines.
- **The docs are the spec, and they are kept honest mechanically:** doc-sync tests (`sim-core/crates/net/tests/wire_doc_sync.rs`, `sim-core/crates/sim/tests/doc_sync.rs`) fail the build if the protocol catalog, match-arc constants, catalog counts, or boss name drift from the source.
- **Assumptions are labeled.** Anything reconstructed from the WC3 map (rather than verified against decompiled triggers) is marked *(inferred)*; where the shipped game deliberately diverges from the source (30-min arc, original enemy roster), the docs say so explicitly.
- **Open questions** are collected at the end of [`docs/06-roadmap-risks-testing.md`](docs/06-roadmap-risks-testing.md).

## Naming note

"Standing Tank Defense" comes from the repository name and the core fantasy (a *standing* — immobile — tank that *defends*). The WC3 source used a literal "tower"; we use a tank purely as the theme. Mechanics are identical in spirit.
