# 02 — Game Design

Design spec for **Standing Tank Defense**, grounded in the extracted Tower Survivors v1.58 data (see [`01-source-analysis.md`](01-source-analysis.md) and [`appendix-A-map-extraction.md`](appendix-A-map-extraction.md)). This is a "GDD-lite": enough to drive the data model and the network design. Every system here is built to stay **deterministic and shardable** — see [`03-network-architecture.md`](03-network-architecture.md). Values taken directly from the map are stated as such; our own choices are marked **[design choice]**.

## 2.1 Design pillars

1. **One tank, no movement, no aiming.** The only verbs are *buy*, *upgrade*, *reroll*, and one manual ability. All skill is in build decisions under time pressure.
2. **Everything stacks, multiplicatively across sources.** The map's rule — *"different damage increases are multiplicative with each other"* — is the power-curve engine. Power comes from layering, not from a tech tree you outgrow.
3. **Randomized offers, meaningful choices.** Each round opens a new shop of random offers, weighted by rarity; rerolls and targeted shop items let you steer the randomness at a cost.
4. **You race the lobby, not fight it.** Players never touch each other's arenas. The competition is *relative survival time*. This is a pillar **and** the reason the netcode can be cheap.
5. **Readable escalation.** Difficulty ramps on a fixed clock with known breakpoints — as shipped, a stepped ramp every **3 minutes** (k = 1..10) across a **30-minute** arc, then the boss. (The WC3 source used a 15-min arc with 10/15-min breakpoints; see `docs/01`.)

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
| Rounds | **~30 s each**, a new **shop every round** | **30 min** of escalating waves |
| Scaling steps | every **3 min** (`RAMP_INTERVAL` = 5400 ticks; steps at 3, 6, …, 30 min) | enemy HP **and** damage step up ≈ +14% per interval, compounding smoothly to ≈ ×5.56 at the boss (`sim::content::enemy_hp_mult`) |
| Boss | at **30 min** (`BOSS_SPAWN_TICK` = 54000) | **The Hippocrate** spawns with a dense escort; **fixed** HP/damage (does not scale); immune to weapon fire — only `Clear` damages it (a multi-`Clear` race) |
| Resolution | — | placement + Last Stand awarded |

*(As shipped. The WC3 source ran ~15 min with 10/15-min scaling steps and its "Samwise" boss — `docs/01`; the standalone game stretched the arc to 30 min with an even 3-min ramp.)*

- **Round number and global clock are server-authoritative** and identical for everyone, so "round N" / "minute N" are comparable across the leaderboard.
- Within a round each arena runs **independently**: your round-N wave is the same *composition* as everyone else's round-N wave (shared wave table) but **per-player seeded**, so spawn timing/positions differ and can't be mirror-copied.
- **Game speed**: the source exposes Normal/Fast/Faster/Hyper, host-controlled. **[design choice]** We fix the *simulation* tick rate (for determinism) and treat "speed" purely as a host-set time-scale on the authoritative clock, applied identically everywhere — never a per-client setting.

## 2.4 Economy

Three income sources, scoped so multipliers don't trivially compound (matching the source's careful scoping):

1. **Passive Gold Income** — flat gold/sec, rising slightly per round. Income-% multipliers apply only to *bonus* income, not the base (source rule).
2. **Kill Bounty** — gold per kill by enemy tier; **+% Bounty** multipliers apply here, plus a fixed **5%-chance proc** for bonus bounty (source: "+200% Bounty Gold with 5% activation chance").
3. **Round bonus / meta-items** — lump sums and time-growing items (e.g. **Magic Treasure**: +250 gold now, value grows +2/sec).

Gold is spent in the **per-round shop**. **Rerolls** refresh the offers: you start with **5** rerolls, cost escalates per use, and some effects grant **Free Rerolls**.

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
- **Time/round modifiers** — "+X% every 30 s (each new shop)"; some sources "no longer stack after 15 minutes."

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
- **Scaling**: enemy base HP **and** damage step up on the 3-min ramp (steps at 3, 6, …, 30 min — §2.3).
- **Boss** (30 min): **The Hippocrate**, fixed stats, **immune to weapon fire — only `Clear` damages it** — and it grinds the tank with cadenced contact hits while a dense escort keeps spawning, so the climax is a sustained multi-`Clear` race. *(The source's boss, "Samwise", likewise had fixed/non-scaling stats; the Clear-only rule was a carried-over design assumption there, and is implemented fact here.)*

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
