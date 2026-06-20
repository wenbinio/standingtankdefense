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

/// Attack behavior for a weapon.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Attack {
    SingleTarget,
    /// Area splash of the given radius (in Fixed integer units) at impact.
    Splash(i64),
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
}

#[derive(Clone, Copy, Debug)]
pub struct EnemyDef {
    pub name: &'static str,
    pub base_hp: i64,
    pub move_speed: i64, // units per tick
    pub contact_damage: i64,
    pub bounty: i64,
    pub armor_class: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct WaveSpawn {
    pub enemy: u16,
    pub cadence_ticks: u32, // spawn one every N ticks
}

/// The weapon the tank starts with (index into [`WEAPONS`]).
pub const STARTING_WEAPON: u16 = 0;

/// M0 weapon catalog (subset; stats adapted from Appendix A).
pub static WEAPONS: &[WeaponDef] = &[
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
    },
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
    },
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
    },
];

/// M0 enemy catalog.
pub static ENEMIES: &[EnemyDef] = &[
    EnemyDef {
        name: "Fel Orc Grunt",
        base_hp: 200,
        move_speed: 8,
        contact_damage: 500,
        bounty: 10,
        armor_class: 0,
    },
    EnemyDef {
        name: "Steam Tank",
        base_hp: 1200,
        move_speed: 4,
        contact_damage: 1500,
        bounty: 40,
        armor_class: 1,
    },
];

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
