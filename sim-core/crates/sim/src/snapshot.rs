//! Byte-level snapshot serialization of `ArenaState` — the BULK-channel wire
//! format (`docs/04 §4.4.6`) used for desync correction and reconnect/replay
//! (`docs/03 §3.7`). Centrally owned because it must mirror the data model in
//! `state.rs` exactly.
//!
//! Guarantees:
//! - **Round-trip identity**: `deserialize(serialize(s)) == s`.
//! - **Deterministic & portable**: little-endian, no padding, no floats, no deps.
//! - Field order mirrors `checksum()` so the two are easy to keep in sync.

use crate::ids::EntityId;
use crate::content::StatusOnHit;
use crate::state::*;
use determinism::{Fixed, Rng};

/// Bump when the on-the-wire layout changes; `deserialize` rejects mismatches.
pub const SNAPSHOT_VERSION: u32 = 3;

#[derive(Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// Ran out of bytes while decoding.
    UnexpectedEof,
    /// Header version did not match [`SNAPSHOT_VERSION`].
    BadVersion(u32),
    /// A tagged union / bool had an out-of-range tag.
    BadTag(u8),
    /// Trailing bytes remained after a full decode.
    TrailingBytes,
}

// ----------------------------- writer -----------------------------

struct W {
    buf: Vec<u8>,
}
impl W {
    fn new() -> W {
        W { buf: Vec::new() }
    }
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn fixed(&mut self, v: Fixed) {
        self.i64(v.raw());
    }
    fn vec2(&mut self, v: Vec2) {
        self.fixed(v.x);
        self.fixed(v.y);
    }
    fn id(&mut self, v: EntityId) {
        self.u32(v.0);
    }
    fn rng(&mut self, v: Rng) {
        self.u64(v.state());
    }
    fn opt_u32(&mut self, v: Option<u32>) {
        match v {
            None => self.u8(0),
            Some(x) => {
                self.u8(1);
                self.u32(x);
            }
        }
    }
    fn len(&mut self, n: usize) {
        self.u32(n as u32);
    }
}

// ----------------------------- reader -----------------------------

struct R<'a> {
    buf: &'a [u8],
    pos: usize,
}
impl<'a> R<'a> {
    fn new(buf: &'a [u8]) -> R<'a> {
        R { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], SnapshotError> {
        let end = self.pos.checked_add(n).ok_or(SnapshotError::UnexpectedEof)?;
        let s = self.buf.get(self.pos..end).ok_or(SnapshotError::UnexpectedEof)?;
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, SnapshotError> {
        Ok(self.take(1)?[0])
    }
    fn bool(&mut self) -> Result<bool, SnapshotError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            t => Err(SnapshotError::BadTag(t)),
        }
    }
    fn u16(&mut self) -> Result<u16, SnapshotError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, SnapshotError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, SnapshotError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, SnapshotError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn fixed(&mut self) -> Result<Fixed, SnapshotError> {
        Ok(Fixed::from_raw(self.i64()?))
    }
    fn vec2(&mut self) -> Result<Vec2, SnapshotError> {
        Ok(Vec2::new(self.fixed()?, self.fixed()?))
    }
    fn id(&mut self) -> Result<EntityId, SnapshotError> {
        Ok(EntityId(self.u32()?))
    }
    fn rng(&mut self) -> Result<Rng, SnapshotError> {
        Ok(Rng::from_seed(self.u64()?))
    }
    fn opt_u32(&mut self) -> Result<Option<u32>, SnapshotError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.u32()?)),
            t => Err(SnapshotError::BadTag(t)),
        }
    }
    fn len(&mut self) -> Result<usize, SnapshotError> {
        Ok(self.u32()? as usize)
    }
}

// --------------------------- serialize ----------------------------

