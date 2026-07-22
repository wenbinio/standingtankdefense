# 02 — Game Design

Design spec for **Standing Tank Defense**, grounded in the extracted Tower Survivors v1.58 data (see [`01-source-analysis.md`](01-source-analysis.md) and [`appendix-A-map-extraction.md`](appendix-A-map-extraction.md)). This is a "GDD-lite": enough to drive the data model and the network design. Every system here is built to stay **deterministic and shardable** — see [`03-network-architecture.md`](03-network-architecture.md). Values taken directly from the map are stated as such; our own choices are marked **[design choice]**.

## 2.1 Design pillars

1. **One tank, no movement, no aiming.** The only verbs are *buy*, *upgrade*, *reroll*, and one manual ability. All skill is in build decisions under time pressure.
2. **Everything stacks, multiplicatively across sources.** The map's rule — *"different damage increases are multiplicative with each other"* — is the power-curve engine. Power comes from layering, not from a tech tree you outgrow.
3. **Randomized offers, meaningful choices.** Each round opens a new shop of random offers, weighted by rarity; rerolls and targeted shop items let you steer the randomness at a cost.
4. **You race the lobby, not fight it.** Players never touch each other's arenas. The competition is *relative survival time*. This is a pillar **and** the reason the netcode can be cheap.
5. **Readable escalation.** Difficulty ramps on a fixed clock with known breakpoints — the source's arc, as shipped: a smooth per-minute ramp across a **15-minute** arc, a **+20% step at 10:00**, then the boss at **15:00** with a post-15:00 **"swift end"** escalation (waves keep spawning and compound hard, so no match stalls). (`docs/01` §1.2; `sim::content::enemy_hp_mult`.)

## 2.2 The tank (player avatar)

- **Stationary**, fixed in its personal arena. (The source calls it "Survivor's Tower"; we re-skin to a tank.)
- **Has HP** plus optional defensive layers: **Armor**, **HP Regen** (with retroactive multipliers), **Dodge** (diminishing returns), and a **Mana Shield** absorb-pool subsystem. At 0 HP the tank dies and the player is eliminated with a recorded placement.
- **Auto-fires** every equipped weapon on its own cadence at a **randomly selected** valid target (no armor-type preference — see RNG, §2.8).
- **One manual ability — `Clear`.** A long-cooldown burst that is also the **only** thing that can damage the end boss. Manual `Clear` is the single real-time combat input a player makes; everything else is a menu action. It is therefore a first-class networked input (see [`04-protocol-and-messages.md`](04-protocol-and-messages.md)).

## 2.3 Match structure & clock

| Phase | Timing | Notes |
| --- | --- | --- |
| Lobby | until ready/countdown | Server assigns slots + master seed |
| Countdown | ~5 s **[design choice]** | Time-sync converges here |
| Rounds | **~30 s each**, a new **shop every round** | **15 min** of escalating waves |
| Scaling | smooth ramp, **+25%/min** after a 2-min grace | enemy HP **and** damage compound per minute to ≈ ×21.8 at 15:00 (`sim::content::enemy_hp_mult`; `RAMP_BASE` is the difficulty dial) |
| Scaling step | at **10 min** (`SCALE_STEP_TICK` = 18000) | an instantaneous **+20%** step (source changelog: "extra scaling after 10 minutes increased by 20%") |
| Boss + swift end | at **15 min** (`BOSS_SPAWN_TICK` = 27000) | **The Hippocrate** spawns; **fixed** HP/damage (does not scale); immune to weapon fire — only `Clear` damages it (a ~10-`Clear` race). Waves do **not** stop: from 15:00 they compound **×1.5/min** ("swift end"), the **shop closes** (it "flees"), and per-round ramp upgrades **stop accruing** |
| Resolution | — | placement + Last Stand awarded; the match ends when ≤1 tank remains or the boss is resolved |

*(As shipped — the restored source arc: the WC3 source runs ~15 min with 10/15-min scaling breakpoints to its boss, and its changelog rules — the 15-min ramp-stop and the fleeing shop — are implemented fact here (`docs/01` §1.2). An earlier chapter of this project shipped a stretched 30-min arc with a metronomic 3-min stepped ramp; this pass returned the game to the source arc, and that 30-min shape is history.)*

