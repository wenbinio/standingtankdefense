//! Wave spawning — AGENT B. Deterministic; randomness only from `s.rng_spawn`.
use crate::content;
use crate::state::*;

/// Phase 3: for each `content::WAVE_M0` entry, if `s.tick % cadence_ticks == 0`,
/// spawn one enemy of that def at a `content::SPAWN_RING` position chosen via
/// `s.rng_spawn.below(SPAWN_RING.len())`. Allocate ids with
/// `s.alloc_entity_id()`; push to `s.enemies` (keep id order). Set hp from
/// `EnemyDef::base_hp`.
pub(crate) fn spawn(s: &mut ArenaState) {
    // Boss phase: spawn Samwise exactly once at the boss tick; afterwards normal
    // waves stop (the shop "flees"). Samwise uses its fixed HP (no scaling) and
    // is immune to weapon fire — only `Clear` damages it (handled in combat).
    if s.tick == content::BOSS_SPAWN_TICK {
        let edef = &content::ENEMIES[content::SAMWISE as usize];
        let id = s.alloc_entity_id();
        s.enemies
            .push(Enemy::new(id, content::SAMWISE, edef.base_hp, content::SPAWN_RING[0]));
        return;
    }
    if s.tick >= content::BOSS_SPAWN_TICK {
        return;
    }

    // Enemy HP scales with match time (identity until 10 min).
    let hp_mult = content::enemy_hp_mult(s.tick);
    // Process wave entries in their fixed catalog order so the rng_spawn draws
    // happen in a deterministic sequence.
    for ws in content::WAVE_M0 {
        if ws.cadence_ticks == 0 {
            continue;
        }
        if s.tick % ws.cadence_ticks == 0 {
            let ring_idx = s.rng_spawn.below(content::SPAWN_RING.len() as u32) as usize;
            let pos = content::SPAWN_RING[ring_idx];
            let edef = &content::ENEMIES[ws.enemy as usize];
            let hp = hp_mult.scale_i64(edef.base_hp);
            let id = s.alloc_entity_id();
            s.enemies.push(Enemy::new(id, ws.enemy, hp, pos));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ArenaState;
    use determinism::Fixed;

    fn blank_state() -> ArenaState {
        // Only spawn() is exercised here; never sim::step() (other agents' stubs
        // panic). Starting Bow weapon is irrelevant to spawning.
        ArenaState::new(0x1234_5678, 0)
    }

    #[test]
    fn spawns_on_cadence_for_first_wave_entry() {
        // WAVE_M0[0]: enemy 0, cadence 15.  WAVE_M0[1]: enemy 1, cadence 120.
        let mut s = blank_state();
        s.tick = 15; // 15 % 15 == 0, 15 % 120 != 0
        let before = s.enemies.len();
        spawn(&mut s);
        assert_eq!(s.enemies.len(), before + 1, "exactly one enemy (entry 0)");
        assert_eq!(s.enemies[0].def, 0);
        // hp from EnemyDef::base_hp (Fel Orc Grunt = 200).
        assert_eq!(s.enemies[0].hp, 200);
    }

    #[test]
    fn no_spawn_off_cadence() {
        let mut s = blank_state();
        s.tick = 7; // 7 % 15 != 0 and 7 % 120 != 0
        spawn(&mut s);
        assert!(s.enemies.is_empty(), "nothing spawns off-cadence");
    }

    #[test]
    fn tick_zero_spawns_both_entries() {
        // 0 % anything == 0 → both wave entries fire on tick 0.
        let mut s = blank_state();
        s.tick = 0;
        spawn(&mut s);
        assert_eq!(s.enemies.len(), 2);
        assert_eq!(s.enemies[0].def, 0);
        assert_eq!(s.enemies[1].def, 1);
        assert_eq!(s.enemies[1].hp, 1200); // Steam Tank base_hp
    }

    #[test]
    fn spawns_at_ring_position() {
        let mut s = blank_state();
        s.tick = 15;
        spawn(&mut s);
        let pos = s.enemies[0].pos;
        // The spawn must be exactly one of the precomputed ring positions.
        assert!(
            content::SPAWN_RING.iter().any(|&r| r == pos),
            "enemy must spawn on the SPAWN_RING"
        );
        // And its distance-from-origin squared must equal a ring radius^2
        // (sanity that we used a ring entry, not the origin).
        assert_ne!(pos, Vec2::new(Fixed::ZERO, Fixed::ZERO));
    }

    #[test]
    fn ids_are_monotonic_and_ordered() {
        let mut s = blank_state();
        s.tick = 0;
        spawn(&mut s);
        assert!(s.enemies[0].id < s.enemies[1].id, "ids allocated in order");
    }

    #[test]
    fn scaling_is_identity_before_ten_minutes() {
        // Early game (the golden-checksum window) must be unscaled.
        assert_eq!(content::enemy_hp_mult(0), determinism::Fixed::ONE);
        assert_eq!(content::enemy_hp_mult(2000), determinism::Fixed::ONE);
        assert_eq!(
            content::enemy_hp_mult(content::SCALE_STEP_1_TICK - 1),
            determinism::Fixed::ONE
        );
        // At 15 min, enemies have ~2× HP.
        assert_eq!(
            content::enemy_hp_mult(content::SCALE_STEP_2_TICK).scale_i64(1000),
            2000
        );
    }

    #[test]
    fn late_game_enemies_spawn_with_scaled_hp() {
        let mut s = blank_state();
        s.tick = content::SCALE_STEP_2_TICK; // 15 min, but this is the boss tick…
        // …so use a tick just before the boss where scaling is ~2×.
        s.tick = content::SCALE_STEP_2_TICK - 15; // a grunt-cadence tick before boss
        // ensure it is a grunt cadence tick (cadence 15)
        assert_eq!(s.tick % 15, 0);
        spawn(&mut s);
        let grunt_base = content::ENEMIES[0].base_hp;
        // hp should be roughly 2× base (just under, since ~tick before 15 min).
        assert!(s.enemies[0].hp > grunt_base, "late enemy HP must be scaled up");
        assert!(s.enemies[0].hp <= grunt_base * 2);
    }

    #[test]
    fn boss_spawns_once_at_boss_tick_then_no_normal_waves() {
        let mut s = blank_state();
        s.tick = content::BOSS_SPAWN_TICK;
        spawn(&mut s);
        assert_eq!(s.enemies.len(), 1, "exactly Samwise spawns at the boss tick");
        assert_eq!(s.enemies[0].def, content::SAMWISE);
        assert!(content::ENEMIES[s.enemies[0].def as usize].boss);

        // After the boss tick, normal waves no longer spawn.
        s.enemies.clear();
        s.tick = content::BOSS_SPAWN_TICK + 15; // a former grunt-cadence tick
        spawn(&mut s);
        assert!(s.enemies.is_empty(), "no normal waves during the boss phase");
    }

    #[test]
    fn spawn_is_deterministic_same_seed() {
        let mut a = blank_state();
        let mut b = a.clone();
        a.tick = 15;
        b.tick = 15;
        spawn(&mut a);
        spawn(&mut b);
        assert_eq!(a.enemies, b.enemies, "same seed/tick → identical spawn");
        assert_eq!(a.rng_spawn.state(), b.rng_spawn.state());

        // Different ring picks over successive cadence ticks remain reproducible.
        let mut c = blank_state();
        let mut d = c.clone();
        let mut pc = Vec::new();
        let mut pd = Vec::new();
        for t in [15u32, 30, 45, 60, 75] {
            c.tick = t;
            d.tick = t;
            spawn(&mut c);
            spawn(&mut d);
        }
        for e in &c.enemies { pc.push(e.pos); }
        for e in &d.enemies { pd.push(e.pos); }
        assert_eq!(pc, pd);
    }
}
