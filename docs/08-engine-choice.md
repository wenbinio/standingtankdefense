# 08 — Engine Choice

Short answer: **the engine is deliberately the *second* decision, not the first.** The load-bearing requirement is a **cross-platform deterministic, fixed-point simulation** ([`05 §5.6`](05-data-model.md)) — the entire netcode rests on it ([`03 §3.3`](03-network-architecture.md)). So the primary architectural choice is to **decouple the simulation from the engine**, then pick a presentation engine that (a) doesn't fight determinism, (b) handles thousands of 2D entities, (c) has good Steamworks support, and (d) is free/royalty-free for a free game.

## 8.1 The decision that matters most: decoupled sim core

```
┌──────────────────────────────────────────────────────────────┐
│  SIMULATION CORE  (engine-independent, deterministic)         │
│  fixed-point math · fixed 30Hz tick · PRNG streams · ECS      │
│  → produces state_checksum; identical on every machine/OS     │
│  → THIS is what the shadow-sim and reconnect replay run        │
└───────────────▲───────────────────────────┬──────────────────┘
                │ inputs / state              │ render snapshot (read-only)
┌───────────────┴───────────────┐  ┌─────────┴──────────────────┐
│  NET LAYER (Steamworks, [07])  │  │  PRESENTATION ENGINE        │
│  director · sockets · channels │  │  sprites · UI · audio · cam │
└────────────────────────────────┘  └────────────────────────────┘
```

The sim core never imports the engine's vector/physics/RNG types (those are non-deterministic across platforms). The engine only **reads** sim state to draw it. This is already baked into [`05`](05-data-model.md) ("integer/fixed-point everywhere in the hot path; floats only in render code") and means **the engine choice cannot break the netcode** — the worst an engine can do is render slightly differently, never desync the simulation.

## 8.2 Recommendation

**Godot 4.x as the presentation/UI/input layer, on top of a standalone deterministic simulation core written in Rust and exposed via GDExtension. Steam via GodotSteam. This is the default unless team skills point elsewhere (§8.4).**

Why this combination for *this* project:

| Requirement | How the recommendation meets it |
| --- | --- |
| **Determinism** | The sim is a separate Rust crate with fixed-point math and seeded PRNG streams — zero dependence on engine floats/physics. Rust gives bit-stable integer math and easy cross-platform CI. |
| **Thousands of entities** | A data-oriented ECS in the core (e.g. `hecs`/`bevy_ecs` as a library, *not* as the renderer) handles survivors-scale swarms; Godot just draws via `MultiMesh`/2D batching. |
| **Steamworks (free P2P)** | **GodotSteam** (GDExtension) wraps `ISteamNetworkingSockets`, lobbies, auth, stats, cloud — everything [`07`](07-steamworks-integration.md) needs. |
| **Free game / licensing** | Godot is MIT — **no royalties, no runtime fee, no licensing risk**, which fits shipping a *free* title and an open, moddable content pipeline ([`05 §5.7`](05-data-model.md)). |
| **2D-first** | Survivors-like is 2D top-down; Godot's 2D renderer/UI is strong and lightweight. |
| **Reconnect/replay reuse** | Because the core is a plain library, the *same* binary runs headless as the shadow-sim, the dedicated-server director, and the leaderboard re-sim ([`07 §7.6`](07-steamworks-integration.md)) — no engine needed for those. |

## 8.3 Alternatives considered

| Option | Strengths | Why not the default |
| --- | --- | --- |
| **Bevy (Rust, all-in-one ECS+render)** | Best determinism & performance story; one language; `steamworks-rs`; the sim core *is* native Bevy ECS | Younger UI/tooling/asset pipeline; more to build for menus/shop UX. **Strong pick if the team is Rust-first** — keep the core as above and just use Bevy to render. |
| **Unity + custom fixed-point sim** | Richest tooling/asset store; DOTS/Burst for huge entity counts; mature Steamworks (Steamworks.NET / Facepunch) | Closed-source + licensing history is a wrinkle for a free title; C# GC and Unity physics are non-deterministic, so you *still* must write a separate fixed-point sim (same decoupling, less of the "free/open" benefit). **Pick if the team is a Unity shop.** |
| **Pure C++ (raylib/SDL + EnTT)** | Maximum control; Steam SDK is native C++; trivially deterministic | Most engineering for UI/tooling/content pipeline; slowest to iterate. Best only if the team is deeply C++-native. |
| **Engine with authoritative built-in netcode (Mirror/Fish-Net/Photon)** | Fast to get "networked movement" | **Anti-fit**: these assume entity-replication/shared-world models — exactly the bandwidth-heavy approach [`03 §3.11`](03-network-architecture.md) rejects. Our model ships *inputs + seeds*, not entities, so built-in replication buys little and fights the design. |

## 8.4 The one factor that flips it

The decoupled-core architecture (§8.1) is **fixed regardless**. Only the rendering engine choice depends on **the team's strongest language/ecosystem**:

- **Rust-first team →** Bevy (render in Bevy; core is just ECS).
- **Generalist / 2D-focused / values free+open →** **Godot 4 + Rust core (the default).**
- **Existing Unity shop →** Unity + custom fixed-point core.
- **Deeply C++-native →** raylib/SDL + EnTT.

This is the open question to lock (tracked alongside [`06 §6.5`](06-roadmap-risks-testing.md)); the spec proceeds on the Godot 4 + Rust-core default until told otherwise. None of [`02`](02-game-design.md)–[`07`](07-steamworks-integration.md) changes with the engine pick — only this doc does.
