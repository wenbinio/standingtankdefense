//! Economy — AGENT C. Integer gold. Bounty scales by `s.economy.bounty_mult`;
//! passive income does NOT (source rule, `docs/02 §2.4`).
use crate::content;
use crate::state::*;
use determinism::Fixed;

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
    let chance = s.economy.bounty_proc_chance_pct;
    let bonus = s.economy.bounty_proc_bonus;
    let mut gained: i64 = 0;
    let mut kills: i64 = 0;
    // Take the kill list out so we can also borrow `rng_proc` for proc rolls.
    let kills_vec = std::mem::take(&mut s.pending_kills);
    for def in &kills_vec {
        let bounty = content::ENEMIES[*def as usize].bounty;
        let base = mult.scale_i64(bounty);
        gained += base;
        // Chance-based bonus bounty (the source's "+X% Bounty with Y% chance").
        // Only draw RNG when the player actually owns a proc, so the baseline
        // RNG cursor is untouched for everyone else.
        if chance > 0 && (s.rng_proc.below(100) as i64) < chance {
            gained += bonus.scale_i64(base);
        }
        kills += 1;
    }
    s.pending_kills = kills_vec;
    s.pending_kills.clear();
    s.award_gold(gained);
    // On-kill trigger: heal the tank per enemy killed this tick.
    if s.tank.heal_on_kill > 0 && kills > 0 && !s.dead {
        s.tank.heal(kills * s.tank.heal_on_kill);
    }
    // On-kill trigger: restore Mana Shield per enemy killed this tick (the source's
    // Maw of Death). Mirrors `heal_on_kill`: applies per kill, capped at the pool
    // max via `restore_mana` (a no-op if the tank owns no shield pool).
    if s.tank.mana_on_kill > 0 && kills > 0 && !s.dead {
        s.tank.restore_mana(kills * s.tank.mana_on_kill);
    }
}

/// Phase 8: add `floor(income_per_tick × income_mult)` to gold. `bounty_mult`
/// still never touches income (source rule); `income_mult` is its own lever.
/// If `income_regen_pct > 0`, also heal the tank that fraction of the award.
pub(crate) fn tick_income(s: &mut ArenaState) {
    let amount = s.economy.income_mult.scale_i64(s.economy.income_per_tick);
    s.award_gold(amount);
    // Income-as-HP-regen (the source's "% of Gold Income as instant HP Regen").
    if s.economy.income_regen_pct > Fixed::ZERO && !s.dead {
        s.tank.heal(s.economy.income_regen_pct.scale_i64(amount));
    }
    // Income-as-survival: route a fraction of the award into the Mana-Shield pool
    // (capped at its max), mirroring the HP-regen path above.
    if s.economy.income_shield_pct > Fixed::ZERO && !s.dead {
        let add = s.economy.income_shield_pct.scale_i64(amount);
        if add > 0 && s.tank.mana_shield < s.tank.mana_shield_max {
            s.tank.mana_shield = (s.tank.mana_shield + add).min(s.tank.mana_shield_max);
        }
    }
}

