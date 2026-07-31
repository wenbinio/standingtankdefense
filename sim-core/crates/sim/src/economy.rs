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

/// Per-tick ceiling on income-as-HP-regen, in **pools** (multiples of the tank's
/// live `max_hp`) once the ceiling has fully ramped in. See
/// [`income_regen_tick_cap`] for the ramp and for why a *constant* fraction of
/// `max_hp` provably cannot work here.
pub(crate) const INCOME_REGEN_CAP_POOLS: i64 = 5;
/// Exponent of the ramp. Four is the SMALLEST integer power that admits any
/// working constant at all — see [`income_regen_tick_cap`].
pub(crate) const INCOME_REGEN_CAP_POW: u32 = 4;

/// Per-tick ceiling on the income-as-HP-regen heal:
///
/// ```text
///     cap(max_hp, t) = INCOME_REGEN_CAP_POOLS × max_hp × (t / BOSS_SPAWN_TICK)^4
/// ```
///
/// evaluated as four truncating integer steps (below). It reaches its full value of
/// `POOLS × max_hp` per tick exactly when the boss arrives, and is a rounding error
/// in the opening minutes.
///
/// ## Why the ceiling exists
///
/// `income_regen_pct` (the source's "% of Gold Income as instant HP Regen",
/// `Entangled Gold Mine`) stacks additively per copy with no cap and is an ECONOMY
/// modifier, so the heal is `pct × copies × income` — an all-economy build maximises
/// *both* factors and the product runs away. Measured with no ceiling: a weaponless
/// eco tank regenerates 32,760 HP/tick into a 24,000 HP pool by tick 1200 and
/// 488,560/tick by tick 3600, against a contact leak of ~10 HP/tick. It cannot die,
/// and the `naked_eco_rush_*` balance guards measure that shop draw instead of early
/// wave pressure. Re-rating the item in `content.rs` cannot separate the cases: the
/// same modifier is load-bearing for every other build (`docs/11 §11.5`).
///
/// ## Why the ceiling RAMPS instead of being a flat fraction of `max_hp`
///
/// A flat fraction was measured across twelve values and there is **no** working
/// one — the window is empty from both ends, on identical 24,000 HP pools:
///
/// * the naked eco-rush leaks ~12–20 HP/tick in its first two minutes, so the guard
///   only goes green once the ceiling is under ~6 HP/tick (`max_hp/4000` ⇒ 97.9% of
///   seeds punished, `max_hp/6000` ⇒ 100%; `max_hp/2000` ⇒ 54.2%);
/// * a real build's heal closes a median deficit of **3,387 HP/tick** (p90 12,097,
///   p99 43,030, max 123,424) on the *same* 24,000 pool, and on 992 of the sampled
///   ticks it is restoring from **negative** HP — absorbing up to 4.1 pools of
///   single-tick overkill before `resolve_deaths` runs.
///
/// The two demands sit ~280× apart at equal `max_hp`, so no multiple of `max_hp`
/// separates them. Measured: at `max_hp/100` per tick (the "obviously too loose" 1% of
/// the pool) the win rate has already collapsed to 2.5% while the guard has not moved
/// off its 50% baseline; and even the loosest non-trivial flat ceiling — a whole pool
/// per tick — costs 20 points of win rate (33.8% → 13.8%), because what real builds
/// draw on is not the RATE but the ability to undo a single tick of overkill several
/// pools deep.
///
/// What *does* separate them is **when** the heal is needed: the naked eco-rush
/// needs it inside the first 3,600 ticks, every real build needs it after ~15,000.
/// A ceiling of `POOLS × max_hp × (t/T)^k` spans a ratio of `(3600/54000)^k`, and
/// closing the required ~8,000× gap over that 15× span needs `k ≥ log(8000)/log(15)
/// = 3.32`. **Four is therefore the smallest integer exponent that works at all**,
/// and it leaves a real window rather than a knife edge: `POOLS` of 3, 5 and 8 were all
/// measured fully in band and identical to each other; 2 sits exactly on the win-rate
/// floor (25.0%) and 1 falls out of it (16.3%).
///
/// ## Exact arithmetic (for the Luau port — transcribe literally)
///
/// ```text
///     c = max_hp * INCOME_REGEN_CAP_POOLS          -- POOLS = 5
///     repeat INCOME_REGEN_CAP_POW (= 4) times:
///         c = floor(c * tick / BOSS_SPAWN_TICK)    -- BOSS_SPAWN_TICK = 54000
///     cap = c
/// ```
///
/// Every step truncates toward zero (all operands are non-negative, so this is
/// `floor`); the truncation is part of the definition and must be reproduced step by
/// step, not folded into one division. `tick` is the arena tick.
///
/// Overflow / parity: the largest intermediate is `c × tick` on the first step,
/// `POOLS × max_hp × tick`. `max_hp` is bounded by `modifiers::STAT_CEIL` (1e12) so
/// this cannot approach i64 overflow; it stays exact in a Luau double (< 2^53) for
/// any `max_hp` below ~2.8e10, five orders above the measured worst run (448k).
pub(crate) fn income_regen_tick_cap(max_hp: i64, tick: u32) -> i64 {
    let mut c = max_hp * INCOME_REGEN_CAP_POOLS;
    for _ in 0..INCOME_REGEN_CAP_POW {
        c = c * (tick as i64) / content::BOSS_SPAWN_TICK as i64;
    }
    c
}

