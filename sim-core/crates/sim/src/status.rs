//! Status-effect system (`docs/05 §2.5`): Poison (DoT), Frost (slow), Fire
//! (damage vulnerability), and Stun (immobilize). Deterministic — integer/Fixed
//! only, stable iteration order, no RNG. Applied on hit by combat, ticked once
//! per sim tick by `tick`, and consulted by combat for damage/movement.

use crate::content::{self, StatusOnHit, FROST_MAX_STACKS};
use crate::state::*;
use determinism::Fixed;

/// Frost duration applied per application (ticks). Stacks clear on expiry.
const FROST_DURATION: u32 = 150; // 5 s @ 30 Hz

/// Freeze duration when an enemy reaches `FROST_MAX_STACKS` (the source's Deep
/// Freeze: "1.5 seconds of Freeze when an enemy reaches 25 stacks of Frost,
/// resetting stacks to 0", `docs/appendix-A §A.2`). 1.5 s @ 30 Hz.
const FREEZE_DURATION: u32 = 45;

/// Radius of a Fire death-explosion. The source says "surrounding enemies"
/// without a fixed number; we use the catalog's common splash radius (300,
/// `docs/appendix-A §A.2`).
const FIRE_EXPLOSION_RADIUS: i64 = 300;

/// Fire death-explosion damage: "1 damage per 5 Stacks" (`docs/appendix-A
/// §A.2`) ⇒ `floor(fire_stacks / 5)`.
const FIRE_STACKS_PER_DAMAGE: u16 = 5;

/// Apply a weapon's on-hit status to an enemy. Poison refreshes to the stronger
/// DoT; frost/fire add stacks (frost capped); stun takes the longer remaining.
pub(crate) fn apply_on_hit(enemy: &mut Enemy, on_hit: &StatusOnHit) {
    let st = &mut enemy.status;
    if on_hit.poison_dps > 0 && on_hit.poison_ticks > 0 {
        // Keep whichever poison deals more total remaining damage.
        let existing = st.poison_dps.saturating_mul(st.poison_ticks as i64);
        let incoming = on_hit.poison_dps.saturating_mul(on_hit.poison_ticks as i64);
        if incoming >= existing {
            st.poison_dps = on_hit.poison_dps;
            st.poison_ticks = on_hit.poison_ticks;
        }
    }
    if on_hit.frost_stacks > 0 {
        st.frost_stacks = st.frost_stacks.saturating_add(on_hit.frost_stacks);
        st.frost_ticks = FROST_DURATION;
        // Freeze payoff: reaching the cap immobilizes the enemy for a short
        // window and resets the stacks (`docs/appendix-A §A.2`). While frozen it
        // takes +50% damage (see `vulnerability_mult`).
        if st.frost_stacks >= FROST_MAX_STACKS {
            st.frost_stacks = 0;
            st.frost_ticks = 0;
            st.freeze_ticks = st.freeze_ticks.max(FREEZE_DURATION);
        } else {
            st.frost_stacks = st.frost_stacks.min(FROST_MAX_STACKS);
        }
    }
    if on_hit.fire_stacks > 0 {
        st.fire_stacks = st.fire_stacks.saturating_add(on_hit.fire_stacks);
    }
    if on_hit.stun_ticks > 0 {
        st.stun_ticks = st.stun_ticks.max(on_hit.stun_ticks);
    }
}

/// Damage-taken multiplier from status: Fire (+0.5% per stack), generic
/// Vulnerability stacks (+1% per stack, from Vulnerability-Pulse auras), and the
/// Freeze payoff (+50% while frozen, `docs/appendix-A §A.2`).
pub(crate) fn vulnerability_mult(enemy: &Enemy) -> Fixed {
    let mut m = Fixed::ONE
        + Fixed::from_ratio(enemy.status.fire_stacks as i64 * 5, 1000)
        + Fixed::from_ratio(enemy.status.vuln_stacks as i64, 100);
    if enemy.status.freeze_ticks > 0 {
        m += Fixed::from_ratio(1, 2); // +50% damage taken while frozen
    }
    m
}

