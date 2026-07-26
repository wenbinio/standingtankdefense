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

1. **Offer-pool unlocks.** The shop's draw pool starts small and widens with account XP. New content is *unlocked*, not *rolled*.

   **Correction (found during the R3 transcription):** an earlier draft of this said it "reuses the existing rarity/weighting system." **There is no such system.** `shop.rs` draws *uniformly* over the full pools into a fixed 4-weapon + 4-modifier layout; the rarity ladder, Black Market, copy-effects and draw-altering meta items described in `[02] §2.5`/`§2.8` are **not implemented**. Pool *restriction* is still trivial — draw from a subset — so the unlock mechanic survives intact. But any rarity weighting would have to be **built**, not reused, and that cost belongs in this milestone's estimate rather than being discovered later.
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


## F8 — R2 throughput: measured results

Bench: `roblox/bench/throughput.luau`, standalone `luau -O2`, Xeon @ 2.10 GHz, against the real `Fixed.luau`. Budget is 33.33 ms/tick for all 8 arenas; the honest simulation ceiling is 30% of that = 10.00 ms, because a Roblox server frame is not ours alone.

**The workload is measured, not assumed.** Steady-state population was derived by Little's law over the real `WAVE_M0`/`BOSS_ESCORT` cadences, then checked against the actual Rust sim driven by `bot::Bot`: **peak 66–67 enemies, 70–84 projectiles, 20 weapons**. This corrects the ~300-entity figure assumed in `[09] §1.6` — pessimistic by ~4.5×. E=300 is retained below as a stress column, not as the target.

| Backend | E=68 (real peak) | E=300 (old assumption) | E=675 (extreme) |
| --- | --- | --- | --- |
| `exact` (C2 `Fixed`) | 0.709 ms/arena — **56.7% of budget** | 4.474 ms — 357.9% | 9.658 ms — 772.6% |
| C2 API over doubles (F5 hatch) | 0.106 ms — 8.5% | 0.744 ms — 59.6% | 1.812 ms — 145.0% |
| Raw doubles (floor) | 0.020 ms — 1.6% | 0.087 ms — 7.0% | 0.196 ms — 15.7% |

All three columns are the **serial** figure — 8 arenas on one thread, the pessimistic floor. Across 8 Actors these divide by up to 8, but Roblox sizes the worker pool from the host and does not guarantee 8, so serial is the honest planning number.

### Result: R2 passes at the real workload, with exact fixed-point intact

**The exact backend fits — 56.7% of a deliberately conservative budget, serially, before `--!native`.** F5 stands as written: the seed-for-seed Rust oracle is preserved and the swap-to-doubles hatch stays shut.

That outcome was not free. The first measurement put `exact` at 4.70 ms/arena (376% of budget), and the diagnosis was that the cost was **algorithmic, not inherent**:

| op | before | after (interp) | after (`--codegen`) |
| --- | ---: | ---: | ---: |
| `div` | 17,700 ns | **707 ns** | 433 ns |
| `fromRatio` | 18,700 ns | **683 ns** | 467 ns |
| `mul` | 736 ns | **324 ns** | 167 ns |
| `sqrt` | ~1,900 ns | **200 ns** | 116 ns |

`div` was bit-by-bit restoring division (~80 iterations × 5-limb compare/subtract); replacing it with **Knuth Algorithm D in base 2^16** bought 25×. Enemy movement had been 72% of the tick purely because `step_toward` divides twice per enemy. The exactness tax fell from 33× to ~6.7×.

Allocation was measured rather than assumed: pre-allocated out-params cost **72 ns against 63 ns to allocate fresh** in the standard VM — a wash. They only win under native codegen (5.5 ns vs 41 ns). So the API was not widened with `*Into` variants; that trade is available later if `--!native` is adopted for the arena, and only then.

### Correctness, and why two test suites

- **`roblox/test/parity.luau`** — 7,309 assertions against vectors generated by running the Rust. Anchors Luau to Rust.
- **`roblox/test/differential.luau`** — 1,769,712 comparisons against `FixedRef.luau`, a no-fast-path reference (schoolbook multiply, binary restoring division). Anchors the *optimizations* to an obviously-correct implementation.

Both are needed, and this was verified rather than assumed. Widening the `div` fast-path guard from |a| < 2^36 to |a| < 2^48 is caught by parity with **2 failing assertions out of 7,309**, and by the differential harness with **269 divergences**. Parity's coverage of fast-path boundaries is real but razor-thin — it knows nothing about guards the Rust doesn't have. Deleting the differential suite would leave optimization work effectively untested.

### Caveats — what this does not prove

Only a Studio run settles R2 formally. The Actor harness has never executed against a real Roblox VM; Roblox ships its own Luau build with different FFlags, allocator and sandbox; this ran on build-container hardware; Actor dispatch and barrier costs are unmodelled; and a real server frame also carries replication, physics and every other script. `--!native` is *not* applied here and typically wins another 1.5–3× on loops like these, so the Roblox figure could be materially better.

Also unmodelled: hazards, minions, auras, vulnerability pulses and Fire chain explosions — all zero across the 24-seed Rust profile, but a build stacking them adds `O(H·E)` and `O(M·E)` passes. Population is pinned, where a real arena oscillates as `Clear` wipes the board.

