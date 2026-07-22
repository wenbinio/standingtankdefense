//! Engine-agnostic **view-model**: a flat, owned snapshot of everything a
//! renderer needs for one frame, derived from the public [`ArenaState`]. It
//! contains no engine types and no `Fixed` — world coordinates are exposed as
//! integer units (`floor_to_int`) so the boundary stays float-free on the sim
//! side; the engine scales to pixels.
//!
//! This is the **contract** the front-end reads each frame (the Godot binding
//! and the text `preview` both consume it), so the determinism core and the
//! renderer can evolve independently. Building a view never mutates the sim and
//! never feeds the checksum.

use crate::content;
use crate::state::{ArenaState, EnemyStatus, DMG_SRC_CLEAR, DMG_SRC_OTHER, DMG_SRC_SPIKES};
use determinism::Fixed;
use std::collections::BTreeMap;

/// A full render snapshot for one tick.
#[derive(Clone, Debug)]
pub struct RenderView {
    pub tick: u32,
    pub round: u32,
    pub dead: bool,
    /// Ticks until `Clear` is ready again (0 ⇒ ready NOW). Pure read of
    /// `tank.clear_cooldown_end` vs the current tick — HUD cooldown indicator.
    pub clear_ready_in: u32,
    /// Full `Clear` cooldown length in ticks (the denominator for a cooldown
    /// fill fraction; constant over a match).
    pub clear_cooldown_total: u32,
    /// Ticks until the next round boundary (= the next shop refresh), always
    /// in `1..=ROUND_TICKS` — HUD round/shop countdown.
    pub ticks_to_next_round: u32,
    pub tank: RenderTank,
    pub enemies: Vec<RenderEnemy>,
    pub projectiles: Vec<RenderProjectile>,
    /// Persistent damaging areas (mines / burning oil) to draw.
    pub hazards: Vec<RenderHazard>,
    /// Summoned allies (Larvae / Spores) to draw.
    pub minions: Vec<RenderMinion>,
    /// Active rotating-wave sweeps to draw (the sector arc around the tank).
    pub sweeps: Vec<RenderSweep>,
    pub economy: RenderEconomy,
    pub shop: Vec<RenderOffer>,
    /// Owned weapons collapsed to `(name, count)`, sorted by name.
    pub arsenal: Vec<RenderArsenalEntry>,
    /// The owned per-weapon-count self-scaling rules ("+X% Piercing per Bow"),
    /// merged per `(source weapon, damage type)` and resolved against the
    /// current arsenal — the HUD's SYNERGIES readout. Pure read of
    /// `Modifiers::weapon_count_scaling` + the owned-weapon counts.
    pub synergies: Vec<RenderSynergy>,
    /// Match scoreboard totals.
    pub stats: RenderStats,
    /// Per-source damage attribution (the DPS-meter rows), in stable
    /// ascending source-id order: real weapons first (catalog order), then the
    /// `DMG_SRC_*` pseudo rows (Spikes / Clear / Other). `Σ total` equals
    /// `stats.damage_dealt` (saturation aside). Pure read of
    /// `ArenaState::damage_by_weapon` + catalog facts.
    pub damage_by_weapon: Vec<RenderDamage>,
}

/// Match-long scoreboard totals (damage dealt / gold earned).
#[derive(Clone, Copy, Debug)]
pub struct RenderStats {
    pub damage_dealt: i64,
    pub gold_earned: i64,
    /// Playstyle telemetry (cosmetic achievements). See `ArenaState`.
    pub bought_attack_mask: u16,
    pub weapons_bought: u32,
    pub economy_purchases: u32,
}

/// World-space integer point (units; tank sits at the origin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderPoint {
    pub x: i64,
    pub y: i64,
}

/// An in-flight projectile to draw. `id` is the stable per-arena entity id
/// (for interpolation across frames); `kind` indexes `content::WEAPONS`
/// (sprite selection); `target_*` is the last known target position (for
/// orienting the sprite along its flight path).
#[derive(Clone, Copy, Debug)]
pub struct RenderProjectile {
    pub id: u32,
    pub kind: u16,
    pub x: i64,
    pub y: i64,
    pub target_x: i64,
    pub target_y: i64,
}