/// Phase: Vulnerability-Pulse auras. For each active pulse whose interval has
/// elapsed, add its magnitude in vuln stacks to every enemy within range.
/// Deterministic (fixed ticks, append-only pulses, stable enemy order).
pub(crate) fn pulse(s: &mut ArenaState) {
    if s.vuln_pulses.is_empty() {
        return;
    }
    let tank_pos = s.tank.pos;
    let mut pulses = std::mem::take(&mut s.vuln_pulses);
    for p in pulses.iter_mut() {
        let range = Fixed::from_int(p.range);
        let r2 = range.mul(range);
        while s.tick >= p.next_tick {
            for e in s.enemies.iter_mut() {
                if tank_pos.dist_sq(e.pos) <= r2 {
                    e.status.vuln_stacks = e.status.vuln_stacks.saturating_add(p.magnitude);
                }
            }
            p.next_tick += p.interval_ticks;
        }
    }
    s.vuln_pulses = pulses;
}

/// Movement-speed multiplier from status (Frost slow: -2% per stack, floored).
pub(crate) fn move_speed_mult(enemy: &Enemy) -> Fixed {
    let slow = Fixed::from_ratio(enemy.status.frost_stacks as i64 * 2, 100);
    let m = Fixed::ONE - slow;
    // Floor at 10% so a fully-frosted enemy still crawls.
    let floor = Fixed::from_ratio(1, 10);
    if m < floor {
        floor
    } else {
        m
    }
}

/// Whether the enemy cannot move this tick (stunned or frozen).
pub(crate) fn is_immobile(enemy: &Enemy) -> bool {
    enemy.status.stun_ticks > 0 || enemy.status.freeze_ticks > 0
}

/// Reap every enemy at `hp <= 0`, pushing its `def` to `pending_kills` and
/// triggering its Fire death-explosion. A Fire-stacked enemy that dies deals
/// `floor(fire_stacks / 5)` damage to surrounding enemies within
/// `FIRE_EXPLOSION_RADIUS` (`docs/appendix-A §A.2`); that damage can chain
/// further deaths, which detonate in turn. Fully deterministic: dead enemies are
/// processed in id-sorted order, and the loop fixes a stable point. The shared
/// death-reaping chokepoint for every combat/status death site so Fire
/// explosions fire wherever an enemy dies. Bosses neither explode nor take
/// explosion damage (they are damaged only by `Clear`).
pub(crate) fn reap_dead(s: &mut ArenaState) {
    let radius = Fixed::from_int(FIRE_EXPLOSION_RADIUS);
    let radius_sq = radius.mul(radius);
    loop {
        // Collect dead enemies (id-sorted) so explosions resolve deterministically.
        let mut dead: Vec<usize> = (0..s.enemies.len())
            .filter(|&i| s.enemies[i].hp <= 0)
            .collect();
        if dead.is_empty() {
            return;
        }
        dead.sort_by_key(|&i| s.enemies[i].id.0);

        // For each dead enemy: record the kill and, if it carried Fire, splash
        // explosion damage onto living non-boss enemies in range.
        let mut explosion_damage: i64 = 0;
        for &di in &dead {
            let (def, fire_stacks, pos) = {
                let e = &s.enemies[di];
                (e.def, e.status.fire_stacks, e.pos)
            };
            s.pending_kills.push(def);
            // Mark reaped so it is not collected again next round.
            s.enemies[di].hp = i64::MIN;
            if fire_stacks == 0 || content::ENEMIES[def as usize].boss {
                continue;
            }
            let dmg = (fire_stacks / FIRE_STACKS_PER_DAMAGE) as i64;
            if dmg <= 0 {
                continue;
            }
            for (i, e) in s.enemies.iter_mut().enumerate() {
                if i == di || e.hp <= 0 || content::ENEMIES[e.def as usize].boss {
                    continue;
                }
                if pos.dist_sq(e.pos) <= radius_sq {
                    e.hp -= dmg;
                    explosion_damage += dmg;
                }
            }
        }
        // Remove the enemies reaped this pass; survivors keep id order.
        s.enemies.retain(|e| e.hp != i64::MIN);
        // Explosion damage is a player source (scoreboard / Bloodmoney).
        s.record_player_damage(explosion_damage);
        // Loop: chained deaths from this pass's explosions detonate next pass.
    }
}

