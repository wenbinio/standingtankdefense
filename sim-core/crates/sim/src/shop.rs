//! Shop / offers. Deterministic; randomness only from `s.rng_shop`.
//!
//! Slot model (source-fidelity pass): every slot is drawn INDEPENDENTLY —
//! first its category (weapon vs modifier), then its rarity from a weighted
//! slot-machine curve, then a uniform pick inside that (category, rarity)
//! bucket. This replaces the earlier fixed 4-weapon/4-modifier layout: the
//! source's shop mixes kinds freely and rare items appear rarely.
//!
//! Duplicate offers within one shop are ALLOWED (independent draws): the
//! source doesn't specify de-duplication, and dupes are useful in a game
//! where everything stacks — kept deliberately simple.
use crate::content;
use crate::state::*;
use determinism::Rng;

/// Number of purchasable slots presented each round / reroll.
const OFFER_SLOTS: usize = 8;

/// Per-slot category weights, weapon : modifier. 1:1 — the catalogs are nearly
/// the same size (96 weapons / 110 modifiers), so an even split approximates
/// "uniform over the combined pool" while staying a named, tunable dial.
const CATEGORY_WEIGHT_WEAPON: u32 = 1;
const CATEGORY_WEIGHT_MODIFIER: u32 = 1;

/// Per-slot rarity weights, indexed by rarity (0 Common, 1 Uncommon, 2 Rare,
/// 3 Epic). INVENTED (unextracted): the source's exact shop weights were not
/// extracted (`docs/01`), so this is a principled slot-machine curve — commons
/// carry the shop, epics stay events. Named so the balance retune can dial it.
pub(crate) const RARITY_WEIGHTS: [u32; 4] = [50, 30, 15, 5];

/// Weighted rarity draw from [`RARITY_WEIGHTS`] (single `rng` draw).
fn draw_rarity(rng: &mut Rng) -> u8 {
    let total: u32 = RARITY_WEIGHTS.iter().sum();
    let mut roll = rng.below(total);
    for (rarity, &w) in RARITY_WEIGHTS.iter().enumerate() {
        if roll < w {
            return rarity as u8;
        }
        roll -= w;
    }
    unreachable!("weights are exhaustive by construction")
}

/// Uniform pick of the `k`-th catalog index with `rarity`, where the caller
/// obtained `k = rng.below(count(rarity))`. Deterministic linear scan in
/// stable catalog order.
fn nth_of_rarity(rarities: impl Iterator<Item = u8>, rarity: u8, k: u32) -> usize {
    let mut seen = 0u32;
    for (i, r) in rarities.enumerate() {
        if r == rarity {
            if seen == k {
                return i;
            }
            seen += 1;
        }
    }
    unreachable!("k < count(rarity) by construction")
}

