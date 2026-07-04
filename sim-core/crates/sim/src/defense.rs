//! Tank defensive layer (`docs/05 §2.2`): Dodge → Armor → Mana Shield → HP, plus
//! per-tick Mana-Shield and HP regeneration. Deterministic — the only RNG is the
//! dodge roll from `rng_proc`.

use crate::content;
use crate::state::{ArenaState, SimEvent};
use determinism::Fixed;

/// Radius (from the tank) of Spikes retaliation.
const SPIKES_RANGE: i64 = 400;

/// Apply `raw` incoming damage to the tank through the defensive layers.
/// Returns whether the hit LANDED (`false` = dodged) so callers can run
/// landed-hit triggers (the first-hit bonus Spikes).
pub(crate) fn hit_tank(s: &mut ArenaState, raw: i64) -> bool {
    if raw <= 0 {
        return false;
    }
    // Dodge: chance to avoid the hit entirely (consumes a proc roll).
    if s.tank.dodge_den > 0 {
        let roll = s.rng_proc.below(s.tank.dodge_den);
        if roll < s.tank.dodge_num {
            return false;
        }
    }
    // The hit landed (even if fully absorbed by the shield) → Spikes will fire.
    s.tank_hit_this_tick = true;
    // Heal-on-damaged (Dreadlord Fang): one flat heal per LANDED hit, regardless
    // of how much is ultimately absorbed. Routes through `Tank::heal` so the
    // `healing_mult` and the max-HP cap apply uniformly.
    if s.tank.heal_on_damaged > 0 {
        s.tank.heal(s.tank.heal_on_damaged);
    }
    // Deflection (source: "+5% of Spikes Damage as Flat Damage Reduction, but
    // cannot reduce more than 50% of an attack"): flat DR equal to
    // `rate × current (multiplied) Spikes damage`, capped at half this hit.
    let mut effective = raw;
    if s.tank.spikes_dr_rate > Fixed::ZERO {
        let spikes_total = s.tank.spikes_mult.scale_i64(s.tank.spikes_damage);
        let dr = s
            .tank
            .spikes_dr_rate
            .scale_i64(spikes_total)
            .clamp(0, raw / 2);
        effective = raw - dr;
    }
    // Armor: flat reduction, but at least 1 damage always lands.
    let mut remaining = (effective - s.tank.armor).max(1);
    // Mana Shield absorbs before HP.
    if s.tank.mana_shield > 0 {
        // Energy Shield: while the shield is active (pool > 0 at hit time), ALL
        // incoming damage — what the shield absorbs AND what overflows to HP — is
        // reduced by `shield_active_dr`. The DR is clamped to `[0, ONE]` so the
        // `(1 - dr)` multiplier can never go negative; the min-1 guarantee is kept
        // so a hit is never reduced fully to zero. Once the shield hits 0 the
        // reduction stops (this whole branch is gated on `mana_shield > 0`).
        // Integer/Fixed only — feeds the checksum.
        let dr = s.tank.shield_active_dr.clamp(Fixed::ZERO, Fixed::ONE);
        remaining = (Fixed::ONE - dr).scale_i64(remaining).max(1);
        let absorbed = remaining.min(s.tank.mana_shield);
        s.tank.mana_shield -= absorbed;
        remaining -= absorbed;
        // Shield-break stun (source: Energy Pulse): detect the `>0 → 0` DOWN-EDGE
        // here, at the moment the shield absorbs the hit that depletes it. The pulse
        // itself (iterate enemies in range, stun them in stable id order) is deferred
        // to `shield_break_stun` so this borrow-only path stays free of the enemy
        // loop. Armed only when the pulse is configured (`range > 0`).
        if s.tank.mana_shield == 0 {
            // Render event: every down-edge announces (independent of whether
            // the Energy-Pulse stun is owned).
            s.emit(SimEvent::ShieldBroke);
            if s.tank.shieldbreak_stun_range > 0 {
                s.shield_broke_this_tick = true;
            }
        }
        // Render event: total applied to shield + HP (post-armor, post-DR).
        s.emit(SimEvent::TankHit {
            damage: absorbed + remaining,
        });
        // Damage taken this tick (shield + HP) — feeds the damage→Spikes
        // conversion, consumed by `spikes` the same tick.
        s.damage_taken_this_tick += absorbed + remaining;
    } else {
        s.emit(SimEvent::TankHit { damage: remaining });
        s.damage_taken_this_tick += remaining;
    }
    s.tank.hp -= remaining;
    true
}