/// Serialize an arena to a portable byte snapshot.
pub fn serialize(s: &ArenaState) -> Vec<u8> {
    let mut w = W::new();
    w.u32(SNAPSHOT_VERSION);

    w.u32(s.tick);
    w.u32(s.round);
    w.u64(s.master_seed);
    w.u32(s.player_id);

    // tank
    w.i64(s.tank.hp);
    w.i64(s.tank.max_hp);
    w.vec2(s.tank.pos);
    w.u32(s.tank.clear_cooldown_end);
    w.i64(s.tank.armor);
    w.u32(s.tank.dodge_num);
    w.u32(s.tank.dodge_den);
    w.i64(s.tank.mana_shield);
    w.i64(s.tank.mana_shield_max);
    w.i64(s.tank.mana_regen_per_tick);
    w.i64(s.tank.hp_regen_per_tick);

    // weapons
    w.len(s.weapons.len());
    for wi in &s.weapons {
        w.id(wi.instance_id);
        w.u16(wi.def);
        w.u32(wi.next_fire_tick);
    }

    // enemies (+ status)
    w.len(s.enemies.len());
    for e in &s.enemies {
        w.id(e.id);
        w.u16(e.def);
        w.i64(e.hp);
        w.vec2(e.pos);
        w.i64(e.status.poison_dps);
        w.u32(e.status.poison_ticks);
        w.u8(e.status.frost_stacks);
        w.u32(e.status.frost_ticks);
        w.u16(e.status.fire_stacks);
        w.u32(e.status.stun_ticks);
    }

    // projectiles (+ on-hit status)
    w.len(s.projectiles.len());
    for p in &s.projectiles {
        w.id(p.id);
        w.vec2(p.pos);
        w.id(p.target);
        w.vec2(p.last_target_pos);
        w.i64(p.damage);
        w.u8(p.damage_type);
        w.fixed(p.splash_radius);
        w.fixed(p.speed);
        w.i64(p.on_hit.poison_dps);
        w.u32(p.on_hit.poison_ticks);
        w.u8(p.on_hit.frost_stacks);
        w.u16(p.on_hit.fire_stacks);
        w.u32(p.on_hit.stun_ticks);
    }

    // economy
    w.i64(s.economy.gold);
    w.i64(s.economy.income_per_tick);
    w.fixed(s.economy.bounty_mult);
    w.u32(s.economy.rerolls_remaining);
    w.i64(s.economy.reroll_cost);

    // modifiers
    w.fixed(s.modifiers.add_global);
    for a in &s.modifiers.add_by_type {
        w.fixed(*a);
    }
    w.fixed(s.modifiers.mul_global);
    w.fixed(s.modifiers.attack_speed);

    // shop
    w.u32(s.shop.shop_seq);
    w.len(s.shop.offers.len());
    for o in &s.shop.offers {
        w.u8(match o.kind {
            OfferKind::Weapon => 0,
            OfferKind::Modifier => 1,
        });
        w.u16(o.def);
        w.i64(o.cost);
    }

    // bookkeeping
    w.u32(s.next_entity_id);
    w.bool(s.dead);
    w.opt_u32(s.death_tick);
    w.len(s.pending_kills.len());
    for k in &s.pending_kills {
        w.u16(*k);
    }

    // rng cursors
    w.rng(s.rng_spawn);
    w.rng(s.rng_targeting);
    w.rng(s.rng_shop);
    w.rng(s.rng_reroll);
    w.rng(s.rng_proc);

    w.buf
}

// -------------------------- deserialize ---------------------------

