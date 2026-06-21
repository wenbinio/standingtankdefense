//! Economy — AGENT C. Integer gold. Bounty scales by `s.economy.bounty_mult`;
//! passive income does NOT (source rule, `docs/02 §2.4`).
use crate::content;
use crate::state::*;

/// Called once when a new round begins (before input). M0: keep simple
/// (no-op, or a small flat per-round bonus). Deterministic.
pub(crate) fn on_round_start(s: &mut ArenaState) {
    // M0: no per-round bonus. Kept as a deterministic no-op.
    let _ = s;
}

/// Phase 7: drain `s.pending_kills`; for each enemy def add
/// `floor(EnemyDef::bounty * bounty_mult)` to gold (use
/// `s.economy.bounty_mult.scale_i64(bounty)`). Leave `pending_kills` empty.
pub(crate) fn collect_bounties(s: &mut ArenaState) {
    let mult = s.economy.bounty_mult;
    let mut gained: i64 = 0;
    let mut kills: i64 = 0;
    for def in s.pending_kills.drain(..) {
        let bounty = content::ENEMIES[def as usize].bounty;
        gained += mult.scale_i64(bounty);
        kills += 1;
    }
    s.economy.gold += gained;
    // On-kill trigger: heal the tank per enemy killed this tick, capped at max HP.
    if s.tank.heal_on_kill > 0 && kills > 0 && !s.dead {
        s.tank.hp = (s.tank.hp + kills * s.tank.heal_on_kill).min(s.tank.max_hp);
    }
}

/// Phase 8: add `s.economy.income_per_tick` to gold (no multiplier).
pub(crate) fn tick_income(s: &mut ArenaState) {
    s.economy.gold += s.economy.income_per_tick;
}

/// Phase 9: if `s.tank.hp <= 0` and not already dead, set `dead = true` and
/// `death_tick = Some(s.tick)`.
pub(crate) fn resolve_deaths(s: &mut ArenaState) {
    if s.tank.hp <= 0 && !s.dead {
        s.dead = true;
        s.death_tick = Some(s.tick);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use determinism::Fixed;

    fn fresh() -> ArenaState {
        ArenaState::new(0xABCD, 0)
    }

    #[test]
    fn tick_income_adds_with_no_multiplier() {
        let mut s = fresh();
        s.economy.gold = 0;
        s.economy.income_per_tick = 20;
        // Even with a non-unit bounty multiplier, passive income is unaffected.
        s.economy.bounty_mult = Fixed::from_int(10);
        tick_income(&mut s);
        assert_eq!(s.economy.gold, 20);
        tick_income(&mut s);
        assert_eq!(s.economy.gold, 40);
    }

    #[test]
    fn collect_bounties_drains_and_scales() {
        let mut s = fresh();
        s.economy.gold = 0;
        s.economy.bounty_mult = Fixed::ONE;
        // def 0 bounty = 10, def 1 bounty = 40.
        s.pending_kills = vec![0, 1, 0];
        collect_bounties(&mut s);
        assert_eq!(s.economy.gold, 10 + 40 + 10);
        assert!(s.pending_kills.is_empty(), "pending_kills must be drained");
    }

    #[test]
    fn collect_bounties_applies_multiplier_with_floor() {
        let mut s = fresh();
        s.economy.gold = 0;
        // 1.5x multiplier; def 1 bounty = 40 -> 60.
        s.economy.bounty_mult = Fixed::from_ratio(3, 2);
        s.pending_kills = vec![1];
        collect_bounties(&mut s);
        assert_eq!(s.economy.gold, 60);

        // Floor check: bounty 10 * 1.5 = 15 exactly; use 0.5x on bounty 10 -> 5.
        s.economy.gold = 0;
        s.economy.bounty_mult = Fixed::from_ratio(1, 2);
        s.pending_kills = vec![0];
        collect_bounties(&mut s);
        assert_eq!(s.economy.gold, 5);
    }

    #[test]
    fn collect_bounties_noop_when_empty() {
        let mut s = fresh();
        s.economy.gold = 123;
        s.pending_kills.clear();
        collect_bounties(&mut s);
        assert_eq!(s.economy.gold, 123);
    }

    #[test]
    fn heal_on_kill_heals_per_kill_capped_at_max_hp() {
        let mut s = fresh();
        s.tank.max_hp = 1000;
        s.tank.hp = 500;
        s.tank.heal_on_kill = 15;
        s.economy.bounty_mult = Fixed::ONE;
        // 3 kills → +45 HP.
        s.pending_kills = vec![0, 1, 0];
        collect_bounties(&mut s);
        assert_eq!(s.tank.hp, 545);

        // Cap at max_hp.
        s.tank.hp = 990;
        s.pending_kills = vec![0, 1, 0];
        collect_bounties(&mut s);
        assert_eq!(s.tank.hp, 1000, "heal cannot exceed max_hp");

        // No heal stat ⇒ HP unchanged.
        let mut s2 = fresh();
        s2.tank.max_hp = 1000;
        s2.tank.hp = 500;
        s2.pending_kills = vec![0];
        collect_bounties(&mut s2);
        assert_eq!(s2.tank.hp, 500);
    }

    #[test]
    fn resolve_deaths_flips_dead_at_zero_hp() {
        let mut s = fresh();
        s.tick = 42;
        s.tank.hp = 0;
        resolve_deaths(&mut s);
        assert!(s.dead);
        assert_eq!(s.death_tick, Some(42));
    }

    #[test]
    fn resolve_deaths_does_not_overwrite_existing_death() {
        let mut s = fresh();
        s.dead = true;
        s.death_tick = Some(7);
        s.tick = 99;
        s.tank.hp = -100;
        resolve_deaths(&mut s);
        assert_eq!(s.death_tick, Some(7), "first death tick must be preserved");
    }

    #[test]
    fn resolve_deaths_alive_when_hp_positive() {
        let mut s = fresh();
        s.tank.hp = 1;
        resolve_deaths(&mut s);
        assert!(!s.dead);
        assert_eq!(s.death_tick, None);
    }
}
