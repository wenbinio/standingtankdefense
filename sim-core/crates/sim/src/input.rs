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
                if s.economy.gold >= offer.cost {
                    s.economy.gold -= offer.cost;
                    match offer.kind {
                        OfferKind::Weapon => {
                            let id = s.alloc_entity_id();
                            s.weapons.push(WeaponInstance {
                                instance_id: id,
                                def: offer.def,
                                next_fire_tick: s.tick,
                            });
                        }
                        OfferKind::Modifier => s.buy_modifier(offer.def),
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
                for mut e in s.enemies.drain(..) {
                    e.hp = e.hp.saturating_sub(CLEAR_DAMAGE);
                    if e.hp <= 0 {
                        s.pending_kills.push(e.def);
                    } else {
                        survivors.push(e);
                    }
                }
                s.enemies = survivors;
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
        assert_eq!(s.shop.offers.len(), 3);
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
