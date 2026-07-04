//! M0 content catalog — a small, hand-picked slice of the extracted Tower
//! Survivors data (`docs/appendix-A-map-extraction.md`). Content is DATA, owned
//! centrally; behavior modules read it but do not edit it. Numbers are tuning,
//! not architecture.

use crate::state::Vec2;
use determinism::Fixed;

// Damage types (index into the armor matrix).
pub const DMG_NORMAL: u8 = 0;
pub const DMG_PIERCING: u8 = 1;
pub const DMG_MAGIC: u8 = 2;
pub const DMG_SIEGE: u8 = 3;
pub const DMG_CHAOS: u8 = 4;

/// Attack behavior for a weapon (`docs/05 §5.2.1`, adapted from the source map).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Attack {
    /// One traveling projectile to a single target.
    SingleTarget,
    /// One traveling projectile; splashes the given radius at impact.
    Splash(i64),
    /// `N` traveling projectiles, each to a distinct random in-range target.
    Barrage(u8),
    /// Instant area pulse: hits every enemy within the radius of the tank.
    Area(i64),
    /// Instant sweeping wave: hits every enemy within `range + extra` of the tank.
    Wave(i64),
    /// Instant chain: the random target plus the `N-1` nearest other enemies.
    Bounce(u8),
    // ---- COMBINED / EXOTIC ATTACK SHAPES (fidelity pass) ---------------------
    // The source pairs a secondary Splash with Bounce/Barrage ("Bounce (4
    // Targets) & Splash (150)"). Modeled as NEW variants (least invasive: no
    // field added to every existing weapon literal); they share the primary
    // shape's damage scope (`attack_scope_id`).
    /// Instant chain like [`Bounce`], where every chained hit ALSO splashes the
    /// given radius at the struck enemy's position. Each enemy is damaged at
    /// most once per attack (chain targets excluded from splashes).
    BounceSplash(u8, i64),
    /// `N` traveling projectiles like [`Barrage`], each splashing the given
    /// radius at impact.
    BarrageSplash(u8, i64),
    /// Sweeping wave like [`Wave`], but ROTATING: firing starts a sweep that
    /// rotates one angular sector per tick around the tank over
    /// `WAVE_SWEEP_TICKS`, hitting each enemy as the sector passes it
    /// (`combat::tick_sweeps`; integer binary-angle math). `bool` = clockwise
    /// (`false` = counterclockwise, the source's "rotating counterclockwise").
    WaveRotating(i64, bool),
}

/// A weapon's signature ability (`docs/05 §5.2`), executed at each hit/fire
/// site by `combat`. Pure data — the behavior lives in `combat`/`status`.
/// Most weapons carry `None` and fire plain damage; the catalog attaches one of
/// these to the ~25 "exotic" weapons whose defining behavior was previously a
/// comment. All magnitudes are integer (no floats) for determinism.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WeaponAbility {
    /// Pure damage; no extra effect (the default).
    None,
    /// Life-drain: heal the tank `per_hit` HP for each enemy this attack damages
    /// (the source's "Heal N health per enemy hit" / life-drain weapons). Routed
    /// through `Tank::heal`, so it is scaled by "+% Healing" and capped at max HP.
    LifeDrain { per_hit: i64 },
    /// Mana-drain: restore `per_hit` to the tank's Mana-Shield pool (capped at its
    /// max) for each enemy this attack damages (the source's mana-restore weapons).
    ManaDrain { per_hit: i64 },
    /// Knockback: shove each damaged enemy `dist` units directly away from the
    /// tank (Slap's "Knockback (300)").
    Knockback { dist: i64 },
    /// Root: immobilize each damaged enemy for `ticks` (Tangle's
    /// "Root" — modeled as a stun that does NOT scale with +% Stun Duration, so
    /// it is applied directly rather than through the on-hit stun path).
    Root { ticks: u32 },
    /// On-hit vulnerability: add `stacks` generic vulnerability stacks (each +1%
    /// damage taken) to every damaged enemy (the source's "Attacks increase
    /// damage taken by N%, stacking" weapons).
    VulnOnHit { stacks: u16 },
    /// Hazard placement: drop a persistent damaging area at the position of the
    /// first enemy this attack damages (Boom Bloom's mine field / scorched
    /// ground). The hazard pulses `dmg` to enemies within `radius` each tick for
    /// `ticks` ticks.
    Hazard { dmg: i64, radius: i64, ticks: u32 },
    /// Summon: when this weapon's hit KILLS an enemy, raise a temporary ally
    /// (Squirm's Larvae / Shroom Doom's Spores) from the corpse, up to a global
    /// cap. `kind` selects the sprite (0 larvae, 1 spores); `hp` and `damage`
    /// seed the minion. An Area weapon that wipes a pack raises several at once.
    Summon { kind: u8, hp: i64, damage: i64 },
    /// PER-ATTACK permanent-regen grant (the source's Healthstone: "Attacks
    /// grant +0.2 permanent HP Regen and 60 Instant HP Regen"). Executed once
    /// per FIRE (not per enemy hit), at the fire site: adds
    /// `regen_milli_per_s / 1000` HP-regen PER SECOND permanently to the tank
    /// (accumulated fixed-point in `Tank::regen_bonus_per_tick`, paid out with
    /// a fractional carry in `defense::regen`), plus an `instant` heal.
    RegenOnAttack {
        regen_milli_per_s: i64,
        instant: i64,
    },
    /// PER-ATTACK self-heal (the source's Holy Bolt: "Heal 80 health") — heals
    /// the tank `amount` once per FIRE, regardless of how many enemies the
    /// attack hits. Distinct from `LifeDrain` (which is per enemy damaged).
    HealOnAttack { amount: i64 },
    /// Miss-chance debuff (the source Ale Launcher's real mechanic: "Attacks
    /// reduce enemy chance to hit by 25% for 3 seconds"). Each damaged enemy
    /// gains the "obscured" status: for `ticks`, its attacks on the tank miss
    /// with `pct`% probability (rolled on `rng_proc`). Strongest pct wins;
    /// duration takes the longer remaining.
    Obscure { pct: i64, ticks: u32 },
    /// TYPED on-hit vulnerability (Thorn "+5% Normal damage taken, stacking";
    /// Liquid Fire's drench): adds `stacks` vulnerability stacks (each +1%
    /// damage taken) that apply ONLY to hits of `dmg_type`, alongside (and
    /// composing additively with) the generic `VulnOnHit` stacks.
    VulnTypeOnHit { dmg_type: u8, stacks: u16 },
}

impl WeaponAbility {
    /// Stable `(tag, a, b, c)` encoding for the checksum/snapshot of any
    /// `WeaponAbility` that rides in per-arena state (currently `Projectile`).
    pub fn words(self) -> (u8, i64, i64, i64) {
        match self {
            WeaponAbility::None => (0, 0, 0, 0),
            WeaponAbility::LifeDrain { per_hit } => (1, per_hit, 0, 0),
            WeaponAbility::ManaDrain { per_hit } => (2, per_hit, 0, 0),
            WeaponAbility::Knockback { dist } => (3, dist, 0, 0),
            WeaponAbility::Root { ticks } => (4, ticks as i64, 0, 0),
            WeaponAbility::VulnOnHit { stacks } => (5, stacks as i64, 0, 0),
            WeaponAbility::Hazard { dmg, radius, ticks } => (6, dmg, radius, ticks as i64),
            WeaponAbility::Summon { kind, hp, damage } => (7, kind as i64, hp, damage),
            WeaponAbility::RegenOnAttack {
                regen_milli_per_s,
                instant,
            } => (8, regen_milli_per_s, instant, 0),
            WeaponAbility::HealOnAttack { amount } => (9, amount, 0, 0),
            WeaponAbility::Obscure { pct, ticks } => (10, pct, ticks as i64, 0),
            WeaponAbility::VulnTypeOnHit { dmg_type, stacks } => {
                (11, dmg_type as i64, stacks as i64, 0)
            }
        }
    }
    /// Inverse of [`words`](Self::words).
    pub fn from_words(tag: u8, a: i64, b: i64, c: i64) -> Option<WeaponAbility> {
        Some(match tag {
            0 => WeaponAbility::None,
            1 => WeaponAbility::LifeDrain { per_hit: a },
            2 => WeaponAbility::ManaDrain { per_hit: a },
            3 => WeaponAbility::Knockback { dist: a },
            4 => WeaponAbility::Root { ticks: a as u32 },
            5 => WeaponAbility::VulnOnHit { stacks: a as u16 },
            6 => WeaponAbility::Hazard {
                dmg: a,
                radius: b,
                ticks: c as u32,
            },
            7 => WeaponAbility::Summon {
                kind: a as u8,
                hp: b,
                damage: c,
            },
            8 => WeaponAbility::RegenOnAttack {
                regen_milli_per_s: a,
                instant: b,
            },
            9 => WeaponAbility::HealOnAttack { amount: a },
            10 => WeaponAbility::Obscure {
                pct: a,
                ticks: b as u32,
            },
            11 => WeaponAbility::VulnTypeOnHit {
                dmg_type: a as u8,
                stacks: b as u16,
            },
            _ => return None,
        })
    }
}

/// Maximum frost stacks (each ~2% slow); referenced by the status system.
pub const FROST_MAX_STACKS: u8 = 25;

/// Status a weapon applies on hit. `NONE` = pure damage (most weapons).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct StatusOnHit {
    pub poison_dps: i64,
    pub poison_ticks: u32,
    pub frost_stacks: u8,
    pub fire_stacks: u16,
    pub stun_ticks: u32,
}

impl StatusOnHit {
    pub const NONE: StatusOnHit = StatusOnHit {
        poison_dps: 0,
        poison_ticks: 0,
        frost_stacks: 0,
        fire_stacks: 0,
        stun_ticks: 0,
    };
}

#[derive(Clone, Copy, Debug)]
pub struct WeaponDef {
    pub name: &'static str,
    pub rarity: u8, // 0 Common, 1 Uncommon, 2 Rare, 3 Epic
    pub cost: i64,
    pub damage: i64,
    pub damage_type: u8,
    pub attack: Attack,
    pub cooldown_ticks: u32,
    pub range: i64,      // integer units; compared via range*range
    pub proj_speed: i64, // units per tick
    /// Status applied to whatever this weapon hits.
    pub on_hit: StatusOnHit,
    /// The source's "Attack Cooldown: N/A" class (Frost/Fire waves): the weapon
    /// fires on its fixed internal period and is UNAFFECTED by "+% Attack
    /// Speed" (`docs/01 §1.3` — must-preserve). `false` for normal weapons.
    pub fixed_rate: bool,
    /// Once-per-round activation window (the source's Monsoon): `0` = a normal
    /// always-armed weapon; `N > 0` = the weapon is active only for the first
    /// `N` ticks of each round (firing on its own `cooldown_ticks` inside the
    /// window), then sleeps until the next round starts.
    pub round_burst_ticks: u32,
    /// Signature ability executed at each hit/fire site (`WeaponAbility::None`
    /// for the many pure-damage weapons).
    pub ability: WeaponAbility,
}

/// Coarse behavior class for an enemy (`docs/05 §5.4`). Drives flavor and a few
/// movement/attack defaults; render reads `EnemyDef::archetype` for telemetry but
/// maps SPRITES by the def index (`view::RenderEnemy::kind`), so adding rows here
/// does not change existing kinds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Archetype {
    /// Cheap, fast-spawning chaff (Doomduck / Squeakzilla).
    Swarm,
    /// Slow, high-HP, often armored (Fanged Death / Bonk).
    Tank,
    /// Fast melee rusher (Bacon / Honk).
    Fast,
    /// Stationary-ish ranged caster (Nope Rope).
    Caster,
    /// Approaches to a standoff range then attacks the tank at range
    /// (the Croak / Spicy / Popsicle family).
    Ranged,
    /// The end boss (The Hippocrate) — immune to weapon fire.
    Boss,
    /// Inert practice dummy (Dodo) — never moves, no contact.
    Inert,
}

