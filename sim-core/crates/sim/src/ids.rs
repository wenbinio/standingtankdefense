//! Shared id and enum types for the simulation. Owned centrally — behavior
//! modules must not change these definitions.

/// Simulation tick counter (30 Hz). The only notion of time in the sim.
pub type Tick = u32;

/// Per-arena entity id. Monotonic; never reused within a match.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EntityId(pub u32);

/// Independent RNG stream purposes (`docs/05 §5.6`). Values are stable on the
/// wire/checksum — do not reorder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum Purpose {
    Spawn = 0,
    Targeting = 1,
    Shop = 2,
    Reroll = 3,
    Proc = 4,
}

/// One player action applied at a single tick (`docs/04 §4.4.3`, M0 subset).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    /// No action this tick.
    Noop,
    /// Purchase the offer in shop slot `slot`.
    BuyOffer { slot: u8 },
    /// Refresh the shop offers (consumes a reroll or charges gold).
    Reroll,
    /// Fire the manual `Clear` ability (also the boss-damage action).
    Clear,
    /// Redeem a held Black Market pick (`ArenaState::pending_black_market`):
    /// grant, free of charge, the Uncommon weapon (`is_weapon = true`, `index`
    /// into `content::WEAPONS`) or the Uncommon Spikes-damage upgrade
    /// (`is_weapon = false`, `index` into `content::MODIFIERS`) of the player's
    /// choosing — the source's "Buy 1 Uncommon Weapon or Spikes Damage Upgrade
    /// of your choosing. The Black Market lasts until a choice is made."
    /// (`research/tower-survivors-map/parsed/catalog.json`). Illegal picks (no
    /// pending pick / index out of range / wrong rarity / non-Spikes upgrade)
    /// are deterministic no-ops (`docs/04 §4.6` anti-cheat discipline). `u8`
    /// covers both catalogs (96 weapons / 110 modifiers).
    BlackMarketPick { is_weapon: bool, index: u8 },
}
