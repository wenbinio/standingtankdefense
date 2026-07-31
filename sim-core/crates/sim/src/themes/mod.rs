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
//!   truth about `content.rs`, in every pack.
//!
//! Two packs ship:
//!
//! | id | register |
//! | --- | --- |
//! | `wardens` | the original grim dark fantasy; the Steam default. Text migrated verbatim from `descriptions.rs`. |
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
