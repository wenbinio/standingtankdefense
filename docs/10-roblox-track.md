# 10 — Roblox Platform Track (additional platform, plan for review)

**Status: plan only — no code exists and none starts until the owner signs off on this document.** The owner has approved Roblox as an **additional** platform alongside the locked Steam/Godot track ([`07`](07-steamworks-integration.md), [`08`](08-engine-choice.md)); this doc is the reviewable plan. Facts about the Roblox platform below were researched 2026-07; anything we could not confirm to primary-source confidence is tagged **VERIFY:** and must be re-checked before the milestone that depends on it. Everything that is our own decision is marked **[design choice]** in the house style.

## 10.1 Executive summary & relationship to the Steam track

**What this is.** A port of Standing Tank Defense to Roblox as a *second shipping surface*: the same game design ([`02`](02-game-design.md)), the same content catalog (`sim-core/crates/sim/src/content.rs` — 96 weapons / 110 modifiers / 12-enemy roster + The Hippocrate), the same cosmetic-only meta (`godot/profile.gd` — 13 skins, 25 achievement-class unlocks counting skins+achievements, 8 challenges), re-implemented on Roblox's stack (Luau server + Roblox clients). The motivation is audience and economics: Roblox has the largest casual/young multiplayer audience in the world, tower defense is one of its proven top genres (§10.5.4), distribution and server hosting are free, and the game's design — menu-driven decisions, one real-time button, independent arenas — is unusually portable because it never depended on twitch input.

**What this is not.** Not a replacement for, or a fork of authority over, the Steam track. The locked decisions in `CLAUDE.md` (Godot 4 + Rust core, sharded-sim netcode, SDR deployment) stand untouched — this track adds a platform, it revisits nothing.

**Single source of truth.** The spec (`docs/01–09`) and the compiled content catalog remain the *only* authority for design and content. The Roblox codebase is a **consumer**, never an author:

- **Design changes flow spec-first.** A rule change lands in `docs/02` (and `content.rs` where it is data) before either platform implements it. The Roblox track never grows a mechanic the spec doesn't have; if the Roblox context genuinely demands a design change (e.g. session length pressure, §10.9), that goes to the owner as a spec proposal first.
- **Content flows through one exporter** (§10.2.2): `content.rs` → generated Luau module. Hand-editing the generated module is forbidden; the generated file carries the catalog's `content_hash` so drift is mechanically detectable, in the same spirit as the doc-sync tests that keep these docs honest.
- **Balance may fork deliberately, per platform [design choice].** Audience, session norms, and input devices differ; if the Roblox tune needs its own `RAMP_BASE`-class dials, that is a *labeled, owner-approved* per-platform overlay on top of the exported catalog (a small "Roblox tuning overrides" table with every divergence listed), never silent edits. Default posture: ship at parity, diverge only with evidence from live data.
- **The E-series additions ([`02 §2.10–2.13`](02-game-design.md)) are spec'd but unimplemented on both platforms**; whether Roblox ships them at parity is an owner question (§10.9).

**Relationship to [`07`](07-steamworks-integration.md).** The Steam posture is "free game, zero server bill, host-authoritative over SDR." Roblox complements it: also zero server bill (Roblox hosts), but with a **true neutral server authority for free** — Roblox servers give us, at no cost, the trusted-host property that on Steam is reserved for the optional dedicated/ranked tier ([`07 §7.6`](07-steamworks-integration.md)). In exchange we accept platform monetization mediation (§10.5) and platform policy risk (§10.8).

## 10.2 What transfers, what doesn't

### 10.2.1 Transfer table

| Asset | Transfers? | How |
| --- | --- | --- |
| Game design ([`02`](02-game-design.md)): arc, economy, shop, statuses, boss, placement rules | **Yes, wholesale** | It's a spec; the Roblox sim implements the same rules |
| Content catalog (`content.rs`: 96 weapons / 110 modifiers / 12 enemies + boss, wave tables, armor matrix, arc constants) | **Yes, mechanically** | Automated exporter (§10.2.2) — one authoritative catalog |
| Balance curves (`enemy_hp_mult`, `RAMP_BASE`, cadences, the 17–23% win-band tune) | **Yes** | Same exporter; per-platform overlay only per §10.1 |
| Art direction & sprites (94+ SVG sources, Ashen Vigil identity, [`09 §9.4-P2`](09-rebuild-plan.md) asset contract) | **Yes, via rasterization** | SVG → PNG pipeline (rsvg-convert/Inkscape, scripted) → Roblox image assets / texture atlases; art *style* carries even if the render tech differs (§10.4) |
| Audio (`gen_sfx.py` WAVs + planned CC0 set, [`09 §9.4-P4`](09-rebuild-plan.md)) | **Yes, with friction** | Upload as Roblox audio assets; per-account upload quotas per 30 days and IP moderation review apply (**VERIFY:** current quota tiers) — our assets are original/CC0 so moderation risk is low, but budget lead time (§10.6) |
| Achievements / skins / challenges (`profile.gd`) | **Yes, remapped** | 12 achievements → Roblox **badges**; 13 tank skins → in-experience unlock shop (DataStore-persisted); 8 challenges → in-experience challenge picker. Same unlock rules, same cosmetic-only contract |
| Determinism discipline ([`05 §5.6`](05-data-model.md)) | **Partially — deliberately relaxed** | See §10.3.4: kept as *server-internal reproducibility*, dropped as a wire protocol |
| Rust sim/net code (`sim-core/`, `adapters/`) | **No** | Roblox runs only Luau server-side; no native code, no GDExtension. The Rust core remains the *reference implementation* the Luau port is checked against (§10.3.5) |
| Godot front-end (`godot/`) | **No** | Rebuilt on Roblox UI/rendering primitives; the [`09 §9.3`](09-rebuild-plan.md) event-stream *contract* (event kinds, snapshot/event split) is reused as a design, not as code |
| Steam integration (`docs/07`, `07a`, steam-transport) | **No** | Platform-specific by definition; Roblox equivalents are in §10.5/§10.6 |
| Wire protocol ([`04`](04-protocol-and-messages.md)) | **No** | Replaced by RemoteEvents under a single-server authority (§10.3); the *validation posture* transfers, the messages don't |

