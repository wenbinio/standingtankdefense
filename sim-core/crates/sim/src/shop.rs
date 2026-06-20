//! Shop / offers — AGENT C. Deterministic; randomness only from `s.rng_shop`.
use crate::content;
use crate::state::*;

/// Number of purchasable slots presented each round / reroll.
const OFFER_SLOTS: usize = 3;

/// Generate a fresh set of offers (recommend 3 slots) by drawing weapon defs
/// from `content::WEAPONS` via `s.rng_shop`. Set each `Offer.cost` from the
/// weapon's `cost`. Replace `s.shop.offers`, increment `s.shop.shop_seq`.
/// Called at each round boundary and on reroll.
pub(crate) fn generate_offers(s: &mut ArenaState) {
    let n = content::WEAPONS.len() as u32;
    let mut offers = Vec::with_capacity(OFFER_SLOTS);
    for _ in 0..OFFER_SLOTS {
        let idx = s.rng_shop.below(n) as usize;
        let def = idx as u16;
        offers.push(Offer {
            weapon_def: def,
            cost: content::WEAPONS[idx].cost,
        });
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
    fn offer_costs_match_weapon_defs() {
        let mut s = fresh();
        generate_offers(&mut s);
        for off in &s.shop.offers {
            assert!((off.weapon_def as usize) < content::WEAPONS.len());
            assert_eq!(off.cost, content::WEAPONS[off.weapon_def as usize].cost);
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
        // Two states with identical seeds must produce identical offers.
        let mut a = ArenaState::new(777, 1);
        let mut b = ArenaState::new(777, 1);
        generate_offers(&mut a);
        generate_offers(&mut b);
        assert_eq!(a.shop.offers, b.shop.offers);
        // And the rng cursor advanced identically.
        assert_eq!(a.rng_shop.state(), b.rng_shop.state());
    }

    #[test]
    fn replaces_previous_offers() {
        let mut s = fresh();
        s.shop.offers = vec![Offer { weapon_def: 99, cost: -1 }];
        generate_offers(&mut s);
        assert_eq!(s.shop.offers.len(), OFFER_SLOTS);
        assert!(s.shop.offers.iter().all(|o| o.weapon_def != 99));
    }
}
