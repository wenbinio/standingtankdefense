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

/// Hard i64 backstop for the compounding percentage riders (`MaxHpPct` /
/// `HpRegenPct` / `ManaRegenPct`). Each such rider multiplies its stat by ~1.25 per
/// purchase, so before the soft caps below a pathological repeat-buy stack could run
/// a stat to i64 overflow and poison every downstream `hp + x`. Since the soft caps
/// landed this is belt-and-braces: the measured max Max HP over 80 seeds is now
/// ~434k, seven orders of magnitude under this ceiling, so the clamp never trips.
/// Kept anyway — it is free, bit-stable across platforms, and guards any future
/// effect that writes these stats without going through [`add_soft_capped`].
const STAT_CEIL: i64 = 1_000_000_000_000;

// ===================== Power-curve bounding (docs/11 §11.2) =====================
//
// Multiplicative stacking is the intended engine (`[02] §2.1`) — the problem was
// never that it is strong, it is that it was UNBOUNDED, and specifically that it is
// unbounded in the TAIL. Measured over 80 seeds (35-min horizon) the distribution
// of peak Max HP is:
//
//     p50  24,000   (the STARTING value — over half of all runs never buy Max HP)
//     p90 117,000
//     p99  3.72e9   ← the whole problem lives here
//
// So this is a tail problem, not a central-tendency problem. A flat nerf or a low
// hard cap would flatten a fantasy that the median run is not even having. What is
// wanted is something invisible at p50–p90 and brutal at p99. Hence a SOFT CAP with
// a CUBIC tail, with every knee placed just above the measured p90:
//
//     effective_increment = raw_increment                        while stat ≤ SOFT
//     effective_increment = raw_increment × (SOFT / stat)³        while stat > SOFT
//
// Properties that matter here:
//   * CONTINUOUS at the knee (the factor is exactly 1 when `stat == SOFT`), so the
//     curve flattens rather than cliffs — no purchase ever becomes worthless, and
//     more is always strictly more.
//   * Below the knee NOTHING changes, bit for bit. Nine runs in ten are unaffected.
//   * Above the knee the stat grows like `SOFT × (1 + ¾n)^⅓` in the number of
//     purchases instead of `×1.25ⁿ`. Sixty compounding buys move Max HP from 130k
//     to ~470k rather than to 3.7 billion.
//
// Pure integer / Fixed math with an i128 intermediate — no floats, bit-stable
// across platforms, and it feeds `state_checksum`.

/// Falloff exponent above a knee. 3 = cubic. Raising it bites the tail harder
/// without moving anything below the knee; the i128 intermediates below are sized
/// for 3 (see [`soft_capped_inc`]).
const SOFT_FALLOFF_POW: u32 = 3;

/// Soft-cap knee for **Max HP**, flat and `%` sources alike. Measured p90 is
/// 117,000, so this sits just clear of nine runs in ten while cutting the p99 tail
/// (3.72e9) by four orders of magnitude.
const MAX_HP_SOFT: i64 = 130_000;
/// Soft-cap knee for **HP regen**, in HP per tick. Measured p90 is 14,670/tick;
/// the p99 was 284,428/tick (8.5M HP/s), which is unloseable by construction.
const HP_REGEN_SOFT: i64 = 16_000;
/// Soft-cap knee for **Mana-Shield regen**, in shield per tick. Measured p90 1,720,
/// p99 569,275.
const MANA_REGEN_SOFT: i64 = 2_000;
/// Soft-cap knee for the **Mana-Shield pool** — a second HP bar, so it gets the
/// same treatment as Max HP. Measured p90 94,000, p99 970,200.
const MANA_SHIELD_SOFT: i64 = 100_000;
/// Soft-cap knee for **Armor**. Armor is flat subtraction, so an unbounded value is
/// literal invulnerability. Measured p90 660, p99 17,240.
const ARMOR_SOFT: i64 = 1_000;
/// Soft-cap knee for the multiplicative damage product `mul_global`, as a raw
/// `Fixed` (×8). Measured p90 is ×1.0 — nine runs in ten never buy a single
/// multiplicative damage source — while p99 reached ×4.18e8. Nine `+25% Damage
/// (Epic)` purchases land at the knee at full value; past it each further
/// multiplicative source contributes a shrinking factor.
const DAMAGE_MUL_SOFT: Fixed = Fixed::from_raw(8 << 16);

