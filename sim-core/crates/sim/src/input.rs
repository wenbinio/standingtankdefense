//! Input application — AGENT C. Applies one player action this tick.
use crate::ids::Input;
use crate::shop;
use crate::state::*;

/// Damage dealt to every enemy by a `Clear`. Large but FINITE: it wipes normal
/// enemies instantly, but the boss (Samwise, ~10M HP) takes several Clears —
/// and `Clear` is the ONLY thing that can hurt the boss.
const CLEAR_DAMAGE: i64 = 3_000_000;
/// Cooldown (in ticks) imposed after a `Clear`.
const CLEAR_COOLDOWN_TICKS: u32 = 300;
/// Gold increment added to `reroll_cost` after a paid reroll.
const REROLL_COST_STEP: i64 = 100;

/// Phase 2: apply `inp`.
/// - `Noop`: nothing.
/// - `BuyOffer{slot}`: if slot valid and `gold >= offer.cost`, deduct gold and
///   push a `WeaponInstance` (def = offer.weapon_def, fresh `alloc_entity_id`,
///   `next_fire_tick = s.tick`). Otherwise ignore (illegal/insufficient).
/// - `Reroll`: if `rerolls_remaining > 0`, decrement then `shop::generate_offers`;
///   else if `gold >= reroll_cost`, deduct, regenerate, and raise `reroll_cost`
///   (e.g. +100). Otherwise ignore.
/// - `Clear`: if `s.tick >= tank.clear_cooldown_end`, kill/damage enemies (M0:
///   damage ALL enemies by a large amount; for each killed push its `def` to
///   `s.pending_kills` and remove it) and set `clear_cooldown_end = s.tick + 300`.
///   Else ignore. Use no RNG, or `s.rng_proc` if you need any.
pub(crate) fn apply(s: &mut ArenaState, inp: Input) {
    match inp {
        Input::Noop => {}

        Input::BuyOffer { slot } => {
            let idx = slot as usize;
            if let Some(offer) = s.shop.offers.get(idx).copied() {
                // A pending meta perk (`docs/06` #5) may apply to this purchase,
                // but never to another META item (no self-duplication / chaining).
                let perk = if ArenaState::offer_is_meta(offer) {
                    None
                } else {
                    let rarity = ArenaState::offer_rarity(offer);
                    s.pending_perk.filter(|p| p.matches(rarity))
                };
                // Black Market voucher makes the matching purchase free.
                let cost = match perk {
                    Some(p) if p.free => 0,
                    _ => offer.cost,
                };
                if s.economy.gold >= cost {
                    s.economy.gold -= cost;
                    s.grant_offer(offer);
                    // Duplicator: grant the extra free copies, then consume the perk.
                    if let Some(p) = perk {
                        for _ in 0..p.extra_copies {
                            s.grant_offer(offer);
                        }
                        s.pending_perk = None;
                    }
                }
            }
        }

        Input::Reroll => {
            if s.economy.rerolls_remaining > 0 {
                s.economy.rerolls_remaining -= 1;
                shop::generate_offers(s);
            } else if s.economy.gold >= s.economy.reroll_cost {
                s.economy.gold -= s.economy.reroll_cost;
                shop::generate_offers(s);
                s.economy.reroll_cost += REROLL_COST_STEP;
            }
        }

        Input::Clear => {
            if s.tick >= s.tank.clear_cooldown_end {
                let mut survivors = Vec::with_capacity(s.enemies.len());
                let mut cleared_damage: i64 = 0;
                for mut e in s.enemies.drain(..) {
                    let before = e.hp.max(0);
                    e.hp = e.hp.saturating_sub(CLEAR_DAMAGE);
                    // Actual HP removed (clamped: no credit past the kill).
                    cleared_damage += before - e.hp.max(0);
                    if e.hp <= 0 {
                        s.pending_kills.push(e.def);
                    } else {
                        survivors.push(e);
                    }
                }
                s.enemies = survivors;
                s.record_player_damage(cleared_damage);
                s.tank.clear_cooldown_end = s.tick + CLEAR_COOLDOWN_TICKS;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content;
    use crate::state::{Enemy, Offer, OfferKind, Vec2};
    use determinism::Fixed;

    fn fresh() -> ArenaState {
        ArenaState::new(0x1234_5678, 0)
    }

    fn mk_enemy(id: u32, def: u16, hp: i64) -> Enemy {
        Enemy::new(crate::ids::EntityId(id), def, hp, Vec2::ZERO)
    }

    #[test]
    fn noop_changes_nothing() {
        let mut s = fresh();
        let before = s.clone();
        apply(&mut s, Input::Noop);
        assert_eq!(s, before);
    }

    #[test]
    fn buy_offer_deducts_gold_and_adds_weapon_when_affordable() {
        let mut s = fresh();
        s.shop.offers = vec![Offer { kind: OfferKind::Weapon, def: 1, cost: 200 }];
        s.economy.gold = 500;
        let weapons_before = s.weapons.len();
        apply(&mut s, Input::BuyOffer { slot: 0 });
        assert_eq!(s.economy.gold, 300);
        assert_eq!(s.weapons.len(), weapons_before + 1);
        let w = s.weapons.last().unwrap();
        assert_eq!(w.def, 1);
        assert_eq!(w.next_fire_tick, s.tick);
    }

    #[test]
    fn buy_offer_ignored_when_unaffordable() {
        let mut s = fresh();
        s.shop.offers = vec![Offer { kind: OfferKind::Weapon, def: 1, cost: 600 }];
        s.economy.gold = 500;
        let weapons_before = s.weapons.len();
        apply(&mut s, Input::BuyOffer { slot: 0 });
        assert_eq!(s.economy.gold, 500);
        assert_eq!(s.weapons.len(), weapons_before);
    }

    #[test]
    fn buy_offer_ignored_for_invalid_slot() {
        let mut s = fresh();
        s.shop.offers = vec![Offer { kind: OfferKind::Weapon, def: 0, cost: 0 }];
        s.economy.gold = 500;
        let weapons_before = s.weapons.len();
        apply(&mut s, Input::BuyOffer { slot: 5 });
        assert_eq!(s.economy.gold, 500);
        assert_eq!(s.weapons.len(), weapons_before);
    }

    #[test]
    fn reroll_consumes_free_reroll_first() {
        let mut s = fresh();
        s.economy.rerolls_remaining = 2;
        s.economy.gold = 1000;
        let cost_before = s.economy.reroll_cost;
        apply(&mut s, Input::Reroll);
        assert_eq!(s.economy.rerolls_remaining, 1);
        assert_eq!(s.economy.gold, 1000, "free reroll must not charge gold");
        assert_eq!(s.economy.reroll_cost, cost_before, "cost unchanged on free reroll");
        assert_eq!(s.shop.offers.len(), 8);
    }

    #[test]
    fn reroll_charges_escalating_gold_when_no_free_rerolls() {
        let mut s = fresh();
        s.economy.rerolls_remaining = 0;
        s.economy.gold = 1000;
        s.economy.reroll_cost = 100;

        apply(&mut s, Input::Reroll);
        assert_eq!(s.economy.gold, 900);
        assert_eq!(s.economy.reroll_cost, 200);

        apply(&mut s, Input::Reroll);
        assert_eq!(s.economy.gold, 700);
        assert_eq!(s.economy.reroll_cost, 300);
    }

    #[test]
    fn reroll_ignored_when_broke_and_no_free() {
        let mut s = fresh();
        s.economy.rerolls_remaining = 0;
        s.economy.gold = 50;
        s.economy.reroll_cost = 100;
        s.shop.offers = vec![Offer { kind: OfferKind::Weapon, def: 7, cost: 1 }];
        let seq_before = s.shop.shop_seq;
        apply(&mut s, Input::Reroll);
        assert_eq!(s.economy.gold, 50);
        assert_eq!(s.shop.shop_seq, seq_before, "no regeneration when ignored");
        assert_eq!(s.shop.offers[0].def, 7);
    }

    // ---- meta / shop items (`docs/06` #5) ----------------------------------

    /// Find a catalog modifier index whose effect matches a predicate.
    fn modifier_idx(pred: impl Fn(&content::ModEffect) -> bool) -> u16 {
        content::MODIFIERS.iter().position(|m| pred(&m.effect)).expect("modifier exists") as u16
    }

    #[test]
    fn duplicator_grants_extra_copies_of_next_matching_purchase() {
        let mut s = fresh();
        // Arm "+N copies of next Common (rarity 0)".
        let dup = modifier_idx(|e| matches!(e, content::ModEffect::GrantDuplicator(0, _)));
        let copies = match content::MODIFIERS[dup as usize].effect {
            content::ModEffect::GrantDuplicator(_, c) => c as usize,
            _ => unreachable!(),
        };
        s.buy_modifier(dup);
        assert!(s.pending_perk.is_some(), "perk armed");

        // Buy a rarity-0 WEAPON; should yield 1 + copies instances.
        let common_weapon = content::WEAPONS.iter().position(|w| w.rarity == 0).unwrap() as u16;
        s.shop.offers = vec![Offer { kind: OfferKind::Weapon, def: common_weapon, cost: 100 }];
        s.economy.gold = 100;
        let before = s.weapons.len();
        apply(&mut s, Input::BuyOffer { slot: 0 });
        assert_eq!(s.weapons.len(), before + 1 + copies, "duplicated copies granted");
        assert!(s.pending_perk.is_none(), "perk consumed");
        assert_eq!(s.economy.gold, 0, "only the base copy costs gold");
    }

    #[test]
    fn duplicator_ignores_non_matching_rarity() {
        let mut s = fresh();
        let dup = modifier_idx(|e| matches!(e, content::ModEffect::GrantDuplicator(0, _)));
        s.buy_modifier(dup);
        // Buy a rarity-2 weapon: perk (rarity 0) must NOT apply and must remain armed.
        let rare_weapon = content::WEAPONS.iter().position(|w| w.rarity == 2).unwrap() as u16;
        s.shop.offers = vec![Offer { kind: OfferKind::Weapon, def: rare_weapon, cost: 0 }];
        let before = s.weapons.len();
        apply(&mut s, Input::BuyOffer { slot: 0 });
        assert_eq!(s.weapons.len(), before + 1, "no extra copies for wrong rarity");
        assert!(s.pending_perk.is_some(), "perk still armed");
    }

    #[test]
    fn voucher_makes_next_matching_purchase_free() {
        let mut s = fresh();
        let voucher = modifier_idx(|e| matches!(e, content::ModEffect::GrantVoucher(1)));
        s.buy_modifier(voucher);
        // A rarity-1 weapon costs more gold than we hold, but the voucher zeroes it.
        let unc_weapon = content::WEAPONS.iter().position(|w| w.rarity == 1).unwrap() as u16;
        s.shop.offers = vec![Offer { kind: OfferKind::Weapon, def: unc_weapon, cost: 9999 }];
        s.economy.gold = 0;
        let before = s.weapons.len();
        apply(&mut s, Input::BuyOffer { slot: 0 });
        assert_eq!(s.weapons.len(), before + 1, "free purchase happened");
        assert_eq!(s.economy.gold, 0, "voucher charged no gold");
        assert!(s.pending_perk.is_none(), "voucher consumed");
    }

    #[test]
    fn meta_items_do_not_consume_or_chain_perks() {
        let mut s = fresh();
        let dup = modifier_idx(|e| matches!(e, content::ModEffect::GrantDuplicator(0, _)));
        s.buy_modifier(dup);
        let armed = s.pending_perk;
        // Buying ANOTHER meta item (a rarity-1 voucher) must not consume the
        // duplicator perk — it replaces it with its own (no self-duplication).
        let voucher = modifier_idx(|e| matches!(e, content::ModEffect::GrantVoucher(1)));
        s.shop.offers = vec![Offer { kind: OfferKind::Modifier, def: voucher, cost: 0 }];
        s.economy.gold = 0;
        apply(&mut s, Input::BuyOffer { slot: 0 });
        // The duplicator did not duplicate the voucher; the perk is now the voucher.
        assert_ne!(s.pending_perk, armed);
        assert_eq!(s.pending_perk.map(|p| p.free), Some(true), "perk is the voucher");
    }

    #[test]
    fn magic_treasure_grants_instant_gold() {
        let mut s = fresh();
        let treasure = modifier_idx(|e| matches!(e, content::ModEffect::GrantGold(_)));
        let gold = match content::MODIFIERS[treasure as usize].effect {
            content::ModEffect::GrantGold(g) => g,
            _ => unreachable!(),
        };
        let before = s.economy.gold;
        s.buy_modifier(treasure);
        assert_eq!(s.economy.gold, before + gold, "instant gold granted");
    }

    #[test]
    fn clear_kills_enemies_and_records_bounties() {
        let mut s = fresh();
        s.tick = 0;
        s.tank.clear_cooldown_end = 0;
        s.enemies = vec![mk_enemy(10, 0, 200), mk_enemy(11, 1, 1200)];
        apply(&mut s, Input::Clear);
        assert!(s.enemies.is_empty(), "all enemies wiped");
        assert_eq!(s.pending_kills, vec![0, 1]);
        assert_eq!(s.tank.clear_cooldown_end, 0 + CLEAR_COOLDOWN_TICKS);
        // sanity: those defs exist in content.
        assert!(content::ENEMIES.len() >= 2);
    }

    #[test]
    fn clear_respects_cooldown() {
        let mut s = fresh();
        s.tick = 100;
        s.tank.clear_cooldown_end = 300; // still on cooldown
        s.enemies = vec![mk_enemy(10, 0, 200)];
        apply(&mut s, Input::Clear);
        assert_eq!(s.enemies.len(), 1, "clear must not fire on cooldown");
        assert!(s.pending_kills.is_empty());
        assert_eq!(s.tank.clear_cooldown_end, 300, "cooldown unchanged");
    }

    #[test]
    fn clear_fires_exactly_at_cooldown_end() {
        let mut s = fresh();
        s.tick = 300;
        s.tank.clear_cooldown_end = 300;
        s.enemies = vec![mk_enemy(10, 0, 200)];
        apply(&mut s, Input::Clear);
        assert!(s.enemies.is_empty());
        assert_eq!(s.tank.clear_cooldown_end, 600);
    }

    #[test]
    fn clear_chips_the_boss_over_several_uses() {
        // Clear deals a large FINITE amount: normal enemies die instantly, but
        // the boss (huge fixed HP) takes several Clears — the only thing that
        // can hurt it.
        let mut s = fresh();
        s.tick = 0;
        s.tank.clear_cooldown_end = 0;
        let _ = Fixed::ONE;
        let boss_hp = content::ENEMIES[content::SAMWISE as usize].base_hp;
        s.enemies = vec![mk_enemy(10, content::SAMWISE, boss_hp)];

        apply(&mut s, Input::Clear);
        assert_eq!(s.enemies.len(), 1, "boss survives a single Clear");
        assert_eq!(s.enemies[0].hp, boss_hp - CLEAR_DAMAGE);

        let needed = (boss_hp + CLEAR_DAMAGE - 1) / CLEAR_DAMAGE; // ceil
        for _ in 1..needed {
            s.tick = s.tank.clear_cooldown_end; // come off cooldown
            apply(&mut s, Input::Clear);
        }
        assert!(s.enemies.is_empty(), "boss dies after enough Clears");
        assert_eq!(s.pending_kills.last(), Some(&content::SAMWISE));
    }
}
