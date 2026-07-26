//! Shop / offers. Deterministic; randomness only from `s.rng_shop`.
//!
//! # Rarity-weighted, escalating offers (`docs/02 §2.5`, `docs/11 §11.2`)
//!
//! Offers are **not** uniform over the catalog. Each slot is drawn in two
//! stages: first a **rarity tier** from a round-dependent weight table, then a
//! **uniform pick inside that tier**. Two-stage is the point — it makes the
//! shown distribution exactly the designed one, independent of how many entries
//! each tier happens to contain, so adding thirty Commons to the catalog does
//! not flood the shop.
//!
//! ## RNG contract (the Luau port must match this exactly)
//!
//! `generate_offers` performs **exactly 16 `rng_shop.below(..)` calls**, in this
//! order, for `slot` in `0..8`:
//!
//! 1. `below(WEIGHT_TOTAL)` — the tier roll (`WEIGHT_TOTAL` is the compile-time
//!    constant `40_000`; it never varies with round, kind or catalog size).
//! 2. `below(n)` — the member roll, where `n` is the number of catalog entries
//!    of this slot's *effective* rarity for this slot's kind (weapon for slots
//!    `0..4`, modifier for `4..8`).
//!
//! The draw **count and order are a pure function of state** — never of the
//! values rolled. The tier→effective-rarity fallback (for an empty tier) is a
//! pure function of the catalog and consumes no randomness. `below` itself may
//! internally consume more than one `next_u32` (Lemire rejection); that is
//! pre-existing behavior of `determinism::Rng` and must be transcribed exactly.
use crate::content;
use crate::state::*;

/// Number of purchasable slots presented each round / reroll.
const OFFER_SLOTS: usize = 8;
/// Of those, the first `WEAPON_SLOTS` are always weapons; the remainder are
/// always modifiers (economy / passives / spikes). A fixed layout so every
/// round offers a steady spread of both, never all-of-one-kind.
const WEAPON_SLOTS: usize = 4;

/// Rarity tiers on the ladder: 0 Common, 1 Uncommon, 2 Rare, 3 Epic.
const TIERS: usize = 4;

/// Tier weights, in per-mille, for the **first** shop of a run (round 0).
///
/// Common is the backdrop and Epic is an event: at 20/1000 per slot, an 8-slot
/// shop contains an Epic ~15% of the time, so you meet one every six or seven
/// shops — often enough to be a real hope, rare enough that hitting one (when
/// you almost certainly cannot afford its 5000 g) is worth bending the plan for.
/// Compare the previous uniform draw, which showed **15%** Epic *per slot*
/// (13/86 weapons) from the opening round — i.e. an Epic in essentially every
/// shop, which is why nothing ever felt like a hit.
const WEIGHTS_EARLY: [u32; TIERS] = [600, 280, 100, 20];

/// Tier weights, in per-mille, at `ESCALATION_ROUNDS` and after.
///
/// The inversion is the escalation: Common collapses from the backdrop to the
/// residue, and Epic goes from an event to the thing a late shop is *about*
/// (~1.6 Epic slots of 8). A minute-20 shop is materially a different object
/// from a minute-1 shop, which is precisely what `docs/11 §11.2` asks for.
const WEIGHTS_LATE: [u32; TIERS] = [150, 300, 350, 200];

/// Rounds over which the early table ramps linearly into the late table.
/// 40 rounds @ 30 s = **20 minutes** — the point `docs/11 §11.2` names as the
/// far end of the feel curve, and the sweep harness's cap. The last third of a
/// run (the 20→30 min boss run-up) sits at full escalation.
const ESCALATION_ROUNDS: u32 = 40;

/// Total tier weight. Constant because both anchor tables sum to 1000 and the
/// interpolation is `EARLY*(N-l) + LATE*l`, so the sum is `1000*N` at every
/// round. A fixed `below` bound keeps the draw bound state-independent.
const WEIGHT_TOTAL: u32 = 1000 * ESCALATION_ROUNDS;

const _: () = {
    assert!(WEIGHTS_EARLY[0] + WEIGHTS_EARLY[1] + WEIGHTS_EARLY[2] + WEIGHTS_EARLY[3] == 1000);
    assert!(WEIGHTS_LATE[0] + WEIGHTS_LATE[1] + WEIGHTS_LATE[2] + WEIGHTS_LATE[3] == 1000);
};