- **Round number and global clock are server-authoritative** and identical for everyone, so "round N" / "minute N" are comparable across the leaderboard.
- Within a round each arena runs **independently**: your round-N wave is the same *composition* as everyone else's round-N wave (shared wave table) but **per-player seeded**, so spawn timing/positions differ and can't be mirror-copied.
- **Game speed**: the source exposes Normal/Fast/Faster/Hyper, host-controlled. **[design choice]** We fix the *simulation* tick rate (for determinism) and treat "speed" purely as a host-set time-scale on the authoritative clock, applied identically everywhere — never a per-client setting.

## 2.4 Economy

Three income sources, scoped so multipliers don't trivially compound (matching the source's careful scoping):

1. **Passive Gold Income** — flat gold/sec, rising slightly per round. Income-% multipliers apply only to *bonus* income, not the base (source rule).
2. **Kill Bounty** — gold per kill by enemy tier; **+% Bounty** multipliers apply here, plus a fixed **5%-chance proc** for bonus bounty (source: "+200% Bounty Gold with 5% activation chance").
3. **Round bonus / meta-items** — lump sums and time-growing items (e.g. **Magic Treasure**: +250 gold now, value grows +2/sec).

Gold is spent in the **per-round shop**. **Rerolls** refresh the offers: you start with **5** rerolls, cost escalates per use, and some effects grant **Free Rerolls**.

**The shop closes at the boss** (source flavor: it "flees in fear of the boss's impending arrival"): from the 15:00 boss tick no new offers are rolled and the stalls **clear** — nothing remains purchasable, and buy/reroll inputs are deterministic no-ops. Spend it before the bell.

## 2.5 Content model

Summarized here; formalized as schemas in [`05-data-model.md`](05-data-model.md). Full extracted lists in Appendix A.

