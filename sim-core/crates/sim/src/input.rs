//! Input application — AGENT C. Applies one player action this tick.
use crate::ids::Input;
use crate::shop;
use crate::state::*;

/// Damage dealt to every enemy by a `Clear`. Large but FINITE: it wipes normal
/// enemies instantly, but the boss (The Hippocrate, ~33M HP) takes
/// ~11 Clears — and `Clear` is the ONLY thing that can hurt the boss.
const CLEAR_DAMAGE: i64 = 3_000_000;
/// Cooldown (in ticks) imposed after a `Clear`. `pub(crate)` so the render
/// view can report the cooldown fraction (`view::RenderView::clear_cooldown_total`).
pub(crate) const CLEAR_COOLDOWN_TICKS: u32 = 300;
/// Paid-reroll pricing, INVENTED (unextracted): the source escalates reroll
/// cost per use but the exact curve was never extracted (`docs/01`); this
/// linear 100g-base / +100g-per-use placeholder stands in until it is. Named
/// dials so the balance retune (or a future extraction) can replace the curve
/// in one place. `REROLL_COST_BASE` seeds `Economy::reroll_cost` in
/// `ArenaState::new`; `REROLL_COST_STEP` is added after each PAID reroll.
pub(crate) const REROLL_COST_BASE: i64 = 100;
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
                // but never to another META item (no self-duplication / chaining
                // — the source's "Does not work on Magic Coins, Magic Treasures
                // or Black Markets"). Rarity AND scope must both match.
                let perk = if ArenaState::offer_is_meta(offer) {
                    None
                } else {
                    s.pending_perk.filter(|p| p.matches(offer))
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
                let mut cleared = Vec::new();
                for mut e in s.enemies.drain(..) {
                    let before = e.hp.max(0);
                    e.hp = e.hp.saturating_sub(CLEAR_DAMAGE);
                    // Actual HP removed (clamped: no credit past the kill).
                    cleared_damage += before - e.hp.max(0);
                    if e.hp <= 0 {
                        s.pending_kills.push(e.def);
                        cleared.push((e.def, e.pos));
                    } else {
                        survivors.push(e);
                    }
                }
                // Render events: Clear kills bypass `reap_dead` (existing
                // behavior: no Fire chain from a Clear), announce them here.
                for (def, pos) in cleared {
                    let edef = &crate::content::ENEMIES[def as usize];
                    s.emit(SimEvent::EnemyKilled {
                        x: pos.x.floor_to_int(),
                        y: pos.y.floor_to_int(),
                        kind: def,
                        boss: edef.boss,
                        bounty: edef.bounty,
                        fire_explosion_radius: 0,
                    });
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
        s.shop.offers = vec![Offer {
            kind: OfferKind::Weapon,
            def: 1,
            cost: 200,
        }];
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
        s.shop.offers = vec![Offer {
            kind: OfferKind::Weapon,
            def: 1,
            cost: 600,
        }];
        s.economy.gold = 500;
        let weapons_before = s.weapons.len();
        apply(&mut s, Input::BuyOffer { slot: 0 });
        assert_eq!(s.economy.gold, 500);
        assert_eq!(s.weapons.len(), weapons_before);
    }

    #[test]
    fn buy_offer_ignored_for_invalid_slot() {
        let mut s = fresh();
        s.shop.offers = vec![Offer {
            kind: OfferKind::Weapon,
            def: 0,
            cost: 0,
        }];
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
        assert_eq!(
            s.economy.reroll_cost, cost_before,
            "cost unchanged on free reroll"
        );
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
        s.shop.offers = vec![Offer {
            kind: OfferKind::Weapon,
            def: 7,
            cost: 1,
        }];
        let seq_before = s.shop.shop_seq;
        apply(&mut s, Input::Reroll);
        assert_eq!(s.economy.gold, 50);
        assert_eq!(s.shop.shop_seq, seq_before, "no regeneration when ignored");
        assert_eq!(s.shop.offers[0].def, 7);
    }

    // ---- meta / shop items (`docs/06` #5) ----------------------------------

    /// Find a catalog modifier index whose effect matches a predicate.
    fn modifier_idx(pred: impl Fn(&content::ModEffect) -> bool) -> u16 {
        content::MODIFIERS
            .iter()
            .position(|m| m.effects.iter().any(&pred))
            .expect("modifier exists") as u16
    }

    /// Find a catalog modifier index by rarity + predicate over the whole def.
    fn modifier_where(pred: impl Fn(&content::ModifierDef) -> bool) -> u16 {
        content::MODIFIERS
            .iter()
            .position(pred)
            .expect("modifier exists") as u16
    }

    #[test]
    fn common_duplicator_multiplies_next_common_upgrade() {
        // Multiplication Gems: "+3 extra copies of the next 500 Gold (Common)
        // UPGRADE" — upgrades only, never weapons.
        let mut s = fresh();
        let dup = modifier_idx(|e| matches!(e, content::ModEffect::GrantDuplicator(0, _)));
        let copies = content::MODIFIERS[dup as usize]
            .effects
            .iter()
            .find_map(|e| match e {
                content::ModEffect::GrantDuplicator(_, c) => Some(*c),
                _ => None,
            })
            .unwrap();
        s.buy_modifier(dup);
        assert!(s.pending_perk.is_some(), "perk armed");

        // Buy a Common non-meta UPGRADE ("+10% Damage", def 0): the purchase
        // applies 1 + copies times.
        let target = modifier_where(|m| {
            m.rarity == 0
                && !m.is_meta()
                && m.effects.len() == 1
                && m.effects[0] == content::ModEffect::DamageGlobalPct(1, 10)
        });
        s.shop.offers = vec![Offer {
            kind: OfferKind::Modifier,
            def: target,
            cost: 500,
        }];
        s.economy.gold = 500;
        let add_before = s.modifiers.add_global;
        apply(&mut s, Input::BuyOffer { slot: 0 });
        // Expected: the +10% additive folded in 1 + copies times.
        let per = Fixed::from_ratio(1, 10);
        let mut expected = Fixed::ZERO;
        for _ in 0..(1 + copies) {
            expected += per;
        }
        assert_eq!(
            s.modifiers.add_global - add_before,
            expected,
            "upgrade applied 1 + copies times"
        );
        assert!(s.pending_perk.is_none(), "perk consumed");
        assert_eq!(s.economy.gold, 0, "only the base copy costs gold");
    }

    #[test]
    fn common_duplicator_does_not_apply_to_weapons() {
        // SOURCE SCOPE: Multiplication Gems target UPGRADES only — buying a
        // Common WEAPON must neither duplicate nor consume the perk.
        let mut s = fresh();
        let dup = modifier_idx(|e| matches!(e, content::ModEffect::GrantDuplicator(0, _)));
        s.buy_modifier(dup);
        let common_weapon = content::WEAPONS.iter().position(|w| w.rarity == 0).unwrap() as u16;
        s.shop.offers = vec![Offer {
            kind: OfferKind::Weapon,
            def: common_weapon,
            cost: 100,
        }];
        s.economy.gold = 100;
        let before = s.weapons.len();
        apply(&mut s, Input::BuyOffer { slot: 0 });
        assert_eq!(s.weapons.len(), before + 1, "no extra weapon copies");
        assert!(s.pending_perk.is_some(), "perk still armed");
        assert_eq!(s.economy.gold, 0, "weapon paid full price");
    }

    #[test]
    fn rare_duplicator_applies_to_weapons_but_not_wrong_rarity() {
        // Duplicator: "+1 extra copy of the next RARE Weapon or Spikes Damage
        // Upgrade" — weapons qualify, but only at the matching rarity.
        let mut s = fresh();
        let dup = modifier_idx(|e| matches!(e, content::ModEffect::GrantDuplicator(2, _)));
        s.buy_modifier(dup);
        // A COMMON weapon: wrong rarity — perk must NOT apply and stays armed.
        let common_weapon = content::WEAPONS.iter().position(|w| w.rarity == 0).unwrap() as u16;
        s.shop.offers = vec![Offer {
            kind: OfferKind::Weapon,
            def: common_weapon,
            cost: 0,
        }];
        let before = s.weapons.len();
        apply(&mut s, Input::BuyOffer { slot: 0 });
        assert_eq!(
            s.weapons.len(),
            before + 1,
            "no extra copies for wrong rarity"
        );
        assert!(s.pending_perk.is_some(), "perk still armed");

        // A RARE weapon: matches — 1 + copies instances land.
        let rare_weapon = content::WEAPONS.iter().position(|w| w.rarity == 2).unwrap() as u16;
        s.shop.offers = vec![Offer {
            kind: OfferKind::Weapon,
            def: rare_weapon,
            cost: 0,
        }];
        let before = s.weapons.len();
        apply(&mut s, Input::BuyOffer { slot: 0 });
        assert_eq!(s.weapons.len(), before + 2, "rare weapon duplicated");
        assert!(s.pending_perk.is_none(), "perk consumed");
    }

    #[test]
    fn voucher_makes_next_matching_purchase_free() {
        let mut s = fresh();
        let voucher = modifier_idx(|e| matches!(e, content::ModEffect::GrantVoucher(1)));
        s.buy_modifier(voucher);
        // A rarity-1 weapon costs more gold than we hold, but the voucher zeroes it.
        let unc_weapon = content::WEAPONS.iter().position(|w| w.rarity == 1).unwrap() as u16;
        s.shop.offers = vec![Offer {
            kind: OfferKind::Weapon,
            def: unc_weapon,
            cost: 9999,
        }];
        s.economy.gold = 0;
        let before = s.weapons.len();
        apply(&mut s, Input::BuyOffer { slot: 0 });
        assert_eq!(s.weapons.len(), before + 1, "free purchase happened");
        assert_eq!(s.economy.gold, 0, "voucher charged no gold");
        assert!(s.pending_perk.is_none(), "voucher consumed");
    }

    #[test]
    fn voucher_scope_covers_spikes_upgrades_but_not_other_upgrades() {
        // Black Market: "Buy 1 Uncommon WEAPON or SPIKES DAMAGE UPGRADE of
        // your choosing" — a Spikes upgrade qualifies, other upgrades don't.
        let voucher = modifier_idx(|e| matches!(e, content::ModEffect::GrantVoucher(1)));

        // An Uncommon NON-spikes upgrade must not consume the voucher.
        let mut s = fresh();
        s.buy_modifier(voucher);
        let plain = modifier_where(|m| {
            m.rarity == 1
                && !m.is_meta()
                && !m.effects.iter().any(|e| {
                    matches!(
                        e,
                        content::ModEffect::SpikesFlat(..)
                            | content::ModEffect::SpikesPct(..)
                            | content::ModEffect::SpikesPoison(..)
                            | content::ModEffect::StackingSpikes(..)
                    )
                })
        });
        s.shop.offers = vec![Offer {
            kind: OfferKind::Modifier,
            def: plain,
            cost: 0,
        }];
        s.economy.gold = 0;
        apply(&mut s, Input::BuyOffer { slot: 0 });
        assert!(s.pending_perk.is_some(), "non-spikes upgrade left it armed");

        // An Uncommon SPIKES upgrade is inside the scope: free, consumed.
        let mut s = fresh();
        s.buy_modifier(voucher);
        let spikes = modifier_where(|m| {
            m.rarity == 1
                && m.effects
                    .iter()
                    .any(|e| matches!(e, content::ModEffect::SpikesFlat(..)))
        });
        s.shop.offers = vec![Offer {
            kind: OfferKind::Modifier,
            def: spikes,
            cost: 9999,
        }];
        s.economy.gold = 0;
        let spikes_before = s.tank.spikes_damage;
        apply(&mut s, Input::BuyOffer { slot: 0 });
        assert!(
            s.tank.spikes_damage > spikes_before,
            "spikes upgrade purchased free"
        );
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
        s.shop.offers = vec![Offer {
            kind: OfferKind::Modifier,
            def: voucher,
            cost: 0,
        }];
        s.economy.gold = 0;
        apply(&mut s, Input::BuyOffer { slot: 0 });
        // The duplicator did not duplicate the voucher; the perk is now the voucher.
        assert_ne!(s.pending_perk, armed);
        assert_eq!(
            s.pending_perk.map(|p| p.free),
            Some(true),
            "perk is the voucher"
        );
    }

    #[test]
    fn magic_treasure_arms_a_growing_pool_not_instant_gold() {
        // Source: a HELD consumable — value starts at 250, grows +2/s, banked
        // on use (here: at the next shop roll) or when a second is bought.
        let mut s = fresh();
        let treasure = modifier_idx(|e| matches!(e, content::ModEffect::GrantGold(_)));
        let base = content::MODIFIERS[treasure as usize]
            .effects
            .iter()
            .find_map(|e| match e {
                content::ModEffect::GrantGold(g) => Some(*g),
                _ => None,
            })
            .unwrap();
        let gold_before = s.economy.gold;
        s.buy_modifier(treasure);
        assert_eq!(s.economy.gold, gold_before, "no instant wallet gold");
        assert_eq!(s.economy.treasure_pool, base, "pool armed at base value");
        assert!(s.ramps.is_empty(), "no permanent income ramp anymore");

        // "Purchasing a second Magic Treasure uses the first": the held pool
        // is banked, then the new one starts fresh at the base value.
        s.economy.treasure_pool = base + 40; // pretend it grew for 20 s
        s.buy_modifier(treasure);
        assert_eq!(s.economy.gold, gold_before + base + 40, "first banked");
        assert_eq!(s.economy.treasure_pool, base, "second starts fresh");
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
        assert_eq!(s.tank.clear_cooldown_end, CLEAR_COOLDOWN_TICKS);
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
        let boss_hp = content::ENEMIES[content::BOSS as usize].base_hp;
        s.enemies = vec![mk_enemy(10, content::BOSS, boss_hp)];

        apply(&mut s, Input::Clear);
        assert_eq!(s.enemies.len(), 1, "boss survives a single Clear");
        assert_eq!(s.enemies[0].hp, boss_hp - CLEAR_DAMAGE);

        let needed = (boss_hp + CLEAR_DAMAGE - 1) / CLEAR_DAMAGE; // ceil
        for _ in 1..needed {
            s.tick = s.tank.clear_cooldown_end; // come off cooldown
            apply(&mut s, Input::Clear);
        }
        assert!(s.enemies.is_empty(), "boss dies after enough Clears");
        assert_eq!(s.pending_kills.last(), Some(&content::BOSS));
    }
}
