# 05 — Data Model, Content Schemas & Determinism

The simulation entity model, the **content-data schemas** (weapons, modifiers, enemies, waves) derived from the real Tower Survivors catalog, and the **RNG/determinism rules** that make every arena reproducible. Field shapes are normative; exact numbers come from Appendix A and are tuning data, not architecture.

## 5.1 Authoritative arena state

Everything the shadow-sim and client must agree on. Anything *not* here (particles, sound, camera) is render-only and never networked or checksummed.

```
ArenaState {
  tick: u32
  tank: Tank
  weapons: Weapon[]                 // owned instances, stable order by instance_id
  modifiers: ModifierStacks         // aggregated +%/flat by scope (see 5.3)
  economy: Economy
  enemies: Enemy[]                  // sorted by entity_id for stable iteration
  projectiles: Projectile[]         // sorted by entity_id
  hazards: Hazard[]                 // persistent areas: burning oil, land mines, auras
  shop: ShopState                   // stateful offer machine (5.5)
  rng: RngCursors                   // per-purpose stream positions (5.6)
  next_entity_id: u32
}
```

```
Tank {
  hp: i64                 // integer HP (values reach millions; see Appendix A)
  max_hp: i64
  armor: i32
  hp_regen: Fixed         // fixed-point, with retroactive multiplier applied
  dodge_pct: Fixed        // diminishing returns
  mana_shield: { pool: i64, max: i64, active: bool, regen: Fixed }
  clear_cooldown_end: u32 // tick
}
```

> **Integer/fixed-point everywhere in the hot path.** Damage, HP, gold reach very large values and must be bit-stable across machines for cheap checksums. Use `i64` for HP/gold/damage and a `Fixed` type for rates/multipliers — implemented as **Q47.16** (an `i64` with 16 fractional bits, saturating arithmetic) in `sim-core/crates/determinism`. No raw floats in anything that feeds the `state_checksum`.

The sketch above is the core; as shipped, `ArenaState` carries a handful of additional authoritative fields (all snapshotted and checksummed — the versioned wire form is `SNAPSHOT_VERSION` **24**):

- **`difficulty`** — the single-player preset (Easy / Normal / Hard) fixed **once at construction**; it selects the arena's starting numbers and stays constant thereafter. SP-only: the MP/director construction path is hard-coded to Normal, so it can never make two peers' arenas disagree.
- **Per-source damage ledger** — `damage_by_weapon` (a stable `entity_id → i64` map crediting damage to the weapon source that caused it) plus **source-attribution stamps** carried on indirect damage so it credits the right weapon: `poison_src` on a poisoned enemy, `Hazard.source`, and `Minion.source`. This is **behavior-neutral bookkeeping** — it changes no outcome — but it lives in the checksum/snapshot so the client and shadow-sim stay bit-identical and the DPS meter reads the same numbers everywhere.
- **`pending_black_market`** — a flag marking a **held, unredeemed** Black Market voucher (a deferred player choice: an Uncommon weapon-or-Spikes upgrade), true until the pick is spent.
- **`PendingPerk.scope`** — a source-derived constraint on a pending free perk: which offer *kinds* (e.g. weapon, or weapon-or-Spikes) it is allowed to consume.
- **`healing_weapon_healthy_dmg`** — part of the aggregated modifier state: a healing-weapon-scoped damage bonus that is active only under its condition (Battle Fervor, while at full health).
- **`treasure_pool`** (on `Economy`) — a held Magic Treasure gold reserve awaiting its payout tick.

## 5.2 Weapon schema

Directly mirrors the extracted weapon stat blocks (Appendix A.2).

