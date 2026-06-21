//! Status-effect system (`docs/05 §2.5`): Poison (DoT), Frost (slow), Fire
//! (damage vulnerability), and Stun (immobilize). Deterministic — integer/Fixed
//! only, stable iteration order, no RNG. Applied on hit by combat, ticked once
//! per sim tick by `tick`, and consulted by combat for damage/movement.

use crate::content::{self, StatusOnHit, FROST_MAX_STACKS};
use crate::state::*;
use determinism::Fixed;

/// Frost duration applied per application (ticks). Stacks clear on expiry.
const FROST_DURATION: u32 = 150; // 5 s @ 30 Hz

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
        st.frost_stacks = (st.frost_stacks.saturating_add(on_hit.frost_stacks)).min(FROST_MAX_STACKS);
        st.frost_ticks = FROST_DURATION;
    }
    if on_hit.fire_stacks > 0 {
        st.fire_stacks = st.fire_stacks.saturating_add(on_hit.fire_stacks);
    }
    if on_hit.stun_ticks > 0 {
        st.stun_ticks = st.stun_ticks.max(on_hit.stun_ticks);
    }
}

/// Damage-taken multiplier from status: Fire (+0.5% per stack) and generic
/// Vulnerability stacks (+1% per stack, from Vulnerability-Pulse auras).
pub(crate) fn vulnerability_mult(enemy: &Enemy) -> Fixed {
    Fixed::ONE
        + Fixed::from_ratio(enemy.status.fire_stacks as i64 * 5, 1000)
        + Fixed::from_ratio(enemy.status.vuln_stacks as i64, 100)
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

/// Whether the enemy cannot move this tick (stunned).
pub(crate) fn is_immobile(enemy: &Enemy) -> bool {
    enemy.status.stun_ticks > 0
}

/// Per-tick status processing: apply Poison DoT, decay timers, clear expired
/// Frost. Poison kills push their `def` to `pending_kills` (so they still award
/// bounty). Boss enemies are immune to Poison (only `Clear` hurts the boss).
pub(crate) fn tick(s: &mut ArenaState) {
    let mut survivors: Vec<Enemy> = Vec::with_capacity(s.enemies.len());
    let mut poison_hits: i64 = 0;
    for mut e in std::mem::take(&mut s.enemies) {
        let immune = content::ENEMIES[e.def as usize].boss;

        // Poison damage-over-time.
        if e.status.poison_ticks > 0 {
            if !immune {
                e.hp -= e.status.poison_dps;
                poison_hits += 1;
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

        if e.hp <= 0 {
            s.pending_kills.push(e.def);
        } else {
            survivors.push(e);
        }
    }
    s.enemies = survivors;
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
        apply_on_hit(&mut e, &StatusOnHit { poison_dps: 20, poison_ticks: 90, ..StatusOnHit::NONE });
        assert_eq!(e.status.poison_dps, 20);
        // Weaker incoming poison does not replace.
        apply_on_hit(&mut e, &StatusOnHit { poison_dps: 5, poison_ticks: 10, ..StatusOnHit::NONE });
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
        assert_eq!(s.enemies[0].status.poison_dps, 0, "poison cleared on expiry");
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
    fn frost_caps_and_slows_then_expires() {
        let mut e = enemy(0, 1000);
        apply_on_hit(&mut e, &StatusOnHit { frost_stacks: 30, ..StatusOnHit::NONE });
        assert_eq!(e.status.frost_stacks, FROST_MAX_STACKS, "frost capped at 25");
        // 25 stacks × 2% = 50% slow → ×0.5.
        assert_eq!(move_speed_mult(&e).scale_i64(1000), 500);

        let mut s = arena_with(vec![e]);
        s.enemies[0].status.frost_ticks = 1;
        tick(&mut s);
        assert_eq!(s.enemies[0].status.frost_stacks, 0, "stacks clear on expiry");
    }

    #[test]
    fn fire_increases_vulnerability() {
        let mut e = enemy(0, 1000);
        apply_on_hit(&mut e, &StatusOnHit { fire_stacks: 10, ..StatusOnHit::NONE });
        // 10 × 0.5% = +5% ⇒ ≈×1.05 (fixed-point floors deterministically).
        let v = vulnerability_mult(&e).scale_i64(1_000_000);
        assert!((1_049_000..=1_050_000).contains(&v), "got {v}");
        // No fire ⇒ exactly identity.
        assert_eq!(vulnerability_mult(&enemy(0, 1000)), Fixed::ONE);
    }

    #[test]
    fn stun_immobilizes_then_wears_off() {
        let mut e = enemy(0, 1000);
        apply_on_hit(&mut e, &StatusOnHit { stun_ticks: 2, ..StatusOnHit::NONE });
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
            .position(|m| matches!(m.effect, content::ModEffect::GrantVulnPulse(..)))
            .expect("a Vulnerability Pulse modifier exists") as u16;
        s.buy_modifier(idx);
        assert_eq!(s.vuln_pulses.len(), 1, "pulse registered");

        // An enemy in range (≤1200), and one far outside.
        s.enemies = vec![
            Enemy::new(EntityId(1), 0, 1_000_000, Vec2::new(Fixed::from_int(500), Fixed::ZERO)),
            Enemy::new(EntityId(2), 0, 1_000_000, Vec2::new(Fixed::from_int(5000), Fixed::ZERO)),
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
    fn boss_is_immune_to_poison() {
        let mut s = arena_with(vec![{
            let mut e = enemy(content::SAMWISE, 1000);
            e.status.poison_dps = 100;
            e.status.poison_ticks = 5;
            e
        }]);
        tick(&mut s);
        assert_eq!(s.enemies[0].hp, 1000, "boss takes no poison damage");
    }
}
