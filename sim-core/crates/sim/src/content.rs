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
}

#[derive(Clone, Copy, Debug)]
pub struct EnemyDef {
    pub name: &'static str,
    pub base_hp: i64,
    pub move_speed: i64, // units per tick
    pub contact_damage: i64,
    pub bounty: i64,
    pub armor_class: u8,
    /// A boss (e.g. Samwise) — immune to weapon fire; only `Clear` damages it.
    pub boss: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct WaveSpawn {
    pub enemy: u16,
    pub cadence_ticks: u32, // spawn one every N ticks
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
}

/// Number of weapon damage scopes: 6 attack classes (0-5), 2 range buckets
/// (6 short / 7 long), 4 rarities (8-11).
pub const NUM_SCOPES: usize = 12;

/// Scope id for a weapon's attack class.
pub fn attack_scope_id(a: Attack) -> u8 {
    match a {
        Attack::SingleTarget => 0,
        Attack::Splash(_) => 1,
        Attack::Barrage(_) => 2,
        Attack::Area(_) => 3,
        Attack::Wave(_) => 4,
        Attack::Bounce(_) => 5,
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
            _ => return None,
        })
    }
}

/// A per-interval growth attached to a modifier — the source's "+X every 30 s".
/// On purchase the modifier's base `effect` applies once; then `effect` here is
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
    pub effect: ModEffect,
    /// Optional per-interval growth (`None` for most modifiers).
    pub ramp: Option<RampSpec>,
}

