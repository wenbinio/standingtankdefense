//! Wave spawning — AGENT B. Deterministic; randomness only from `s.rng_spawn`.
use crate::content;
use crate::state::*;

/// Phase 3: for each `content::WAVE_M0` entry, if `s.tick % cadence_ticks == 0`,
/// spawn one enemy of that def at a `content::SPAWN_RING` position chosen via
/// `s.rng_spawn.below(SPAWN_RING.len())`. Allocate ids with
/// `s.alloc_entity_id()`; push to `s.enemies` (keep id order). Set hp from
/// `EnemyDef::base_hp`.
pub(crate) fn spawn(s: &mut ArenaState) {
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
            let id = s.alloc_entity_id();
            s.enemies.push(Enemy {
                id,
                def: ws.enemy,
                hp: edef.base_hp,
                pos,
            });
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
