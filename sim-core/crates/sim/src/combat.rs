//! Combat behavior — AGENT B. Implement the three phases below. Read content via
//! `crate::content`, use `s.rng_targeting`/`s.rng_proc` for any randomness, and
//! keep all math integer/Fixed (no floats). Preserve determinism: iterate in a
//! stable order (by `instance_id` / `id`); never iterate a HashMap.
use crate::content::{self, Attack};
use crate::economy;
use crate::state::*;
use determinism::Fixed;

/// Phase 4: each ready weapon picks a target (RANDOM among enemies in range,
/// via `s.rng_targeting` — matches the source's "attack at random") and emits a
/// `Projectile`. Update `next_fire_tick = s.tick + cooldown`. No target ⇒ no fire.
pub(crate) fn fire_weapons(s: &mut ArenaState) {
    let _ = (&content::WEAPONS, Fixed::ZERO, s);
    todo!("AGENT B: fire_weapons")
}

/// Phase 5: move each projectile toward its target by `speed` (use
/// `Vec2::step_toward`). On arrival apply damage: single-target hits the target;
/// `Attack::Splash(r)` hits all enemies within `r` of the impact point. Apply
/// `content::damage_multiplier(dmg_type, enemy.armor_class)`. Remove dead enemies
/// and call `economy::award_bounty(s, enemy_def)` for each kill. Remove arrived
/// projectiles (and projectiles whose target vanished — detonate at last_target_pos).
pub(crate) fn advance_projectiles(s: &mut ArenaState) {
    let _ = (Attack::SingleTarget, economy::on_round_start as fn(&mut ArenaState), s);
    todo!("AGENT B: advance_projectiles")
}

/// Phase 6: move each enemy toward the tank (origin) by its `move_speed`. On
/// contact (reaches the tank) deal `contact_damage` to `s.tank.hp` and remove
/// the enemy. Keep `s.enemies` ordered by id (append on spawn; stable removal).
pub(crate) fn move_enemies(s: &mut ArenaState) {
    todo!("AGENT B: move_enemies")
}