/// An enemy's special ability (`docs/05 §5.4` `abilities[]`). Kept deliberately
/// small and ENEMY-only — this is NOT the weapon ability path. Static content,
/// so it lives entirely on `EnemyDef` and never feeds the snapshot/checksum.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnemyAbility {
    /// No special ability (plain melee contact-only enemy).
    None,
    /// The breather/spitter standoff attack: once the enemy is within `range`
    /// units of the tank it fires every `cooldown_ticks`, dealing `damage` of
    /// `damage_type` to the tank (run through the tank's defensive layer). The
    /// firing phase is derived deterministically from `(tick, enemy id)` so it
    /// needs no per-enemy runtime state.
    RangedAttack {
        range: i64,
        cooldown_ticks: u32,
        damage: i64,
        damage_type: u8,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct EnemyDef {
    pub name: &'static str,
    pub base_hp: i64,
    pub move_speed: i64, // units per tick
    pub contact_damage: i64,
    pub bounty: i64,
    pub armor_class: u8,
    /// Coarse behavior class (flavor + render telemetry).
    pub archetype: Archetype,
    /// Special ability (enemy-side); `EnemyAbility::None` for plain melee.
    pub ability: EnemyAbility,
    /// A boss (e.g. The Hippocrate) — immune to weapon fire; only
    /// `Clear` damages it.
    pub boss: bool,
}

/// Armor classes (columns of [`damage_multiplier`]'s matrix).
pub const ARMOR_LIGHT: u8 = 0; // most chaff (full piercing, neutral else)
pub const ARMOR_MEDIUM: u8 = 1; // Fanged Death (magic-weak, siege-resistant)
pub const ARMOR_FORTIFIED: u8 = 2; // Bonk — tanky vs everything but Siege

#[derive(Clone, Copy, Debug)]
pub struct WaveSpawn {
    pub enemy: u16,
    pub cadence_ticks: u32, // spawn one every N ticks
    /// Gate: this entry only spawns once `tick >= start_tick` (escalating roster).
    /// `0` means active from the start. The mix gets richer/deadlier as gates open.
    pub start_tick: u32,
}

/// A stacking modifier's effect. Ratios are `(num, den)` → `Fixed::from_ratio`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModEffect {
    /// +% additive to ALL weapon damage.
    DamageGlobalPct(i64, i64),
    /// +% additive to one damage type's damage.
    DamageTypePct(u8, i64, i64),
    /// A multiplicative damage factor: multiplies total damage by `1 + num/den`.
    DamageMulPct(i64, i64),
    /// +% additive attack speed (reduces effective cooldown).
    AttackSpeedPct(i64, i64),
    /// +% additive kill bounty.
    BountyPct(i64, i64),
    /// +flat passive gold income per tick.
    IncomeFlat(i64),
    /// +flat max HP (and current HP).
    MaxHp(i64),
    /// +flat armor (flat damage reduction).
    Armor(i64),
    /// +Mana Shield pool (and regen/tick): `(pool, regen_per_tick)`.
    ManaShield(i64, i64),
    /// +flat HP regeneration per tick.
    HpRegen(i64),
    /// +dodge chance numerator (out of `Tank::dodge_den`, capped).
    Dodge(u32),
    /// +% additive damage for weapons matching a SCOPE id (attack class / range
    /// bucket / rarity — see [`attack_scope_id`] / [`range_scope_id`] /
    /// [`rarity_scope_id`]). `(scope_id, num, den)`.
    DamageScopePct(u8, i64, i64),
    /// +flat Spikes damage (retaliation when the tank is hit).
    SpikesFlat(i64),
    /// +% Spikes damage.
    SpikesPct(i64, i64),
    /// Register a Vulnerability-Pulse aura `(magnitude_stacks, range, interval_ticks)`.
    /// Intercepted in `ArenaState::buy_modifier` (carries per-arena state).
    GrantVulnPulse(i64, i64, i64),
    /// +flat HP healed to the tank each time an enemy dies (on-kill trigger).
    HealOnKill(i64),
    /// +flat HP healed to the tank each tick an enemy takes poison damage.
    HealOnPoison(i64),
    /// +% multiplier on passive gold income (additive into `income_mult`).
    IncomePct(i64, i64),
    /// +% of each income award also healed to the tank ("income as HP regen").
    IncomeRegenPct(i64, i64),
    /// Chance-based bonus bounty: `(chance_pct, bonus_pct)` — with `chance_pct%`
    /// probability per kill, pay an extra `bonus_pct%` of the base bounty.
    BountyProc(i64, i64),
    /// +% bonus damage dealt to **stunned** enemies (target-conditional, at impact).
    DamageVsStunnedPct(i64, i64),
    /// +% bonus damage dealt to **poisoned** enemies (target-conditional, at impact).
    DamageVsPoisonedPct(i64, i64),
    /// +% applied Poison DoT magnitude (the source's "+% Poison damage").
    PoisonDamagePct(i64, i64),
    /// +% applied Stun duration (the source's "+% Stun Duration").
    StunDurationPct(i64, i64),
    /// META: arm a one-shot duplicator — the next non-meta purchase of `(rarity)`
    /// (`255` = any) yields `copies` extra free copies. `(rarity, copies)`.
    GrantDuplicator(i64, i64),
    /// META: arm a one-shot voucher — the next non-meta purchase of `(rarity)`
    /// is free (the source's Black Market "of your choosing"). `(rarity)`.
    GrantVoucher(i64),
    /// META: instantly grant flat gold on purchase (the source's Magic Treasure).
    GrantGold(i64),
    /// +% multiplier on all healing the tank receives ("+% Healing").
    HealingPct(i64, i64),
    /// +% of missing HP healed once per second ("% Missing HP Heal every second").
    MissingHpHealPct(i64, i64),
    /// Grant one one-shot revive (Ankh); on a fatal hit, fully repair and gain
    /// `bonus` Max HP instead of dying.
    GrantRevive(i64),
    /// Self-scaling damage `(weapon_def, dmg_type, num_pct)`: +`num_pct`% damage
    /// of `dmg_type` per owned copy of `weapon_def` ("+1% Piercing per Bow").
    DamagePerWeapon(i64, i64, i64),
    /// Trade `(hp_cost, gold_gain)`: reduce Max HP by `hp_cost` (clamp HP to the
    /// new max) and grant `gold_gain` gold. Intercepted in `buy_modifier`.
    TradeMaxHpForGold(i64, i64),
    /// Trade `(regen_cost, gold_gain)`: reduce HP regen/tick by `regen_cost` (MAY
    /// go negative → a per-tick drain) and grant `gold_gain` gold. Intercepted in
    /// `buy_modifier`.
    TradeRegenForGold(i64, i64),
    /// +damage-scaled bounty `(num, den)`: each point of player damage dealt
    /// awards `floor(damage × num/den)` gold (the source's "Bloodmoney").
    GoldPerDamagePct(i64, i64),
    /// +% of each income award also added to the Mana-Shield pool `(num, den)`,
    /// capped at its max (mirrors `IncomeRegenPct` for the shield).
    IncomeShieldPct(i64, i64),
    /// +% of CURRENT Max HP added to Max HP (and current HP), `(num, den)`. The
    /// percentage is taken of `tank.max_hp` at the moment it applies, so in a
    /// bundle it compounds on whatever flat Max-HP effect ran before it (the
    /// source's "+25% Max HP" rider on Imbued Masonry). Integer/Fixed only.
    MaxHpPct(i64, i64),
    /// +% of CURRENT HP-regen/tick added to HP-regen/tick, `(num, den)` (the
    /// source's "+25% HP Regen" rider on Renew). Compounds on the flat regen that
    /// ran before it in the same bundle.
    HpRegenPct(i64, i64),
    /// +% of CURRENT Mana-Shield regen/tick added to it, `(num, den)` (the
    /// source's "+25% Mana Regeneration" rider on Recharge). Compounds on the flat
    /// shield-regen that ran before it in the same bundle.
    ManaRegenPct(i64, i64),
    /// +% Damage Reduction while the Mana Shield is active `(num, den)` (the
    /// source's Energy Shield rider). Accumulates additively into
    /// `tank.shield_active_dr`; at hit time, while `mana_shield > 0`, ALL incoming
    /// damage (shield + overflow) is scaled by `1 - dr` (clamped to `[0, ONE]`,
    /// min-1 lands). Integer/Fixed only.
    ShieldActiveDrPct(i64, i64),
    /// +flat HP healed each time an incoming hit LANDS (not dodged) — the source's
    /// "+N Heal when damaged" (Dreadlord Fang). Accumulates into
    /// `tank.heal_on_damaged`; routes through `Tank::heal` (one heal per landed
    /// hit, respecting `healing_mult` + the max-HP cap).
    HealOnDamaged(i64),
    /// DYNAMIC `+n/d% global damage per 2000 Max HP` (the source's Mastercrafted
    /// Masonry). Resolved LIVE from `tank.max_hp` at fire time (NOT baked at
    /// purchase) — like `DamagePerWeapon`. Accumulates the per-unit rate into
    /// `Modifiers::dmg_per_maxhp_rate`; the live bonus is `rate × (max_hp / 2000)`,
    /// folded into the per-weapon multiplier via `dynamic_global_add`.
    DamagePerMaxHp(i64, i64),
    /// DYNAMIC `+n/d% global damage per 50% Bounty` (the source's Golden Ring).
    /// Resolved LIVE from `economy.bounty_mult` at fire time. Accumulates the rate
    /// into `Modifiers::dmg_per_bounty_rate`; the live bonus is
    /// `rate × ((bounty_mult − 1) / 0.5)`, folded in via `dynamic_global_add`.
    DamagePerBountyPct(i64, i64),
    /// DYNAMIC `+n/d% global damage while the Mana Shield is active` (the source's
    /// Arcane Mark) — the offensive mirror of `ShieldActiveDrPct`. Accumulates into
    /// `Modifiers::shield_active_dmg`; added to the global additive at fire time iff
    /// `tank.mana_shield > 0`, via `dynamic_global_add`.
    ShieldActiveDamagePct(i64, i64),
    /// `+N Mana Shield restored each time an enemy dies` (the source's Maw of
    /// Death). Mirrors `HealOnKill`: accumulates into `tank.mana_on_kill` and fires
    /// once per kill in `collect_bounties`, routed through `Tank::restore_mana`
    /// (cap-respecting).
    ManaOnKill(i64),
    // ---- EXPANSION E2 (four exotic mechanics) --------------------------------
    /// SHIELD-BREAK STUN `(range, stun_ticks)` (source: Energy Pulse). When the
    /// Mana Shield transitions `>0 → 0` from a hit, stun every enemy within `range`
    /// for `stun_ticks`. Sets `tank.shieldbreak_stun_range/_ticks`; the down-edge is
    /// detected in `defense::hit_tank` and the AoE applied (stable id order) in
    /// `defense::shield_break_stun`.
    ShieldBreakStun(i64, i64),
    /// SPIKES POISON `(dps_per_tick, ticks)` (source: Poison Armor). When Spikes
    /// retaliation lands on an enemy, ALSO apply this Poison DoT to it (reuses the
    /// existing poison status). Sets `tank.spikes_poison_dps/_ticks`.
    SpikesPoison(i64, i64),
    /// STACKING SPIKES `(per_stack, max_stacks)` (source: Bloody Spikes). Each landed
    /// hit grows `tank.spikes_stacks` by 1 up to `max_stacks`; the bonus spikes
    /// damage is `per_stack × spikes_stacks`. Resets at the round boundary. Sets
    /// `tank.spikes_stack_per` and `tank.spikes_stacks_max`.
    StackingSpikes(i64, i64),
    /// DAMAGE/POISON AURA `(range, cadence_ticks, damage)` (source: Blight Aura).
    /// Every `cadence_ticks` (integer; NOT wall-clock), deal `damage` and apply the
    /// tank's `aura_poison_*` DoT to all enemies within `range` (stable id order).
    /// Sets `tank.aura_range/_cadence/_damage`; the per-tank `aura_tick` counter
    /// drives the cadence in `combat::tick_aura`. The poison rider is set alongside
    /// this effect in `buy_modifier` (the `aura_poison_*` fields).
    DamageAura(i64, i64, i64),
    // ---- EXPANSION E3 (source-fidelity mechanics pass) -----------------------
    /// DEEP FREEZE OPT-IN (source: "+1.5 seconds of Freeze when an enemy reaches
    /// 25 stacks of Frost, resetting stacks to 0"). Sets `tank.deep_freeze`;
    /// WITHOUT it, 25 frost stacks now merely cap (max slow, no freeze) — the
    /// freeze payoff is no longer baseline (`status::apply_on_hit`).
    GrantDeepFreeze,
    /// +% Frost strength `(num, den)` (source: "+% Frost damage and slow
    /// strength"): scales the per-stack Frost slow (and nothing else; frost
    /// carries no direct damage in this model). Accumulates additively into
    /// `Modifiers::frost_strength_mult`.
    FrostDamagePct(i64, i64),
    /// +% Fire strength `(num, den)` (source: "+% Fire damage and damage
    /// vulnerability"): scales BOTH the per-stack Fire vulnerability and the
    /// Fire death-explosion damage. Accumulates into `Modifiers::fire_dmg_mult`.
    FireDamagePct(i64, i64),
    /// +% Fire-EXPLOSION damage alone `(num, den)` (source: Combustion "+%
    /// bonus explosion damage"). Accumulates into
    /// `Modifiers::fire_explosion_mult`; multiplies with `FireDamagePct` on the
    /// explosion (distinct multiplicative sources).
    CombustionPct(i64, i64),
    /// +% enemies hit by Bounce and Barrage `(num, den)` (source: "+25% Enemies
    /// hit by Bounce and Barrage"). Accumulates into
    /// `Modifiers::bounce_barrage_pct`; resolved per FIRE as an integer bonus:
    /// `targets += floor(base_targets × pct)`.
    BounceBarragePct(i64, i64),
    /// META-ish economy grant: +N free shop rerolls (the source's Free Reroll
    /// items). Mutates `economy.rerolls_remaining` — the same counter the shop
    /// consumes.
    GrantFreeRerolls(i64),
    /// FROST ARMOR (source: "+2 stacks of Frost to an enemy when damaged"):
    /// tank-side retaliation status — when the tank is hit, apply this many
    /// Frost stacks to enemies in Spikes range (`defense::spikes`, which fires
    /// even with zero Spikes damage). Accumulates into `tank.retaliate_frost`.
    FrostArmor(i64),
    /// FLAMING ARMOR (source: "+20 stacks of Fire to an enemy when damaged"):
    /// the Fire twin of [`FrostArmor`]. Accumulates into `tank.retaliate_fire`.
    FireArmor(i64),
    /// FIRST-HIT SPIKES (source: "+240 Spikes Damage on the first attack"):
    /// flat bonus Spikes damage added to the retaliation for an enemy's FIRST
    /// landed hit on the tank (per enemy; tracked in `EnemyStatus::hit_tank`).
    /// Accumulates into `tank.spikes_first_hit`.
    SpikesFirstHit(i64),
    /// DEFLECTION `(num, den)` (source: "+5% of Spikes Damage as Flat Damage
    /// Reduction, but cannot reduce more than 50% of an attack"): flat DR equal
    /// to `rate × current Spikes damage`, capped at half the incoming hit.
    /// Accumulates into `tank.spikes_dr_rate`.
    SpikesAsDrPct(i64, i64),
    /// DAMAGE-TAKEN→SPIKES `(num, den)` (source: "+30% of Damage Taken Spikes
    /// Damage"): the Spikes retaliation for a hit gains `rate × damage the tank
    /// took this tick` as flat bonus damage. Accumulates into
    /// `tank.dmg_taken_to_spikes`.
    DamageTakenToSpikesPct(i64, i64),
    /// HEAL-CONDITIONAL DAMAGE `(num, den)` (source: "+35% Damage … when at 95%
    /// health or above"): a GLOBAL additive damage bonus active only while
    /// `tank.hp ≥ 95% of max_hp`, resolved LIVE at fire time via
    /// `dynamic_global_add`. Accumulates into `Modifiers::healthy_dmg`.
    DamageWhileHealthyPct(i64, i64),
}

/// Number of weapon damage scopes: 6 attack classes (0-5), 2 range buckets
/// (6 short / 7 long), 4 rarities (8-11).
pub const NUM_SCOPES: usize = 12;

/// Scope id for a weapon's attack class. Combined shapes classify by their
/// PRIMARY shape (Bounce&Splash → Bounce, Barrage&Splash → Barrage, rotating
/// waves → Wave) so per-class "+% Damage" upgrades keep applying to them.
pub fn attack_scope_id(a: Attack) -> u8 {
    match a {
        Attack::SingleTarget => 0,
        Attack::Splash(_) => 1,
        Attack::Barrage(_) | Attack::BarrageSplash(..) => 2,
        Attack::Area(_) => 3,
        Attack::Wave(_) | Attack::WaveRotating(..) => 4,
        Attack::Bounce(_) | Attack::BounceSplash(..) => 5,
    }
}
/// Scope id for a weapon's range bucket: 6 = short (≤600), 7 = long (≥900).
pub fn range_scope_id(range: i64) -> u8 {
    if range <= 600 {
        6
    } else {
        7
    }
}
/// Scope id for a weapon's rarity (8 = Common … 11 = Epic).
pub fn rarity_scope_id(rarity: u8) -> u8 {
    8 + rarity.min(3)
}

impl ModEffect {
    /// Stable `(tag, a, b, c)` encoding for checksum/snapshot of ramps.
    pub fn words(self) -> (u8, i64, i64, i64) {
        match self {
            ModEffect::DamageGlobalPct(n, d) => (0, n, d, 0),
            ModEffect::DamageTypePct(t, n, d) => (1, t as i64, n, d),
            ModEffect::DamageMulPct(n, d) => (2, n, d, 0),
            ModEffect::AttackSpeedPct(n, d) => (3, n, d, 0),
            ModEffect::BountyPct(n, d) => (4, n, d, 0),
            ModEffect::IncomeFlat(f) => (5, f, 0, 0),
            ModEffect::MaxHp(f) => (6, f, 0, 0),
            ModEffect::Armor(a) => (7, a, 0, 0),
            ModEffect::ManaShield(p, r) => (8, p, r, 0),
            ModEffect::HpRegen(r) => (9, r, 0, 0),
            ModEffect::Dodge(n) => (10, n as i64, 0, 0),
            ModEffect::DamageScopePct(s, n, d) => (11, s as i64, n, d),
            ModEffect::SpikesFlat(n) => (12, n, 0, 0),
            ModEffect::SpikesPct(n, d) => (13, n, d, 0),
            ModEffect::GrantVulnPulse(m, r, i) => (14, m, r, i),
            ModEffect::HealOnKill(n) => (15, n, 0, 0),
            ModEffect::HealOnPoison(n) => (16, n, 0, 0),
            ModEffect::IncomePct(n, d) => (17, n, d, 0),
            ModEffect::IncomeRegenPct(n, d) => (18, n, d, 0),
            ModEffect::BountyProc(c, b) => (19, c, b, 0),
            ModEffect::DamageVsStunnedPct(n, d) => (20, n, d, 0),
            ModEffect::DamageVsPoisonedPct(n, d) => (21, n, d, 0),
            ModEffect::PoisonDamagePct(n, d) => (22, n, d, 0),
            ModEffect::StunDurationPct(n, d) => (23, n, d, 0),
            ModEffect::GrantDuplicator(r, c) => (24, r, c, 0),
            ModEffect::GrantVoucher(r) => (25, r, 0, 0),
            ModEffect::GrantGold(g) => (26, g, 0, 0),
            ModEffect::HealingPct(n, d) => (27, n, d, 0),
            ModEffect::MissingHpHealPct(n, d) => (28, n, d, 0),
            ModEffect::GrantRevive(b) => (29, b, 0, 0),
            ModEffect::DamagePerWeapon(def, ty, n) => (30, def, ty, n),
            ModEffect::TradeMaxHpForGold(hp, g) => (31, hp, g, 0),
            ModEffect::TradeRegenForGold(r, g) => (32, r, g, 0),
            ModEffect::GoldPerDamagePct(n, d) => (33, n, d, 0),
            ModEffect::IncomeShieldPct(n, d) => (34, n, d, 0),
            ModEffect::MaxHpPct(n, d) => (35, n, d, 0),
            ModEffect::HpRegenPct(n, d) => (36, n, d, 0),
            ModEffect::ManaRegenPct(n, d) => (37, n, d, 0),
            ModEffect::ShieldActiveDrPct(n, d) => (38, n, d, 0),
            ModEffect::HealOnDamaged(n) => (39, n, 0, 0),
            ModEffect::DamagePerMaxHp(n, d) => (40, n, d, 0),
            ModEffect::DamagePerBountyPct(n, d) => (41, n, d, 0),
            ModEffect::ShieldActiveDamagePct(n, d) => (42, n, d, 0),
            ModEffect::ManaOnKill(n) => (43, n, 0, 0),
            ModEffect::ShieldBreakStun(r, t) => (44, r, t, 0),
            ModEffect::SpikesPoison(dps, t) => (45, dps, t, 0),
            ModEffect::StackingSpikes(per, max) => (46, per, max, 0),
            ModEffect::DamageAura(r, c, d) => (47, r, c, d),
            ModEffect::GrantDeepFreeze => (48, 0, 0, 0),
            ModEffect::FrostDamagePct(n, d) => (49, n, d, 0),
            ModEffect::FireDamagePct(n, d) => (50, n, d, 0),
            ModEffect::CombustionPct(n, d) => (51, n, d, 0),
            ModEffect::BounceBarragePct(n, d) => (52, n, d, 0),
            ModEffect::GrantFreeRerolls(n) => (53, n, 0, 0),
            ModEffect::FrostArmor(n) => (54, n, 0, 0),
            ModEffect::FireArmor(n) => (55, n, 0, 0),
            ModEffect::SpikesFirstHit(n) => (56, n, 0, 0),
            ModEffect::SpikesAsDrPct(n, d) => (57, n, d, 0),
            ModEffect::DamageTakenToSpikesPct(n, d) => (58, n, d, 0),
            ModEffect::DamageWhileHealthyPct(n, d) => (59, n, d, 0),
        }
    }

    /// META items modify the shop/purchase flow rather than the tank's stats.
    /// Duplicators/vouchers must NOT trigger on, or be duplicated by, each other.
    pub fn is_meta(self) -> bool {
        matches!(
            self,
            ModEffect::GrantDuplicator(..)
                | ModEffect::GrantVoucher(..)
                | ModEffect::GrantGold(..)
                | ModEffect::TradeMaxHpForGold(..)
                | ModEffect::TradeRegenForGold(..)
        )
    }
    /// Income/gold-generation effects — what a "no economy" run abstains from.
    /// Render-only telemetry; never feeds the checksum.
    pub fn is_economy(self) -> bool {
        matches!(
            self,
            ModEffect::IncomeFlat(..) | ModEffect::IncomePct(..) | ModEffect::GrantGold(..)
        )
    }
    /// Inverse of [`words`](Self::words).
    pub fn from_words(tag: u8, a: i64, b: i64, c: i64) -> Option<ModEffect> {
        Some(match tag {
            0 => ModEffect::DamageGlobalPct(a, b),
            1 => ModEffect::DamageTypePct(a as u8, b, c),
            2 => ModEffect::DamageMulPct(a, b),
            3 => ModEffect::AttackSpeedPct(a, b),
            4 => ModEffect::BountyPct(a, b),
            5 => ModEffect::IncomeFlat(a),
            6 => ModEffect::MaxHp(a),
            7 => ModEffect::Armor(a),
            8 => ModEffect::ManaShield(a, b),
            9 => ModEffect::HpRegen(a),
            10 => ModEffect::Dodge(a as u32),
            11 => ModEffect::DamageScopePct(a as u8, b, c),
            12 => ModEffect::SpikesFlat(a),
            13 => ModEffect::SpikesPct(a, b),
            14 => ModEffect::GrantVulnPulse(a, b, c),
            15 => ModEffect::HealOnKill(a),
            16 => ModEffect::HealOnPoison(a),
            17 => ModEffect::IncomePct(a, b),
            18 => ModEffect::IncomeRegenPct(a, b),
            19 => ModEffect::BountyProc(a, b),
            20 => ModEffect::DamageVsStunnedPct(a, b),
            21 => ModEffect::DamageVsPoisonedPct(a, b),
            22 => ModEffect::PoisonDamagePct(a, b),
            23 => ModEffect::StunDurationPct(a, b),
            24 => ModEffect::GrantDuplicator(a, b),
            25 => ModEffect::GrantVoucher(a),
            26 => ModEffect::GrantGold(a),
            27 => ModEffect::HealingPct(a, b),
            28 => ModEffect::MissingHpHealPct(a, b),
            29 => ModEffect::GrantRevive(a),
            30 => ModEffect::DamagePerWeapon(a, b, c),
            31 => ModEffect::TradeMaxHpForGold(a, b),
            32 => ModEffect::TradeRegenForGold(a, b),
            33 => ModEffect::GoldPerDamagePct(a, b),
            34 => ModEffect::IncomeShieldPct(a, b),
            35 => ModEffect::MaxHpPct(a, b),
            36 => ModEffect::HpRegenPct(a, b),
            37 => ModEffect::ManaRegenPct(a, b),
            38 => ModEffect::ShieldActiveDrPct(a, b),
            39 => ModEffect::HealOnDamaged(a),
            40 => ModEffect::DamagePerMaxHp(a, b),
            41 => ModEffect::DamagePerBountyPct(a, b),
            42 => ModEffect::ShieldActiveDamagePct(a, b),
            43 => ModEffect::ManaOnKill(a),
            44 => ModEffect::ShieldBreakStun(a, b),
            45 => ModEffect::SpikesPoison(a, b),
            46 => ModEffect::StackingSpikes(a, b),
            47 => ModEffect::DamageAura(a, b, c),
            48 => ModEffect::GrantDeepFreeze,
            49 => ModEffect::FrostDamagePct(a, b),
            50 => ModEffect::FireDamagePct(a, b),
            51 => ModEffect::CombustionPct(a, b),
            52 => ModEffect::BounceBarragePct(a, b),
            53 => ModEffect::GrantFreeRerolls(a),
            54 => ModEffect::FrostArmor(a),
            55 => ModEffect::FireArmor(a),
            56 => ModEffect::SpikesFirstHit(a),
            57 => ModEffect::SpikesAsDrPct(a, b),
            58 => ModEffect::DamageTakenToSpikesPct(a, b),
            59 => ModEffect::DamageWhileHealthyPct(a, b),
            _ => return None,
        })
    }
}

