//! The authoritative arena data model + deterministic vector math.
//! Owned centrally (the cross-agent seam). Behavior modules READ/WRITE these
//! fields but must not change the struct definitions.

use crate::content;
use crate::ids::*;
use determinism::{Fixed, Rng};

/// 2D point/vector in Fixed units. The tank sits at the origin.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Vec2 {
    pub x: Fixed,
    pub y: Fixed,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 {
        x: Fixed::ZERO,
        y: Fixed::ZERO,
    };
    #[inline]
    pub fn new(x: Fixed, y: Fixed) -> Vec2 {
        Vec2 { x, y }
    }
    /// Squared distance to `o` (no sqrt; use for range checks vs `range*range`).
    #[inline]
    pub fn dist_sq(self, o: Vec2) -> Fixed {
        let dx = o.x - self.x;
        let dy = o.y - self.y;
        dx.mul(dx) + dy.mul(dy)
    }
    /// Move from `self` toward `target` by at most `max_step`; clamps to target
    /// on arrival. Fully deterministic (integer sqrt). Returns the new point.
    pub fn step_toward(self, target: Vec2, max_step: Fixed) -> Vec2 {
        let dx = target.x - self.x;
        let dy = target.y - self.y;
        let d2 = dx.mul(dx) + dy.mul(dy);
        let step2 = max_step.mul(max_step);
        if d2 <= step2 || d2 == Fixed::ZERO {
            return target;
        }
        let dist = d2.sqrt();
        Vec2 {
            x: self.x + dx.mul(max_step).div(dist),
            y: self.y + dy.mul(max_step).div(dist),
        }
    }
}

/// The player's stationary tank.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Tank {
    pub hp: i64,
    pub max_hp: i64,
    pub pos: Vec2,
    pub clear_cooldown_end: Tick,
}

/// An owned weapon instance (multiple copies of one def stack as separate
/// instances — the source's "everything stacks").
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WeaponInstance {
    pub instance_id: EntityId,
    pub def: u16, // index into content::WEAPONS
    pub next_fire_tick: Tick,
}

/// An active enemy.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Enemy {
    pub id: EntityId,
    pub def: u16, // index into content::ENEMIES
    pub hp: i64,
    pub pos: Vec2,
}

/// An in-flight projectile (homes on `target`; applies splash at arrival).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Projectile {
    pub id: EntityId,
    pub pos: Vec2,
    pub target: EntityId,
    pub last_target_pos: Vec2,
    pub damage: i64,
    pub damage_type: u8,
    pub splash_radius: Fixed, // ZERO ⇒ single target
    pub speed: Fixed,
}

/// Player economy state.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Economy {
    pub gold: i64,
    pub income_per_tick: i64, // base passive income (no multiplier — source rule)
    pub bounty_mult: Fixed,   // applies to kill bounty only
    pub rerolls_remaining: u32,
    pub reroll_cost: i64,
}

/// One purchasable shop slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Offer {
    pub weapon_def: u16,
    pub cost: i64,
}

/// The per-round shop. M0 offers weapons only.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ShopState {
    pub offers: Vec<Offer>,
    pub shop_seq: u32,
}

/// The complete authoritative arena state. `step()` is a pure function of this
/// plus the tick's `Input` (`docs/03 §3.3`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ArenaState {
    pub tick: Tick,
    pub round: u32, // u32::MAX sentinel before the first round starts
    pub master_seed: u64,
    pub player_id: u32,

    pub tank: Tank,
    pub weapons: Vec<WeaponInstance>,
    pub enemies: Vec<Enemy>,
    pub projectiles: Vec<Projectile>,
    pub economy: Economy,
    pub shop: ShopState,

    pub next_entity_id: u32,
    pub dead: bool,
    pub death_tick: Option<Tick>,

    // Per-purpose RNG streams (cursors ride in snapshots).
    pub rng_spawn: Rng,
    pub rng_targeting: Rng,
    pub rng_shop: Rng,
    pub rng_reroll: Rng,
    pub rng_proc: Rng,
}

impl ArenaState {
    /// Fresh arena for `master_seed`/`player_id`. Starts with one Bow so combat
    /// is exercised from tick 0; the first `step()` generates the round-0 shop.
    pub fn new(master_seed: u64, player_id: u32) -> ArenaState {
        let d = |p: Purpose| Rng::derive(master_seed, player_id, p as u32, 0);
        let mut s = ArenaState {
            tick: 0,
            round: u32::MAX,
            master_seed,
            player_id,
            tank: Tank {
                hp: 24_000,
                max_hp: 24_000,
                pos: Vec2::ZERO,
                clear_cooldown_end: 0,
            },
            weapons: Vec::new(),
            enemies: Vec::new(),
            projectiles: Vec::new(),
            economy: Economy {
                gold: 500,
                income_per_tick: 20, // 600 gold/s baseline (tuning)
                bounty_mult: Fixed::ONE,
                rerolls_remaining: 5,
                reroll_cost: 100,
            },
            shop: ShopState::default(),
            next_entity_id: 1,
            dead: false,
            death_tick: None,
            rng_spawn: d(Purpose::Spawn),
            rng_targeting: d(Purpose::Targeting),
            rng_shop: d(Purpose::Shop),
            rng_reroll: d(Purpose::Reroll),
            rng_proc: d(Purpose::Proc),
        };
        let id = s.alloc_entity_id();
        s.weapons.push(WeaponInstance {
            instance_id: id,
            def: content::STARTING_WEAPON,
            next_fire_tick: 0,
        });
        s
    }

    /// Allocate a fresh, never-reused entity id.
    #[inline]
    pub fn alloc_entity_id(&mut self) -> EntityId {
        let id = EntityId(self.next_entity_id);
        self.next_entity_id += 1;
        id
    }

    /// Index of a live enemy by id, if present.
    pub fn enemy_index(&self, id: EntityId) -> Option<usize> {
        self.enemies.iter().position(|e| e.id == id)
    }
}