/// Generate a fresh set of offers: for each of the `OFFER_SLOTS` slots draw
/// category (weapon vs modifier, [`CATEGORY_WEIGHT_WEAPON`]:[`CATEGORY_WEIGHT_MODIFIER`]),
/// then rarity ([`RARITY_WEIGHTS`]), then a uniform member of that bucket —
/// exactly 3 `s.rng_shop` draws per slot. Every (category, rarity) bucket is
/// non-empty in the shipped catalog (guarded by a test below); if a retune
/// ever empties one, the draw falls back one rarity toward Common (no extra
/// RNG draws, so the cursor advance stays fixed). Replaces `s.shop.offers`,
/// increments `s.shop.shop_seq`. Called at round boundary / reroll.
pub(crate) fn generate_offers(s: &mut ArenaState) {
    let mut offers = Vec::with_capacity(OFFER_SLOTS);
    for _ in 0..OFFER_SLOTS {
        let cat_total = CATEGORY_WEIGHT_WEAPON + CATEGORY_WEIGHT_MODIFIER;
        let is_weapon = s.rng_shop.below(cat_total) < CATEGORY_WEIGHT_WEAPON;
        let mut rarity = draw_rarity(&mut s.rng_shop);
        let count = |r: u8| -> u32 {
            if is_weapon {
                content::WEAPONS.iter().filter(|w| w.rarity == r).count() as u32
            } else {
                content::MODIFIERS.iter().filter(|m| m.rarity == r).count() as u32
            }
        };
        // Defensive fallback (unreachable with the shipped catalog).
        while rarity > 0 && count(rarity) == 0 {
            rarity -= 1;
        }
        let k = s.rng_shop.below(count(rarity));
        let offer = if is_weapon {
            let idx = nth_of_rarity(content::WEAPONS.iter().map(|w| w.rarity), rarity, k);
            Offer {
                kind: OfferKind::Weapon,
                def: idx as u16,
                cost: content::WEAPONS[idx].cost,
            }
        } else {
            let idx = nth_of_rarity(content::MODIFIERS.iter().map(|m| m.rarity), rarity, k);
            Offer {
                kind: OfferKind::Modifier,
                def: idx as u16,
                cost: content::MODIFIERS[idx].cost,
            }
        };
        offers.push(offer);
    }
    s.shop.offers = offers;
    s.shop.shop_seq += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> ArenaState {
        ArenaState::new(0xDEAD_BEEF, 0)
    }

    #[test]
    fn produces_configured_slot_count() {
        let mut s = fresh();
        generate_offers(&mut s);
        assert_eq!(s.shop.offers.len(), OFFER_SLOTS);
    }

    #[test]
    fn offer_costs_match_their_catalog_def() {
        let mut s = fresh();
        generate_offers(&mut s);
        for off in &s.shop.offers {
            let expected = match off.kind {
                OfferKind::Weapon => {
                    assert!((off.def as usize) < content::WEAPONS.len());
                    content::WEAPONS[off.def as usize].cost
                }
                OfferKind::Modifier => {
                    assert!((off.def as usize) < content::MODIFIERS.len());
                    content::MODIFIERS[off.def as usize].cost
                }
            };
            assert_eq!(off.cost, expected);
        }
    }

    #[test]
    fn increments_shop_seq() {
        let mut s = fresh();
        let before = s.shop.shop_seq;
        generate_offers(&mut s);
        assert_eq!(s.shop.shop_seq, before + 1);
        generate_offers(&mut s);
        assert_eq!(s.shop.shop_seq, before + 2);
    }

    #[test]
    fn deterministic_for_fixed_rng() {
        let mut a = ArenaState::new(777, 1);
        let mut b = ArenaState::new(777, 1);
        generate_offers(&mut a);
        generate_offers(&mut b);
        assert_eq!(a.shop.offers, b.shop.offers);
        assert_eq!(a.rng_shop.state(), b.rng_shop.state());
    }

    #[test]
    fn offers_can_include_both_kinds_over_many_draws() {
        // Across enough rerolls both a weapon and a modifier should appear,
        // confirming the combined pool is sampled.
        let mut s = ArenaState::new(0x5151, 2);
        let mut saw_weapon = false;
        let mut saw_modifier = false;
        for _ in 0..50 {
            generate_offers(&mut s);
            for o in &s.shop.offers {
                match o.kind {
                    OfferKind::Weapon => saw_weapon = true,
                    OfferKind::Modifier => saw_modifier = true,
                }
            }
        }
        assert!(saw_weapon && saw_modifier, "combined pool not sampled");
    }

    #[test]
    fn every_category_rarity_bucket_is_populated() {
        // The weighted draw assumes every (category, rarity) bucket has at
        // least one member; the fallback exists but should stay unreachable.
        for r in 0..4u8 {
            assert!(
                content::WEAPONS.iter().any(|w| w.rarity == r),
                "no weapons of rarity {r}"
            );
            assert!(
                content::MODIFIERS.iter().any(|m| m.rarity == r),
                "no modifiers of rarity {r}"
            );
        }
    }

    #[test]
    fn any_slot_can_hold_either_kind() {
        // The old layout pinned slots 0-3 to weapons and 4-7 to modifiers;
        // slots are now independent, so over many shops the FIRST and LAST
        // slot must each see both kinds.
        let mut s = ArenaState::new(0xA110C, 3);
        let mut first = (false, false);
        let mut last = (false, false);
        for _ in 0..200 {
            generate_offers(&mut s);
            for (slot, seen) in [(0usize, &mut first), (OFFER_SLOTS - 1, &mut last)] {
                match s.shop.offers[slot].kind {
                    OfferKind::Weapon => seen.0 = true,
                    OfferKind::Modifier => seen.1 = true,
                }
            }
        }
        assert!(first.0 && first.1, "slot 0 must mix kinds");
        assert!(last.0 && last.1, "last slot must mix kinds");
    }

    #[test]
    fn rarity_weighting_favors_common_over_epic() {
        // Statistical but DETERMINISTIC (fixed seed): across many shops the
        // rarity histogram must follow the weight ordering C > U > R > E, and
        // every rarity must actually appear.
        let mut s = ArenaState::new(0xC0FFEE, 1);
        let mut hist = [0u32; 4];
        for _ in 0..500 {
            generate_offers(&mut s);
            for o in &s.shop.offers {
                let r = ArenaState::offer_rarity(*o);
                hist[r as usize] += 1;
            }
        }
        assert!(
            hist[0] > hist[1] && hist[1] > hist[2] && hist[2] > hist[3],
            "histogram must follow the weight curve, got {hist:?}"
        );
        assert!(hist[3] > 0, "epics must still appear, got {hist:?}");
    }

    #[test]
    fn replaces_previous_offers() {
        let mut s = fresh();
        s.shop.offers = vec![Offer {
            kind: OfferKind::Weapon,
            def: 99,
            cost: -1,
        }];
        generate_offers(&mut s);
        assert_eq!(s.shop.offers.len(), OFFER_SLOTS);
        assert!(s.shop.offers.iter().all(|o| o.def != 99 || o.cost != -1));
    }
}