```
WeaponDef {
  id: string                       // "wpn_frost_bow"
  display_name: string
  rarity: Common|Uncommon|Rare|Epic
  cost: u32                        // 500 (common) … ~5000 (epic)
  damage_types: DamageType[]       // base ∈ {Normal,Piercing,Magic,Siege,Chaos}
  status_flavors: Status[]         // ∈ {Poison,Frost,Fire,Spikes}  (may be empty)
  attack: AttackType               // tagged union (5.2.1)
  base_damage: i64
  attack_cooldown: Fixed | "N/A"   // "N/A" ⇒ ignores +Attack Speed
  range: 300|600|900|1200
  ability: WeaponAbility?          // Stun/Root/Knockback/Freeze/Drain/Summon/Heal/VulnStack/...
}
```

### 5.2.1 AttackType (tagged union)
```
AttackType =
  | SingleTarget
  | Splash      { radius: u16 }
  | Bounce      { targets: u8 }
  | Barrage     { targets: u8 }
  | Wave        { extra_range: u16, rotation: CW|CCW|none }
  | Area        { radius: u16 | "enemies_in_range" }
```

### 5.2.2 Owned instance
```
Weapon {
  instance_id: u32
  def_id: string
  fire_cooldown_end: u32           // next-fire tick
  ability_state: {...}             // per-weapon (e.g. Focusfire ramp, drain target id)
}
```
Multiple copies of the same `def_id` stack as separate instances (the source's "everything stacks").

## 5.3 Modifier / stacking model

The source rule is **additive within a source, multiplicative across sources**. We aggregate modifiers into scoped buckets and apply them in a fixed order.

```
Modifier {
  id: string
  rarity: Rarity
  cost: u32
  effects: Effect[]
}
Effect {
  scope: Scope                     // 5.3.1
  stat: Stat                       // Damage|AttackSpeed|Range|MaxHP|HPRegen|Armor|Dodge|
                                   //   ManaShield|Income|Bounty|StunDuration|Healing|...
  op: Add | Mul                    // Add → flat/additive bucket; Mul → its own factor
  value: Fixed
  condition: Condition?            // e.g. target_is(Stunned|Poisoned), per_round_ramp, after_15min_no_stack
}
```

### 5.3.1 Scope (what a modifier targets)
```
Scope =
  | AllWeapons
  | DamageType(Normal|Piercing|Magic|Siege|Chaos)
  | StatusFlavor(Poison|Frost|Fire|Spikes)
  | AttackClass(SingleTarget|Splash|Bounce|Barrage|Wave|Area)
  | RangeTier(300|600|900|1200)
  | Rarity(Common|Uncommon|Rare|Epic)
  | EnemyState(Stunned|Poisoned|Frozen|Burning)
  | Global                          // HP/economy/etc.
```

**Damage resolution order** (deterministic, documented so client == shadow):
```
final = base_damage
      × (1 + Σ additive% in every matching scope)        // additive within each source-type
      × Π (1 + mul% per distinct multiplicative source)  // multiplicative across sources
      × armor_matrix[damage_type][target_armor_class]
      × enemy_state_vulnerability (Fire stacks, Freeze, vuln-stacks)
```

## 5.4 Enemy & wave schemas

```
EnemyDef {
  id: string                       // "squeakzilla", "nope_rope", "the_hippocrate"
  base_hp: i64
  move_speed: Fixed
  contact_damage: i64
  bounty: u32
  armor_class: u8                  // indexes the damage matrix (war3mapMisc)
  archetype: Swarm|Tank|Fast|Caster|Ranged|Splitter|Boss
  abilities: EnemyAbility[]        // standoff ranged attacks, summons, etc.
  scales: bool                     // the boss (The Hippocrate) = false (fixed)
}

WaveTable {
  id: string                       // "wave_12"
  round: u16
  spawns: SpawnGroup[]             // { enemy_id, count, cadence_ticks, gate_tick }
}

ScalingCurve {
  minute: u8                       // whole-minute compound point k (piecewise-linear between)
                                   //   the SOURCE arc: ×RAMP_BASE/min after a 2-min grace,
                                   //   +20% step at 10:00 (SCALE_STEP_TICK), ×SWIFT_END/min
                                   //   "swift end" from 15:00 — sim::content::enemy_hp_mult
  hp_mult: Fixed
  damage_mult: Fixed
}
```
Spawn *positions/timing jitter* come from the per-player `spawn` RNG stream (5.6); composition is shared. The boss (**The Hippocrate**, spawned at `BOSS_SPAWN_TICK` = 27000 — 15 min, the source match length) ignores `ScalingCurve` (`scales=false` — fixed HP *and* fixed contact damage). From the same tick the shop closes (offers clear; buy/reroll are deterministic no-ops) and per-round ramp modifiers stop accruing (`modifiers::apply_ramps`) — all pure functions of the tick, so nothing new crosses the wire.

## 5.5 Shop / offer state machine (stateful — must replay exactly)

Targeted items (Black Market, Multiplication Gems, copy effects) make offers **order-dependent**, so the shop is modeled as an explicit state machine advanced only by ordered inputs.

```
ShopState {
  round: u16
  shop_seq: u32
  offers: Offer[]                  // current slots (server-gated, see 04 ShopOffer)
  reroll_count_remaining: u8       // starts 5
  reroll_cost: u32                 // escalates per use
  pending_copies: { rarity: Rarity, count: u8 }[]   // Multiplication Gems / copy effects
  black_market_pick: bool          // awaiting a specific-weapon choice
}
```
Rules captured from the source:
- A new shop is generated **each round** (drives all "every 30s" ramps).
- **Reroll** consumes `reroll_count_remaining` (or charges escalating gold) and draws a new `offers[]` from the `shop` RNG stream.
- **Multiplication Gems / copies** mutate `pending_copies`, which the *next* matching purchase consumes → grants extra weapon/upgrade instances.
- **Black Market** sets `black_market_pick`, opening a chooser for a specific Uncommon weapon.

Because all of this is deterministic from `(shop seed, ordered inputs)`, a reconnecting client rebuilds identical offers — and a player cannot save-scum.

## 5.6 RNG streams & determinism

(Networked view in [`03 §3.6`](03-network-architecture.md); design rationale in [`02 §2.8`](02-game-design.md).)

```
RngCursors { spawn: u64, targeting: u64, shop: u64, reroll: u64, proc: u64 }
```
- **PRNG**: a fast, well-distributed, *portable* generator — **PCG32** or **xoshiro256\*\***. No platform `rand()`, no language-default RNG (non-portable).
- **Seeding**: `seed(purpose) = splitmix64( hash(master_seed, player_id, purpose, round) )`. Independent purposes ⇒ a reroll never perturbs spawns; a crit roll never perturbs the shop. Keeps replays stable and bugs local.
- **Cursors are part of `ArenaState`** and ride in snapshots, so reconnect/correction restores RNG position exactly.
- **Purposes**:
  - `spawn` — wave spawn jitter/positions.
  - `targeting` — random target selection (the source's "attack at random").
  - `shop` — offer generation & rerolls.
  - `reroll` — (kept separate so reroll *count/cost* logic can't desync offer draws).
  - `proc` — crits, the 5% bounty proc, Fire-explosion, dodge rolls.

### 5.6.1 Determinism rules (the banned-ops list)
To keep client == shadow so checksums match and corrections stay rare:
- **Fixed 30 Hz timestep.** Never advance the sim by wall-clock delta.
- **Integer / fixed-point math** for everything that feeds `state_checksum`. Floats only in render code.
- **Stable iteration order**: iterate `enemies`/`projectiles` by `entity_id`; never iterate a hashmap in hash order.
- **Deterministic PRNG only**, pulled from the correct purpose stream; never `system_rand`, never time-seeded.
- **No wall-clock, no thread/race-dependent state, no float-keyed containers** in the sim.
- **Order-of-operations is part of the spec**: damage resolution (5.3), status application, and death checks happen in a fixed documented order each tick.

### 5.6.2 `state_checksum`
A rolling 32/64-bit hash over the **authoritative-relevant** fields each tick-batch: `tick`, `tank` (hp/shield/cooldowns), per-enemy `(entity_id, hp, pos_fixed, status_stacks)`, `economy`, `RngCursors`, and the additional authoritative fields listed in §5.1 (the SP `difficulty` preset, the per-source damage ledger and its attribution stamps, held-choice flags, and the conditional/held modifier and economy state). The damage ledger is **behavior-neutral bookkeeping**, but it is checksummed anyway so client and shadow stay bit-identical. Excludes render-only state. This is what `Digest` reports and the shadow-sim compares (see [`04 §4.4.4`](04-protocol-and-messages.md)).

## 5.7 Content authoring

Weapons/modifiers/enemies/waves are **data**, not code — declarative definitions validated against the schemas above and hashed into the `content_hash` used at join time ([`04 §4.4.1`](04-protocol-and-messages.md)). The extracted `research/tower-survivors-map/parsed/catalog.json` is the seed corpus for the v1 content set; Appendix A is its human-readable form.

> **Amended 2026-07** (flagged locked-decision drift, approved in [`09 §9.2`](09-rebuild-plan.md)): as shipped, the content catalog is **compiled Rust** — static `WeaponDef`/`ModifierDef`/`EnemyDef` tables in `sim-core/crates/sim/src/content.rs` (96 weapons / 110 modifiers / a 12-entry enemy roster — counts gated against the tables by `crates/sim/tests/doc_sync.rs`), bootstrapped from `catalog.json` by `gen_catalog.py` and then hand-curated. The **schema discipline above is retained** — the Rust structs mirror these shapes field-for-field, and drift is caught by doc-sync tests — but the "authored as JSON/TOML files" clause is not how the game is built. This has served the balance/test workflow well and is the accepted state. Two consequences:
> - **`content_hash` is currently a placeholder** (`DEMO_CONTENT_HASH = 0xC0DE_C0DE` in the GDExtension), which defeats the version-drift gate the join handshake exists for. A **real `content_hash`** — a deterministic serialization of the compiled tables, hashed — is planned in the ship track ([`09 §9.4-P6`](09-rebuild-plan.md)).
> - **Externalized JSON/TOML data files remain a possible future** (e.g. for modding or hot-tuning), **not a requirement**; if adopted, they load into the same schema-shaped tables and feed the same hash.

## 5.8 Run-config surface — mutators & starting loadouts (proposed; docs/02 §2.11–2.12)

Config-surface sketch for two **post-source additions** (owner-approved concepts, numbers pending sign-off — see [`02 §2.10–2.13`](02-game-design.md)). Both are **pre-run configuration applied once at `ArenaState` construction** — after construction the sim has no config code path, just different numbers — so determinism, snapshots, and `state_checksum` are untouched by construction.

```
RunConfig {
  mutators: u32       // bitmask; bit i = row i of the mutator table (02 §2.11, stable order).
                      //   0 = the standard ruleset. Deltas are integer/(num,den) Fixed ratios
                      //   over named content constants — never floats, never mid-match.
  loadout: u8         // SP ONLY. 0 = default start; else 1-based index into the loadout
                      //   table (02 §2.12) ⇒ constructor params (start_weapon, gold_delta).
}
```

**Hash / join-gate inclusion rule:**
- The join-time comparison ([`04 §4.4.1`](04-protocol-and-messages.md)) is over `ruleset_hash = H(content_hash ‖ mutators)`: the active mutator bitmask is appended to the content-hash preimage, so a lobby member with a mismatched mutator config fails the **same gate** that catches content/version drift and can never join the match. (Until the real `content_hash` lands — §5.7 — the placeholder is what gets extended.)
- `loadout` is **excluded from the MP gate**: it is single-player-only and the MP constructor path never accepts it, so it can't desync anything. It **is included in the replay header** alongside seed + input log, so score verification ([`07 §7.6`](07-steamworks-integration.md), [`02 §2.10`](02-game-design.md)) re-sims the exact run.
- **Record/board keying:** standard records require `mutators == 0`; each distinct nonzero bitmask keys its own record/board namespace. Loadout runs are proposed record-eligible (sidegrades) — an open owner decision in [`02 §2.12`](02-game-design.md).