/// Phase 9: resolve a fatal hit. A pending revive (Ankh) is consumed first —
/// fully repairing the tank and granting `revive_bonus_hp` Max HP — so the tank
/// survives. Only when no revive remains does it die.
pub(crate) fn resolve_deaths(s: &mut ArenaState) {
    if s.tank.hp <= 0 && !s.dead {
        if s.tank.revives > 0 {
            s.tank.revives -= 1;
            s.tank.max_hp += s.tank.revive_bonus_hp;
            s.tank.hp = s.tank.max_hp; // full repair
            return;
        }
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
    fn income_mult_scales_passive_income_floored() {
        let mut s = fresh();
        s.economy.gold = 0;
        s.economy.income_per_tick = 20;
        s.economy.income_mult = Fixed::ONE; // identity baseline
        tick_income(&mut s);
        assert_eq!(s.economy.gold, 20);
        // +25% income ⇒ 25/tick.
        s.economy.income_mult = Fixed::ONE + Fixed::from_ratio(1, 4);
        tick_income(&mut s);
        assert_eq!(s.economy.gold, 45);
    }

    #[test]
    fn income_does_not_use_bounty_mult() {
        // Source rule: passive income is untouched by the bounty multiplier.
        let mut s = fresh();
        s.economy.gold = 0;
        s.economy.income_per_tick = 20;
        s.economy.bounty_mult = Fixed::from_int(10);
        tick_income(&mut s);
        assert_eq!(s.economy.gold, 20);
    }

    #[test]
    fn income_regen_heals_a_fraction_of_income_capped() {
        let mut s = fresh();
        s.economy.gold = 0;
        s.economy.income_per_tick = 100;
        s.tank.max_hp = 1000;
        s.tank.hp = 500;
        s.economy.income_regen_pct = Fixed::from_ratio(1, 4); // 25%
        tick_income(&mut s);
        assert_eq!(s.economy.gold, 100);
        assert_eq!(s.tank.hp, 525, "25% of 100 income healed");

        // Cap at max_hp.
        s.tank.hp = 990;
        tick_income(&mut s);
        assert_eq!(s.tank.hp, 1000);

        // Dead tank does not heal.
        s.dead = true;
        s.tank.hp = 0;
        tick_income(&mut s);
        assert_eq!(s.tank.hp, 0);
    }

    #[test]
    fn income_and_bounty_feed_gold_scoreboard() {
        let mut s = fresh();
        s.economy.gold = 0;
        s.total_gold_earned = 0;
        s.economy.income_per_tick = 20;
        s.economy.bounty_mult = Fixed::ONE;
        tick_income(&mut s);
        s.pending_kills = vec![1]; // bounty 40
        collect_bounties(&mut s);
        assert_eq!(s.economy.gold, 60);
        assert_eq!(s.total_gold_earned, 60, "income + bounty both scored");
    }

    #[test]
    fn income_shield_refills_mana_shield_capped() {
        let mut s = fresh();
        s.economy.gold = 0;
        s.economy.income_per_tick = 100;
        s.tank.mana_shield = 0;
        s.tank.mana_shield_max = 30;
        s.economy.income_shield_pct = Fixed::from_ratio(1, 4); // 25% of income → shield
        tick_income(&mut s);
        assert_eq!(s.economy.gold, 100);
        assert_eq!(s.tank.mana_shield, 25, "25% of 100 income into shield");
        // Capped at max.
        tick_income(&mut s);
        assert_eq!(s.tank.mana_shield, 30, "shield capped at max");
        // Dead tank gains no shield.
        s.dead = true;
        s.tank.mana_shield = 0;
        tick_income(&mut s);
        assert_eq!(s.tank.mana_shield, 0);
    }

    #[test]
    fn bounty_proc_pays_bonus_and_is_deterministic() {
        // A 100% chance proc always pays the bonus.
        let mut s = fresh();
        s.economy.gold = 0;
        s.economy.bounty_mult = Fixed::ONE;
        s.economy.bounty_proc_chance_pct = 100;
        s.economy.bounty_proc_bonus = Fixed::from_int(2); // +200%
        s.pending_kills = vec![1]; // bounty 40 → 40 + 80 = 120
        collect_bounties(&mut s);
        assert_eq!(s.economy.gold, 120);

        // No proc owned ⇒ no RNG draw, no bonus, cursor untouched.
        let mut a = fresh();
        let mut b = fresh();
        let cursor = a.rng_proc.state();
        a.economy.bounty_mult = Fixed::ONE;
        b.economy.bounty_mult = Fixed::ONE;
        a.pending_kills = vec![1];
        b.pending_kills = vec![1];
        collect_bounties(&mut a);
        assert_eq!(a.economy.gold, 540, "base bounty only");
        assert_eq!(a.rng_proc.state(), cursor, "no RNG drawn without a proc");

        // Same seed + same proc ⇒ identical outcome (determinism).
        let mut c = ArenaState::new(999, 0);
        let mut d = ArenaState::new(999, 0);
        for s in [&mut c, &mut d] {
            s.economy.gold = 0;
            s.economy.bounty_mult = Fixed::ONE;
            s.economy.bounty_proc_chance_pct = 50;
            s.economy.bounty_proc_bonus = Fixed::from_int(2);
            s.pending_kills = vec![1, 1, 1, 1, 1];
        }
        collect_bounties(&mut c);
        collect_bounties(&mut d);
        assert_eq!(c.economy.gold, d.economy.gold);
        assert_eq!(c.rng_proc.state(), d.rng_proc.state());
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
    fn mana_on_kill_restores_shield_per_kill_capped_at_max() {
        let mut s = fresh();
        s.tank.mana_shield_max = 1000;
        s.tank.mana_shield = 500;
        s.tank.mana_on_kill = 15;
        s.economy.bounty_mult = Fixed::ONE;
        // 3 kills → +45 shield.
        s.pending_kills = vec![0, 1, 0];
        collect_bounties(&mut s);
        assert_eq!(s.tank.mana_shield, 545, "shield restored per kill");

        // Cap at mana_shield_max.
        s.tank.mana_shield = 990;
        s.pending_kills = vec![0, 1, 0];
        collect_bounties(&mut s);
        assert_eq!(s.tank.mana_shield, 1000, "shield restore cannot exceed max");

        // No shield pool ⇒ restore is a no-op (mirrors restore_mana guard).
        let mut s2 = fresh();
        s2.tank.mana_shield_max = 0;
        s2.tank.mana_shield = 0;
        s2.tank.mana_on_kill = 15;
        s2.pending_kills = vec![0];
        collect_bounties(&mut s2);
        assert_eq!(s2.tank.mana_shield, 0, "no pool ⇒ mana-on-kill is a no-op");
    }

    #[test]
    fn revive_survives_fatal_hit_then_dies_without_one() {
        let mut s = fresh();
        s.tank.revives = 1;
        s.tank.revive_bonus_hp = 2000;
        let max0 = s.tank.max_hp;
        s.tick = 100;
        s.tank.hp = -50;
        resolve_deaths(&mut s);
        assert!(!s.dead, "revive prevented death");
        assert_eq!(s.tank.revives, 0, "revive consumed");
        assert_eq!(s.tank.max_hp, max0 + 2000, "gained bonus Max HP");
        assert_eq!(s.tank.hp, s.tank.max_hp, "fully repaired");

        // A second fatal hit with no revives left → death.
        s.tank.hp = 0;
        resolve_deaths(&mut s);
        assert!(s.dead);
        assert_eq!(s.death_tick, Some(100));
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
