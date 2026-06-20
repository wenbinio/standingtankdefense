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

> **Integer/fixed-point everywhere in the hot path.** Damage, HP, gold reach very large values and must be bit-stable across machines for cheap checksums. Use `i64` for HP/gold/damage and a `Fixed` (e.g. 32.32) type for rates/multipliers. No raw floats in anything that feeds the `state_checksum`.

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
  id: string                       // "felorc_grunt", "firebreather", "samwise"
  base_hp: i64
  move_speed: Fixed
  contact_damage: i64
  bounty: u32
  armor_class: u8                  // indexes the damage matrix (war3mapMisc)
  archetype: Swarm|Tank|Fast|Caster|Ranged|Splitter|Boss
  abilities: EnemyAbility[]        // rotating breath arcs, summons, etc.
  scales: bool                     // Samwise = false (fixed)
}

WaveTable {
  id: string                       // "wave_12"
  round: u16
  spawns: SpawnGroup[]             // { enemy_id, count, cadence_ticks, gate_tick }
}

ScalingCurve {
  step: base | post_10min | post_15min
  hp_mult: Fixed
  damage_mult: Fixed
}
```
Spawn *positions/timing jitter* come from the per-player `spawn` RNG stream (5.6); composition is shared. Samwise ignores `ScalingCurve` (`scales=false`).

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
A rolling 32/64-bit hash over the **authoritative-relevant** fields each tick-batch: `tick`, `tank` (hp/shield/cooldowns), per-enemy `(entity_id, hp, pos_fixed, status_stacks)`, `economy`, and `RngCursors`. Excludes render-only state. This is what `Digest` reports and the shadow-sim compares (see [`04 §4.4.4`](04-protocol-and-messages.md)).

## 5.7 Content authoring

Weapons/modifiers/enemies/waves are **data**, not code — authored as files (JSON/TOML) validated against the schemas above and hashed into the `content_hash` used at join time ([`04 §4.4.1`](04-protocol-and-messages.md)). The extracted `research/tower-survivors-map/parsed/catalog.json` is the seed corpus for the v1 content set; Appendix A is its human-readable form.