/// A per-interval growth attached to a modifier — the source's "+X every 30 s".
/// On purchase the modifier's base `effects` apply once; then `effect` here is
/// re-applied every `interval_ticks` for the rest of the match.
#[derive(Clone, Copy, Debug)]
pub struct RampSpec {
    pub effect: ModEffect,
    pub interval_ticks: u32,
}

/// Interval for "every 30 seconds" ramps (= one round @ 30 Hz).
pub const RAMP_PER_ROUND: u32 = 30 * 30;

#[derive(Clone, Copy, Debug)]
pub struct ModifierDef {
    pub name: &'static str,
    pub rarity: u8,
    pub cost: i64,
    /// Effects applied, in order, on purchase. Most modifiers carry exactly one.
    pub effects: &'static [ModEffect],
    /// Optional per-interval growth (`None` for most modifiers).
    pub ramp: Option<RampSpec>,
}

impl ModifierDef {
    /// True if any of this modifier's effects is an income/gold-generation
    /// effect (render-only telemetry; never feeds the checksum).
    pub fn is_economy(&self) -> bool {
        self.effects.iter().any(|e| e.is_economy())
    }
    /// True if any of this modifier's effects is a META effect (must not
    /// trigger or be duplicated).
    pub fn is_meta(&self) -> bool {
        self.effects.iter().any(|e| e.is_meta())
    }
}

