//! Wave spawning — AGENT B. Deterministic; randomness only from `s.rng_spawn`.
use crate::content;
use crate::state::*;

/// Phase 3: for each `content::WAVE_M0` entry, if `s.tick % cadence_ticks == 0`,
/// spawn one enemy of that def at a `content::SPAWN_RING` position chosen via
/// `s.rng_spawn.below(SPAWN_RING.len())`. Allocate ids with
/// `s.alloc_entity_id()`; push to `s.enemies` (keep id order). Set hp from
/// `EnemyDef::base_hp`.
pub(crate) fn spawn(s: &mut ArenaState) {
    // Boss phase (30 min+): spawn Hungry Hungry Happypotamus exactly once at
    // the boss tick. The boss uses its fixed HP (no scaling) and is immune to
    // weapon fire — only `Clear` damages it (handled in combat). UNLIKE the old
    // design, normal waves do NOT fully stop: a relentless ESCORT swarm keeps
    // pouring in alongside the boss. The escort serves the climax three ways —
    //   1) contact-damage VOLUME that pressures even a heavily-defended tank,
    //   2) it forces the player to keep Clearing, and every Clear also chips the
    //      boss, so the boss is a real MULTI-CLEAR FIGHT rather than a stalemate,
    //   3) it makes the boss the wall most runs end at instead of a victory lap.
    if s.tick == content::BOSS_SPAWN_TICK {
        let edef = &content::ENEMIES[content::SAMWISE as usize];
        let id = s.alloc_entity_id();
        s.enemies
            .push(Enemy::new(id, content::SAMWISE, edef.base_hp, content::SPAWN_RING[0]));
        // fall through: the escort swarm below also spawns on this tick.
    }
    if s.tick >= content::BOSS_SPAWN_TICK {
        // Boss escort: a dense grunt/raider flood at the peak HP tier. Cadences
        // are tight so the board stays full (keeps the player's Clear cycling onto
        // the boss). Deterministic ring picks via `rng_spawn`, same as normal waves.
        let hp_mult = content::enemy_hp_mult(s.tick);
        for ws in content::BOSS_ESCORT {
            if ws.cadence_ticks != 0 && s.tick % ws.cadence_ticks == 0 {
                let ring_idx = s.rng_spawn.below(content::SPAWN_RING.len() as u32) as usize;
                let pos = content::SPAWN_RING[ring_idx];
                let edef = &content::ENEMIES[ws.enemy as usize];
                let hp = hp_mult.scale_i64(edef.base_hp);
                let id = s.alloc_entity_id();
                s.enemies.push(Enemy::new(id, ws.enemy, hp, pos));
            }
        }
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
        // Escalation gate: this entry is dormant until its start_tick.
        if s.tick < ws.start_tick {
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
        // WAVE_M0[0]: enemy 0 (Fel Orc Grunt), cadence 6.  WAVE_M0[1]: enemy 1
        // (Steam Tank), cadence 120. Tick 6 fires only the grunt entry (all gated
        // entries — peon/raider/etc — start well after tick 6).
        let mut s = blank_state();
        s.tick = 6; // 6 % 6 == 0, 6 % 120 != 0
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
        s.tick = 7; // 7 % 6 != 0, 7 % 120 != 0, and below all gated entries
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
        s.tick = 6; // grunt cadence (6) fires
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
        // Use a grunt-cadence (6) tick just before the 15-min cliff, where the
        // 10→15 ramp has nearly reached ×2 (and is still ≤ ×2). Grunt is wave
        // entry 0 (catalog order), so it is `enemies[0]` among this tick's spawns.
        let mut s = blank_state();
        s.tick = content::SCALE_STEP_2_TICK - 6; // 26994, a grunt-cadence tick
        assert_eq!(s.tick % 6, 0);
        spawn(&mut s);
        let grunt_base = content::ENEMIES[0].base_hp;
        assert_eq!(s.enemies[0].def, 0, "first spawn this tick is the grunt (entry 0)");
        // hp should be just under 2× base (a tick before the 15-min ×2 tier).
        assert!(s.enemies[0].hp > grunt_base, "late enemy HP must be scaled up");
        assert!(s.enemies[0].hp <= grunt_base * 2);
    }

    #[test]
    fn boss_spawns_once_at_boss_tick_with_escort_then_only_escort() {
        // At the boss tick the boss spawns exactly once; the BOSS_ESCORT swarm may
        // also spawn this tick (all escort cadences divide the boss tick). Assert
        // exactly ONE boss is present and that the boss is among the spawns.
        let mut s = blank_state();
        s.tick = content::BOSS_SPAWN_TICK;
        spawn(&mut s);
        let bosses = s
            .enemies
            .iter()
            .filter(|e| content::ENEMIES[e.def as usize].boss)
            .count();
        assert_eq!(bosses, 1, "exactly one boss spawns at the boss tick");
        assert!(s.enemies.iter().any(|e| e.def == content::SAMWISE));

        // The boss spawns ONLY once: at a later boss-phase tick no second boss
        // appears, but the escort swarm keeps coming (the climax is a real fight).
        s.enemies.clear();
        s.tick = content::BOSS_SPAWN_TICK + 4; // an escort grunt-cadence tick (4)
        spawn(&mut s);
        assert!(
            s.enemies.iter().all(|e| !content::ENEMIES[e.def as usize].boss),
            "the boss spawns once, not again during the boss phase"
        );
        assert!(!s.enemies.is_empty(), "the escort swarm keeps spawning in the boss phase");
    }

    #[test]
    fn every_roster_enemy_spawns_over_the_match() {
        // Walk the whole pre-boss timeline; collect every enemy def that spawns.
        // Every catalog enemy except the boss (the Happypotamus, which arrives via the
        // dedicated boss tick) must appear via the escalating WAVE_M0 schedule.
        let mut s = blank_state();
        let mut seen = std::collections::BTreeSet::new();
        let mut t = 0u32;
        while t < content::BOSS_SPAWN_TICK {
            s.tick = t;
            let before = s.enemies.len();
            spawn(&mut s);
            for e in &s.enemies[before..] {
                seen.insert(e.def);
            }
            t += 1;
        }
        for (idx, def) in content::ENEMIES.iter().enumerate() {
            if def.boss {
                continue;
            }
            assert!(
                seen.contains(&(idx as u16)),
                "roster enemy {} (def {idx}) never spawned in WAVE_M0",
                def.name
            );
        }
        // Sanity: the boss itself spawns at the boss tick.
        let mut bs = blank_state();
        bs.tick = content::BOSS_SPAWN_TICK;
        spawn(&mut bs);
        assert_eq!(bs.enemies[0].def, content::SAMWISE);
    }

    #[test]
    fn gated_entries_dormant_until_start_tick() {
        // The Fel Orc Peon (def 3) is gated; before its start_tick it must not
        // spawn even on a tick divisible by its cadence.
        let peon = content::ENEMIES.iter().position(|e| e.name == "Fel Orc Peon").unwrap() as u16;
        let ws = content::WAVE_M0.iter().find(|w| w.enemy == peon).unwrap();
        assert!(ws.start_tick > 0, "peon entry is gated");
        // Largest cadence-multiple strictly below the gate.
        let mut pre = (ws.start_tick / ws.cadence_ticks) * ws.cadence_ticks;
        if pre >= ws.start_tick {
            pre -= ws.cadence_ticks;
        }
        let mut s = blank_state();
        s.tick = pre;
        assert!(s.tick < ws.start_tick);
        assert_eq!(s.tick % ws.cadence_ticks, 0);
        let before: usize = s.enemies.iter().filter(|e| e.def == peon).count();
        spawn(&mut s);
        let after: usize = s.enemies.iter().filter(|e| e.def == peon).count();
        assert_eq!(after, before, "gated peon must not spawn before its start_tick");
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