/// M4 modifier catalog — a representative slice across every scope (additive
/// global/by-type, multiplicative, attack-speed, economy, defensive). Numbers
/// adapted from the extracted upgrades (`docs/appendix-A-map-extraction.md`).
pub static MODIFIERS: &[ModifierDef] = &[
    ModifierDef { name: "+10% Damage", rarity: 0, cost: 500, effect: ModEffect::DamageGlobalPct(1, 10), ramp: None },
    ModifierDef { name: "+10% Piercing Damage", rarity: 0, cost: 500, effect: ModEffect::DamageTypePct(DMG_PIERCING, 1, 10), ramp: None },
    ModifierDef { name: "+10% Siege Damage", rarity: 0, cost: 500, effect: ModEffect::DamageTypePct(DMG_SIEGE, 1, 10), ramp: None },
    ModifierDef { name: "+10% Magic Damage", rarity: 0, cost: 500, effect: ModEffect::DamageTypePct(DMG_MAGIC, 1, 10), ramp: None },
    ModifierDef { name: "+25% Damage (Epic)", rarity: 3, cost: 5000, effect: ModEffect::DamageMulPct(1, 4), ramp: None },
    ModifierDef { name: "+10% Attack Speed", rarity: 0, cost: 500, effect: ModEffect::AttackSpeedPct(1, 10), ramp: None },
    ModifierDef { name: "+50% Kill Bounty", rarity: 1, cost: 1500, effect: ModEffect::BountyPct(1, 2), ramp: None },
    ModifierDef { name: "+20 Gold Income", rarity: 0, cost: 500, effect: ModEffect::IncomeFlat(20), ramp: None },
    // Economy depth (`docs/06` #5/source): income multiplier, income→HP-regen,
    // and a gambling bounty proc. `income_mult` is its own lever — `bounty_mult`
    // still never touches passive income (source rule).
    ModifierDef { name: "+10% Gold Income", rarity: 1, cost: 1500, effect: ModEffect::IncomePct(10, 100), ramp: None },
    ModifierDef { name: "+25% Gold Income", rarity: 2, cost: 3000, effect: ModEffect::IncomePct(25, 100), ramp: None },
    ModifierDef { name: "Golden Vitality (25% of Income as HP Regen)", rarity: 2, cost: 3000, effect: ModEffect::IncomeRegenPct(25, 100), ramp: None },
    ModifierDef { name: "Lucky Strikes (5% chance: +200% Bounty)", rarity: 2, cost: 3000, effect: ModEffect::BountyProc(5, 200), ramp: None },
    ModifierDef { name: "+2000 Max HP", rarity: 1, cost: 1500, effect: ModEffect::MaxHp(2000), ramp: None },
    ModifierDef { name: "+10 Armor", rarity: 0, cost: 500, effect: ModEffect::Armor(10), ramp: None },
    ModifierDef { name: "+2000 Mana Shield", rarity: 1, cost: 1500, effect: ModEffect::ManaShield(2000, 10), ramp: None },
    ModifierDef { name: "+50 HP Regen", rarity: 0, cost: 500, effect: ModEffect::HpRegen(50), ramp: None },
    ModifierDef { name: "+10% Dodge", rarity: 1, cost: 1500, effect: ModEffect::Dodge(10), ramp: None },
    // Time-scaling growth modifiers (`docs/06`): a base effect now + a smaller
    // effect re-applied every round, so they compound over a match.
    ModifierDef { name: "Building Power (+2% Damage, +1%/round)", rarity: 2, cost: 3000,
        effect: ModEffect::DamageGlobalPct(2, 100),
        ramp: Some(RampSpec { effect: ModEffect::DamageGlobalPct(1, 100), interval_ticks: RAMP_PER_ROUND }) },
    ModifierDef { name: "Escalating Chaos (+20% Chaos, +3%/round)", rarity: 2, cost: 3000,
        effect: ModEffect::DamageTypePct(DMG_CHAOS, 20, 100),
        ramp: Some(RampSpec { effect: ModEffect::DamageTypePct(DMG_CHAOS, 3, 100), interval_ticks: RAMP_PER_ROUND }) },
    ModifierDef { name: "Compounding Greed (+10 Income, +5/round)", rarity: 1, cost: 1500,
        effect: ModEffect::IncomeFlat(10),
        ramp: Some(RampSpec { effect: ModEffect::IncomeFlat(5), interval_ticks: RAMP_PER_ROUND }) },
    ModifierDef { name: "Hardening (+10 Armor, +5/round)", rarity: 1, cost: 1500,
        effect: ModEffect::Armor(10),
        ramp: Some(RampSpec { effect: ModEffect::Armor(5), interval_ticks: RAMP_PER_ROUND }) },
    // Per-scope damage (`docs/06`): +% for weapons matching an attack class /
    // range bucket / rarity (scope ids from attack_scope_id/range_scope_id/rarity_scope_id).
    ModifierDef { name: "+25% Single-Target Damage", rarity: 1, cost: 1500, effect: ModEffect::DamageScopePct(0, 25, 100), ramp: None },
    ModifierDef { name: "+25% Splash Damage", rarity: 1, cost: 1500, effect: ModEffect::DamageScopePct(1, 25, 100), ramp: None },
    ModifierDef { name: "+25% Barrage Damage", rarity: 1, cost: 1500, effect: ModEffect::DamageScopePct(2, 25, 100), ramp: None },
    ModifierDef { name: "+25% Area Damage", rarity: 1, cost: 1500, effect: ModEffect::DamageScopePct(3, 25, 100), ramp: None },
    ModifierDef { name: "+25% Wave Damage", rarity: 1, cost: 1500, effect: ModEffect::DamageScopePct(4, 25, 100), ramp: None },
    ModifierDef { name: "+25% Bounce Damage", rarity: 1, cost: 1500, effect: ModEffect::DamageScopePct(5, 25, 100), ramp: None },
    ModifierDef { name: "+25% Short-Range Damage (300/600)", rarity: 1, cost: 1500, effect: ModEffect::DamageScopePct(6, 25, 100), ramp: None },
    ModifierDef { name: "+25% Long-Range Damage (900/1200)", rarity: 1, cost: 1500, effect: ModEffect::DamageScopePct(7, 25, 100), ramp: None },
    ModifierDef { name: "+100% Common Weapon Damage", rarity: 1, cost: 1500, effect: ModEffect::DamageScopePct(8, 100, 100), ramp: None },
    // Status-conditional & flavor damage (`docs/06` #4): bonus damage vs enemies
    // in a status, and scalers on the statuses the tank applies.
    ModifierDef { name: "+20% Damage to Stunned", rarity: 1, cost: 1500, effect: ModEffect::DamageVsStunnedPct(20, 100), ramp: None },
    ModifierDef { name: "+25% Damage to Poisoned", rarity: 1, cost: 1500, effect: ModEffect::DamageVsPoisonedPct(25, 100), ramp: None },
    ModifierDef { name: "+10% Poison Damage", rarity: 0, cost: 500, effect: ModEffect::PoisonDamagePct(10, 100), ramp: None },
    ModifierDef { name: "+50% Stun Duration", rarity: 1, cost: 1500, effect: ModEffect::StunDurationPct(50, 100), ramp: None },
    // Spikes (`docs/06`): retaliation damage to nearby enemies when the tank is hit.
    ModifierDef { name: "+80 Spikes Damage", rarity: 0, cost: 500, effect: ModEffect::SpikesFlat(80), ramp: None },
    ModifierDef { name: "+300 Spikes Damage", rarity: 1, cost: 1500, effect: ModEffect::SpikesFlat(300), ramp: None },
    ModifierDef { name: "+50% Spikes Damage", rarity: 1, cost: 1500, effect: ModEffect::SpikesPct(50, 100), ramp: None },
    ModifierDef { name: "Bloody Spikes (+80, +10/round)", rarity: 2, cost: 3000,
        effect: ModEffect::SpikesFlat(80),
        ramp: Some(RampSpec { effect: ModEffect::SpikesFlat(10), interval_ticks: RAMP_PER_ROUND }) },
    // Vulnerability Pulse (`docs/06`): +5% damage taken / second to enemies in 1200 range.
    ModifierDef { name: "Vulnerability Pulse", rarity: 2, cost: 3000,
        effect: ModEffect::GrantVulnPulse(5, 1200, 30), ramp: None },
    // On-event triggers (`docs/06`): heal the tank on enemy-kill / on-poison-tick.
    ModifierDef { name: "+15 Heal on Kill", rarity: 1, cost: 1500, effect: ModEffect::HealOnKill(15), ramp: None },
    ModifierDef { name: "+60 Heal on Kill", rarity: 2, cost: 3000, effect: ModEffect::HealOnKill(60), ramp: None },
    ModifierDef { name: "Vampiric Spores (+5 HP/poison tick)", rarity: 2, cost: 3000, effect: ModEffect::HealOnPoison(5), ramp: None },
    // Meta / shop items (`docs/06` #5): they bend the purchase flow, not the
    // tank's stats. Resolved deterministically in `input::apply` (a one-shot
    // `PendingPerk`), and flagged `is_meta` so they never trigger or duplicate
    // each other.
    ModifierDef { name: "Magic Coin (+3 copies of next Common)", rarity: 1, cost: 1500, effect: ModEffect::GrantDuplicator(0, 3), ramp: None },
    ModifierDef { name: "Duplicator (+1 copy of next Rare)", rarity: 2, cost: 3000, effect: ModEffect::GrantDuplicator(2, 1), ramp: None },
    ModifierDef { name: "Black Market (next Uncommon is free)", rarity: 1, cost: 1500, effect: ModEffect::GrantVoucher(1), ramp: None },
    ModifierDef { name: "Magic Treasure (+250 Gold, +5 income/round)", rarity: 1, cost: 1000,
        effect: ModEffect::GrantGold(250),
        ramp: Some(RampSpec { effect: ModEffect::IncomeFlat(5), interval_ticks: RAMP_PER_ROUND }) },
    // Self-scaling / healing / revive (`docs/06` #6): bespoke survival & growth.
    ModifierDef { name: "Ankh of Reincarnation (revive, +2000 Max HP)", rarity: 3, cost: 5000, effect: ModEffect::GrantRevive(2000), ramp: None },
    ModifierDef { name: "+25% Healing", rarity: 1, cost: 1500, effect: ModEffect::HealingPct(25, 100), ramp: None },
    ModifierDef { name: "Regeneration (1.5% Missing HP/sec)", rarity: 2, cost: 3000, effect: ModEffect::MissingHpHealPct(15, 1000), ramp: None },
    ModifierDef { name: "+1% Piercing Damage per Bow", rarity: 2, cost: 3000, effect: ModEffect::DamagePerWeapon(0, DMG_PIERCING as i64, 1), ramp: None },
    ModifierDef { name: "+1% Siege Damage per Mortar", rarity: 2, cost: 3000, effect: ModEffect::DamagePerWeapon(1, DMG_SIEGE as i64, 1), ramp: None },
    // Stacking damage generator (`docs/06` #5): the Death Engine weapon's Chaos
    // damage scales +10% per Death Engine owned (self-referential count).
    ModifierDef { name: "Overclocked Death Engine (+10% Chaos per Death Engine)", rarity: 2, cost: 3000, effect: ModEffect::DamagePerWeapon(DEATH_ENGINE as i64, DMG_CHAOS as i64, 10), ramp: None },
    // Damage ↔ economy trades & damage-scaled bounty (`docs/06` #5): pay survival
    // stats for gold, and convert dealt damage into gold ("Bloodmoney"), and
    // income into a survival buffer.
    ModifierDef { name: "Blood Pact (-1000 Max HP, +2000 Gold)", rarity: 1, cost: 0, effect: ModEffect::TradeMaxHpForGold(1000, 2000), ramp: None },
    ModifierDef { name: "Last Rites (-100 HP Regen, +5000 Gold)", rarity: 2, cost: 0, effect: ModEffect::TradeRegenForGold(100, 5000), ramp: None },
    ModifierDef { name: "Bloodmoney (+1 Gold per 100 Damage)", rarity: 1, cost: 1500, effect: ModEffect::GoldPerDamagePct(1, 100), ramp: None },
    ModifierDef { name: "Bloodmoney II (+1 Gold per 20 Damage)", rarity: 2, cost: 3000, effect: ModEffect::GoldPerDamagePct(1, 20), ramp: None },
    ModifierDef { name: "Wartithe (25% of Income as Mana Shield)", rarity: 2, cost: 3000, effect: ModEffect::IncomeShieldPct(25, 100), ramp: None },
    // GEN-MODIFIERS-BEGIN (generated by research/tower-survivors-map/gen_catalog.py)
    ModifierDef { name: "+4000 Mana Shield", rarity: 3, cost: 5000, effect: ModEffect::ManaShield(4000, 20), ramp: None },
    ModifierDef { name: "+500 Max HP", rarity: 0, cost: 500, effect: ModEffect::MaxHp(500), ramp: None },
    ModifierDef { name: "+10% Piercing Damage", rarity: 0, cost: 500, effect: ModEffect::DamageTypePct(DMG_PIERCING, 10, 100), ramp: None },
    ModifierDef { name: "+10% Normal Damage", rarity: 0, cost: 500, effect: ModEffect::DamageTypePct(DMG_NORMAL, 10, 100), ramp: None },
    ModifierDef { name: "+10% Siege Damage", rarity: 0, cost: 500, effect: ModEffect::DamageTypePct(DMG_SIEGE, 10, 100), ramp: None },
    ModifierDef { name: "+10% Chaos Damage", rarity: 0, cost: 500, effect: ModEffect::DamageTypePct(DMG_CHAOS, 10, 100), ramp: None },
    ModifierDef { name: "+1000 Max HP", rarity: 1, cost: 1500, effect: ModEffect::MaxHp(1000), ramp: None },
    ModifierDef { name: "+10 Armor", rarity: 0, cost: 500, effect: ModEffect::Armor(10), ramp: None },
    ModifierDef { name: "+2000 Max HP", rarity: 2, cost: 3000, effect: ModEffect::MaxHp(2000), ramp: None },
    ModifierDef { name: "+50% Kill Bounty", rarity: 2, cost: 3000, effect: ModEffect::BountyPct(50, 100), ramp: None },
    ModifierDef { name: "+2000 Mana Shield", rarity: 2, cost: 3000, effect: ModEffect::ManaShield(2000, 10), ramp: None },
    ModifierDef { name: "+20 Gold Income", rarity: 0, cost: 500, effect: ModEffect::IncomeFlat(20), ramp: None },
    ModifierDef { name: "+5 Armor", rarity: 0, cost: 500, effect: ModEffect::Armor(5), ramp: None },
    ModifierDef { name: "+10% Attack Speed", rarity: 0, cost: 500, effect: ModEffect::AttackSpeedPct(10, 100), ramp: None },
    ModifierDef { name: "+80 HP Regen", rarity: 1, cost: 1500, effect: ModEffect::HpRegen(80), ramp: None },
    ModifierDef { name: "+20 HP Regen", rarity: 0, cost: 500, effect: ModEffect::HpRegen(20), ramp: None },
    ModifierDef { name: "+5 Gold Income", rarity: 0, cost: 500, effect: ModEffect::IncomeFlat(5), ramp: None },
    ModifierDef { name: "+10 Gold Income", rarity: 0, cost: 500, effect: ModEffect::IncomeFlat(10), ramp: None },
    ModifierDef { name: "+100% Kill Bounty", rarity: 2, cost: 3000, effect: ModEffect::BountyPct(100, 100), ramp: None },
    ModifierDef { name: "+10% Magic Damage", rarity: 0, cost: 500, effect: ModEffect::DamageTypePct(DMG_MAGIC, 10, 100), ramp: None },
    ModifierDef { name: "+40 HP Regen", rarity: 0, cost: 500, effect: ModEffect::HpRegen(40), ramp: None },
    ModifierDef { name: "+10000 Mana Shield", rarity: 3, cost: 5000, effect: ModEffect::ManaShield(10000, 50), ramp: None },
    ModifierDef { name: "+10% Dodge", rarity: 0, cost: 500, effect: ModEffect::Dodge(10), ramp: None },
    ModifierDef { name: "+200% Kill Bounty", rarity: 2, cost: 3000, effect: ModEffect::BountyPct(200, 100), ramp: None },
    ModifierDef { name: "+5000 Max HP", rarity: 3, cost: 5000, effect: ModEffect::MaxHp(5000), ramp: None },
    ModifierDef { name: "+1000 Mana Shield", rarity: 1, cost: 1500, effect: ModEffect::ManaShield(1000, 5), ramp: None },
    ModifierDef { name: "+200 HP Regen", rarity: 1, cost: 1500, effect: ModEffect::HpRegen(200), ramp: None },
    // GEN-MODIFIERS-END
];

