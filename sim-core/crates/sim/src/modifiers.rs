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
            vs_stunned: Fixed::ZERO,
            vs_poisoned: Fixed::ZERO,
            poison_dmg_mult: Fixed::ONE,
            stun_dur_mult: Fixed::ONE,
            weapon_count_scaling: Vec::new(),
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

    /// Additive self-scaling % for a weapon of `dmg_type`, given the owned
    /// weapons: `Σ rule.per × (count of rule.weapon_def)` over rules matching the
    /// type. Resolved live at fire time (it depends on the current arsenal).
    pub fn self_scaling_add(&self, dmg_type: u8, weapons: &[crate::state::WeaponInstance]) -> Fixed {
        let mut add = Fixed::ZERO;
        for rule in &self.weapon_count_scaling {
            if rule.dmg_type != dmg_type {
                continue;
            }
            let count = weapons.iter().filter(|w| w.def == rule.weapon_def).count() as i64;
            if count > 0 {
                add += rule.per.mul(Fixed::from_int(count));
            }
        }
        add
    }

    /// Scale a weapon's on-hit status by the player's flavor modifiers
    /// (Poison-damage % and Stun-duration %). Depends only on the modifiers, so
    /// it bakes into the hit at fire time (like base damage). Frost/Fire stacks
    /// are unaffected here.
    pub fn scale_on_hit(&self, mut on_hit: content::StatusOnHit) -> content::StatusOnHit {
        if on_hit.poison_dps > 0 && self.poison_dmg_mult != Fixed::ONE {
            on_hit.poison_dps = self.poison_dmg_mult.scale_i64(on_hit.poison_dps);
        }
        if on_hit.stun_ticks > 0 && self.stun_dur_mult != Fixed::ONE {
            on_hit.stun_ticks = self.stun_dur_mult.scale_i64(on_hit.stun_ticks as i64) as u32;
        }
        on_hit
    }

    /// Fold a purchased modifier in (applies each of its base `effects`, in order).
    pub fn apply(&mut self, def: &ModifierDef, economy: &mut Economy, tank: &mut Tank) {
        for &effect in def.effects {
            self.apply_effect(effect, economy, tank);
        }
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
                // Additive, but HARD-CAPPED at 70% of `dodge_den` (integer math:
                // `dodge_den * 7 / 10`). This is a deliberate nerf: at the old
                // 99% ceiling dodge was an invincibility button — once a build hit
                // it, NO enemy lever (HP / contact / volume / boss) could pressure
                // it and ~88% of runs coasted to the 60-min cap purely on dodge.
                // Capping the effective avoid-rate at 70% leaves dodge strong but
                // mortal: ~30% of hits still land, so the HP staircase and the boss
                // can finally threaten a snowball, and mitigation (armor / shield /
                // regen / spikes) becomes a real competing axis instead of a
                // strictly-worse alternative. Integer/Fixed only — feeds the
                // checksum, so no floats.
                let cap = tank.dodge_den.saturating_mul(7) / 10;
                tank.dodge_num = (tank.dodge_num + n).min(cap);
            }
            ModEffect::DamageScopePct(sid, n, d) => {
                self.add_by_scope[sid as usize % content::NUM_SCOPES] += Fixed::from_ratio(n, d)
            }
            ModEffect::SpikesFlat(n) => tank.spikes_damage += n,
            ModEffect::SpikesPct(n, d) => tank.spikes_mult += Fixed::from_ratio(n, d),
            ModEffect::HealOnKill(n) => tank.heal_on_kill += n,
            ModEffect::HealOnPoison(n) => tank.heal_on_poison += n,
            ModEffect::IncomePct(n, d) => economy.income_mult += Fixed::from_ratio(n, d),
            ModEffect::IncomeRegenPct(n, d) => economy.income_regen_pct += Fixed::from_ratio(n, d),
            ModEffect::BountyProc(c, b) => {
                economy.bounty_proc_chance_pct += c;
                economy.bounty_proc_bonus += Fixed::from_ratio(b, 100);
            }
            ModEffect::DamageVsStunnedPct(n, d) => self.vs_stunned += Fixed::from_ratio(n, d),
            ModEffect::DamageVsPoisonedPct(n, d) => self.vs_poisoned += Fixed::from_ratio(n, d),
            ModEffect::PoisonDamagePct(n, d) => self.poison_dmg_mult += Fixed::from_ratio(n, d),
            ModEffect::StunDurationPct(n, d) => self.stun_dur_mult += Fixed::from_ratio(n, d),
            ModEffect::HealingPct(n, d) => tank.healing_mult += Fixed::from_ratio(n, d),
            ModEffect::MissingHpHealPct(n, d) => tank.missing_hp_heal_pct += Fixed::from_ratio(n, d),
            ModEffect::GrantRevive(bonus) => {
                tank.revives += 1;
                tank.revive_bonus_hp = tank.revive_bonus_hp.max(bonus);
            }
            ModEffect::DamagePerWeapon(def, ty, num) => {
                self.weapon_count_scaling.push(crate::state::WeaponCountScale {
                    weapon_def: def as u16,
                    dmg_type: ty as u8,
                    per: Fixed::from_ratio(num, 100),
                });
            }
            ModEffect::GoldPerDamagePct(n, d) => economy.gold_per_damage += Fixed::from_ratio(n, d),
            ModEffect::IncomeShieldPct(n, d) => economy.income_shield_pct += Fixed::from_ratio(n, d),
            // Registered as per-arena trigger / purchase-flow state in
            // `buy_modifier`; they have no aggregate contribution here. The
            // HP/regen→gold trades need the full ArenaState (gold scoreboard) and
            // are likewise intercepted in `buy_modifier`.
            ModEffect::GrantVulnPulse(..)
            | ModEffect::GrantDuplicator(..)
            | ModEffect::GrantVoucher(..)
            | ModEffect::GrantGold(..)
            | ModEffect::TradeMaxHpForGold(..)
            | ModEffect::TradeRegenForGold(..) => {}
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
        // A single-effect def matching the catalog's one-effect entries. Leaks a
        // 'static slice so the test def can hold `effects: &'static [ModEffect]`.
        let effects: &'static [ModEffect] = Box::leak(Box::new([effect]));
        ModifierDef { name: "t", rarity: 0, cost: 0, effects, ramp: None }
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
                && md.effects.iter().any(|e| matches!(e, ModEffect::DamageGlobalPct(2, 100))))
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

    fn modifier_idx(pred: impl Fn(ModEffect) -> bool) -> u16 {
        content::MODIFIERS
            .iter()
            .position(|m| m.effects.iter().any(|&e| pred(e)))
            .expect("catalog item exists") as u16
    }

    #[test]
    fn trade_maxhp_for_gold_reduces_maxhp_and_grants_scored_gold() {
        let mut s = ArenaState::new(1, 0);
        let idx = modifier_idx(|e| matches!(e, ModEffect::TradeMaxHpForGold(..)));
        let (hp_cost, gold_gain) = content::MODIFIERS[idx as usize]
            .effects
            .iter()
            .find_map(|e| match e {
                ModEffect::TradeMaxHpForGold(h, g) => Some((*h, *g)),
                _ => None,
            })
            .unwrap();
        let max0 = s.tank.max_hp;
        let gold0 = s.economy.gold;
        // Clamp path: set HP near the post-trade max so the clamp is exercised.
        s.tank.hp = max0;
        s.buy_modifier(idx);
        assert_eq!(s.tank.max_hp, max0 - hp_cost, "max hp reduced");
        assert_eq!(s.tank.hp, max0 - hp_cost, "hp clamped to new max");
        assert_eq!(s.economy.gold, gold0 + gold_gain);
        assert_eq!(s.total_gold_earned, gold_gain, "trade gold scored");
    }

    #[test]
    fn trade_regen_for_gold_can_go_negative() {
        let mut s = ArenaState::new(1, 0);
        let idx = modifier_idx(|e| matches!(e, ModEffect::TradeRegenForGold(..)));
        let (regen_cost, gold_gain) = content::MODIFIERS[idx as usize]
            .effects
            .iter()
            .find_map(|e| match e {
                ModEffect::TradeRegenForGold(r, g) => Some((*r, *g)),
                _ => None,
            })
            .unwrap();
        s.tank.hp_regen_per_tick = 0;
        let gold0 = s.economy.gold;
        s.buy_modifier(idx);
        assert_eq!(s.tank.hp_regen_per_tick, -regen_cost, "regen may go negative");
        assert_eq!(s.economy.gold, gold0 + gold_gain);
    }

    #[test]
    fn gold_per_damage_accrues_from_combat_damage() {
        // Buy a Bloodmoney, then deal damage and confirm gold accrues + is scored.
        let mut s = ArenaState::new(1, 0);
        let idx = modifier_idx(|e| matches!(e, ModEffect::GoldPerDamagePct(..)));
        s.buy_modifier(idx);
        assert!(s.economy.gold_per_damage > Fixed::ZERO);
        let gold0 = s.economy.gold;
        // record 1000 player damage directly (same path all sites use).
        s.record_player_damage(1000);
        assert_eq!(s.total_damage_dealt, 1000);
        let expected = s.economy.gold_per_damage.scale_i64(1000);
        assert!(expected > 0);
        assert_eq!(s.economy.gold, gold0 + expected);
        assert_eq!(s.total_gold_earned, expected, "bloodmoney gold scored");
    }

    #[test]
    fn overclocked_death_engine_scales_with_its_own_count() {
        // Buying the generator modifier registers a self-scaling rule keyed to the
        // Death Engine weapon; owning N Death Engines adds N × per to its damage.
        let mut s = ArenaState::new(1, 0);
        s.weapons.clear();
        let idx = modifier_idx(|e| matches!(e, ModEffect::DamagePerWeapon(d, _, _) if d == content::DEATH_ENGINE as i64));
        s.buy_modifier(idx);
        for _ in 0..3 {
            let id = s.alloc_entity_id();
            s.weapons.push(crate::state::WeaponInstance {
                instance_id: id,
                def: content::DEATH_ENGINE,
                next_fire_tick: s.tick,
            });
        }
        // 3 engines × 10% = +~30% Chaos self-scaling (fixed-point floors slightly).
        let add = s.modifiers.self_scaling_add(content::DMG_CHAOS, &s.weapons);
        let v = add.scale_i64(1000);
        assert!((299..=300).contains(&v), "≈3 × 10% = 30%, got {v}");
        // A non-engine weapon of the same type is unaffected when count is 0.
        let none = s.modifiers.self_scaling_add(content::DMG_CHAOS, &[]);
        assert_eq!(none, Fixed::ZERO, "no engines ⇒ no self-scaling");
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
