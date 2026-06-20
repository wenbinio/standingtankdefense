//! Standing Tank Defense — deterministic simulation core (M0).
//!
//! Engine-independent (no Godot types). Pure, fixed-tick, fixed-point. The
//! whole netcode rests on `step()` being a deterministic function of
//! `(state, input)` (`docs/03 §3.3`, `docs/05 §5.6`).
//!
//! Module ownership (see `CLAUDE.md`):
//! - centrally owned seams: `ids`, `state`, `content`, this file (`step`/`checksum`).
//! - behavior modules: `combat`, `waves` (Agent B); `economy`, `shop`, `input` (Agent C).

pub mod content;
mod combat;
mod economy;
mod ids;
mod input;
mod shop;
mod state;
mod waves;

pub use ids::*;
pub use state::*;

use determinism::Checksum;

/// Simulation frequency. The only clock the sim knows.
pub const TICK_HZ: u32 = 30;
/// Ticks per round (a new shop opens each round). 30 s @ 30 Hz.
pub const ROUND_TICKS: u32 = 30 * TICK_HZ;

/// Advance the arena by exactly one tick, applying `inp` (this player's action).
///
/// The order of operations below IS part of the spec: client and the
/// validating shadow-sim must apply phases in this exact sequence so their
/// `checksum()` agree (`docs/05 §5.6.1`).
pub fn step(s: &mut ArenaState, inp: Input) {
    if s.dead {
        // Frozen after death, but keep advancing the tick so checksum traces
        // across runs stay length-aligned.
        s.tick += 1;
        return;
    }

    // 1. Round boundary → refresh shop + per-round economy.
    let new_round = s.tick / ROUND_TICKS;
    if new_round != s.round {
        s.round = new_round;
        shop::generate_offers(s);
        economy::on_round_start(s);
    }
    // 2. Player input (buy / reroll / clear).
    input::apply(s, inp);
    // 3. Spawn enemies for the current wave.
    waves::spawn(s);
    // 4. Weapons select targets and emit projectiles.
    combat::fire_weapons(s);
    // 5. Projectiles move; on arrival apply (splash) damage; kills award bounty.
    combat::advance_projectiles(s);
    // 6. Enemies advance toward the tank; contact damage on arrival.
    combat::move_enemies(s);
    // 7. Passive income.
    economy::tick_income(s);
    // 8. Death check (tank hp ≤ 0).
    economy::resolve_deaths(s);

    s.tick += 1;
}

/// Deterministic `state_checksum` over all authoritative-relevant fields.
/// Iterates entities in id-sorted order so it is independent of vector
/// ordering (`docs/04 §4.4.4`). Render-only state is excluded by construction.
pub fn checksum(s: &ArenaState) -> u64 {
    let mut c = Checksum::new();
    c.write_u32(s.tick);
    c.write_u32(s.round);
    c.write_u64(s.master_seed);
    c.write_u32(s.player_id);

    c.write_i64(s.tank.hp);
    c.write_i64(s.tank.max_hp);
    c.write_fixed(s.tank.pos.x);
    c.write_fixed(s.tank.pos.y);
    c.write_u32(s.tank.clear_cooldown_end);

    c.write_i64(s.economy.gold);
    c.write_i64(s.economy.income_per_tick);
    c.write_fixed(s.economy.bounty_mult);
    c.write_u32(s.economy.rerolls_remaining);
    c.write_i64(s.economy.reroll_cost);

    c.write_u32(s.next_entity_id);
    c.write_u32(s.dead as u32);
    c.write_u32(s.death_tick.unwrap_or(u32::MAX));

    c.write_u64(s.rng_spawn.state());
    c.write_u64(s.rng_targeting.state());
    c.write_u64(s.rng_shop.state());
    c.write_u64(s.rng_reroll.state());
    c.write_u64(s.rng_proc.state());

    // Weapons (instance ids are monotonic; checksum in id order).
    let mut w: Vec<&WeaponInstance> = s.weapons.iter().collect();
    w.sort_by_key(|x| x.instance_id);
    c.write_u32(w.len() as u32);
    for x in w {
        c.write_u32(x.instance_id.0);
        c.write_u32(x.def as u32);
        c.write_u32(x.next_fire_tick);
    }

    // Shop offers (slot order is meaningful).
    c.write_u32(s.shop.shop_seq);
    c.write_u32(s.shop.offers.len() as u32);
    for o in &s.shop.offers {
        c.write_u32(o.weapon_def as u32);
        c.write_i64(o.cost);
    }

    // Enemies in id order.
    let mut e: Vec<&Enemy> = s.enemies.iter().collect();
    e.sort_by_key(|x| x.id);
    c.write_u32(e.len() as u32);
    for x in e {
        c.write_u32(x.id.0);
        c.write_u32(x.def as u32);
        c.write_i64(x.hp);
        c.write_fixed(x.pos.x);
        c.write_fixed(x.pos.y);
    }

    // Projectiles in id order.
    let mut p: Vec<&Projectile> = s.projectiles.iter().collect();
    p.sort_by_key(|x| x.id);
    c.write_u32(p.len() as u32);
    for x in p {
        c.write_u32(x.id.0);
        c.write_fixed(x.pos.x);
        c.write_fixed(x.pos.y);
        c.write_u32(x.target.0);
        c.write_i64(x.damage);
        c.write_u32(x.damage_type as u32);
        c.write_fixed(x.splash_radius);
        c.write_fixed(x.speed);
    }

    c.finish()
}
