# 06 — Roadmap, Risks & Testing

How to build Standing Tank Defense in de-risking order, what can go wrong (networking-weighted), and how we prove the netcode works. Deployment target is a **free Steam app over Steamworks P2P / Steam Datagram Relay** — see [`07-steamworks-integration.md`](07-steamworks-integration.md).

## 6.1 Milestones (vertical slices, netcode-first)

Each milestone is a *playable* slice; we add network surface before content depth, because networking is the expensive-to-retrofit risk.

### M0 — Deterministic single-arena sim (no network) ✅ **DONE**
- Fixed 30 Hz tick; integer/fixed-point combat; one tank, a few weapons, one wave, the offer/shop loop, gold, death.
- **Exit test**: run the same seed + scripted input log twice → **identical `state_checksum`** every tick (the determinism harness, §6.3). This gates everything else.
- **Status:** implemented in [`sim-core/`](../sim-core/) — `determinism` + `sim` + `harness` crates, 48 tests passing, gate `PASS`. A golden-checksum test runs across the CI OS matrix to prove *cross-platform* bit-identical results, not just same-machine reproducibility. Built orchestrator-style: central seams (`ids`/`state`/`content`/`lib`/determinism) hand-authored, behavior modules (`combat`/`waves`, `economy`/`shop`/`input`) implemented by parallel agents against fixed interfaces, integrated centrally.

### M1 — Shadow-sim & checksums in-process ✅ **DONE**
- Run two instances of the sim (the "client" and the "shadow") in one process from the same seed+inputs; diff checksums each tick.
- Build `Snapshot` serialize/deserialize + replay-from-snapshot.
- **Exit test**: inject a deliberate divergence → detected within one digest interval → corrected by snapshot → checksums reconverge.
- **Status:** done in [`sim-core/`](../sim-core/). `sim::snapshot` gives byte-level `serialize`/`deserialize` (round-trip identity, versioned, zero-dep). `harness::shadow::ShadowRunner` runs client+shadow, compares `state_checksum` every digest interval, and corrects the client from a shadow snapshot on mismatch; `replay_from_snapshot` is the reconnect path. Verified: injected divergence detected and corrected within one interval (byte-identical after), reconverges with no spurious corrections, and snapshot+input-log replay reproduces the reference exactly. 58 tests passing. Built orchestrator-style: snapshot seam authored centrally, the shadow driver + exit tests by a scoped agent, integrated and independently re-verified.

### M2 — Match director + transport (Steam) ✅ **DONE**
- Stand up the match director (lobby, clock/time beacons, seed schedule, input ordering/ack, leaderboard, death/placement) over **SteamNetworkingSockets** ([`07`](07-steamworks-integration.md)).
- 2 players, 2 arenas, host-authoritative; CONTROL/TELEMETRY/BULK channels mapped to Steam reliable/unreliable lanes.
- **Exit test**: two clients play independent arenas; one's lag/stall never affects the other's tick rate (the core thesis, §6.3 latency injection).
- **Status:** done in [`sim-core/crates/net`](../sim-core/crates/net/) — **transport-abstracted** so it runs without the Steam SDK in this environment (a deterministic in-process `Hub` with latency/stall injection now; the `ISteamNetworkingSockets`/SDR adapter drops in behind the same `Msg`/`Outbound` interface on a dev machine per [`07`](07-steamworks-integration.md)). Implemented: authoritative `Director` (clock, per-player shadow-sims, input ordering + `InputAck`, digest validation → `Snapshot` correction, death/placement), thin non-predictive `Client`, `wire` Msg codec, shared `Schedule` (the apply-tick invariant), `CONTROL/TELEMETRY/BULK` channels. **Exit test proves the thesis**: P2's arena trace is byte-identical whether or not P1 is stalled — one client's lag cannot perturb another's simulation (the exact WC3 lockstep failure this architecture removes); healthy clients stay bit-identical to their authoritative shadows through the transport with zero corrections. 82 tests workspace-wide. Built orchestrator-style: transport/wire/hub/schedule seams authored centrally; director and client implemented by two parallel agents (decoupled by the wire protocol); integrated, a stale-snapshot test assertion corrected, and the independence exit test authored + verified centrally.

