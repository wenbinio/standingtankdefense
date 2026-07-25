# 10 — Roblox Fork: Design Decisions

The Roblox build is a **fork, not a port** (`[09] Part 3, Option C`). It shares the content catalog, the stacking engine, and the sharded-arena architecture with the Steam build; it does **not** share the run structure, the competitive layer, or the progression model, because those are what `[09] Part 2` found mismatched with how Roblox retains and monetizes.

This document locks the fork's design decisions. Everything else inherits from `[02]`/`[03]`/`[05]` unchanged.

## F0 — What is shared, what forks

| Layer | Shared with Steam? |
| --- | --- |
| Content catalog (86 weapons, 91 modifiers, 12 enemies, wave tables, damage matrix) | **Shared**, generated from one source (`[09] §1.5`) |
| Combat / modifier stacking / status rules | **Shared semantics**, reimplemented in Luau |
| Arena independence, seeded RNG streams | **Shared** |
| Run length & difficulty timeline | **Forked** (F1) |
| Elimination & competitive layer | **Forked** (F2) |
| Meta-progression & monetization | **Forked** (F3) — does not exist on Steam v1 |
| Co-play | **Forked** (F4) — new |
| Authority model & netcode | **Forked** (F6) — Roblox server-authoritative |

## F1 — Run length: 30 min → 5 min

The implemented sim runs a **30-minute** arc (`content.rs: BOSS_SPAWN_TICK = 54000` @ 30 Hz), not the 15 minutes described in `[02] §2.3` — the code is ahead of that doc. Either way it is far too long for Roblox, where the 2026 discovery algorithm penalizes long committed sessions in favor of repeat short ones (`[09] §2.3`).

**Decision:** the Roblox fork targets a **5-minute run** with the boss at the end.

- Boss spawn: tick **9000** (5 min).
- Round length: **20 s** (600 ticks), down from 30 s → **15 shop decisions per run**.
- Ramp interval: **30 s** (900 ticks) — the Steam interval under the same `/6` rescale.
- The escalation *curve shape* is reused, compressed onto the shorter timeline — this is a **timeline rescale, not a rebalance**. The existing tuning work is preserved.

**Why the ramp interval is not the round length.** An earlier draft of F1 tied the two together. That would have been a rebalance in disguise: with a 600-tick interval the ramp compounds **15** times before the boss instead of Steam's **10**, moving the difficulty endpoint from ≈×5.56 to ≈×12.9 at the same per-interval factor. Keeping the interval at 900 preserves the compounding count, and therefore the endpoint, with **zero retuning** — which is what makes the "rescale, not rebalance" claim actually true. Shop cadence (20 s) and difficulty cadence (30 s) are simply independent; there is no reason they must agree.

This invariant is machine-checked: `roblox_export.rs` asserts `ramp_intervals_to_boss` is equal on both timelines, so if anyone reties them the test fails and names the consequence.

All timeline constants live in one `timeline` table in `content.json` so the Steam and Roblox schedules sit side by side and divergence stays visible.

## F2 — Elimination → instant re-entry (decouple "run" from "match")

`[09] §2.3` identified elimination as a structural problem: half the lobby is dead early with nothing to do, and on Roblox they leave rather than spectate.

**Decision:** stop making a run and a match the same object.

- **Your run is solo and instantly restartable.** Death → results screen → retry in ~3 seconds. Dead time is approximately zero. The sharded architecture already gives this for free: your arena never depended on anyone else's.
- **The competitive layer becomes a rolling 5-minute server leaderboard** over run scores in the window, rather than a single elimination bracket.
- **Last Stand survives** as an award: granted when you are the only player in the current cohort still alive on your run. It keeps the prestige objective from `[02] §2.7` without the dead-time cost.

This requires **no simulation change at all** — it is purely a change to the director and the shell. That is the payoff of arena independence.

## F3 — Meta-progression and monetization

This is the layer `[02] §2.9` defers to v2. On Roblox it is v1, because without it there is nothing to sell and no reason to return (`[09] §2.2`).

