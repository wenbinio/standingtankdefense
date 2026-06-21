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
use crate::state::ArenaState;
use std::collections::BTreeMap;

/// A full render snapshot for one tick.
#[derive(Clone, Debug)]
pub struct RenderView {
    pub tick: u32,
    pub round: u32,
    pub dead: bool,
    pub tank: RenderTank,
    pub enemies: Vec<RenderEnemy>,
    pub projectiles: Vec<RenderPoint>,
    pub economy: RenderEconomy,
    pub shop: Vec<RenderOffer>,
    /// Owned weapons collapsed to `(name, count)`, sorted by name.
    pub arsenal: Vec<RenderArsenalEntry>,
}

/// World-space integer point (units; tank sits at the origin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderPoint {
    pub x: i64,
    pub y: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderTank {
    pub x: i64,
    pub y: i64,
    pub hp: i64,
    pub max_hp: i64,
    pub revives: u32,
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
    pub name: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderEconomy {
    pub gold: i64,
    /// Effective passive income per tick (base × income multiplier).
    pub income_per_tick: i64,
    pub rerolls_remaining: u32,
    pub reroll_cost: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderOffer {
    pub slot: u8,
    pub name: &'static str,
    pub cost: i64,
    pub is_weapon: bool,
    pub affordable: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderArsenalEntry {
    pub name: &'static str,
    pub count: u32,
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
                name: def.name,
            }
        })
        .collect();

    let projectiles = s
        .projectiles
        .iter()
        .map(|p| RenderPoint {
            x: p.pos.x.floor_to_int(),
            y: p.pos.y.floor_to_int(),
        })
        .collect();

    let economy = RenderEconomy {
        gold: s.economy.gold,
        income_per_tick: s.economy.income_mult.scale_i64(s.economy.income_per_tick),
        rerolls_remaining: s.economy.rerolls_remaining,
        reroll_cost: s.economy.reroll_cost,
    };

    let shop = s
        .shop
        .offers
        .iter()
        .enumerate()
        .map(|(i, off)| {
            let (name, is_weapon) = match off.kind {
                crate::state::OfferKind::Weapon => (content::WEAPONS[off.def as usize].name, true),
                crate::state::OfferKind::Modifier => (content::MODIFIERS[off.def as usize].name, false),
            };
            RenderOffer {
                slot: i as u8,
                name,
                cost: off.cost,
                is_weapon,
                affordable: off.cost <= s.economy.gold,
            }
        })
        .collect();

    let mut counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    for w in &s.weapons {
        *counts.entry(content::WEAPONS[w.def as usize].name).or_insert(0) += 1;
    }
    let arsenal = counts
        .into_iter()
        .map(|(name, count)| RenderArsenalEntry { name, count })
        .collect();

    RenderView {
        tick: s.tick,
        round: s.round,
        dead: s.dead,
        tank,
        enemies,
        projectiles,
        economy,
        shop,
        arsenal,
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
    fn shop_affordability_tracks_gold() {
        let mut s = ArenaState::new(7, 0);
        step(&mut s, Input::Noop); // generate offers (round 0 boundary at tick 0)
        s.economy.gold = 0;
        let v = snapshot(&s);
        assert!(v.shop.iter().all(|o| !o.affordable), "broke ⇒ nothing affordable");
        s.economy.gold = i64::MAX / 2;
        let v = snapshot(&s);
        assert!(v.shop.iter().all(|o| o.affordable), "rich ⇒ all affordable");
    }
}