/// `raw_inc × (soft / |stat|)^SOFT_FALLOFF_POW`, returned unchanged below the knee.
/// Integer only (i128 intermediate; the result is always ≤ `raw_inc` in magnitude,
/// so narrowing back to `i64` cannot overflow). `soft` must be > 0.
///
/// Worst-case intermediate: `|raw_inc| ≤ 0.25·STAT_CEIL = 2.5e11` and the largest
/// knee is 1e5, so the numerator peaks near `2.5e11 × 1e15 = 2.5e26`, twelve orders
/// of magnitude under `i128::MAX`.
fn soft_capped_inc(raw_inc: i64, stat: i64, soft: i64) -> i64 {
    let mag = i128::from(stat.unsigned_abs());
    let s = soft as i128;
    if mag <= s {
        return raw_inc;
    }
    let mut num = raw_inc as i128;
    let mut den: i128 = 1;
    for _ in 0..SOFT_FALLOFF_POW {
        num *= s;
        den *= mag;
    }
    (num / den) as i64
}

/// Apply `raw_inc` to `stat` through the soft cap, clamp to the `STAT_CEIL`
/// backstop, and return the increment that was actually applied (so a caller can
/// keep a paired value — current HP behind max HP, current shield behind its pool
/// — exactly in step). Reductions (`raw_inc < 0`) are attenuated the same way,
/// which keeps the function monotone and its inverse well behaved.
fn add_soft_capped(stat: &mut i64, raw_inc: i64, soft: i64) -> i64 {
    let inc = soft_capped_inc(raw_inc, *stat, soft);
    let next = stat.saturating_add(inc).clamp(-STAT_CEIL, STAT_CEIL);
    let applied = next - *stat;
    *stat = next;
    applied
}

/// `Fixed` twin of [`soft_capped_inc`], for the multiplicative damage product.
/// Attenuates the *added* factor (the `+25%` of a `×1.25` source), never the
/// product already banked, so the curve is monotone and continuous at the knee.
fn soft_capped_factor(factor: Fixed, current: Fixed, soft: Fixed) -> Fixed {
    if current <= soft {
        return factor;
    }
    let s = soft.raw() as i128;
    let c = current.raw() as i128;
    let mut num = factor.raw() as i128;
    let mut den: i128 = 1;
    for _ in 0..SOFT_FALLOFF_POW {
        num *= s;
        den *= c;
    }
    Fixed::from_raw((num / den) as i64)
}

// ===================== Arsenal breadth synergy (docs/11 §11.2) =====================
//
// Measured failure: a sampled run's arsenal was `Magic Bolt ×14` — one weapon
// bought fourteen times. Copies stacked cleanly and NOTHING rewarded breadth, so
// an 86-weapon catalog collapsed to "find the best, buy it repeatedly".
//
// Note the shape of the old incentive: an extra copy and an extra DISTINCT weapon
// were exactly equivalent — both just add one more firing unit. Breadth was not
// worse, it was *neutral*, and a tie is decided by whichever weapon happens to be
// cheapest or strongest. So the cheapest possible thumb on the scale flips it.
//
// This is deliberately a BONUS, not a nerf on copies: your fourteenth Magic Bolt is
// worth exactly what it always was. A build that covers more damage types / attack
// classes / distinct weapons simply gets a global additive damage bonus on top.
// The bonus is derived LIVE from the owned arsenal (never baked at purchase), so
// selling into or growing out of breadth tracks immediately.
//
//   synergy% = 10% × (distinct damage types − 1)            capped at 4 extra (+40%)
//            +  8% × (distinct attack classes − 1)          capped at 5 extra (+40%)
//            +  5% × (distinct weapon defs − 1)             capped at 8 extra (+40%)
//
// Maximum +120%, reached only by a genuinely wide build (all 5 damage types, all 6
// attack classes, 9+ distinct weapons). `Magic Bolt ×14` scores exactly +0%.
// It is GLOBAL ADDITIVE and folds in through `self_scaling_add` — i.e. it lands in
// the same additive pool as `add_global`/`add_self`/`add_dyn` and is then multiplied
// by `mul_global`, exactly like every other additive source (`[02] §2.1`). The
// composition ORDER is unchanged.