/// Tier weights for `round`, by exact integer interpolation between the two
/// anchor tables. No division, no floats: `w[i] = EARLY[i]*(N-l) + LATE[i]*l`
/// with `l = min(round, N)`. Sums to [`WEIGHT_TOTAL`] for every round.
///
/// `round` is `u32::MAX` before the first round starts (the `ArenaState::new`
/// sentinel); that is treated as round 0, so a shop generated before the clock
/// starts uses the opening table rather than the endgame one.
pub(crate) fn rarity_weights(round: u32) -> [u32; TIERS] {
    let l = if round == u32::MAX { 0 } else { round.min(ESCALATION_ROUNDS) };
    let e = ESCALATION_ROUNDS - l;
    let mut w = [0u32; TIERS];
    let mut i = 0;
    while i < TIERS {
        w[i] = WEIGHTS_EARLY[i] * e + WEIGHTS_LATE[i] * l;
        i += 1;
    }
    w
}

/// How many catalog entries of each rarity exist, for one kind. Pure function of
/// the (static) catalog; recomputed per shop rather than cached so the sim keeps
/// no hidden state. ~180 integer compares per shop — free at this cadence.
fn tier_counts(rarity_of: impl Fn(usize) -> u8, len: usize) -> [u32; TIERS] {
    let mut c = [0u32; TIERS];
    for i in 0..len {
        let r = rarity_of(i);
        if (r as usize) < TIERS {
            c[r as usize] += 1;
        }
    }
    c
}

/// Roll a tier from `weights` (which must sum to [`WEIGHT_TOTAL`]).
/// One `below(WEIGHT_TOTAL)` draw; the walk consumes no further randomness.
fn draw_tier(rng: &mut determinism::Rng, weights: &[u32; TIERS]) -> usize {
    let mut r = rng.below(WEIGHT_TOTAL);
    let mut t = 0;
    while t < TIERS - 1 {
        if r < weights[t] {
            return t;
        }
        r -= weights[t];
        t += 1;
    }
    TIERS - 1
}

/// Map a rolled tier onto one the catalog can actually serve: prefer the tier
/// itself, else the nearest **lower** tier, else the nearest higher one.
/// Deterministic, RNG-free, and a pure function of the catalog — so an empty
/// tier can never perturb the draw sequence. Downward-first keeps a missing
/// Epic pool from silently inflating the perceived rarity of what is offered.
fn effective_tier(counts: &[u32; TIERS], tier: usize) -> usize {
    if counts[tier] > 0 {
        return tier;
    }
    for d in 1..TIERS {
        if d <= tier && counts[tier - d] > 0 {
            return tier - d;
        }
        if tier + d < TIERS && counts[tier + d] > 0 {
            return tier + d;
        }
    }
    tier
}

/// Index of the `k`-th (0-based) catalog entry whose rarity is `tier`.
/// `k` must be `< counts[tier]`.
fn nth_of_tier(rarity_of: impl Fn(usize) -> u8, len: usize, tier: usize, k: u32) -> usize {
    let mut seen = 0u32;
    for i in 0..len {
        if rarity_of(i) as usize == tier {
            if seen == k {
                return i;
            }
            seen += 1;
        }
    }
    // Unreachable for `k < counts[tier]`; fall back to the last entry rather
    // than panicking so a malformed catalog degrades instead of killing a match.
    len.saturating_sub(1)
}

