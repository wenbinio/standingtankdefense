//! Theme packs — presentation, never content (`docs/12`).
//!
//! A theme pack maps **catalog index → display strings** and nothing else. It is
//! the whole of `docs/12 §12.2`'s hard rule made structural:
//!
//! - Every field here is a `&'static str`. Nothing in this module is readable
//!   from `step()`, feeds `state_checksum`, or appears in `content.json` — so
//!   adding, editing or swapping a pack cannot move `content_hash` and cannot
//!   invalidate the R3 trace corpus.
//! - Indices are identity. A pack may not add, remove or reorder an entry; the
//!   arrays are fixed-length and a test pins them to the catalog's length.
//! - Flavor may lie to the player. The **tip may not** — it is the mechanical
//!   truth about `content.rs`, in every pack. Balance passes move that truth, so
//!   [`tests::modifier_tips_quote_the_catalog_magnitudes`] and its neighbours
//!   re-derive every quoted magnitude from the catalog rather than trusting it.
//!
//! Two packs ship:
//!
//! | id | register |
//! | --- | --- |
//! | `wardens` | the original grim dark fantasy; the Steam default. Text migrated from `descriptions.rs`, figures kept current with `content.rs`. |
//! | `facility` | the Roblox fork's containment-log deadpan (`docs/12 §12.4`). Original fiction in the genre — see `§12.5`. |
//!
//! ### Display names
//!
//! `wardens` deliberately leaves [`Entry::name`] empty: its display names *are*
//! the catalog names (`Bow`, `Mortar Launcher`, …), and duplicating 177 of them
//! into a second table would only create drift. An empty name therefore falls
//! back to `content.rs` via [`ThemePack::weapon_name`] /
//! [`ThemePack::modifier_name`]; always read names through those, never off the
//! struct field. `facility` names every entry.

pub mod facility;
pub mod wardens;

use crate::content;

/// Weapons covered by every pack. Pinned to `content::WEAPONS.len()` by test.
pub const NUM_WEAPONS: usize = 86;
/// Modifiers covered by every pack. Pinned to `content::MODIFIERS.len()` by test.
pub const NUM_MODIFIERS: usize = 91;

/// One catalog entry's display strings.
///
/// `name` may be empty, meaning "use the catalog name" (see the module docs);
/// `flavor` and `tip` are always written out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub name: &'static str,
    pub flavor: &'static str,
    pub tip: &'static str,
}

impl Entry {
    /// Out-of-range / missing lookups return this rather than panicking.
    pub const EMPTY: Entry = Entry { name: "", flavor: "", tip: "" };

    /// `(flavor, tip)` — the shape `descriptions.rs` has always exposed.
    pub const fn text(self) -> (&'static str, &'static str) {
        (self.flavor, self.tip)
    }
}

/// A complete presentation layer over the weapon + modifier catalog.
pub struct ThemePack {
    /// Stable machine id (`"wardens"`, `"facility"`). Selection key.
    pub id: &'static str,
    /// Human-readable pack name, for a settings menu.
    pub label: &'static str,
    /// Display words for rarity 0..=3, in catalog rarity order. `docs/12 §12.5`
    /// requires the tier vocabulary be invented rather than borrowed.
    pub rarity_tiers: [&'static str; 4],
    pub weapons: &'static [Entry; NUM_WEAPONS],
    pub modifiers: &'static [Entry; NUM_MODIFIERS],
}

impl ThemePack {
    /// Display strings for weapon index `i`; [`Entry::EMPTY`] out of range.
    pub fn weapon(&self, i: u16) -> Entry {
        self.weapons.get(i as usize).copied().unwrap_or(Entry::EMPTY)
    }
    /// Display strings for modifier index `i`; [`Entry::EMPTY`] out of range.
    pub fn modifier(&self, i: u16) -> Entry {
        self.modifiers.get(i as usize).copied().unwrap_or(Entry::EMPTY)
    }

