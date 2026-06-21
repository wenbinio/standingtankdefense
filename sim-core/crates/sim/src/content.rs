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
#[derive(Clone, Copy, Debug)]
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
}

#[derive(Clone, Copy, Debug)]
pub struct ModifierDef {
    pub name: &'static str,
    pub rarity: u8,
    pub cost: i64,
    pub effect: ModEffect,
}

/// M4 modifier catalog — a representative slice across every scope (additive
/// global/by-type, multiplicative, attack-speed, economy, defensive). Numbers
/// adapted from the extracted upgrades (`docs/appendix-A-map-extraction.md`).
pub static MODIFIERS: &[ModifierDef] = &[
    ModifierDef { name: "+10% Damage", rarity: 0, cost: 500, effect: ModEffect::DamageGlobalPct(1, 10) },
    ModifierDef { name: "+10% Piercing Damage", rarity: 0, cost: 500, effect: ModEffect::DamageTypePct(DMG_PIERCING, 1, 10) },
    ModifierDef { name: "+10% Siege Damage", rarity: 0, cost: 500, effect: ModEffect::DamageTypePct(DMG_SIEGE, 1, 10) },
    ModifierDef { name: "+10% Magic Damage", rarity: 0, cost: 500, effect: ModEffect::DamageTypePct(DMG_MAGIC, 1, 10) },
    ModifierDef { name: "+25% Damage (Epic)", rarity: 3, cost: 5000, effect: ModEffect::DamageMulPct(1, 4) },
    ModifierDef { name: "+10% Attack Speed", rarity: 0, cost: 500, effect: ModEffect::AttackSpeedPct(1, 10) },
    ModifierDef { name: "+50% Kill Bounty", rarity: 1, cost: 1500, effect: ModEffect::BountyPct(1, 2) },
    ModifierDef { name: "+20 Gold Income", rarity: 0, cost: 500, effect: ModEffect::IncomeFlat(20) },
    ModifierDef { name: "+2000 Max HP", rarity: 1, cost: 1500, effect: ModEffect::MaxHp(2000) },
];

/// The weapon the tank starts with (index into [`WEAPONS`]).
pub const STARTING_WEAPON: u16 = 0;

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