/// Shield-break stun pulse (source: Energy Pulse). If the Mana Shield transitioned
/// `>0 → 0` from a hit this tick, stun every non-boss enemy within
/// `shieldbreak_stun_range` for `shieldbreak_stun_ticks`, then consume the flag.
/// Iterates enemies in STABLE id order (the live `enemies` vec is id-ordered) so
/// the result is run-to-run identical. Integer/Fixed only; no RNG. Reuses the
/// existing stun status (`stun_ticks`, takes the longer remaining).
pub(crate) fn shield_break_stun(s: &mut ArenaState) {
    let broke = s.shield_broke_this_tick;
    s.shield_broke_this_tick = false;
    if !broke {
        return;
    }
    let ticks = s.tank.shieldbreak_stun_ticks;
    if ticks == 0 {
        return;
    }
    let range = Fixed::from_int(s.tank.shieldbreak_stun_range);
    let r2 = range.mul(range);
    let tank_pos = s.tank.pos;
    // `s.enemies` is maintained in id order by every spawn/reap path, so this loop
    // is already stable-id-ordered; stun is order-independent regardless.
    for e in s.enemies.iter_mut() {
        if content::ENEMIES[e.def as usize].boss {
            continue;
        }
        if tank_pos.dist_sq(e.pos) <= r2 {
            e.status.stun_ticks = e.status.stun_ticks.max(ticks);
        }
    }
}

/// Spikes retaliation: if the tank was hit this tick, deal its (multiplied)
/// spikes damage to every non-boss enemy within `SPIKES_RANGE`. Consumes and
/// resets the hit flag. Kills push to `pending_kills` (so they award bounty).
pub(crate) fn spikes(s: &mut ArenaState) {
    let hit = s.tank_hit_this_tick;
    s.tank_hit_this_tick = false;
    // Consume the per-tick transients unconditionally so they are always 0 at
    // the tick boundary (they are only ever non-zero when `hit` is too).
    let first_bonus = std::mem::take(&mut s.spikes_first_bonus_this_tick);
    let dmg_taken = std::mem::take(&mut s.damage_taken_this_tick);
    if !hit {
        return;
    }
    // Stacking spikes (source: Bloody Spikes): each landed hit grows the stack
    // counter by 1 up to the cap; the bonus spikes damage is `per × stacks`. This
    // grows BEFORE the retaliation so the hit that triggered the stack benefits from
    // it (the source applies the new stack immediately). Integer only; resets at the
    // round boundary (see `economy`/round phase). A no-op when `spikes_stack_per == 0`.
    let mut stack_bonus: i64 = 0;
    if s.tank.spikes_stack_per > 0 {
        if s.tank.spikes_stacks < s.tank.spikes_stacks_max {
            s.tank.spikes_stacks += 1;
        }
        stack_bonus = s
            .tank
            .spikes_stack_per
            .saturating_mul(s.tank.spikes_stacks as i64);
    }
    // Spikes damage = (flat + stacking bonus + first-hit bonus + damage-taken
    // conversion) × multiplier. The first-hit bonus (source: "+240 Spikes on
    // the first attack") and the conversion (source: "+30% of Damage Taken
    // Spikes Damage") are flat riders on THIS retaliation, so they scale with
    // "+% Spikes" like every other flat spikes source.
    let converted = s.tank.dmg_taken_to_spikes.scale_i64(dmg_taken);
    let base = s
        .tank
        .spikes_damage
        .saturating_add(stack_bonus)
        .saturating_add(first_bonus)
        .saturating_add(converted);
    let dmg = s.tank.spikes_mult.scale_i64(base);
    // Retaliation STATUS (independent of spikes damage): Poison Armor's DoT
    // plus the Frost/Flaming Armor stacks — a tank with only a status armor
    // (zero Spikes damage) still retaliates with the status.
    let status = content::StatusOnHit {
        poison_dps: s.tank.spikes_poison_dps,
        poison_ticks: s.tank.spikes_poison_ticks,
        frost_stacks: s.tank.retaliate_frost,
        fire_stacks: s.tank.retaliate_fire,
        stun_ticks: 0,
    };
    let has_status = (status.poison_dps > 0 && status.poison_ticks > 0)
        || status.frost_stacks > 0
        || status.fire_stacks > 0;
    if dmg <= 0 && !has_status {
        return;
    }
    let deep_freeze = s.tank.deep_freeze;
    let range = Fixed::from_int(SPIKES_RANGE);
    let r2 = range.mul(range);
    let tank_pos = s.tank.pos;
    let mut survivors = Vec::with_capacity(s.enemies.len());
    let mut spikes_dealt: i64 = 0;
    // `s.enemies` is id-ordered, so this retaliation pass is stable across runs.
    for mut e in std::mem::take(&mut s.enemies) {
        let boss = content::ENEMIES[e.def as usize].boss;
        if !boss && tank_pos.dist_sq(e.pos) <= r2 {
            if dmg > 0 {
                e.hp -= dmg;
                spikes_dealt += dmg;
            }
            if has_status && crate::status::apply_on_hit(&mut e, &status, deep_freeze) {
                // Frost Armor drove the enemy to the Deep-Freeze payoff.
                s.emit(SimEvent::FreezeProc { id: e.id.0 });
            }
        }
        if e.hp <= 0 {
            s.pending_kills.push(e.def);
            // Render event: spikes kills bypass `reap_dead` (existing behavior:
            // no Fire chain from this path), so announce the kill here.
            let edef = &content::ENEMIES[e.def as usize];
            s.emit(SimEvent::EnemyKilled {
                x: e.pos.x.floor_to_int(),
                y: e.pos.y.floor_to_int(),
                kind: e.def,
                boss: edef.boss,
                bounty: edef.bounty,
                fire_explosion_radius: 0,
            });
        } else {
            survivors.push(e);
        }
    }
    s.enemies = survivors;
    // Spikes retaliation counts as player damage (scoreboard + Bloodmoney).
    s.record_player_damage(spikes_dealt);
}