### Damage-type matrix
Five base types — **Normal, Piercing, Magic, Siege, Chaos** — each with armor-class multipliers from a matrix (the source's `war3mapMisc.txt`, e.g. Piercing 2× vs the first armor class). Layered on top are **status flavors**: **Poison, Frost, Fire, Spikes**. Damage bonuses are **additive within a source, multiplicative across sources**.

### Weapons (the auto-fire emitters)
Each weapon = `{base damage types, attack type, damage, DPS, attack cooldown, range, ability}`. From the 79-weapon extract:

- **Attack types**: `Single Target`, `Splash(R)`, `Bounce(N targets)`, `Barrage(N targets)`, `Wave(+R, rotating)`, `Area(R)` / `Area(enemies-in-range)`.
- **Range tiers**: 300 / 600 / 900 / 1200 (upgrades target these tiers).
- **Cooldown tiers**: 0.2 → 10.0, plus **`N/A`** (no cooldown; *does not benefit from +Attack Speed* — a real special case).
- **Abilities** attached to weapons: Stun, Root, Knockback, Freeze, on-hit vulnerability stacks, life/mana drain, summons (Skeletal Mage, Infernal), heals, etc.

### Status effects (exact rules from source)
- **Poison** — DoT + move/attack-speed slow (e.g. "50% slow & 900 dmg over 3s").
- **Frost** — 2% move/attack-speed slow per stack, **max 25**; upgrades can **Freeze** at 25 stacks (+50% frost damage taken).
- **Fire** — +0.5% all-damage-taken per stack; enemies **explode on death**.
- **Spikes** — retaliation damage when the tank is hit; its own +% scaling.
- **Stun / Root / Knockback** — crowd control; `+% Stun Duration` and "+% damage to stunned" upgrades exist.

### Modifiers (the stacking engine)
- **Category/scope modifiers** — by damage type ("+10% Chaos Damage"), by range tier, by attack class ("+25% Splash Weapons", "+25% Single Target Weapons"), by rarity ("+100% Common Weapons"), by enemy state ("+25% to Poisoned/Stunned").
- **Global modifiers** — +Attack Speed, +Range, +Max HP / +%, +HP Regen / +%, +Armor, +Dodge, Mana Shield grants, +Income, +Bounty.
- **Time/round modifiers** — "+X% every 30 s (each new shop)"; per the source rule these "no longer stack after 15 minutes" — accrual freezes at the boss tick (`modifiers::apply_ramps`), the already-accrued value keeps working.

### Rarity ladder (offer weighting & cost)
**Common (500g) → Uncommon → Rare → Epic**. Rarity sets draw weight, cost, and magnitude. *("Epic" and its ~5000g top cost are inferred — see `docs/01` §1.4; only the 500g Common cost is source-confirmed.)*

### Meta / targeted shop items (make the offer stream *stateful*)
- **Black Market** — buy one specific Uncommon weapon of your choice.
- **Multiplication Gems** — next Common upgrade is granted +3 copies.
- **Magic Treasure / Magic Coins** — economy items used from a shop inventory.
- **Copy effects** — "+1 copy of the next Rare weapon."

> These matter for networking: because items modify *future* offers, the offer RNG stream is **stateful and order-dependent**, so it must be replayed exactly on reconnect (§2.8, [`05`](05-data-model.md)).

## 2.6 Enemies & waves

- Enemy fields: `{HP, move speed, contact damage, bounty, armor class, archetype, abilities}`. The shipped roster is an **original 12-entry funny-animal cast** (`sim::content::ENEMIES` — Squeakzilla, Doomduck, Bacon, Honk, Bonk, Nope Rope, Croak, Spicy, Popsicle, Dodo, Fanged Death, plus the boss) covering the source's archetypes: melee swarmers, fast rushers, armored bruisers, casters, and ranged breathers/spitters. (The WC3 roster — Fel Orc Peons, Warlocks, Fire/Ice/Poison/Lava breathers — is cataloged in `docs/01` / Appendix A; no WC3 names or assets ship.)
- **Wave table per round** defines composition, counts, and cadence; shared across players for a given round, **per-player seeded** for individual spawn timing/position.
- **Scaling**: enemy base HP **and** damage compound on the per-minute ramp, with the +20% step at 10 min and the swift-end escalation from 15 min (§2.3).
- **Boss** (15 min): **The Hippocrate**, fixed stats, **immune to weapon fire — only `Clear` damages it** — and it grinds the tank with cadenced contact hits while the regular waves keep spawning on the swift-end curve, so the climax is a sustained multi-`Clear` race (~10 Clears) against a swelling tide. *(The source's boss, "Samwise", likewise had fixed/non-scaling stats; the Clear-only rule was a carried-over design assumption there, and is implemented fact here.)*

## 2.7 Win / lose & placement (last man standing)

- **Eliminated** at 0 HP; server records the **death tick**.
- **Placement = reverse order of elimination** (last to die = 1st).
- **Win = surviving (top) half of the lobby; lose = bottom half.** Sole survivor = **Last Stand**. The prestige objective (the source's +250 score, carried forward): *survive the full arc, outlive everyone, kill the boss.*
- **Tiebreak** (deterministic, no coin flip) **[design choice]**: simultaneous deaths broken by (1) higher round reached, then (2) more damage dealt, then (3) lower slot index — so placement is a total order.
- The match **ends** when ≤1 tank remains or the boss is resolved.

Competitively, the only fact the network must agree on exactly is **"who died, in what order"** — small and low-frequency. That is a deliberate design choice that keeps the netcode honest.

## 2.8 Randomization & determinism (design-level)

The game is "randomized," but for a fair, cheat-resistant, reconnectable multiplayer game, randomness is **deterministic and server-owned**:

- The server issues a **master seed** per match and derives **per-player, per-purpose RNG streams**: `spawn`, `offer/shop`, `targeting`, `proc` (crit/bounty/Fire-explosion), `reroll`. See [`05-data-model.md`](05-data-model.md) §RNG.
- Given a player's seeds + their **ordered input log**, their entire arena is **reproducible** — the basis for reconnection and server-side validation.
- The **shop/offer stream is stateful** (Black Market, copies, Multiplication Gems alter future draws), so it is modeled as a deterministic state machine advanced only by ordered inputs — players can't save-scum a reroll.

## 2.9 Out of scope for v1

- Co-op / shared-arena mode (architecture can grow into it; [03 §9](03-network-architecture.md)).
- The full single-player **challenge/score & skins** meta (the system exists in the source; we note it as a v2 meta-progression layer, not v1 multiplayer).
- **Cross-arena interaction** ("send a creep to a rival"): explicitly excluded in v1 because it would couple the otherwise-independent sims — noted as a deliberate networking trade-off in [03 §9](03-network-architecture.md), not a v1 feature.

---

## Post-source additions (§2.10–§2.13) — approved concepts, spec'd for review

The four systems below are **original, post-source additions**: they are *not* extracted from Tower Survivors, and per the source-fidelity posture ([`09` §fidelity program](09-rebuild-plan.md)) they are labeled as such — the same class of addition as the catalog's E1–E3 expansion batches in `sim::content` (this design track is **E4** when it lands in code, and stays flagged there and in `CREDITS.md`). The *concepts* are owner-approved; every number marked **`TUNABLE-PENDING-SWEEP`** and every item on an **OPEN QUESTIONS** line is a reviewable proposal awaiting owner sign-off — the **set-bonus numbers (§2.13)** and the **mutator list (§2.11)** explicitly so. Nothing in this block is implemented yet; each spec is written to be handed to an agent against the invariants: deterministic integer/`Fixed` math only, sim core engine-independent, director authority over anything competitively load-bearing, config hashed per [`05 §5.8`](05-data-model.md).

## 2.10 Score attack (canonical score formula)

**Motivation.** The game already produces several ad-hoc scores — the net view's per-player tracker, the results panel, the profile's score-threshold achievements — and the ship track adds Steam leaderboards ([`06` M4](06-roadmap-risks-testing.md), [`07 §7.7`](07-steamworks-integration.md)). Without one canonical function these will drift into incomparable numbers. This section defines **the** score: one deterministic integer formula shared by local records, the results screen, and future Steam boards, so a score is the same number wherever it is displayed and can be **re-derived by a replay verifier**.

**The formula [design choice].** All arithmetic is `i64`, saturating, integer-only (`ilog2` = floor of log₂ over `u64`); term weights are named constants and every one is **`TUNABLE-PENDING-SWEEP`**:

```
SCORE = ((T_surv + T_dmg + T_boss + T_place + T_last) × DIFF_NUM) / DIFF_DEN

T_surv  = survival_ticks / SCORE_SURV_DIV          // SCORE_SURV_DIV = 3  → 10 pts per second survived
T_dmg   = SCORE_DMG_W × ilog2(1 + damage_dealt)    // SCORE_DMG_W  = 250 → log-compressed, like the
                                                   //   tracker's display score (match.gd `_score`),
                                                   //   but INTEGER (ilog2, not float log10)
T_boss  = SCORE_BOSS  if this arena killed the boss, else 0        // SCORE_BOSS = 5000
T_place = SCORE_PLACE_W × (lobby_size − placement)  // SCORE_PLACE_W = 250; MP only (lobby_size 1 ⇒ 0)
T_last  = SCORE_LAST_STAND  if Last Stand, else 0   // SCORE_LAST_STAND = 250 — the source's "+250
                                                   //   score" prestige number, carried forward (§2.7)
```

- `survival_ticks` = the player's **death tick**, or the match-resolution tick for a tank still alive at the end (§2.7). A full run to the boss is 27000+ ticks ⇒ `T_surv` ≈ 9000+, so survival stays the dominant term — scoring mirrors pillar 4 (*you race the lobby*).
- `damage_dealt` = `total_damage_dealt` from the arena stats. Log compression keeps a 10⁶-damage run (~4 750) and a 10¹²-damage snowball (~9 750) on the same axis; damage differentiates but never outruns survival.
- **Difficulty interaction**: `DIFF_NUM/DIFF_DEN` is a reserved slot for a future difficulty ladder over the `RAMP_BASE` dial ([`06` §M4 balance pass](06-roadmap-risks-testing.md)). The shipped tune **is** Normal = `1/1`, and **only Normal-difficulty runs are board-eligible**. Game speed (30/45/60/90 tps) is cadence-only (§2.3) and never touches the score.
- **Exclusions**: mutated runs (§2.11) never enter the standard boards — they key their own board namespaces. Loadout runs (§2.12) are proposed board-eligible (they are sidegrades) — flagged below.

**Determinism & authority.** The score is computed in the **render/meta layer** — from `stats_record()` (`godot/sim_view.gd`) plus the director-owned placement/death-tick facts — **never inside `ArenaState`**: it is not part of `state_checksum`, not in snapshots, and never crosses the wire during a match. (The in-match tracker may keep its cosmetic float display; the canonical formula owns records/results/boards.) Two small stats-surface additions are required: the survival tick and a `boss_killed` flag exposed alongside the existing stats — render-only telemetry, same class as `bought_attack_mask`. **Anti-cheat is the replay-verified submission story of [`07 §7.6`](07-steamworks-integration.md)**: a board write ships `(seed, input_log, RunConfig)`; the verifier re-sims, recomputes the stats, recomputes this formula, and rejects a submission whose claimed score disagrees. The formula being pure-integer over re-simmable facts is what makes that check exact.

**Test & balance hook.** Weight sanity is checked with the existing 80-seed bot sweep ([`06 §6.1` M4 balance pass](06-roadmap-risks-testing.md)): across the four archetypes, score must rank runs in placement-then-survival order (no archetype may out-score a longer-surviving one on damage alone) before any board ships.

**OPEN QUESTIONS (owner sign-off needed):** the five weights (`3 / 250 / 5000 / 250 / 250`); whether loadout runs (§2.12) share the standard board; whether `T_place`/`T_last` should also pay out in single-player vs. bots (proposal: no — SP boards rank on `T_surv+T_dmg+T_boss` only).

## 2.11 Run mutators

**Motivation.** The randomized shop gives run-to-run variety, but every run today plays the same *ruleset*. Mutators add owner-curated, deterministic rule twists — replayability for veterans and a lobby-level "house rules" knob for multiplayer — without touching the tuned standard envelope, because mutated runs are fenced off from standard records.

**Spec.** A mutator is a **small, fixed set of deterministic tuning deltas** selected **before** the run and applied **once, at `ArenaState` construction** — content-table ratio deltas and constructor fields, all integer/`(num,den)` `Fixed` ratios, never mid-match, never floats. The active set is a bitmask (`RunConfig.mutators`, [`05 §5.8`](05-data-model.md)) in the stable order of this table. Proposed initial set — **every delta `TUNABLE-PENDING-SWEEP`**, the list itself pending owner sign-off:

| bit | id | Name | Deltas (exact, on named sim constants/fields) |
| --- | --- | --- | --- |
| 0 | `m_glass_cannon` | **Glass Cannon** | enemy HP ×1/2 · tank max HP ×1/2 |
| 1 | `m_swarm` | **Swarm** | wave `cadence_ticks` ×1/2 (double spawn rate) · enemy HP ×3/5 |
| 2 | `m_barren` | **Barren Market** | shop rarity draw weights: Uncommon/Rare/Epic → 0 (Common-only offers; Black Market unavailable) |
| 3 | `m_no_reroll` | **No Reroll** | `reroll_count_remaining` 5 → 0 · gold rerolls disabled (reroll input = deterministic no-op, like the post-boss shop) |
| 4 | `m_overclock` | **Overclock** | all weapon `cooldown_ticks` ×1/2 (floor 1 tick; includes fixed-rate weapons — this scales the *data*, not +Attack Speed, so the N/A rule of §2.5 is untouched) · enemy HP ×3/2 |
| 5 | `m_iron_tank` | **Iron Tank** | `Clear` disabled (input = deterministic no-op) · tank max HP ×2. ⚠ The boss is then unkillable (§2.6): the boss phase becomes a pure swift-end survival race; match still resolves by elimination order |
| 6 | `m_famine` | **Famine** | base passive income ×1/2 · bounty ×3/2 (a *config* delta on the un-multiplied base — the source's "income multipliers touch only bonus income" scoping rule of §2.4 still holds in-run) |
| 7 | `m_blitz` | **Blitz** | ramp grace 2 min → 0 (`RAMP_GRACE_MIN` = 0; the +25%/min ramp starts at 0:00) |

- **Records**: mutated runs are **excluded from standard records and boards**; each distinct `mutators` bitmask keys its **own** record/board namespace ([`05 §5.8`](05-data-model.md)). Scores inside a mutator board still use the §2.10 formula.
- **Multiplayer**: the mutator set is **host-set in the lobby, visible to every member before ready-up**, identical for all players (starts stay symmetric — mutators are ruleset, not per-player handicaps), and fixed at `MatchStart`.

**Determinism & authority.** Mutators are pre-run config, not runtime state: after construction the sim has no "mutator code path", just different numbers — so determinism, snapshots, and the checksum are untouched by construction. The director owns the authoritative set; **the bitmask is folded into the join-gate hash** (`ruleset_hash`, [`05 §5.8`](05-data-model.md)), so a client with a mismatched mutator config fails the same gate that catches content drift ([`04 §4.4.1`](04-protocol-and-messages.md)) and can never enter the lobby's match.

**Test & balance hook.** Each shipped mutator gets a determinism run (same seed+inputs+bitmask ⇒ identical checksum trace) and an 80-seed bot sweep to confirm it is *playable* (win rate within a wide 5–50% sanity band, not the standard 17–23% target — mutators may be deliberately lopsided).

**OPEN QUESTIONS (owner sign-off needed):** the mutator list itself (which of the 8 ship; any additions); every delta ratio; whether mutator sets may combine freely or ship as curated single picks; the Iron Tank boss-unkillable resolution rule.

## 2.12 Unlock-gated starting loadouts (single-player only)

**Motivation.** The achievement/skin meta (`godot/profile.gd`) currently pays out only cosmetics. Loadouts give the challenge achievements a small *gameplay* payoff — an alternate opening that skips the first shop lottery for a chosen weapon class — while staying **sidegrades**: the point is a different first minute, not a stronger one.

**Spec.** A loadout = **one starting weapon + a gold delta**, passed as single-player constructor parameters. Proposed set — each maps a **real, existing** achievement id from `profile.gd` to that attack class's **Common (500 g)** weapon, with the gold delta pre-paying the exact cost (start gold 500 → 0), so a loadout is gold-neutral and the "benefit" is being armed from tick 0 plus certainty of the first buy:

| Loadout | Unlock (achievement id — `profile.gd`) | Start weapon (Common, 500 g) | Gold delta |
| --- | --- | --- | --- |
| Marksman's Start | `purist_single` — *One-Trick Sniper* | Bow | −500 |
| Bombardier's Start | `purist_splash` — *Boom Enthusiast* | Mortar Launcher | −500 |
| Fusillade Start | `purist_barrage` — *Spray 'n' Pray* | Knives | −500 |
| Warden's Start | `purist_area` — *Aura Farmer* | Thornburst | −500 |
| Emberwake Start | `purist_wave` — *Wavy Gravy* | Immolation Aura | −500 |
| Ricochet Start | `purist_bounce` — *Ricochet Rascal* | Seeker Axe | −500 |

(Weapons referenced by catalog name, resolved to def indices at implementation time; each is the cheapest shipped Common of its attack class in `sim::content::WEAPONS`.)

**Principles [design choice]:**
- **SP-only.** Multiplayer starts stay **symmetric** — every tank opens identically; competitive integrity and the "meta is cosmetic in MP" principle (`profile.gd`'s header contract) are preserved. The MP constructor path simply never accepts a loadout parameter.
- **Power-neutral intent.** Every loadout is cost-exact (weapon granted, cost deducted); no loadout may ship with a net-positive gold or stat delta. Sidegrade, not head start.
- **Sim-constructor surface.** `ArenaState::new` gains optional `(start_weapon, gold_delta)` (the `RunConfig.loadout` index, [`05 §5.8`](05-data-model.md)); the granted weapon uses the normal purchase plumbing (it sets `bought_attack_mask`, counts as a bought weapon for achievements).
- **Records**: **proposed — allowed on standard records** (they are gold-neutral sidegrades, and excluding them would punish using the meta at all) — but the run's loadout id is always recorded in the replay header so the verifier re-sims the true start. **Flagged for owner** (see OPEN QUESTIONS).

**Determinism & authority.** Like mutators: pure pre-run config applied at construction; zero runtime branching, zero checksum impact by construction. SP-only means no director involvement — but the loadout id **must** ride in the replay header, or §2.10's verifier cannot reproduce the run.

**Test & balance hook.** A `balance_guards`-style check per loadout: the bot on each loadout must stay within a few points of the default start's win rate across the 80-seed sweep (proposal: ±5 pts of the 17–23% band, `TUNABLE-PENDING-SWEEP`) — proving "sidegrade" empirically, not rhetorically.

**OPEN QUESTIONS (owner sign-off needed):** standard-records eligibility (proposed: allowed); the six achievement→loadout pairings (alternatives: `no_economy`/`war_profiteer`-gated economy-flavored starts were considered and dropped for not fitting the one-weapon constructor shape); whether loadouts appear in the SP challenge picker UI or a separate pre-run menu.

## 2.13 Synergy set bonuses

**Motivation.** Build identity today comes from stacking scalers; the shop already nudges players toward damage-type/status families, but committing to a family pays out only linearly. Set bonuses add **threshold moments** — "one more Frost weapon completes the set" — that reward committed builds and make the arsenal legible as a *build*, not a pile. This is the most design-sensitive addition: it injects free power on top of a tuned envelope, so everything here is a proposal.

**Source-fidelity note.** The original had **scalers, not sets** — Frost/Fire strength upgrades, Combustion, Battle Fervor are all *purchased* modifiers. Set-collection bonuses are an **E-series original addition** and must stay labeled as such (code comment + `CREDITS.md`), per the [`09`](09-rebuild-plan.md) fidelity posture.

**Spec.** Six sets, keyed to **existing** damage-type/status families — membership is a pure predicate over shipped `WeaponDef` data (no new content tags), and every threshold effect **composes with an existing mechanic** (named per cell) rather than adding a new engine class. **All thresholds and magnitudes `TUNABLE-PENDING-SWEEP`; the whole table is pending owner sign-off.**

| Set | Membership predicate (existing data; family size) | Pieces T1 / T2 | T1 effect (existing hook) | T2 effect (existing hook) |
| --- | --- | --- | --- | --- |
| **Frost** | `on_hit.frost_stacks > 0` (9 weapons) | 3 / 6 | +1 Frost stack on every frost-applying hit (the `scale_on_hit` path, like the Poison/Stun scalers) | **Deep Freeze granted** (`GrantDeepFreeze` — the E3 opt-in effect, no purchase needed) |
| **Fire** | `on_hit.fire_stacks > 0` (12) | 3 / 6 | +25% Fire strength (`FireDamagePct`) | **+100% Combustion** (`CombustionPct`; a distinct multiplicative source, stacks with the Combustion upgrade) |
| **Poison** | `on_hit.poison_dps > 0` (6) | 2 / 4 | +25% Poison damage (`PoisonDamagePct`) | +25% damage vs Poisoned (`DamageVsPoisonedPct`) |
| **Piercing** | `damage_type == DMG_PIERCING` (20) | 3 / 6 | +15% Piercing damage (`DamageTypePct`) | Piercing hits add +1 **typed** vulnerability stack (the Thorn `VulnTypeOnHit` mechanic as a set rider) |
| **Siege** | `damage_type == DMG_SIEGE` (17) | 3 / 6 | +15% Siege damage (`DamageTypePct`) | Siege hits **Stun 0.25 s** (8 ticks, through the on-hit stun path — so "+% Stun Duration" scales it) |
| **Healing** | `WeaponDef::is_healing()` (7) | 2 / 4 | **+15% Healing** (`HealingPct`) | +25% Healing-Weapon damage (`HealingWeaponDamagePct`; distinct source from Battle Fervor) |

- **Piece counting [design choice — proposed: distinct]:** a "piece" is a **distinct owned `def_id`**; extra copies never add pieces (copies already pay out via "everything stacks" — sets reward *breadth* inside a family). A weapon counts toward **every** family it qualifies for (Frost Bomb is Frost *and* Piercing; Poison Bomb is Poison *and* Siege). Thresholds are per-family (2/4 for the two small families of 6–7 members; 3/6 elsewhere).
- **Set-bonus magnitudes are additive within the set-bonus source and multiplicative across sources**, exactly like any other modifier (§2.5) — no new stacking rule.
- **UI expectation:** the arsenal panel (the [`09 §9.4-P5`](09-rebuild-plan.md) weapon-detail extension) shows per-set piece counts and lit/unlit threshold effects; the shop card of a set-member weapon shows its family tag(s).

**Determinism & authority.** Set state is a **derived value**: recomputed in the purchase path (`buy` — weapons are never sold, so piece counts are monotonic) as a pure function of `weapons[]`, granting/revoking nothing retroactively. Effects land through the existing modifier aggregates (plus the existing `tank.deep_freeze` flag), so checksum/snapshot coverage is inherited from mechanics that already ride in them; any new derived field that becomes authoritative state enters `checksum()`/snapshot per the [`05 §5.6`](05-data-model.md) rules with one documented golden re-baseline. MP needs no new protocol: purchases are already ordered inputs; the shadow-sim derives identical set state.

**Test & balance hook.** The full 80-seed archetype sweep re-runs with sets enabled and must be re-dialed back into the 17–23% win band via `RAMP_BASE` (the designated dial) **before merge**; a guard test pins that a 6-piece Frost bot build actually receives Deep Freeze and that no set grants exceed their table values.

**OPEN QUESTIONS (owner sign-off needed):** every threshold and magnitude in the table (the explicit sign-off item); distinct-vs-copies piece counting; whether multi-family weapons should count everywhere (proposed: yes) or in one elected family; whether T2 Siege's stun needs a boss exemption (boss is `Clear`-only for *damage* — proposal: boss immune to set-rider stun too); whether sets apply in multiplayer at launch or SP-first.
