//! Shop / offers. Deterministic; randomness only from `s.rng_shop`.
use crate::content;
use crate::state::*;

/// Number of purchasable slots presented each round / reroll.
const OFFER_SLOTS: usize = 8;
/// Of those, the first `WEAPON_SLOTS` are always weapons; the remainder are
/// always modifiers (economy / passives / spikes). A fixed layout so every
/// round offers a steady spread of both, never all-of-one-kind.
const WEAPON_SLOTS: usize = 4;

/// Generate a fresh set of offers: the first `WEAPON_SLOTS` drawn uniformly from
/// the weapon pool, the rest from the modifier pool, via `s.rng_shop`. Replace
/// `s.shop.offers`, increment `s.shop.shop_seq`. Called at round boundary / reroll.
pub(crate) fn generate_offers(s: &mut ArenaState) {
    let nw = content::WEAPONS.len() as u32;
    let nm = content::MODIFIERS.len() as u32;
    let mut offers = Vec::with_capacity(OFFER_SLOTS);
    for slot in 0..OFFER_SLOTS {
        let offer = if slot < WEAPON_SLOTS {
            let r = s.rng_shop.below(nw) as usize;
            Offer {
                kind: OfferKind::Weapon,
                def: r as u16,
                cost: content::WEAPONS[r].cost,
            }
        } else {
            let r = s.rng_shop.below(nm) as usize;
            Offer {
                kind: OfferKind::Modifier,
                def: r as u16,
                cost: content::MODIFIERS[r].cost,
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