/// M4 modifier catalog. CONTENT-FIDELITY pass: entries that map to a source
/// upgrade (see the extraction under `research/`) carry the source upgrade's
/// name, and bundled multi-effect source upgrades are re-bundled into one
/// `ModifierDef { effects: &[…] }` (effects apply in slice order). Some source
/// secondaries need combat/defense mechanics not yet built — those keep the NAME
/// plus the modelable PRIMARY effect, with a `// TODO(M1c/M2/M3)` note for the
/// exotic part. A handful of entries have no clean source mapping and are kept as
/// representative slices (flagged "representative"). Numbers track the source
/// effect text; existing cost/rarity preserved.
pub static MODIFIERS: &[ModifierDef] = &[
    // representative: "+10% to ALL damage" has no single source upgrade (closest
    // is Improved Attacks "+5% all types"); kept as a generic global-damage item.
    ModifierDef {
        name: "+10% Damage",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::DamageGlobalPct(1, 10)],
        ramp: None,
    },
    ModifierDef {
        name: "Improved Piercing Attacks",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::DamageTypePct(DMG_PIERCING, 1, 10)],
        ramp: None,
    },
    ModifierDef {
        name: "Improved Siege Attacks",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::DamageTypePct(DMG_SIEGE, 1, 10)],
        ramp: None,
    },
    ModifierDef {
        name: "Improved Magic Attacks",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::DamageTypePct(DMG_MAGIC, 1, 10)],
        ramp: None,
    },
    // representative Epic multiplier (no source upgrade is a flat ×-damage item):
    // a true force-multiplier on an already-spiky build.
    ModifierDef {
        name: "+25% Damage (Epic)",
        rarity: 3,
        cost: 5000,
        effects: &[ModEffect::DamageMulPct(2, 5)],
        ramp: None,
    },
    ModifierDef {
        name: "Rapidfire",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::AttackSpeedPct(1, 10)],
        ramp: None,
    },
    ModifierDef {
        name: "Bounty Hunter",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::BountyPct(1, 2)],
        ramp: None,
    },
    // Entangled Gold Mine (A0H0): "+20 Gold Income | +25% of Gold Income as instant
    // HP Regen". RE-BUNDLED — the live catalog had split this into "+20 Gold Income"
    // + a standalone "Golden Vitality (... as HP Regen)"; that standalone is DELETED
    // and folded back here (source ratio 25%, was 40% on the spun-off item).
    // CATALOG-FIDELITY PASS: moved rarity 0→1 (cost 1500) — placed by POWER on
    // the invented ladder: income + a heal engine in one item is strictly
    // stronger than its 500g income peers, and at common price the spammable
    // heal loop let a WEAPONLESS eco tank turn immortal inside two minutes on
    // too many seeds (balance_guards' eco-rush punish). Effect unchanged.
    ModifierDef {
        name: "Entangled Gold Mine",
        rarity: 1,
        cost: 1500,
        effects: &[
            ModEffect::IncomeFlat(20),
            ModEffect::IncomeRegenPct(25, 100),
        ],
        ramp: None,
    },
    // ECONOMY SNOWBALL (high-ceiling/high-risk): income multipliers are the engine
    // of the snowball-or-die path. Cheap, but they buy gold not survival.
    // representative income-multiplier items (Gold Mine below is the true source
    // "+10 Income +10% Income"; these pure ×income items are kept as a slice).
    // (catalog-fidelity: these two invented ×income items granted DOUBLE their
    // labels — 20%/50% — which over-fueled the income snowball; aligned to
    // their names.)
    ModifierDef {
        name: "+10% Gold Income",
        rarity: 1,
        cost: 1000,
        effects: &[ModEffect::IncomePct(10, 100)],
        ramp: None,
    },
    ModifierDef {
        name: "+25% Gold Income",
        rarity: 2,
        cost: 2000,
        effects: &[ModEffect::IncomePct(25, 100)],
        ramp: None,
    },
    // Transmute (A0AE): "+100% Kill Bounty | +200% Bounty Gold with 5% activation
    // chance". RE-BUNDLED — the live catalog modeled ONLY the proc ("Lucky Strikes");
    // now both the flat bounty and the gambling proc ride one named upgrade.
    ModifierDef {
        name: "Transmute",
        rarity: 2,
        cost: 3000,
        effects: &[
            ModEffect::BountyPct(100, 100),
            ModEffect::BountyProc(5, 200),
        ],
        ramp: None,
    },
    // Imbued Masonry (A04J): "+2000 Max HP | +25% Max HP". RE-BUNDLED via the new
    // MaxHpPct rider (the +25% is taken of max_hp AFTER the +2000 flat applies).
    ModifierDef {
        name: "Imbued Masonry",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::MaxHp(2000), ModEffect::MaxHpPct(25, 100)],
        ramp: None,
    },
    ModifierDef {
        name: "+10 Armor",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::Armor(10)],
        ramp: None,
    },
    // Moonwell (A0FA): "+2000 Mana Shield | +10 Mana Shield every second" — the
    // per-second shield regen is modeled as the ManaShield regen-per-tick field.
    ModifierDef {
        name: "Moonwell",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::ManaShield(2000, 10)],
        ramp: None,
    },
    // representative flat-regen item (the true source +50-regen upgrades all bundle
    // a secondary; this plain +50 is kept as a slice).
    ModifierDef {
        name: "+50 HP Regen",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::HpRegen(50)],
        ramp: None,
    },
    // (dedupe: renamed from "Evasion" — the rarity-0 GEN entry below keeps the
    // plain name; this is the higher-rarity copy, following the "Greater"
    // precedent.)
    ModifierDef {
        name: "Greater Evasion",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::Dodge(10)],
        ramp: None,
    },
    // Time-scaling growth modifiers (`docs/06`): a base effect now + a smaller
    // effect re-applied every round, so they compound over a match.
    // Power Generator (A099): "+2% Damage | +1% Damage every 30 seconds".
    ModifierDef {
        name: "Power Generator",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::DamageGlobalPct(2, 100)],
        ramp: Some(RampSpec {
            effect: ModEffect::DamageGlobalPct(1, 100),
            interval_ticks: RAMP_PER_ROUND,
        }),
    },
    // Scroll of Chaos (A0IB): "+20% Chaos | +3% Chaos every 30 seconds".
    ModifierDef {
        name: "Scroll of Chaos",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::DamageTypePct(DMG_CHAOS, 20, 100)],
        ramp: Some(RampSpec {
            effect: ModEffect::DamageTypePct(DMG_CHAOS, 3, 100),
            interval_ticks: RAMP_PER_ROUND,
        }),
    },
    // representative compounding-income ramp (no exact source; Gold Mine/Magic
    // Treasure are the named income items). Kept as a slice.
    ModifierDef {
        name: "Compounding Greed (+10 Income, +5/round)",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::IncomeFlat(10)],
        ramp: Some(RampSpec {
            effect: ModEffect::IncomeFlat(5),
            interval_ticks: RAMP_PER_ROUND,
        }),
    },
    // Blessed Armor (A096): "+10 Armor | +15 Bonus Armor every 30 seconds | -1 when
    // damaged". The flat-now + per-round growth is modeled; the small -1-on-damage
    // decay is dropped (a minor exotic, not worth a mechanic). Ramp kept at the
    // live +5/round (the source's +15/round would be a balance retune — flagged).
    ModifierDef {
        name: "Blessed Armor",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::Armor(10)],
        ramp: Some(RampSpec {
            effect: ModEffect::Armor(5),
            interval_ticks: RAMP_PER_ROUND,
        }),
    },
    // Per-scope damage (`docs/06`): +% for weapons matching an attack class /
    // range bucket / rarity (scope ids from attack_scope_id/range_scope_id/rarity_scope_id).
    // Focusfire (A07I): "+25% Damage for Single Target Weapons".
    ModifierDef {
        name: "Focusfire",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::DamageScopePct(0, 25, 100)],
        ramp: None,
    },
    // representative per-attack-class items (the source has no clean +Splash/
    // +Barrage/+Area/+Bounce upgrade — it bundles "Bounce and Barrage" together);
    // kept as a slice exposing each attack scope.
    ModifierDef {
        name: "+25% Splash Damage",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::DamageScopePct(1, 25, 100)],
        ramp: None,
    },
    ModifierDef {
        name: "+25% Barrage Damage",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::DamageScopePct(2, 25, 100)],
        ramp: None,
    },
    ModifierDef {
        name: "+25% Area Damage",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::DamageScopePct(3, 25, 100)],
        ramp: None,
    },
    // Wavefire (A0JO): "+25% Damage for Wave Weapons".
    ModifierDef {
        name: "Wavefire",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::DamageScopePct(4, 25, 100)],
        ramp: None,
    },
    ModifierDef {
        name: "+25% Bounce Damage",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::DamageScopePct(5, 25, 100)],
        ramp: None,
    },
    // Command Aura (A078): "+25% Damage for 300 and 600 Attack Range Weapons".
    ModifierDef {
        name: "Command Aura",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::DamageScopePct(6, 25, 100)],
        ramp: None,
    },
    // Trueshot Aura (A04T): "+25% Damage for 900 and 1200 Attack Range Weapons".
    ModifierDef {
        name: "Trueshot Aura",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::DamageScopePct(7, 25, 100)],
        ramp: None,
    },
    // Engineering Upgrade (A09F): "+100% Damage for 500 Gold (Common) Weapons".
    ModifierDef {
        name: "Engineering Upgrade",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::DamageScopePct(8, 100, 100)],
        ramp: None,
    },
    // Status-conditional & flavor damage (`docs/06` #4): bonus damage vs enemies
    // in a status, and scalers on the statuses the tank applies.
    // Bash (A032): "+20% Damage to Stunned enemies".
    ModifierDef {
        name: "Bash",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::DamageVsStunnedPct(20, 100)],
        ramp: None,
    },
    // Corrosive Poison (A04R): "+25% Damage to Poisoned enemies".
    ModifierDef {
        name: "Corrosive Poison",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::DamageVsPoisonedPct(25, 100)],
        ramp: None,
    },
    // Potent Poison (A034): "+10% Poison damage".
    ModifierDef {
        name: "Potent Poison",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::PoisonDamagePct(10, 100)],
        ramp: None,
    },
    // Dazing Stuns (A0CH): "+50% Stun Duration".
    ModifierDef {
        name: "Dazing Stuns",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::StunDurationPct(50, 100)],
        ramp: None,
    },
    // Spikes (`docs/06`): retaliation damage to nearby enemies when the tank is hit.
    // Dreadlord Fang (A03A): "+80 Spikes Damage | +8 Heal when damaged". Both
    // effects modeled: flat spikes retaliation + a flat heal on every landed hit.
    ModifierDef {
        name: "Dreadlord Fang",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::SpikesFlat(80), ModEffect::HealOnDamaged(8)],
        ramp: None,
    },
    // representative big-flat-spikes item (source has +160/+400 flats, not +300);
    // kept as a slice.
    ModifierDef {
        name: "+300 Spikes Damage",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::SpikesFlat(300)],
        ramp: None,
    },
    // representative pure-%-spikes item (source always bundles flat+%); kept as a slice.
    ModifierDef {
        name: "+50% Spikes Damage",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::SpikesPct(50, 100)],
        ramp: None,
    },
    // Growing Spikes (A0DU): "+80 Spikes Damage | +10 Spikes Damage every 30 seconds".
    ModifierDef {
        name: "Growing Spikes",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::SpikesFlat(80)],
        ramp: Some(RampSpec {
            effect: ModEffect::SpikesFlat(10),
            interval_ticks: RAMP_PER_ROUND,
        }),
    },
    // Vulnerability Totem (A0EN): "Vulnerability Pulse: +5% damage taken to all
    // enemies within 1200 range every second, stacking."
    ModifierDef {
        name: "Vulnerability Totem",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::GrantVulnPulse(5, 1200, 30)],
        ramp: None,
    },
    // On-event triggers (`docs/06`): heal the tank on enemy-kill / on-poison-tick.
    // Mask of Death (A07S): "+1000 Max HP | +15 Heal when an enemy dies". RE-BUNDLED
    // — the live "+15 Heal on Kill" gains its Max-HP half from the source upgrade.
    ModifierDef {
        name: "Mask of Death",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::MaxHp(1000), ModEffect::HealOnKill(15)],
        ramp: None,
    },
    // representative larger heal-on-kill item (no source for a bare +60-on-kill).
    ModifierDef {
        name: "+60 Heal on Kill",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::HealOnKill(60)],
        ramp: None,
    },
    // Reanimating Poison (A0FL): "+5 instant HP Regen when an enemy takes Poison
    // damage from a Weapon | +1 ... from an Upgrade". Primary (weapon-poison heal)
    // modeled; the upgrade-poison half collapses into the same per-tick heal.
    ModifierDef {
        name: "Reanimating Poison",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::HealOnPoison(5)],
        ramp: None,
    },
    // Meta / shop items (`docs/06` #5): they bend the purchase flow, not the
    // tank's stats. Resolved deterministically in `input::apply` (a one-shot
    // `PendingPerk`), and flagged `is_meta` so they never trigger or duplicate
    // each other.
    // Multiplication Gems (A0BC): "+3 extra copies of the next 500 Gold (Common)
    // Upgrade" (the live name "Magic Coin" was wrong — Magic Coin is "+5 Gold
    // Income"; the common-duplicator is Multiplication Gems).
    ModifierDef {
        name: "Multiplication Gems",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::GrantDuplicator(0, 3)],
        ramp: None,
    },
    // Duplicator (A0EH): "+1 extra copy of the next Rare Weapon or Spikes Upgrade".
    ModifierDef {
        name: "Duplicator",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::GrantDuplicator(2, 1)],
        ramp: None,
    },
    // Black Market (A0GR): "Buy 1 Uncommon Weapon or Spikes Upgrade of your choosing".
    ModifierDef {
        name: "Black Market",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::GrantVoucher(1)],
        ramp: None,
    },
    // Magic Treasure (A0FP): "When used, gain +250 Gold | Gold value increases
    // by 2 per second | Purchasing a second Magic Treasure uses the first."
    // APPROXIMATION (honest): the source item is a HELD consumable banked on a
    // player-chosen "use"; a use-consumable needs an input variant we don't
    // ship yet. Nearest input-free semantics, implemented via the `GrantGold`
    // interception in `ArenaState::buy_modifier`: on purchase the 250 goes
    // into a visible holding pool (`Economy::treasure_pool`) that grows +2/s
    // (`economy::tick_income`) and AUTO-BANKS when the next shop rolls
    // (`economy::on_round_start`); buying a second Treasure banks the first
    // (as in the source). This replaces the earlier instant +250 + permanent
    // +5-income/round ramp, which had no source basis.
    ModifierDef {
        name: "Magic Treasure",
        rarity: 1,
        cost: 1000,
        effects: &[ModEffect::GrantGold(250)],
        ramp: None,
    },
    // Self-scaling / healing / revive (`docs/06` #6): bespoke survival & growth.
    // Ankh of Reconstruction (A01T): "Upon fatal damage, fully repair the tower,
    // using up the Ankh but gaining +2000 Max HP."
    ModifierDef {
        name: "Ankh of Reconstruction",
        rarity: 3,
        cost: 5000,
        effects: &[ModEffect::GrantRevive(2000)],
        ramp: None,
    },
    // Healing Hand (A0BI): "+25% Healing".
    ModifierDef {
        name: "Healing Hand",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::HealingPct(25, 100)],
        ramp: None,
    },
    // Living Wood (A09C): "+2000 Max HP | +1.5% Missing HP Heal every second".
    // RE-BUNDLED — the live "Regeneration" modeled only the missing-HP heal; the
    // Max-HP half now rides the named upgrade.
    ModifierDef {
        name: "Living Wood",
        rarity: 2,
        cost: 3000,
        effects: &[
            ModEffect::MaxHp(2000),
            ModEffect::MissingHpHealPct(15, 1000),
        ],
        ramp: None,
    },
    // Enchanted Moon Arrow (A0CZ): "+100% Piercing Damage | +1% Piercing per Bow".
    // RE-BUNDLED — the live item modeled only the per-Bow self-scaling; the flat
    // +100% Piercing half now rides the named upgrade.
    ModifierDef {
        name: "Enchanted Moon Arrow",
        rarity: 2,
        cost: 3000,
        effects: &[
            ModEffect::DamageTypePct(DMG_PIERCING, 100, 100),
            ModEffect::DamagePerWeapon(0, DMG_PIERCING as i64, 1),
        ],
        ramp: None,
    },
    // Refined Explosives (A0D3): "+100% Siege Damage | +1% Siege per Boulder".
    // RE-BUNDLED (flat +100% Siege + per-Boulder self-scaling). NOTE: the source
    // keys "per Boulder"; here it keys per Mortar Launcher (weapon def 1) as the
    // live catalog did — the Boulder weapon lives in the GEN block at a non-stable
    // index, so the stable index 1 (Mortar) is used. Flagged as a judgement call.
    ModifierDef {
        name: "Refined Explosives",
        rarity: 2,
        cost: 3000,
        effects: &[
            ModEffect::DamageTypePct(DMG_SIEGE, 100, 100),
            ModEffect::DamagePerWeapon(1, DMG_SIEGE as i64, 1),
        ],
        ramp: None,
    },
    // Stacking damage generator (`docs/06` #5): the Death Engine weapon's Chaos
    // damage scales +10% per Death Engine owned (self-referential count).
    // representative self-referential generator (a self-scaling generator weapon
    // does +10% per copy; this models that as a +Chaos-per-Death-Engine
    // modifier keyed to the stable Death Engine weapon index — kept, ramp test
    // depends on it).
    ModifierDef {
        name: "Overclocked Death Engine (+10% Chaos per Death Engine)",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::DamagePerWeapon(
            DEATH_ENGINE as i64,
            DMG_CHAOS as i64,
            10,
        )],
        ramp: None,
    },
    // Damage ↔ economy trades & damage-scaled bounty (`docs/06` #5): pay survival
    // stats for gold, and convert dealt damage into gold, and income into a buffer.
    // Philosopher's Stone (A01S): "-1000 Max HP | +2000 Gold".
    ModifierDef {
        name: "Philosopher's Stone",
        rarity: 1,
        cost: 0,
        effects: &[ModEffect::TradeMaxHpForGold(1000, 2000)],
        ramp: None,
    },
    // Cursed Treasure (A04P): "-100 HP Regen | +5000 Gold".
    ModifierDef {
        name: "Cursed Treasure",
        rarity: 2,
        cost: 0,
        effects: &[ModEffect::TradeRegenForGold(100, 5000)],
        ramp: None,
    },
    // representative damage→gold items (the source's "Bloodmoney" gold-per-damage
    // mechanic, no single named upgrade); kept as a slice.
    ModifierDef {
        name: "Bloodmoney (+1 Gold per 100 Damage)",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::GoldPerDamagePct(1, 60)],
        ramp: None,
    },
    ModifierDef {
        name: "Bloodmoney II (+1 Gold per 20 Damage)",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::GoldPerDamagePct(1, 12)],
        ramp: None,
    },
    // representative income→shield item (mirrors income→HP for the shield); no
    // single source name. Kept as a slice.
    ModifierDef {
        name: "Wartithe (25% of Income as Mana Shield)",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::IncomeShieldPct(25, 100)],
        ramp: None,
    },
    // GEN-MODIFIERS-BEGIN (originally generated by research/tower-survivors-map/
    // gen_catalog.py; names restored / re-bundled in the content-fidelity pass).
    // representative ramping epic shield (no exact source; closest named shields
    // are Recharge / Energy Shield below). Kept as a slice.
    ModifierDef {
        name: "Aegis Protocol (+2500 Shield, +400/round)",
        rarity: 3,
        cost: 5000,
        effects: &[ModEffect::ManaShield(2500, 20)],
        ramp: Some(RampSpec {
            effect: ModEffect::ManaShield(400, 4),
            interval_ticks: RAMP_PER_ROUND,
        }),
    },
    // Improved Masonry (A001): "+500 Max HP".
    ModifierDef {
        name: "Improved Masonry",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::MaxHp(500)],
        ramp: None,
    },
    ModifierDef {
        name: "Greater Piercing Attacks",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::DamageTypePct(DMG_PIERCING, 10, 100)],
        ramp: None,
    },
    ModifierDef {
        name: "Improved Normal Attacks",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::DamageTypePct(DMG_NORMAL, 10, 100)],
        ramp: None,
    },
    // (dedupe: renamed from a second "Improved Siege Attacks", following the
    // Greater Piercing precedent.)
    ModifierDef {
        name: "Greater Siege Attacks",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::DamageTypePct(DMG_SIEGE, 10, 100)],
        ramp: None,
    },
    ModifierDef {
        name: "Improved Chaos Attacks",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::DamageTypePct(DMG_CHAOS, 10, 100)],
        ramp: None,
    },
    // representative +1000-Max-HP item (the source bundles +1000 with a secondary
    // on Mask of Death / Magic Seeds); kept as a plain slice.
    ModifierDef {
        name: "+1000 Max HP",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::MaxHp(1000)],
        ramp: None,
    },
    // representative +10-armor item (every source +10-armor upgrade bundles a
    // secondary — Blessed/Spiky/Frost/Poison Armor); kept as a plain slice.
    // (dedupe: renamed from a second "+10 Armor".)
    ModifierDef {
        name: "Greater Tower Armor",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::Armor(10)],
        ramp: None,
    },
    // representative +2000-Max-HP item (Imbued Masonry above is the bundled source).
    ModifierDef {
        name: "+2000 Max HP",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::MaxHp(2000)],
        ramp: None,
    },
    // representative +50%-bounty item (Bounty Hunter above is the named source).
    ModifierDef {
        name: "+50% Kill Bounty",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::BountyPct(50, 100)],
        ramp: None,
    },
    // Recharge (A0EX): "+4000 Mana Shield | +25% Mana Regeneration". RE-BUNDLED via
    // the new ManaRegenPct rider (+25% of the shield's per-tick regen, applied after
    // the flat pool). NOTE: kept this entry's existing pool (2000) per rule 5 rather
    // than the source's 4000 — flagged as a judgement call.
    ModifierDef {
        name: "Recharge",
        rarity: 2,
        cost: 3000,
        effects: &[
            ModEffect::ManaShield(2000, 10),
            ModEffect::ManaRegenPct(25, 100),
        ],
        ramp: None,
    },
    // representative +20-income item (Entangled Gold Mine above is the bundled source).
    ModifierDef {
        name: "+20 Gold Income",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::IncomeFlat(20)],
        ramp: None,
    },
    // Tower Armor (A02G): "+5 Armor".
    ModifierDef {
        name: "Tower Armor",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::Armor(5)],
        ramp: None,
    },
    // representative +attack-speed item (Rapidfire above is the named source).
    ModifierDef {
        name: "+10% Attack Speed",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::AttackSpeedPct(10, 100)],
        ramp: None,
    },
    // Renew (A066): "+80 HP Regen | +25% HP Regen". RE-BUNDLED via the new HpRegenPct
    // rider (+25% of the per-tick regen, applied after the flat +80).
    ModifierDef {
        name: "Renew",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::HpRegen(80), ModEffect::HpRegenPct(25, 100)],
        ramp: None,
    },
    // Repair Crew (A00G): "+20 HP Regen".
    ModifierDef {
        name: "Repair Crew",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::HpRegen(20)],
        ramp: None,
    },
    // Magic Coin (A01L): "+5 Gold Income".
    ModifierDef {
        name: "Magic Coin",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::IncomeFlat(5)],
        ramp: None,
    },
    // Gold Mine (A02J): "+10 Gold Income | +10% Gold Income". RE-BUNDLED — the live
    // "+10 Gold Income" gains its +10% income-multiplier half from the source.
    ModifierDef {
        name: "Gold Mine",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::IncomeFlat(10), ModEffect::IncomePct(10, 100)],
        ramp: None,
    },
    // representative +100%-bounty item (Transmute above is the bundled named source).
    ModifierDef {
        name: "+100% Kill Bounty",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::BountyPct(100, 100)],
        ramp: None,
    },
    // (dedupe: renamed from a second "Improved Magic Attacks", following the
    // Greater Piercing precedent.)
    ModifierDef {
        name: "Greater Magic Attacks",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::DamageTypePct(DMG_MAGIC, 10, 100)],
        ramp: None,
    },
    // representative +40-regen item (Wisp/Rejuvenating Petal bundle a secondary);
    // kept as a plain slice.
    ModifierDef {
        name: "+40 HP Regen",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::HpRegen(40)],
        ramp: None,
    },
    // Energy Shield (A0FD): "+10000 Mana Shield | +30% Damage Reduction while Mana
    // Shield is active". Both effects modeled: the huge shield pool plus the
    // conditional -30% DR that applies to ALL incoming damage while the shield holds.
    ModifierDef {
        name: "Energy Shield",
        rarity: 3,
        cost: 5000,
        effects: &[
            ModEffect::ManaShield(10000, 50),
            ModEffect::ShieldActiveDrPct(30, 100),
        ],
        ramp: None,
    },
    // Evasion (A0CL): "+10% Dodge".
    ModifierDef {
        name: "Evasion",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::Dodge(10)],
        ramp: None,
    },
    // representative ramping-bounty item (Golden Ring's "+1% Damage per 50% Bounty"
    // secondary is exotic; this models a plain bounty ramp). Kept as a slice.
    ModifierDef {
        name: "Escalating Plunder (+100% Bounty, +15%/round)",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::BountyPct(100, 100)],
        ramp: Some(RampSpec {
            effect: ModEffect::BountyPct(15, 100),
            interval_ticks: RAMP_PER_ROUND,
        }),
    },
    // representative ramping-Max-HP item (Magic Seeds "+1000 +5/sec" is the closest
    // source; this is a bigger epic slice). Kept.
    ModifierDef {
        name: "Living Fortress (+2500 Max HP, +500/round)",
        rarity: 3,
        cost: 5000,
        effects: &[ModEffect::MaxHp(2500)],
        ramp: Some(RampSpec {
            effect: ModEffect::MaxHp(500),
            interval_ticks: RAMP_PER_ROUND,
        }),
    },
    // Mana Shield (A0EQ): "+1000 Mana Shield".
    ModifierDef {
        name: "Mana Shield",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::ManaShield(1000, 5)],
        ramp: None,
    },
    // representative ramping-regen item (no exact source; closest named regen items
    // are Renew / Repair Crew). Kept as a slice.
    ModifierDef {
        name: "Mending Engine (+120 Regen, +30/round)",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::HpRegen(120)],
        ramp: Some(RampSpec {
            effect: ModEffect::HpRegen(30),
            interval_ticks: RAMP_PER_ROUND,
        }),
    },
    // GEN-MODIFIERS-END
    // EXPANSION batch E1 — four NEW source upgrades with DYNAMIC damage scalers
    // (resolved live at fire time, never baked at purchase). The source map prices
    // upgrades via a separate in-game gold system with no per-item cost in the
    // catalog, so cost/rarity follow the batch guidance (rarity 2-3 / cost
    // 3000-5000 for these scaling items) — flagged as a judgement call.
    // Mastercrafted Masonry (A0CR): "+5000 Max HP | +1% Damage per 2000 Max HP
    // Gained". The damage half scales LIVE with the tank's current Max HP (so later
    // Max-HP buys retroactively boost it), via DamagePerMaxHp.
    ModifierDef {
        name: "Mastercrafted Masonry",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::MaxHp(5000), ModEffect::DamagePerMaxHp(1, 100)],
        ramp: None,
    },
    // Golden Ring (A0H3): "+200% Kill Bounty | +1% Damage per 50% Kill Bounty". The
    // damage half scales LIVE with the bounty multiplier (above the 1.0 base), via
    // DamagePerBountyPct. rarity 3 / cost 5000 (a top-end bounty-snowball payoff).
    ModifierDef {
        name: "Golden Ring",
        rarity: 3,
        cost: 5000,
        effects: &[
            ModEffect::BountyPct(200, 100),
            ModEffect::DamagePerBountyPct(1, 100),
        ],
        ramp: None,
    },
    // Arcane Mark (A0F3): "+4000 Mana Shield | +20% Damage while Mana Shield is
    // active". The offensive mirror of Energy Shield's defensive +DR. Shield regen
    // 20/tick matches Energy Shield's pool→regen ratio (10000→50, i.e. 4000→20).
    // rarity 3 / cost 5000.
    ModifierDef {
        name: "Arcane Mark",
        rarity: 3,
        cost: 5000,
        effects: &[
            ModEffect::ManaShield(4000, 20),
            ModEffect::ShieldActiveDamagePct(20, 100),
        ],
        ramp: None,
    },
    // Maw of Death (A0EU): "+2000 Mana Shield | +15 Mana regenerated when an enemy
    // dies". The shield half uses Moonwell's 2000→10/tick regen ratio; the on-kill
    // half restores 15 to the shield per kill (cap-respecting). rarity 2 / cost 3000.
    ModifierDef {
        name: "Maw of Death",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::ManaShield(2000, 10), ModEffect::ManaOnKill(15)],
        ramp: None,
    },
    // EXPANSION batch E2 — four MEDIUM-RISK exotic mechanics from the source map.
    // The source prices upgrades via a separate in-game gold system with no per-item
    // cost in the catalog, so cost/rarity follow the batch guidance (rarity 2-3 /
    // cost 3000-5000) — flagged as a judgement call. Each is ADD-only; no existing
    // entry's effects are touched.
    // Energy Pulse (A0F0/F1/F2): "+2000 Mana Shield | +0.5 s of Stun to enemies in
    // 1200 range when the Mana Shield de-activates." The shield half uses Moonwell's
    // 2000→10/tick regen ratio; the stun half fires on the shield's >0→0 down-edge
    // (range 1200 source units; 0.5 s = 15 ticks @ 30 Hz). rarity 2 / cost 3000.
    ModifierDef {
        name: "Energy Pulse",
        rarity: 2,
        cost: 3000,
        effects: &[
            ModEffect::ManaShield(2000, 10),
            ModEffect::ShieldBreakStun(1200, 15),
        ],
        ramp: None,
    },
    // Poison Armor (A0DE/F/G): "+10 Armor | +40 Poison damage per second for 3 s to
    // an enemy when damaged." Modeled as flat armor + a Spikes-applied Poison DoT on
    // the reflected attacker (reuses the existing poison status). Source "40/s for
    // 3 s": at 30 Hz that is 40/30 ≈ 1.33/tick; rounded UP to an integer 2/tick over
    // 90 ticks (≈60/s) so the DoT is integer-meaningful — flagged as a judgement
    // call. Pairs with a small flat Spikes so retaliation can land the poison.
    // rarity 1 / cost 1500.
    ModifierDef {
        name: "Poison Armor",
        rarity: 1,
        cost: 1500,
        effects: &[
            ModEffect::Armor(10),
            ModEffect::SpikesFlat(40),
            ModEffect::SpikesPoison(2, 90),
        ],
        ramp: None,
    },
    // Bloody Spikes (A0KT): "+80 Spikes Damage | +10 Spikes damage per second to an
    // enemy when damaged, stacking; each stack adds +100% more." The source is a
    // per-second stacking DoT with no cap; here it is adapted to the per-HIT Spikes
    // model as an accumulating flat bonus: each landed hit adds one stack (capped at
    // 25, matching the Frost-stack ceiling) and the bonus spikes damage is
    // `20 × stacks` (up to +500). Resets at the round boundary (matching the
    // Spiky/Growing-Spikes "resets when a new shop is made"). rarity 2 / cost 3000 —
    // flagged as a judgement call for the per-hit adaptation + cap/reset.
    ModifierDef {
        name: "Bloody Spikes",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::SpikesFlat(80), ModEffect::StackingSpikes(20, 25)],
        ramp: None,
    },
    // Blight Aura (A0CP/T/U): "+200 HP Regen | Deal 200 Poison damage to all enemies
    // in 600 range every 1 second." Modeled as a periodic AoE: every 30 ticks (1 s @
    // 30 Hz) deal 200 damage AND apply a Poison DoT (2/tick × 90 ticks, mirroring
    // Poison Armor's DoT) to every enemy within 600 source units, in stable id order.
    // The +200 HP Regen half is modeled too (Repair-Crew-style flat regen).
    // rarity 3 / cost 5000.
    ModifierDef {
        name: "Blight Aura",
        rarity: 3,
        cost: 5000,
        effects: &[ModEffect::HpRegen(200), ModEffect::DamageAura(600, 30, 200)],
        ramp: None,
    },
    // EXPANSION E3 (fidelity-mechanics pass) — Deep Freeze becomes an OPT-IN
    // upgrade, as in the source: "+1.5 seconds of Freeze when an enemy reaches
    // 25 stacks of Frost, resetting stacks to 0. Freeze increases Frost damage
    // taken by 50% and freezes the enemy in place." WITHOUT this modifier, 25
    // frost stacks now merely cap at max slow (baseline behavior change —
    // deliberate; the freeze payoff used to be always-on). Cost/rarity are a
    // judgement call (the source prices upgrades out-of-catalog).
    ModifierDef {
        name: "Deep Freeze",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::GrantDeepFreeze],
        ramp: None,
    },
    // ---- RESTORED UPGRADES (catalog-fidelity pass) ----------------------------
    // Source upgrades the shipped catalog had dropped, composed from existing
    // ModEffects. The source prices upgrades out-of-catalog, so each is placed
    // on the invented rarity/cost ladder by power (documented per entry).
    // Improved Attacks (source: "+5% Normal / Piercing / Magic / Siege / Chaos
    // Damage") — the all-types stat line; rarity 1 (five 5% effects in one buy
    // outvalues a single common).
    ModifierDef {
        name: "Improved Attacks",
        rarity: 0,
        cost: 500,
        effects: &[
            ModEffect::DamageTypePct(DMG_NORMAL, 5, 100),
            ModEffect::DamageTypePct(DMG_PIERCING, 5, 100),
            ModEffect::DamageTypePct(DMG_MAGIC, 5, 100),
            ModEffect::DamageTypePct(DMG_SIEGE, 5, 100),
            ModEffect::DamageTypePct(DMG_CHAOS, 5, 100),
        ],
        ramp: None,
    },
    // Scattershot (source: "+25% Enemies hit by Bounce and Barrage") — a
    // build-defining multi-hit multiplier; rarity 2 like the other
    // build-around scalers.
    ModifierDef {
        name: "Scattershot",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::BounceBarragePct(25, 100)],
        ramp: None,
    },
    // Biting Cold (source: "+10% Frost damage and slow strength") — the frost
    // twin of Potent Poison; rarity 0 to match it.
    ModifierDef {
        name: "Biting Cold",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::FrostDamagePct(10, 100)],
        ramp: None,
    },
    // Stoked Flames (source: "+10% Fire damage and damage vulnerability") —
    // the fire twin of Potent Poison; rarity 0 to match it.
    ModifierDef {
        name: "Stoked Flames",
        rarity: 0,
        cost: 500,
        effects: &[ModEffect::FireDamagePct(10, 100)],
        ramp: None,
    },
    // Warlord's Banner (source epic omni-buff: "+30% N/P/M/S/C/Spikes Damage,
    // +30% Damage to Stunned, +15% Poison, +15% Frost, +15% Fire").
    // APPROXIMATION (documented): composed exactly from the existing per-type
    // effects; the "+15% Fire" leg uses FireDamagePct which scales fire-stack
    // vulnerability + explosions (the engine's fire-damage model). rarity 3.
    ModifierDef {
        name: "Warlord's Banner",
        rarity: 3,
        cost: 5000,
        effects: &[
            ModEffect::DamageTypePct(DMG_NORMAL, 30, 100),
            ModEffect::DamageTypePct(DMG_PIERCING, 30, 100),
            ModEffect::DamageTypePct(DMG_MAGIC, 30, 100),
            ModEffect::DamageTypePct(DMG_SIEGE, 30, 100),
            ModEffect::DamageTypePct(DMG_CHAOS, 30, 100),
            ModEffect::SpikesPct(30, 100),
            ModEffect::DamageVsStunnedPct(30, 100),
            ModEffect::PoisonDamagePct(15, 100),
            ModEffect::FrostDamagePct(15, 100),
            ModEffect::FireDamagePct(15, 100),
        ],
        ramp: None,
    },
    // Wellspring (source: "+40 HP Regen | +100% of HP Regen health regenerated
    // over 10 seconds every 30 seconds"). APPROXIMATION (documented): the
    // periodic 10-s burst is amortized into a flat +33% HP-regen rider
    // (100% × 10 s / 30 s ≈ +33% average throughput) — no burst-heal engine is
    // added. rarity 1 (Renew's tier).
    ModifierDef {
        name: "Wellspring",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::HpRegen(40), ModEffect::HpRegenPct(33, 100)],
        ramp: None,
    },
    // Magic Seeds (source name, generic: "+1000 Max HP | +5 Max HP every
    // second") — the per-second growth rides the per-round ramp (5/s × 30 s =
    // +150 Max HP per round). rarity 1.
    ModifierDef {
        name: "Magic Seeds",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::MaxHp(1000)],
        ramp: Some(RampSpec {
            effect: ModEffect::MaxHp(150),
            interval_ticks: RAMP_PER_ROUND,
        }),
    },
    // Ambush Barbs (source: "+80 Spikes Damage | +240 Spikes Damage on the
    // first attack") — the first-hit spike burst. rarity 1 (spikes-pack tier).
    ModifierDef {
        name: "Ambush Barbs",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::SpikesFlat(80), ModEffect::SpikesFirstHit(240)],
        ramp: None,
    },
    // Flaming Armor (source: "+160 Spikes Damage | +20 stacks of Fire to an
    // enemy when damaged"). rarity 2 (a payoff piece for fire builds).
    ModifierDef {
        name: "Flaming Armor",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::SpikesFlat(160), ModEffect::FireArmor(20)],
        ramp: None,
    },
    // Vengeplate (source: "+400 Spikes Damage | +30% of Damage Taken Spikes
    // Damage"). rarity 2 (the heavy spikes payoff).
    ModifierDef {
        name: "Vengeplate",
        rarity: 2,
        cost: 3000,
        effects: &[
            ModEffect::SpikesFlat(400),
            ModEffect::DamageTakenToSpikesPct(30, 100),
        ],
        ramp: None,
    },
    // Deflection (source name, generic: "+160 Spikes Damage | +5% of Spikes
    // Damage as Flat Damage Reduction, but cannot reduce more than 50% of an
    // attack" — the 50% cap is engine-side). rarity 2.
    ModifierDef {
        name: "Deflection",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::SpikesFlat(160), ModEffect::SpikesAsDrPct(5, 100)],
        ramp: None,
    },
    // Frost Armor (source name, generic: "+10 Armor | +2 stacks of Frost to an
    // enemy when damaged"). rarity 1 (Poison Armor's tier).
    ModifierDef {
        name: "Frost Armor",
        rarity: 1,
        cost: 1500,
        effects: &[ModEffect::Armor(10), ModEffect::FrostArmor(2)],
        ramp: None,
    },
    // Combustion (source name, generic: "+500% Fire Explosion damage").
    // rarity 2 (a fire-build payoff multiplier).
    ModifierDef {
        name: "Combustion",
        rarity: 2,
        cost: 3000,
        effects: &[ModEffect::CombustionPct(500, 100)],
        ramp: None,
    },
    // Battle Fervor (source: "+50% Healing | +35% Damage for Healing Weapons
    // and Upgrades when at 95% health or above"). APPROXIMATION (documented):
    // the +35% is modeled as a GLOBAL healthy-threshold damage bonus
    // (DamageWhileHealthyPct) rather than scoping to healing weapons only —
    // no healing-weapon damage scope exists. rarity 2.
    ModifierDef {
        name: "Battle Fervor",
        rarity: 2,
        cost: 3000,
        effects: &[
            ModEffect::HealingPct(50, 100),
            ModEffect::DamageWhileHealthyPct(35, 100),
        ],
        ramp: None,
    },
    // Chaotic Resonance (source: "+100% Chaos Damage | +1% Chaos Damage per
    // Chaos Orb") — the Bow/Mortar per-copy-scaler pattern. rarity 2.
    ModifierDef {
        name: "Chaotic Resonance",
        rarity: 2,
        cost: 3000,
        effects: &[
            ModEffect::DamageTypePct(DMG_CHAOS, 100, 100),
            ModEffect::DamagePerWeapon(CHAOS_ORB as i64, DMG_CHAOS as i64, 1),
        ],
        ramp: None,
    },
    // Arcane Resonance (source: "+100% Magic Damage | +1% Magic Damage per
    // Magic Missile"). rarity 2.
    ModifierDef {
        name: "Arcane Resonance",
        rarity: 2,
        cost: 3000,
        effects: &[
            ModEffect::DamageTypePct(DMG_MAGIC, 100, 100),
            ModEffect::DamagePerWeapon(MAGIC_MISSILE as i64, DMG_MAGIC as i64, 1),
        ],
        ramp: None,
    },
    // Axe Rack (source: "+100% Normal Damage | +1% Normal Damage per Throwing
    // Axe"). rarity 2.
    ModifierDef {
        name: "Axe Rack",
        rarity: 2,
        cost: 3000,
        effects: &[
            ModEffect::DamageTypePct(DMG_NORMAL, 100, 100),
            ModEffect::DamagePerWeapon(THROWING_AXES as i64, DMG_NORMAL as i64, 1),
        ],
        ramp: None,
    },
    // Free Reroll (source: the Free Reroll shop items; docs/01 §1.4 "Free
    // Reroll sources exist"). One banked reroll; rarity 1 — a slot-machine
    // pull priced against real mid-tier stats (escalating reroll costs make a
    // banked reroll worth more than a common).
    ModifierDef {
        name: "Free Reroll",
        rarity: 1,
        cost: 1000,
        effects: &[ModEffect::GrantFreeRerolls(1)],
        ramp: None,
    },
];