**Decision:** account-persistent progression, built out of machinery that already exists:

1. **Offer-pool unlocks.** The shop's draw pool starts small and widens with account XP. Reuses the existing rarity/weighting system — new content is *unlocked*, not *rolled*.
2. **Tank chassis.** A handful of starting loadouts (starting weapon + one passive). Light mechanical variety, strong identity hook.
3. **Cosmetics.** Skins, projectile trails, arena themes. The primary monetization surface.

**Explicit trade-off, stated rather than buried:** this is a **no-gacha, no-paid-power** model. `[09] §2.2` found that the Roblox TD games earning $2–5M/month monetize through unit rolls and luck boosts, and this design deliberately declines that. The consequence is a materially lower revenue ceiling in exchange for not shipping a predatory loop into a majority-minor audience. If the revenue ceiling is the priority rather than the design, that is a decision to revisit **explicitly** — it is not a detail to be reversed silently downstream.

## F4 — Co-play: rival arena spectating

"7-Day Intentional Co-Play Days per User" is now a significant ranking signal, and the game is 8 players in parallel isolation (`[09] §2.3`).

**Decision for v1: live rival-arena spectating.** Watch a friend's arena in a side panel or fullscreen while your own run continues.

This is nearly free: the client already receives one arena's view, so a second view is *the same replication path pointed at a different arena*. **It does not touch arena independence** — no simulation coupling, no shared state, no new determinism surface.

**Deferred to v2: cross-arena interaction** ("send a creep to a rival", `[02] §2.9` / `[03] §9`). It is the stronger social hook but it is the one change that genuinely breaks the arenas-are-independent property, and it must be scoped against the throughput budget in `[09] §1.3` before it is considered.

## F5 — Numeric contract

Luau has no 64-bit integers (`[09] §1.4`); the Rust sim is `i64` Q48.16 fixed-point with `i128` intermediates and saturating narrowing.

**Decision:** implement `Fixed.luau` as an **exact emulation** of the Rust semantics — split-limb multiply so `mul`/`div`/`scale` are bit-identical to the Rust, including saturation. Cost is real (several double-ops per multiply) and is accepted deliberately:

- It preserves the **seed-for-seed correctness oracle** (`[09] §1.5`), which is the difference between "transcribe 9.5k lines and hope" and "transcribe and diff".
- The API is the seam. If the throughput spike shows fixed-point math is the bottleneck, the module's *internals* can be swapped to plain doubles without touching a single caller.

Get it correct first, behind an API that lets it get fast later.

## F6 — Authority: server-authoritative, one Actor per arena

`[03]` has each client simulate its own arena with a server shadow-sim validating. On Roblox that inverts (`[09] §1.2`): client exploit tooling is endemic, so **all arenas simulate on the Roblox server**, and clients render only.

- **One parallel Luau `Actor` per arena.** Arena independence is exactly the isolation property `Actor`s require.
- **Deleted from `[03]`:** shadow-sim, input validation layer, desync reconciliation, host migration, clock sync, SDR transport.
- **Kept from `[03]`:** the director's job — seed issue, round clock, alive/dead truth, leaderboard.
- Clients receive only their own arena's view (plus a spectated arena under F4), packed into `buffer`s and replicated at 10–15 Hz with client-side interpolation (`[09] §1.6`).

## F7 — Build order

Mirrors the netcode-first discipline of `[06]`, retargeted:

| Milestone | Gate |
| --- | --- |
| **R0** Numeric core | `Fixed.luau` + `Rng.luau` bit-identical to Rust across generated parity vectors |
| **R1** Content pipeline | `content.json` generated from `content.rs`; Luau loads it and round-trips every def |
| **R2** Throughput spike | 8 arenas × worst-case wave × 30 Hz inside one server budget, measured (`[09] §1.3`) — **this gates the rest**. First results in §F8 |
| **R3** Single arena | One arena simulating in Luau, diffed against the Rust oracle on shared seeds |
| **R4** Shell | Shop UI, run loop, instant retry (F2) |
| **R5** Multiplayer | Director, rolling leaderboard, rival spectating (F4) |
| **R6** Meta | Unlocks, chassis, cosmetics (F3) |

