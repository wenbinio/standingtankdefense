//! Tank defensive layer (`docs/05 §2.2`): Dodge → Armor → Mana Shield → HP, plus
//! per-tick Mana-Shield and HP regeneration. Deterministic — the only RNG is the
//! dodge roll from `rng_proc`.

use crate::content;
use crate::state::ArenaState;
use determinism::Fixed;

/// Radius (from the tank) of Spikes retaliation.
const SPIKES_RANGE: i64 = 400;

/// Apply `raw` incoming damage to the tank through the defensive layers.
pub(crate) fn hit_tank(s: &mut ArenaState, raw: i64) {
    if raw <= 0 {
        return;
    }
    // Dodge: chance to avoid the hit entirely (consumes a proc roll).
    if s.tank.dodge_den > 0 {
        let roll = s.rng_proc.below(s.tank.dodge_den);
        if roll < s.tank.dodge_num {
            return;
        }
    }
    // The hit landed (even if fully absorbed by the shield) → Spikes will fire.
    s.tank_hit_this_tick = true;
    // Armor: flat reduction, but at least 1 damage always lands.
    let mut remaining = (raw - s.tank.armor).max(1);
    // Mana Shield absorbs before HP.
    if s.tank.mana_shield > 0 {
        let absorbed = remaining.min(s.tank.mana_shield);
        s.tank.mana_shield -= absorbed;
        remaining -= absorbed;
    }
    s.tank.hp -= remaining;
}

/// Spikes retaliation: if the tank was hit this tick, deal its (multiplied)
/// spikes damage to every non-boss enemy within `SPIKES_RANGE`. Consumes and
/// resets the hit flag. Kills push to `pending_kills` (so they award bounty).
pub(crate) fn spikes(s: &mut ArenaState) {
    let hit = s.tank_hit_this_tick;
    s.tank_hit_this_tick = false;
    if !hit {
        return;
    }
    let dmg = s.tank.spikes_mult.scale_i64(s.tank.spikes_damage);
    if dmg <= 0 {
        return;
    }
    let range = Fixed::from_int(SPIKES_RANGE);
    let r2 = range.mul(range);
    let tank_pos = s.tank.pos;
    let mut survivors = Vec::with_capacity(s.enemies.len());
    let mut spikes_dealt: i64 = 0;
    for mut e in std::mem::take(&mut s.enemies) {
        let boss = content::ENEMIES[e.def as usize].boss;
        if !boss && tank_pos.dist_sq(e.pos) <= r2 {
            e.hp -= dmg;
            spikes_dealt += dmg;
        }
        if e.hp <= 0 {
            s.pending_kills.push(e.def);
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
    if s.tank.missing_hp_heal_pct > Fixed::ZERO && s.tick % TICKS_PER_SECOND == 0 {
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
        assert_eq!(s.tank.hp, s.tank.max_hp - 1000 + 200, "regen doubled by healing mult");
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
        assert_eq!(s.tank.hp, -50, "drain unaffected by healing_mult, can go fatal");
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
}