/// The weapon the tank starts with (index into [`WEAPONS`]).
pub const STARTING_WEAPON: u16 = 0;

/// Stable index of the self-scaling "Death Engine" generator weapon (referenced
/// by the "Overclocked Death Engine" modifier so it scales with its own count).
pub const DEATH_ENGINE: u16 = 10;

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
    },
    // 1 — Mortar Launcher: siege splash.
    WeaponDef {
        name: "Mortar Launcher",
        rarity: 0,
        cost: 500,
        damage: 300,
        damage_type: DMG_SIEGE,
        attack: Attack::Splash(300),
        cooldown_ticks: 60, // 2.0s
        range: 1200,
        proj_speed: 30,
        on_hit: StatusOnHit::NONE,
    },
    // 2 — Frost Bow: applies Frost stacks (slow).
    WeaponDef {
        name: "Frost Bow",
        rarity: 1,
        cost: 1500,
        damage: 125,
        damage_type: DMG_MAGIC,
        attack: Attack::SingleTarget,
        cooldown_ticks: 15, // 0.5s
        range: 900,
        proj_speed: 50,
        on_hit: StatusOnHit { frost_stacks: 5, ..StatusOnHit::NONE },
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
        on_hit: StatusOnHit { poison_dps: 20, poison_ticks: 90, ..StatusOnHit::NONE },
    },
    // 4 — Flamecaster: applies Fire stacks (vulnerability + explode on death).
    WeaponDef {
        name: "Flamecaster",
        rarity: 1,
        cost: 1500,
        damage: 100,
        damage_type: DMG_CHAOS,
        attack: Attack::Splash(150),
        cooldown_ticks: 20,
        range: 600,
        proj_speed: 40,
        on_hit: StatusOnHit { fire_stacks: 5, ..StatusOnHit::NONE },
    },
    // 5 — Storm Hammer: hard single hit that Stuns.
    WeaponDef {
        name: "Storm Hammer",
        rarity: 2,
        cost: 3000,
        damage: 400,
        damage_type: DMG_MAGIC,
        attack: Attack::SingleTarget,
        cooldown_ticks: 45,
        range: 600,
        proj_speed: 40,
        on_hit: StatusOnHit { stun_ticks: 45, ..StatusOnHit::NONE },
    },
    // 6 — Ballista: a Barrage hitting several targets at once.
    WeaponDef {
        name: "Ballista",
        rarity: 1,
        cost: 1500,
        damage: 100,
        damage_type: DMG_SIEGE,
        attack: Attack::Barrage(4),
        cooldown_ticks: 30,
        range: 1200,
        proj_speed: 50,
        on_hit: StatusOnHit::NONE,
    },
    // 7 — Immolation: an instant Area pulse around the tank that burns.
    WeaponDef {
        name: "Immolation",
        rarity: 1,
        cost: 1500,
        damage: 80,
        damage_type: DMG_CHAOS,
        attack: Attack::Area(300),
        cooldown_ticks: 30,
        range: 300,
        proj_speed: 0,
        on_hit: StatusOnHit { fire_stacks: 2, ..StatusOnHit::NONE },
    },
    // 8 — Shockwave Axe: an instant sweeping Wave.
    WeaponDef {
        name: "Shockwave Axe",
        rarity: 2,
        cost: 3000,
        damage: 500,
        damage_type: DMG_NORMAL,
        attack: Attack::Wave(300),
        cooldown_ticks: 60,
        range: 300,
        proj_speed: 0,
        on_hit: StatusOnHit::NONE,
    },
    // 9 — Moon Glaive: an instant Bounce chaining to nearby enemies.
    WeaponDef {
        name: "Moon Glaive",
        rarity: 1,
        cost: 1500,
        damage: 150,
        damage_type: DMG_PIERCING,
        attack: Attack::Bounce(4),
        cooldown_ticks: 30,
        range: 600,
        proj_speed: 0,
        on_hit: StatusOnHit::NONE,
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
        damage: 250,
        damage_type: DMG_CHAOS,
        attack: Attack::SingleTarget,
        cooldown_ticks: 30,
        range: 900,
        proj_speed: 45,
        on_hit: StatusOnHit::NONE,
    },
    // GEN-WEAPONS-BEGIN (generated by research/tower-survivors-map/gen_catalog.py)
    WeaponDef { name: "Magic Missile", rarity: 0, cost: 500, damage: 75, damage_type: DMG_NORMAL, attack: Attack::SingleTarget, cooldown_ticks: 30, range: 900, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Boulder", rarity: 0, cost: 500, damage: 75, damage_type: DMG_SIEGE, attack: Attack::SingleTarget, cooldown_ticks: 30, range: 900, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Magic Bolt", rarity: 0, cost: 500, damage: 75, damage_type: DMG_MAGIC, attack: Attack::SingleTarget, cooldown_ticks: 30, range: 900, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Chaos Orb", rarity: 0, cost: 500, damage: 75, damage_type: DMG_CHAOS, attack: Attack::SingleTarget, cooldown_ticks: 30, range: 900, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Throwing Axes", rarity: 0, cost: 500, damage: 100, damage_type: DMG_PIERCING, attack: Attack::Bounce(4), cooldown_ticks: 15, range: 300, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Chaos Skulls", rarity: 0, cost: 500, damage: 75, damage_type: DMG_CHAOS, attack: Attack::Bounce(4), cooldown_ticks: 15, range: 600, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Chaos Heart", rarity: 2, cost: 3000, damage: 600, damage_type: DMG_CHAOS, attack: Attack::Wave(300), cooldown_ticks: 90, range: 300, proj_speed: 0, on_hit: StatusOnHit::NONE },  // exotic: Heal (base only)
    WeaponDef { name: "Missile Barrage", rarity: 3, cost: 5000, damage: 2500, damage_type: DMG_PIERCING, attack: Attack::Barrage(8), cooldown_ticks: 150, range: 1200, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 0, stun_ticks: 90 } },
    WeaponDef { name: "Seeker Axe", rarity: 0, cost: 500, damage: 150, damage_type: DMG_PIERCING, attack: Attack::Bounce(4), cooldown_ticks: 30, range: 900, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Steam Cannon", rarity: 1, cost: 1500, damage: 200, damage_type: DMG_SIEGE, attack: Attack::Splash(300), cooldown_ticks: 30, range: 300, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 0, stun_ticks: 15 } },
    WeaponDef { name: "Demon Eye", rarity: 2, cost: 3000, damage: 800, damage_type: DMG_CHAOS, attack: Attack::SingleTarget, cooldown_ticks: 30, range: 600, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: reduce enemy (base only)
    WeaponDef { name: "Impaler", rarity: 1, cost: 1500, damage: 150, damage_type: DMG_PIERCING, attack: Attack::SingleTarget, cooldown_ticks: 15, range: 600, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 0, stun_ticks: 22 } },
    WeaponDef { name: "Chaos Swarm", rarity: 0, cost: 500, damage: 150, damage_type: DMG_CHAOS, attack: Attack::Splash(300), cooldown_ticks: 30, range: 1200, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Catapult", rarity: 1, cost: 1500, damage: 300, damage_type: DMG_SIEGE, attack: Attack::SingleTarget, cooldown_ticks: 30, range: 600, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Wind Spear", rarity: 3, cost: 5000, damage: 4000, damage_type: DMG_PIERCING, attack: Attack::SingleTarget, cooldown_ticks: 30, range: 900, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: Knockback (base only)
    WeaponDef { name: "Crippler", rarity: 2, cost: 3000, damage: 1000, damage_type: DMG_PIERCING, attack: Attack::SingleTarget, cooldown_ticks: 30, range: 600, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: permanent (base only)
    WeaponDef { name: "Lifeleecher", rarity: 2, cost: 3000, damage: 500, damage_type: DMG_NORMAL, attack: Attack::SingleTarget, cooldown_ticks: 60, range: 1200, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: Heal (base only)
    WeaponDef { name: "Spell Glaive", rarity: 1, cost: 1500, damage: 300, damage_type: DMG_MAGIC, attack: Attack::Bounce(4), cooldown_ticks: 60, range: 600, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Glaive Thrower", rarity: 0, cost: 500, damage: 150, damage_type: DMG_NORMAL, attack: Attack::Bounce(4), cooldown_ticks: 30, range: 600, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Spikewheel Launcher", rarity: 1, cost: 1500, damage: 400, damage_type: DMG_SIEGE, attack: Attack::Bounce(8), cooldown_ticks: 60, range: 1200, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Meatapult", rarity: 2, cost: 3000, damage: 500, damage_type: DMG_NORMAL, attack: Attack::Splash(300), cooldown_ticks: 60, range: 900, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 0, stun_ticks: 60 } },
    WeaponDef { name: "Arcane Blaster", rarity: 1, cost: 1500, damage: 300, damage_type: DMG_MAGIC, attack: Attack::SingleTarget, cooldown_ticks: 30, range: 600, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 0, stun_ticks: 45 } },
    WeaponDef { name: "Quills", rarity: 1, cost: 1500, damage: 200, damage_type: DMG_PIERCING, attack: Attack::SingleTarget, cooldown_ticks: 30, range: 900, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 3, poison_ticks: 90, frost_stacks: 0, fire_stacks: 0, stun_ticks: 0 } },
    WeaponDef { name: "Living Spittle", rarity: 1, cost: 1500, damage: 200, damage_type: DMG_MAGIC, attack: Attack::SingleTarget, cooldown_ticks: 30, range: 900, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 3, poison_ticks: 90, frost_stacks: 0, fire_stacks: 0, stun_ticks: 0 } },
    WeaponDef { name: "Poison Bomb", rarity: 1, cost: 1500, damage: 400, damage_type: DMG_SIEGE, attack: Attack::Splash(300), cooldown_ticks: 60, range: 1200, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 3, poison_ticks: 90, frost_stacks: 0, fire_stacks: 0, stun_ticks: 0 } },
    WeaponDef { name: "Serpent", rarity: 2, cost: 3000, damage: 600, damage_type: DMG_NORMAL, attack: Attack::SingleTarget, cooldown_ticks: 30, range: 600, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 10, poison_ticks: 90, frost_stacks: 0, fire_stacks: 0, stun_ticks: 0 } },
    WeaponDef { name: "Overloaded Catapult", rarity: 2, cost: 3000, damage: 600, damage_type: DMG_SIEGE, attack: Attack::Splash(300), cooldown_ticks: 60, range: 600, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Chaos Claw", rarity: 1, cost: 1500, damage: 250, damage_type: DMG_CHAOS, attack: Attack::SingleTarget, cooldown_ticks: 30, range: 900, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 0, stun_ticks: 45 } },
    WeaponDef { name: "Net Thrower", rarity: 1, cost: 1500, damage: 125, damage_type: DMG_NORMAL, attack: Attack::SingleTarget, cooldown_ticks: 15, range: 1200, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 0, stun_ticks: 22 } },
    WeaponDef { name: "Thornburst", rarity: 0, cost: 500, damage: 150, damage_type: DMG_PIERCING, attack: Attack::Area(600), cooldown_ticks: 60, range: 600, proj_speed: 0, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Chaotic Spirit", rarity: 3, cost: 5000, damage: 1250, damage_type: DMG_MAGIC, attack: Attack::Bounce(8), cooldown_ticks: 150, range: 900, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Energy Pulse", rarity: 2, cost: 3000, damage: 800, damage_type: DMG_MAGIC, attack: Attack::Wave(300), cooldown_ticks: 90, range: 300, proj_speed: 0, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 0, stun_ticks: 60 } },
    WeaponDef { name: "Cluster Rockets", rarity: 2, cost: 3000, damage: 600, damage_type: DMG_CHAOS, attack: Attack::Barrage(12), cooldown_ticks: 120, range: 1200, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Frost Bomb", rarity: 2, cost: 3000, damage: 500, damage_type: DMG_PIERCING, attack: Attack::Splash(150), cooldown_ticks: 15, range: 600, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 3, fire_stacks: 0, stun_ticks: 0 } },
    WeaponDef { name: "Bouncy Cannonball", rarity: 2, cost: 3000, damage: 600, damage_type: DMG_NORMAL, attack: Attack::Bounce(4), cooldown_ticks: 90, range: 900, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 0, stun_ticks: 90 } },
    WeaponDef { name: "Soulstealer", rarity: 3, cost: 5000, damage: 7500, damage_type: DMG_NORMAL, attack: Attack::Bounce(4), cooldown_ticks: 90, range: 600, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: Heal (base only)
    WeaponDef { name: "Splasher", rarity: 0, cost: 500, damage: 75, damage_type: DMG_NORMAL, attack: Attack::Splash(300), cooldown_ticks: 10, range: 600, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Fire Bow", rarity: 1, cost: 1500, damage: 400, damage_type: DMG_PIERCING, attack: Attack::Barrage(4), cooldown_ticks: 30, range: 600, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 3, stun_ticks: 0 } },
    WeaponDef { name: "Chaos Web", rarity: 1, cost: 1500, damage: 250, damage_type: DMG_CHAOS, attack: Attack::Bounce(8), cooldown_ticks: 30, range: 600, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 5, poison_ticks: 90, frost_stacks: 0, fire_stacks: 0, stun_ticks: 0 } },
    WeaponDef { name: "Magic Claw", rarity: 1, cost: 1500, damage: 300, damage_type: DMG_MAGIC, attack: Attack::Bounce(4), cooldown_ticks: 30, range: 300, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: Mana (base only)
    WeaponDef { name: "Liquid Fire Hurler", rarity: 1, cost: 1500, damage: 333, damage_type: DMG_SIEGE, attack: Attack::SingleTarget, cooldown_ticks: 10, range: 900, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 3, stun_ticks: 0 } },  // exotic: damage taken (base only)
    WeaponDef { name: "Boulder Toss", rarity: 2, cost: 3000, damage: 900, damage_type: DMG_NORMAL, attack: Attack::Splash(300), cooldown_ticks: 90, range: 300, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: reduce enemy (base only)
    WeaponDef { name: "Bloody Spikes", rarity: 3, cost: 5000, damage: 5000, damage_type: DMG_NORMAL, attack: Attack::Wave(300), cooldown_ticks: 30, range: 300, proj_speed: 0, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 0, stun_ticks: 30 } },
    WeaponDef { name: "Inferno Stone", rarity: 3, cost: 5000, damage: 3000, damage_type: DMG_CHAOS, attack: Attack::Area(150), cooldown_ticks: 240, range: 900, proj_speed: 0, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 50, stun_ticks: 60 } },  // exotic: Summon (base only)
    WeaponDef { name: "Flame Generator", rarity: 3, cost: 5000, damage: 5000, damage_type: DMG_MAGIC, attack: Attack::Area(300), cooldown_ticks: 150, range: 600, proj_speed: 0, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 250, stun_ticks: 0 } },  // exotic: damage taken (base only)
    WeaponDef { name: "Firebreather", rarity: 1, cost: 1500, damage: 100, damage_type: DMG_PIERCING, attack: Attack::Splash(150), cooldown_ticks: 15, range: 900, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 5, stun_ticks: 0 } },  // exotic: damage taken (base only)
    WeaponDef { name: "Lavaspitter", rarity: 3, cost: 5000, damage: 1500, damage_type: DMG_SIEGE, attack: Attack::Splash(300), cooldown_ticks: 90, range: 1200, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 150, stun_ticks: 0 } },  // exotic: damage taken (base only)
    WeaponDef { name: "Frostbolt", rarity: 1, cost: 1500, damage: 125, damage_type: DMG_PIERCING, attack: Attack::SingleTarget, cooldown_ticks: 15, range: 900, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 5, fire_stacks: 0, stun_ticks: 0 } },
    WeaponDef { name: "Living Ice", rarity: 1, cost: 1500, damage: 200, damage_type: DMG_MAGIC, attack: Attack::Splash(150), cooldown_ticks: 30, range: 900, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 2, fire_stacks: 0, stun_ticks: 0 } },
    WeaponDef { name: "Ice Generator", rarity: 3, cost: 5000, damage: 1500, damage_type: DMG_MAGIC, attack: Attack::Area(375), cooldown_ticks: 30, range: 1200, proj_speed: 0, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 5, fire_stacks: 0, stun_ticks: 0 } },
    WeaponDef { name: "Ice Spears", rarity: 2, cost: 3000, damage: 450, damage_type: DMG_NORMAL, attack: Attack::Barrage(4), cooldown_ticks: 45, range: 600, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 5, fire_stacks: 0, stun_ticks: 0 } },
    WeaponDef { name: "Knives", rarity: 0, cost: 500, damage: 50, damage_type: DMG_PIERCING, attack: Attack::Barrage(4), cooldown_ticks: 15, range: 900, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Blaster", rarity: 1, cost: 1500, damage: 150, damage_type: DMG_SIEGE, attack: Attack::SingleTarget, cooldown_ticks: 15, range: 600, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: damage taken (base only)
    WeaponDef { name: "Bandit Sniper", rarity: 1, cost: 1500, damage: 100, damage_type: DMG_NORMAL, attack: Attack::SingleTarget, cooldown_ticks: 15, range: 900, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: damage taken (base only)
    WeaponDef { name: "Bombs", rarity: 1, cost: 1500, damage: 250, damage_type: DMG_SIEGE, attack: Attack::Splash(300), cooldown_ticks: 30, range: 300, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 0, stun_ticks: 15 } },
    WeaponDef { name: "Death Coil", rarity: 1, cost: 1500, damage: 200, damage_type: DMG_CHAOS, attack: Attack::SingleTarget, cooldown_ticks: 30, range: 900, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: damage taken (base only)
    WeaponDef { name: "Chaos Skull Bomb", rarity: 2, cost: 3000, damage: 450, damage_type: DMG_CHAOS, attack: Attack::Splash(150), cooldown_ticks: 90, range: 900, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 0, stun_ticks: 45 } },
    WeaponDef { name: "Icebreather", rarity: 2, cost: 3000, damage: 600, damage_type: DMG_SIEGE, attack: Attack::Splash(300), cooldown_ticks: 90, range: 900, proj_speed: 45, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 2, fire_stacks: 0, stun_ticks: 0 } },  // exotic: explode (base only)
    WeaponDef { name: "Frostwave", rarity: 2, cost: 3000, damage: 500, damage_type: DMG_MAGIC, attack: Attack::Wave(150), cooldown_ticks: 60, range: 300, proj_speed: 0, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 3, fire_stacks: 0, stun_ticks: 0 } },
    WeaponDef { name: "Flamewave", rarity: 1, cost: 1500, damage: 400, damage_type: DMG_NORMAL, attack: Attack::Wave(150), cooldown_ticks: 60, range: 300, proj_speed: 0, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 20, stun_ticks: 0 } },  // exotic: damage taken (base only)
    WeaponDef { name: "Chaotic Spirit Bolt", rarity: 1, cost: 1500, damage: 125, damage_type: DMG_CHAOS, attack: Attack::SingleTarget, cooldown_ticks: 10, range: 600, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: Heal (base only)
    WeaponDef { name: "Manabolt", rarity: 1, cost: 1500, damage: 266, damage_type: DMG_MAGIC, attack: Attack::SingleTarget, cooldown_ticks: 10, range: 1200, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: drain (base only)
    WeaponDef { name: "Death Generator", rarity: 3, cost: 5000, damage: 1500, damage_type: DMG_CHAOS, attack: Attack::SingleTarget, cooldown_ticks: 30, range: 1200, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: Raises (base only)
    WeaponDef { name: "Immolation Aura", rarity: 1, cost: 1500, damage: 60, damage_type: DMG_MAGIC, attack: Attack::Wave(150), cooldown_ticks: 6, range: 300, proj_speed: 0, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 2, stun_ticks: 0 } },  // exotic: damage taken (base only)
    WeaponDef { name: "Goblin Land Mines", rarity: 3, cost: 5000, damage: 7500, damage_type: DMG_SIEGE, attack: Attack::Wave(150), cooldown_ticks: 30, range: 300, proj_speed: 0, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 0, stun_ticks: 90 } },  // exotic: land mine (base only)
    WeaponDef { name: "Quill Burst", rarity: 1, cost: 1500, damage: 400, damage_type: DMG_PIERCING, attack: Attack::Splash(300), cooldown_ticks: 60, range: 1200, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: damage taken (base only)
    WeaponDef { name: "Arcane Burst", rarity: 1, cost: 1500, damage: 400, damage_type: DMG_MAGIC, attack: Attack::Splash(300), cooldown_ticks: 60, range: 1200, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: damage taken (base only)
    WeaponDef { name: "Meteor Barrage", rarity: 3, cost: 5000, damage: 5000, damage_type: DMG_SIEGE, attack: Attack::Barrage(8), cooldown_ticks: 300, range: 1200, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Ale Launcher", rarity: 1, cost: 1500, damage: 100, damage_type: DMG_SIEGE, attack: Attack::Splash(150), cooldown_ticks: 15, range: 900, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: Heal (base only)
    WeaponDef { name: "Chaos Bolt", rarity: 1, cost: 1500, damage: 200, damage_type: DMG_CHAOS, attack: Attack::SingleTarget, cooldown_ticks: 10, range: 1200, proj_speed: 45, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Rotating Orb of Lightning", rarity: 2, cost: 3000, damage: 600, damage_type: DMG_MAGIC, attack: Attack::Area(900), cooldown_ticks: 30, range: 900, proj_speed: 0, on_hit: StatusOnHit::NONE },
    WeaponDef { name: "Lightning Generator", rarity: 2, cost: 3000, damage: 500, damage_type: DMG_MAGIC, attack: Attack::SingleTarget, cooldown_ticks: 60, range: 1200, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: Mana (base only)
    WeaponDef { name: "Flame Nova", rarity: 1, cost: 1500, damage: 200, damage_type: DMG_CHAOS, attack: Attack::Area(300), cooldown_ticks: 30, range: 300, proj_speed: 0, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 20, stun_ticks: 0 } },  // exotic: damage taken (base only)
    WeaponDef { name: "Shocker", rarity: 1, cost: 1500, damage: 125, damage_type: DMG_SIEGE, attack: Attack::Area(375), cooldown_ticks: 8, range: 900, proj_speed: 0, on_hit: StatusOnHit { poison_dps: 0, poison_ticks: 0, frost_stacks: 0, fire_stacks: 0, stun_ticks: 60 } },
    WeaponDef { name: "Entangler", rarity: 3, cost: 5000, damage: 1500, damage_type: DMG_NORMAL, attack: Attack::SingleTarget, cooldown_ticks: 10, range: 900, proj_speed: 45, on_hit: StatusOnHit::NONE },  // exotic: Root (base only)
    // GEN-WEAPONS-END
];

/// Enemy catalog. Index 2 is the end boss, Samwise.
pub static ENEMIES: &[EnemyDef] = &[
    EnemyDef {
        name: "Fel Orc Grunt",
        base_hp: 200,
        move_speed: 8,
        contact_damage: 500,
        bounty: 10,
        armor_class: 0,
        boss: false,
    },
    EnemyDef {
        name: "Steam Tank",
        base_hp: 1200,
        move_speed: 4,
        contact_damage: 1500,
        bounty: 40,
        armor_class: 1,
        boss: false,
    },
    // 2 — Samwise: fixed huge HP, immune to weapon fire; only `Clear` hurts it.
    EnemyDef {
        name: "Samwise",
        base_hp: 10_000_000,
        move_speed: 3,
        contact_damage: 100_000,
        bounty: 0,
        armor_class: 0,
        boss: true,
    },
];

/// Index of the boss enemy def.
pub const SAMWISE: u16 = 2;

// Match timeline (ticks @ 30 Hz). Enemy scaling steps at 10 and 15 minutes; the
// boss spawns at 15 minutes (`docs/02 §2.3`, adapted from the source map).
pub const SCALE_STEP_1_TICK: u32 = 10 * 60 * 30; // 18000 — 10 min
pub const SCALE_STEP_2_TICK: u32 = 15 * 60 * 30; // 27000 — 15 min
/// Samwise spawns here; normal waves stop.
pub const BOSS_SPAWN_TICK: u32 = SCALE_STEP_2_TICK;

/// Enemy HP scaling at `tick`: identity until 10 min, then ramps to ×2 by 15
/// min, then steeper "to bring the game to a swift end". Deterministic.
pub fn enemy_hp_mult(tick: u32) -> Fixed {
    let span = (SCALE_STEP_2_TICK - SCALE_STEP_1_TICK) as i64; // 5 min window
    if tick < SCALE_STEP_1_TICK {
        Fixed::ONE
    } else if tick < SCALE_STEP_2_TICK {
        // +100% linearly across minutes 10–15.
        Fixed::ONE + Fixed::from_ratio((tick - SCALE_STEP_1_TICK) as i64, span)
    } else {
        // ×2 at 15 min, then +100% per additional 5 minutes.
        Fixed::from_int(2) + Fixed::from_ratio((tick - SCALE_STEP_2_TICK) as i64, span)
    }
}

/// M0 wave: a steady mix, continuous (no scaling). Used every round.
pub static WAVE_M0: &[WaveSpawn] = &[
    WaveSpawn {
        enemy: 0,
        cadence_ticks: 15,
    },
    WaveSpawn {
        enemy: 1,
        cadence_ticks: 120,
    },
];

/// Enemies spawn on this ring (radius ~1500) and march toward the tank.
/// Precomputed (no trig) so spawn positions are deterministic.
pub static SPAWN_RING: &[Vec2] = &[
    v(1500, 0),
    v(1061, 1061),
    v(0, 1500),
    v(-1061, 1061),
    v(-1500, 0),
    v(-1061, -1061),
    v(0, -1500),
    v(1061, -1061),
];

const fn v(x: i64, y: i64) -> Vec2 {
    Vec2 {
        x: Fixed::from_int(x),
        y: Fixed::from_int(y),
    }
}

/// Armor/damage matrix: `DAMAGE_MATRIX[damage_type][armor_class]` as a Fixed
/// multiplier (adapted from `war3mapMisc.txt`). M0 uses two armor classes.
pub fn damage_multiplier(damage_type: u8, armor_class: u8) -> Fixed {
    // rows = Normal, Piercing, Magic, Siege, Chaos ; cols = armor class 0, 1
    const M: [[(i64, i64); 2]; 5] = [
        [(1, 1), (1, 1)], // Normal
        [(2, 1), (1, 1)], // Piercing: 2x vs class 0
        [(1, 1), (2, 1)], // Magic:    2x vs class 1
        [(1, 1), (1, 2)], // Siege:    0.5x vs class 1
        [(1, 1), (1, 1)], // Chaos
    ];
    let dt = damage_type as usize % 5;
    let ac = (armor_class as usize).min(1);
    let (n, d) = M[dt][ac];
    Fixed::from_ratio(n, d)
}