/// Per-tick status processing: apply Poison DoT, decay timers, clear expired
/// Frost. Poison kills push their `def` to `pending_kills` (so they still award
/// bounty). Boss enemies are immune to Poison (only `Clear` hurts the boss).
pub(crate) fn tick(s: &mut ArenaState) {
    let mut survivors: Vec<Enemy> = Vec::with_capacity(s.enemies.len());
    let mut poison_hits: i64 = 0;
    let mut poison_damage: i64 = 0;
    for mut e in std::mem::take(&mut s.enemies) {
        let immune = content::ENEMIES[e.def as usize].boss;

        // Poison damage-over-time.
        if e.status.poison_ticks > 0 {
            if !immune {
                e.hp -= e.status.poison_dps;
                poison_hits += 1;
                poison_damage += e.status.poison_dps;
            }
            e.status.poison_ticks -= 1;
            if e.status.poison_ticks == 0 {
                e.status.poison_dps = 0;
            }
        }
        // Frost duration / expiry.
        if e.status.frost_ticks > 0 {
            e.status.frost_ticks -= 1;
            if e.status.frost_ticks == 0 {
                e.status.frost_stacks = 0;
            }
        }
        // Stun duration.
        if e.status.stun_ticks > 0 {
            e.status.stun_ticks -= 1;
        }
        // Freeze duration (clears cleanly; no residual effect).
        if e.status.freeze_ticks > 0 {
            e.status.freeze_ticks -= 1;
        }

        survivors.push(e);
    }
    s.enemies = survivors;
    // Reap poison kills (and their Fire death-explosions) in a stable pass.
    reap_dead(s);
    // Poison DoT counts toward the player's damage scoreboard / Bloodmoney.
    s.record_player_damage(poison_damage);
    // On-poison trigger: heal the tank per enemy that took poison this tick.
    if s.tank.heal_on_poison > 0 && poison_hits > 0 && !s.dead {
        s.tank.heal(poison_hits * s.tank.heal_on_poison);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::EntityId;

    fn enemy(def: u16, hp: i64) -> Enemy {
        Enemy::new(EntityId(1), def, hp, Vec2::ZERO)
    }

    fn arena_with(enemies: Vec<Enemy>) -> ArenaState {
        let mut s = ArenaState::new(1, 0);
        s.enemies = enemies;
        s
    }

    #[test]
    fn poison_application_keeps_stronger_dot() {
        let mut e = enemy(0, 1000);
        apply_on_hit(
            &mut e,
            &StatusOnHit {
                poison_dps: 20,
                poison_ticks: 90,
                ..StatusOnHit::NONE
            },
        );
        assert_eq!(e.status.poison_dps, 20);
        // Weaker incoming poison does not replace.
        apply_on_hit(
            &mut e,
            &StatusOnHit {
                poison_dps: 5,
                poison_ticks: 10,
                ..StatusOnHit::NONE
            },
        );
        assert_eq!(e.status.poison_dps, 20);
        assert_eq!(e.status.poison_ticks, 90);
    }

    #[test]
    fn poison_ticks_deal_damage_and_expire() {
        let mut s = arena_with(vec![{
            let mut e = enemy(0, 1000);
            e.status.poison_dps = 100;
            e.status.poison_ticks = 3;
            e
        }]);
        for _ in 0..3 {
            tick(&mut s);
        }
        assert_eq!(s.enemies[0].hp, 700, "3 ticks × 100 dps");
        assert_eq!(s.enemies[0].status.poison_ticks, 0);
        assert_eq!(
            s.enemies[0].status.poison_dps, 0,
            "poison cleared on expiry"
        );
    }

    #[test]
    fn poison_kill_records_bounty() {
        let mut s = arena_with(vec![{
            let mut e = enemy(0, 50);
            e.status.poison_dps = 100;
            e.status.poison_ticks = 5;
            e
        }]);
        tick(&mut s);
        assert!(s.enemies.is_empty(), "poison killed the enemy");
        assert_eq!(s.pending_kills, vec![0]);
    }

    #[test]
    fn frost_slows_below_cap_then_expires() {
        let mut e = enemy(0, 1000);
        // Stay strictly below the cap so no freeze triggers (24 stacks).
        apply_on_hit(
            &mut e,
            &StatusOnHit {
                frost_stacks: 24,
                ..StatusOnHit::NONE
            },
        );
        assert_eq!(
            e.status.frost_stacks, 24,
            "frost stacks accumulate below cap"
        );
        assert_eq!(e.status.freeze_ticks, 0, "no freeze below cap");
        // 24 stacks × 2% = 48% slow → ×0.52.
        assert_eq!(move_speed_mult(&e).scale_i64(1000), 520);

        let mut s = arena_with(vec![e]);
        s.enemies[0].status.frost_ticks = 1;
        tick(&mut s);
        assert_eq!(
            s.enemies[0].status.frost_stacks, 0,
            "stacks clear on expiry"
        );
    }

    #[test]
    fn frost_freezes_at_max_resetting_stacks_and_boosting_damage() {
        let mut e = enemy(0, 1000);
        // Reaching the cap (25) freezes: stacks reset, freeze timer set.
        apply_on_hit(
            &mut e,
            &StatusOnHit {
                frost_stacks: FROST_MAX_STACKS,
                ..StatusOnHit::NONE
            },
        );
        assert_eq!(e.status.frost_stacks, 0, "stacks reset on freeze");
        assert_eq!(e.status.frost_ticks, 0, "frost slow cleared on freeze");
        assert_eq!(
            e.status.freeze_ticks, FREEZE_DURATION,
            "freeze armed for 1.5 s"
        );
        assert!(is_immobile(&e), "frozen enemy is immobile");
        // Frozen enemy takes +50% damage.
        assert_eq!(
            vulnerability_mult(&e).scale_i64(1000),
            1500,
            "+50% while frozen"
        );

        // Overshooting the cap in one application still freezes once.
        let mut e2 = enemy(0, 1000);
        apply_on_hit(
            &mut e2,
            &StatusOnHit {
                frost_stacks: 30,
                ..StatusOnHit::NONE
            },
        );
        assert_eq!(e2.status.frost_stacks, 0);
        assert_eq!(e2.status.freeze_ticks, FREEZE_DURATION);

        // Freeze decays to nothing and restores normal damage taken.
        let mut s = arena_with(vec![e]);
        for _ in 0..FREEZE_DURATION {
            tick(&mut s);
        }
        assert_eq!(s.enemies[0].status.freeze_ticks, 0, "freeze wears off");
        assert!(!is_immobile(&s.enemies[0]));
        assert_eq!(vulnerability_mult(&s.enemies[0]), Fixed::ONE);
    }

    #[test]
    fn fire_stacked_enemy_explodes_on_death_damaging_neighbors() {
        // A 50-fire-stack enemy dies → explosion = floor(50/5) = 10 damage to
        // each neighbor within FIRE_EXPLOSION_RADIUS; a far enemy is untouched.
        let mut s = arena_with(vec![
            {
                let mut e = Enemy::new(EntityId(1), 0, -1, Vec2::ZERO); // already dead
                e.status.fire_stacks = 50;
                e
            },
            // Neighbor in range, low hp → the 10 explosion damage chains a kill.
            Enemy::new(
                EntityId(2),
                0,
                8,
                Vec2::new(Fixed::from_int(100), Fixed::ZERO),
            ),
            // Neighbor in range, high hp → survives, takes 10.
            Enemy::new(
                EntityId(3),
                0,
                1000,
                Vec2::new(Fixed::from_int(200), Fixed::ZERO),
            ),
            // Out of range (> 300) → untouched.
            Enemy::new(
                EntityId(4),
                0,
                1000,
                Vec2::new(Fixed::from_int(1000), Fixed::ZERO),
            ),
        ]);
        reap_dead(&mut s);
        // Dead source + chained neighbor are reaped; both defs recorded.
        assert_eq!(s.enemies.len(), 2, "two survivors remain");
        assert_eq!(
            s.pending_kills,
            vec![0, 0],
            "source + chained kill recorded"
        );
        let near = s.enemies.iter().find(|e| e.id == EntityId(3)).unwrap();
        assert_eq!(near.hp, 990, "in-range survivor took 10 explosion damage");
        let far = s.enemies.iter().find(|e| e.id == EntityId(4)).unwrap();
        assert_eq!(far.hp, 1000, "out-of-range enemy untouched");
        // Explosion damage (10 to the chained kill + 10 to the survivor) scores.
        assert_eq!(s.total_damage_dealt, 20);
    }

    #[test]
    fn no_fire_no_explosion() {
        // A plain (no-fire) death reaps without damaging neighbors.
        let mut s = arena_with(vec![
            Enemy::new(EntityId(1), 0, -1, Vec2::ZERO),
            Enemy::new(
                EntityId(2),
                0,
                100,
                Vec2::new(Fixed::from_int(50), Fixed::ZERO),
            ),
        ]);
        reap_dead(&mut s);
        assert_eq!(s.enemies.len(), 1);
        assert_eq!(s.enemies[0].hp, 100, "neighbor unharmed without Fire");
        assert_eq!(s.pending_kills, vec![0]);
    }

    #[test]
    fn fire_explosion_is_deterministic() {
        let build = || {
            arena_with(vec![
                {
                    let mut e = Enemy::new(EntityId(1), 0, -1, Vec2::ZERO);
                    e.status.fire_stacks = 100;
                    e
                },
                Enemy::new(
                    EntityId(2),
                    0,
                    5,
                    Vec2::new(Fixed::from_int(80), Fixed::ZERO),
                ),
                Enemy::new(
                    EntityId(3),
                    0,
                    5,
                    Vec2::new(Fixed::from_int(120), Fixed::ZERO),
                ),
            ])
        };
        let mut a = build();
        let mut b = build();
        reap_dead(&mut a);
        reap_dead(&mut b);
        assert_eq!(a.enemies, b.enemies);
        assert_eq!(a.pending_kills, b.pending_kills);
        assert_eq!(a.total_damage_dealt, b.total_damage_dealt);
    }

    #[test]
    fn fire_increases_vulnerability() {
        let mut e = enemy(0, 1000);
        apply_on_hit(
            &mut e,
            &StatusOnHit {
                fire_stacks: 10,
                ..StatusOnHit::NONE
            },
        );
        // 10 × 0.5% = +5% ⇒ ≈×1.05 (fixed-point floors deterministically).
        let v = vulnerability_mult(&e).scale_i64(1_000_000);
        assert!((1_049_000..=1_050_000).contains(&v), "got {v}");
        // No fire ⇒ exactly identity.
        assert_eq!(vulnerability_mult(&enemy(0, 1000)), Fixed::ONE);
    }

    #[test]
    fn stun_immobilizes_then_wears_off() {
        let mut e = enemy(0, 1000);
        apply_on_hit(
            &mut e,
            &StatusOnHit {
                stun_ticks: 2,
                ..StatusOnHit::NONE
            },
        );
        assert!(is_immobile(&e));
        let mut s = arena_with(vec![e]);
        tick(&mut s); // 2 → 1
        assert!(is_immobile(&s.enemies[0]));
        tick(&mut s); // 1 → 0
        assert!(!is_immobile(&s.enemies[0]));
    }

    #[test]
    fn vulnerability_pulse_stacks_and_raises_damage_taken() {
        let mut s = ArenaState::new(7, 0);
        let idx = content::MODIFIERS
            .iter()
            .position(|m| {
                m.effects
                    .iter()
                    .any(|e| matches!(e, content::ModEffect::GrantVulnPulse(..)))
            })
            .expect("a Vulnerability Pulse modifier exists") as u16;
        s.buy_modifier(idx);
        assert_eq!(s.vuln_pulses.len(), 1, "pulse registered");

        // An enemy in range (≤1200), and one far outside.
        s.enemies = vec![
            Enemy::new(
                EntityId(1),
                0,
                1_000_000,
                Vec2::new(Fixed::from_int(500), Fixed::ZERO),
            ),
            Enemy::new(
                EntityId(2),
                0,
                1_000_000,
                Vec2::new(Fixed::from_int(5000), Fixed::ZERO),
            ),
        ];
        assert_eq!(vulnerability_mult(&s.enemies[0]), Fixed::ONE);

        // First pulse at tick 30 adds 5 stacks to the near enemy only.
        s.tick = 30;
        pulse(&mut s);
        assert_eq!(s.enemies[0].status.vuln_stacks, 5);
        assert_eq!(s.enemies[1].status.vuln_stacks, 0, "far enemy unaffected");
        let v = vulnerability_mult(&s.enemies[0]).scale_i64(1000);
        assert!((1049..=1050).contains(&v), "≈+5% damage taken, got {v}");

        // Second pulse stacks further.
        s.tick = 60;
        pulse(&mut s);
        assert_eq!(s.enemies[0].status.vuln_stacks, 10);
    }

    #[test]
    fn heal_on_poison_heals_per_poisoned_enemy_capped() {
        let mut s = arena_with(vec![
            {
                let mut e = enemy(0, 1000);
                e.status.poison_dps = 10;
                e.status.poison_ticks = 5;
                e
            },
            {
                let mut e = enemy(0, 1000);
                e.status.poison_dps = 10;
                e.status.poison_ticks = 5;
                e
            },
        ]);
        s.tank.max_hp = 1000;
        s.tank.hp = 500;
        s.tank.heal_on_poison = 5;
        tick(&mut s);
        // 2 enemies took poison ⇒ +10 HP.
        assert_eq!(s.tank.hp, 510);

        // Cap at max_hp.
        s.tank.hp = 998;
        tick(&mut s);
        assert_eq!(s.tank.hp, 1000);
    }

    #[test]
    fn poison_damage_accumulates_on_scoreboard() {
        let mut s = arena_with(vec![
            {
                let mut e = enemy(0, 1000);
                e.status.poison_dps = 30;
                e.status.poison_ticks = 5;
                e
            },
            {
                let mut e = enemy(0, 1000);
                e.status.poison_dps = 30;
                e.status.poison_ticks = 5;
                e
            },
        ]);
        s.economy.gold_per_damage = Fixed::from_ratio(1, 4); // exactly representable
        let gold0 = s.economy.gold;
        tick(&mut s);
        // 2 enemies × 30 dps = 60 poison damage this tick.
        assert_eq!(s.total_damage_dealt, 60);
        assert_eq!(s.economy.gold, gold0 + 15, "60 × 1/4 = 15 gold");
    }

    #[test]
    fn boss_is_immune_to_poison() {
        let mut s = arena_with(vec![{
            let mut e = enemy(content::BOSS, 1000);
            e.status.poison_dps = 100;
            e.status.poison_ticks = 5;
            e
        }]);
        tick(&mut s);
        assert_eq!(s.enemies[0].hp, 1000, "boss takes no poison damage");
    }
}