/// A persistent hazard (mine field / burning oil) to draw.
#[derive(Clone, Copy, Debug)]
pub struct RenderHazard {
    pub id: u32,
    pub x: i64,
    pub y: i64,
    pub radius: i64,
    pub ticks_left: u32,
    pub damage_type: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderTank {
    pub x: i64,
    pub y: i64,
    pub hp: i64,
    pub max_hp: i64,
    pub revives: u32,
}

/// A summoned ally to render (`kind`: 0 larvae, 1 spores). `id` is the stable
/// per-arena entity id (for interpolation across frames).
#[derive(Clone, Copy, Debug)]
pub struct RenderMinion {
    pub id: u32,
    pub x: i64,
    pub y: i64,
    pub kind: u8,
}

/// An active rotating-wave sweep (`Attack::WaveRotating`) to draw: an arc
/// sector of `radius` around the tank, currently at `angle_bam` and advancing
/// `step_bam` binary-angle units per tick (65536 = full turn), clockwise or
/// counterclockwise. `weapon_kind` selects the visual; `ticks_left` fades it.
#[derive(Clone, Copy, Debug)]
pub struct RenderSweep {
    pub id: u32,
    pub weapon_kind: u16,
    pub radius: i64,
    pub angle_bam: u16,
    pub step_bam: u16,
    pub clockwise: bool,
    pub ticks_left: u32,
}

/// Per-enemy status-flag bits for [`RenderEnemy::status_flags`] (tints/FX).
pub const STATUS_FROST: u8 = 1 << 0;
pub const STATUS_POISON: u8 = 1 << 1;
pub const STATUS_FIRE: u8 = 1 << 2;
pub const STATUS_VULN: u8 = 1 << 3;
pub const STATUS_STUN: u8 = 1 << 4;
pub const STATUS_FREEZE: u8 = 1 << 5;

/// Collapse live status counters to render flag bits.
fn status_flags(st: &EnemyStatus) -> u8 {
    let mut f = 0;
    if st.frost_stacks > 0 {
        f |= STATUS_FROST;
    }
    if st.poison_ticks > 0 {
        f |= STATUS_POISON;
    }
    if st.fire_stacks > 0 {
        f |= STATUS_FIRE;
    }
    if st.vuln_stacks > 0 {
        f |= STATUS_VULN;
    }
    if st.stun_ticks > 0 {
        f |= STATUS_STUN;
    }
    if st.freeze_ticks > 0 {
        f |= STATUS_FREEZE;
    }
    f
}

#[derive(Clone, Copy, Debug)]
pub struct RenderEnemy {
    pub id: u32,
    pub x: i64,
    pub y: i64,
    pub hp: i64,
    /// Catalog base HP — a denominator for an approximate HP bar.
    pub base_hp: i64,
    pub kind: u16,
    pub boss: bool,
    /// Active status effects as `STATUS_*` flag bits (frost/poison/fire/vuln/
    /// stun/freeze) — drives status tints and looping FX.
    pub status_flags: u8,
    pub name: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderEconomy {
    pub gold: i64,
    /// Effective passive income per tick — the base plus the MULTIPLIED bonus
    /// (income-% applies only to bonus income; source rule, `docs/02 §2.4`).
    pub income_per_tick: i64,
    pub rerolls_remaining: u32,
    pub reroll_cost: i64,
    /// Magic Treasure holding pool (0 = none held): gold accruing +2/s,
    /// banked automatically when the next shop rolls.
    pub treasure_pool: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderOffer {
    pub slot: u8,
    pub name: &'static str,
    pub cost: i64,
    pub is_weapon: bool,
    pub affordable: bool,
    /// 0 common · 1 uncommon · 2 rare · 3 epic (drives the shop frame colour).
    pub rarity: u8,
}

/// One owned weapon stack: `(name, count)` plus the catalog facts the arsenal
/// panel groups by (attack class / damage type / rarity). Pure catalog reads —
/// nothing here is new sim state.
#[derive(Clone, Copy, Debug)]
pub struct RenderArsenalEntry {
    pub name: &'static str,
    pub count: u32,
    /// Weapon catalog index (`content::WEAPONS`).
    pub kind: u16,
    /// Attack-class scope id (`content::attack_scope_id`): 0 Single ·
    /// 1 Splash · 2 Barrage · 3 Area · 4 Wave · 5 Bounce.
    pub class_id: u8,
    /// Damage type: 0 Normal · 1 Piercing · 2 Magic · 3 Siege · 4 Chaos.
    pub damage_type: u8,
    /// 0 common · 1 uncommon · 2 rare · 3 epic (drives the count colour).
    pub rarity: u8,
}

/// One owned per-weapon-count self-scaling rule (a `DamagePerWeapon`-style
/// modifier), resolved against the current arsenal for display. The sim
/// resolves its OWN copy live at fire time (`Modifiers::self_scaling_add`);
/// this mirrors that arithmetic (`per × owned count`) read-only. Percentages
/// are milli-percent integers (1000 = 1%), rounded to nearest — display-only,
/// never fed back into anything.
#[derive(Clone, Copy, Debug)]
pub struct RenderSynergy {
    /// Weapon catalog index whose owned count drives the bonus.
    pub source_kind: u16,
    /// That weapon's display name.
    pub source_name: &'static str,
    /// Damage type the bonus applies to (0..=4, as in `RenderArsenalEntry`).
    pub damage_type: u8,
    /// Bonus per owned copy, milli-percent (1000 ⇒ +1% per copy).
    pub per_copy_milli_pct: i64,
    /// Current owned count of `source_kind`.
    pub count: u32,
    /// Current total bonus, milli-percent (= per-copy × count).
    pub bonus_milli_pct: i64,
}

/// One damage-attribution row (the DPS meter): a weapon's lifetime damage —
/// or a pseudo source's (Spikes / Clear / Other). `name` doubles as the
/// translation key at the render boundary, like every other content name.
#[derive(Clone, Copy, Debug)]
pub struct RenderDamage {
    /// Weapon catalog index, or a `crate::state::DMG_SRC_*` pseudo id.
    pub source: u16,
    /// Catalog display name, or "Spikes" / "Clear" / "Other".
    pub name: &'static str,
    /// The weapon's damage type 0..=4 (row colour); 255 for pseudo rows.
    pub damage_type: u8,
    /// Currently owned copies of the weapon (0 for pseudo rows / sold-out
    /// hypothetical — weapons are never removed today).
    pub count: u32,
    /// Total damage attributed to this source over the match.
    pub total: i64,
}

/// Display-only conversion: a `Fixed` fraction → milli-percent (1000 = 1%),
/// rounded to nearest. Integer math throughout; the result never feeds the
/// checksum or any sim decision.
fn fixed_milli_pct(f: Fixed) -> i64 {
    (f.raw() * 100_000 + (1 << (Fixed::FRAC_BITS - 1))) >> Fixed::FRAC_BITS
}

/// Build a [`RenderView`] from current state. Read-only; allocation-light.
pub fn snapshot(s: &ArenaState) -> RenderView {
    let tank = RenderTank {
        x: s.tank.pos.x.floor_to_int(),
        y: s.tank.pos.y.floor_to_int(),
        hp: s.tank.hp,
        max_hp: s.tank.max_hp,
        revives: s.tank.revives,
    };

    let enemies = s
        .enemies
        .iter()
        .map(|e| {
            let def = &content::ENEMIES[e.def as usize];
            RenderEnemy {
                id: e.id.0,
                x: e.pos.x.floor_to_int(),
                y: e.pos.y.floor_to_int(),
                hp: e.hp,
                base_hp: def.base_hp,
                kind: e.def,
                boss: def.boss,
                status_flags: status_flags(&e.status),
                name: def.name,
            }
        })
        .collect();

    let projectiles = s
        .projectiles
        .iter()
        .map(|p| RenderProjectile {
            id: p.id.0,
            kind: p.weapon_kind,
            x: p.pos.x.floor_to_int(),
            y: p.pos.y.floor_to_int(),
            target_x: p.last_target_pos.x.floor_to_int(),
            target_y: p.last_target_pos.y.floor_to_int(),
        })
        .collect();

    let hazards = s
        .hazards
        .iter()
        .map(|h| RenderHazard {
            id: h.id.0,
            x: h.pos.x.floor_to_int(),
            y: h.pos.y.floor_to_int(),
            radius: h.radius,
            ticks_left: h.ticks_left,
            damage_type: h.damage_type,
        })
        .collect();

    let minions = s
        .minions
        .iter()
        .map(|m| RenderMinion {
            id: m.id.0,
            x: m.pos.x.floor_to_int(),
            y: m.pos.y.floor_to_int(),
            kind: m.kind,
        })
        .collect();

    let sweeps = s
        .sweeps
        .iter()
        .map(|sw| RenderSweep {
            id: sw.id.0,
            weapon_kind: sw.weapon_kind,
            radius: sw.radius.floor_to_int(),
            angle_bam: sw.angle_bam,
            step_bam: sw.step_bam,
            clockwise: sw.clockwise,
            ticks_left: sw.ticks_left,
        })
        .collect();

    let economy = RenderEconomy {
        gold: s.economy.gold,
        income_per_tick: crate::economy::income_award(&s.economy),
        rerolls_remaining: s.economy.rerolls_remaining,
        reroll_cost: s.economy.reroll_cost,
        treasure_pool: s.economy.treasure_pool,
    };

    let shop = s
        .shop
        .offers
        .iter()
        .enumerate()
        .map(|(i, off)| {
            let (name, is_weapon, rarity) = match off.kind {
                crate::state::OfferKind::Weapon => {
                    let w = &content::WEAPONS[off.def as usize];
                    (w.name, true, w.rarity)
                }
                crate::state::OfferKind::Modifier => {
                    let m = &content::MODIFIERS[off.def as usize];
                    (m.name, false, m.rarity)
                }
            };
            RenderOffer {
                slot: i as u8,
                name,
                cost: off.cost,
                is_weapon,
                affordable: off.cost <= s.economy.gold,
                rarity,
            }
        })
        .collect();

    // Keyed `(name, def)` so iteration stays name-sorted (the panel's order)
    // while each stack keeps its catalog identity for grouping/synergies.
    let mut counts: BTreeMap<(&'static str, u16), u32> = BTreeMap::new();
    for w in &s.weapons {
        *counts
            .entry((content::WEAPONS[w.def as usize].name, w.def))
            .or_insert(0) += 1;
    }
    let arsenal: Vec<RenderArsenalEntry> = counts
        .into_iter()
        .map(|((name, def), count)| {
            let w = &content::WEAPONS[def as usize];
            RenderArsenalEntry {
                name,
                count,
                kind: def,
                class_id: content::attack_scope_id(w.attack),
                damage_type: w.damage_type,
                rarity: w.rarity,
            }
        })
        .collect();

    // Owned self-scaling rules, merged per (source weapon, damage type) —
    // buying the same scaler twice stacks additively, exactly as
    // `Modifiers::self_scaling_add` sums the rules at fire time. BTreeMap ⇒
    // stable (source, type) order.
    let mut rules: BTreeMap<(u16, u8), Fixed> = BTreeMap::new();
    for r in &s.modifiers.weapon_count_scaling {
        *rules
            .entry((r.weapon_def, r.dmg_type))
            .or_insert(Fixed::ZERO) += r.per;
    }
    let synergies = rules
        .into_iter()
        .map(|((def, ty), per)| {
            let count = arsenal
                .iter()
                .find(|e| e.kind == def)
                .map_or(0, |e| e.count);
            RenderSynergy {
                source_kind: def,
                source_name: content::WEAPONS[def as usize].name,
                damage_type: ty,
                per_copy_milli_pct: fixed_milli_pct(per),
                count,
                bonus_milli_pct: fixed_milli_pct(per.mul(Fixed::from_int(count as i64))),
            }
        })
        .collect();

    // Damage-attribution rows: the ledger is a BTreeMap, so iteration is
    // already in ascending source order (weapons, then the pseudo ids at the
    // top of the u16 range). Owned counts resolve against the arsenal stacks.
    let damage_by_weapon = s
        .damage_by_weapon
        .iter()
        .map(|(&source, &total)| {
            let (name, damage_type) = match source {
                DMG_SRC_SPIKES => ("Spikes", 255),
                DMG_SRC_CLEAR => ("Clear", 255),
                DMG_SRC_OTHER => ("Other", 255),
                def => {
                    let w = &content::WEAPONS[def as usize];
                    (w.name, w.damage_type)
                }
            };
            let count = arsenal
                .iter()
                .find(|e| e.kind == source)
                .map_or(0, |e| e.count);
            RenderDamage {
                source,
                name,
                damage_type,
                count,
                total,
            }
        })
        .collect();

    RenderView {
        tick: s.tick,
        round: s.round,
        dead: s.dead,
        clear_ready_in: s.tank.clear_cooldown_end.saturating_sub(s.tick),
        clear_cooldown_total: crate::input::CLEAR_COOLDOWN_TICKS,
        ticks_to_next_round: crate::ROUND_TICKS - s.tick % crate::ROUND_TICKS,
        tank,
        enemies,
        projectiles,
        hazards,
        minions,
        sweeps,
        economy,
        shop,
        arsenal,
        synergies,
        stats: RenderStats {
            damage_dealt: s.total_damage_dealt,
            gold_earned: s.total_gold_earned,
            bought_attack_mask: s.bought_attack_mask,
            weapons_bought: s.weapons_bought,
            economy_purchases: s.economy_purchases,
        },
        damage_by_weapon,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{step, Input};

    #[test]
    fn snapshot_reflects_live_state() {
        let mut s = ArenaState::new(0xBEEF, 0);
        // Spawn some action: run a few ticks so enemies appear.
        for _ in 0..40 {
            step(&mut s, Input::Noop);
        }
        let v = snapshot(&s);
        assert_eq!(v.tick, s.tick);
        assert_eq!(v.tank.max_hp, s.tank.max_hp);
        assert_eq!(v.enemies.len(), s.enemies.len());
        assert_eq!(v.shop.len(), s.shop.offers.len());
        // Tank at origin.
        assert_eq!((v.tank.x, v.tank.y), (0, 0));
        // Every enemy carries a real catalog name + base hp denominator.
        for e in &v.enemies {
            assert!(!e.name.is_empty());
            assert!(e.base_hp > 0);
        }
    }

    #[test]
    fn clear_and_round_timing_track_state() {
        let mut s = ArenaState::new(0xBEEF, 0);
        // Fresh arena: Clear ready, a full round ahead.
        let v = snapshot(&s);
        assert_eq!(v.clear_ready_in, 0, "Clear starts ready");
        assert_eq!(v.clear_cooldown_total, crate::input::CLEAR_COOLDOWN_TICKS);
        assert_eq!(v.ticks_to_next_round, crate::ROUND_TICKS);
        // Use Clear: cooldown counts down from the full length.
        step(&mut s, Input::Clear);
        let v = snapshot(&s);
        assert_eq!(
            v.clear_ready_in,
            crate::input::CLEAR_COOLDOWN_TICKS - 1,
            "one tick already elapsed post-step"
        );
        for _ in 0..(crate::input::CLEAR_COOLDOWN_TICKS - 1) {
            step(&mut s, Input::Noop);
        }
        let v = snapshot(&s);
        assert_eq!(v.clear_ready_in, 0, "ready again after the cooldown");
        // Round countdown: always in 1..=ROUND_TICKS and consistent with tick.
        assert_eq!(
            v.ticks_to_next_round,
            crate::ROUND_TICKS - s.tick % crate::ROUND_TICKS
        );
        assert!(v.ticks_to_next_round >= 1 && v.ticks_to_next_round <= crate::ROUND_TICKS);
    }

    #[test]
    fn stats_reflect_scoreboard_totals() {
        let mut s = ArenaState::new(0xBEEF, 0);
        s.total_damage_dealt = 12_345;
        s.total_gold_earned = 6_789;
        let v = snapshot(&s);
        assert_eq!(v.stats.damage_dealt, 12_345);
        assert_eq!(v.stats.gold_earned, 6_789);
    }

    #[test]
    fn arsenal_entries_carry_catalog_facts() {
        let s = ArenaState::new(0xBEEF, 0);
        let v = snapshot(&s);
        // The starting weapon is owned ⇒ exactly one stack, count 1, and its
        // catalog facts round-trip.
        assert_eq!(v.arsenal.len(), 1);
        let e = &v.arsenal[0];
        let def = &content::WEAPONS[e.kind as usize];
        assert_eq!(e.kind, content::STARTING_WEAPON);
        assert_eq!(e.count, 1);
        assert_eq!(e.name, def.name);
        assert_eq!(e.class_id, content::attack_scope_id(def.attack));
        assert_eq!(e.damage_type, def.damage_type);
        assert_eq!(e.rarity, def.rarity);
        assert!(e.class_id <= 5 && e.damage_type <= 4 && e.rarity <= 3);
        // No self-scaling modifiers owned ⇒ no synergies.
        assert!(v.synergies.is_empty());
    }

    #[test]
    fn synergies_resolve_per_times_count() {
        use crate::state::WeaponCountScale;
        use determinism::Fixed;
        let mut s = ArenaState::new(0xBEEF, 0);
        // Own 12 copies of weapon 0 ("+1% Piercing per Bow" territory) by
        // pushing instances directly — a view test, not a shop test.
        s.weapons.clear();
        for _ in 0..12 {
            let id = s.alloc_entity_id();
            s.weapons.push(crate::state::WeaponInstance {
                instance_id: id,
                def: 0,
                next_fire_tick: 0,
            });
        }
        // Two rules for the same (weapon, type) merge additively (1% + 1%);
        // a rule for an unowned weapon resolves to count 0 / bonus 0.
        let per_1pct = Fixed::from_ratio(1, 100);
        s.modifiers.weapon_count_scaling.push(WeaponCountScale {
            weapon_def: 0,
            dmg_type: content::DMG_PIERCING,
            per: per_1pct,
        });
        s.modifiers.weapon_count_scaling.push(WeaponCountScale {
            weapon_def: 0,
            dmg_type: content::DMG_PIERCING,
            per: per_1pct,
        });
        s.modifiers.weapon_count_scaling.push(WeaponCountScale {
            weapon_def: 1,
            dmg_type: content::DMG_SIEGE,
            per: per_1pct,
        });
        let v = snapshot(&s);
        assert_eq!(v.synergies.len(), 2, "merged per (source, type)");
        let bow = &v.synergies[0];
        assert_eq!(bow.source_kind, 0);
        assert_eq!(bow.damage_type, content::DMG_PIERCING);
        assert_eq!(bow.count, 12);
        // 2% per copy × 12 ≈ 24% — milli-percent; `Fixed::from_ratio` floors,
        // so allow a few milli of quantization (the HUD rounds to whole %).
        assert!((bow.per_copy_milli_pct - 2_000).abs() <= 2, "≈2%/copy");
        assert!((bow.bonus_milli_pct - 24_000).abs() <= 24, "≈24%");
        let unowned = &v.synergies[1];
        assert_eq!((unowned.source_kind, unowned.count), (1, 0));
        assert_eq!(unowned.bonus_milli_pct, 0);
    }

    #[test]
    fn shop_affordability_tracks_gold() {
        let mut s = ArenaState::new(7, 0);
        step(&mut s, Input::Noop); // generate offers (round 0 boundary at tick 0)
        s.economy.gold = 0;
        let v = snapshot(&s);
        // Only free items (cost 0) are affordable when broke; the flag must
        // track cost regardless of which offers were drawn.
        assert!(v.shop.iter().all(|o| o.affordable == (o.cost <= 0)));
        s.economy.gold = i64::MAX / 2;
        let v = snapshot(&s);
        assert!(v.shop.iter().all(|o| o.affordable), "rich ⇒ all affordable");
    }
}
