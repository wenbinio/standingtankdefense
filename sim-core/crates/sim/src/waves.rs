//! Wave spawning — AGENT B. Deterministic; randomness only from `s.rng_spawn`.
use crate::content;
use crate::state::*;

/// Phase 3: for each `content::WAVE_M0` entry, if `s.tick % cadence_ticks == 0`,
/// spawn one enemy of that def at a `content::SPAWN_RING` position chosen via
/// `s.rng_spawn.below(SPAWN_RING.len())`. Allocate ids with
/// `s.alloc_entity_id()`; push to `s.enemies` (keep id order). Set hp from
/// `EnemyDef::base_hp`.
pub(crate) fn spawn(s: &mut ArenaState) {
    // Boss phase (30 min+): spawn The Hippocrate exactly once at
    // the boss tick. The boss uses its fixed HP (no scaling) and is immune to
    // weapon fire — only `Clear` damages it (handled in combat). UNLIKE the old
    // design, normal waves do NOT fully stop: a relentless ESCORT swarm keeps
    // pouring in alongside the boss. The escort serves the climax three ways —
    //   1) contact-damage VOLUME that pressures even a heavily-defended tank,
    //   2) it forces the player to keep Clearing, and every Clear also chips the
    //      boss, so the boss is a real MULTI-CLEAR FIGHT rather than a stalemate,
    //   3) it makes the boss the wall most runs end at instead of a victory lap.
    if s.tick == content::BOSS_SPAWN_TICK {
        let edef = &content::ENEMIES[content::BOSS as usize];
        let id = s.alloc_entity_id();
        s.enemies.push(Enemy::new(
            id,
            content::BOSS,
            edef.base_hp,
            content::SPAWN_RING[0],
        ));
        s.emit(SimEvent::BossSpawned { id: id.0 });
        // fall through: the escort swarm below also spawns on this tick.
    }
    if s.tick >= content::BOSS_SPAWN_TICK {
        // Boss escort: a dense grunt/raider flood at the peak HP tier. Cadences
        // are tight so the board stays full (keeps the player's Clear cycling onto
        // the boss). Deterministic ring picks via `rng_spawn`, same as normal waves.
        let hp_mult = content::enemy_hp_mult(s.tick);
        for ws in content::BOSS_ESCORT {
            if ws.cadence_ticks != 0 && s.tick.is_multiple_of(ws.cadence_ticks) {
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
        if s.tick.is_multiple_of(ws.cadence_ticks) {
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
        // WAVE_M0[0]: Squeakzilla at `EARLY_GRUNT_CADENCE` (18); WAVE_M0[1]: Fanged
        // Death at 95. The swarm cadence fires only the swarm entry (all gated entries
        // — chaff/rusher/etc — start well after, and 18 % 95 != 0).
        let g = content::EARLY_GRUNT_CADENCE;
        let mut s = blank_state();
        s.tick = g; // g % g == 0, g % 95 != 0
        let before = s.enemies.len();
        spawn(&mut s);
        assert_eq!(s.enemies.len(), before + 1, "exactly one enemy (entry 0)");
        assert_eq!(s.enemies[0].def, 0);
        // hp from EnemyDef::base_hp (Squeakzilla = 200).
        assert_eq!(s.enemies[0].hp, 200);
    }

    #[test]
    fn no_spawn_off_cadence() {
        let mut s = blank_state();
        s.tick = 7; // 7 % 18 != 0, 7 % 95 != 0, and below all gated entries
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
        assert_eq!(s.enemies[1].hp, 1200); // Fanged Death base_hp
    }

    #[test]
    fn spawns_at_ring_position() {
        let mut s = blank_state();
        s.tick = content::EARLY_GRUNT_CADENCE; // grunt cadence fires
        spawn(&mut s);
        let pos = s.enemies[0].pos;
        // The spawn must be exactly one of the precomputed ring positions.
        assert!(
            content::SPAWN_RING.contains(&pos),
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
    fn hp_curve_is_a_3min_stepped_ramp() {
        // BALANCE PASS: the curve is a STEPPED "RAMP" on a strict 3-min cadence —
        // every 5400-tick interval is `gentle climb → warning → step`, with steps at
        // k*5400 for k=1..=10. The old ×413863 hack is gone; the curve escalates
        // SMOOTHLY to the dialed boss endpoint (≈ ×120 after the catalog-fidelity
        // pass re-dialed `RAMP_JUMP` for the stronger catalog). Pins the shape.
        use determinism::Fixed;
        let m = content::enemy_hp_mult;
        let interval = content::RAMP_INTERVAL; // 5400
        assert_eq!(interval, 5400);

        // (1) Starts at exactly ×1 and is monotonic non-decreasing across the
        //     whole curve (no downward steps anywhere).
        assert_eq!(m(0), Fixed::ONE);
        let mut prev = Fixed::ONE;
        let mut t = 0u32;
        while t <= content::BOSS_SPAWN_TICK + 600 {
            let cur = m(t);
            assert!(cur >= prev, "curve dipped at tick {t}");
            prev = cur;
            t += 30; // sample once per second — fast and dense enough
        }

        // (2) Exact post-step multipliers at each 3-min boundary (×10000),
        //     compounding ×1.6141 per interval up to ≈ ×120 at the boss. These are
        //     the exact `Fixed` values of the catalog-fidelity-pass curve (`G·W·J`
        //     with J = 1.55 — re-dialed UP for the far stronger re-anchored
        //     catalog, capped by boss reachability); the boss clamp returns
        //     `base(10)`.
        let post: [(u32, i64); 11] = [
            (0, 10000),
            (5400, 16141),
            (10800, 26053),
            (16200, 42053),
            (21600, 67879),
            (27000, 109566),
            (32400, 176855),
            (37800, 285467),
            (43200, 460783),
            (48600, 743765),
            (54000, 1200536), // the 30-min boss tier — the ≈ ×120 endpoint.
        ];
        for (tick, mult10k) in post {
            assert_eq!(
                m(tick).scale_i64(10000),
                mult10k,
                "post-step mult at {tick}"
            );
        }
        // The boss phase HOLDS the endpoint `base(10)` (≈ ×120).
        let boss = m(content::BOSS_SPAWN_TICK);
        assert_eq!(boss.scale_i64(10000), 1200536);
        assert_eq!(
            m(content::BOSS_SPAWN_TICK + 5000),
            boss,
            "boss phase holds the endpoint"
        );

        // (3) Within an interval: a gentle region, then a STEEPER warning region.
        //     Verify on the k=1 interval [5400, 10800).
        let lo = 5400;
        let warn_start = lo + content::GENTLE_TICKS; // 9900
                                                     // Average slope over a 100-tick span in the gentle region vs the warning
                                                     // region (×1e6/tick) — a wide window avoids per-tick quantization noise.
        let gentle_slope = (m(warn_start - 100).scale_i64(1_000_000)
            - m(warn_start - 200).scale_i64(1_000_000))
            / 100;
        let warn_slope =
            (m(warn_start + 100).scale_i64(1_000_000) - m(warn_start).scale_i64(1_000_000)) / 100;
        assert!(gentle_slope > 0, "gentle region must rise");
        assert!(
            warn_slope >= gentle_slope * 3,
            "warning slope ({warn_slope}) must be perceptibly steeper than gentle ({gentle_slope})"
        );

        // (4) The boundary STEP: the jump from the pre-step (warning peak) value to
        //     the next interval's post-step value is a real instantaneous +55% step
        //     (J, re-dialed for the re-anchored catalog), larger than any single
        //     warning-region tick step.
        let pre_step = m(lo + interval - 1); // tick 10799, warning peak
        let post_step = m(lo + interval); //   tick 10800, post-step
        let step = post_step.scale_i64(1_000_000) - pre_step.scale_i64(1_000_000);
        assert!(
            step > warn_slope * 50,
            "boundary step must dwarf a single warning tick"
        );
        // +55% (J): post ≈ pre × 1.55 (within rounding, ×1000).
        assert_eq!(
            post_step.scale_i64(1000),
            pre_step.mul(Fixed::from_ratio(31, 20)).scale_i64(1000)
        );
    }

    #[test]
    fn late_game_enemies_spawn_with_scaled_hp() {
        // Use a grunt-cadence (6) tick late in the curve (just before the 15-min
        // boundary), where the multiplier is well above ×1. Grunt is wave entry 0
        // (catalog order), so it is `enemies[0]` among this tick's spawns.
        let mut s = blank_state();
        // A grunt-cadence (`EARLY_GRUNT_CADENCE`) tick just before the 15-min step.
        s.tick = content::SCALE_STEP_2_TICK - content::EARLY_GRUNT_CADENCE; // 26982
        assert_eq!(s.tick % content::EARLY_GRUNT_CADENCE, 0);
        spawn(&mut s);
        let grunt_base = content::ENEMIES[0].base_hp;
        assert_eq!(
            s.enemies[0].def, 0,
            "first spawn this tick is the grunt (entry 0)"
        );
        // hp should be scaled up (the curve is ~×7.1 here — k4 warning region,
        // just before the 15-min step — after the catalog-fidelity `RAMP_JUMP`
        // re-dial; a smooth escalation, not the old hack).
        assert!(
            s.enemies[0].hp > grunt_base * 5,
            "late enemy HP must be scaled up"
        );
        assert!(s.enemies[0].hp <= grunt_base * 10);
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
        assert!(s.enemies.iter().any(|e| e.def == content::BOSS));

        // The boss spawns ONLY once: at a later boss-phase tick no second boss
        // appears, but the escort swarm keeps coming (the climax is a real fight).
        s.enemies.clear();
        s.tick = content::BOSS_SPAWN_TICK + 4; // an escort grunt-cadence tick (4)
        spawn(&mut s);
        assert!(
            s.enemies
                .iter()
                .all(|e| !content::ENEMIES[e.def as usize].boss),
            "the boss spawns once, not again during the boss phase"
        );
        assert!(
            !s.enemies.is_empty(),
            "the escort swarm keeps spawning in the boss phase"
        );
    }

    #[test]
    fn every_roster_enemy_spawns_over_the_match() {
        // Walk the whole pre-boss timeline; collect every enemy def that spawns.
        // Every catalog enemy except the boss (The Hippocrate, which arrives via the
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
        assert_eq!(bs.enemies[0].def, content::BOSS);
    }

    #[test]
    fn gated_entries_dormant_until_start_tick() {
        // The Doomduck (def 3) is gated; before its start_tick it must not
        // spawn even on a tick divisible by its cadence.
        let peon = content::ENEMIES
            .iter()
            .position(|e| e.name == "Doomduck")
            .unwrap() as u16;
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
        assert_eq!(
            after, before,
            "gated peon must not spawn before its start_tick"
        );
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
        for e in &c.enemies {
            pc.push(e.pos);
        }
        for e in &d.enemies {
            pd.push(e.pos);
        }
        assert_eq!(pc, pd);
    }
}
