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

/// Overflow guard for the compounding percentage riders (`MaxHpPct` / `HpRegenPct`
/// / `ManaRegenPct`). Each such rider multiplies its stat by ~1.25 per purchase,
/// so a pathological repeat-buy stack would otherwise run a stat to i64 overflow
/// and poison every downstream `hp + x`. 1e12 sits orders of magnitude above any
/// reachable real build (max HP tops out in the low millions even on a full
/// snowball) yet leaves ~7 orders of headroom under i64::MAX for downstream adds,
/// so the clamp is bit-stable across platforms and never trips in normal play.
const STAT_CEIL: i64 = 1_000_000_000_000;

/// Phase: apply each active time-scaling ramp whose interval has elapsed
/// (`docs/06`). Deterministic — fixed ticks, fixed order (ramps are append-only,
/// never reordered).
///
/// SOURCE RULE (docs/01, changelog): per-round "+N per round" ramps "no longer
/// stack after 15 minutes" — accrual FREEZES at the boss tick
/// (`content::BOSS_SPAWN_TICK`). The already-accrued value keeps working; only
/// the growth stops. One guard here covers every ramping modifier.
pub(crate) fn apply_ramps(s: &mut ArenaState) {
    if s.tick >= content::BOSS_SPAWN_TICK || s.ramps.is_empty() {
        return;
    }
    let mut ramps = std::mem::take(&mut s.ramps);
    for r in ramps.iter_mut() {
        while s.tick >= r.next_apply {
            s.modifiers
                .apply_effect(r.effect, &mut s.economy, &mut s.tank);
            r.next_apply += r.interval_ticks;
        }
    }
    s.ramps = ramps;
}