### M3 — 8-player lobby, reconnect, host migration ✅ **DONE**
- Full 8 slots; Steam lobby + SDR relay; reconnect (re-seed + replay); host migration to a hot standby.
- **Exit test**: kill the host mid-match → director role migrates → all 8 arenas continue; a dropped client rejoins and catches up to the live tick.
- **Status:** done in [`sim-core/crates/net`](../sim-core/crates/net/). **Reconnect:** a mid-match `Client::reconnecting` sends `Join`; the director replies with an authoritative `Snapshot` of that player's shadow; the client adopts it and tracks the arena's own tick thereafter (a late joiner whose local `iter` is far below the live tick aligns correctly, running consistently "behind" in wall-time, which is bit-correct because state is indexed by arena tick). **`eight_players_reconnect`** exit test: 8 independent arenas under one director; one player drops and rejoins, adopts state, and stays on the canonical trajectory with zero corrections while the other seven remain in lockstep with their shadows. **Host migration:** **`host_migration`** exit test runs a hot-standby director fed identical inbound in lockstep (bit-identical state, asserted at the handoff); killing the primary mid-match hands off seamlessly — clients continue in sync with the new director with zero corrections, inputs/acks working on both sides. 81 tests workspace-wide. Built orchestrator-style: the reconnect protocol (coupled client+director change) authored centrally with two correction-semantics fixes found during integration; exit tests authored and verified centrally.

