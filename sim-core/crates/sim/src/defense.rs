//! Tank defensive layer (`docs/05 §2.2`): Dodge → Armor → Mana Shield → HP, plus
//! per-tick Mana-Shield and HP regeneration. Deterministic — the only RNG is the
//! dodge roll from `rng_proc`.

use crate::state::ArenaState;

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

/// Per-tick regeneration of the Mana Shield and HP (each capped at its max).
pub(crate) fn regen(s: &mut ArenaState) {
    if s.tank.mana_regen_per_tick > 0 && s.tank.mana_shield < s.tank.mana_shield_max {
        s.tank.mana_shield =
            (s.tank.mana_shield + s.tank.mana_regen_per_tick).min(s.tank.mana_shield_max);
    }
    if s.tank.hp_regen_per_tick > 0 && s.tank.hp < s.tank.max_hp {
        s.tank.hp = (s.tank.hp + s.tank.hp_regen_per_tick).min(s.tank.max_hp);
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
