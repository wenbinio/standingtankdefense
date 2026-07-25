# 09 — Roblox Port & Market Assessment

Assessment of (a) what it would actually cost to port **Standing Tank Defense** to Roblox, and (b) whether there is a market for it there. Researched July 2026 against the current codebase (`sim-core/` ~15.7k lines Rust, `godot/` front-end, Steam transport adapter) and public Roblox platform data.

> **Verdict.** The port is *technically very feasible* — Roblox's server model happens to fit the sharded-arena architecture better than the Steam P2P deployment does, and roughly two-thirds of the netcode work simply disappears. But **there is no code reuse: it is a 100% rewrite in Luau.** And the harder problem is not technical. The game **as specified in `[02]`** — run-based, no meta-progression, 8-player elimination — is a **poor fit for how Roblox actually monetizes and retains**. Tower defense is one of Roblox's biggest genres, but the money in it is *gacha unit collection wearing a TD skin*, which is precisely the layer `[02] §2.9` defers out of v1. Porting without forking the design would put a well-built game into a market that rewards a different one.

---

## Part 1 — The port

### 1.1 The one constraint that decides everything

**Roblox runs Luau and only Luau.** There is no native-extension path: no GDExtension analogue, no C ABI, no dynamic libraries, and no WebAssembly runtime. WASM support is an open *feature request* on the developer forum with no shipped implementation and no staff commitment; the only workaround developers cite is source-to-source transpilation, which is not a viable path for a 9.5k-line fixed-point simulation crate.

Consequences:

- The Rust `sim` crate **cannot be bound, embedded, or linked**. It must be **transcribed into Luau**.
- The GDExtension seam (`godot/rust/src/lib.rs`, 601 lines) has no counterpart. There is nothing to bind *to*.
- `adapters/steam-transport` and everything Steamworks (`[07]`, `[07a]`) is dead on Roblox.

So: **zero lines port.** What ports is the *design*, the *content data*, the *balance numbers*, and — importantly — the existing Rust build as a **correctness oracle** (see §1.5).

### 1.2 Component-by-component disposition

| Component | Lines | On Roblox |
| --- | ---: | --- |
| `crates/sim` (combat, content, state, modifiers, waves, economy, status, defense, shop, bot) | 9,527 | **Rewrite in Luau.** This is the actual game and the bulk of the work. |
| `crates/determinism` (fixed-point, PRNG streams, checksum) | 332 | **Rewrite, and partly redesign** — see §1.4. Requirement weakens (server-authoritative), so it may shrink. |
| `crates/net` (director, lobby, hub, client, wire, replay, results, clock sync) | 4,876 | **~70% deleted.** Roblox provides the authoritative server, hosting, matchmaking, ordered reliable messaging, and rejoin. What survives is a few hundred lines of Luau: slot assignment, seed issue, alive/dead truth, placement, leaderboard. |
| `adapters/steam-transport` | 524 | **Deleted.** |
| `godot/rust/src/lib.rs` (GDExtension binding) | 601 | **Deleted** — no equivalent seam exists. |
| `godot/*.gd` (front-end: lobby, match, shop UI, FX, audio) | 2,715 | **Rewrite in Luau**, but the *structure* and UI flow port well; this is the most mechanical part. |
| `crates/harness`, `crates/preview` | 970 | Dev-only. Keep the Rust ones and use them against the Luau port (§1.5). |
| `docs/` (1,465 lines), `research/tower-survivors-map/`, extracted catalog | — | **Ports 1:1.** Full value retained. |

**The architecture ports better than the deployment does.** `[03]`'s core thesis — N independent arenas coordinated by a small authoritative director — is a *better* fit on Roblox than on Steam. Roblox gives you a real dedicated server process per instance, for free, with no host-migration problem (`[03] §host migration` and the Steam SDR reasoning in `[07]` become moot). What was a clever workaround for "no server budget" becomes simply the platform default.

But the authority split inverts. `[03]` has each **client** simulate its own arena with a server shadow-sim validating. On Roblox that is the wrong call: client-side exploit tooling is endemic and trivially available, and a client-authoritative sim would be cheated within days. The Roblox version should run **all 8 arenas on the server** and replicate each player only their own arena's view. That is a strictly simpler authority model — and it deletes the shadow-sim, the input validation layer, and the desync-reconciliation path — at the cost of putting the entire simulation load on one Roblox server.

### 1.3 Hard problem #1 — simulation throughput in Luau

8 arenas × hundreds of enemies × 30 Hz, all on one server, in an interpreted language. This is the single biggest technical risk and it deserves a spike before anything else.

Mitigations that are genuinely available:

- **Parallel Luau `Actor`s map onto arenas perfectly.** The arenas-are-independent insight (`[03] §3.1`) is exactly the isolation property `Actor`s require — one `Actor` per arena, no shared mutable state, no synchronization beyond the round clock. This is an unusually clean fit; most Roblox games struggle to find work that parallelizes this cleanly.
- **No `Instance`s in the sim.** Enemies must be plain Luau tables / `buffer`s, never Parts, never `Humanoid`s, never pathfinding. Roblox's cost model punishes instance count, not arithmetic.
- **`--!native` codegen** on the hot loops.

Community stress tests (e.g. a published 2,000-NPC RTS/TD test replicating at 20 Hz for ~100 KB/s) suggest the target is reachable, but those are single-arena. Budget the spike as: one arena, worst-case wave from `crates/sim/src/waves.rs`, measure; then 8 in parallel.

### 1.4 Hard problem #2 — fixed-point math has no home in Luau

`crates/determinism` is built on `i64` fixed-point (`[05] §5.6`). **Luau has no 64-bit integers** — all numbers are IEEE doubles with a 53-bit integer-safe mantissa. The multiplicative-stacking engine (`[02] §2.2`, "everything stacks multiplicatively across sources") is exactly the kind of computation that eats headroom, and `crates/sim/src/combat.rs` is 1,930 lines of it.

Two ways out:

1. **Narrow the fixed-point format** to fit 53 bits safely (e.g. 32.16 with audited intermediate ranges), and re-verify every multiply chain in `combat.rs`/`modifiers.rs` for overflow. Real work, but it preserves bit-exact determinism and lets the Rust build stay a checksum oracle.
2. **Drop bit-exact determinism.** Server-authoritative simulation removes the *reason* determinism was sacred: nobody is re-simulating the same arena on two machines, so there is no desync to detect and no reconnect-by-replay to support (Roblox rejoin can just resume from server state). Seeded RNG is still needed for fairness and anti-save-scum, but `state_checksum` and float-free hot paths are no longer load-bearing.

Option 2 is cheaper and honest about the platform, but it **forfeits the cross-implementation test harness in §1.5** and permanently forks the two versions' behavior. Recommendation: take option 1 through the port, then decide.

### 1.5 The unfair advantage: the Rust build is a free test oracle

This is the strongest argument for the port being cheaper than it looks. Because the existing sim is deterministic and seeded, you can:

1. Run the Rust `preview`/`harness` on seed *S* and dump per-tick state.
2. Run the Luau port on seed *S*.
3. Diff.

That turns "did I transcribe 9.5k lines of combat and modifier rules correctly?" from a months-long QA problem into a mechanical, automatable one — provided §1.4 option 1 is taken. Nobody porting a game normally gets this. It is worth the fixed-point work to keep it.

Likewise, `crates/sim/src/content.rs` (1,597 lines of hardcoded content) should be made to **emit JSON**, and the Luau side should consume that generated file rather than hand-copying the catalog (**86 weapons, 91 modifiers, 12 enemies** as implemented — distinct from the 79/87 figures in `[appendix-A]`, which count the raw WC3 extraction, not the shipped tables). That keeps balance single-source across both versions and is a small, high-leverage change to the *existing* codebase.

### 1.6 Hard problem #3 — replication and rendering

Each client only ever sees **its own arena**, so replication is 1 arena per client, not 8 — a large win the architecture hands you for free.

**Corrected after measurement:** this section originally assumed ~300 entities per arena. Profiling the real Rust sim under the bot puts the actual peak at **66–67 enemies and 70–84 projectiles** — pessimistic by roughly 4.5×. The replication budget is consequently *not* a problem: 300 entities at a 3-byte delta encoding fits in a single ~900-byte payload, and the real ~67 fits with room to spare (~22 KB/s per client at 15 Hz, ~176 KB/s server-side for eight). Packing cost is ~0.16% of one core for all eight arenas. The paragraph below is kept because the encoding constraints it describes still bind.

`UnreliableRemoteEvent` caps at ~900 bytes per payload (dropped above 1000), and Luau numbers serialize at ~9 bytes each. So: pack positions into `buffer`s (which compress on the wire), replicate at 10–15 Hz, and interpolate client-side. If §1.4 option 1 is taken, the client can instead *predict* enemy motion from the seed and receive only periodic corrections — the determinism work paying for itself a second time.

Rendering hundreds of sprites is its own problem: no Parts per enemy. Either a 2D `ScreenGui` presentation (closest to the current Godot front-end, and the `RenderView` contract in `crates/sim/src/view.rs` already exposes exactly the flat integer arrays this needs) or instanced billboards.