R0–R2 are the risk. R3 onward is transcription against an oracle.

## F8 — R2 throughput: first measured results

Bench run standalone (`luau -O2`, Xeon @ 2.10 GHz) against the real `Fixed.luau`, not a stub. Budget is 33.33 ms/tick for all 8 arenas; the honest simulation ceiling is taken as 30% of that = 10.00 ms.

> **The absolute percentages below are provisional.** They were captured while `Fixed.luau` was still being optimized, and the exact backend is GC-bound with ±30% run-to-run variance; a re-run mid-optimization already showed the realistic case moving from 376% to 114% serial. **What is durable here is the diagnosis and the ratios, not the digits.** Treat the table as an order of magnitude and re-run the bench for current numbers.

**The workload is measured, not assumed.** Steady-state population was derived by Little's law over the real `WAVE_M0`/`BOSS_ESCORT` cadences, then checked against the actual Rust sim driven by `bot::Bot`: **peak 66–67 enemies, 70–84 projectiles, 20 weapons**. This corrects the ~300-entity figure assumed in `[09] §1.6` — pessimistic by ~4.5×.

| Backend | Real peak (E≈67) | `[09] §1.6` worst case (E=300) |
| --- | --- | --- |
| `exact` (C2 `Fixed`) | 4.70 ms/arena — 376% serial, **47% across 8 Actors** | 19.58 ms — 1566% serial, 196% parallel (**fail**) |
| C2 API over doubles | 0.108 ms — 8.6% | 0.600 ms — 48% |
| Raw doubles | 0.020 ms — 1.6% | 0.088 ms — 7.1% |

`--codegen` (proxy for `--!native`) buys a consistent ≈2.1×.

### Read

**The simulation is cheap and the architecture is sound. What costs is the exact `Fixed` emulation — and that cost is algorithmic, not inherent.**

- Exactness tax is **33×** (221× vs raw doubles, against 6.8× for the same API over doubles).
- `div` is the culprit at **17.7 µs/op — 14,236× native**, because `divMag` was a bit-by-bit restoring division (~80 iterations × 5-limb compare/subtract). Enemy movement alone is **72% of the tick** purely because `step_toward` does two divisions per enemy.
- The exact backend is also **GC-bound** (a table allocated per op), giving ±30% run-to-run variance.

At the *real* workload the exact backend already fits across 8 Actors (47%), so F5 is not yet in trouble. But the margin is thin and noisy, so three fixes are being applied **before** the F5 "swap to doubles" hatch is considered — all of which preserve the seed-for-seed Rust oracle:

1. Replace `divMag` with Knuth Algorithm D or reciprocal-multiply (projected to move `exact` from ~470% to ~100% serial).
2. Get `div` out of `step_toward`.
3. Kill an `O(W×E)` rescan — **1,670 weapon-range scans/tick** at E=300/W=20, because `combat.rs::fire_weapons` deliberately does not advance cooldown when nothing is in range, so short-range weapons rescan every enemy every tick. *This one is a finding about the existing Rust sim, not the port, and is worth fixing upstream on its own merits.*

### Status: inconclusive as a formal gate, but not blocking

Only a Studio run settles R2 — the Actor harness has never executed against a real Roblox VM, and the standalone numbers come from a different Luau build, allocator and sandbox, on build-container hardware, with no Actor dispatch or barrier costs modelled. **Serial is the honest planning column.** Hazards, minions, auras and Fire-chains are unmodelled (all zero in the Rust profile), and population is pinned where real arenas oscillate with `Clear`.

What the bench *does* settle is the decision R2 was gating: the 33× backend ratio is a property of the code, not the host, and will not invert on Roblox hardware. **R3 is cleared to start** once the division work lands.