### 10.2.2 The content exporter (one catalog, two consumers) [design choice]

A small Rust bin, proposed at **`tools/luau-export/`** (outside the `sim-core` workspace, like `adapters/steam-transport/`, so the offline CI gate stays dependency-free), depending on the `sim` crate:

- Walks the static tables (`WEAPONS`, `MODIFIERS`, `ENEMIES`, `WAVE_M0`+wave tables, the armor matrix, `SPAWN_RING`, and the named arc/difficulty constants — `BOSS_SPAWN_TICK`, `SCALE_STEP_TICK`, `RAMP_PER_ROUND`, `DIFF_*`, …) and serializes them to a generated, frozen **Luau ModuleScript** (`Content.luau`), plus a small constants module.
- **Numeric conversion happens here, in one place**: `Fixed` (Q47.16) values are re-emitted in the Luau-safe integer format chosen in §10.3.3 (16-frac ints with an exporter-run range audit, or `(num, den)` pairs where the audit demands it). Enum variants (`Attack`, `WeaponAbility`) become tagged tables mirroring the Rust `words()`-style encodings.
- Emits the catalog's **`content_hash`** into the module — when the real content hash lands on the Steam track ([`09 §9.4-P6`](09-rebuild-plan.md)), both platforms print the same hash, and a stale export is mechanically visible.
- CI job: regenerate and `git diff --exit-code` the checked-in generated file, so a `content.rs` change that isn't re-exported fails the build — the same "docs are kept honest mechanically" pattern as `doc_sync.rs`.

English display names/descriptions export too (`descriptions.rs` voice carries); localization on Roblox reuses the existing translation CSV as input to Roblox's localization tables (S-effort, listed in R3).

## 10.3 Architecture mapping (docs/03 → one Roblox server)

This is the meat. [`03`](03-network-architecture.md)'s topology exists to solve a problem Roblox *doesn't have*: on Steam there is no free neutral server, so authority is a host player plus per-client sims plus shadow-validation. **On Roblox, every match gets a free, neutral, Roblox-operated server.** That collapses the architecture:

```
        ┌──────────────────────────────────────────────────────────────┐
        │        ONE ROBLOX SERVER (Luau) — the whole authority        │
        │  match director (clock · seeds · alive/dead · placement)     │
        │  + ALL 8 ARENA SIMS, in-process, 30 Hz fixed-tick            │
        └───▲──────────▲──────────▲──────────▲──────────▲──────────────┘
   input intents │ (RemoteEvents, │ validated, rate-limited)
   render state  │ (own-arena deltas 10 Hz · others' digests 1–2 Hz)
        ┌────────┴─┐  ┌────────┴─┐  ┌────────┴─┐        ┌──────────┐
        │ Client 1 │  │ Client 2 │  │ Client 3 │  ····  │ Client 8 │   ← pure renderers
        └──────────┘  └──────────┘  └──────────┘        └──────────┘
```

The director and the sharded arenas from [`03 §3.4`](03-network-architecture.md) both survive — they just live in the **same Luau process**. The sharding insight ("arenas are independent") stops being a *network* property and becomes a *compute* property: it lets the server tick 8 fully independent arenas with no cross-arena locking, stagger them within a frame, and cap them independently.

### 10.3.1 Server sim: one script, 8 arenas, 30 Hz