    /// Resolved weapon display name — the pack's, or the catalog's when the pack
    /// declines to rename (see the module docs). `""` out of range.
    pub fn weapon_name(&self, i: u16) -> &'static str {
        let n = self.weapon(i).name;
        if n.is_empty() {
            content::WEAPONS.get(i as usize).map(|w| w.name).unwrap_or("")
        } else {
            n
        }
    }
    /// Resolved modifier display name; see [`weapon_name`](Self::weapon_name).
    pub fn modifier_name(&self, i: u16) -> &'static str {
        let n = self.modifier(i).name;
        if n.is_empty() {
            content::MODIFIERS.get(i as usize).map(|m| m.name).unwrap_or("")
        } else {
            n
        }
    }

    /// `(flavor, tip)` for weapon index `i`; `("", "")` out of range.
    pub fn weapon_text(&self, i: u16) -> (&'static str, &'static str) {
        self.weapon(i).text()
    }
    /// `(flavor, tip)` for modifier index `i`; `("", "")` out of range.
    pub fn modifier_text(&self, i: u16) -> (&'static str, &'static str) {
        self.modifier(i).text()
    }

    /// Display word for a rarity (clamped to the top tier).
    pub fn rarity_tier(&self, rarity: u8) -> &'static str {
        self.rarity_tiers[(rarity as usize).min(3)]
    }
}

/// Every shipped pack, in a stable order. `PACKS[0]` is the default.
pub static PACKS: &[&ThemePack] = &[&wardens::PACK, &facility::PACK];

/// The Steam build's pack (`docs/12 §12.4`).
pub const DEFAULT_ID: &str = "wardens";

/// Look a pack up by [`ThemePack::id`].
pub fn pack(id: &str) -> Option<&'static ThemePack> {
    PACKS.iter().copied().find(|p| p.id == id)
}

