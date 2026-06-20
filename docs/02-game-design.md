# 02 — Game Design

Design spec for **Standing Tank Defense**. This is deliberately a "GDD‑lite": enough to drive the data model and the network design, not a balance bible. Numbers tagged _(inferred)_ are starting points adapted from the WC3 source and should be tuned in playtest. Everything here is built so the simulation is **deterministic and shardable** — see [`03-network-architecture.md`](03-network-architecture.md).

## 2.1 Design pillars

1. **One tank, no movement, no aiming.** The only verbs are *buy* and *upgrade*. All skill is in build decisions under time pressure. (One exception: a single manual ability — see *Clear*.)
2. **Everything stacks.** Power comes from layering weapons and multiplicative modifiers, not from a tech tree you climb and leave behind.
3. **Randomized offers, meaningful choices.** Each decision point presents a random subset of the catalog; scarcity + rarity make builds emergent and replayable.
4. **You race the lobby, not fight it.** Players never touch each other's arenas. The competition is *relative survival time*. This is a pillar **and** the reason the netcode can be cheap.
5. **Readable escalation.** Difficulty ramps on a clear clock so a player always knows roughly how close the next breakpoint is.

## 2.2 The tank (player avatar)

- **Stationary.** Fixed position in the center (or rear) of a personal arena.
- **Has HP.** Enemies that reach the tank (or its objective) deal damage; at 0 HP the tank dies and the player is eliminated (and gets a placement).
- **Auto‑fires.** All equipped weapons fire automatically on their own cadence at targets selected by their targeting rule.
- **One manual ability — `Clear`.** A long‑cooldown panic/burst button (themed as a cannon "clear"). It is also the **only** thing that can damage the end boss (carried from the source's Samwise rule). Manual `Clear` activation is the *one* real‑time player input that affects combat timing, so it is a first‑class networked input (see protocol doc). Everything else is a menu decision.

## 2.3 Match structure & clock

| Phase | Duration _(inferred)_ | Notes |
| --- | --- | --- |
| Lobby | until ready/countdown | Server assigns slots + master seed |
| Countdown | ~5 s | Time‑sync converges here |
| Rounds 1–40 | ~30 s each → ~20 min | Each round = one wave + a shop/offer window |
| Boss | after round 40 | **Boss** spawns; only `Clear` damages it |
| Resolution | — | Placement + Last Stand awarded |

- The **round number and global clock are server‑authoritative** and the same for everyone (so "wave N" is comparable across the leaderboard).
- Within a round, each player's arena runs **independently** — your wave‑12 swarm is the same *composition* as everyone else's wave‑12 swarm (same spawn table) but seeded per‑player so individual spawn jitter differs and can't be copied.

## 2.4 Economy

Two income sources, deliberately scoped so multipliers don't trivially compound:

1. **Passive income** — a flat gold/second that rises slightly per round. *Income multipliers do NOT apply to this* (carried from the source rule).
2. **Bounty income** — gold per kill, scaled by enemy tier. *Income multipliers DO apply here* (e.g. Golden Medallion's +100% bounty).
3. **Round bonus** — a lump sum at round end (some economy upgrades add to this, e.g. Gold Delivery's +1000/round).

Gold is spent in the **offer window**: a randomized set of purchasable cards (new weapons, category modifiers, economy upgrades) drawn from the catalog by rarity weighting.

## 2.5 Content model (weapons / modifiers / rarities)

This is summarized here and formalized as schemas in [`05-data-model.md`](05-data-model.md).

### Weapons (categories)
Each weapon is an auto‑firing emitter with: damage, fire interval, range, projectile/AoE behavior, and a **targeting rule**. Starting categories (adapted from source):

- **Single‑target** — high per‑hit, picks one target.
- **Area** — AoE on hit/impact.
- **Poison / DoT** — applies stacking damage‑over‑time.
- **Multifire** — fires at multiple targets / extra projectiles.
- **Focusfire** — ramps damage the longer it hits the same target.
- **Bombardment** — periodic large AoE strikes at range.
- **Spikes** — short‑range retaliation / contact damage near the tank.

**Targeting is randomized** (no armor‑type preference). Random target selection is an explicit RNG draw and therefore uses a dedicated deterministic RNG stream (see §2.8 and the data‑model doc).

### Modifiers (the "stacking" engine)
- **Category modifiers** — e.g. *Areafire*: +% damage and +% AoE to all Area weapons. The general form is `{scope: category, stat: X, op: add|mul, value: v}`.
- **Global modifiers** — affect all weapons (e.g. +attack speed, +range).
- **Economy modifiers** — *Gold Delivery* (+flat/sec, +flat/round), *Golden Medallion* (+% bounty, proc chance for extra bounty).
- **Defensive modifiers** — +max HP, +regen, damage reduction, slow auras.

### Rarity / quality tiers
Offers are drawn with rarity weighting: **Common → Rare → Epic** (e.g. Epic‑quality 5000g upgrades like Multifire/Focusfire/Bombardment from the source). Rarity affects draw weight, cost, and magnitude.

## 2.6 Enemies & waves

- Enemies are simple: HP, move speed, contact damage, bounty, and an archetype (swarmer, tank, fast, splitter, etc.). Late game adds **scaling multipliers** to base HP/damage per round.
- A **wave table** per round defines spawn composition, counts, and cadence. The same table is used by all players in a given round; the **per‑player seed** decides spawn timing/positions so arenas feel individual and can't be mirrored.
- **Boss** (after round 40): a single high‑HP unit, immune to normal weapons, killable only by `Clear`. Surviving it (or outliving the lobby) is the terminal event.

## 2.7 Win / lose & placement (last man standing)

- A player is **eliminated** when their tank reaches 0 HP. Their **death tick** (server‑authoritative) is recorded.
- **Placement = reverse order of elimination.** Last to die = 1st place.
- **Win** = finishing in the **surviving (top) half** of the lobby; **lose** = bottom half. Sole survivor additionally earns a **Last Stand**.
- Ties / simultaneous deaths are broken by a deterministic tiebreak (e.g. higher wave reached, then more damage dealt, then lower player slot) so placement is fully ordered without a coin flip.
- The match **ends** when ≤1 player remains *or* the boss is resolved; remaining survivors are placed by current survival metric.

Because placement is just "an ordering of server‑confirmed death ticks," the only thing the network layer must get exactly right competitively is **who died when** — a tiny, low‑rate fact. That's a deliberate design choice that keeps the netcode honest.

## 2.8 Randomization & determinism (design‑level)

The game is "randomized," but for a fair, cheat‑resistant, reconnectable multiplayer game, randomness must be **deterministic and server‑owned**:

- The server issues a **master seed** per match and derives **per‑player, per‑purpose RNG streams** (spawn stream, offer/loot stream, targeting stream, proc stream). See [`05-data-model.md`](05-data-model.md) §RNG.
- Given a player's seed + their ordered input log, their entire arena is **reproducible** — this is what makes reconnection and server‑side validation possible.
- Players cannot "reroll" loot by save‑scumming because the offer stream is server‑seeded and advances deterministically.

## 2.9 Out of scope for v1

- Co‑op / shared‑arena mode (the architecture *can* grow into it; see [03 §9](03-network-architecture.md)).
- Cosmetics/meta‑progression economy.
- Cross‑arena interaction ("send a creep to a rival") — explicitly excluded in v1 because it would couple the otherwise‑independent sims; noted as a future networking trade‑off, not a v1 feature.