/// Reconstruct an arena from a byte snapshot. Inverse of [`serialize`].
pub fn deserialize(bytes: &[u8]) -> Result<ArenaState, SnapshotError> {
    let mut r = R::new(bytes);
    let ver = r.u32()?;
    if ver != SNAPSHOT_VERSION {
        return Err(SnapshotError::BadVersion(ver));
    }

    let tick = r.u32()?;
    let round = r.u32()?;
    let master_seed = r.u64()?;
    let player_id = r.u32()?;

    let tank = Tank {
        hp: r.i64()?,
        max_hp: r.i64()?,
        pos: r.vec2()?,
        clear_cooldown_end: r.u32()?,
        armor: r.i64()?,
        dodge_num: r.u32()?,
        dodge_den: r.u32()?,
        mana_shield: r.i64()?,
        mana_shield_max: r.i64()?,
        mana_regen_per_tick: r.i64()?,
        hp_regen_per_tick: r.i64()?,
    };

    let mut weapons = Vec::new();
    for _ in 0..r.len()? {
        weapons.push(WeaponInstance {
            instance_id: r.id()?,
            def: r.u16()?,
            next_fire_tick: r.u32()?,
        });
    }

    let mut enemies = Vec::new();
    for _ in 0..r.len()? {
        enemies.push(Enemy {
            id: r.id()?,
            def: r.u16()?,
            hp: r.i64()?,
            pos: r.vec2()?,
            status: EnemyStatus {
                poison_dps: r.i64()?,
                poison_ticks: r.u32()?,
                frost_stacks: r.u8()?,
                frost_ticks: r.u32()?,
                fire_stacks: r.u16()?,
                stun_ticks: r.u32()?,
            },
        });
    }

    let mut projectiles = Vec::new();
    for _ in 0..r.len()? {
        projectiles.push(Projectile {
            id: r.id()?,
            pos: r.vec2()?,
            target: r.id()?,
            last_target_pos: r.vec2()?,
            damage: r.i64()?,
            damage_type: r.u8()?,
            splash_radius: r.fixed()?,
            speed: r.fixed()?,
            on_hit: StatusOnHit {
                poison_dps: r.i64()?,
                poison_ticks: r.u32()?,
                frost_stacks: r.u8()?,
                fire_stacks: r.u16()?,
                stun_ticks: r.u32()?,
            },
        });
    }

    let economy = Economy {
        gold: r.i64()?,
        income_per_tick: r.i64()?,
        bounty_mult: r.fixed()?,
        rerolls_remaining: r.u32()?,
        reroll_cost: r.i64()?,
    };

    let modifiers = Modifiers {
        add_global: r.fixed()?,
        add_by_type: [r.fixed()?, r.fixed()?, r.fixed()?, r.fixed()?, r.fixed()?],
        mul_global: r.fixed()?,
        attack_speed: r.fixed()?,
    };

    let shop_seq = r.u32()?;
    let mut offers = Vec::new();
    for _ in 0..r.len()? {
        let kind = match r.u8()? {
            0 => OfferKind::Weapon,
            1 => OfferKind::Modifier,
            t => return Err(SnapshotError::BadTag(t)),
        };
        offers.push(Offer {
            kind,
            def: r.u16()?,
            cost: r.i64()?,
        });
    }
    let shop = ShopState { offers, shop_seq };

    let next_entity_id = r.u32()?;
    let dead = r.bool()?;
    let death_tick = r.opt_u32()?;
    let mut pending_kills = Vec::new();
    for _ in 0..r.len()? {
        pending_kills.push(r.u16()?);
    }

    let rng_spawn = r.rng()?;
    let rng_targeting = r.rng()?;
    let rng_shop = r.rng()?;
    let rng_reroll = r.rng()?;
    let rng_proc = r.rng()?;

    if r.pos != bytes.len() {
        return Err(SnapshotError::TrailingBytes);
    }

    Ok(ArenaState {
        tick,
        round,
        master_seed,
        player_id,
        tank,
        weapons,
        enemies,
        projectiles,
        economy,
        shop,
        modifiers,
        next_entity_id,
        dead,
        death_tick,
        pending_kills,
        rng_spawn,
        rng_targeting,
        rng_shop,
        rng_reroll,
        rng_proc,
    })
}

// ----------------------------- tests ------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::Input;
    use crate::{checksum, step};

    /// Drive a varied state, snapshotting/restoring at intervals.
    fn scripted_input(tick: u32) -> Input {
        match tick {
            5 => Input::BuyOffer { slot: 0 },
            40 => Input::BuyOffer { slot: 1 },
            120 => Input::Reroll,
            125 => Input::BuyOffer { slot: 0 },
            300 => Input::Clear,
            _ => Input::Noop,
        }
    }

    #[test]
    fn roundtrip_fresh_state() {
        let s = ArenaState::new(0xDEAD_BEEF, 3);
        let back = deserialize(&serialize(&s)).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn roundtrip_along_a_run_preserves_state_and_checksum() {
        let mut s = ArenaState::new(0x1234_5678_9ABC_DEF0, 0);
        for tick in 0..1500u32 {
            step(&mut s, scripted_input(tick));
            if tick % 37 == 0 {
                let bytes = serialize(&s);
                let back = deserialize(&bytes).unwrap();
                assert_eq!(s, back, "state mismatch at tick {tick}");
                assert_eq!(checksum(&s), checksum(&back), "checksum mismatch at {tick}");
                // re-serializing the restored state is byte-identical
                assert_eq!(bytes, serialize(&back), "byte mismatch at {tick}");
            }
        }
    }

    #[test]
    fn restored_state_continues_identically() {
        // Snapshot at T, then continuing from the snapshot must match continuing
        // the original (the reconnect/replay property, docs/04 §4.4.6).
        let mut a = ArenaState::new(0xABCD_1234, 1);
        for tick in 0..400u32 {
            step(&mut a, scripted_input(tick));
        }
        let mut b = deserialize(&serialize(&a)).unwrap();
        for tick in 400..900u32 {
            step(&mut a, scripted_input(tick));
            step(&mut b, scripted_input(tick));
            assert_eq!(checksum(&a), checksum(&b), "diverged after restore at {tick}");
        }
    }

    #[test]
    fn bad_version_is_rejected() {
        let mut bytes = serialize(&ArenaState::new(1, 0));
        bytes[0] = 0xFF; // corrupt version
        assert!(matches!(deserialize(&bytes), Err(SnapshotError::BadVersion(_))));
    }

    #[test]
    fn truncated_is_rejected() {
        let bytes = serialize(&ArenaState::new(1, 0));
        assert_eq!(
            deserialize(&bytes[..bytes.len() - 3]),
            Err(SnapshotError::UnexpectedEof)
        );
    }
}