/// The default pack — never `None`, unlike [`pack`].
pub fn default_pack() -> &'static ThemePack {
    pack(DEFAULT_ID).expect("the default pack is in PACKS")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn pack_arrays_are_pinned_to_the_catalog() {
        assert_eq!(NUM_WEAPONS, content::WEAPONS.len());
        assert_eq!(NUM_MODIFIERS, content::MODIFIERS.len());
    }

    /// `docs/12 §12.3`: a pack covers ALL 86 weapons and ALL 91 modifiers. No
    /// blank flavor, no blank tip, no blank resolved name — in either pack.
    #[test]
    fn every_pack_covers_every_index() {
        for p in PACKS {
            for i in 0..NUM_WEAPONS as u16 {
                let e = p.weapon(i);
                assert!(!p.weapon_name(i).is_empty(), "{} weapon[{i}] name", p.id);
                assert!(!e.flavor.is_empty(), "{} weapon[{i}] flavor", p.id);
                assert!(!e.tip.is_empty(), "{} weapon[{i}] tip", p.id);
            }
            for i in 0..NUM_MODIFIERS as u16 {
                let e = p.modifier(i);
                assert!(!p.modifier_name(i).is_empty(), "{} modifier[{i}] name", p.id);
                assert!(!e.flavor.is_empty(), "{} modifier[{i}] flavor", p.id);
                assert!(!e.tip.is_empty(), "{} modifier[{i}] tip", p.id);
            }
            assert!(p.rarity_tiers.iter().all(|t| !t.is_empty()), "{} rarity tiers", p.id);
        }
    }

    /// A pack that reuses one blurb across many indices is padding, not writing.
    /// The catalog itself carries duplicate NAMES (four modifier names repeat), so
    /// names are only checked for the pack that renames everything.
    #[test]
    fn facility_entries_are_distinct() {
        let p = &facility::PACK;
        let mut names = BTreeSet::new();
        let mut flavors = BTreeSet::new();
        let mut tips = BTreeSet::new();
        for i in 0..NUM_WEAPONS as u16 {
            let e = p.weapon(i);
            assert!(names.insert(e.name), "duplicate facility weapon name: {:?}", e.name);
            assert!(flavors.insert(e.flavor), "duplicate facility weapon flavor at {i}");
            tips.insert(e.tip);
        }
        for i in 0..NUM_MODIFIERS as u16 {
            let e = p.modifier(i);
            assert!(names.insert(e.name), "duplicate facility modifier name: {:?}", e.name);
            assert!(flavors.insert(e.flavor), "duplicate facility modifier flavor at {i}");
            tips.insert(e.tip);
        }
        // Tips MAY repeat (the catalog genuinely ships duplicate effects — e.g.
        // "+10 Armor" twice), but only a handful of them.
        assert!(tips.len() > 160, "tips are too repetitive ({} distinct of 177)", tips.len());
    }

    #[test]
    fn wardens_text_is_the_migrated_descriptions_table() {
        // The public `descriptions` API must keep returning exactly what it did.
        for i in 0..NUM_WEAPONS as u16 {
            assert_eq!(crate::descriptions::weapon_text(i), wardens::PACK.weapon_text(i));
        }
        for i in 0..NUM_MODIFIERS as u16 {
            assert_eq!(crate::descriptions::modifier_text(i), wardens::PACK.modifier_text(i));
        }
        // Wardens declines to rename: names fall through to the catalog.
        assert_eq!(wardens::PACK.weapon_name(0), content::WEAPONS[0].name);
        assert_eq!(wardens::PACK.modifier_name(0), content::MODIFIERS[0].name);
    }

    // ================= tips vs. the catalog's real magnitudes =================
    //
    // `docs/12 §12.5`, last line: "Every mechanical **tip** must stay accurate.
    // Flavor may lie to the player; the tip may not."
    //
    // The failure this guards against has already happened: `docs/11 §11.8`
    // roughly doubled every Max-HP grant and every Mana-Shield pool grant in
    // `content.rs`, and both packs went on quoting the pre-retune figures. The
    // checks below re-derive every figure FROM `content::MODIFIERS`/`WEAPONS`, so
    // the next balance pass breaks a test instead of quietly turning two dozen
    // tips into lies. They deliberately do not parse English: each one NAMES the
    // entries it covers, so a tip dropping off the list is visible in a diff.

    /// `b[k]` continues a number leftward: a digit, or the point of a decimal such
    /// as `1.5` (a sentence-ending `300.` does not).
    fn joins_left(b: &[u8], k: usize) -> bool {
        b[k].is_ascii_digit() || (b[k] == b'.' && k >= 1 && b[k - 1].is_ascii_digit())
    }
    /// `b[k]` continues a number rightward; mirror of [`joins_left`].
    fn joins_right(b: &[u8], k: usize) -> bool {
        b[k].is_ascii_digit() || (b[k] == b'.' && k + 1 < b.len() && b[k + 1].is_ascii_digit())
    }

    /// `n` appears in `tip` as a standalone number — not inside a longer number,
    /// and not as one half of a decimal.
    fn quotes(tip: &str, n: i64) -> bool {
        let needle = n.to_string();
        let b = tip.as_bytes();
        let mut from = 0;
        while let Some(off) = tip[from..].find(&needle) {
            let at = from + off;
            let end = at + needle.len();
            let left = at == 0 || !joins_left(b, at - 1);
            let right = end == b.len() || !joins_right(b, end);
            if left && right {
                return true;
            }
            from = at + 1;
        }
        false
    }

    /// Every standalone integer in `tip`. Decimals (`1.5`, `0.5`) are skipped —
    /// they are ratios the cases below express as their own figures.
    fn integers(tip: &str) -> Vec<i64> {
        let b = tip.as_bytes();
        let (mut out, mut i) = (Vec::new(), 0);
        while i < b.len() {
            if !b[i].is_ascii_digit() {
                i += 1;
                continue;
            }
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            let dec = (start > 0 && joins_left(b, start - 1)) || (i < b.len() && joins_right(b, i));
            if !dec {
                out.push(tip[start..i].parse().expect("a run of digits parses"));
            }
        }
        out
    }

    /// Below this a number in one of these tips is a percentage, a per-tick rate or
    /// a count; at or above it, it is a POOL — a Max-HP or Mana-Shield magnitude,
    /// i.e. exactly what `§11.8` moved. Every pool-sized number in a covered tip
    /// has to be one the catalog explains.
    const POOL_FLOOR: i64 = 500;

    fn modifier_def(i: u16) -> &'static content::ModifierDef {
        &content::MODIFIERS[i as usize]
    }

    /// Σ of the entry's flat `MaxHp` grants.
    fn max_hp(i: u16) -> i64 {
        modifier_def(i)
            .effects
            .iter()
            .filter_map(|e| match e {
                content::ModEffect::MaxHp(n) => Some(*n),
                _ => None,
            })
            .sum()
    }

    /// The pool of the entry's `ManaShield` grant.
    fn shield_pool(i: u16) -> i64 {
        one(i, |e| match e {
            content::ModEffect::ManaShield(p, _) => Some(vec![*p]),
            _ => None,
        })[0]
    }

    /// What the entry's per-round ramp re-applies, as one figure.
    fn ramp(i: u16) -> i64 {
        let r = modifier_def(i).ramp.unwrap_or_else(|| panic!("modifier[{i}] has no ramp"));
        match r.effect {
            content::ModEffect::MaxHp(n) => n,
            content::ModEffect::ManaShield(p, _) => p,
            other => panic!("modifier[{i}] ramps {other:?}, which no tip below quotes"),
        }
    }

    /// The figures of the first effect `f` matches.
    fn one(i: u16, f: impl Fn(&content::ModEffect) -> Option<Vec<i64>>) -> Vec<i64> {
        modifier_def(i)
            .effects
            .iter()
            .find_map(f)
            .unwrap_or_else(|| panic!("modifier[{i}] lost the effect its tip describes"))
    }

    /// `(index, figures every pack's tip must quote, further pool-sized numbers it
    /// is allowed to contain)`. The allowances are constants that are NOT this
    /// entry's own grant, so they must not move with the pool.
    fn numeric_tip_cases() -> Vec<(u16, Vec<i64>, Vec<i64>)> {
        let pct = |i| {
            one(i, |e| match e {
                content::ModEffect::IncomePct(n, d) => Some(vec![n * 100 / d]),
                _ => None,
            })
        };
        let per_damage = |i| {
            one(i, |e| match e {
                content::ModEffect::GoldPerDamagePct(n, d) => Some(vec![*n, *d]),
                _ => None,
            })
        };
        let income = |i| {
            one(i, |e| match e {
                content::ModEffect::IncomeFlat(n) => Some(vec![*n]),
                _ => None,
            })
        };
        vec![
            // ---- `docs/11 §11.8`: the Max-HP grants ----
            (11, vec![max_hp(11)], vec![]), // Imbued Masonry
            (38, vec![max_hp(38)], vec![]), // Mask of Death
            (47, vec![max_hp(47)], vec![]), // Living Wood
            (57, vec![max_hp(57)], vec![]), // Improved Masonry
            (62, vec![max_hp(62)], vec![]), // catalog name "+1000 Max HP"; grants more
            (64, vec![max_hp(64)], vec![]), // catalog name "+2000 Max HP"; grants more
            (80, vec![max_hp(80), ramp(80)], vec![]), // Living Fortress
            // Mastercrafted Masonry. Its trailing 2000 is the per-unit divisor in
            // `modifiers::Modifiers::dynamic_global_add` (+1% damage per 2000 Max
            // HP) — a live scaler, not a grant, so it is allowed, not required.
            (83, vec![max_hp(83)], vec![2000]),
            // ---- `docs/11 §11.8`: the Mana-Shield pools ----
            (13, vec![shield_pool(13)], vec![]), // Moonwell
            (56, vec![shield_pool(56), ramp(56)], vec![]), // Aegis Protocol
            (66, vec![shield_pool(66)], vec![]), // Recharge
            (77, vec![shield_pool(77)], vec![]), // Energy Shield
            (81, vec![shield_pool(81)], vec![]), // Mana Shield
            (85, vec![shield_pool(85)], vec![]), // Arcane Mark
            (86, vec![shield_pool(86)], vec![]), // Maw of Death
            // Energy Pulse; its 1200 is the shield-break stun radius, not a pool.
            (
                87,
                vec![shield_pool(87)],
                one(87, |e| match e {
                    content::ModEffect::ShieldBreakStun(r, _) => Some(vec![*r]),
                    _ => None,
                }),
            ),
            // ---- Max HP granted or paid outside a plain grant ----
            (
                45, // Ankh of Reconstruction
                one(45, |e| match e {
                    content::ModEffect::GrantRevive(n) => Some(vec![*n]),
                    _ => None,
                }),
                vec![],
            ),
            (
                51, // Philosopher's Stone
                one(51, |e| match e {
                    content::ModEffect::TradeMaxHpForGold(hp, gold) => Some(vec![*hp, *gold]),
                    _ => None,
                }),
                vec![],
            ),
            // ---- economy figures an EARLIER balance pass moved the same way ----
            (8, pct(8), vec![]),          // catalog name "+10% Gold Income"
            (9, pct(9), vec![]),          // catalog name "+25% Gold Income"
            (53, per_damage(53), vec![]), // catalog name "…per 100 Damage"
            (54, per_damage(54), vec![]), // catalog name "…per 20 Damage"
            (67, income(67), vec![]),
            (72, income(72), vec![]),
        ]
    }

    /// The core check. For every covered modifier, in EVERY pack: the tip quotes
    /// the figure `content.rs` actually grants, and carries no pool-sized number
    /// the catalog does not explain — which is what a stale figure looks like.
    #[test]
    fn modifier_tips_quote_the_catalog_magnitudes() {
        for (i, required, allowed) in numeric_tip_cases() {
            for p in PACKS {
                let tip = p.modifier(i).tip;
                for n in &required {
                    assert!(
                        quotes(tip, *n),
                        "{} modifier[{i}] tip must quote {n}, which content.rs grants: {tip:?}",
                        p.id
                    );
                }
                for n in integers(tip).into_iter().filter(|n| *n >= POOL_FLOOR) {
                    assert!(
                        required.contains(&n) || allowed.contains(&n),
                        "{} modifier[{i}] tip quotes {n}, which content.rs does not grant \
                         (it grants {required:?}): {tip:?}",
                        p.id
                    );
                }
            }
        }
    }

    /// `IncomeFlat` pays out EVERY TICK (`economy::tick_income` is phase 9 of every
    /// `step`). A tip saying "per round" overstates it by 900×, so for these the
    /// figure alone is not enough — the period has to be right too.
    #[test]
    fn flat_income_tips_state_the_right_period() {
        for i in [67u16, 72] {
            for p in PACKS {
                let tip = p.modifier(i).tip;
                assert!(tip.contains("tick"), "{} modifier[{i}] tip: {tip:?}", p.id);
                assert!(!tip.contains("round"), "{} modifier[{i}] tip: {tip:?}", p.id);
            }
        }
    }

    /// The Epic multiplier is the one covered entry whose two packs render the same
    /// number differently (`×1.4` vs `+40%`), so it gets its own check rather than
    /// a row above. Both renderings are derived from the catalog.
    #[test]
    fn multiplicative_damage_tip_matches_the_catalog() {
        const IDX: u16 = 4;
        let (n, d) = modifier_def(IDX)
            .effects
            .iter()
            .find_map(|e| match e {
                content::ModEffect::DamageMulPct(n, d) => Some((*n, *d)),
                _ => None,
            })
            .expect("modifier[4] is the multiplicative-damage entry");
        // `modifiers::apply_effect` banks ×(1 + n/d); one copy is far below the
        // soft cap, so that product is what a player actually gets.
        let permille = 1000 + n * 1000 / d;
        assert_eq!(permille % 100, 0, "×{permille} no longer renders in one decimal");
        let factor = format!("{}.{}", permille / 1000, (permille % 1000) / 100);
        let pct = n * 100 / d;
        let f_tip = facility::PACK.modifier(IDX).tip;
        let w_tip = wardens::PACK.modifier(IDX).tip;
        assert!(f_tip.contains(&format!("×{factor}")), "facility modifier[4]: {f_tip:?}");
        assert!(w_tip.contains(&format!("+{pct}%")), "wardens modifier[4]: {w_tip:?}");
    }

    /// The weapon side. `facility` states each ability magnitude as a figure, so it
    /// can be checked straight against `content::WEAPONS`; `wardens` states them in
    /// words ("feeds the tank"), which is why it is not covered here. `Root`,
    /// `Summon` and `VulnOnHit` are excluded deliberately: several facility tips
    /// render those in prose or leave a secondary rider unsaid, which is an
    /// omission rather than a contradiction.
    #[test]
    fn facility_weapon_tips_quote_the_ability_magnitudes() {
        use content::WeaponAbility as A;
        let mut checked = 0;
        for (i, w) in content::WEAPONS.iter().enumerate() {
            let tip = facility::PACK.weapon(i as u16).tip;
            let (what, n) = match w.ability {
                A::LifeDrain { per_hit } => ("life-drain per hit", per_hit),
                A::ManaDrain { per_hit } => ("mana-drain per hit", per_hit),
                A::Knockback { dist } => ("knockback distance", dist),
                A::Hazard { dmg, .. } => ("hazard damage per tick", dmg),
                _ => continue,
            };
            assert!(quotes(tip, n), "facility weapon[{i}] tip must quote its {what} ({n}): {tip:?}");
            checked += 1;
        }
        assert_eq!(checked, 10, "the ability-magnitude weapon set moved; re-check those tips");
    }

    #[test]
    fn lookup_and_out_of_range() {
        assert_eq!(pack("wardens").map(|p| p.id), Some("wardens"));
        assert_eq!(pack("facility").map(|p| p.id), Some("facility"));
        assert!(pack("nope").is_none());
        assert_eq!(default_pack().id, DEFAULT_ID);
        for p in PACKS {
            assert_eq!(p.weapon(9999), Entry::EMPTY);
            assert_eq!(p.modifier(9999), Entry::EMPTY);
            assert_eq!(p.weapon_name(9999), "");
            assert_eq!(p.modifier_name(9999), "");
            assert_eq!(p.rarity_tier(200), p.rarity_tiers[3]);
        }
    }
}
