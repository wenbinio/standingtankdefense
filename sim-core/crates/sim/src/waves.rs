//! Wave spawning — AGENT B. Deterministic; randomness only from `s.rng_spawn`.
use crate::content;
use crate::state::*;

/// Phase 3: for each `content::WAVE_M0` entry, if `s.tick % cadence_ticks == 0`,
/// spawn one enemy of that def at a `content::SPAWN_RING` position chosen via
/// `s.rng_spawn.below(SPAWN_RING.len())`. Allocate ids with
/// `s.alloc_entity_id()`; push to `s.enemies` (keep id order). Set hp from
/// `EnemyDef::base_hp`.
pub(crate) fn spawn(s: &mut ArenaState) {
    // Boss arrival (15:00, the source arc): spawn The Hippocrate exactly once at
    // the boss tick. The boss uses its FIXED HP (no scaling — source changelog)
    // and is immune to weapon fire — only `Clear` damages it (handled in combat).
    // Normal waves do NOT stop: the regular schedule keeps spawning through the
    // boss fight and rides the post-15:00 SWIFT-END escalation (`enemy_hp_mult`).
    // That continuing, hard-scaling tide replaced the old bespoke escort swarm —
    // it pressures even a heavily-defended tank, forces the player to keep
    // Clearing (every Clear also chips the Clear-only boss), and guarantees a
    // stalled match ends ("to bring the game to a swift end", docs/01).
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
        // fall through: the normal schedule below also spawns on this tick.
    }

    // Enemy HP scales with match time (`enemy_hp_mult` — the source-shaped curve).
    let hp_mult = content::enemy_hp_mult_at(s.tick, s.difficulty);
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
    fn hp_curve_is_the_source_arc() {
        // SOURCE-ARC RESTORATION (docs/01 §1.2): the curve is a smooth per-minute
        // base ramp, a +20% step AT 10:00 (`SCALE_STEP_TICK`), and from 15:00
        // (the boss tick) a "swift end" that compounds ×2 per minute on top of
        // the frozen 15:00 value. Pins the shape and the tuned dial.
        use determinism::Fixed;
        let m = content::enemy_hp_mult;
        let boss = content::BOSS_SPAWN_TICK; // 27000
        assert_eq!(boss, 27000);
        assert_eq!(content::SCALE_STEP_TICK, 18000);

        // (1) Starts at exactly ×1 and is monotonic non-decreasing across the
        //     whole curve, through the swift end (no downward steps anywhere).
        assert_eq!(m(0), Fixed::ONE);
        let mut prev = Fixed::ONE;
        let mut t = 0u32;
        while t <= boss + 5 * 1800 {
            let cur = m(t);
            assert!(cur >= prev, "curve dipped at tick {t}");
            prev = cur;
            t += 30; // sample once per second — fast and dense enough
        }

        // (2) Exact multipliers at the key arc times (×10000) — the tuned
        //     `RAMP_BASE` dial (5/4 = +25%/min after the 2-min opening grace),
        //     the +20% step at 10:00, and the ×1.5/min swift end. Pinned so a
        //     retune must be deliberate.
        let pins: [(u32, i64); 9] = [
            (0, 10000),             //  0:00 — ×1
            (2 * 1800, 10000),      //  2:00 — still ×1 (the opening grace)
            (5 * 1800, 19531),      //  5:00 — base ramp (≈ ×1.95)
            (18000 - 1, 59597),     // 10:00⁻ — just before the step (≈ ×5.96)
            (18000, 71525),         // 10:00 — the +20% step lands (≈ ×7.15)
            (14 * 1800, 174622),    // 14:00 — ≈ ×17.5
            (27000, 218277),        // 15:00 — the boss tick / swift-end knee (≈ ×21.8)
            (27000 + 1800, 327416), // 16:00 — swift end: ×1.5 past the knee (≈ ×32.7)
            (27000 + 5400, 736686), // 18:00 — ×1.5³ past the knee (≈ ×73.7)
        ];
        for (tick, mult10k) in pins {
            assert_eq!(m(tick).scale_i64(10000), mult10k, "curve pin at {tick}");
        }

        // (3) The 10:00 step is exactly +20% (instantaneous).
        assert_eq!(
            m(18000).scale_i64(100),
            m(18000 - 1).mul(Fixed::from_ratio(6, 5)).scale_i64(100)
        );

        // (4) Swift end: each minute past the boss tick multiplies by ×1.5
        //     (SWIFT_END = 3/2) — a stalled match ends.
        for k in 1..=4u32 {
            assert_eq!(
                m(boss + k * 1800).scale_i64(100),
                m(boss + (k - 1) * 1800)
                    .mul(Fixed::from_ratio(3, 2))
                    .scale_i64(100),
                "swift end must compound ×1.5 each minute (minute {k})"
            );
        }
        // …and is CONTINUOUS at the knee (no step at 15:00 — the slope explodes,
        // the value does not jump).
        assert!(
            m(boss).scale_i64(10000) - m(boss - 30).scale_i64(10000)
                < m(boss).scale_i64(10000) / 20,
            "no instantaneous jump at the boss tick"
        );
    }

    #[test]
    fn difficulty_scales_the_base_ramp_only() {
        // SP DIFFICULTY PRESETS: Easy/Hard swap only the per-minute BASE ramp
        // (Easy 97/80 · Normal 5/4 · Hard 103/80 — ×0.85 / ×1.15 on Normal's
        // +25%/min increment); the grace window, the 10:00 step and the swift
        // end are shared. Normal must stay bit-identical to `enemy_hp_mult`.
        use crate::content::{enemy_hp_mult, enemy_hp_mult_at, DIFF_EASY, DIFF_HARD, DIFF_NORMAL};
        let pow = |num: i64, den: i64, k: u32| -> Fixed {
            let f = Fixed::from_ratio(num, den);
            let mut b = Fixed::ONE;
            for _ in 0..k {
                b = b.mul(f);
            }
            b
        };

        // (1) Normal == the calibrated competitive curve, every sampled tick.
        let mut t = 0u32;
        while t <= content::BOSS_SPAWN_TICK + 4 * 1800 {
            assert_eq!(enemy_hp_mult_at(t, DIFF_NORMAL), enemy_hp_mult(t));
            t += 30;
        }

        // (2) Opening grace: ×1 on all three through 2:00.
        for d in [DIFF_EASY, DIFF_NORMAL, DIFF_HARD] {
            assert_eq!(enemy_hp_mult_at(2 * 1800, d), Fixed::ONE);
        }

        // (3) Whole-minute pins: at 5:00 (3 compounded minutes past the grace)
        // each preset is exactly its ramp rate cubed — and strictly ordered.
        let e5 = enemy_hp_mult_at(5 * 1800, DIFF_EASY);
        let n5 = enemy_hp_mult_at(5 * 1800, DIFF_NORMAL);
        let h5 = enemy_hp_mult_at(5 * 1800, DIFF_HARD);
        assert_eq!(e5, pow(97, 80, 3));
        assert_eq!(n5, pow(5, 4, 3));
        assert_eq!(h5, pow(103, 80, 3));
        assert!(e5 < n5 && n5 < h5);

        // (4) The +20% step and the ×1.5/min swift end are difficulty-blind:
        // the same multiplicative jumps land on every preset.
        for d in [DIFF_EASY, DIFF_HARD] {
            // ×100 granularity, like the Normal-curve pin: one tick of
            // piecewise-linear drift sits below it, the +20% jump far above.
            assert_eq!(
                enemy_hp_mult_at(content::SCALE_STEP_TICK, d).scale_i64(100),
                enemy_hp_mult_at(content::SCALE_STEP_TICK - 1, d)
                    .mul(Fixed::from_ratio(6, 5))
                    .scale_i64(100),
                "10:00 step must be exactly +20% on preset {d}"
            );
            let boss = content::BOSS_SPAWN_TICK;
            assert_eq!(
                enemy_hp_mult_at(boss + 1800, d).scale_i64(100),
                enemy_hp_mult_at(boss, d)
                    .mul(Fixed::from_ratio(3, 2))
                    .scale_i64(100),
                "swift end must compound ×1.5/min on preset {d}"
            );
        }

        // (5) Constructor plumbing: the preset lands on the arena (invalid
        // codes clamp to Normal) and spawns actually ride it — the same tick's
        // spawn has less HP on Easy than on Hard.
        assert_eq!(ArenaState::new(7, 0).difficulty, DIFF_NORMAL);
        assert_eq!(
            ArenaState::new_with_difficulty(7, 0, 9).difficulty,
            DIFF_NORMAL
        );
        let spawn_hp = |d: u8| -> i64 {
            let mut s = ArenaState::new_with_difficulty(7, 0, d);
            s.tick = 14 * 1800; // late pre-boss, deep into the ramp
            s.tick -= s.tick % content::EARLY_GRUNT_CADENCE; // grunt stream fires
            spawn(&mut s);
            s.enemies[0].hp
        };
        assert!(spawn_hp(DIFF_EASY) < spawn_hp(DIFF_NORMAL));
        assert!(spawn_hp(DIFF_NORMAL) < spawn_hp(DIFF_HARD));
    }

    #[test]
    fn late_game_enemies_spawn_with_scaled_hp() {
        // Use a grunt-cadence tick late in the curve (just before the boss tick),
        // where the multiplier is far above ×1. Grunt is wave entry 0 (catalog
        // order), so it is `enemies[0]` among this tick's spawns.
        let mut s = blank_state();
        // A grunt-cadence (`EARLY_GRUNT_CADENCE`) tick just before the boss.
        s.tick = content::BOSS_SPAWN_TICK - content::EARLY_GRUNT_CADENCE; // 26982
        assert_eq!(s.tick % content::EARLY_GRUNT_CADENCE, 0);
        spawn(&mut s);
        let grunt_base = content::ENEMIES[0].base_hp;
        assert_eq!(
            s.enemies[0].def, 0,
            "first spawn this tick is the grunt (entry 0)"
        );
        // hp should be scaled up hard (the curve is ≈ ×24.3 at 15:00⁻ on the
        // restored source arc — base ramp × the 10:00 step).
        assert!(
            s.enemies[0].hp > grunt_base * 20,
            "late enemy HP must be scaled up"
        );
        assert!(s.enemies[0].hp <= grunt_base * 30);
    }

    #[test]
    fn boss_spawns_once_and_normal_waves_continue() {
        // At the boss tick the boss spawns exactly once; the NORMAL schedule may
        // also spawn this tick (the boss tick is a multiple of several cadences).
        // Assert exactly ONE boss is present and that the boss is among the spawns.
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
        // The boss's HP is its FIXED base_hp — it never rides the curve.
        let b = s.enemies.iter().find(|e| e.def == content::BOSS).unwrap();
        assert_eq!(b.hp, content::ENEMIES[content::BOSS as usize].base_hp);

        // The boss spawns ONLY once: at a later boss-phase tick no second boss
        // appears, but the REGULAR waves keep coming (the swift-end tide — the
        // climax is a real fight, not a duel in an empty arena).
        s.enemies.clear();
        s.tick = content::BOSS_SPAWN_TICK + 8; // a pre-boss-flood cadence tick (8)
        spawn(&mut s);
        assert!(
            s.enemies
                .iter()
                .all(|e| !content::ENEMIES[e.def as usize].boss),
            "the boss spawns once, not again during the boss phase"
        );
        assert!(
            !s.enemies.is_empty(),
            "normal waves keep spawning through the boss fight"
        );
        // …and those boss-phase spawns ride the (swift-end) curve multiplier.
        let grunt = s.enemies.iter().find(|e| e.def == 0).unwrap();
        assert!(
            grunt.hp > content::ENEMIES[0].base_hp * 20,
            "boss-phase spawns must carry swift-end-scaled HP"
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