### 1.7 Effort

For one experienced Luau developer, working from a complete spec and a working reference implementation:

- **Spike** (one arena, worst-case wave, 8 parallel Actors, measure): **1–2 weeks**. Do this before committing.
- **Playable vertical slice** (one arena, real content, shop, waves, boss): **6–10 weeks**.
- **8-player match, matchmaking, placement, leaderboard**: **+3–4 weeks** (small, because Roblox provides most of it).
- **Ship-quality UI, art, audio, polish**: **+6–10 weeks**, and this is where a Roblox launch actually lives or dies.

Call it **4–6 months to a launchable experience.** The spec and the reference implementation are worth a lot here — you are transcribing known-correct logic, not designing.

### 1.8 The ongoing tax

Two implementations of a 9.5k-line simulation in two languages will diverge. Generating content data from a single source (§1.5) contains part of it; behavior drift in `combat.rs` will not be contained. Budget for the Roblox version becoming a **separate product** within a year, not a synchronized port.

---

## Part 2 — The market

### 2.1 The demand looks excellent, at first glance

- Roblox reached **~144M DAU** (Q4 2025) and ~382M MAU in early 2026.
- **Tower defense is a top-tier Roblox genre.** *Anime Defenders* runs 50K–200K CCU, 3.4B+ total visits, and an estimated **$2–5M/month**, ranking ~9th among highest-earning Roblox experiences. *Toilet Tower Defense* has passed **4B visits** with a ~152K peak CCU.
- The **18+ cohort is the fastest-growing** segment (up 50%+ YoY) and **monetizes 50%+ better** than under-18s — which is the cohort most likely to want build-craft depth.
- **No survivors-like has broken out on Roblox.** Searching the space returns Steam/itch/browser titles, not Roblox ones. That's a genuine gap.

### 2.2 What that demand is actually buying

Here is the problem. The top Roblox "tower defense" games are **not** strategy games. *Anime Defenders*, *Anime Vanguards*, *All Star Tower Defense*, *Toilet Tower Defense* are **gacha collection games** using a TD loop as the container. Players pay for **unit rolls, luck boosts, and trait rerolls**; they return daily because their *account* gets permanently stronger; they trade units with friends. The TD map is where you go to watch the units you own perform.

Standing Tank Defense is the opposite by design: **all power is earned inside a 15-minute run and discarded at the end.** `[02] §2.9` explicitly defers the challenge/score/skins meta-progression layer to v2. On Steam that is a virtue — it's what makes it a game rather than a treadmill. On Roblox, **that deferred layer is the product**. Without it there is nothing to sell and no reason to open the app tomorrow.

### 2.3 Three structural mismatches with the 2026 Roblox algorithm

Roblox's 2026 discovery changes reward specific things, and the design pushes against all three:

1. **Return rate over session length.** The algorithm moved to a 28-day retention window and explicitly measures D1/D7 return. Reporting notes that *"games designed for deep, long-form play are being penalized"* — a completed long session reads as lower intent than a player coming back for a second short one. A 15-minute committed match with a boss finale is the penalized shape.
2. **"7-Day Intentional Co-Play Days per User"** — how often players join *with friends* — is now a significant ranking signal. Standing Tank Defense is 8 players in **parallel isolation** who never interact (`[02] §2.1` pillar 4, and the same property that makes the netcode cheap). It is technically multiplayer and experientially solo. That is the worst possible position for a co-play metric.
3. **Elimination.** Half the lobby is eliminated in the first half of the match and has nothing to do. On Roblox, eliminated players don't spectate — they leave, which the algorithm reads as a bad engagement signal. Roblox's successful elimination games (*Dress to Impress*, round-based obbies) run 2–4 minute rounds with immediate re-entry, not 15-minute ones.

### 2.4 The economics

Developers net **70% in Robux** on in-experience purchases, then convert at DevEx (**$0.0038/Robux** standard; $0.0054 for 18+ US spend as of June 2026). After the app-store cut, the effective take is roughly **24–29¢ per consumer dollar**, or ~25–35% including Creator Rewards.

Applied honestly: a well-made, non-gacha, no-collection, elimination-based TD is not going to sit anywhere near the $2–5M/month cohort. Those numbers are produced by monetization mechanics this game deliberately does not have. A realistic outcome for a competent Roblox launch without a collection layer and without paid UA is **low four figures per month, if it finds an audience at all** — and Roblox discovery is winner-take-most.

### 2.5 The honest read on the genre gap