impl Default for Modifiers {
    fn default() -> Modifiers {
        Modifiers::new()
    }
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
            dmg_per_maxhp_rate: Fixed::ZERO,
            dmg_per_bounty_rate: Fixed::ZERO,
            shield_active_dmg: Fixed::ZERO,
            frost_strength_mult: Fixed::ONE,
            fire_dmg_mult: Fixed::ONE,
            fire_explosion_mult: Fixed::ONE,
            bounce_barrage_pct: Fixed::ZERO,
            healthy_dmg: Fixed::ZERO,
            healing_weapon_healthy_dmg: Fixed::ZERO,
        }
    }

    /// Live GLOBAL additive damage bonus from the three DYNAMIC scalers, evaluated
    /// against the current `tank`/`economy` (NOT baked at purchase) — the offensive
    /// analogue of `self_scaling_add`. Folded into the per-weapon multiplier at fire
    /// time exactly like `self_scaling_add` (additive, then `×mul_global`):
    ///   * Mastercrafted Masonry: `rate × (max_hp / 2000)`.
    ///   * Golden Ring: `rate × ((bounty_mult − 1) / 0.5)`  ( = rate × 2 × bonus ).
    ///   * Arcane Mark: `shield_active_dmg` while `mana_shield > 0`, else nothing.
    ///
    /// Integer/Fixed only (feeds the checksum). A `bounty_mult` below the 1.0 base
    /// (never happens in normal play) is clamped so the term can't go negative.
    pub fn dynamic_global_add(&self, tank: &Tank, economy: &Economy) -> Fixed {
        let mut add = Fixed::ZERO;
        if self.dmg_per_maxhp_rate != Fixed::ZERO && tank.max_hp > 0 {
            // units = max_hp / 2000 (Fixed); bonus = rate × units.
            let units = Fixed::from_ratio(tank.max_hp, 2000);
            add += self.dmg_per_maxhp_rate.mul(units);
        }
        if self.dmg_per_bounty_rate != Fixed::ZERO {
            // bounty bonus above the 1.0 base, in units of 50% (×2 of the fraction).
            let bonus = economy.bounty_mult - Fixed::ONE;
            if bonus > Fixed::ZERO {
                let units = bonus.mul(Fixed::from_int(2));
                add += self.dmg_per_bounty_rate.mul(units);
            }
        }
        if self.shield_active_dmg != Fixed::ZERO && tank.mana_shield > 0 {
            add += self.shield_active_dmg;
        }
        // GLOBAL heal-conditional damage ("+35% Damage … when at 95% health or
        // above"): active iff the shared ≥95%-HP gate (`Tank::is_healthy`) holds.
        if self.healthy_dmg != Fixed::ZERO && tank.is_healthy() {
            add += self.healthy_dmg;
        }
        add
    }

    /// HEALING-WEAPON-scoped heal-conditional additive bonus (Battle Fervor's
    /// "+35% Damage for Healing Weapons … when at 95% health or above"):
    /// nonzero only when the FIRING weapon is a healing weapon
    /// ([`WeaponDef::is_healing`]) AND the same ≥95%-HP gate as
    /// `DamageWhileHealthyPct` holds ([`Tank::is_healthy`]). Resolved LIVE at
    /// fire time and folded into the per-weapon multiplier exactly like
    /// `dynamic_global_add` (additive, then `×mul_global`). Integer/Fixed only.
    pub fn healing_weapon_add(&self, w: &WeaponDef, tank: &Tank) -> Fixed {
        if self.healing_weapon_healthy_dmg != Fixed::ZERO && w.is_healing() && tank.is_healthy() {
            self.healing_weapon_healthy_dmg
        } else {
            Fixed::ZERO
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
    pub fn self_scaling_add(
        &self,
        dmg_type: u8,
        weapons: &[crate::state::WeaponInstance],
    ) -> Fixed {
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
            ModEffect::ShieldActiveDrPct(n, d) => tank.shield_active_dr += Fixed::from_ratio(n, d),
            ModEffect::HealOnDamaged(n) => tank.heal_on_damaged += n,
            ModEffect::SpikesFlat(n) => tank.spikes_damage += n,
            ModEffect::SpikesPct(n, d) => tank.spikes_mult += Fixed::from_ratio(n, d),
            ModEffect::HealOnKill(n) => tank.heal_on_kill += n,
            ModEffect::ManaOnKill(n) => tank.mana_on_kill += n,
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
            ModEffect::MissingHpHealPct(n, d) => {
                tank.missing_hp_heal_pct += Fixed::from_ratio(n, d)
            }
            ModEffect::GrantRevive(bonus) => {
                tank.revives += 1;
                tank.revive_bonus_hp = tank.revive_bonus_hp.max(bonus);
            }
            ModEffect::DamagePerWeapon(def, ty, num) => {
                self.weapon_count_scaling
                    .push(crate::state::WeaponCountScale {
                        weapon_def: def as u16,
                        dmg_type: ty as u8,
                        per: Fixed::from_ratio(num, 100),
                    });
            }
            // DYNAMIC global-damage scalers (resolved live at fire time via
            // `dynamic_global_add`, mirroring `DamagePerWeapon`/`self_scaling_add`).
            // Accumulate only the per-unit RATE / flat bonus here.
            ModEffect::DamagePerMaxHp(n, d) => self.dmg_per_maxhp_rate += Fixed::from_ratio(n, d),
            ModEffect::DamagePerBountyPct(n, d) => {
                self.dmg_per_bounty_rate += Fixed::from_ratio(n, d)
            }
            ModEffect::ShieldActiveDamagePct(n, d) => {
                self.shield_active_dmg += Fixed::from_ratio(n, d)
            }
            ModEffect::GoldPerDamagePct(n, d) => economy.gold_per_damage += Fixed::from_ratio(n, d),
            ModEffect::IncomeShieldPct(n, d) => {
                economy.income_shield_pct += Fixed::from_ratio(n, d)
            }
            // Percentage riders evaluated against the CURRENT stat at apply-time
            // (so in a bundle they compound on the flat effect listed before them).
            // Integer/Fixed only — these feed the checksum. The compounding is
            // CLAMPED to `STAT_CEIL`: repeatedly buying a `[flat, pct]` bundle
            // multiplies the stat by ~1.25 each time, which would otherwise overflow
            // i64 (and poison every downstream `hp + x`) in a pathological purchase
            // stack. `STAT_CEIL` (1e12) sits far above any reachable real build yet
            // leaves ample headroom for downstream adds, so the clamp is bit-stable
            // across platforms and never trips in normal play. Same spirit as the
            // Dodge hard-cap. The inc itself is also derived from the clamped stat,
            // so it stays bounded.
            ModEffect::MaxHpPct(n, d) => {
                let inc = Fixed::from_ratio(n, d).scale_i64(tank.max_hp);
                tank.max_hp = (tank.max_hp + inc).min(STAT_CEIL);
                tank.hp += inc;
            }
            ModEffect::HpRegenPct(n, d) => {
                let inc = Fixed::from_ratio(n, d).scale_i64(tank.hp_regen_per_tick);
                tank.hp_regen_per_tick =
                    (tank.hp_regen_per_tick + inc).clamp(-STAT_CEIL, STAT_CEIL);
            }
            ModEffect::ManaRegenPct(n, d) => {
                let inc = Fixed::from_ratio(n, d).scale_i64(tank.mana_regen_per_tick);
                tank.mana_regen_per_tick =
                    (tank.mana_regen_per_tick + inc).clamp(-STAT_CEIL, STAT_CEIL);
            }
            // EXPANSION E2 — plain tank-field setters (no aggregate; integer only).
            // Shield-break stun (Energy Pulse): arm the pulse range/duration. Keep the
            // STRONGER of any stacked copies so two Energy Pulses don't shrink the
            // window.
            ModEffect::ShieldBreakStun(r, t) => {
                tank.shieldbreak_stun_range = tank.shieldbreak_stun_range.max(r);
                tank.shieldbreak_stun_ticks = tank.shieldbreak_stun_ticks.max(t as u32);
            }
            // Spikes poison (Poison Armor): keep whichever DoT deals more total
            // remaining damage (mirrors the poison-application rule in `status`).
            ModEffect::SpikesPoison(dps, t) => {
                let existing = tank
                    .spikes_poison_dps
                    .saturating_mul(tank.spikes_poison_ticks as i64);
                let incoming = dps.saturating_mul(t);
                if incoming >= existing {
                    tank.spikes_poison_dps = dps;
                    tank.spikes_poison_ticks = t as u32;
                }
            }
            // Stacking spikes (Bloody Spikes): set the per-stack bonus and the cap.
            // Additive per-stack, max cap takes the larger of stacked copies.
            ModEffect::StackingSpikes(per, max) => {
                tank.spikes_stack_per += per;
                tank.spikes_stacks_max = tank.spikes_stacks_max.max(max as u32);
            }
            // Damage/poison aura (Blight Aura): arm the emitter. Keep the wider range,
            // shorter (more frequent) cadence, and larger damage of any stacked
            // copies; the poison rider mirrors Poison Armor's DoT (2/tick × 90).
            ModEffect::DamageAura(r, c, d) => {
                tank.aura_range = tank.aura_range.max(r);
                tank.aura_damage += d;
                let c = c as u32;
                tank.aura_cadence = if tank.aura_cadence == 0 {
                    c
                } else {
                    tank.aura_cadence.min(c)
                };
                tank.aura_poison_dps = tank.aura_poison_dps.max(2);
                tank.aura_poison_ticks = tank.aura_poison_ticks.max(90);
            }
            // EXPANSION E3 — fidelity-mechanics pass.
            // Deep Freeze opt-in: arm the tank flag (status::apply_on_hit gates
            // the 25-stack freeze payoff on it).
            ModEffect::GrantDeepFreeze => tank.deep_freeze = true,
            // Frost/Fire strength scalers (additive accumulation, like the
            // poison/stun flavor scalers above).
            ModEffect::FrostDamagePct(n, d) => self.frost_strength_mult += Fixed::from_ratio(n, d),
            ModEffect::FireDamagePct(n, d) => self.fire_dmg_mult += Fixed::from_ratio(n, d),
            ModEffect::CombustionPct(n, d) => self.fire_explosion_mult += Fixed::from_ratio(n, d),
            // "+% Enemies hit by Bounce and Barrage" (resolved per fire).
            ModEffect::BounceBarragePct(n, d) => self.bounce_barrage_pct += Fixed::from_ratio(n, d),
            // Free rerolls: the same counter the shop's Reroll input consumes.
            ModEffect::GrantFreeRerolls(n) => {
                economy.rerolls_remaining =
                    economy.rerolls_remaining.saturating_add(n.max(0) as u32)
            }
            // Status-on-damaged armor riders (Frost / Flaming Armor): applied by
            // the Spikes retaliation pass (which fires even at 0 Spikes damage).
            ModEffect::FrostArmor(n) => {
                tank.retaliate_frost = tank.retaliate_frost.saturating_add(n.max(0) as u8)
            }
            ModEffect::FireArmor(n) => {
                tank.retaliate_fire = tank.retaliate_fire.saturating_add(n.max(0) as u16)
            }
            // Spikes extensions.
            ModEffect::SpikesFirstHit(n) => tank.spikes_first_hit += n,
            ModEffect::SpikesAsDrPct(n, d) => tank.spikes_dr_rate += Fixed::from_ratio(n, d),
            ModEffect::DamageTakenToSpikesPct(n, d) => {
                tank.dmg_taken_to_spikes += Fixed::from_ratio(n, d)
            }
            // Heal-conditional global damage (resolved live in dynamic_global_add).
            ModEffect::DamageWhileHealthyPct(n, d) => self.healthy_dmg += Fixed::from_ratio(n, d),
            // Heal-conditional HEALING-WEAPON damage (resolved live, per firing
            // weapon, in healing_weapon_add).
            ModEffect::HealingWeaponDamagePct(n, d) => {
                self.healing_weapon_healthy_dmg += Fixed::from_ratio(n, d)
            }
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
        ModifierDef {
            name: "t",
            rarity: 0,
            cost: 0,
            effects,
            ramp: None,
        }
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
        m.apply(
            &modifier(ModEffect::DamageTypePct(DMG_PIERCING, 1, 2)),
            &mut e,
            &mut t,
        );
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
            .position(|md| {
                md.ramp.is_some()
                    && md
                        .effects
                        .iter()
                        .any(|e| matches!(e, ModEffect::DamageGlobalPct(2, 100)))
            })
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
        assert!(
            (1_049_000..=1_050_000).contains(&v),
            "expected ≈1.05, got {v}"
        );

        // The ramp's next_apply advanced past the last interval (no double-apply).
        assert_eq!(s.ramps[0].next_apply, interval * 4);
    }

    #[test]
    fn ramps_stop_accruing_at_the_boss_tick() {
        // SOURCE RULE: "+N per round" ramps "no longer stack after 15 minutes"
        // (docs/01 changelog). From `BOSS_SPAWN_TICK` on, `apply_ramps` freezes —
        // the accrued value keeps working, the growth stops.
        let idx = content::MODIFIERS
            .iter()
            .position(|md| md.ramp.is_some())
            .expect("a ramping modifier exists") as u16;
        let mut s = ArenaState::new(1, 0);
        s.buy_modifier(idx);
        // Accrue normally up to just before the boss tick.
        s.tick = content::BOSS_SPAWN_TICK - 1;
        apply_ramps(&mut s);
        let frozen_next = s.ramps[0].next_apply;
        let frozen_mods = s.modifiers.clone();
        // At and past the boss tick: no further accrual, ever.
        for dt in [0u32, 1, 900, 5400] {
            s.tick = content::BOSS_SPAWN_TICK + dt;
            apply_ramps(&mut s);
            assert_eq!(
                s.ramps[0].next_apply, frozen_next,
                "ramp accrued past the boss tick (dt={dt})"
            );
            assert_eq!(
                s.modifiers, frozen_mods,
                "modifier aggregate changed past the boss tick (dt={dt})"
            );
        }
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
        assert_eq!(
            s.tank.hp_regen_per_tick, -regen_cost,
            "regen may go negative"
        );
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
        let idx = modifier_idx(
            |e| matches!(e, ModEffect::DamagePerWeapon(d, _, _) if d == content::DEATH_ENGINE as i64),
        );
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
    fn damage_per_maxhp_scales_with_live_max_hp() {
        // Mastercrafted Masonry: +1% damage per 2000 Max HP, resolved LIVE from the
        // current tank.max_hp (never baked). Buy it, then verify the dynamic add
        // tracks max_hp — including Max-HP bought AFTER the scaler.
        let mut s = ArenaState::new(1, 0);
        let idx = modifier_idx(|e| matches!(e, ModEffect::DamagePerMaxHp(..)));
        let max_before = s.tank.max_hp;
        s.buy_modifier(idx); // grants +5000 Max HP and the +1%/2000 scaler
        let add = s.modifiers.dynamic_global_add(&s.tank, &s.economy);
        // bonus = (max_hp / 2000) × 1% ; at the post-buy max_hp.
        let expect = Fixed::from_ratio(s.tank.max_hp, 2000).mul(Fixed::from_ratio(1, 100));
        assert_eq!(add, expect, "per-MaxHp add tracks the live max_hp");
        assert!(
            s.tank.max_hp >= max_before + 5000,
            "flat Max HP applied too"
        );

        // Buying more Max HP afterwards retroactively raises the bonus (live, not baked).
        let before = s.modifiers.dynamic_global_add(&s.tank, &s.economy);
        s.modifiers
            .apply_effect(ModEffect::MaxHp(20000), &mut s.economy, &mut s.tank);
        let after = s.modifiers.dynamic_global_add(&s.tank, &s.economy);
        assert!(
            after > before,
            "later Max-HP buys boost the per-MaxHp damage"
        );
    }

    #[test]
    fn damage_per_bounty_scales_with_live_bounty() {
        // Golden Ring: +1% damage per 50% Kill Bounty, from the live bounty_mult.
        let mut s = ArenaState::new(1, 0);
        let idx = modifier_idx(|e| matches!(e, ModEffect::DamagePerBountyPct(..)));
        s.buy_modifier(idx); // +200% bounty (mult 1.0→3.0) and the +1%/50% scaler
        let add = s.modifiers.dynamic_global_add(&s.tank, &s.economy);
        // bonus above base = 2.0 ; in 50%-units = ×2 → 4 units × 1% = +4%.
        let bonus = s.economy.bounty_mult - Fixed::ONE;
        let expect = Fixed::from_ratio(1, 100).mul(bonus.mul(Fixed::from_int(2)));
        assert_eq!(add, expect, "per-Bounty add tracks the live bounty_mult");
        let v = add.scale_i64(1000);
        assert!((39..=40).contains(&v), "≈+4% with +200% bounty, got {v}");

        // More bounty afterwards raises the bonus.
        let before = s.modifiers.dynamic_global_add(&s.tank, &s.economy);
        s.modifiers
            .apply_effect(ModEffect::BountyPct(100, 100), &mut s.economy, &mut s.tank);
        let after = s.modifiers.dynamic_global_add(&s.tank, &s.economy);
        assert!(
            after > before,
            "later bounty buys boost the per-Bounty damage"
        );
    }

    #[test]
    fn shield_active_damage_applies_only_while_shield_up() {
        // Arcane Mark: +20% damage WHILE the Mana Shield is active (mana_shield > 0).
        let mut s = ArenaState::new(1, 0);
        let idx = modifier_idx(|e| matches!(e, ModEffect::ShieldActiveDamagePct(..)));
        s.buy_modifier(idx); // +4000 shield pool + the conditional +20%
        assert!(s.tank.mana_shield > 0, "Arcane Mark grants a shield pool");
        let up = s.modifiers.dynamic_global_add(&s.tank, &s.economy);
        // 0.20 isn't exactly representable; Fixed floors deterministically (≈199).
        assert!(
            (199..=200).contains(&up.scale_i64(1000)),
            "≈+20% while shield up"
        );
        // Drop the shield: the bonus disappears.
        s.tank.mana_shield = 0;
        let down = s.modifiers.dynamic_global_add(&s.tank, &s.economy);
        assert_eq!(down, Fixed::ZERO, "no bonus while shield is down");
    }

    #[test]
    fn ramp_apply_is_idempotent_between_intervals() {
        let idx = content::MODIFIERS
            .iter()
            .position(|md| md.ramp.is_some())
            .unwrap() as u16;
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

    // ---- EXPANSION E3: fidelity mechanics ------------------------------------

    #[test]
    fn grant_free_rerolls_tops_up_the_shop_counter() {
        let (mut m, mut e, mut t) = parts();
        let before = e.rerolls_remaining;
        m.apply_effect(ModEffect::GrantFreeRerolls(3), &mut e, &mut t);
        assert_eq!(e.rerolls_remaining, before + 3, "free rerolls granted");
    }

    #[test]
    fn deep_freeze_catalog_entry_arms_the_tank_flag() {
        let mut s = ArenaState::new(1, 0);
        assert!(!s.tank.deep_freeze, "baseline: no Deep Freeze");
        let idx = modifier_idx(|e| matches!(e, ModEffect::GrantDeepFreeze));
        assert_eq!(content::MODIFIERS[idx as usize].name, "Deep Freeze");
        s.buy_modifier(idx);
        assert!(s.tank.deep_freeze, "purchase arms the freeze payoff");
    }

    #[test]
    fn frost_fire_and_combustion_effects_accumulate() {
        let (mut m, mut e, mut t) = parts();
        m.apply_effect(ModEffect::FrostDamagePct(1, 4), &mut e, &mut t);
        m.apply_effect(ModEffect::FrostDamagePct(1, 4), &mut e, &mut t);
        assert_eq!(m.frost_strength_mult.scale_i64(1000), 1500, "+25% +25%");
        m.apply_effect(ModEffect::FireDamagePct(1, 2), &mut e, &mut t);
        assert_eq!(m.fire_dmg_mult.scale_i64(1000), 1500);
        m.apply_effect(ModEffect::CombustionPct(1, 1), &mut e, &mut t);
        assert_eq!(
            m.fire_explosion_mult.scale_i64(1000),
            2000,
            "Combustion alone"
        );
        m.apply_effect(ModEffect::BounceBarragePct(1, 4), &mut e, &mut t);
        assert_eq!(m.bounce_barrage_pct.scale_i64(1000), 250);
    }

    #[test]
    fn healthy_damage_applies_only_at_95_percent_hp_or_above() {
        let mut s = ArenaState::new(1, 0);
        s.modifiers.apply_effect(
            ModEffect::DamageWhileHealthyPct(35, 100),
            &mut s.economy,
            &mut s.tank,
        );
        s.tank.max_hp = 10_000;
        s.tank.hp = 10_000; // full
        let full = s.modifiers.dynamic_global_add(&s.tank, &s.economy);
        assert!(
            (349..=350).contains(&full.scale_i64(1000)),
            "≈+35% while at/above 95%"
        );
        s.tank.hp = 9_500; // exactly 95%
        assert_eq!(
            s.modifiers.dynamic_global_add(&s.tank, &s.economy),
            full,
            "boundary (95%) still counts"
        );
        s.tank.hp = 9_499; // below the threshold
        assert_eq!(
            s.modifiers.dynamic_global_add(&s.tank, &s.economy),
            Fixed::ZERO,
            "bonus gone below 95%"
        );
    }

    // ---- Battle Fervor: healing-weapon-scoped healthy damage -----------------

    #[test]
    fn catalog_healing_weapon_classification_is_pinned() {
        // THE healing-weapon set (`WeaponDef::is_healing`) — Battle Fervor's
        // +35% scope. If a weapon is added/removed here, that is a deliberate
        // balance decision, not an accident: update this pin consciously.
        let healing: Vec<&str> = content::WEAPONS
            .iter()
            .filter(|w| w.is_healing())
            .map(|w| w.name)
            .collect();
        assert_eq!(
            healing,
            [
                "Suckula",
                "Lifeleecher",
                "Chaotic Spirit Bolt",
                "Healthstone",
                "Holy Bolt",
                "Mendweaver",
                "Healing Sprayer",
            ],
            "healing-weapon classification changed"
        );
    }

    #[test]
    fn healing_weapon_healthy_damage_gates_on_weapon_class_and_hp() {
        let mut s = ArenaState::new(1, 0);
        s.modifiers.apply_effect(
            ModEffect::HealingWeaponDamagePct(35, 100),
            &mut s.economy,
            &mut s.tank,
        );
        let healer = content::WEAPONS
            .iter()
            .find(|w| w.name == "Holy Bolt")
            .unwrap();
        let plain = content::WEAPONS.iter().find(|w| w.name == "Bow").unwrap();
        assert!(healer.is_healing() && !plain.is_healing());
        s.tank.max_hp = 10_000;
        s.tank.hp = 10_000; // full
        let full = s.modifiers.healing_weapon_add(healer, &s.tank);
        assert!(
            (349..=350).contains(&full.scale_i64(1000)),
            "≈+35% for a healing weapon at/above 95%"
        );
        assert_eq!(
            s.modifiers.healing_weapon_add(plain, &s.tank),
            Fixed::ZERO,
            "non-healing weapon never gets the scoped bonus"
        );
        s.tank.hp = 9_500; // exactly 95% — same gate as DamageWhileHealthyPct
        assert_eq!(
            s.modifiers.healing_weapon_add(healer, &s.tank),
            full,
            "boundary (95%) still counts"
        );
        s.tank.hp = 9_499; // below the threshold: neither weapon class
        assert_eq!(
            s.modifiers.healing_weapon_add(healer, &s.tank),
            Fixed::ZERO,
            "bonus gone below 95%"
        );
        assert_eq!(s.modifiers.healing_weapon_add(plain, &s.tank), Fixed::ZERO);
        // The scoped bonus never leaks into the GLOBAL dynamic add.
        s.tank.hp = 10_000;
        assert_eq!(
            s.modifiers.dynamic_global_add(&s.tank, &s.economy),
            Fixed::ZERO,
            "healing-weapon bonus is not a global add"
        );
    }

    #[test]
    fn battle_fervor_scopes_its_damage_to_healing_weapons() {
        // Catalog wiring: +50% Healing (unchanged) + the SCOPED +35% — the
        // global healthy_dmg aggregate stays untouched.
        let mut s = ArenaState::new(1, 0);
        let idx = content::MODIFIERS
            .iter()
            .position(|m| m.name == "Battle Fervor")
            .expect("Battle Fervor exists") as u16;
        let healing0 = s.tank.healing_mult;
        s.buy_modifier(idx);
        assert_eq!(
            s.tank.healing_mult,
            healing0 + Fixed::from_ratio(50, 100),
            "+50% Healing half unchanged"
        );
        assert_eq!(
            s.modifiers.healing_weapon_healthy_dmg,
            Fixed::from_ratio(35, 100),
            "+35% rides the healing-weapon-scoped aggregate"
        );
        assert_eq!(
            s.modifiers.healthy_dmg,
            Fixed::ZERO,
            "no global healthy-damage from Battle Fervor anymore"
        );
    }

    #[test]
    fn status_armor_and_spikes_rider_effects_wire_their_tank_fields() {
        let (mut m, mut e, mut t) = parts();
        m.apply_effect(ModEffect::FrostArmor(2), &mut e, &mut t);
        m.apply_effect(ModEffect::FrostArmor(2), &mut e, &mut t);
        assert_eq!(t.retaliate_frost, 4, "frost armor stacks accumulate");
        m.apply_effect(ModEffect::FireArmor(20), &mut e, &mut t);
        assert_eq!(t.retaliate_fire, 20);
        m.apply_effect(ModEffect::SpikesFirstHit(240), &mut e, &mut t);
        assert_eq!(t.spikes_first_hit, 240);
        m.apply_effect(ModEffect::SpikesAsDrPct(1, 20), &mut e, &mut t);
        assert_eq!(t.spikes_dr_rate, Fixed::from_ratio(1, 20));
        m.apply_effect(ModEffect::DamageTakenToSpikesPct(3, 10), &mut e, &mut t);
        assert_eq!(t.dmg_taken_to_spikes, Fixed::from_ratio(3, 10));
    }

    #[test]
    fn per_weapon_count_scaler_supports_plus_100_percent_per_copy() {
        // The Chaos Orb / Magic Missile / Throwing Axes pattern ("+100% X per
        // copy of Y") rides the existing DamagePerWeapon mechanism: per = 100.
        let mut s = ArenaState::new(1, 0);
        s.weapons.clear();
        let missile = content::WEAPONS
            .iter()
            .position(|w| w.name == "Magic Missile")
            .unwrap() as u16;
        s.modifiers.apply_effect(
            ModEffect::DamagePerWeapon(missile as i64, content::DMG_MAGIC as i64, 100),
            &mut s.economy,
            &mut s.tank,
        );
        for _ in 0..2 {
            let id = s.alloc_entity_id();
            s.weapons.push(crate::state::WeaponInstance {
                instance_id: id,
                def: missile,
                next_fire_tick: 0,
            });
        }
        let add = s.modifiers.self_scaling_add(content::DMG_MAGIC, &s.weapons);
        assert_eq!(add, Fixed::from_int(2), "+100% per copy × 2 copies = +200%");
    }

    // ---- EXPANSION E2: catalog wiring ---------------------------------------

    #[test]
    fn e2_catalog_entries_wire_their_tank_fields() {
        // Energy Pulse arms the shield-break stun (range 1200, 15 ticks) + a shield.
        let mut s = ArenaState::new(1, 0);
        let ep = modifier_idx(|e| matches!(e, ModEffect::ShieldBreakStun(..)));
        s.buy_modifier(ep);
        assert_eq!(s.tank.shieldbreak_stun_range, 1200);
        assert_eq!(s.tank.shieldbreak_stun_ticks, 15);
        assert!(s.tank.mana_shield > 0, "Energy Pulse grants a shield pool");

        // Poison Armor arms the spikes poison DoT (+ armor + flat spikes).
        let pa = modifier_idx(|e| matches!(e, ModEffect::SpikesPoison(..)));
        let mut s = ArenaState::new(1, 0);
        let armor0 = s.tank.armor;
        s.buy_modifier(pa);
        assert_eq!(s.tank.spikes_poison_dps, 2);
        assert_eq!(s.tank.spikes_poison_ticks, 90);
        assert!(s.tank.armor > armor0 && s.tank.spikes_damage > 0);

        // Bloody Spikes arms the stacking spikes (per-stack 20, cap 25).
        let bs = modifier_idx(|e| matches!(e, ModEffect::StackingSpikes(..)));
        let mut s = ArenaState::new(1, 0);
        s.buy_modifier(bs);
        assert_eq!(s.tank.spikes_stack_per, 20);
        assert_eq!(s.tank.spikes_stacks_max, 25);
        assert!(s.tank.spikes_damage >= 80, "flat spikes applied too");

        // Blight Aura arms the periodic AoE (range 600, cadence 30, dmg 200) + poison.
        let ba = modifier_idx(|e| matches!(e, ModEffect::DamageAura(..)));
        let mut s = ArenaState::new(1, 0);
        s.buy_modifier(ba);
        assert_eq!(s.tank.aura_range, 600);
        assert_eq!(s.tank.aura_cadence, 30);
        assert_eq!(s.tank.aura_damage, 200);
        assert_eq!(s.tank.aura_poison_dps, 2);
        assert_eq!(s.tank.aura_poison_ticks, 90);
        assert_eq!(s.tank.aura_tick, 0, "cadence counter starts at 0");
    }
}