/// Ticks per in-game second (30 Hz) — cadence of the Missing-HP pulse.
const TICKS_PER_SECOND: u32 = 30;

/// Per-tick regeneration of the Mana Shield and HP (each capped at its max),
/// plus the once-per-second Missing-HP heal. HP healing routes through
/// `Tank::heal` so the "+% Healing" multiplier and the cap apply uniformly.
pub(crate) fn regen(s: &mut ArenaState) {
    // Healthstone permanent regen: a fixed-point per-tick bonus paid out
    // through an integer carry so no fraction is lost (deterministic floors).
    if s.tank.regen_bonus_per_tick > Fixed::ZERO {
        s.tank.regen_carry += s.tank.regen_bonus_per_tick;
        let whole = s.tank.regen_carry.floor_to_int();
        if whole > 0 {
            s.tank.regen_carry -= Fixed::from_int(whole);
            s.tank.heal(whole);
        }
    }
    if s.tank.mana_regen_per_tick > 0 && s.tank.mana_shield < s.tank.mana_shield_max {
        s.tank.mana_shield =
            (s.tank.mana_shield + s.tank.mana_regen_per_tick).min(s.tank.mana_shield_max);
    }
    if s.tank.hp_regen_per_tick > 0 {
        s.tank.heal(s.tank.hp_regen_per_tick);
    } else if s.tank.hp_regen_per_tick < 0 {
        // Negative regen (from a regen→gold trade) is a drain: subtract its
        // magnitude from HP each tick. It bypasses `Tank::heal`/`healing_mult`
        // and CAN bring the tank to death via the normal death path.
        s.tank.hp += s.tank.hp_regen_per_tick;
    }
    // Missing-HP heal: once per second, restore a fraction of the HP deficit.
    if s.tank.missing_hp_heal_pct > Fixed::ZERO && s.tick.is_multiple_of(TICKS_PER_SECOND) {
        let missing = s.tank.max_hp - s.tank.hp;
        if missing > 0 {
            let amount = s.tank.missing_hp_heal_pct.scale_i64(missing);
            s.tank.heal(amount);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> ArenaState {
        ArenaState::new(0xD3F, 0)
    }

    #[test]
    fn plain_damage_reduces_hp() {
        let mut s = fresh();
        let hp0 = s.tank.hp;
        hit_tank(&mut s, 500);
        assert_eq!(s.tank.hp, hp0 - 500);
    }

    #[test]
    fn armor_reduces_but_min_one_lands() {
        let mut s = fresh();
        s.tank.armor = 100;
        let hp0 = s.tank.hp;
        hit_tank(&mut s, 500);
        assert_eq!(s.tank.hp, hp0 - 400);
        // A hit fully absorbed by armor still deals 1.
        s.tank.armor = 10_000;
        let hp1 = s.tank.hp;
        hit_tank(&mut s, 500);
        assert_eq!(s.tank.hp, hp1 - 1);
    }

    #[test]
    fn mana_shield_absorbs_then_overflows_to_hp() {
        let mut s = fresh();
        s.tank.mana_shield = 300;
        s.tank.mana_shield_max = 300;
        let hp0 = s.tank.hp;
        hit_tank(&mut s, 500); // 300 absorbed, 200 to HP
        assert_eq!(s.tank.mana_shield, 0);
        assert_eq!(s.tank.hp, hp0 - 200);
    }

    #[test]
    fn dodge_can_avoid_a_hit_and_is_deterministic() {
        // 100% dodge → no damage, regardless of roll.
        let mut s = fresh();
        s.tank.dodge_num = 100;
        s.tank.dodge_den = 100;
        let hp0 = s.tank.hp;
        hit_tank(&mut s, 9999);
        assert_eq!(s.tank.hp, hp0, "guaranteed dodge takes no damage");

        // Partial dodge: deterministic for a fixed rng.
        let mut a = fresh();
        let mut b = a.clone();
        a.tank.dodge_num = 50;
        b.tank.dodge_num = 50;
        for _ in 0..100 {
            hit_tank(&mut a, 100);
            hit_tank(&mut b, 100);
        }
        assert_eq!(a.tank.hp, b.tank.hp, "same seed → same dodge outcomes");
        assert_eq!(a.rng_proc.state(), b.rng_proc.state());
    }

    fn enemy_at(id: u32, hp: i64, x: i64) -> crate::state::Enemy {
        let pos = crate::state::Vec2::new(Fixed::from_int(x), Fixed::ZERO);
        crate::state::Enemy::new(crate::ids::EntityId(id), 0, hp, pos)
    }

    #[test]
    fn spikes_retaliate_on_nearby_enemies_only_when_hit() {
        let mut s = fresh();
        s.tank.spikes_damage = 100;
        s.enemies = vec![enemy_at(1, 200, 100), enemy_at(2, 200, 1000)]; // near, far

        // Not hit this tick → no retaliation.
        spikes(&mut s);
        assert_eq!(s.enemies[0].hp, 200);

        // Take a hit, then retaliate: near enemy takes 100, far enemy untouched.
        hit_tank(&mut s, 500);
        assert!(s.tank_hit_this_tick);
        spikes(&mut s);
        assert!(!s.tank_hit_this_tick, "hit flag consumed");
        assert_eq!(s.enemies[0].hp, 100, "near enemy took spikes");
        assert_eq!(s.enemies[1].hp, 200, "far enemy out of range");
    }

    #[test]
    fn spikes_mult_scales_and_kills_award_bounty() {
        let mut s = fresh();
        s.tank.spikes_damage = 100;
        s.tank.spikes_mult = Fixed::from_ratio(3, 2); // ×1.5 ⇒ 150
        s.enemies = vec![enemy_at(1, 150, 50)];
        hit_tank(&mut s, 100);
        spikes(&mut s);
        assert!(s.enemies.is_empty(), "150 spikes killed the 150-hp enemy");
        assert_eq!(s.pending_kills, vec![0]);
    }

    #[test]
    fn spikes_does_nothing_without_spikes_damage() {
        let mut s = fresh();
        s.enemies = vec![enemy_at(1, 200, 50)];
        hit_tank(&mut s, 100); // sets the flag
        spikes(&mut s);
        assert_eq!(s.enemies[0].hp, 200, "no spikes stat → no retaliation");
    }

    #[test]
    fn healing_mult_scales_hp_regen() {
        let mut s = fresh();
        s.tank.hp = s.tank.max_hp - 1000;
        s.tank.hp_regen_per_tick = 100;
        s.tank.healing_mult = Fixed::from_int(2); // +100% healing
        regen(&mut s);
        assert_eq!(
            s.tank.hp,
            s.tank.max_hp - 1000 + 200,
            "regen doubled by healing mult"
        );
    }

    #[test]
    fn missing_hp_heal_pulses_once_per_second() {
        let mut s = fresh();
        s.tank.max_hp = 10_000;
        s.tank.hp = 0;
        s.tank.missing_hp_heal_pct = Fixed::from_ratio(1, 4); // 25% of missing (exact)

        // Mid-second → no pulse.
        s.tick = 15;
        regen(&mut s);
        assert_eq!(s.tank.hp, 0, "only pulses on a second boundary");

        // On the boundary → heals 25% of the 10000 deficit.
        s.tick = 30;
        regen(&mut s);
        assert_eq!(s.tank.hp, 2500);
    }

    #[test]
    fn negative_regen_drains_hp_each_tick() {
        let mut s = fresh();
        s.tank.hp = 1000;
        s.tank.max_hp = 24_000;
        s.tank.hp_regen_per_tick = -100; // a drain (from a regen→gold trade)
        regen(&mut s);
        assert_eq!(s.tank.hp, 900, "drain subtracts magnitude each tick");
        regen(&mut s);
        assert_eq!(s.tank.hp, 800);
        // The drain bypasses healing_mult and can push HP below zero (death path).
        s.tank.healing_mult = Fixed::from_int(5);
        s.tank.hp = 50;
        regen(&mut s);
        assert_eq!(
            s.tank.hp, -50,
            "drain unaffected by healing_mult, can go fatal"
        );
    }

    #[test]
    fn spikes_damage_records_to_scoreboard_and_bloodmoney() {
        let mut s = fresh();
        s.tank.spikes_damage = 200;
        s.economy.gold_per_damage = Fixed::from_ratio(1, 4); // exactly representable
        let gold0 = s.economy.gold;
        s.enemies = vec![enemy_at(1, 1000, 50)]; // survives, 200 spikes
        hit_tank(&mut s, 100);
        spikes(&mut s);
        assert_eq!(s.total_damage_dealt, 200, "spikes counted as player damage");
        assert_eq!(s.economy.gold, gold0 + 50, "200 × 1/4 = 50 gold");
        assert_eq!(s.total_gold_earned, 50);
    }

    #[test]
    fn shield_active_dr_reduces_hits_while_shield_up_then_stops() {
        // 50% DR (1/2 is exactly representable) with a 300 shield pool.
        let mut s = fresh();
        s.tank.mana_shield = 300;
        s.tank.mana_shield_max = 300;
        s.tank.shield_active_dr = Fixed::from_ratio(1, 2);
        let hp0 = s.tank.hp;

        // Hit of 500 while shield up: reduced to 250, fully absorbed (≤300 pool),
        // no HP loss. Shield drops to 50.
        hit_tank(&mut s, 500);
        assert_eq!(s.tank.mana_shield, 50, "250 absorbed of a 300 pool");
        assert_eq!(s.tank.hp, hp0, "fully absorbed → no HP loss");

        // Next 500 hit, shield still up (50): reduced to 250, 50 absorbed, 200 to HP.
        hit_tank(&mut s, 500);
        assert_eq!(s.tank.mana_shield, 0, "remaining shield drained");
        assert_eq!(s.tank.hp, hp0 - 200, "overflow also reduced (250-50=200)");

        // Shield now 0 → DR no longer applies. A 500 hit lands in full to HP.
        let hp1 = s.tank.hp;
        hit_tank(&mut s, 500);
        assert_eq!(s.tank.hp, hp1 - 500, "no reduction once shield is 0");
    }

    #[test]
    fn shield_active_dr_keeps_min_one_and_clamps() {
        // 100% DR must still let at least 1 through while the shield is up.
        let mut s = fresh();
        s.tank.mana_shield = 1000;
        s.tank.mana_shield_max = 1000;
        s.tank.shield_active_dr = Fixed::ONE; // 100%
        hit_tank(&mut s, 500);
        assert_eq!(s.tank.mana_shield, 999, "min-1 still absorbed at 100% DR");

        // Over-100% DR is clamped to 100% (multiplier never negative): same result.
        let mut s2 = fresh();
        s2.tank.mana_shield = 1000;
        s2.tank.mana_shield_max = 1000;
        s2.tank.shield_active_dr = Fixed::from_int(5); // 500% → clamped to 100%
        let hp0 = s2.tank.hp;
        hit_tank(&mut s2, 500);
        assert_eq!(s2.tank.mana_shield, 999, "clamped DR still lets 1 land");
        assert_eq!(
            s2.tank.hp, hp0,
            "no negative-damage HP gain from over-clamp"
        );
    }

    #[test]
    fn heal_on_damaged_heals_landed_hit_not_dodged_and_caps() {
        // Lands on a hit: heals the flat amount (routed through `heal`).
        let mut s = fresh();
        s.tank.hp = s.tank.max_hp - 1000;
        s.tank.heal_on_damaged = 8;
        let hp_before = s.tank.hp;
        hit_tank(&mut s, 100); // 100 damage, +8 heal → net -92
        assert_eq!(
            s.tank.hp,
            hp_before + 8 - 100,
            "landed hit heals then takes damage"
        );

        // healing_mult scales the heal.
        let mut s = fresh();
        s.tank.hp = s.tank.max_hp - 1000;
        s.tank.heal_on_damaged = 8;
        s.tank.healing_mult = Fixed::from_int(2); // +100% healing → 16
        let hp_before = s.tank.hp;
        hit_tank(&mut s, 100);
        assert_eq!(
            s.tank.hp,
            hp_before + 16 - 100,
            "heal scaled by healing_mult"
        );

        // Dodged hit does NOT heal (and takes no damage).
        let mut s = fresh();
        s.tank.dodge_num = 100;
        s.tank.dodge_den = 100; // guaranteed dodge
        s.tank.hp = s.tank.max_hp - 1000;
        s.tank.heal_on_damaged = 8;
        let hp_before = s.tank.hp;
        hit_tank(&mut s, 100);
        assert_eq!(s.tank.hp, hp_before, "dodged hit neither heals nor damages");

        // Max-HP cap honored: a hit's heal cannot push HP above max. Set HP so the
        // post-damage value plus heal would exceed max; heal clamps at max.
        let mut s = fresh();
        s.tank.hp = s.tank.max_hp; // already full
        s.tank.heal_on_damaged = 8;
        hit_tank(&mut s, 4); // heal clamps to max, then 4 damage lands
        assert_eq!(
            s.tank.hp,
            s.tank.max_hp - 4,
            "heal capped at max, damage still applies"
        );
    }

    #[test]
    fn regen_refills_up_to_max() {
        let mut s = fresh();
        s.tank.mana_shield = 0;
        s.tank.mana_shield_max = 100;
        s.tank.mana_regen_per_tick = 30;
        s.tank.hp = s.tank.max_hp - 50;
        s.tank.hp_regen_per_tick = 20;
        for _ in 0..10 {
            regen(&mut s);
        }
        assert_eq!(s.tank.mana_shield, 100, "shield capped at max");
        assert_eq!(s.tank.hp, s.tank.max_hp, "hp capped at max");
    }

    // ---- EXPANSION E2: shield-break stun (Energy Pulse) ---------------------

    #[test]
    fn shield_break_stun_fires_on_the_down_edge_and_stuns_only_in_range() {
        // Tank with a 300 shield and the pulse armed (range 500, 15-tick stun).
        let mut s = fresh();
        s.tank.mana_shield = 300;
        s.tank.mana_shield_max = 300;
        s.tank.shieldbreak_stun_range = 500;
        s.tank.shieldbreak_stun_ticks = 15;
        // Two enemies: one in range (≤500), one outside.
        s.enemies = vec![enemy_at(1, 1000, 400), enemy_at(2, 1000, 1000)];

        // A hit that does NOT deplete the shield → no break, no stun.
        hit_tank(&mut s, 100); // 100 absorbed, shield 200 left
        assert!(!s.shield_broke_this_tick, "shield still up → no break edge");
        shield_break_stun(&mut s);
        assert_eq!(
            s.enemies[0].status.stun_ticks, 0,
            "no stun while shield holds"
        );

        // A hit that depletes the shield → the >0→0 down-edge fires the pulse.
        hit_tank(&mut s, 9999); // drains the remaining 200 to 0
        assert!(s.shield_broke_this_tick, "shield broke this tick");
        shield_break_stun(&mut s);
        assert!(!s.shield_broke_this_tick, "break flag consumed");
        assert_eq!(s.enemies[0].status.stun_ticks, 15, "in-range enemy stunned");
        assert_eq!(
            s.enemies[1].status.stun_ticks, 0,
            "out-of-range enemy not stunned"
        );
    }

    #[test]
    fn shield_break_stun_does_not_fire_when_shield_already_zero() {
        // Shield already at 0 → a hit lands on HP, no >0→0 transition, no stun.
        let mut s = fresh();
        s.tank.mana_shield = 0;
        s.tank.mana_shield_max = 300;
        s.tank.shieldbreak_stun_range = 500;
        s.tank.shieldbreak_stun_ticks = 15;
        s.enemies = vec![enemy_at(1, 1000, 100)];
        hit_tank(&mut s, 500);
        assert!(!s.shield_broke_this_tick, "no shield to break ⇒ no edge");
        shield_break_stun(&mut s);
        assert_eq!(
            s.enemies[0].status.stun_ticks, 0,
            "no stun when shield was already 0"
        );
    }

    #[test]
    fn shield_break_stun_is_deterministic_across_two_runs() {
        // Same setup run twice must produce identical enemy state (stable id order).
        let build = || {
            let mut s = fresh();
            s.tank.mana_shield = 100;
            s.tank.mana_shield_max = 100;
            s.tank.shieldbreak_stun_range = 600;
            s.tank.shieldbreak_stun_ticks = 15;
            s.enemies = vec![
                enemy_at(3, 1000, 200),
                enemy_at(1, 1000, 300),
                enemy_at(2, 1000, 5000), // out of range
            ];
            s
        };
        let mut a = build();
        let mut b = build();
        hit_tank(&mut a, 9999);
        hit_tank(&mut b, 9999);
        shield_break_stun(&mut a);
        shield_break_stun(&mut b);
        assert_eq!(a.enemies, b.enemies, "AoE stun identical across runs");
        assert_eq!(a.enemies[0].status.stun_ticks, 15);
        assert_eq!(a.enemies[2].status.stun_ticks, 0, "far enemy untouched");
    }

    // ---- EXPANSION E2: spikes poison + stacking spikes ----------------------

    #[test]
    fn spikes_apply_poison_to_the_reflected_attacker() {
        let mut s = fresh();
        s.tank.spikes_damage = 100;
        s.tank.spikes_poison_dps = 2;
        s.tank.spikes_poison_ticks = 90;
        s.enemies = vec![enemy_at(1, 1000, 100), enemy_at(2, 1000, 1000)]; // near, far
        hit_tank(&mut s, 100);
        spikes(&mut s);
        // Near enemy took spikes AND the poison DoT.
        assert_eq!(s.enemies[0].hp, 900, "near enemy took 100 spikes");
        assert_eq!(s.enemies[0].status.poison_dps, 2, "near enemy poisoned");
        assert_eq!(s.enemies[0].status.poison_ticks, 90);
        // Far enemy untouched (out of spikes range).
        assert_eq!(s.enemies[1].hp, 1000);
        assert_eq!(s.enemies[1].status.poison_dps, 0, "far enemy not poisoned");
    }

    // ---- EXPANSION E3: fidelity mechanics ------------------------------------

    #[test]
    fn frost_and_flaming_armor_stack_status_on_retaliation_even_with_zero_spikes() {
        // Frost Armor (+2 frost) + Flaming Armor (+20 fire) with NO spikes
        // damage: a landed hit still applies both statuses to enemies in range.
        let mut s = fresh();
        s.tank.spikes_damage = 0;
        s.tank.retaliate_frost = 2;
        s.tank.retaliate_fire = 20;
        s.enemies = vec![enemy_at(1, 1000, 100), enemy_at(2, 1000, 1000)]; // near, far
        hit_tank(&mut s, 100);
        spikes(&mut s);
        assert_eq!(s.enemies[0].hp, 1000, "no spikes damage dealt");
        assert_eq!(s.enemies[0].status.frost_stacks, 2, "frost armor applied");
        assert_eq!(s.enemies[0].status.fire_stacks, 20, "flaming armor applied");
        assert_eq!(s.enemies[1].status.frost_stacks, 0, "far enemy untouched");

        // Repeated hits stack the statuses (frost capped at the max).
        hit_tank(&mut s, 100);
        spikes(&mut s);
        assert_eq!(s.enemies[0].status.frost_stacks, 4);
        assert_eq!(s.enemies[0].status.fire_stacks, 40);
    }

    #[test]
    fn frost_armor_freeze_payoff_requires_deep_freeze() {
        // 13 retaliations × 2 stacks crosses 25: with deep_freeze the enemy
        // freezes; without, it caps at 25 stacks.
        for deep in [false, true] {
            let mut s = fresh();
            s.tank.retaliate_frost = 2;
            s.tank.deep_freeze = deep;
            s.enemies = vec![enemy_at(1, 1_000_000, 100)];
            for _ in 0..13 {
                hit_tank(&mut s, 10);
                spikes(&mut s);
            }
            let st = &s.enemies[0].status;
            if deep {
                assert!(st.freeze_ticks > 0, "deep freeze: enemy froze");
                assert_eq!(st.frost_stacks, 0, "stacks reset on freeze");
            } else {
                assert_eq!(st.freeze_ticks, 0, "no freeze without the upgrade");
                assert_eq!(st.frost_stacks, 25, "stacks capped");
            }
        }
    }

    #[test]
    fn first_hit_bonus_spikes_fire_once_per_enemy() {
        // +240 Spikes on an enemy's FIRST landed hit: a persistent (ranged-
        // style) attacker triggers it once; its later hits don't.
        let mut s = fresh();
        s.tank.spikes_damage = 100;
        s.tank.spikes_first_hit = 240;
        s.enemies = vec![enemy_at(1, 1_000_000, 100)];

        // First landed hit: simulate the combat-side caller contract.
        let landed = hit_tank(&mut s, 50);
        assert!(landed);
        if landed && !s.enemies[0].status.hit_tank {
            s.enemies[0].status.hit_tank = true;
            s.spikes_first_bonus_this_tick += s.tank.spikes_first_hit;
        }
        let hp0 = s.enemies[0].hp;
        spikes(&mut s);
        assert_eq!(hp0 - s.enemies[0].hp, 340, "100 flat + 240 first-hit bonus");

        // Second hit from the SAME enemy: no bonus.
        let landed = hit_tank(&mut s, 50);
        if landed && !s.enemies[0].status.hit_tank {
            s.spikes_first_bonus_this_tick += s.tank.spikes_first_hit;
        }
        let hp1 = s.enemies[0].hp;
        spikes(&mut s);
        assert_eq!(
            hp1 - s.enemies[0].hp,
            100,
            "no first-hit bonus on later hits"
        );
    }

    #[test]
    fn deflection_converts_spikes_into_flat_dr_capped_at_half_the_hit() {
        // 25% of 800 spikes = 200 flat DR (1/4 is exactly representable).
        let mut s = fresh();
        s.tank.spikes_damage = 800;
        s.tank.spikes_dr_rate = Fixed::from_ratio(1, 4);
        let hp0 = s.tank.hp;
        hit_tank(&mut s, 500);
        assert_eq!(s.tank.hp, hp0 - 300, "500 − 200 spikes-DR");

        // Cap: DR can never exceed 50% of the attack (tiny 60-damage hit:
        // 200 → capped to 30).
        let mut s2 = fresh();
        s2.tank.spikes_damage = 800;
        s2.tank.spikes_dr_rate = Fixed::from_ratio(1, 4);
        let hp0 = s2.tank.hp;
        hit_tank(&mut s2, 60);
        assert_eq!(s2.tank.hp, hp0 - 30, "DR capped at half the hit");

        // "+% Spikes" raises the DR too (it keys off multiplied spikes).
        let mut s3 = fresh();
        s3.tank.spikes_damage = 800;
        s3.tank.spikes_mult = Fixed::from_int(2); // 1600 effective spikes
        s3.tank.spikes_dr_rate = Fixed::from_ratio(1, 4); // 400 DR
        let hp0 = s3.tank.hp;
        hit_tank(&mut s3, 1000);
        assert_eq!(s3.tank.hp, hp0 - 600, "1000 − 400 multiplied-spikes DR");
    }

    #[test]
    fn damage_taken_converts_into_bonus_spikes() {
        // +25% of damage taken as spikes: a 1000 hit adds 250 to this tick's
        // retaliation (on top of the flat 100).
        let mut s = fresh();
        s.tank.spikes_damage = 100;
        s.tank.dmg_taken_to_spikes = Fixed::from_ratio(1, 4); // exactly representable
        s.enemies = vec![enemy_at(1, 1_000_000, 100)];
        hit_tank(&mut s, 1000);
        assert_eq!(
            s.damage_taken_this_tick, 1000,
            "post-mitigation damage recorded"
        );
        let hp0 = s.enemies[0].hp;
        spikes(&mut s);
        assert_eq!(hp0 - s.enemies[0].hp, 350, "100 flat + 25% of 1000 taken");
        assert_eq!(s.damage_taken_this_tick, 0, "transient consumed");

        // Next tick without a hit: back to the flat value only.
        hit_tank(&mut s, 0); // no-op (raw ≤ 0)
        spikes(&mut s);
        assert_eq!(hp0 - s.enemies[0].hp, 350, "no retaliation without a hit");
    }

    #[test]
    fn healthstone_permanent_regen_pays_out_through_the_carry() {
        // +0.5 HP/tick fixed-point bonus: after 2 ticks exactly 1 HP has been
        // paid (no fraction lost to flooring).
        let mut s = fresh();
        s.tank.max_hp = 1_000_000;
        s.tank.hp = 1000;
        s.tank.regen_bonus_per_tick = Fixed::from_ratio(1, 2);
        regen(&mut s);
        assert_eq!(s.tank.hp, 1000, "0.5 carried, nothing paid yet");
        assert_eq!(s.tank.regen_carry, Fixed::from_ratio(1, 2));
        regen(&mut s);
        assert_eq!(s.tank.hp, 1001, "carry crossed 1 ⇒ 1 HP paid");
        assert_eq!(
            s.tank.regen_carry,
            Fixed::ZERO,
            "remainder retained exactly"
        );
        // 60 more ticks at 0.5/tick ⇒ +30 HP, deterministic.
        for _ in 0..60 {
            regen(&mut s);
        }
        assert_eq!(s.tank.hp, 1031);
    }

    #[test]
    fn stacking_spikes_accumulate_to_the_cap_then_respect_the_reset() {
        // per-stack 20, cap 3 → bonus grows 20,40,60 then holds at 60.
        let mut s = fresh();
        s.tank.spikes_damage = 0; // isolate the stacking bonus
        s.tank.spikes_stack_per = 20;
        s.tank.spikes_stacks_max = 3;
        s.enemies = vec![enemy_at(1, 1_000_000, 100)];

        let hit_and_spike = |s: &mut ArenaState| {
            hit_tank(s, 100);
            let before = s.enemies[0].hp;
            spikes(s);
            before - s.enemies[0].hp // damage dealt this retaliation
        };
        assert_eq!(hit_and_spike(&mut s), 20, "stack 1 ⇒ 20 spikes");
        assert_eq!(s.tank.spikes_stacks, 1);
        assert_eq!(hit_and_spike(&mut s), 40, "stack 2 ⇒ 40 spikes");
        assert_eq!(hit_and_spike(&mut s), 60, "stack 3 ⇒ 60 spikes (cap)");
        assert_eq!(hit_and_spike(&mut s), 60, "stays at cap, no further growth");
        assert_eq!(s.tank.spikes_stacks, 3, "stacks capped at max");

        // Reset (the round boundary clears it): a fresh stack starts at 20 again.
        s.tank.spikes_stacks = 0;
        assert_eq!(hit_and_spike(&mut s), 20, "after reset, stack 1 ⇒ 20 again");
    }
}