- **Fixed 30 Hz tick** driven off `RunService.Heartbeat` (60 Hz) with an integer accumulator — every sim tick is a whole tick, never a wall-clock delta, exactly the [`05 §5.6.1`](05-data-model.md) rule. Host-set game speed (30/45/60/90 tps, the shipped `net::GameSpeed` model) maps to ticks-per-heartbeat scheduling; **Hyper (90 tps) may need to be dropped or gated on measured headroom** on Roblox servers (**decided by the R0 measurement**).
- **Data layout: structure-of-arrays in `buffer` objects, not tables of tables [design choice].** Luau's `buffer` type gives packed typed storage with zero-overhead access under native codegen; per-arena entity pools are preallocated flat buffers (id/pos/hp/status lanes) with free-lists, mirroring the Rust core's stable-`entity_id` iteration. This is the single biggest lever for both CPU and GC (§10.8-L1).
- `--!native` (native codegen) on the sim modules; hot loops written to stay in the native path (no polymorphic tables, no closures-per-entity).
- **Entity cap per arena** is load-bearing here (as it already is on the Steam track — [`06`](06-roadmap-risks-testing.md) R3): the cap bounds both tick cost and replication cost. Proposed starting cap: **500 enemies + 200 projectiles per arena** (the [`09 §9.8`](09-rebuild-plan.md) stress-scene numbers), revisited at R0.

**Honest tick-budget estimate.** The Rust core runs a full arena tick with ~250× real-time headroom ([`09 §9.0`](09-rebuild-plan.md)) — order of ~10–100 µs/arena late-game. Well-written Luau under native codegen with buffers typically lands **~5–20× slower than optimized Rust** for integer/array-heavy loops (**VERIFY:** no authoritative cross-language benchmark; this range is from Luau performance documentation and community benchmarks, and is exactly what R0 exists to measure). That puts a *worst-case late-game 8-arena tick* at roughly **1–10 ms**, inside the 33 ms tick budget — but the server core also pays for replication serialization, physics/engine overhead, and GC. Mitigations if R0 measures hot: stagger arena stepping across heartbeats (arenas are independent; skew ≤ 1 tick is invisible), lower the entity cap, aggregate late-game chaff spawns. **R0's exit criterion is a measured p99, not this estimate** (§10.7).

**Server memory** is not a realistic constraint: 8 arenas × capped pools is a few MB of buffers against a multi-GB per-server allowance (**VERIFY:** current Roblox server memory ceiling; community figure ~6 GB class).

### 10.3.2 Clients: pure renderers + input intents

- The client owns **zero authoritative state**. It renders replicated arena state and sends **intents**: `buy(slot)`, `reroll`, `black_market_pick(id)`, `clear`, ready/lobby actions. This is [`03 §3.9`](03-network-architecture.md)'s director-validation posture verbatim, and on Roblox it is *mandatory*, not just principled: Roblox exploiters can fire arbitrary RemoteEvents with arbitrary payloads, so the server must treat every message as hostile.
- **Validation** (same checks as the shipped director): enough gold, offer actually on the board, reroll count/cost valid, `Clear` off cooldown, shop open (post-boss no-op rule), sender owns the arena they're acting on. Illegal intents are dropped silently (deterministic no-op, matching the sim's existing posture).
- **Rate limiting [design choice]:** per-player token bucket (e.g. burst 10, refill 5/s — generous for a menu game) on the intent RemoteEvent; type-check every payload field before touching it; kick on sustained abuse.
- The client applies **optimistic UI** only for latency-masking (button press feedback), never predicted sim state — at one real-time input (`Clear`) and menu actions, round-trip latency is cosmetically irrelevant.

### 10.3.3 Integer math under Luau doubles [design choice]

Luau numbers are IEEE-754 doubles: integers are **exact up to 2^53**, and integer arithmetic (`+ - * // %`, `math.floor`) on in-range values is bit-exact and portable. The Rust core's Q47.16 `Fixed` cannot port directly (a Q47.16 × Q47.16 product needs >53 bits). Strategy:

1. **Plain integers stay plain integers** — HP, gold, damage, ticks, positions are already integer in the Rust core; totals like `damage_dealt` only ever accumulate (adds are safe far past any realistic value below 2^53).
2. **Multipliers become 16-frac integers applied via a single `mul16(value, m)` helper** — `(value * m) // 65536`, with the Rust core's documented order-of-operations ([`05 §5.3`](05-data-model.md)) applied as *sequential* chained multiplies (floor after each), never a pre-combined product. Safe iff `value * m < 2^53`.
3. **A range audit, enforced twice**: the exporter statically audits every catalog value/multiplier against declared range budgets (e.g. per-hit damage < 2^32, single multiplier factor < 2^20 ⇒ product < 2^52), and dev builds assert the same bounds at runtime in `mul16`. Where the audit finds a genuinely over-range case, that path drops to a two-limb (hi/lo 32-bit) multiply helper — expected to be rare or absent.
4. **Fallback if the audit fights us:** scale fractional precision down to 8 bits (Q·.8) for the offending domains — coarser rates, double the headroom. The exporter owns the conversion, so this is a regeneration, not a hand-port.

No floats anywhere in the sim modules — same banned-ops list as [`05 §5.6.1`](05-data-model.md) (plus: no `math.random` — the port brings its own PCG32/xoshiro-class PRNG over integer ops; no iteration over hash-part of tables in sim state — buffers/arrays only).

### 10.3.4 What happens to determinism (kept vs dropped)