**Replication is not a constraint.** *(Numbers below corrected once `Wire.luau` was built and measured — the original estimate was optimistic by about one payload at the 300-entity figure.)* At the **real** peak (67 enemies / 84 projectiles) a delta frame is **325–423 bytes** — a single ~900-byte `UnreliableRemoteEvent` payload, **5.3 KB/s** per client at 15 Hz and **42.6 KB/s** server-side for eight, against a ~22 KB/s per-client budget. Across a full 56,000-tick run on the real sim the worst frame observed was **981 bytes**, still one payload.

At 300 entities — the figure `[09] §1.6` originally assumed — the average delta is 858 B but the **worst case is 1024 B, i.e. two chunks**, so the earlier "300 entities fit one payload" claim does not hold. It holds comfortably at the workload the sim actually produces, which is what matters. Only a deliberately absurd 300-entities-all-moving case exceeds the bandwidth budget (29.8 KB/s).

### Carried forward

An upstream finding about the **existing Rust sim**, not the port: `combat.rs::fire_weapons` does not advance cooldown when nothing is in range, so short-range weapons rescan every enemy every tick — **1,670 weapon-range scans/tick** at E=300/W=20, turning an amortised `O(W·E/cooldown)` into a per-tick `O(W·E)`. Worth fixing on its own merits, and it would benefit both builds.

## F9 — Findings carried upstream (about the Steam build, not the fork)

The R3 transcription put eight independent readers through `sim-core` line by line, which surfaced divergences no one was looking for. **None of these are port bugs**, and none were "fixed" in transit — every agent transcribed the Rust as it stands and flagged the gap. Recording them here so they are decided deliberately rather than rediscovered.

### `docs/02` is materially stale against the implementation

Five confirmed divergences, in rough order of consequence:

1. **Match length.** `[02] §2.3` describes a ~15-minute arc; `content.rs` implements **30 minutes** (`BOSS_SPAWN_TICK = 54000`).
2. **The entire shop meta-layer is unimplemented.** `[02] §2.5`/`§2.8` specify a rarity ladder driving draw weight, Black Market, Multiplication Gems, "copy of the next Rare", and a *stateful, order-dependent* offer stream. `shop.rs` draws **uniformly over the full pools** into a fixed 4+4 layout. `pending_perk` alters the next matching **purchase** (free / +N copies), not future **draws** — so the offer stream is advanced only by `generateOffers`, and there is nothing to save-scum because there is nothing stateful to scum.
3. **Income scoping is inverted.** `[02] §2.4` says income-% multipliers apply "only to *bonus* income, not the base". `economy.rs` applies `income_mult` to the whole of `income_per_tick`, base included.
4. **No dodge diminishing-returns curve.** `[02] §2.2` implies one; `defense.rs` rolls a flat `dodge_num/dodge_den`, and the only non-linearity is a hard **70% cap** applied in `modifiers.rs` at purchase time.
5. **Frost and Poison have no attack-speed/movement slow beyond movement.** `[02] §2.5` gives Frost a "move/**attack**-speed slow" and Poison a slow; `status.rs` exposes only `move_speed_mult`, consumed by `combat::move_enemies`. Enemy attack cadence never consults either.

Each is a **doc-vs-code** question, not a transcription question. The code is the shipped behavior; the doc is the older intent. Someone should decide, per item, which one is wrong — and `[02]` should carry the result. Until then, `[02]` should not be read as a description of the current game.

### `lib.rs::checksum` has blind spots

The checksum omits several fields that are **checksum-relevant sim state**, not render-only: `tank.shield_active_dr`, `tank.heal_on_damaged`, `tank.mana_on_kill`, `modifiers.dmg_per_maxhp_rate`, `modifiers.dmg_per_bounty_rate`, `modifiers.shield_active_dmg`, and `Projectile.last_target_pos`.

This matters twice over: a genuine desync in any of them goes undetected on Steam, **and** an R3 transcription error in any of them passes the Roblox gate silently. Widening the checksum would change every existing digest, so it is a deliberate call — but it should be a call, not an oversight.

### A real inefficiency in `combat.rs`

`fire_weapons` does not advance a weapon's cooldown when nothing is in range, so short-range weapons re-scan every enemy every tick — **1,670 weapon-range scans/tick** at E=300/W=20, turning an amortised `O(W·E/cooldown)` into a per-tick `O(W·E)`. Load-bearing behavior, so the port transcribes it as-is; fixing it would benefit both builds and must be done on both sides at once, or the oracle breaks.

### Wave gates are not timeline-scaled (fixed here)

`waves[].start_tick` gates ship pre-baked at Steam scale (up to 45000). Under the F1 Roblox timeline (boss at 9000) **7 of 17 gates would never open**, silently collapsing the roster to its opening mix with no error anywhere. Fixed by emitting `tick_scale_num`/`tick_scale_den` per timeline and having `Waves.luau` rescale the gates — identity on Steam, so the oracle is untouched. Every shipped gate divides evenly by 6, so the rescale is exact.