/// Generate a fresh set of offers: the first `WEAPON_SLOTS` from the weapon
/// pool, the rest from the modifier pool, each **rarity-weighted by the current
/// round** (see the module header for the exact draw sequence). Replaces
/// `s.shop.offers` and increments `s.shop.shop_seq`. Called at the round
/// boundary and on reroll.
pub(crate) fn generate_offers(s: &mut ArenaState) {
    let weights = rarity_weights(s.round);
    let wcounts = tier_counts(|i| content::WEAPONS[i].rarity, content::WEAPONS.len());
    let mcounts = tier_counts(|i| content::MODIFIERS[i].rarity, content::MODIFIERS.len());

    let mut offers = Vec::with_capacity(OFFER_SLOTS);
    for slot in 0..OFFER_SLOTS {
        // Draw 1 of 2: the tier. Same bound for every slot, every round.
        let tier = draw_tier(&mut s.rng_shop, &weights);
        let offer = if slot < WEAPON_SLOTS {
            let t = effective_tier(&wcounts, tier);
            // Draw 2 of 2: uniform inside the tier.
            let k = s.rng_shop.below(wcounts[t]);
            let r = nth_of_tier(|i| content::WEAPONS[i].rarity, content::WEAPONS.len(), t, k);
            Offer { kind: OfferKind::Weapon, def: r as u16, cost: content::WEAPONS[r].cost }
        } else {
            let t = effective_tier(&mcounts, tier);
            let k = s.rng_shop.below(mcounts[t]);
            let r = nth_of_tier(|i| content::MODIFIERS[i].rarity, content::MODIFIERS.len(), t, k);
            Offer { kind: OfferKind::Modifier, def: r as u16, cost: content::MODIFIERS[r].cost }
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

    /// Rarity of an offer, whichever kind it is.
    fn rarity_of(o: &Offer) -> u8 {
        match o.kind {
            OfferKind::Weapon => content::WEAPONS[o.def as usize].rarity,
            OfferKind::Modifier => content::MODIFIERS[o.def as usize].rarity,
        }
    }

    /// Rarity histogram over `shops` generations at a fixed round.
    fn histogram(seed: u64, round: u32, shops: usize) -> [u32; TIERS] {
        let mut s = ArenaState::new(seed, 0);
        s.round = round;
        let mut h = [0u32; TIERS];
        for _ in 0..shops {
            generate_offers(&mut s);
            for o in &s.shop.offers {
                h[rarity_of(o) as usize] += 1;
            }
        }
        h
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
        s.shop.offers = vec![Offer { kind: OfferKind::Weapon, def: 99, cost: -1 }];
        generate_offers(&mut s);
        assert_eq!(s.shop.offers.len(), OFFER_SLOTS);
        assert!(s.shop.offers.iter().all(|o| o.def != 99 || o.cost != -1));
    }

    // ---- rarity weighting ------------------------------------------------

    #[test]
    fn weights_always_sum_to_the_constant_total() {
        for round in [0u32, 1, 7, 20, 39, 40, 41, 100, 59_999, u32::MAX] {
            let w = rarity_weights(round);
            assert_eq!(w.iter().sum::<u32>(), WEIGHT_TOTAL, "round {round}");
        }
    }

    #[test]
    fn weights_are_monotone_in_round() {
        // Common only ever falls, Epic only ever rises, over the whole ramp.
        let mut prev = rarity_weights(0);
        for round in 1..=ESCALATION_ROUNDS + 5 {
            let w = rarity_weights(round);
            assert!(w[0] <= prev[0], "common rose at round {round}");
            assert!(w[3] >= prev[3], "epic fell at round {round}");
            prev = w;
        }
        // Plateau: past the ramp the table stops moving.
        assert_eq!(rarity_weights(ESCALATION_ROUNDS), rarity_weights(u32::MAX - 1));
    }

    #[test]
    fn pre_round_sentinel_uses_the_opening_table() {
        assert_eq!(rarity_weights(u32::MAX), rarity_weights(0));
    }

    #[test]
    fn every_tier_is_servable_by_both_catalogs() {
        // The escalation is only real if all four tiers actually exist; if a
        // catalog loses a tier the fallback silently flattens the curve.
        let wc = tier_counts(|i| content::WEAPONS[i].rarity, content::WEAPONS.len());
        let mc = tier_counts(|i| content::MODIFIERS[i].rarity, content::MODIFIERS.len());
        for t in 0..TIERS {
            assert!(wc[t] > 0, "no weapons of rarity {t}");
            assert!(mc[t] > 0, "no modifiers of rarity {t}");
            assert_eq!(effective_tier(&wc, t), t);
            assert_eq!(effective_tier(&mc, t), t);
        }
        assert_eq!(wc.iter().sum::<u32>() as usize, content::WEAPONS.len());
        assert_eq!(mc.iter().sum::<u32>() as usize, content::MODIFIERS.len());
    }

    #[test]
    fn empty_tier_falls_back_downward_then_upward() {
        assert_eq!(effective_tier(&[5, 0, 3, 1], 1), 0); // down first
        assert_eq!(effective_tier(&[0, 0, 3, 1], 1), 2); // nothing below → up
        assert_eq!(effective_tier(&[7, 0, 0, 0], 3), 0);
        assert_eq!(effective_tier(&[0, 0, 0, 9], 0), 3);
    }

    #[test]
    fn nth_of_tier_enumerates_in_catalog_order() {
        let rar = |i: usize| [0u8, 2, 1, 2, 0][i];
        assert_eq!(nth_of_tier(rar, 5, 0, 0), 0);
        assert_eq!(nth_of_tier(rar, 5, 0, 1), 4);
        assert_eq!(nth_of_tier(rar, 5, 2, 0), 1);
        assert_eq!(nth_of_tier(rar, 5, 2, 1), 3);
        assert_eq!(nth_of_tier(rar, 5, 1, 0), 2);
    }

    #[test]
    fn early_shops_are_mostly_common_and_rarely_epic() {
        let h = histogram(0xC0FFEE, 0, 4000);
        let total: u32 = h.iter().sum();
        // Common is the backdrop.
        assert!(h[0] * 100 / total >= 50, "common share too low: {h:?}");
        // Epic is an event, not furniture: ~2%.
        assert!(h[3] * 100 / total <= 5, "epic too common early: {h:?}");
        // …but it is not absent — the jackpot must be reachable.
        assert!(h[3] > 0, "no epic ever offered early: {h:?}");
    }

    #[test]
    fn late_shops_invert_the_ladder() {
        let h = histogram(0xC0FFEE, ESCALATION_ROUNDS, 4000);
        let total: u32 = h.iter().sum();
        assert!(h[0] * 100 / total <= 25, "common still dominant late: {h:?}");
        assert!(h[3] * 100 / total >= 12, "epic not an event late: {h:?}");
        assert!(h[2] + h[3] > h[0] + h[1], "late shop not high-rarity: {h:?}");
    }

    #[test]
    fn escalation_is_visible_between_early_and_late() {
        let early = histogram(0x5EED, 0, 4000);
        let late = histogram(0x5EED, ESCALATION_ROUNDS, 4000);
        assert!(late[3] > early[3] * 5, "epic did not escalate: {early:?} → {late:?}");
        assert!(early[0] > late[0] * 2, "common did not recede: {early:?} → {late:?}");
        // Mid-run sits strictly between the two ends.
        let mid = histogram(0x5EED, ESCALATION_ROUNDS / 2, 4000);
        assert!(early[3] < mid[3] && mid[3] < late[3], "no smooth ramp: {mid:?}");
    }

    #[test]
    fn draw_sequence_is_exactly_two_below_calls_per_slot_in_slot_order() {
        // Pins the RNG contract the Luau port has to reproduce. Replays the
        // documented sequence against a cloned stream and requires it to
        // produce the same offers AND land on the same RNG state.
        let mut s = ArenaState::new(0xABCD_1234, 3);
        s.round = 17;
        let mut shadow = s.rng_shop;
        generate_offers(&mut s);

        let weights = rarity_weights(17);
        let wc = tier_counts(|i| content::WEAPONS[i].rarity, content::WEAPONS.len());
        let mc = tier_counts(|i| content::MODIFIERS[i].rarity, content::MODIFIERS.len());
        for slot in 0..OFFER_SLOTS {
            let mut acc = shadow.below(WEIGHT_TOTAL);
            let mut tier = TIERS - 1;
            for (t, w) in weights.iter().enumerate().take(TIERS - 1) {
                if acc < *w {
                    tier = t;
                    break;
                }
                acc -= *w;
            }
            let expect = if slot < WEAPON_SLOTS {
                let t = effective_tier(&wc, tier);
                let k = shadow.below(wc[t]);
                let i = nth_of_tier(|i| content::WEAPONS[i].rarity, content::WEAPONS.len(), t, k);
                Offer { kind: OfferKind::Weapon, def: i as u16, cost: content::WEAPONS[i].cost }
            } else {
                let t = effective_tier(&mc, tier);
                let k = shadow.below(mc[t]);
                let i =
                    nth_of_tier(|i| content::MODIFIERS[i].rarity, content::MODIFIERS.len(), t, k);
                Offer { kind: OfferKind::Modifier, def: i as u16, cost: content::MODIFIERS[i].cost }
            };
            assert_eq!(s.shop.offers[slot], expect, "slot {slot} diverged");
        }
        assert_eq!(shadow.state(), s.rng_shop.state(), "extra/missing draws");
    }

    #[test]
    fn slot_layout_is_unchanged() {
        // The Luau `Shop.luau` and the meta layer both assume 4 weapons then 4
        // modifiers. Weighting changes *what* is drawn, never the layout.
        let mut s = ArenaState::new(4242, 0);
        s.round = 25;
        generate_offers(&mut s);
        for (i, o) in s.shop.offers.iter().enumerate() {
            let want = if i < WEAPON_SLOTS { OfferKind::Weapon } else { OfferKind::Modifier };
            assert_eq!(o.kind, want, "slot {i}");
        }
    }
}