The single-authority model **relaxes** [`03`](03-network-architecture.md)'s determinism *requirements* without abandoning the *discipline*:

**Dropped (no longer needed):**
- Cross-machine client↔shadow agreement: there is no client sim and no shadow sim — the server sim is the only sim.
- The entire checksum/digest/snapshot-correction machinery on the wire: no `state_checksum` comparisons, no digest messages, no snapshot resync, no desync class of bug.
- Host migration: there is no player host. (Its replacement problem — Roblox server death — is in §10.3.6.)

**Kept (still cheap, still valuable) [design choice]:**
- **Server-internal reproducibility**: same seed + same ordered input log ⇒ identical replay, on the same build. All the ingredients survive the port for free (integer-only math §10.3.3, seeded per-purpose PRNG streams per [`05 §5.6`](05-data-model.md), stable iteration order, fixed tick). This buys: replay-based debugging of live incidents, a compact "suspicious result" audit trail (log seeds+inputs per match — a few hundred bytes), regression testing, and the cross-implementation parity harness (§10.3.5).
- **Server-owned RNG**: unchanged — clients never see a seed at all now, which is *stronger* than the Steam track's reveal-when-needed schedule. Save-scumming and offer-precomputation remain impossible.

### 10.3.5 Parity with the Rust reference [design choice]

Two codebases implementing one spec **will** drift (§10.8-L4). Mitigation is mechanical, not aspirational: the Rust `harness`/`preview` gains a **canonical trace dump mode** (per-tick event/kill/death/gold trace for a given seed + scripted input log, over the exported integer domain), and the Luau port replays the same seed+inputs and diffs the trace. Trace parity across an agreed seed set is a standing CI-style gate from R1 onward. Where the Luau port *intentionally* diverges (per-platform balance overlay, §10.1), the overlay is applied to the reference run too, keeping the diff clean.

### 10.3.6 Replication design (what ships to whom)

| Stream | Direction / audience | Cadence | Content & encoding | Rough size |
| --- | --- | --- | --- | --- |
| Own-arena entity deltas | S→ owning client | **10 Hz** (client interpolates to 60) | `buffer`-packed: entity id (u16), pos quantized to i16 per axis, hp-permille (u8) on change, status flags (u8) on change; spawns/deaths as explicit events (the [`09 §9.3`](09-rebuild-plan.md) event-kind list is the menu) | worst case ≈ 500 enemies × ~6 B × 10 Hz ≈ **30 KB/s**; typical mid-game far lower |
| Own-arena discrete events | S→ owning client | on occurrence | kills, impacts (aggregated per tick), gold, shop board, boss events — drives FX/audio exactly like the Godot event stream | ≪ 1 KB/s |
| Other-arena digests | S→ all | **1–2 Hz** | per player: hp, gold, kills, round, alive/placement — the 8-cell net view | ~100–300 B/s |
| Spectate feed (dead player watching a rival) | S→ requesting client | 10 Hz, one arena at a time | same encoding as own-arena stream, swapped on demand | as own-arena |
| Clock/round/boss beacons | S→ all | per round + key ticks | round #, authoritative tick, boss spawn | tens of B |
| Input intents | C→S | sporadic | §10.3.2 | tens of B each |