The absence of a survivors-like hit on Roblox is ambiguous evidence. It might be an unexploited niche. More likely it reflects that an **auto-battler with no movement and no aiming** — where `[02] §2.2` states the only real-time input is one `Clear` button — competes poorly for attention against Roblox's action-forward catalog among the under-18 majority. The build-decision depth that makes this game good lands with the 18+ cohort, which is growing fast but is still ~27% of age-verified users.

---

## Part 3 — Options

**Option A — Don't port. Ship Steam.** The game as designed is a Steam game: paid-attention audience, no gacha expectation, run-based roguelite depth is the native idiom, and the netcode already exists and works. *Recommended if the goal is to ship the game in `[02]`.*

**Option B — Port as a funnel.** Cheap Roblox version to build audience for Steam. **Weak.** Roblox players largely do not convert to Steam purchases, cross-promotion out of Roblox is restricted, and a low-effort Roblox entry gets buried by discovery. This spends 4 months for a marketing channel that probably doesn't fire.

**Option C — Fork the design for Roblox.** Keep the genuinely portable, genuinely good parts — the arena simulation, the multiplicative stacking engine, the 86-weapon/91-modifier content catalog, the shop/reroll loop — and rebuild the shell around Roblox's actual reward structure:

- **Add persistent collection and meta-progression.** Unlockable weapons, account-level modifiers, cosmetic tank skins (the v2 meta layer in `[02] §2.9`, promoted to v1). This is what you sell and what brings players back.
- **Shorten and de-eliminate.** 4–6 minute runs; eliminated players re-enter the next round immediately instead of watching.
- **Make co-play real.** Even lightweight cross-arena interaction — visible rival arenas, shared objectives, the "send a creep to a rival" mechanic explicitly excluded in `[02] §2.9` / `[03] §9` — directly serves the co-play ranking signal. Note this is the one change that breaks the arenas-are-independent property, so it must be scoped carefully against §1.3's throughput budget.

Option C is the highest-EV Roblox path, and it is honest about what it is: **a different product that shares a content catalog and a simulation design with the Steam game.** Not a port.

### Recommendation

Ship Steam first (Option A). If Roblox is wanted afterward, take **Option C**, and gate it on the §1.7 throughput spike — 1–2 weeks of work that de-risks the entire 4–6 month estimate. Regardless of the decision, do the `content.rs` → JSON extraction from §1.5 now: it is small, it improves the current codebase on its own merits, and it is the one thing that makes any future port materially cheaper.

---

## Sources

Platform scale and demographics: [Roblox statistics 2026 (STG Research)](https://www.shanethegamer.com/research/roblox-statistics/), [Backlinko — Roblox users](https://backlinko.com/roblox-users). Genre revenue and CCU: [RoWatcher — 10 highest-earning Roblox games 2026](https://rowatcher.com/news/the-10-highest-earning-roblox-games-in-2026-and-what-they-mean-for-the-platform), [BloxQuiz — tower defense live rankings](https://www.bloxquiz.gg/stats/category/tower-defense), [Anime Defenders guide](https://bloxguidesgg.com/games/anime-defenders). Discovery algorithm: [Roblox newsroom — Optimizing Discovery](https://about.roblox.com/newsroom/2026/06/optimizing-discovery-great-games-reach-millions-players-roblox), [RoWatcher — what the algorithm rewards in 2026](https://rowatcher.com/news/what-the-roblox-algorithm-actually-rewards-in-2026-not-ccu), [Roblox DevForum — improved Recommended For You](https://devforum.roblox.com/t/boost-your-discovery-with-the-improved-recommended-for-you-algorithm-and-analytics-for-creators/3587441). Economics: [Roblox DevEx help page](https://en.help.roblox.com/hc/en-us/articles/13061189551124-Developer-Exchange-Help-and-Information-Page), [RoHire — 2026 DevEx cheat sheet](https://rohire.dev/blog/2026-devex-cheat-sheet), [RoLearn — developer revenue share](https://rolearn.dev/insights/roblox-developer-revenue-share-2026/). Technical: [Parallel Luau](https://create.roblox.com/docs/scripting/multithreading), [Luau native code generation](https://create.roblox.com/docs/luau/native-code-gen), [WASM runtime feature request (unshipped)](https://devforum.roblox.com/t/next-generation-language-compatibility-add-an-optional-native-webassembly-wasm-runtime/4729292), [UnreliableRemoteEvent size limits](https://devforum.roblox.com/t/what-are-real-limits-of-data-being-sent-when-using-unreliableremoteevent/3038324), [2000-NPC replication stress test](https://devforum.roblox.com/t/2000-npc-stress-test-%E2%80%9320hz-replication-at-100-kb-s-wip/4736967), [Players.MaxPlayers](https://create.roblox.com/docs/reference/engine/classes/Players#MaxPlayers).