/// Additive % per distinct damage type beyond the first, as `(num, den)`.
const SYNERGY_PER_TYPE: (i64, i64) = (10, 100);
/// Max extra damage types counted (5 types ⇒ 4 beyond the first).
const SYNERGY_TYPE_CAP: i64 = 4;
/// Additive % per distinct attack class beyond the first, as `(num, den)`.
const SYNERGY_PER_CLASS: (i64, i64) = (8, 100);
/// Max extra attack classes counted (6 classes ⇒ 5 beyond the first).
const SYNERGY_CLASS_CAP: i64 = 5;
/// Additive % per distinct weapon def beyond the first, as `(num, den)`.
const SYNERGY_PER_DEF: (i64, i64) = (5, 100);
/// Max extra distinct weapon defs counted.
const SYNERGY_DEF_CAP: i64 = 8;
/// Bitset width for the distinct-weapon-def count (86 defs today; the guard below
/// keeps the count exact as the catalog grows and is a no-op if it ever exceeds
/// this, which would only under-count, never panic).
const DEF_BITSET_WORDS: usize = 4;

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
            dmg_per_maxhp_rate: Fixed::ZERO,
            dmg_per_bounty_rate: Fixed::ZERO,
            shield_active_dmg: Fixed::ZERO,
        }
    }

    /// Live GLOBAL additive damage bonus from the three DYNAMIC scalers, evaluated
    /// against the current `tank`/`economy` (NOT baked at purchase) — the offensive
    /// analogue of `self_scaling_add`. Folded into the per-weapon multiplier at fire
    /// time exactly like `self_scaling_add` (additive, then `×mul_global`):
    ///   * Mastercrafted Masonry: `rate × (max_hp / 2000)`.
    ///   * Golden Ring: `rate × ((bounty_mult − 1) / 0.5)`  ( = rate × 2 × bonus ).
    ///   * Arcane Mark: `shield_active_dmg` while `mana_shield > 0`, else nothing.
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
        add
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

    /// GLOBAL additive damage % earned by covering multiple damage types / attack
    /// classes / distinct weapons — the breadth incentive (`docs/11 §11.2`). A pure
    /// function of the owned arsenal: no state, no RNG, resolved live at fire time
    /// exactly like [`Modifiers::self_scaling_add`].
    ///
    /// ```text
    /// synergy = 0.10 × min(distinct_damage_types  − 1, 4)
    ///         + 0.08 × min(distinct_attack_classes − 1, 5)
    ///         + 0.05 × min(distinct_weapon_defs    − 1, 8)
    /// ```
    ///
    /// An empty arsenal scores `ZERO` (each `distinct − 1` term is clamped at 0).
    /// Deliberately a bonus and never a penalty: stacking copies is worth exactly
    /// what it always was; breadth is simply worth more.
    pub fn arsenal_synergy_add(weapons: &[crate::state::WeaponInstance]) -> Fixed {
        if weapons.is_empty() {
            return Fixed::ZERO;
        }
        let mut type_mask: u32 = 0;
        let mut class_mask: u32 = 0;
        let mut def_mask = [0u64; DEF_BITSET_WORDS];
        for w in weapons {
            let wd = content::WEAPONS[w.def as usize];
            type_mask |= 1u32 << (wd.damage_type as u32 % 5);
            class_mask |= 1u32 << (content::attack_scope_id(wd.attack) as u32 % 6);
            let bit = w.def as usize;
            if bit < DEF_BITSET_WORDS * 64 {
                def_mask[bit / 64] |= 1u64 << (bit % 64);
            }
        }
        let types = i64::from(type_mask.count_ones());
        let classes = i64::from(class_mask.count_ones());
        let defs: i64 = def_mask.iter().map(|w| i64::from(w.count_ones())).sum();

        let extra = |n: i64, cap: i64| (n - 1).clamp(0, cap);
        let mut add = Fixed::ZERO;
        add += Fixed::from_ratio(
            SYNERGY_PER_TYPE.0 * extra(types, SYNERGY_TYPE_CAP),
            SYNERGY_PER_TYPE.1,
        );
        add += Fixed::from_ratio(
            SYNERGY_PER_CLASS.0 * extra(classes, SYNERGY_CLASS_CAP),
            SYNERGY_PER_CLASS.1,
        );
        add += Fixed::from_ratio(
            SYNERGY_PER_DEF.0 * extra(defs, SYNERGY_DEF_CAP),
            SYNERGY_PER_DEF.1,
        );
        add
    }

    /// Additive self-scaling % for a weapon of `dmg_type`, given the owned
    /// weapons: `Σ rule.per × (count of rule.weapon_def)` over rules matching the
    /// type, PLUS the arsenal-breadth synergy (global, type-independent — see
    /// [`Modifiers::arsenal_synergy_add`]). Resolved live at fire time (it depends
    /// on the current arsenal).
    ///
    /// Both terms are ADDITIVE and land in the caller's single additive pool before
    /// the `× mul_global` step, so the composition order is exactly as before:
    /// `(1 + add_static + add_self + add_dyn) × mul_global`. The synergy rides here
    /// (rather than in `weapon_damage_mult`) because this is the only fire-time hook
    /// that is handed the owned arsenal.
    pub fn self_scaling_add(&self, dmg_type: u8, weapons: &[crate::state::WeaponInstance]) -> Fixed {
        self.self_scaling_add_with(dmg_type, weapons, Self::arsenal_synergy_add(weapons))
    }

    /// [`Modifiers::self_scaling_add`] with the arsenal-breadth term supplied by
    /// the caller. The synergy is a pure function of `weapons` and the arsenal
    /// cannot change inside one `fire_weapons` call, so the caller computes the
    /// distinct-count ONCE per tick instead of once per damage application. Purely
    /// a hoist: for `synergy == arsenal_synergy_add(weapons)` this is
    /// bit-identical to `self_scaling_add`, and the value lives in a local — it
    /// never enters `ArenaState`, so it never enters the checksum.
    pub fn self_scaling_add_with(
        &self,
        dmg_type: u8,
        weapons: &[crate::state::WeaponInstance],
        synergy: Fixed,
    ) -> Fixed {
        let mut add = synergy;
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
            // SOFT-CAPPED multiplicative source. The added factor (the `+0.25` of a
            // `×1.25` source) is attenuated by `(DAMAGE_MUL_SOFT / mul_global)²` once
            // the banked product passes ×8; below that it is untouched. Continuous at
            // the knee, monotone, and pure integer (see `soft_capped_factor`).
            ModEffect::DamageMulPct(n, d) => {
                let f = soft_capped_factor(
                    Fixed::from_ratio(n, d),
                    self.mul_global,
                    DAMAGE_MUL_SOFT,
                );
                self.mul_global = self.mul_global.mul(Fixed::ONE + f)
            }
            ModEffect::AttackSpeedPct(n, d) => self.attack_speed += Fixed::from_ratio(n, d),
            ModEffect::BountyPct(n, d) => economy.bounty_mult += Fixed::from_ratio(n, d),
            ModEffect::IncomeFlat(f) => economy.income_per_tick += f,
            // Flat Max HP goes through the SAME soft cap as the % rider, so the knee
            // is a property of the STAT, not of one effect. Below it a `+2000 Max HP`
            // is exactly +2000; above it the round-ramping `+500/round` sources stop
            // compounding into the millions.
            ModEffect::MaxHp(f) => {
                let inc = add_soft_capped(&mut tank.max_hp, f, MAX_HP_SOFT);
                tank.hp += inc;
            }
            ModEffect::Armor(a) => {
                add_soft_capped(&mut tank.armor, a, ARMOR_SOFT);
            }
            ModEffect::ManaShield(pool, regen) => {
                let inc = add_soft_capped(&mut tank.mana_shield_max, pool, MANA_SHIELD_SOFT);
                tank.mana_shield += inc;
                add_soft_capped(&mut tank.mana_regen_per_tick, regen, MANA_REGEN_SOFT);
            }
            ModEffect::HpRegen(r) => {
                add_soft_capped(&mut tank.hp_regen_per_tick, r, HP_REGEN_SOFT);
            }
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
            // DYNAMIC global-damage scalers (resolved live at fire time via
            // `dynamic_global_add`, mirroring `DamagePerWeapon`/`self_scaling_add`).
            // Accumulate only the per-unit RATE / flat bonus here.
            ModEffect::DamagePerMaxHp(n, d) => self.dmg_per_maxhp_rate += Fixed::from_ratio(n, d),
            ModEffect::DamagePerBountyPct(n, d) => self.dmg_per_bounty_rate += Fixed::from_ratio(n, d),
            ModEffect::ShieldActiveDamagePct(n, d) => self.shield_active_dmg += Fixed::from_ratio(n, d),
            ModEffect::GoldPerDamagePct(n, d) => economy.gold_per_damage += Fixed::from_ratio(n, d),
            ModEffect::IncomeShieldPct(n, d) => economy.income_shield_pct += Fixed::from_ratio(n, d),
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
            // Each of the three is additionally SOFT-CAPPED (see `soft_capped_inc`):
            // below the knee the rider applies at full strength (so no ordinary build
            // changes at all), above it the increment decays as `(SOFT/stat)²` and the
            // stat grows ~√n instead of exponentially. `STAT_CEIL` stays as the hard
            // i64 backstop but is now unreachable in practice.
            ModEffect::MaxHpPct(n, d) => {
                let raw = Fixed::from_ratio(n, d).scale_i64(tank.max_hp);
                let inc = add_soft_capped(&mut tank.max_hp, raw, MAX_HP_SOFT);
                tank.hp += inc;
            }
            ModEffect::HpRegenPct(n, d) => {
                let raw = Fixed::from_ratio(n, d).scale_i64(tank.hp_regen_per_tick);
                add_soft_capped(&mut tank.hp_regen_per_tick, raw, HP_REGEN_SOFT);
            }
            ModEffect::ManaRegenPct(n, d) => {
                let raw = Fixed::from_ratio(n, d).scale_i64(tank.mana_regen_per_tick);
                add_soft_capped(&mut tank.mana_regen_per_tick, raw, MANA_REGEN_SOFT);
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
                let existing = tank.spikes_poison_dps.saturating_mul(tank.spikes_poison_ticks as i64);
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
        assert!(s.tank.max_hp >= max_before + 5000, "flat Max HP applied too");

        // Buying more Max HP afterwards retroactively raises the bonus (live, not baked).
        let before = s.modifiers.dynamic_global_add(&s.tank, &s.economy);
        s.modifiers.apply_effect(ModEffect::MaxHp(20000), &mut s.economy, &mut s.tank);
        let after = s.modifiers.dynamic_global_add(&s.tank, &s.economy);
        assert!(after > before, "later Max-HP buys boost the per-MaxHp damage");
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
        s.modifiers.apply_effect(ModEffect::BountyPct(100, 100), &mut s.economy, &mut s.tank);
        let after = s.modifiers.dynamic_global_add(&s.tank, &s.economy);
        assert!(after > before, "later bounty buys boost the per-Bounty damage");
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
        assert!((199..=200).contains(&up.scale_i64(1000)), "≈+20% while shield up");
        // Drop the shield: the bonus disappears.
        s.tank.mana_shield = 0;
        let down = s.modifiers.dynamic_global_add(&s.tank, &s.economy);
        assert_eq!(down, Fixed::ZERO, "no bonus while shield is down");
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

    // ---- Power-curve soft caps (docs/11 §11.2) -------------------------------

    #[test]
    fn soft_cap_is_identity_below_the_knee() {
        // The knee is placed above the measured p90 of every stat, so ordinary
        // play must be bit-for-bit unchanged.
        assert_eq!(soft_capped_inc(500, 0, MAX_HP_SOFT), 500);
        assert_eq!(soft_capped_inc(2000, MAX_HP_SOFT - 1, MAX_HP_SOFT), 2000);
        // Exactly AT the knee the factor is 1 — the curve is continuous there.
        assert_eq!(soft_capped_inc(2000, MAX_HP_SOFT, MAX_HP_SOFT), 2000);
    }

    #[test]
    fn soft_cap_decays_cubically_above_the_knee() {
        let s = MAX_HP_SOFT;
        // At 2× the knee the rider is worth 1/8 (cubic); at 4×, 1/64.
        assert_eq!(soft_capped_inc(8000, 2 * s, s), 1000);
        assert_eq!(soft_capped_inc(64000, 4 * s, s), 1000);
        // Monotone: a bigger raw increment is still a bigger effective increment.
        assert!(soft_capped_inc(9000, 2 * s, s) > soft_capped_inc(8000, 2 * s, s));
        // And more stat always still means more, never less.
        assert!(soft_capped_inc(8000, 2 * s, s) > 0);
    }

    #[test]
    fn max_hp_stops_running_away_but_never_stops_growing() {
        // 200 repeat buys of the `[+2000 flat, +25%]` Imbued Masonry bundle. Before
        // the soft cap this compounded to ~1e12 (the STAT_CEIL backstop); now it has
        // to land in a range a player can still reason about.
        let (mut m, mut e, mut t) = parts();
        let mut prev = t.max_hp;
        for _ in 0..200 {
            m.apply_effect(ModEffect::MaxHp(2000), &mut e, &mut t);
            m.apply_effect(ModEffect::MaxHpPct(25, 100), &mut e, &mut t);
            assert!(t.max_hp > prev, "every purchase is still strictly an upgrade");
            prev = t.max_hp;
        }
        assert!(
            t.max_hp < 1_000_000,
            "200 compounding Max-HP buys stay under 1e6, got {}",
            t.max_hp
        );
        assert!(t.max_hp > MAX_HP_SOFT, "and they do get well past the knee");
        assert_eq!(t.hp, t.max_hp, "current HP tracked every applied increment");
    }

    #[test]
    fn damage_mul_product_is_bounded_but_monotone() {
        let (mut m, mut e, mut t) = parts();
        let mut prev = m.mul_global;
        for _ in 0..250 {
            m.apply_effect(ModEffect::DamageMulPct(1, 4), &mut e, &mut t); // ×1.25 each
            assert!(m.mul_global > prev, "each multiplicative source still helps");
            prev = m.mul_global;
        }
        let v = m.mul_global.scale_i64(1000);
        // Was ×4.2e8 in the measured tail; the soft cap lands it in the tens.
        assert!((8_000..200_000).contains(&v), "expected ×8..×200, got ×{}", v as f64 / 1000.0);
    }

    #[test]
    fn first_nine_multiplicative_sources_are_untouched() {
        // Below the ×8 knee the product must be exactly the old `Π(1 + f)`.
        let (mut m, mut e, mut t) = parts();
        let mut expect = Fixed::ONE;
        for _ in 0..9 {
            m.apply_effect(ModEffect::DamageMulPct(1, 4), &mut e, &mut t);
            expect = expect.mul(Fixed::ONE + Fixed::from_ratio(1, 4));
            if expect > DAMAGE_MUL_SOFT {
                break;
            }
            assert_eq!(m.mul_global, expect, "unchanged below the knee");
        }
    }

    // ---- Arsenal breadth synergy (docs/11 §11.2) -----------------------------

    fn arsenal(defs: &[u16]) -> Vec<crate::state::WeaponInstance> {
        defs.iter()
            .enumerate()
            .map(|(i, &def)| crate::state::WeaponInstance {
                instance_id: crate::EntityId(i as u32 + 1),
                def,
                next_fire_tick: 0,
            })
            .collect()
    }

    /// A weapon def for each of the five damage types (first match in the catalog).
    fn one_per_damage_type() -> Vec<u16> {
        (0..5u8)
            .filter_map(|t| {
                content::WEAPONS.iter().position(|w| w.damage_type == t).map(|i| i as u16)
            })
            .collect()
    }

    #[test]
    fn stacked_copies_earn_no_synergy() {
        let empty = Modifiers::arsenal_synergy_add(&arsenal(&[]));
        assert_eq!(empty, Fixed::ZERO, "no arsenal, no synergy");
        // `Magic Bolt ×14` — the measured degenerate build — scores exactly zero.
        let mono = arsenal(&[content::STARTING_WEAPON; 14]);
        assert_eq!(
            Modifiers::arsenal_synergy_add(&mono),
            Fixed::ZERO,
            "one weapon copied N times is one type, one class, one def"
        );
        // ...and one copy is worth exactly as much as it was: the bonus is never
        // negative, so stacking is not taxed.
        assert_eq!(Modifiers::arsenal_synergy_add(&arsenal(&[content::STARTING_WEAPON])), Fixed::ZERO);
    }

    #[test]
    fn synergy_grows_with_breadth_and_is_capped() {
        let defs = one_per_damage_type();
        assert_eq!(defs.len(), 5, "the catalog covers all five damage types");
        let mut last = Fixed::ZERO;
        for k in 1..=defs.len() {
            let add = Modifiers::arsenal_synergy_add(&arsenal(&defs[..k]));
            assert!(add >= last, "synergy is monotone in breadth");
            last = add;
        }
        assert!(last > Fixed::ZERO, "a five-type arsenal earns a real bonus");
        // Cap: adding a tenth, eleventh… distinct def cannot grow the def term past
        // SYNERGY_DEF_CAP, so the bonus is bounded no matter how wide the build.
        let wide: Vec<u16> = (0..30u16).collect();
        let a = Modifiers::arsenal_synergy_add(&arsenal(&wide));
        let wider: Vec<u16> = (0..60u16).collect();
        let b = Modifiers::arsenal_synergy_add(&arsenal(&wider));
        assert_eq!(a, b, "synergy saturates — it cannot itself become a runaway");
        let max = Fixed::from_ratio(SYNERGY_PER_TYPE.0 * SYNERGY_TYPE_CAP, SYNERGY_PER_TYPE.1)
            + Fixed::from_ratio(SYNERGY_PER_CLASS.0 * SYNERGY_CLASS_CAP, SYNERGY_PER_CLASS.1)
            + Fixed::from_ratio(SYNERGY_PER_DEF.0 * SYNERGY_DEF_CAP, SYNERGY_PER_DEF.1);
        assert!(a <= max, "never exceeds the documented ceiling");
    }

    #[test]
    fn synergy_rides_the_additive_pool_at_fire_time() {
        // It must reach damage through `self_scaling_add` (the only fire-time hook
        // handed the arsenal), additively — so the composition order is unchanged.
        let m = Modifiers::new();
        let defs = one_per_damage_type();
        let wide = arsenal(&defs);
        let mono = arsenal(&[defs[0]; 5]);
        let wide_add = m.self_scaling_add(content::WEAPONS[defs[0] as usize].damage_type, &wide);
        let mono_add = m.self_scaling_add(content::WEAPONS[defs[0] as usize].damage_type, &mono);
        assert_eq!(mono_add, Fixed::ZERO);
        assert_eq!(wide_add, Modifiers::arsenal_synergy_add(&wide));
        assert!(wide_add > mono_add, "breadth beats stacking in the additive pool");
    }

    #[test]
    fn synergy_is_global_not_per_damage_type() {
        // Every weapon in the arsenal benefits, whatever it fires — otherwise the
        // bonus would just be another per-type modifier.
        let m = Modifiers::new();
        let w = arsenal(&one_per_damage_type());
        let base = m.self_scaling_add(0, &w);
        for t in 1..5u8 {
            assert_eq!(m.self_scaling_add(t, &w), base);
        }
    }
}