- **Budget check**: the community-established per-client replication comfort zone is ≈ **50 KB/s** (**VERIFY:** informal/undocumented figure, widely cited on the DevForum; no official number). Worst-case own-arena (30 KB/s) + everything else fits, but only *because of the entity cap* — the cap is a correctness requirement of this design, and R0 measures the real number.
- **Transport mapping**: position/delta ticks over **UnreliableRemoteEvent** (note its ~900-byte per-event payload cap — **VERIFY:** exact current limit — so a full 3 KB late-game delta splits across events or the tick's changed-subset is chunked); discrete events, shop state, digests, and intents over reliable RemoteEvents. If late-game bandwidth measures hot: drop own-arena cadence to 7.5 Hz past minute 12, or aggregate far-from-tank chaff into cluster records.
- **Anti-cheat note**: replicating *only* render-relevant state means a client never holds another player's shop, RNG position, or anything actionable — information exposure is strictly narrower than the Steam design's.

### 10.3.7 Sessions, reconnection, and server loss

- **Reconnection is nearly free** — the headline simplification vs [`03 §3.7`](03-network-architecture.md)/[`04 §4.4.6`](04-protocol-and-messages.md). A Roblox rejoin is a fresh client joining the server; the server marks the slot reconnectable (keyed by UserId), keeps the arena simulating meanwhile (the "continue" grace policy from [`03 §3.7`](03-network-architecture.md), now the only sensible option), and the rejoining client just receives current state like any late-joining renderer. No input-log replay, no snapshot protocol, no client catch-up sim. A rejoin window (e.g. rest of match, since the server sims on regardless) is a trivial policy knob.
- **Match instancing [design choice]**: v1 is a **single place with 8-player servers** — Roblox matchmaking fills servers, an in-server lobby (ready-up, host-set speed, skin display) runs the [`04`](04-protocol-and-messages.md)-equivalent lifecycle, and consecutive matches loop in the same server (good for the social loop and for Creator Rewards engagement time). Private matches = Roblox **private servers** (§10.5). Reserved-server teleport architecture (separate lobby place) is deferred until CCU justifies it.
- **Server loss** (crash, or platform update-triggered shutdown) ends the match with no equivalent of host migration. Accepted for v1 **[design choice]**: matches are ≤ ~17 minutes, `game:BindToClose` + soft-shutdown handling migrates *between* matches, and update rollouts are scheduled off-peak. Periodic MemoryStore checkpointing for mid-match resurrection is possible but disproportionate; revisit only if live data shows meaningful mid-match server mortality.
- **Meta persistence**: profile (earned achievements/badges, selected skin, bests, run history — the `profile.gd` shape) in **DataStore** (per-UserId), badges via BadgeService, session-scoped state never persisted. Same single-writer discipline `profile.gd` already documents.

## 10.4 Presentation approach

Three candidates, compared against Roblox audience/discovery norms and our asset base:

| | (a) Faithful 2D/UI port | (b) 3D isometric board, billboarded sprites | (c) Full 3D |
| --- | --- | --- | --- |
| Fidelity to shipped game | Highest — pixel-for-pixel the Godot arena | High — same top-down board, same readability | Lowest — re-stages everything |
| Reuse of our art | Direct (rasterized sprites in ScreenGuis) | Direct (sprites as billboards/decals on a 3D board) | Concept art only; full remodel |
| Roblox audience/discovery fit | **Poor.** Pure-2D UI games are rare on Roblox and essentially absent from the top charts (**VERIFY:** no chart-topping pure-2D precedent found); thumbnails/screenshots read as "not a real Roblox game"; no avatar presence | **Good.** Reads as a Roblox game in thumbnails; matches the visual language of the genre's hits (low-poly/stylized 3D boards); avatars can stand beside their tank | **Good** for discovery, but cost is a second art program |
| Effort | S–M | **M** | L–XL |
| Mobile/touch (majority of Roblox play) | Good (it's all UI) | Good (fixed isometric camera, tap targets) | Camera management burden |
| Risk | Discovery failure (§10.8-L3) amplified | Modest — new render layer, known quantity | Schedule + art budget |

**Recommendation: (b) — 3D isometric board with billboarded/flat-sprite entities [design choice].** One fixed isometric camera per arena on a decorated 3D board; enemies/projectiles rendered as camera-facing quads (SurfaceGui/decal on flat parts, or particle-style billboards) using the rasterized Ashen Vigil sprites; the tank itself as a simple 3D model wearing the sprite-derived skin art (skins stay meaningful, and the player's avatar can lean on the railing next to their tank — free Roblox-native charm and screenshot appeal). The shop/HUD is a straight port of the Godot UI layout to Roblox UI. This keeps our art direction and the game's top-down readability *and* looks native to the platform — (a)'s discovery penalty is disqualifying for a track whose entire rationale is audience, and (c) buys nothing (b) doesn't at several times the cost. The 8-arena net view ports as a UI grid fed by digests, unchanged in concept. Final pick is owner question #1 (§10.9).

## 10.5 Monetization design

Posture first, mechanics second: **the meta stays cosmetic. No pay-to-win — nothing purchasable may touch the sim.** This is the existing contract (`profile.gd`: "COSMETIC ONLY — none of this ever feeds the deterministic sim") promoted to a platform-track principle, and it is also the *correct commercial read* of the Roblox TD genre, where cosmetic/collection monetization dominates the healthy end of the market. Paid convenience that skips *grind* is acceptable only where it never touches match outcomes (e.g. unlocking a skin early); paid *power* never.

### 10.5.1 Platform economics (researched 2026-07)

- **Marketplace fee**: Roblox takes 30% of every in-experience sale; the developer nets 70% in Robux.
- **DevEx (cash-out)**: **$0.0038 per earned Robux** (rate updated 2025-09-05), with a **VERIFY:** special higher rate ($0.0054/R$) applying to eligible in-game spend by US 18+ players (effective 2026-06-08 — confirm mechanics and eligibility before modeling on it). **Minimum cash-out: 30,000 earned Robux (≈ $114).** Only *earned* Robux qualify.
- **Effective share of consumer spend ≈ 25%** (**VERIFY:** depends on the buyer's Robux pack price; players pay roughly $0.0099–0.0125/R$, dev nets 70% × $0.0038 ⇒ ~21–27¢ per consumer $1). Budget with 25%.
- **Engagement payouts**: the old Premium Payouts / Engagement-Based Payouts program was **deprecated 2025-07-24 and replaced by "Creator Rewards"** — payment proportional to engaged playtime of spending users. **VERIFY:** community-estimated yield ≈ 0.5–1.2 R$ per engagement-hour; treat as order-of-magnitude only.
- **Private servers**: developer-priced monthly Robux subscriptions per game. Platform churn note: **Roblox Plus** (platform subscription, $4.99/mo, launched 2026-04-30) included unlimited free private servers, **VERIFY:** being cut to *one complimentary private server per game* from 2026-09-24, restoring developer private-server revenue. This area is actively moving — re-check at R3.
- **In-experience subscriptions** exist (monthly, Robux-priced, ~70% net like other sales — **VERIFY:** current subscription fee split).

### 10.5.2 Product mapping [design choice]

| Existing meta | Roblox mechanism | Notes |
| --- | --- | --- |
| 13 tank skins (achievement-gated) | **Stay achievement-gated (free)**; sell *additional* Roblox-exclusive skin lines via in-experience shop (developer products) and limited game-pass bundles | Never paywall the 13 earned skins — they are the challenge meta's payoff |
| Achievements (12) | Roblox **badges** (free, platform-native trophy surface) | Badge grant mirrors `profile.gd::record_match` rules |
| Challenges (8) | Free in-experience picker (as today) | A "challenge pack" *of new cosmetic-reward challenges* is a candidate game pass — content, not power |
| Themes / board cosmetics | In-experience shop: arena board themes, tank trail/hit FX styles, death animations | The natural recurring cosmetic surface for a game with one immobile avatar |
| Private lobbies | Roblox **private servers**, modest monthly price (e.g. R$100/mo class) | Friend-group lobbies; also the "house rules" home if mutators ([`02 §2.11`](02-game-design.md)) ship |
| Supporter tier | Optional **in-experience subscription**: monthly cosmetic drop + name flair | Low-pressure; evaluate at R3, not launch-blocking |
| Passive | **Creator Rewards** engagement payout | Free money proportional to retention; rewards the same thing good design does |

Explicitly rejected: paid rerolls, paid gold, paid weapons/modifiers, XP boosts, loot boxes of any kind, and trading economies (the genre's grey-market magnet, and a moderation/compliance burden we don't want).

### 10.5.3 Worked revenue example — **all figures VERIFY: illustrative, not projections**

Assumptions (each uncertain, tagged once here): DAU ≈ 20× average CCU (**VERIFY**); gross consumer spend ARPDAU $0.02–0.05 for a cosmetic-only casual title (**VERIFY**); effective developer share 25% of consumer spend (§10.5.1); Creator Rewards ≈ 1 R$/engagement-hour (**VERIFY**); avg session ~45 min.

| Tier | Avg CCU | ≈DAU | Purchases (dev share) | Creator Rewards | Ballpark total |
| --- | --- | --- | --- | --- | --- |
| Small (finds a niche) | 20 | ~400 | ~$2/day | ~480 R$/day ≈ $1.8/day | **~$100–150/mo** — hobby scale; DevEx threshold reached in ~1 month |
| Modest (stable genre entry) | 300 | ~6,000 | ~$45/day | ~7,200 R$/day ≈ $27/day | **~$2,000–2,500/mo** |
| Hit (front-page/TD-chart class) | 5,000 | ~100,000 | ~$1,250/day | ~120k R$/day ≈ $450/day | **~$50k/mo** |

The spread is the honest story: outcomes are **hit-driven and extremely skewed**; the small tier is the base case, the hit tier requires discovery success (§10.5.4). No revenue from this track should be planned into anything.

### 10.5.4 Discovery reality check

- The genre benchmark ceiling is real: **Tower Defense Simulator** has passed ~4B lifetime visits; **Toilet Tower Defense** ~2B+ with day-to-day CCU in the tens of thousands; anime-IP TD games (All Star Tower Defense etc.) are similarly huge (**VERIFY:** current visit/CCU figures). The audience demonstrably loves this genre — and the incumbents are entrenched.
- Discovery is algorithmic (home-page recommendation driven by session length, D1/D7 retention, and monetization quality signals) plus paid **sponsored ads** (~$0.10–0.50 per click class — **VERIFY**) — the cold-start problem is the defining commercial risk: a new experience with no CCU gets no organic surface area until retention metrics earn it. Plan: R4 runs a small, metered ad-spend test ($10–50/day class) purely to buy a statistically usable retention sample, with explicit go/no-go thresholds — not a growth budget.
- Our differentiators to lean on in store copy/thumbnail: *last-man-standing race* (nobody in the Roblox TD chart does 8-way survival racing), 15-minute complete-match arc (fits platform session norms), and the funny-animal enemy cast (thumbnail-friendly, age-appropriate).

## 10.6 Compliance & safety

- **Age rating**: set by Roblox's Maturity & Compliance Questionnaire. Our content — cartoon funny-animal enemies, no blood, unrealistic stylized combat — should land **Minimal (5+) or Mild (9+)** (**VERIFY:** questionnaire outcome; answer it honestly re: repeated mild fantasy violence). ESRB-aligned labels are rolling out on top of Roblox's categories; no design change expected either way. Nothing in the game (no gambling-shaped mechanics — the shop is a deterministic-RNG *purchase* board with no paid randomness, and we keep it that way; no scary imagery; no user-generated content surfaces) pushes the rating up.
- **Chat & social**: use platform `TextChatService` exclusively (built-in filtering/moderation, COPPA-scoped) — never custom chat, never free-text that bypasses filtering. Player-visible names come from the platform. No off-platform links.
- **IP / originality**: our position is clean and documented — `CREDITS.md` states all shipped content is original; the enemy roster, boss, weapon names, and art are ours; "inspired by a WC3 custom map's *mechanics*" is not an IP encumbrance (mechanics aren't protectable expression; no Blizzard/WC3 names or assets exist in the project, and none may enter the Roblox build either — including in store metadata). The name "Standing Tank Defense" and all store art must continue to avoid trade-dress proximity to existing Roblox TD titles.
- **Uploaded-asset moderation**: every image/audio asset passes Roblox moderation with nonzero queue time and false-positive risk; batch uploads early per milestone, keep the SVG→PNG/audio pipelines re-runnable so a rejected asset is regenerated, not hand-fixed. Original/CC0-only sourcing (the [`09 §9.4-P4`](09-rebuild-plan.md) policy) is what makes this low-risk.
- **Data**: DataStore holds only gameplay meta keyed by UserId (no PII beyond what the platform provides); comply with Roblox's data & privacy requirements as-is. GDPR right-to-erasure requests arrive via Roblox messaging — build the DataStore removal path at R3, not later.
- **Account/operational compliance**: DevEx requires an eligible, ID-verified account holder and tax documentation — an *owner* decision about who holds the publishing account and receives payouts (§10.9), settled before R3, since group vs. personal account changes payout plumbing.

## 10.7 Milestone plan R0–R4

Same philosophy as [`06`](06-roadmap-risks-testing.md): risk-first ordering, each milestone gated by an objective exit criterion. Effort classes S/M/L as in [`09 §9.6`](09-rebuild-plan.md). R0 gates everything, exactly as M0 did.

| # | Milestone | Contents | Objective exit criterion (gate) | Effort |
| --- | --- | --- | --- | --- |
| **R0** | **Perf & math spike** | One-arena core loop in Luau (spawn/move/target/damage/status subset) on buffers + native codegen; the §10.3.3 integer-math kit (`mul16`, range audit); exporter walking skeleton (enemies + a weapon subset); replication prototype of the own-arena delta stream; Rust trace-dump mode | On a real Roblox server: **8 concurrent capped arenas (500 enemies + 200 projectiles each) tick at 30 Hz with p99 sim cost ≤ 15 ms/tick and per-client replication ≤ 50 KB/s measured**; Luau replay of ≥ 20 seeds matches the Rust reference trace exactly on the ported subset. Miss ⇒ redesign (lower caps / lower cadence / scaled units) before any further work | **M** |
| **R1** | **Vertical slice** | Full 15-min arc single-player-in-server: complete exported catalog, shop/reroll/Black Market state machine, modifiers/statuses/hazards/summons, boss + swift end + `Clear`; presentation approach (§10.4) implemented for one arena; HUD/shop UI port | A human (and the ported bot) plays the **full arc to a boss resolution** on a live server; trace-parity gate green across the full catalog on the agreed seed set; sim + replication budgets still inside R0 numbers | **L** |
| **R2** | **8-player match loop** | In-server lobby (ready-up, host-set speed), 8 concurrent arenas, digests + net view grid, spectate feed, death/placement/Last Stand resolution, reconnection window, intent validation + rate limiting hardened | **8 real testers complete a full match**: placement/tiebreak order provably matches [`02 §2.7`](02-game-design.md) rules; a mid-match rejoin renders correctly within 5 s; a malicious-client test script (illegal/spammed intents) produces zero server errors and zero sim effects | **M–L** |
| **R3** | **Meta & monetization** | DataStore profile + bests + run history; 12 badges; skin shop (earned skins free, cosmetic shop per §10.5.2); private servers; localization tables from the existing CSV; GDPR removal path; monetization re-verification of every §10.5.1 **VERIFY** figure | Purchases grant/persist correctly across sessions and server restarts; a written **no-P2W audit** (every sellable item enumerated, none touches sim state) signed off by the owner; badges award per `profile.gd` rules in live play | **M** |
| **R4** | **Soft launch + ads test** | Maturity questionnaire, store page (icon/thumbnails from batch-E-class art), analytics funnels (session length, D1/D7, match completion), metered sponsored-ads test with a fixed small budget, live-ops runbook (update windows, soft shutdown) | A pre-registered decision memo: after the fixed ad budget, **measured D1 retention and cost-per-new-player vs. thresholds set in the memo before spend** (VERIFY: platform norms suggest D1 ≥ ~20–25% as a viability line) ⇒ owner makes an explicit continue/park decision on data | **M** |

Sequencing: R0 → R1 → R2 → (R3 ∥ R4-prep). The exporter and parity harness are built once, in R0/R1, and run as standing gates thereafter. Per the operating model, milestones decompose into tightly-scoped agent tasks against the interfaces defined here (exporter output shape, replication schema, intent set) — those interfaces are defined centrally before fan-out.

## 10.8 Risk register

| # | Risk | Sev | Mitigation |
| --- | --- | --- | --- |
| L1 | **Luau perf**: 8 capped arenas + serialization miss the 30 Hz budget on real servers (GC pauses, native-codegen limits) | High | R0 measures before anything else is built; buffers/SoA + preallocation (GC-flat by construction); arena staggering; cap/cadence dials; worst case: lower cap is a *balance* change → owner sign-off per the divergence policy |
| L2 | **Double-precision math port bugs**: silent overflow past 2^53 or order-of-ops drift vs the Rust core | High | §10.3.3 range audit (static in exporter + dev-build asserts); single `mul16` chokepoint; the trace-parity harness (§10.3.5) catches drift on real seeds, not by inspection |
| L3 | **Discovery failure**: the cold-start problem eats the track — game is good, nobody arrives | High (likelihood), bounded (cost) | Presentation choice (b) for platform-native appeal; R4 is a *metered test with pre-registered thresholds*, not an open-ended spend; total downside is capped at the R0–R4 build cost + the fixed ad budget |
| L4 | **Second-codebase drift**: Luau and Rust implementations diverge over time | High | Exporter as the only content path + generated-file diff in CI; trace-parity gate standing from R1; spec-first divergence policy (§10.1) with a written per-platform overlay; the Roblox track never patches rules locally |
| L5 | **Moderation delays / asset rejections** stall art & audio integration | Med | Batch uploads at milestone start; re-runnable asset pipelines; original/CC0-only sourcing keeps rejection rates low; no launch date promised against a moderation queue |
| L6 | **Platform policy/economics churn** (DevEx rate, Creator Rewards, private-server rules — all changed within the last ~12 months) | Med | Monetization numbers re-verified at R3 (every VERIFY tag is the checklist); design principle (cosmetic-only) is robust to rate changes; no revenue is load-bearing for the project |
| L7 | **Server loss mid-match** (crash/update) ends matches with no migration | Low | §10.3.7: soft-shutdown between matches, off-peak update windows, ≤17-min matches bound the loss; revisit checkpointing only on live evidence |
| L8 | **Audience mismatch**: a menu-driven build-optimization game under-retains with Roblox's young mobile majority | Med | 15-min arc fits session norms; funny-animal cast + presentation (b) meet the audience visually; R1 playtests with target-age players before R2 investment; E-series variety systems (§10.9) held as retention levers |
| L9 | **Compliance surprise**: questionnaire outcome higher than expected, or a mechanic read as gambling-adjacent | Low | No paid randomness anywhere (§10.5.2 rejections); shop is skill-facing deterministic RNG with no purchase link; pre-check the questionnaire at R3 |

## 10.9 Open questions for the owner

1. **Presentation** (§10.4): confirm recommendation (b) 3D isometric board with billboarded sprites — or direct (a)/(c).
2. **Monetization aggressiveness** (§10.5.2): cosmetic shop + private servers only (recommended floor)? Add the game-pass challenge/cosmetic bundles at launch or later? Ship the in-experience subscription at all?
3. **E-series parity** ([`02 §2.10–2.13`](02-game-design.md)): do score attack, mutators, loadouts, and set bonuses ship on Roblox at parity when they land in the spec, or does Roblox launch on the pre-E ruleset with E-series as post-launch content drops (retention levers, per L8)?
4. **Publishing account & payouts** (§10.6): who owns/operates the Roblox account (personal vs. group), who is the ID-verified DevEx recipient, and who handles the tax paperwork? Needed before R3.
5. **Naming/branding**: ship as "Standing Tank Defense" on Roblox, or a platform title variant? (Store-name A/B is cheap during R4.)
6. **Budget ceiling for R4's ad test** (a fixed number pre-registered in the decision memo).
7. **Session-length pressure**: if R1 playtests show the 15-minute fixed arc is too long for the platform's youngest cohort, is a shorter *labeled* per-platform arc variant even on the table (a spec-first proposal per §10.1), or is the arc inviolate?

## 10.10 References (researched 2026-07)

- DevEx rates & thresholds — https://en.help.roblox.com/hc/en-us/articles/13061189551124-Developer-Exchange-Help-and-Information-Page
- Engagement-based payouts → Creator Rewards — https://create.roblox.com/docs/production/monetization/engagement-based-payouts
- Game passes / developer products / marketplace fee — https://create.roblox.com/docs/production/monetization/passes
- Private-server & Roblox Plus changes — https://devforum.roblox.com/t/upcoming-updates-to-private-servers-new-tools-and-new-earning-opportunities/4590482
- Luau native code generation — https://create.roblox.com/docs/luau/native-code-gen
- Content maturity & compliance questionnaire — https://create.roblox.com/docs/production/promotion/content-maturity
- Replication budget discussion (informal ~50 KB/s figure) — https://devforum.roblox.com/t/remote-limits-per-remote-or-overall-whered-the-limit-even-come-from/2873609
- Roblox ToU / IP policy — https://en.help.roblox.com/hc/en-us/articles/115004647846-Roblox-Terms-of-Use