### M4 — Content depth & balance ✅ **SYSTEMS DONE** (catalog breadth ongoing)
- Import the extracted catalog (`research/.../catalog.json`): full weapon/modifier/enemy/wave set, rarities, status systems (Poison/Frost/Fire/Spikes), Mana Shield, Samwise boss, `Clear`.
- Steam stats/leaderboards/achievements for placement, Last Stand, and the challenge/score meta.
- **Every gameplay SYSTEM is implemented, deterministic, and tested** (`sim` crate, 80 tests; 116 workspace-wide; zero warnings; golden checksum re-baselined each pass):
  - **Modifier / stacking engine** (`sim::modifiers`, `docs/05 §5.3`) — additive‑within‑source, multiplicative‑across‑sources; scoped damage (global/per‑type/multiplicative), attack speed, bounty %, income, max HP, **and defensive** (armor, mana shield, HP regen, dodge). Modifiers are purchasable in the shop (`Offer` = weapon **or** modifier).
  - **Status effects** (`sim::status`) — Poison (DoT, kills award bounty), Frost (stacking slow, capped/decaying), Fire (damage vulnerability), Stun (immobilize); applied on hit, ticked as a step phase, consulted by combat for damage + movement.
  - **All attack types** (`content::Attack`) — SingleTarget, Splash, **Barrage**, **Area**, **Wave**, **Bounce** (traveling projectiles for the first three; instant resolution via the shared `apply_weapon_hit` for the rest).
  - **Defensive layer** (`sim::defense`) — contact damage routed Dodge → Armor → Mana Shield → HP, with per‑tick mana/HP regen.
  - **Match arc** — `content::enemy_hp_mult` HP scaling (identity → ×2 by 15 min → steeper), and the **Samwise** boss spawned at the 15‑min tick: immune to weapon fire, killable only by `Clear` (now large‑but‑finite, so the boss is a multi‑Clear fight).
  - **Steam** results mapping (`net::results::MatchStats`) + transport‑adapter plan (`net::steam`) behind the same `Inbound`/`Outbound` interface the in‑process `Hub` uses (live SDK binding can't build in this sandbox; documented + tested scaffold).
  - A `full_arc` integration test drives the entire 11‑phase `step()` pipeline through a whole match to the boss spawn and proves it composes byte‑deterministically.
- **Catalog imported & audited.** The extracted catalog is now in the sim (`gen_catalog.py` bootstrap → **85 weapons / 40 modifiers**, then hand-curated names + cost/rarity). Two read-only alignment agents verified the generated entries against the source: weapons convert 1:1 and correctly (damage/type incl. "& flavor", attack family + params, cooldown ×30 incl. N/A→30, status math) — the only non-source value is a deliberate default of 3 Frost/Fire stacks on 3 flavor-tagged weapons that list no explicit stacks (kept, since Frost/Fire are modeled only as status); modifiers all map correctly. A naming agent assigned source-accurate names (Magic Missile, Throwing Axes, Steam Cannon, Inferno Stone, Cluster Rockets, …).
- **Prioritized content backlog (new mechanics the import revealed — ranked by value/effort).** ~60 source upgrades can't be expressed by the current `ModEffect` set; the highest-leverage additions, each a tight task against the existing seams:
  1. ~~**Time-scaling growth** (~15 upgrades: "+X every 30 s")~~ ✅ **DONE** — `RampSpec` on `ModifierDef` + `ActiveRamp` on the arena; a ramping modifier applies a base effect on purchase and re-applies a growth effect every `RAMP_PER_ROUND` ticks (`modifiers::apply_ramps` phase). `ModEffect` got a stable `words`/`from_words` codec so ramps ride in checksum + snapshot. Catalog gains Building Power, Escalating Chaos, Compounding Greed, Hardening. Deterministic, tested, golden re-baselined.
  2. ~~**Per-scope damage** (~7: "+% for Splash/Wave/Single-Target/range-bucket/rarity")~~ ✅ **DONE** — `ModEffect::DamageScopePct(scope_id, …)` + a 12-slot `add_by_scope` aggregate (6 attack classes, 2 range buckets, 4 rarities; `content::*_scope_id`). The full per-weapon multiplier (global + type + scope) is resolved at **fire time** by `Modifiers::weapon_damage_mult` and baked into projectile damage (so no new projectile fields), or applied live for instant attacks. Catalog gains 9 scope modifiers. Deterministic, tested, golden re-baselined.
  3. ~~**Spikes + trigger framework** (~19) — the tank as a damage *source*.~~ ✅ **DONE**: ✅ **Spikes** (on-being-hit retaliation: `Tank::spikes_damage`/`spikes_mult` + a `tank_hit_this_tick` flag set in `hit_tank`, applied in the `defense::spikes` phase; `SpikesFlat`/`SpikesPct` + Bloody Spikes ramp). ✅ **Periodic aura trigger** — **Vulnerability Pulse** (`VulnPulse` on the arena, registered via the intercepted `GrantVulnPulse` effect, applied in `status::pulse`; stacks generic `vuln_stacks` = +1%/stack damage-taken on enemies in range each interval). ✅ **On-event triggers** — `ModEffect::HealOnKill`/`HealOnPoison` set `Tank::heal_on_kill`/`heal_on_poison`; heal-on-kill fires in `economy::collect_bounties` (per enemy killed this tick), heal-on-poison in `status::tick` (per enemy taking poison this tick), both capped at `max_hp` and skipped when dead. Catalog gains +15/+60 Heal on Kill and Vampiric Spores. (Damaging auras like Immolation/Blight are already covered by `Area`/`Wave` weapons.) Deterministic, tested, golden re-baselined.
  4. **Status-conditional & flavor damage** (~13: "+% vs Stunned/Poisoned", "+% Poison/Frost/Fire damage", "+% Stun Duration") — needs a status/flavor damage hook.
  5. **Meta/shop items** (~8: copies/duplicators, Black Market, Magic Treasure) — director-side, not the sim hot path.
- **Economy depth** (new, hot-path source mechanics beyond flat income/bounty). ✅ **DONE**: the economy is now a real strategic system, not just a flat drip. **Income multiplier** (`ModEffect::IncomePct` → `Economy::income_mult`, applied in `economy::tick_income`; `bounty_mult` still never touches passive income per the source rule). **Income-as-HP-regen** (`IncomeRegenPct` → `Economy::income_regen_pct`; each income award also heals the tank that fraction, capped at `max_hp`, skipped when dead — the source's "% of Gold Income as instant HP Regen", tying eco to survival). **Gambling bounty proc** (`BountyProc(chance_pct, bonus_pct)` → `Economy::bounty_proc_chance_pct`/`bounty_proc_bonus`; per kill in `collect_bounties`, with `chance%` it pays an extra `bonus%` of the base bounty, rolled from the existing seeded `rng_proc` stream — RNG drawn **only** when a proc is owned, so the baseline cursor is untouched). Catalog gains +10%/+25% Gold Income, Golden Vitality, Lucky Strikes. Deterministic, snapshot v9, golden re-baselined.
  6. **Self-scaling, healing, revive** (Terror/Ankh, "% per N Max HP", "% Missing HP heal", `HealingPct`) — smaller, bespoke.
- A few **exotic weapon abilities** (summons, life/mana drain, rotating-wave geometry, fire-explode-on-death) are currently imported as base weapons (`// exotic: … (base only)` comments) and would be re-enabled by the trigger framework in (3).

### M5 — Hardening & ship
- Anti-cheat posture for a free P2P game (§6.2, [`07 §7.6`](07-steamworks-integration.md)), soak/chaos tests, Steam release (free).

## 6.2 Risk register (networking-weighted)

| # | Risk | Likelihood | Impact | Mitigation |
| --- | --- | --- | --- | --- |
| R1 | **Non-determinism** in the sim → constant checksum mismatches → snapshot spam | High | High | Integer/fixed-point hot path; determinism harness in CI (M0 gate); banned-ops lint; stable iteration order ([`05 §5.6.1`](05-data-model.md)) |
| R2 | **Host-authority trust** (free P2P: host could cheat its own arena/placement) | Med | Med | Honest scoping: casual has low incentive; Steam auth tickets prevent impersonation; optional community **dedicated** servers for ranked; replay submission + server-side sanity checks for leaderboards ([`07 §7.6`](07-steamworks-integration.md)) |
| R3 | **Late-game swarm** tanks the host's CPU running N shadow-sims | Med | High | Sim is light & deterministic; spot-check validation (full re-sim only on suspicion/placement); entity caps & culling; budget per arena ([`03 §3.9`](03-network-architecture.md)) |
| R4 | **Host migration** loses match state | Med | High | Director state is tiny + replicated to hot standby; arenas are seed+input derived so they survive migration ([`03 §3.10`](03-network-architecture.md)) |
| R5 | **NAT / connectivity** failures between peers | Med | Med | Steam Datagram Relay (SDR) routes via Valve's backbone — no port-forwarding, NAT punch handled; relay fallback always available ([`07 §7.2`](07-steamworks-integration.md)) |
| R6 | **Content/version drift** between players → desync | Low | High | `content_hash` gate at join ([`04 §4.4.1`](04-protocol-and-messages.md)); Steam ensures a single shipped build |
| R7 | **Snapshot bandwidth** spikes if corrections are frequent | Low | Med | Corrections are per-*one*-arena & rare if R1 controlled; delta-encode snapshots; cap correction rate, escalate to kick |
| R8 | **Float determinism across CPUs/compilers** | Med | Med | Avoid floats in sim entirely (fixed-point); if unavoidable, pin rounding & forbid fast-math; the pairwise-determinism + snapshot net makes residual drift survivable ([`03 §3.3`](03-network-architecture.md)) |

## 6.3 Test plan (proving the netcode)

### Determinism harness (the foundation, CI-gated)
- Replay engine: `(seed, input_log) → checksum_trace`. Same inputs must yield byte-identical traces across runs, OSes, and CPUs. Cross-platform CI matrix (Win/Linux/Mac) runs it on every commit. A divergence **fails the build** (R1/R8).

### Latency & chaos injection (the thesis test)
- A network-shim layer injects per-link **latency, jitter, loss, reorder, and stalls** on the Steam channels.
- **Core thesis assertion**: a 2 s stall or 300 ms RTT on player A's link leaves player B's local tick rate unchanged (no cross-arena coupling — the WC3 failure we're avoiding). Automated.
- Pathological cases: host on a bad link; one client at 40% packet loss; mid-match host kill.

### Reconnect / migration tests
- Drop & rejoin at random ticks → client catches up to live tick with matching checksum.
- Kill host at random ticks → migration completes, all arenas continue, placement still correct.

### Soak & scale
- 8-arena, full-content, 20-minute matches running for hours; watch for memory growth, checksum drift, snapshot-rate creep, host CPU under late-game swarm (R3).

### Anti-cheat / validation tests
- Forge illegal inputs (buy without gold, reroll past limit, Clear on cooldown) → rejected at ordering.
- Tampered client checksum → snapshot correction → suspicion → kick threshold.
- Placement/death always fully re-validated by the authority.

## 6.4 Recommended stack (concrete, revisable)

| Layer | Recommendation | Why |
| --- | --- | --- |
| Transport | **Steamworks `ISteamNetworkingSockets`** (+ SDR relay) | Free routing over Valve's backbone, NAT/encryption/auth built in; perfect for a free P2P game ([`07`](07-steamworks-integration.md)) |
| Matchmaking/lobby | **Steam lobbies** (`ISteamMatchmaking`) | No backend to host |
| Identity/anti-impersonation | **Steam auth tickets**, VAC-eligible | Free; ties placement to a SteamID |
| Meta (leaderboards/stats/achievements) | **Steam Stats & Leaderboards** | Last Stand / placement / challenge score, server-side stored |
| Save (challenge meta) | **Steam Cloud** | Replaces the WC3 "codeless save/load" hack outright |
| Sim language | determinism-friendly (Rust/C++/C#-with-fixed-point) | Bit-stable cross-platform sim (R1/R8) |
| Engine | **Godot 4 + Rust sim core (locked)** ([`08`](08-engine-choice.md)) | Sim is an engine-independent Rust crate; Godot renders; Steam via GodotSteam |

## 6.5 Open questions

1. **Disconnect grace policy** — when a client drops, does their arena keep running on the host shadow (can still place) or freeze? Per-match toggle? ([`03 §3.7`](03-network-architecture.md))
2. **Ranked vs. casual authority** — ship casual as host-authoritative P2P; offer optional **dedicated/community servers** for a trusted ranked mode? (R2)
3. **Snapshot cadence** — fixed interval vs. on-demand-only; tune against determinism confidence after M1.
4. **Co-op / shared-arena mode** — explicitly out of v1 (re-introduces cross-arena coupling, [`02 §2.9`](02-game-design.md)); revisit post-ship.
5. **Cross-arena interaction** ("send a creep to a rival") — would couple sims and break the bandwidth/independence guarantees; deliberately excluded, noted for a possible separate mode.
6. **Map script** — first-hand decompile of the protected `war3map` script (wave timings, exact economy tick) would refine the *(inferred)* flow numbers ([`research/tower-survivors-map/README.md`](../research/tower-survivors-map/README.md)).
