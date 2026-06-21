//! The modifier / stacking engine (`docs/05 §5.3`). Folds purchased modifiers
//! into a small aggregate and answers the live queries damage resolution and
//! weapon firing ask each tick. Deterministic: pure fixed-point, no RNG, no
//! allocation. Struct fields live in `state.rs` (so snapshots/checksums can
//! serialize them); the behavior lives here.
//!
//! Stacking rule (from the source map): **additive within a kind, multiplicative
//! across distinct multiplicative sources**. So all `+%` of the same additive
//! flavor sum, then the result is multiplied by each independent `×` source.

use crate::content::{ModEffect, ModifierDef};
use crate::state::{Economy, Modifiers, Tank};
use determinism::Fixed;

impl Modifiers {
    pub fn new() -> Modifiers {
        Modifiers {
            add_global: Fixed::ZERO,
            add_by_type: [Fixed::ZERO; 5],
            mul_global: Fixed::ONE,
            attack_speed: Fixed::ZERO,
        }
    }

    /// Fold a purchased modifier in. Damage/attack-speed effects accumulate in
    /// this aggregate; economy/defensive effects apply immediately to `economy`
    /// / `tank` (they need no per-tick re-evaluation).
    pub fn apply(&mut self, def: &ModifierDef, economy: &mut Economy, tank: &mut Tank) {
        match def.effect {
            ModEffect::DamageGlobalPct(n, d) => self.add_global += Fixed::from_ratio(n, d),
            ModEffect::DamageTypePct(t, n, d) => {
                self.add_by_type[t as usize % 5] += Fixed::from_ratio(n, d)
            }
            ModEffect::DamageMulPct(n, d) => {
                self.mul_global = self.mul_global.mul(Fixed::ONE + Fixed::from_ratio(n, d))
            }
            ModEffect::AttackSpeedPct(n, d) => self.attack_speed += Fixed::from_ratio(n, d),
            ModEffect::BountyPct(n, d) => economy.bounty_mult += Fixed::from_ratio(n, d),
            ModEffect::IncomeFlat(f) => economy.income_per_tick += f,
            ModEffect::MaxHp(f) => {
                tank.max_hp += f;
                tank.hp += f;
            }
        }
    }

    /// Total damage multiplier for a hit of `damage_type`: `(1 + Σ matching
    /// additive%) × Π multiplicative factors`.
    pub fn damage_mult(&self, damage_type: u8) -> Fixed {
        let additive = Fixed::ONE + self.add_global + self.add_by_type[damage_type as usize % 5];
        additive.mul(self.mul_global)
    }

    /// Attack-speed multiplier (`1 + Σ attack-speed%`). Effective cooldown is the
    /// base cooldown divided by this.
    pub fn attack_speed_mult(&self) -> Fixed {
        Fixed::ONE + self.attack_speed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{self, DMG_PIERCING};
    use crate::state::ArenaState;

    fn parts() -> (Modifiers, Economy, Tank) {
        let s = ArenaState::new(1, 0);
        (Modifiers::new(), s.economy, s.tank)
    }

    fn modifier(effect: ModEffect) -> ModifierDef {
        ModifierDef { name: "t", rarity: 0, cost: 0, effect }
    }

    #[test]
    fn no_modifiers_is_identity() {
        let m = Modifiers::new();
        assert_eq!(m.damage_mult(DMG_PIERCING), Fixed::ONE);
        assert_eq!(m.attack_speed_mult(), Fixed::ONE);
    }

    #[test]
    fn additive_within_kind_sums() {
        let (mut m, mut e, mut t) = parts();
        // Use exactly-representable ratios (1/4) so the assertion is precise;
        // inexact ratios like 1/10 floor deterministically, which is fine.
        m.apply(&modifier(ModEffect::DamageGlobalPct(1, 4)), &mut e, &mut t);
        m.apply(&modifier(ModEffect::DamageGlobalPct(1, 4)), &mut e, &mut t);
        // +25% +25% additive = ×1.50
        assert_eq!(m.damage_mult(0).scale_i64(1000), 1500);
    }

    #[test]
    fn multiplicative_across_sources() {
        let (mut m, mut e, mut t) = parts();
        m.apply(&modifier(ModEffect::DamageGlobalPct(1, 1)), &mut e, &mut t); // +100% additive ⇒ ×2
        m.apply(&modifier(ModEffect::DamageMulPct(1, 1)), &mut e, &mut t); // ×(1+1)=×2 multiplicative
        // (1 + 1.0) × 2 = 4
        assert_eq!(m.damage_mult(0).scale_i64(1000), 4000);
    }

    #[test]
    fn per_type_only_affects_that_type() {
        let (mut m, mut e, mut t) = parts();
        m.apply(&modifier(ModEffect::DamageTypePct(DMG_PIERCING, 1, 2)), &mut e, &mut t);
        assert_eq!(m.damage_mult(DMG_PIERCING).scale_i64(1000), 1500); // +50%
        assert_eq!(m.damage_mult(content::DMG_SIEGE), Fixed::ONE); // unaffected
    }

    #[test]
    fn economy_and_hp_apply_immediately() {
        let (mut m, mut e, mut t) = parts();
        let hp0 = t.hp;
        let maxhp0 = t.max_hp;
        let income0 = e.income_per_tick;
        let bounty0 = e.bounty_mult;
        m.apply(&modifier(ModEffect::IncomeFlat(20)), &mut e, &mut t);
        m.apply(&modifier(ModEffect::BountyPct(1, 2)), &mut e, &mut t);
        m.apply(&modifier(ModEffect::MaxHp(2000)), &mut e, &mut t);
        assert_eq!(e.income_per_tick, income0 + 20);
        assert_eq!(e.bounty_mult, bounty0 + Fixed::from_ratio(1, 2));
        assert_eq!(t.max_hp, maxhp0 + 2000);
        assert_eq!(t.hp, hp0 + 2000);
        // Aggregate damage untouched by economy/hp modifiers.
        assert_eq!(m.damage_mult(0), Fixed::ONE);
    }

    #[test]
    fn attack_speed_accumulates() {
        let (mut m, mut e, mut t) = parts();
        m.apply(&modifier(ModEffect::AttackSpeedPct(1, 4)), &mut e, &mut t);
        assert_eq!(m.attack_speed_mult().scale_i64(1000), 1250);
    }
}