/// Phase 8: add `floor(income_per_tick × income_mult)` to gold. `bounty_mult`
/// still never touches income (source rule); `income_mult` is its own lever.
/// If `income_regen_pct > 0`, also heal the tank that fraction of the award,
/// **bounded per tick by [`income_regen_tick_cap`]** (`docs/11 §11.3` #5: bind the
/// tail, stay invisible at p90). The bound is applied to the *input* of
/// `Tank::heal`, so `+% Healing` still scales it and the pool clamp still applies.
pub(crate) fn tick_income(s: &mut ArenaState) {
    let amount = s.economy.income_mult.scale_i64(s.economy.income_per_tick);
    s.award_gold(amount);
    // Income-as-HP-regen (the source's "% of Gold Income as instant HP Regen").
    if s.economy.income_regen_pct > Fixed::ZERO && !s.dead {
        let raw = s.economy.income_regen_pct.scale_i64(amount);
        let cap = income_regen_tick_cap(s.tank.max_hp, s.tick);
        s.tank.heal(raw.min(cap));
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
        // Late enough that the per-tick ceiling has fully ramped in (5 × max_hp),
        // so this test measures the RATIO, not the ceiling.
        s.tick = content::BOSS_SPAWN_TICK;
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
    fn income_regen_tick_cap_ramps_from_nothing_to_pools_at_the_boss() {
        let max_hp = 24_000i64;
        let boss = content::BOSS_SPAWN_TICK;
        // Fully ramped in exactly at the boss spawn.
        assert_eq!(
            income_regen_tick_cap(max_hp, boss),
            INCOME_REGEN_CAP_POOLS * max_hp
        );
        // A rounding error over the `naked_eco_rush` deadline (tick 3600): the whole
        // point — the opening cannot be carried by income-as-HP-regen.
        let early = income_regen_tick_cap(max_hp, 3600);
        assert!(
            (1..=8).contains(&early),
            "cap at the eco-rush deadline must be single-digit HP/tick, was {early}"
        );
        // Zero at tick 0, and monotonically non-decreasing in the tick.
        assert_eq!(income_regen_tick_cap(max_hp, 0), 0);
        let mut prev = 0;
        for t in (0..=boss).step_by(500) {
            let c = income_regen_tick_cap(max_hp, t);
            assert!(c >= prev, "cap must not decrease (tick {t})");
            prev = c;
        }
        // Proportional to the pool: buying Max HP buys sustain headroom.
        assert_eq!(
            income_regen_tick_cap(2 * max_hp, boss),
            2 * income_regen_tick_cap(max_hp, boss)
        );
        // Pure integer truncation, no float: identical on every call.
        assert_eq!(
            income_regen_tick_cap(max_hp, 12_345),
            income_regen_tick_cap(max_hp, 12_345)
        );
    }

    #[test]
    fn income_regen_is_bounded_early_even_with_absurd_income() {
        // The degenerate case the ceiling exists for: a huge stacked ratio and a
        // huge income in the opening minutes heals almost nothing.
        let mut s = fresh();
        s.tank.max_hp = 24_000;
        s.tank.hp = 1_000;
        s.tick = 1_200;
        s.economy.income_per_tick = 1_000_000;
        s.economy.income_regen_pct = Fixed::from_int(5); // 500% of income
        tick_income(&mut s);
        assert!(
            s.tank.hp - 1_000 <= income_regen_tick_cap(24_000, 1_200),
            "early heal must be bounded by the ramped ceiling"
        );
        assert!(s.tank.hp < 1_010, "24k pool, tick 1200 ⇒ single-digit heal");
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