/// The weapon the tank starts with (index into [`WEAPONS`]).
pub const STARTING_WEAPON: u16 = 0;

/// Stable index of the self-scaling "Death Engine" generator weapon (referenced
/// by the "Overclocked Death Engine" modifier so it scales with its own count).
pub const DEATH_ENGINE: u16 = 10;

/// Stable indices of the weapons the restored per-copy damage scalers key on
/// (the source's "+1% X Damage per <weapon>" upgrades — same pattern as the
/// Bow/Mortar scalers, which predate these consts and use literal 0/1).
pub const MAGIC_MISSILE: u16 = 11;
pub const CHAOS_ORB: u16 = 14;
pub const THROWING_AXES: u16 = 15;

/// Weapon catalog (subset; stats adapted from Appendix A). Indices are stable —
/// `STARTING_WEAPON` and tests refer to them by position.
pub static WEAPONS: &[WeaponDef] = &[
    // 0 — Bow (the starting weapon): pure single-target piercing.
    WeaponDef {
        name: "Bow",
        rarity: 0,
        cost: 500,
        damage: 75,
        damage_type: DMG_PIERCING,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30, // 1.0s
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // 1 — Mortar Launcher: siege splash.
    WeaponDef {
        name: "Mortar Launcher",
        rarity: 0,
        cost: 500,
        damage: 210,
        damage_type: DMG_SIEGE,
        attack: Attack::Splash(300),
        cooldown_ticks: 30, // 1.0s source tier — STEADY ANCHOR: cheap reliable splash floor
        // (fidelity pass: the drifted 1.6 s cd snapped DOWN to the 1.0 s tier,
        // DPS-neutral — the 2.0 s tier made the early splash floor too chunky
        // vs chaff and broke the modest-opener balance guard)
        range: 1200,
        proj_speed: 30,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // 2 — Frost Bow: applies Frost stacks (slow).
    WeaponDef {
        name: "Frost Bow",
        rarity: 1,
        cost: 1500,
        damage: 130,
        damage_type: DMG_MAGIC,
        attack: Attack::SingleTarget,
        cooldown_ticks: 15, // 0.5s
        range: 900,
        proj_speed: 50,
        // HIGH-CEILING COMBO ENABLER: modest direct damage, but a rapid frost
        // stacker that drives a single target to the 25-stack FREEZE payoff fast
        // (7/hit → freeze in ~4 hits). Floor is mediocre solo; ceiling is huge
        // once the freeze-at-25 lock lands or paired with Damage-to-Frozen.
        on_hit: StatusOnHit {
            frost_stacks: 7,
            ..StatusOnHit::NONE
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // 3 — Poison Bow: light hit + a strong damage-over-time.
    WeaponDef {
        name: "Poison Bow",
        rarity: 0,
        cost: 500,
        damage: 60,
        damage_type: DMG_PIERCING,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30,
        range: 900,
        proj_speed: 45,
        // STEADY ANCHOR: cheap, reliable damage-over-time floor. Scales with
        // Poison-damage / Damage-to-Poisoned mods but has a low solo ceiling.
        // (poison_dps pinned at 20 — production combat tests assert it.)
        on_hit: StatusOnHit {
            poison_dps: 20,
            poison_ticks: 90,
            ..StatusOnHit::NONE
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // 4 — Flamecaster: applies Fire stacks (vulnerability + explode on death).
    WeaponDef {
        name: "Flamecaster",
        rarity: 1,
        cost: 1500,
        damage: 85,
        damage_type: DMG_CHAOS,
        attack: Attack::Splash(150),
        cooldown_ticks: 15, // 0.5s source tier
        range: 600,
        proj_speed: 40,
        // HIGH-CEILING FIRE ENABLER: low direct damage, but splashes heavy fire
        // stacks across a pack. With the explode-on-death payoff this chain-
        // detonates whole waves (massive ceiling); without setup it is a modest
        // short-range splasher (real but unspectacular floor).
        on_hit: StatusOnHit {
            fire_stacks: 6,
            ..StatusOnHit::NONE
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // 5 — Storm Hammer: a heavy stunning splash (FIDELITY: the source block is
    // "Normal Splash(300) 500, cd 2.0, r 900, Stun 2 s" — the shipped Magic
    // SingleTarget shape was a same-name mismatch; damage re-anchored to our
    // HP scale, shape/stun restored).
    WeaponDef {
        name: "Storm Hammer",
        rarity: 2,
        cost: 3000,
        damage: 1050,
        damage_type: DMG_NORMAL,
        attack: Attack::Splash(300),
        cooldown_ticks: 60, // 2.0s source tier
        range: 900,
        proj_speed: 40,
        // HIGH-CEILING BURST: a wide crater that stun-locks a pack for 2 s.
        // Slow between throws; combos explosively with Damage-to-Stunned.
        on_hit: StatusOnHit {
            stun_ticks: 60,
            ..StatusOnHit::NONE
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // 6 — Ballista: the ALPHA-STRIKE volley (FIDELITY: source is "Piercing
    // Barrage(8), 2500, cd 5.0, r 1200, Stun 3 s" — shipped had flattened it
    // into a fast Barrage(4) chip weapon; the high-per-volley/slow identity is
    // restored, damage re-anchored to our HP scale).
    WeaponDef {
        name: "Ballista",
        rarity: 1,
        cost: 1500,
        damage: 450,
        damage_type: DMG_PIERCING,
        attack: Attack::Barrage(8),
        cooldown_ticks: 150, // 5.0s source tier
        range: 1200,
        proj_speed: 50,
        // BOOM-OR-BUST: eight stunning bolts, then a long reload.
        on_hit: StatusOnHit {
            stun_ticks: 90,
            ..StatusOnHit::NONE
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // 7 — Immolation: an instant Area pulse around the tank that burns.
    WeaponDef {
        name: "Immolation",
        rarity: 1,
        cost: 1500,
        damage: 100,
        damage_type: DMG_CHAOS,
        attack: Attack::Area(300),
        cooldown_ticks: 30,
        range: 300,
        proj_speed: 0,
        // FIDELITY: the source pulse is "200 dmg + Fire (20 stacks)" — a 10:1
        // dmg:stacks ratio of HEAVY fire stacking; restored at our damage
        // anchor (100 dmg : 10 stacks; shipped had flattened it to 2 stacks).
        on_hit: StatusOnHit {
            fire_stacks: 10,
            ..StatusOnHit::NONE
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // 8 — Shockwave Axe: an instant sweeping Wave.
    WeaponDef {
        name: "Shockwave Axe",
        rarity: 2,
        cost: 3000,
        damage: 900,
        damage_type: DMG_NORMAL,
        attack: Attack::Wave(300),
        cooldown_ticks: 60,
        range: 300,
        proj_speed: 0,
        // HIGH-CEILING BOARD-WIPE: hits everything in a wide sweep for big damage,
        // but on a long cooldown and only at point-blank — between sweeps the tank
        // eats the wave, so the floor is risky; the ceiling clears packs outright.
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // 9 — Moon Glaive: an instant Bounce chaining to nearby enemies (FIDELITY:
    // source is "Piercing Bounce(4), 100, cd 0.5, r 300" — the fast/short
    // rhythm is restored; DPS preserved by moving (cd, dmg) onto the tier).
    WeaponDef {
        name: "Moon Glaive",
        rarity: 1,
        cost: 1500,
        damage: 90,
        damage_type: DMG_PIERCING,
        attack: Attack::Bounce(4),
        cooldown_ticks: 15, // 0.5s source tier
        range: 300,
        proj_speed: 0,
        // STEADY-MID ANCHOR: dependable instant chain to 4 nearby foes.
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // 10 — Death Engine: a self-scaling damage GENERATOR. Pairs with the
    // "Overclocked Death Engine" modifier (+10% Chaos per Death Engine), so its
    // damage ramps with the number of copies owned (the source's stacking
    // generators). Hand-authored with a STABLE index (referenced by that
    // modifier) — kept outside the GEN block.
    WeaponDef {
        name: "Death Engine",
        rarity: 2,
        cost: 3000,
        damage: 95,
        damage_type: DMG_CHAOS,
        attack: Attack::SingleTarget,
        cooldown_ticks: 10, // 0.33s source tier (the generator's fast chip rhythm)
        range: 1200,
        proj_speed: 45,
        // HIGH-CEILING SNOWBALL (boom-or-bust): a single copy is a weak, overpriced
        // single-target hit. But with "Overclocked Death Engine" (+10% Chaos per
        // copy) the count compounds on itself — N copies each deal ~(1 + N/10)×, so
        // the build is quadratic in copies. Commit hard and it runs away with the
        // game; buy one or two and it is a deliberate trap (the high-risk path).
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // GEN-WEAPONS-BEGIN (hand-rebalanced for SPIKY, high-ceiling/high-risk variety,
    // then re-anchored by the CATALOG-FIDELITY pass: cooldowns/ranges moved back
    // onto the source ladder — 0.5/1/2/3/5/8/10 s cds (plus the source's 0.2/0.33 s
    // fast tiers), 300/600/900/1200 ranges — with (cd, dmg) moved TOGETHER so each
    // weapon's DPS is roughly preserved while its RHYTHM matches the source: slow
    // heavy hitters slow again, fast chip weapons fast). DESIGN: rarity buys CEILING + VARIANCE, not
    // gold-efficiency. Commons = the safe, reliable, low-ceiling FLOOR (~2.5–4.5k
    // dmg/1000g, always-on). Rares/Epics fan across a WIDE ceiling: some pay off
    // enormously but are slow / point-blank / setup-gated / fragile (boom-or-bust);
    // a few are steadier anchors. Fire/Frost weapons are tuned HIGH-CEILING on the
    // assumption the explode-on-death / freeze-at-25 payoffs exist.
    // --- Single-target commons: the steady gold-efficiency floor (random-pick safe) ---
    WeaponDef {
        name: "Magic Missile",
        rarity: 0,
        cost: 500,
        damage: 110,
        damage_type: DMG_MAGIC, // FIDELITY: the source Magic Missile is MAGIC (was Normal)
        attack: Attack::SingleTarget,
        cooldown_ticks: 30, // 1.0s source tier
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Boulder",
        rarity: 0,
        cost: 500,
        damage: 120,
        damage_type: DMG_SIEGE,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30, // 1.0s source tier
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Magic Bolt",
        rarity: 0,
        cost: 500,
        damage: 110,
        damage_type: DMG_MAGIC,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30, // 1.0s source tier
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Chaos Orb",
        rarity: 0,
        cost: 500,
        damage: 105,
        damage_type: DMG_CHAOS,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30, // 1.0s source tier
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // FIDELITY: the source Throwing Axes are "Normal SingleTarget, 75, cd 1.0,
    // r 900" (one of the five plain commons); the Bounce identity belongs to
    // the Seeker Axe / Moon Glaive family. Restored to the common-ST band.
    WeaponDef {
        name: "Throwing Axes",
        rarity: 0,
        cost: 500,
        damage: 105,
        damage_type: DMG_NORMAL,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30, // 1.0s source tier
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Chaos Skulls",
        rarity: 0,
        cost: 500,
        damage: 35,
        damage_type: DMG_CHAOS,
        attack: Attack::Bounce(3),
        cooldown_ticks: 15, // 0.5s source tier
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // --- HIGH-CEILING RARES/EPICS: big payoff, real risk (slow / point-blank / setup) ---
    WeaponDef {
        name: "Suckula",
        rarity: 2,
        cost: 3000,
        damage: 1300,
        damage_type: DMG_CHAOS,
        attack: Attack::Wave(300),
        cooldown_ticks: 90, // 3.0s source tier
        range: 300,
        proj_speed: 0,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::LifeDrain { per_hit: 40 },
    }, // exotic: Heal (base only); point-blank board-wipe, slow
    WeaponDef {
        name: "Missile Barrage",
        rarity: 3,
        cost: 5000,
        damage: 1700,
        damage_type: DMG_PIERCING,
        attack: Attack::Barrage(8),
        cooldown_ticks: 90, // 3.0s source tier
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 90,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    }, // HIGH-CEILING: 8×1400 stun-volley, swingy on small boards
    WeaponDef {
        name: "Seeker Axe",
        rarity: 0,
        cost: 500,
        damage: 65,
        damage_type: DMG_PIERCING,
        attack: Attack::Bounce(3),
        cooldown_ticks: 30, // 1.0s source tier
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Steam Cannon",
        rarity: 1,
        cost: 1500,
        damage: 320,
        damage_type: DMG_SIEGE,
        attack: Attack::Splash(300),
        cooldown_ticks: 30,
        range: 300,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 15,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // FIDELITY: the source Demon Eye "reduces enemy damage by 50% for 10 s" —
    // a DEFENSIVE debuff, not a vulnerability. Obscure (a 50% miss chance for
    // 10 s) equals a 50% damage reduction in expectation and is the closest
    // shipped mechanic, so the eye now blinds instead of marking.
    WeaponDef {
        name: "Demon Eye",
        rarity: 2,
        cost: 3000,
        damage: 1300,
        damage_type: DMG_CHAOS,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30, // 1.0s source tier
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::Obscure {
            pct: 50,
            ticks: 300,
        },
    }, // exotic: blind (base only); high single-target ceiling
    WeaponDef {
        name: "Impaler",
        rarity: 1,
        cost: 1500,
        damage: 240,
        damage_type: DMG_PIERCING,
        attack: Attack::SingleTarget,
        cooldown_ticks: 15,
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 22,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Chaos Swarm",
        rarity: 0,
        cost: 500,
        damage: 105,
        damage_type: DMG_CHAOS,
        attack: Attack::Splash(300),
        cooldown_ticks: 30,
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Catapult",
        rarity: 1,
        cost: 1500,
        damage: 470,
        damage_type: DMG_SIEGE,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30, // 1.0s source tier
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Slap",
        rarity: 3,
        cost: 5000,
        damage: 3700,
        damage_type: DMG_PIERCING,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30, // 1.0s source tier (source: 4000 @ cd 1.0)
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::Knockback { dist: 300 },
    }, // exotic: Knockback (base only); GLASS-CANNON nuke, single-target only
    WeaponDef {
        name: "Crippler",
        rarity: 2,
        cost: 3000,
        damage: 1450,
        damage_type: DMG_PIERCING,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30, // 1.0s source tier
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::VulnOnHit { stacks: 8 },
    }, // exotic: permanent (base only)
    WeaponDef {
        name: "Lifeleecher",
        rarity: 2,
        cost: 3000,
        damage: 1150,
        damage_type: DMG_NORMAL,
        attack: Attack::SingleTarget,
        cooldown_ticks: 60, // 2.0s source tier
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::LifeDrain { per_hit: 40 },
    }, // exotic: Heal (base only)
    WeaponDef {
        name: "Spell Glaive",
        rarity: 1,
        cost: 1500,
        damage: 430,
        damage_type: DMG_MAGIC,
        attack: Attack::Bounce(4),
        cooldown_ticks: 60, // 2.0s source tier (source: Magic Bounce(4) 300 @ cd 2.0)
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Glaive Thrower",
        rarity: 0,
        cost: 500,
        damage: 80,
        damage_type: DMG_NORMAL,
        attack: Attack::Bounce(3),
        cooldown_ticks: 30, // 1.0s source tier
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Spikewheel Launcher",
        rarity: 1,
        cost: 1500,
        damage: 330,
        damage_type: DMG_SIEGE,
        attack: Attack::Bounce(6),
        cooldown_ticks: 60, // 2.0s source tier (source: Siege Bounce(8) 400 @ cd 2.0)
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Meatapult",
        rarity: 2,
        cost: 3000,
        damage: 1100,
        damage_type: DMG_NORMAL,
        attack: Attack::Splash(300),
        cooldown_ticks: 60, // 2.0s source tier
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 60,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Arcane Blaster",
        rarity: 1,
        cost: 1500,
        damage: 360,
        damage_type: DMG_MAGIC,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30,
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 45,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // FIDELITY: the source Quills are "Piercing & Spikes, Area (Enemies In
    // Range), 150, cd 2.0, r 600" — a spiny burst around the tank, not a
    // poisoned single-target dart (poison belongs to Living Spittle).
    WeaponDef {
        name: "Quills",
        rarity: 1,
        cost: 1500,
        damage: 230,
        damage_type: DMG_PIERCING,
        attack: Attack::Area(600),
        cooldown_ticks: 60, // 2.0s source tier
        range: 600,
        proj_speed: 0,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Living Spittle",
        rarity: 1,
        cost: 1500,
        damage: 290,
        damage_type: DMG_MAGIC,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30, // 1.0s source tier
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 8,
            poison_ticks: 90,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Poison Bomb",
        rarity: 1,
        cost: 1500,
        damage: 530,
        damage_type: DMG_SIEGE,
        attack: Attack::Splash(300),
        cooldown_ticks: 60, // 2.0s source tier
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 8,
            poison_ticks: 90,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Serpent",
        rarity: 2,
        cost: 3000,
        damage: 870,
        damage_type: DMG_NORMAL,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30, // 1.0s source tier
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 22,
            poison_ticks: 90,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    }, // poison-stacking ceiling
    WeaponDef {
        name: "Overloaded Catapult",
        rarity: 2,
        cost: 3000,
        damage: 650,
        damage_type: DMG_SIEGE,
        attack: Attack::Splash(300),
        cooldown_ticks: 30, // 1.0s source tier
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Chaos Claw",
        rarity: 1,
        cost: 1500,
        damage: 400,
        damage_type: DMG_CHAOS,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30, // 1.0s source tier (source: Chaos ST 250 @ cd 1.0, stun 1.5s)
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 45,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Net Thrower",
        rarity: 1,
        cost: 1500,
        damage: 230,
        damage_type: DMG_NORMAL,
        attack: Attack::SingleTarget,
        cooldown_ticks: 15,
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 22,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Thornburst",
        rarity: 0,
        cost: 500,
        damage: 80,
        damage_type: DMG_PIERCING,
        attack: Attack::Area(300),
        cooldown_ticks: 60, // 2.0s source tier (range snapped onto the 300 tier)
        range: 300,
        proj_speed: 0,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Chaotic Spirit",
        rarity: 3,
        cost: 5000,
        damage: 3200,
        damage_type: DMG_MAGIC,
        attack: Attack::Bounce(8),
        cooldown_ticks: 150, // 5.0s source tier (source: Magic Bounce(8) 1250 @ cd 5.0)
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    }, // HIGH-CEILING: 8× chain, slow
    WeaponDef {
        name: "Energy Pulse",
        rarity: 2,
        cost: 3000,
        damage: 1100,
        damage_type: DMG_MAGIC,
        attack: Attack::Wave(300),
        cooldown_ticks: 90, // 3.0s source tier (source: Magic Wave(+300) 800 @ cd 3.0, stun 2s)
        range: 300,
        proj_speed: 0,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 60,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Cluster Rockets",
        rarity: 2,
        cost: 3000,
        damage: 1050,
        damage_type: DMG_CHAOS,
        attack: Attack::Barrage(12),
        cooldown_ticks: 150, // 5.0s tier (source cd 4.0 sits between tiers; snapped up)
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    }, // HIGH-CEILING: 12-projectile saturation, swingy on thin boards
    WeaponDef {
        name: "Frost Bomb",
        rarity: 2,
        cost: 3000,
        damage: 300,
        damage_type: DMG_PIERCING,
        attack: Attack::Splash(150),
        cooldown_ticks: 15, // 0.5s source tier (source: Piercing&Frost Splash(150) 500 @ cd 0.5)
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 3,
            fire_stacks: 0,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // NOTE (catalog-fidelity audit): this entry wears the source's LIVING
    // WATER block — "Normal Bounce(4) 600, cd 3.0, r 900, Stun 3 s". The
    // audit's "restore Living Water" item is therefore already present under
    // this shipped name; no duplicate entry is added.
    WeaponDef {
        name: "Bouncy Cannonball",
        rarity: 2,
        cost: 3000,
        damage: 1250,
        damage_type: DMG_NORMAL,
        attack: Attack::Bounce(4),
        cooldown_ticks: 90, // 3.0s source tier
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 90,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // FIDELITY: Soulstealer had been wearing the source CHAIN-HEAL block (that
    // block now lives on Mendweaver, below). Its own identity per the source
    // changelog is "+20 Mana Shield per attack" on a long-range Magic bolt —
    // the per-attack shield grant rides ManaDrain (identical for a
    // single-target weapon: one enemy damaged per fire).
    WeaponDef {
        name: "Soulstealer",
        rarity: 3,
        cost: 5000,
        damage: 2000,
        damage_type: DMG_MAGIC,
        attack: Attack::SingleTarget,
        cooldown_ticks: 60, // 2.0s source tier
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::ManaDrain { per_hit: 20 },
    }, // exotic: shield-per-attack (base only); APEX EPIC single-target
    WeaponDef {
        name: "Splasher",
        rarity: 0,
        cost: 500,
        damage: 45,
        damage_type: DMG_NORMAL,
        attack: Attack::Splash(300),
        cooldown_ticks: 10, // 0.33s source tier (source: Normal Splash(300) 75 @ cd 0.33)
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Fire Bow",
        rarity: 1,
        cost: 1500,
        damage: 190,
        damage_type: DMG_PIERCING,
        attack: Attack::Barrage(4),
        cooldown_ticks: 30,
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 4,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    }, // fire enabler, ramps with explode payoff
    WeaponDef {
        name: "Chaos Web",
        rarity: 1,
        cost: 1500,
        damage: 150,
        damage_type: DMG_CHAOS,
        attack: Attack::Bounce(6),
        cooldown_ticks: 30, // 1.0s source tier (source: Chaos&Poison Bounce(8) 250 @ cd 1.0)
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 6,
            poison_ticks: 90,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Magic Claw",
        rarity: 1,
        cost: 1500,
        damage: 240,
        damage_type: DMG_MAGIC,
        attack: Attack::Bounce(4),
        cooldown_ticks: 30,
        range: 300,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::ManaDrain { per_hit: 20 },
    }, // exotic: Mana (base only)
    WeaponDef {
        name: "Liquid Fire Hurler",
        rarity: 1,
        cost: 1500,
        damage: 210,
        damage_type: DMG_SIEGE,
        attack: Attack::SingleTarget,
        cooldown_ticks: 10,
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 4,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        // ADAPTATION (documented): the source drench is "+25% FIRE damage
        // taken" — but Fire is a STATUS here, not one of the five matrix
        // damage types, so `VulnTypeOnHit` cannot key on it. The drench stays
        // a small GENERIC vulnerability (fire-stack damage routes through
        // generic vuln), which is the closest expressible shape.
        ability: WeaponAbility::VulnOnHit { stacks: 3 },
    }, // exotic: drench (base only); fast fire stacker
    WeaponDef {
        name: "Boulder Toss",
        rarity: 2,
        cost: 3000,
        damage: 1300,
        damage_type: DMG_NORMAL,
        attack: Attack::Splash(300),
        cooldown_ticks: 60, // 2.0s source tier
        range: 300,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::VulnOnHit { stacks: 5 },
    }, // exotic: reduce enemy (base only)
    WeaponDef {
        name: "Bloody Spikes",
        rarity: 3,
        cost: 5000,
        damage: 1850,
        damage_type: DMG_NORMAL,
        attack: Attack::Wave(300),
        cooldown_ticks: 30, // 1.0s source tier (source: Normal&Spikes Wave 5000 @ cd 1.0, stun 1s)
        range: 300,
        proj_speed: 0,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 30,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    }, // HIGH-CEILING wave nuke, point-blank
    WeaponDef {
        name: "Shroom Doom",
        rarity: 3,
        cost: 5000,
        damage: 8500,
        damage_type: DMG_CHAOS,
        attack: Attack::Area(300),
        cooldown_ticks: 240, // 8.0s source tier (the Infernal-summoner block's rhythm)
        range: 600,
        proj_speed: 0,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 60,
            stun_ticks: 60,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::Summon {
            kind: 1,
            hp: 1500,
            damage: 600,
        },
    }, // BOOM-OR-BUST: huge nuke + 60 fire, very slow cd; wiping a pack RAISES a host of Spores
    WeaponDef {
        name: "Flame Generator",
        rarity: 3,
        cost: 5000,
        damage: 4250,
        damage_type: DMG_MAGIC,
        attack: Attack::Area(300),
        cooldown_ticks: 150, // 5.0s source tier (source: Magic&Fire Area(300) 5000 @ cd 5.0)
        range: 600,
        proj_speed: 0,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 200,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::VulnOnHit { stacks: 4 },
    }, // exotic: damage taken (base only); FIRE PAYOFF ENGINE: drenches packs in 200 fire each pulse → explode-chain ceiling is enormous
    WeaponDef {
        name: "Firebreather",
        rarity: 1,
        cost: 1500,
        damage: 180,
        damage_type: DMG_PIERCING,
        attack: Attack::Splash(150),
        cooldown_ticks: 15,
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 5,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::VulnOnHit { stacks: 5 },
    }, // exotic: damage taken (base only); rapid fire stacker
    WeaponDef {
        name: "Lavaspitter",
        rarity: 3,
        cost: 5000,
        damage: 3600,
        damage_type: DMG_SIEGE,
        attack: Attack::Splash(300),
        cooldown_ticks: 90, // 3.0s source tier (source: Siege&Fire Splash(300) 1500 @ cd 3.0)
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 150,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::VulnOnHit { stacks: 5 },
    }, // exotic: damage taken (base only); long-range fire payoff
    WeaponDef {
        name: "Frostbolt",
        rarity: 1,
        cost: 1500,
        damage: 170,
        damage_type: DMG_PIERCING,
        attack: Attack::SingleTarget,
        cooldown_ticks: 15,
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 5,
            fire_stacks: 0,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    }, // frost enabler toward freeze-at-25
    WeaponDef {
        name: "Living Ice",
        rarity: 1,
        cost: 1500,
        damage: 300,
        damage_type: DMG_MAGIC,
        attack: Attack::Splash(150),
        cooldown_ticks: 30,
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 2,
            fire_stacks: 0,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Ice Generator",
        rarity: 3,
        cost: 5000,
        damage: 1000,
        damage_type: DMG_MAGIC,
        attack: Attack::Area(375),
        cooldown_ticks: 45, // internal period — "Attack Cooldown: N/A" class (fixed_rate)
        range: 900,
        proj_speed: 0,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 5,
            fire_stacks: 0,
            stun_ticks: 0,
        },
        // FIDELITY: the source ice-shard wave is the "Attack Cooldown: N/A"
        // class — the frost-wave family does NOT benefit from +% Attack Speed
        // (docs/01 §1.3 must-preserve).
        fixed_rate: true,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    }, // FROST PAYOFF ENGINE: AoE 5-stacks → mass-freeze ceiling
    WeaponDef {
        name: "Ice Spears",
        rarity: 2,
        cost: 3000,
        damage: 550,
        damage_type: DMG_NORMAL,
        attack: Attack::Barrage(4),
        cooldown_ticks: 45, // 1.5s source tier (source: Normal&Frost Barrage(4) 450 @ cd 1.5)
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 5,
            fire_stacks: 0,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Knives",
        rarity: 0,
        cost: 500,
        damage: 65,
        damage_type: DMG_PIERCING,
        attack: Attack::Barrage(3),
        cooldown_ticks: 15, // 0.5s source tier
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Blaster",
        rarity: 1,
        cost: 1500,
        damage: 210,
        damage_type: DMG_SIEGE,
        attack: Attack::SingleTarget,
        cooldown_ticks: 15,
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        // FIDELITY: the source Steam-Cannon-role block stacks SIEGE
        // vulnerability specifically ("+5% Siege damage taken, stacking").
        ability: WeaponAbility::VulnTypeOnHit {
            dmg_type: DMG_SIEGE,
            stacks: 5,
        },
    }, // exotic: typed siege vulnerability (base only)
    WeaponDef {
        name: "Bandit Sniper",
        rarity: 1,
        cost: 1500,
        damage: 200,
        damage_type: DMG_NORMAL,
        attack: Attack::SingleTarget,
        cooldown_ticks: 15, // 0.5s source tier
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::VulnOnHit { stacks: 5 },
    }, // exotic: damage taken (base only)
    WeaponDef {
        name: "Bombs",
        rarity: 1,
        cost: 1500,
        damage: 300,
        damage_type: DMG_SIEGE,
        attack: Attack::Splash(300),
        cooldown_ticks: 30,
        range: 300,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 15,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Sting",
        rarity: 1,
        cost: 1500,
        damage: 370,
        damage_type: DMG_CHAOS,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30, // 1.0s source tier
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        // FIDELITY: the source block stacks CHAOS vulnerability specifically
        // ("+10% Chaos damage taken, stacking").
        ability: WeaponAbility::VulnTypeOnHit {
            dmg_type: DMG_CHAOS,
            stacks: 10,
        },
    }, // exotic: typed chaos vulnerability (base only)
    WeaponDef {
        name: "Chaos Skull Bomb",
        rarity: 2,
        cost: 3000,
        damage: 1450,
        damage_type: DMG_CHAOS,
        attack: Attack::Splash(300),
        cooldown_ticks: 90, // 3.0s source tier (source: Chaos Splash 450 @ cd 3.0, stun 1.5s)
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 45,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Icebreather",
        rarity: 2,
        cost: 3000,
        damage: 1500,
        damage_type: DMG_SIEGE,
        attack: Attack::Splash(300),
        cooldown_ticks: 90, // 3.0s source tier (source: Siege&Frost Splash(300) 600 @ cd 3.0)
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 2,
            fire_stacks: 0,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::VulnOnHit { stacks: 4 },
    }, // exotic: explode (base only)
    // FIDELITY: the source frost wave "attacks in a COUNTERCLOCKWISE rotating
    // pattern" (Magic & Frost Wave(+150), 500 @ cd 2.0, Frost 3) and the frost
    // wave family does not benefit from +% Attack Speed (docs/01 §1.3).
    WeaponDef {
        name: "Frostwave",
        rarity: 2,
        cost: 3000,
        damage: 950,
        damage_type: DMG_MAGIC,
        attack: Attack::WaveRotating(150, false),
        cooldown_ticks: 60, // 2.0s source tier
        range: 300,
        proj_speed: 0,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 3,
            fire_stacks: 0,
            stun_ticks: 0,
        },
        fixed_rate: true,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // FIDELITY: the source fire wave "attacks in a CLOCKWISE rotating pattern"
    // (Normal & Fire Wave(+150), 400 @ cd 2.0, Fire 20) and — unlike the frost
    // family — DOES benefit from +% Attack Speed (source changelog). The old
    // generic-vuln rider is dropped: the heavy Fire stacks carry the
    // vulnerability themselves.
    WeaponDef {
        name: "Flamewave",
        rarity: 1,
        cost: 1500,
        damage: 480,
        damage_type: DMG_NORMAL,
        attack: Attack::WaveRotating(150, true),
        cooldown_ticks: 60, // 2.0s source tier
        range: 300,
        proj_speed: 0,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 20,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Chaotic Spirit Bolt",
        rarity: 1,
        cost: 1500,
        damage: 190,
        damage_type: DMG_CHAOS,
        attack: Attack::SingleTarget,
        cooldown_ticks: 10,
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::LifeDrain { per_hit: 40 },
    }, // exotic: Heal (base only); fast cheap floor
    WeaponDef {
        name: "Manabolt",
        rarity: 1,
        cost: 1500,
        damage: 320,
        damage_type: DMG_MAGIC,
        attack: Attack::SingleTarget,
        cooldown_ticks: 10,
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::ManaDrain { per_hit: 80 },
    }, // exotic: drain (base only); fast long-range
    WeaponDef {
        name: "Squirm",
        rarity: 3,
        cost: 5000,
        damage: 1100,
        damage_type: DMG_CHAOS,
        attack: Attack::SingleTarget,
        cooldown_ticks: 15, // 0.5s source tier
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::Summon {
            kind: 0,
            hp: 500,
            damage: 250,
        },
    }, // steady high-rarity anchor; every kill RAISES a Larva
    WeaponDef {
        name: "Immolation Aura",
        rarity: 0,
        cost: 500,
        damage: 40,
        damage_type: DMG_MAGIC,
        attack: Attack::Wave(150),
        cooldown_ticks: 6, // 0.2s source tier (the "constantly attacking" fire wave)
        range: 300,
        proj_speed: 0,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 2,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::VulnOnHit { stacks: 3 },
    }, // exotic: damage taken (base only); cheap point-blank pulse
    WeaponDef {
        name: "Boom Bloom",
        rarity: 3,
        cost: 5000,
        damage: 3500,
        damage_type: DMG_SIEGE,
        attack: Attack::Wave(200),
        cooldown_ticks: 60, // 2.0s tier
        range: 300,
        proj_speed: 0,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 90,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::Hazard {
            dmg: 1000,
            radius: 200,
            ticks: 90,
        },
    }, // exotic: mine field (base only); point-blank stun-wave nuke
    WeaponDef {
        name: "Quill Burst",
        rarity: 1,
        cost: 1500,
        damage: 450,
        damage_type: DMG_PIERCING,
        attack: Attack::Splash(300),
        cooldown_ticks: 60, // 2.0s source tier (source: Piercing Splash(300) 400 @ cd 2.0)
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        // FIDELITY: the source block stacks PIERCING vulnerability ("+10%
        // Piercing damage taken, stacking").
        ability: WeaponAbility::VulnTypeOnHit {
            dmg_type: DMG_PIERCING,
            stacks: 10,
        },
    }, // exotic: typed piercing vulnerability (base only)
    WeaponDef {
        name: "Arcane Burst",
        rarity: 1,
        cost: 1500,
        damage: 480,
        damage_type: DMG_MAGIC,
        attack: Attack::Splash(300),
        cooldown_ticks: 60, // 2.0s source tier (source: Magic Splash(300) 400 @ cd 2.0)
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        // FIDELITY: the source block stacks MAGIC vulnerability ("+10% Magic
        // damage taken, stacking").
        ability: WeaponAbility::VulnTypeOnHit {
            dmg_type: DMG_MAGIC,
            stacks: 10,
        },
    }, // exotic: typed magic vulnerability (base only)
    // FIDELITY: the source block is "Siege Barrage(8) & SPLASH (300), 5000 @
    // cd 10.0, r 1200" — the shipped entry had dropped the splash half. The
    // combined shape and the 10 s alpha rhythm are restored (damage moved onto
    // the tier DPS-neutrally; the Siege-Volley mechanic anchor this shape
    // premiered on is repurposed into the restored burning-oil weapon below).
    WeaponDef {
        name: "Meteor Barrage",
        rarity: 3,
        cost: 5000,
        damage: 8500,
        damage_type: DMG_SIEGE,
        attack: Attack::BarrageSplash(8, 300),
        cooldown_ticks: 300, // 10.0s source tier
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    }, // BOOM-OR-BUST: 8 cratering meteors on a very long cooldown — feast (whole-screen wipe) or famine (caught reloading)
    // FIDELITY: the real Ale Launcher is "Normal Splash(300), 900 @ cd 3.0,
    // r 300; attacks reduce enemy chance to hit by 25% for 3 s" — a slow
    // point-blank keg that BLINDS the pack. The heal-splash block it had been
    // wearing moves to the restored Healing Sprayer (below).
    WeaponDef {
        name: "Ale Launcher",
        rarity: 1,
        cost: 1500,
        damage: 700,
        damage_type: DMG_NORMAL,
        attack: Attack::Splash(300),
        cooldown_ticks: 90, // 3.0s source tier
        range: 300,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::Obscure { pct: 25, ticks: 90 },
    }, // exotic: miss-chance (base only)
    WeaponDef {
        name: "Chaos Bolt",
        rarity: 1,
        cost: 1500,
        damage: 290,
        damage_type: DMG_CHAOS,
        attack: Attack::SingleTarget,
        cooldown_ticks: 10,
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    WeaponDef {
        name: "Rotating Orb of Lightning",
        rarity: 2,
        cost: 3000,
        damage: 360,
        damage_type: DMG_MAGIC,
        attack: Attack::Area(600),
        cooldown_ticks: 30,
        range: 600,
        proj_speed: 0,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    }, // wide always-on aura, steady rare anchor
    WeaponDef {
        name: "Lightning Generator",
        rarity: 2,
        cost: 3000,
        damage: 780,
        damage_type: DMG_MAGIC,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30,
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::ManaDrain { per_hit: 20 },
    }, // exotic: Mana (base only)
    WeaponDef {
        name: "Flame Nova",
        rarity: 1,
        cost: 1500,
        damage: 190,
        damage_type: DMG_CHAOS,
        attack: Attack::Area(300),
        cooldown_ticks: 30, // 1.0s source tier (source: Chaos&Fire Area 200 @ cd 1.0, Fire 20)
        range: 300,
        proj_speed: 0,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 20,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::VulnOnHit { stacks: 6 },
    }, // exotic: damage taken (base only); fire-payoff AoE
    WeaponDef {
        name: "Shocker",
        rarity: 1,
        cost: 1500,
        damage: 110,
        damage_type: DMG_SIEGE,
        attack: Attack::Area(300),
        cooldown_ticks: 15, // 0.5s tier (source rocket-stream is cd 0.25; snapped to the ladder)
        range: 600,
        proj_speed: 0,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 60,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    }, // fast perma-stun aura
    WeaponDef {
        name: "Tangle",
        rarity: 3,
        cost: 5000,
        damage: 1100,
        damage_type: DMG_NORMAL,
        attack: Attack::SingleTarget,
        cooldown_ticks: 10, // 0.33s source tier (source: Normal&Poison ST 1500 @ cd 0.334)
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::Root { ticks: 30 },
    }, // exotic: Root (base only); fast steady epic anchor
    // GEN-WEAPONS-END
    // ---- MECHANIC ANCHORS (E3 fidelity-mechanics pass, resolved by the
    // CATALOG-FIDELITY pass) ----------------------------------------------------
    // Entries exercising the NEW engine mechanics (combined attack shapes,
    // fixed-rate cooldowns, once-per-round bursts, rotating waves, per-attack
    // abilities). Monsoon/Healthstone/Holy Bolt/Maelstrom/Splitting Glaive are
    // the proper source-faithful entries; slot 87 was repurposed (see below).
    // Unit tests reference these by name (stable 86..=91 indices).
    // 86 — source "Bounce (4 Targets) & Splash (150), dmg 150, cd 1.0, r 900".
    WeaponDef {
        name: "Splitting Glaive",
        rarity: 1,
        cost: 1500,
        damage: 150,
        damage_type: DMG_PIERCING,
        attack: Attack::BounceSplash(4, 150),
        cooldown_ticks: 30,
        range: 900,
        proj_speed: 0,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // 87 — Cinderwagon (REPURPOSED anchor slot; the Barrage&Splash shape it
    // premiered moved onto Meteor Barrage, whose source block it is). This is
    // the restored BURNING-OIL siege engine: "Siege & Fire Splash(300), 1500 @
    // cd 3.0, r 1200; attacks leave burning oil for 3 s (1000 dmg/s); Fire
    // (150 stacks, 75 on attack + 75 over time)". Damage re-anchored; the oil
    // rides the existing Hazard ability (35/tick ≈ 1050/s for 90 ticks); the
    // over-time half of the fire stacks collapses into the on-attack 75.
    // (Original name — the source's WC3-unit name is not reused.)
    WeaponDef {
        name: "Cinderwagon",
        rarity: 2,
        cost: 3000,
        damage: 900,
        damage_type: DMG_SIEGE,
        attack: Attack::Splash(300),
        cooldown_ticks: 90, // 3.0s source tier
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 0,
            fire_stacks: 75,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::Hazard {
            dmg: 35,
            radius: 300,
            ticks: 90,
        },
    },
    // 88 — Monsoon (source: "Area (900), cd 1.0 (While Active); activates once
    // per round to damage all enemies in range every 1 s for 10 s"). Fires on
    // its internal 1 s period for the first 300 ticks of each round, then
    // sleeps; `fixed_rate` because its period is not an attack-speed cooldown.
    WeaponDef {
        name: "Monsoon",
        rarity: 2,
        cost: 3000,
        damage: 600,
        damage_type: DMG_MAGIC,
        attack: Attack::Area(900),
        cooldown_ticks: 30,
        range: 900,
        proj_speed: 0,
        on_hit: StatusOnHit::NONE,
        fixed_rate: true,
        round_burst_ticks: 300,
        ability: WeaponAbility::None,
    },
    // 89 — Healthstone (source: "Attacks grant +0.2 permanent HP Regen and 60
    // Instant HP Regen"; dmg 1000, cd 1.0, r 600). +0.2/s = 200 milli/s.
    WeaponDef {
        name: "Healthstone",
        rarity: 2,
        cost: 3000,
        damage: 1000,
        damage_type: DMG_PIERCING,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30,
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::RegenOnAttack {
            regen_milli_per_s: 200,
            instant: 60,
        },
    },
    // 90 — Holy Bolt (source: "dmg 500, cd 2.0, r 1200; Heal 80 health" — per
    // ATTACK, not per enemy hit; distinct from LifeDrain).
    WeaponDef {
        name: "Holy Bolt",
        rarity: 1,
        cost: 1500,
        damage: 500,
        damage_type: DMG_NORMAL,
        attack: Attack::SingleTarget,
        cooldown_ticks: 60,
        range: 1200,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::HealOnAttack { amount: 80 },
    },
    // 91 — rotating frost wave (source: "Magic & Frost, Wave (+150),
    // counterclockwise rotating pattern, Frost (3 stacks)"; the N/A-cooldown
    // wave class ⇒ `fixed_rate`). The sweep mechanics live in
    // `combat::tick_sweeps`.
    WeaponDef {
        name: "Maelstrom",
        rarity: 2,
        cost: 3000,
        damage: 300,
        damage_type: DMG_MAGIC,
        attack: Attack::WaveRotating(150, false),
        cooldown_ticks: 60,
        range: 300,
        proj_speed: 0,
        on_hit: StatusOnHit {
            poison_dps: 0,
            poison_ticks: 0,
            frost_stacks: 3,
            fire_stacks: 0,
            stun_ticks: 0,
        },
        fixed_rate: true,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
    // ---- RESTORED WEAPONS (catalog-fidelity pass) ----------------------------
    // Source blocks the shipped roster had dropped, appended with stable
    // indices 92+. Names are original where the source name was WC3-specific;
    // generic fantasy names the source used are kept.
    // 92 — Mendweaver: the restored CHAIN-HEAL block ("Normal Bounce(4) 7500 @
    // cd 3.0, r 600; Heal 200 per enemy hit") — previously worn by
    // Soulstealer. Damage re-anchored into the epic band; the 200-per-hit heal
    // is kept verbatim.
    WeaponDef {
        name: "Mendweaver",
        rarity: 3,
        cost: 5000,
        damage: 3000,
        damage_type: DMG_NORMAL,
        attack: Attack::Bounce(4),
        cooldown_ticks: 90, // 3.0s source tier
        range: 600,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::LifeDrain { per_hit: 200 },
    },
    // 93 — Healing Sprayer (source name, generic): "Siege Splash(150), 100 @
    // cd 0.5, r 900; Heal 4 per enemy hit" — the fast heal-splash block the
    // shipped Ale Launcher had been wearing.
    WeaponDef {
        name: "Healing Sprayer",
        rarity: 1,
        cost: 1500,
        damage: 120,
        damage_type: DMG_SIEGE,
        attack: Attack::Splash(150),
        cooldown_ticks: 15, // 0.5s source tier
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::LifeDrain { per_hit: 4 },
    },
    // 94 — Thorn (source name, generic): "Normal SingleTarget, 100 @ cd 0.5,
    // r 900; attacks increase NORMAL damage taken by 5%, stacking" — the
    // typed-vulnerability chip weapon, restored with the typed mechanic.
    WeaponDef {
        name: "Thorn",
        rarity: 1,
        cost: 1500,
        damage: 100,
        damage_type: DMG_NORMAL,
        attack: Attack::SingleTarget,
        cooldown_ticks: 15, // 0.5s source tier
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::VulnTypeOnHit {
            dmg_type: DMG_NORMAL,
            stacks: 5,
        },
    },
    // 95 — Poison Spear (source name, generic): "Piercing & Poison Barrage(4),
    // 50 @ cd 0.5, r 900" — the fast poison-flavored volley common.
    WeaponDef {
        name: "Poison Spear",
        rarity: 0,
        cost: 500,
        damage: 55,
        damage_type: DMG_PIERCING,
        attack: Attack::Barrage(4),
        cooldown_ticks: 15, // 0.5s source tier
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit {
            poison_dps: 5,
            poison_ticks: 90,
            frost_stacks: 0,
            fire_stacks: 0,
            stun_ticks: 0,
        },
        fixed_rate: false,
        round_burst_ticks: 0,
        ability: WeaponAbility::None,
    },
];

/// Enemy catalog. Indices 0/1/2 are STABLE (render maps sprites by index, the
/// boss is index 2 via `BOSS`); new roster rows are appended at 3+.
/// Stats adapted from the source map (`docs/appendix-A-map-extraction.md §A.1`).
pub static ENEMIES: &[EnemyDef] = &[
    // 0 — Squeakzilla: the baseline swarm melee.
    EnemyDef {
        name: "Squeakzilla",
        base_hp: 200,
        move_speed: 8,
        contact_damage: 500,
        bounty: 10,
        armor_class: ARMOR_LIGHT,
        archetype: Archetype::Swarm,
        ability: EnemyAbility::None,
        boss: false,
    },
    // 1 — Fanged Death: slow, medium-armored bruiser.
    EnemyDef {
        name: "Fanged Death",
        base_hp: 1200,
        move_speed: 4,
        contact_damage: 1500,
        bounty: 40,
        armor_class: ARMOR_MEDIUM,
        archetype: Archetype::Tank,
        ability: EnemyAbility::None,
        boss: false,
    },
    // 2 — The Hippocrate: a doctor-hippo who swore to "first, do no harm" — he lied.
    //     The 30-min END-GAME boss. Fixed huge HP,
    // immune to weapon fire; only `Clear` hurts it (CLEAR_DAMAGE = 3M/use ⇒ ~11
    // Clears to kill its 33M HP). It does NOT self-destruct: when it reaches the
    // tank it PLANTS and grinds with a CADENCED contact hit (every
    // `BOSS_CONTACT_CADENCE` ticks, ×11 tier ⇒ ~0.33M/hit, dodge/armor/shield
    // honored) — see `combat::move_enemies`. That makes the climax a sustained
    // multi-Clear RACE: the player must out-Clear the boss's DPS (while the dense
    // escort piles on) before being ground down, instead of the old single
    // dodge-coin-flip on one 1.1M burst. After 30 min of player scaling this is the
    // real climax wall MOST runs end at, not a pushover. (Const id stays BOSS;
    // sprite key unchanged.)
    EnemyDef {
        name: "The Hippocrate",
        base_hp: 33_000_000,
        move_speed: 3,
        contact_damage: 45_000,
        bounty: 0,
        armor_class: ARMOR_LIGHT,
        archetype: Archetype::Boss,
        ability: EnemyAbility::None,
        boss: true,
    },
    // 3 — Doomduck: cheapest, weakest chaff — early swarm filler.
    EnemyDef {
        name: "Doomduck",
        base_hp: 120,
        move_speed: 8,
        contact_damage: 350,
        bounty: 6,
        armor_class: ARMOR_LIGHT,
        archetype: Archetype::Swarm,
        ability: EnemyAbility::None,
        boss: false,
    },
    // 4 — Bacon: fast melee rusher (reaches the tank quickly).
    EnemyDef {
        name: "Bacon",
        base_hp: 260,
        move_speed: 16,
        contact_damage: 700,
        bounty: 16,
        armor_class: ARMOR_LIGHT,
        archetype: Archetype::Fast,
        ability: EnemyAbility::None,
        boss: false,
    },
    // 5 — Honk: even faster, glassier melee.
    EnemyDef {
        name: "Honk",
        base_hp: 180,
        move_speed: 22,
        contact_damage: 600,
        bounty: 18,
        armor_class: ARMOR_LIGHT,
        archetype: Archetype::Fast,
        ability: EnemyAbility::None,
        boss: false,
    },
    // 6 — Bonk: very tanky, Fortified armor (only Siege bites hard).
    EnemyDef {
        name: "Bonk",
        base_hp: 4000,
        move_speed: 3,
        contact_damage: 2200,
        bounty: 80,
        armor_class: ARMOR_FORTIFIED,
        archetype: Archetype::Tank,
        ability: EnemyAbility::None,
        boss: false,
    },
    // 7 — Nope Rope: a caster that stands off at long range and pelts the
    // tank with magic bolts.
    EnemyDef {
        name: "Nope Rope",
        base_hp: 320,
        move_speed: 6,
        contact_damage: 200,
        bounty: 30,
        armor_class: ARMOR_LIGHT,
        archetype: Archetype::Caster,
        ability: EnemyAbility::RangedAttack {
            range: 900,
            cooldown_ticks: 45,
            damage: 250,
            damage_type: DMG_MAGIC,
        },
        boss: false,
    },
    // 8 — Croak: ranged spitter; light, frequent piercing spit.
    EnemyDef {
        name: "Croak",
        base_hp: 300,
        move_speed: 6,
        contact_damage: 200,
        bounty: 22,
        armor_class: ARMOR_LIGHT,
        archetype: Archetype::Ranged,
        ability: EnemyAbility::RangedAttack {
            range: 700,
            cooldown_ticks: 30,
            damage: 180,
            damage_type: DMG_PIERCING,
        },
        boss: false,
    },
    // 9 — Spicy: ranged breather; harder-hitting chaos breath at standoff.
    EnemyDef {
        name: "Spicy",
        base_hp: 600,
        move_speed: 5,
        contact_damage: 300,
        bounty: 34,
        armor_class: ARMOR_LIGHT,
        archetype: Archetype::Ranged,
        ability: EnemyAbility::RangedAttack {
            range: 600,
            cooldown_ticks: 36,
            damage: 420,
            damage_type: DMG_CHAOS,
        },
        boss: false,
    },
    // 10 — Popsicle: slow, durable ranged breather with siege breath.
    EnemyDef {
        name: "Popsicle",
        base_hp: 900,
        move_speed: 4,
        contact_damage: 350,
        bounty: 40,
        armor_class: ARMOR_MEDIUM,
        archetype: Archetype::Ranged,
        ability: EnemyAbility::RangedAttack {
            range: 650,
            cooldown_ticks: 48,
            damage: 500,
            damage_type: DMG_SIEGE,
        },
        boss: false,
    },
    // 11 — Dodo: inert practice target — never moves, no contact, easy bounty.
    EnemyDef {
        name: "Dodo",
        base_hp: 800,
        move_speed: 0,
        contact_damage: 0,
        bounty: 12,
        armor_class: ARMOR_LIGHT,
        archetype: Archetype::Inert,
        ability: EnemyAbility::None,
        boss: false,
    },
];

/// Index of the boss enemy def.
pub const BOSS: u16 = 2;

// Match timeline (ticks @ 30 Hz). The HP/damage curve (`enemy_hp_mult`) is a
// STEPPED "RAMP" on a strict 3-MINUTE cadence: every interval is gentle climb →
// warning → step, with steps at 3,6,…,30 min (see `RAMP_INTERVAL`). BALANCE PASS
// (+ catalog-fidelity re-dial): the step is +55% (the old ×413863 cliff hack is
// gone); the curve escalates smoothly to a ≈ ×120 boss endpoint — re-dialed up
// from +14%/×5.56 because the re-anchored catalog made player trajectories far
// stronger (the 80-seed sweep had drifted to 86% wins). These named tick constants pin the
// ROSTER schedule below (the wave/surge gates) and a couple of tests; the HP curve
// itself no longer keys off them.
pub const SCALE_STEP_1_TICK: u32 = 10 * 60 * 30; // 18000 — 10 min (roster reference)
pub const SCALE_STEP_2_TICK: u32 = 15 * 60 * 30; // 27000 — 15 min (roster reference)
pub const CLIFF_20_TICK: u32 = 20 * 60 * 30; // 36000 — 20 min (roster surge gate)
pub const CLIFF_25_TICK: u32 = 25 * 60 * 30; // 45000 — 25 min (roster surge gate)
/// The end-game boss ("The Hippocrate") spawns here; normal waves
/// stop. Moved from 15 min to 30 min — it is the climax most runs end at.
pub const BOSS_SPAWN_TICK: u32 = 30 * 60 * 30; // 54000 — 30 min

/// While the boss is planted on the tank it lands a contact hit every this many
/// ticks (a PERSISTENT attrition attack, not a one-shot self-destruct). 20 ticks
/// @30 Hz ⇒ 1.5 hits/s; with the ×11 boss-phase multiplier each hit is ~0.33M
/// (dodge/armor/shield apply). Tuned so the boss is a multi-Clear RACE the player
/// must win, deadly to an unprepared snowball but survivable by a strong build.
/// Pure function of `s.tick` — deterministic, no new RNG/state. (`combat::move_enemies`.)
pub const BOSS_CONTACT_CADENCE: u32 = 16;

/// One difficulty interval = 3 minutes @ 30 Hz. Cliffs land at `k*RAMP_INTERVAL`
/// for k = 1..=10; the boss tick (54000) is the k=10 boundary.
pub const RAMP_INTERVAL: u32 = 3 * 60 * 30; // 5400 — 3 min
/// Within each interval, the gentle climb spans this many ticks; the remaining
/// `RAMP_INTERVAL - GENTLE_TICKS` ticks are the steeper "warning" sub-ramp that
/// signals the cliff is coming.
pub const GENTLE_TICKS: u32 = 4500; // first ~2.5 min: the gentle climb
/// The "warning" window — the final ~30 s of each interval. Over it the curve
/// rises perceptibly STEEPER than the gentle climb (the telegraph), then the
/// dramatic step lands AT the next 3-min boundary.
pub const WARN_TICKS: u32 = RAMP_INTERVAL - GENTLE_TICKS; // 900 — final 30 s

/// The three per-interval multiplicative factors of the stepped "RAMP". An
/// interval that starts (post-cliff) at value `b` runs:
///   1. GENTLE climb  `b → b·G`           over the first `GENTLE_TICKS`,
///   2. WARNING ramp  `b·G → b·G·W`       over the final `WARN_TICKS` (steeper),
///   3. STEP UP       `b·G·W → b·G·W·J`   instantaneously AT the 3-min boundary.
///
/// So each interval multiplies difficulty by `G·W·J`, and over 10 intervals that
/// compounds to the boss endpoint `base(10)` (the boss tick rides the same value).
///
/// BALANCE PASS (`docs/06`): this REPLACES the interim ×413863 late-game hack. The
/// old hack set `J = 3.5` (+250%), compounding to an absurd ≈ ×413863 at the boss —
/// a meaningless number that piled ALL difficulty into one boss wall. The principled
/// curve keeps the same gentle→warning→step SHAPE and the strict 3-min cadence, but
/// the per-interval factors are tuned so the WHOLE 30-min run escalates smoothly:
///   • `G = +2.5%` gentle climb, `W = +1.6%` warning ramp (UNCHANGED), and
///   • `J = +55%` step — the CATALOG-FIDELITY re-dial (was +14%/×5.56): the
///     re-anchored catalog (source cooldown tiers, typed vulnerabilities,
///     restored upgrades) made player power compound so much faster that the
///     +14% curve drifted to 86% sweep wins. `J` alone cannot reach the 17-23%
///     band: at J ≥ 1.6 no bot build reaches the 30-min boss any more
///     (`net/tests/soak.rs` requires a real build to REACH the boss), so J is
///     parked at the highest arc-preserving tier (+55%; factor ≈ 1.6141 per
///     interval, `base(10) ≈ 120`), the 80-seed sweep sits at 65% wins, and
///     closing the remaining gap is the NEXT WAVE's full retune (catalog power
///     vs curve — a J-only fix cannot decouple 20-min difficulty from boss
///     reachability).
/// `J` is the OVERALL-SCALE / win-rate dial. Both enemy HP *and* contact damage
/// scale on this curve, so the late game is genuinely harder (denser, tankier,
/// deadlier) without any one-number absurdity.
/// `G·W·J = 1.025·1.016·1.55` (Fixed product ≈ 1.6141). Pure `(num,den)` Fixed
/// ratios — no floats, integer-parametrized, feeds `state_checksum`.
const RAMP_GENTLE: (i64, i64) = (41, 40); //  G = +2.5% gentle climb (1.025)
const RAMP_WARN: (i64, i64) = (127, 125); //  W = +1.6% over the short warning window (1.016)
const RAMP_JUMP: (i64, i64) = (31, 20); //    J = +55% per-interval step (1.55) — the overall-scale dial

/// Enemy HP scaling at `tick` (also scales contact/ranged damage — see
/// `combat::move_enemies` / `enemy_ranged_attacks`). The shape is a STEPPED "RAMP"
/// on a strict 3-MINUTE cadence. Each 3-min interval is `gentle climb → warning →
/// step`:
///   • a CALM smooth rise for the first ~2.5 min (`RAMP_GENTLE`, +2.5% total),
///   • a perceptibly steeper sub-ramp over the final ~30 s (`RAMP_WARN`, +1.6%) —
///     the telegraph that a step is coming,
///   • an instantaneous +55% step up AT the 3-min boundary (`RAMP_JUMP`).
/// Steps land at 3,6,…,30 min. The function is monotonic non-decreasing, CONTINUOUS
/// within each interval (the only instantaneous jumps are the steps at the
/// boundaries), and lands at ≈ ×120 at the 30-min boss (tick 54000) — the endpoint
/// (`base(10)`) after the catalog-fidelity `RAMP_JUMP` re-dial (which replaced the
/// balance pass's +14%/×5.56, itself a replacement of the interim ×413863 hack).
/// `RAMP_JUMP` is the overall-scale / win-rate dial. Integer/fixed-point only.
pub fn enemy_hp_mult(tick: u32) -> Fixed {
    let g = |x: Fixed| x.mul(Fixed::from_ratio(RAMP_GENTLE.0, RAMP_GENTLE.1));
    let w = |x: Fixed| x.mul(Fixed::from_ratio(RAMP_WARN.0, RAMP_WARN.1));
    let j = |x: Fixed| x.mul(Fixed::from_ratio(RAMP_JUMP.0, RAMP_JUMP.1));

    // `base(k)` = the post-cliff value at the start of interval `k`, built by
    // compounding the per-interval factor `k` times from ×1. Deterministic and
    // cheap (k ≤ 10). The boss endpoint is `base(10)` (the k=10 boundary), so the
    // boss-phase clamp is DERIVED from the same compounding — no magic constant to
    // drift when `RAMP_JUMP` is re-tuned.
    let base = |k: u32| -> Fixed {
        let mut b = Fixed::ONE;
        for _ in 0..k {
            b = j(w(g(b)));
        }
        b
    };

    // 30 min+: boss phase. Hold the peak endpoint tier (`base(10)`) — the boss AND
    // its escort swarm (see `BOSS_ESCORT`) ride this multiplier. Continuous with the
    // k=9→k=10 cliff because it IS that cliff's post value.
    if tick >= BOSS_SPAWN_TICK {
        return base(10);
    }

    // Linear interpolation `from → to` (Fixed) over `[lo, hi)`.
    let lerp_f = |lo: u32, hi: u32, from: Fixed, to: Fixed| -> Fixed {
        let span = (hi - lo) as i64;
        from + (to - from).mul(Fixed::from_ratio((tick - lo) as i64, span))
    };

    let k = tick / RAMP_INTERVAL; // interval index 0..=9
    let lo = k * RAMP_INTERVAL;
    let warn_start = lo + GENTLE_TICKS;
    let hi = lo + RAMP_INTERVAL;

    let bk = base(k);
    let gentle_end = g(bk); // value at the end of the gentle climb
    let warn_end = w(gentle_end); // pre-cliff peak (just before the step)

    if tick < warn_start {
        // Gentle climb: bk → bk·G across the first GENTLE_TICKS.
        lerp_f(lo, warn_start, bk, gentle_end)
    } else {
        // Warning sub-ramp: bk·G → bk·G·W across the final WARN_TICKS (steeper).
        // The dramatic step (warn_end → base(k+1)) lands AT `hi`, i.e. the next
        // interval's post-cliff value.
        lerp_f(warn_start, hi, gentle_end, warn_end)
    }
}

/// Ticks per in-game minute at 30 Hz (gate-time helper for the schedule).
const MIN: u32 = 60 * 30;

/// Boss-phase ESCORT swarm (30 min+): spawned alongside The Hippocrate
/// by `waves::spawn` at the peak ×11 HP tier. A relentless swarm/rusher/
/// bruiser flood whose job is contact-damage VOLUME and keeping the player's Clear
/// cycling (every Clear also chips the Clear-only boss). Tuned so the boss phase is
/// the wall MOST runs end at — survivable only by a genuinely prepared snowball.
pub static BOSS_ESCORT: &[WaveSpawn] = &[
    WaveSpawn {
        enemy: 0,
        cadence_ticks: 4,
        start_tick: BOSS_SPAWN_TICK,
    }, // Squeakzilla — dense floor (was 5)
    WaveSpawn {
        enemy: 4,
        cadence_ticks: 12,
        start_tick: BOSS_SPAWN_TICK,
    }, // Bacon — fast pressure (was 18)
    WaveSpawn {
        enemy: 5,
        cadence_ticks: 15,
        start_tick: BOSS_SPAWN_TICK,
    }, // Honk — very fast (was 22)
    WaveSpawn {
        enemy: 1,
        cadence_ticks: 45,
        start_tick: BOSS_SPAWN_TICK,
    }, // Fanged Death — periodic bruiser (was 60)
];

/// Match wave schedule: an ESCALATING mix. Early ticks are the original Grunt +
/// Fanged-Death baseline (entries 0/1, ungated); progressively richer/deadlier
/// roster entries gate in over the match via `start_tick`. Entries are processed
/// in catalog order every tick (stable `rng_spawn` draw sequence). The boss tick
/// stops all of this (handled in `waves::spawn`). HP scales via `enemy_hp_mult`.
/// EARLY-THROUGHPUT cadences (the eco-rush lever). The opening waves are tuned so
/// that an UNARMED tank (no weapons) leaks enough contact damage past its free
/// Clear to die by tick ≤3600 (the eco-rush punish, target 1) — while a MODEST
/// opener (a weapon or two + some HP/armor) thins the board fast enough to reach a
/// stable state and survive past 3600. The separation lever is OFFENSE: the spawn
/// rate sits just above what a weaponless tank can survive on Clear alone, but well
/// within what even a small arsenal can hold. These are the PRIMARY early-game knob;
/// raising any cadence (slower spawns) gentles the opening, lowering it makes it
/// deadlier. Integer ticks — deterministic, no floats. (Balance pass: see `docs/06`.)
pub const EARLY_GRUNT_CADENCE: u32 = 18; // Squeakzilla swarm floor (was 6)
pub const EARLY_PEON_CADENCE: u32 = 45; // Doomduck trickle (was 18)
pub const EARLY_RAIDER_CADENCE: u32 = 95; // Bacon rush (was 55)
pub const EARLY_BANDIT_CADENCE: u32 = 130; // Honk rush (was 100)

pub static WAVE_M0: &[WaveSpawn] = &[
    // --- baseline (from the start) ---
    // EARLY GRACE WITH AN ECO-RUSH PUNISH — BALANCE PASS. The opening is gentle
    // enough that a MODEST opener (1–2 weapons + some HP/armor) establishes board
    // control and survives past tick 3600, but a NAKED tank (no weapons/defense)
    // cannot out-Clear the steady leak and dies by ≤3600. The lever is VOLUME +
    // RUSH CADENCE (no one-shot spike): a weaponless tank's only tool is the free
    // 10 s Clear, and the fast-rusher streams (Raider/Bandit) arrive inside that
    // cooldown, so leak accumulates. Cadences come from the `EARLY_*_CADENCE`
    // constants above (the primary early-game knob).
    WaveSpawn {
        enemy: 0,
        cadence_ticks: EARLY_GRUNT_CADENCE,
        start_tick: 0,
    }, // Squeakzilla — swarm floor
    WaveSpawn {
        enemy: 1,
        cadence_ticks: 95,
        start_tick: 0,
    }, // Fanged Death — periodic bruiser (slow, 1500 contact)
    // --- early escalation (≈12s+): cheap chaff streams in early ---
    WaveSpawn {
        enemy: 3,
        cadence_ticks: EARLY_PEON_CADENCE,
        start_tick: MIN / 5,
    }, // Doomduck
    WaveSpawn {
        enemy: 11,
        cadence_ticks: 600,
        start_tick: MIN / 2,
    }, // Dodo (rare, inert)
    // --- ≈25s: fast melee rushers — the core of the eco-rush punish. Raiders are
    //     fast (speed 16) and hit hard (700 contact), so they reach the tank inside
    //     the Clear cooldown and a weaponless tank can't keep them off. ---
    WaveSpawn {
        enemy: 4,
        cadence_ticks: EARLY_RAIDER_CADENCE,
        start_tick: 5 * MIN / 12,
    }, // Bacon
    // --- ≈1 min: a second chaff trickle thickens the wall ---
    WaveSpawn {
        enemy: 3,
        cadence_ticks: 70,
        start_tick: MIN,
    }, // Doomduck (second stream from 1 min)
    // --- ≈45 s: even faster bandit rushers pile on (pulled early — fastest enemy,
    //     arrives inside the Clear cooldown, so it's the main eco-rush punisher) ---
    WaveSpawn {
        enemy: 5,
        cadence_ticks: EARLY_BANDIT_CADENCE,
        start_tick: 3 * MIN / 4,
    }, // Honk (very fast)
    // --- ≈90 s: ranged spitters start pelting from STANDOFF. Pulled early on the
    //     balance pass: standoff DPS (it pelts without reaching the tank, and a
    //     weaponless tank can't kill it between Clears) is what closes the eco-rush
    //     stalemate — it breaks the "Clear keeps the board empty forever" loophole
    //     so a naked tank reliably dies by ≤3600, while a real build just shoots it. ---
    WaveSpawn {
        enemy: 8,
        cadence_ticks: 120,
        start_tick: 3 * MIN / 2,
    }, // Croak (ranged, early standoff)
    // --- ≈4 min: casters + heavier ranged breath ---
    WaveSpawn {
        enemy: 7,
        cadence_ticks: 180,
        start_tick: 4 * MIN,
    }, // Nope Rope (caster)
    WaveSpawn {
        enemy: 9,
        cadence_ticks: 200,
        start_tick: 4 * MIN,
    }, // Spicy (ranged)
    // --- ≈6 min: fortified bruisers + slow ice breath, the late-game wall ---
    WaveSpawn {
        enemy: 6,
        cadence_ticks: 300,
        start_tick: 6 * MIN,
    }, // Bonk (Fortified)
    WaveSpawn {
        enemy: 10,
        cadence_ticks: 240,
        start_tick: 6 * MIN,
    }, // Popsicle (ranged)
    // --- POST-15 CLIFF SURGES: discrete roster jumps coinciding with the HP cliffs
    //     so each step is felt as MORE enemies AND tougher enemies, not just an HP
    //     bump. Telegraphed by the HP ramp in `enemy_hp_mult` landing at the same
    //     tick. These keep a snowballing player pressured between/at the cliffs.
    // Cliff #2 @20 min: a heavy fortified surge + extra fast rushers.
    WaveSpawn {
        enemy: 6,
        cadence_ticks: 150,
        start_tick: CLIFF_20_TICK,
    }, // Bonk (surge, was 300)
    WaveSpawn {
        enemy: 5,
        cadence_ticks: 60,
        start_tick: CLIFF_20_TICK,
    }, // Honk (fast surge)
    // Cliff #3 @25 min: relentless breathers + a swarm flood into the boss.
    WaveSpawn {
        enemy: 9,
        cadence_ticks: 90,
        start_tick: CLIFF_25_TICK,
    }, // Spicy (surge)
    WaveSpawn {
        enemy: 10,
        cadence_ticks: 100,
        start_tick: CLIFF_25_TICK,
    }, // Popsicle (surge)
    WaveSpawn {
        enemy: 0,
        cadence_ticks: 8,
        start_tick: CLIFF_25_TICK,
    }, // Squeakzilla (pre-boss flood)
];

/// Enemies spawn on this ring and march toward the tank. The eight angular
/// directions are unchanged, but the BALANCE PASS STAGGERS THE RADII (≈1200 /
/// 1500 / 1800 around the ring) so arrivals DESYNCHRONIZE: a near spawn lands
/// sooner than a far one on the same tick. This breaks the old "all spawns at
/// r≈1500 arrive together, so one Clear catches the whole synchronized wave"
/// loophole that let a weaponless Clear-spammer phase-lock the board to zero
/// contact on some seeds — now a steady trickle always has an enemy in-flight
/// when Clear is on cooldown, so an unarmed tank reliably leaks and dies (the
/// eco-rush punish). A real build just shoots them, so it is unaffected.
/// Precomputed (no trig) so spawn positions are deterministic.
pub static SPAWN_RING: &[Vec2] = &[
    v(1200, 0),      // near (arrives soonest)
    v(1273, 1273),   // far  (≈1800 diag)
    v(0, 1500),      // mid
    v(-849, 849),    // near (≈1200 diag)
    v(1800, 0),      // far
    v(-1061, -1061), // mid (≈1500 diag)
    v(0, -1200),     // near
    v(1273, -1273),  // far (≈1800 diag)
];

const fn v(x: i64, y: i64) -> Vec2 {
    Vec2 {
        x: Fixed::from_int(x),
        y: Fixed::from_int(y),
    }
}

/// Number of armor classes (columns in [`damage_multiplier`]'s matrix).
pub const NUM_ARMOR_CLASSES: usize = 3;

/// Armor/damage matrix: `DAMAGE_MATRIX[damage_type][armor_class]` as a Fixed
/// multiplier (adapted from the source extraction). Three armor classes:
/// 0 Light, 1 Medium, 2 Fortified. Fortified shrugs off everything except Siege:
/// Siege bites HARD, Piercing/Magic/Normal/Chaos are reduced.
pub fn damage_multiplier(damage_type: u8, armor_class: u8) -> Fixed {
    // rows = Normal, Piercing, Magic, Siege, Chaos ; cols = Light, Medium, Fortified
    const M: [[(i64, i64); NUM_ARMOR_CLASSES]; 5] = [
        [(1, 1), (1, 1), (1, 2)],  // Normal:   0.5x vs Fortified
        [(2, 1), (1, 1), (7, 20)], // Piercing: 2x vs Light, 0.35x vs Fortified
        [(1, 1), (2, 1), (1, 2)],  // Magic:    2x vs Medium, 0.5x vs Fortified
        [(1, 1), (1, 2), (3, 2)],  // Siege:    0.5x vs Medium, 1.5x vs Fortified
        [(1, 1), (1, 1), (1, 1)],  // Chaos:    ignores armor (1x everywhere)
    ];
    let dt = damage_type as usize % 5;
    let ac = (armor_class as usize).min(NUM_ARMOR_CLASSES - 1);
    let (n, d) = M[dt][ac];
    Fixed::from_ratio(n, d)
}
