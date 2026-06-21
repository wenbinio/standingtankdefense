//! The modifier / stacking engine (`docs/05 §5.3`). Folds purchased modifiers
//! into a small aggregate and answers the live queries damage resolution and
//! weapon firing ask each tick. Deterministic: pure fixed-point, no RNG, no
//! allocation. Struct fields live in `state.rs` (so snapshots/checksums can
//! serialize them); the behavior lives here.
//!
//! Stacking rule (from the source map): **additive within a kind, multiplicative
//! across distinct multiplicative sources**. So all `+%` of the same additive
//! flavor sum, then the result is multiplied by each independent `×` source.

use crate::content::{self, ModEffect, ModifierDef, WeaponDef};
use crate::state::{ArenaState, Economy, Modifiers, Tank};
use determinism::Fixed;

/// Phase: apply each active time-scaling ramp whose interval has elapsed
/// (`docs/06`). Deterministic — fixed ticks, fixed order (ramps are append-only,
/// never reordered).
pub(crate) fn apply_ramps(s: &mut ArenaState) {
    if s.ramps.is_empty() {
        return;
    }
    let mut ramps = std::mem::take(&mut s.ramps);
    for r in ramps.iter_mut() {
        while s.tick >= r.next_apply {
            s.modifiers.apply_effect(r.effect, &mut s.economy, &mut s.tank);
            r.next_apply += r.interval_ticks;
        }
    }
    s.ramps = ramps;
}

impl Modifiers {
    pub fn new() -> Modifiers {
        Modifiers {
            add_global: Fixed::ZERO,
            add_by_type: [Fixed::ZERO; 5],
            add_by_scope: [Fixed::ZERO; content::NUM_SCOPES],
            mul_global: Fixed::ONE,
            attack_speed: Fixed::ZERO,
        }
    }

    /// Full damage multiplier for a specific weapon: global + its damage type +
    /// each scope it matches (attack class, range bucket, rarity), times the
    /// multiplicative product. Consulted at fire time (`docs/05 §5.3`).
    pub fn weapon_damage_mult(&self, w: &WeaponDef) -> Fixed {
        let mut add = self.add_global + self.add_by_type[w.damage_type as usize % 5];
        add += self.add_by_scope[content::attack_scope_id(w.attack) as usize];
        add += self.add_by_scope[content::range_scope_id(w.range) as usize];
        add += self.add_by_scope[content::rarity_scope_id(w.rarity) as usize];
        (Fixed::ONE + add).mul(self.mul_global)
    }

    /// Fold a purchased modifier in (applies its base `effect`).
    pub fn apply(&mut self, def: &ModifierDef, economy: &mut Economy, tank: &mut Tank) {
        self.apply_effect(def.effect, economy, tank);
    }

    /// Apply a single [`ModEffect`] (used both for a modifier's base effect and
    /// for each tick of a time-scaling ramp). Damage/attack-speed effects
    /// accumulate in this aggregate; economy/defensive effects apply immediately.
    pub fn apply_effect(&mut self, effect: ModEffect, economy: &mut Economy, tank: &mut Tank) {
        match effect {
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
            ModEffect::Armor(a) => tank.armor += a,
            ModEffect::ManaShield(pool, regen) => {
                tank.mana_shield_max += pool;
                tank.mana_shield += pool;
                tank.mana_regen_per_tick += regen;
            }
            ModEffect::HpRegen(r) => tank.hp_regen_per_tick += r,
            ModEffect::Dodge(n) => {
                // Additive, capped just below 100% so a hit can always land.
                tank.dodge_num = (tank.dodge_num + n).min(tank.dodge_den.saturating_sub(1));
            }
            ModEffect::DamageScopePct(sid, n, d) => {
                self.add_by_scope[sid as usize % content::NUM_SCOPES] += Fixed::from_ratio(n, d)
            }
            ModEffect::SpikesFlat(n) => tank.spikes_damage += n,
            ModEffect::SpikesPct(n, d) => tank.spikes_mult += Fixed::from_ratio(n, d),
            // Registered as per-arena trigger state in `buy_modifier`; no aggregate.
            ModEffect::GrantVulnPulse(..) => {}
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
        ModifierDef { name: "t", rarity: 0, cost: 0, effect, ramp: None }
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

    #[test]
    fn ramping_modifier_grows_each_interval() {
        // Find a ramping modifier in the catalog (Building Power: +2% now, +1%/round).
        let idx = content::MODIFIERS
            .iter()
            .position(|md| md.ramp.is_some()
                && matches!(md.effect, ModEffect::DamageGlobalPct(2, 100)))
            .expect("a +2%/+1% global-damage ramp exists") as u16;

        let mut s = ArenaState::new(1, 0);
        s.buy_modifier(idx);
        assert_eq!(s.ramps.len(), 1, "ramp registered on purchase");
        let base = s.modifiers.damage_mult(0); // ≈ ×1.02

        // Apply three intervals of growth.
        let interval = content::RAMP_PER_ROUND;
        for k in 1..=3u32 {
            s.tick = interval * k;
            apply_ramps(&mut s);
        }
        let after = s.modifiers.damage_mult(0); // ≈ ×1.05 (0.02 + 3×0.01)
        assert!(after > base, "ramp increased the multiplier");
        let v = after.scale_i64(1_000_000);
        assert!((1_049_000..=1_050_000).contains(&v), "expected ≈1.05, got {v}");

        // The ramp's next_apply advanced past the last interval (no double-apply).
        assert_eq!(s.ramps[0].next_apply, interval * 4);
    }

    #[test]
    fn ramp_apply_is_idempotent_between_intervals() {
        let idx = content::MODIFIERS.iter().position(|md| md.ramp.is_some()).unwrap() as u16;
        let mut s = ArenaState::new(2, 0);
        s.buy_modifier(idx);
        let snap = (s.modifiers.clone(), s.economy.clone(), s.tank.clone());
        // A tick before the first interval: nothing should change.
        s.tick = content::RAMP_PER_ROUND - 1;
        apply_ramps(&mut s);
        assert_eq!(s.modifiers, snap.0);
        assert_eq!(s.economy, snap.1);
        assert_eq!(s.tank, snap.2);
    }
}
