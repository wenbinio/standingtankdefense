//! Byte-level snapshot serialization of `ArenaState` — the BULK-channel wire
//! format (`docs/04 §4.4.6`) used for desync correction and reconnect/replay
//! (`docs/03 §3.7`). Centrally owned because it must mirror the data model in
//! `state.rs` exactly.
//!
//! Guarantees:
//! - **Round-trip identity**: `deserialize(serialize(s)) == s`.
//! - **Deterministic & portable**: little-endian, no padding, no floats, no deps.
//! - Field order mirrors `checksum()` so the two are easy to keep in sync.

use crate::content::{ModEffect, StatusOnHit, WeaponAbility};
use crate::ids::EntityId;
use crate::state::*;
use determinism::{Fixed, Rng};

/// Bump when the on-the-wire layout changes; `deserialize` rejects mismatches.
/// v22: `Modifiers::healing_weapon_healthy_dmg` — Battle Fervor's +35% scoped
/// to HEALING weapons only (`WeaponDef::is_healing`) at ≥95% HP, replacing the
/// documented global-`healthy_dmg` approximation. Checksummed per the parity
/// rule.
/// v21 (FIDELITY PASSES, one combined bump): the economy pass added
/// `Economy::treasure_pool` (Magic Treasure's held, growing gold pool) and
/// `PendingPerk::scope` (the source's per-item perk scoping); the E3
/// mechanics pass added new Tank fields (permanent-regen bonus + carry,
/// Deep-Freeze flag, Frost/Flaming-Armor retaliation stacks, first-hit /
/// Deflection / damage-taken→Spikes riders), new EnemyStatus fields
/// (obscured miss-chance, typed vulnerability stacks, hit-the-tank flag),
/// new Modifiers fields (frost/fire/explosion strength, bounce-barrage
/// targets, heal-conditional damage), and the rotating-wave `sweeps`
/// collection. All checksummed per the parity rule.
/// v20: `Projectile::weapon_kind` (render bookkeeping; snapshot-carried so
/// reconnect redraws correctly, checksummed per the parity rule). The
/// transient `ArenaState::events` buffer and per-tick flags/accumulators are
/// deliberately NOT serialized (`docs/09 §9.3`).
pub const SNAPSHOT_VERSION: u32 = 22;

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
        let end = self
            .pos
            .checked_add(n)
            .ok_or(SnapshotError::UnexpectedEof)?;
        let s = self
            .buf
            .get(self.pos..end)
            .ok_or(SnapshotError::UnexpectedEof)?;
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
    w.i64(s.tank.spikes_damage);
    w.fixed(s.tank.spikes_mult);
    w.fixed(s.tank.shield_active_dr);
    w.i64(s.tank.heal_on_damaged);
    w.i64(s.tank.heal_on_kill);
    w.i64(s.tank.mana_on_kill);
    w.i64(s.tank.heal_on_poison);
    w.fixed(s.tank.healing_mult);
    w.fixed(s.tank.missing_hp_heal_pct);
    w.u32(s.tank.revives);
    w.i64(s.tank.revive_bonus_hp);
    // EXPANSION E2 exotic-mechanic tank state (mirrors the checksum order).
    w.i64(s.tank.shieldbreak_stun_range);
    w.u32(s.tank.shieldbreak_stun_ticks);
    w.i64(s.tank.spikes_poison_dps);
    w.u32(s.tank.spikes_poison_ticks);
    w.i64(s.tank.spikes_stack_per);
    w.u32(s.tank.spikes_stacks);
    w.u32(s.tank.spikes_stacks_max);
    w.i64(s.tank.aura_range);
    w.u32(s.tank.aura_cadence);
    w.i64(s.tank.aura_damage);
    w.i64(s.tank.aura_poison_dps);
    w.u32(s.tank.aura_poison_ticks);
    w.u32(s.tank.aura_tick);
    // EXPANSION E3 fidelity-mechanics tank state (mirrors the checksum order).
    w.fixed(s.tank.regen_bonus_per_tick);
    w.fixed(s.tank.regen_carry);
    w.bool(s.tank.deep_freeze);
    w.u8(s.tank.retaliate_frost);
    w.u16(s.tank.retaliate_fire);
    w.i64(s.tank.spikes_first_hit);
    w.fixed(s.tank.spikes_dr_rate);
    w.fixed(s.tank.dmg_taken_to_spikes);

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
        w.u16(e.status.vuln_stacks);
        w.u32(e.status.stun_ticks);
        w.u32(e.status.freeze_ticks);
        w.u16(e.status.obscure_pct);
        w.u32(e.status.obscure_ticks);
        for v in &e.status.vuln_by_type {
            w.u16(*v);
        }
        w.bool(e.status.hit_tank);
    }

    // projectiles (+ on-hit status)
    w.len(s.projectiles.len());
    for p in &s.projectiles {
        w.id(p.id);
        w.u16(p.weapon_kind);
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
        let (atag, a, b, c) = p.ability.words();
        w.u8(atag);
        w.i64(a);
        w.i64(b);
        w.i64(c);
    }

    // hazards (land mines / burning oil)
    w.len(s.hazards.len());
    for h in &s.hazards {
        w.id(h.id);
        w.vec2(h.pos);
        w.i64(h.dmg);
        w.u8(h.damage_type);
        w.i64(h.radius);
        w.u32(h.ticks_left);
    }

    // minions (summoned Larvae / Spores)
    w.len(s.minions.len());
    for m in &s.minions {
        w.id(m.id);
        w.vec2(m.pos);
        w.u8(m.kind);
        w.i64(m.hp);
        w.i64(m.damage);
        w.u8(m.damage_type);
        w.u32(m.next_attack_tick);
        w.u32(m.expire_tick);
    }

    // rotating-wave sweeps
    w.len(s.sweeps.len());
    for sw in &s.sweeps {
        w.id(sw.id);
        w.u16(sw.weapon_kind);
        w.i64(sw.damage);
        w.u8(sw.damage_type);
        w.fixed(sw.radius);
        w.u16(sw.angle_bam);
        w.u16(sw.step_bam);
        w.u32(sw.ticks_left);
        w.bool(sw.clockwise);
        w.i64(sw.on_hit.poison_dps);
        w.u32(sw.on_hit.poison_ticks);
        w.u8(sw.on_hit.frost_stacks);
        w.u16(sw.on_hit.fire_stacks);
        w.u32(sw.on_hit.stun_ticks);
        let (atag, a, b, c) = sw.ability.words();
        w.u8(atag);
        w.i64(a);
        w.i64(b);
        w.i64(c);
    }

    // economy
    w.i64(s.economy.gold);
    w.i64(s.economy.income_per_tick);
    w.fixed(s.economy.income_mult);
    w.fixed(s.economy.income_regen_pct);
    w.fixed(s.economy.bounty_mult);
    w.i64(s.economy.bounty_proc_chance_pct);
    w.fixed(s.economy.bounty_proc_bonus);
    w.fixed(s.economy.gold_per_damage);
    w.fixed(s.economy.income_shield_pct);
    w.u32(s.economy.rerolls_remaining);
    w.i64(s.economy.reroll_cost);
    w.i64(s.economy.treasure_pool);

    // modifiers
    w.fixed(s.modifiers.add_global);
    for a in &s.modifiers.add_by_type {
        w.fixed(*a);
    }
    for a in &s.modifiers.add_by_scope {
        w.fixed(*a);
    }
    w.fixed(s.modifiers.mul_global);
    w.fixed(s.modifiers.attack_speed);
    w.fixed(s.modifiers.vs_stunned);
    w.fixed(s.modifiers.vs_poisoned);
    w.fixed(s.modifiers.poison_dmg_mult);
    w.fixed(s.modifiers.stun_dur_mult);
    w.len(s.modifiers.weapon_count_scaling.len());
    for r in &s.modifiers.weapon_count_scaling {
        w.u16(r.weapon_def);
        w.u8(r.dmg_type);
        w.fixed(r.per);
    }
    w.fixed(s.modifiers.dmg_per_maxhp_rate);
    w.fixed(s.modifiers.dmg_per_bounty_rate);
    w.fixed(s.modifiers.shield_active_dmg);
    w.fixed(s.modifiers.frost_strength_mult);
    w.fixed(s.modifiers.fire_dmg_mult);
    w.fixed(s.modifiers.fire_explosion_mult);
    w.fixed(s.modifiers.bounce_barrage_pct);
    w.fixed(s.modifiers.healthy_dmg);
    w.fixed(s.modifiers.healing_weapon_healthy_dmg);

    // active ramps
    w.len(s.ramps.len());
    for r in &s.ramps {
        let (tag, a, b, c) = r.effect.words();
        w.u8(tag);
        w.i64(a);
        w.i64(b);
        w.i64(c);
        w.u32(r.interval_ticks);
        w.u32(r.next_apply);
    }

    // active vulnerability pulses
    w.len(s.vuln_pulses.len());
    for p in &s.vuln_pulses {
        w.u16(p.magnitude);
        w.i64(p.range);
        w.u32(p.interval_ticks);
        w.u32(p.next_tick);
    }

    // pending meta perk (duplicator/voucher)
    match s.pending_perk {
        Some(p) => {
            w.u8(1);
            w.u8(p.rarity);
            w.u8(p.scope.as_u8());
            w.u32(p.extra_copies);
            w.u8(p.free as u8);
        }
        None => w.u8(0),
    }

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
    w.i64(s.total_damage_dealt);
    w.i64(s.total_gold_earned);
    w.i64(s.bought_attack_mask as i64);
    w.i64(s.weapons_bought as i64);
    w.i64(s.economy_purchases as i64);

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
        spikes_damage: r.i64()?,
        spikes_mult: r.fixed()?,
        shield_active_dr: r.fixed()?,
        heal_on_damaged: r.i64()?,
        heal_on_kill: r.i64()?,
        mana_on_kill: r.i64()?,
        heal_on_poison: r.i64()?,
        healing_mult: r.fixed()?,
        missing_hp_heal_pct: r.fixed()?,
        revives: r.u32()?,
        revive_bonus_hp: r.i64()?,
        shieldbreak_stun_range: r.i64()?,
        shieldbreak_stun_ticks: r.u32()?,
        spikes_poison_dps: r.i64()?,
        spikes_poison_ticks: r.u32()?,
        spikes_stack_per: r.i64()?,
        spikes_stacks: r.u32()?,
        spikes_stacks_max: r.u32()?,
        aura_range: r.i64()?,
        aura_cadence: r.u32()?,
        aura_damage: r.i64()?,
        aura_poison_dps: r.i64()?,
        aura_poison_ticks: r.u32()?,
        aura_tick: r.u32()?,
        regen_bonus_per_tick: r.fixed()?,
        regen_carry: r.fixed()?,
        deep_freeze: r.bool()?,
        retaliate_frost: r.u8()?,
        retaliate_fire: r.u16()?,
        spikes_first_hit: r.i64()?,
        spikes_dr_rate: r.fixed()?,
        dmg_taken_to_spikes: r.fixed()?,
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
                vuln_stacks: r.u16()?,
                stun_ticks: r.u32()?,
                freeze_ticks: r.u32()?,
                obscure_pct: r.u16()?,
                obscure_ticks: r.u32()?,
                vuln_by_type: [r.u16()?, r.u16()?, r.u16()?, r.u16()?, r.u16()?],
                hit_tank: r.bool()?,
            },
        });
    }

    let mut projectiles = Vec::new();
    for _ in 0..r.len()? {
        projectiles.push(Projectile {
            id: r.id()?,
            weapon_kind: r.u16()?,
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
            ability: {
                let tag = r.u8()?;
                let a = r.i64()?;
                let b = r.i64()?;
                let c = r.i64()?;
                WeaponAbility::from_words(tag, a, b, c).ok_or(SnapshotError::BadTag(tag))?
            },
        });
    }

    let mut hazards = Vec::new();
    for _ in 0..r.len()? {
        hazards.push(Hazard {
            id: r.id()?,
            pos: r.vec2()?,
            dmg: r.i64()?,
            damage_type: r.u8()?,
            radius: r.i64()?,
            ticks_left: r.u32()?,
        });
    }

    let mut minions = Vec::new();
    for _ in 0..r.len()? {
        minions.push(Minion {
            id: r.id()?,
            pos: r.vec2()?,
            kind: r.u8()?,
            hp: r.i64()?,
            damage: r.i64()?,
            damage_type: r.u8()?,
            next_attack_tick: r.u32()?,
            expire_tick: r.u32()?,
        });
    }

    let mut sweeps = Vec::new();
    for _ in 0..r.len()? {
        sweeps.push(WaveSweep {
            id: r.id()?,
            weapon_kind: r.u16()?,
            damage: r.i64()?,
            damage_type: r.u8()?,
            radius: r.fixed()?,
            angle_bam: r.u16()?,
            step_bam: r.u16()?,
            ticks_left: r.u32()?,
            clockwise: r.bool()?,
            on_hit: StatusOnHit {
                poison_dps: r.i64()?,
                poison_ticks: r.u32()?,
                frost_stacks: r.u8()?,
                fire_stacks: r.u16()?,
                stun_ticks: r.u32()?,
            },
            ability: {
                let tag = r.u8()?;
                let a = r.i64()?;
                let b = r.i64()?;
                let c = r.i64()?;
                WeaponAbility::from_words(tag, a, b, c).ok_or(SnapshotError::BadTag(tag))?
            },
        });
    }

    let economy = Economy {
        gold: r.i64()?,
        income_per_tick: r.i64()?,
        income_mult: r.fixed()?,
        income_regen_pct: r.fixed()?,
        bounty_mult: r.fixed()?,
        bounty_proc_chance_pct: r.i64()?,
        bounty_proc_bonus: r.fixed()?,
        gold_per_damage: r.fixed()?,
        income_shield_pct: r.fixed()?,
        rerolls_remaining: r.u32()?,
        reroll_cost: r.i64()?,
        treasure_pool: r.i64()?,
    };

    let add_global = r.fixed()?;
    let add_by_type = [r.fixed()?, r.fixed()?, r.fixed()?, r.fixed()?, r.fixed()?];
    let mut add_by_scope = [Fixed::ZERO; 12];
    for x in add_by_scope.iter_mut() {
        *x = r.fixed()?;
    }
    let modifiers = Modifiers {
        add_global,
        add_by_type,
        add_by_scope,
        mul_global: r.fixed()?,
        attack_speed: r.fixed()?,
        vs_stunned: r.fixed()?,
        vs_poisoned: r.fixed()?,
        poison_dmg_mult: r.fixed()?,
        stun_dur_mult: r.fixed()?,
        weapon_count_scaling: {
            let mut v = Vec::new();
            for _ in 0..r.len()? {
                v.push(WeaponCountScale {
                    weapon_def: r.u16()?,
                    dmg_type: r.u8()?,
                    per: r.fixed()?,
                });
            }
            v
        },
        dmg_per_maxhp_rate: r.fixed()?,
        dmg_per_bounty_rate: r.fixed()?,
        shield_active_dmg: r.fixed()?,
        frost_strength_mult: r.fixed()?,
        fire_dmg_mult: r.fixed()?,
        fire_explosion_mult: r.fixed()?,
        bounce_barrage_pct: r.fixed()?,
        healthy_dmg: r.fixed()?,
        healing_weapon_healthy_dmg: r.fixed()?,
    };

    let mut ramps = Vec::new();
    for _ in 0..r.len()? {
        let tag = r.u8()?;
        let a = r.i64()?;
        let b = r.i64()?;
        let c = r.i64()?;
        let effect = ModEffect::from_words(tag, a, b, c).ok_or(SnapshotError::BadTag(tag))?;
        ramps.push(ActiveRamp {
            effect,
            interval_ticks: r.u32()?,
            next_apply: r.u32()?,
        });
    }

    let mut vuln_pulses = Vec::new();
    for _ in 0..r.len()? {
        vuln_pulses.push(VulnPulse {
            magnitude: r.u16()?,
            range: r.i64()?,
            interval_ticks: r.u32()?,
            next_tick: r.u32()?,
        });
    }

    let pending_perk = match r.u8()? {
        0 => None,
        1 => Some(PendingPerk {
            rarity: r.u8()?,
            scope: {
                let tag = r.u8()?;
                PerkScope::from_u8(tag).ok_or(SnapshotError::BadTag(tag))?
            },
            extra_copies: r.u32()?,
            free: r.u8()? != 0,
        }),
        t => return Err(SnapshotError::BadTag(t)),
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
    let total_damage_dealt = r.i64()?;
    let total_gold_earned = r.i64()?;
    let bought_attack_mask = r.i64()? as u16;
    let weapons_bought = r.i64()? as u32;
    let economy_purchases = r.i64()? as u32;

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
        hazards,
        minions,
        sweeps,
        economy,
        shop,
        modifiers,
        ramps,
        vuln_pulses,
        pending_perk,
        tank_hit_this_tick: false,
        shield_broke_this_tick: false,
        damage_taken_this_tick: 0,
        spikes_first_bonus_this_tick: 0,
        events: Events::default(),
        next_entity_id,
        dead,
        death_tick,
        pending_kills,
        total_damage_dealt,
        total_gold_earned,
        bought_attack_mask,
        weapons_bought,
        economy_purchases,
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
    fn roundtrip_preserves_e2_exotic_tank_state() {
        // Set every EXPANSION E2 field (incl. the dynamic counters) to non-default
        // values and confirm the snapshot/checksum round-trips them.
        let mut s = ArenaState::new(0xE2E2, 1);
        s.tank.shieldbreak_stun_range = 1200;
        s.tank.shieldbreak_stun_ticks = 15;
        s.tank.spikes_poison_dps = 2;
        s.tank.spikes_poison_ticks = 90;
        s.tank.spikes_stack_per = 20;
        s.tank.spikes_stacks = 7;
        s.tank.spikes_stacks_max = 25;
        s.tank.aura_range = 600;
        s.tank.aura_cadence = 30;
        s.tank.aura_damage = 200;
        s.tank.aura_poison_dps = 2;
        s.tank.aura_poison_ticks = 90;
        s.tank.aura_tick = 17;
        let back = deserialize(&serialize(&s)).unwrap();
        assert_eq!(s, back);
        assert_eq!(checksum(&s), checksum(&back));
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
            assert_eq!(
                checksum(&a),
                checksum(&b),
                "diverged after restore at {tick}"
            );
        }
    }

    #[test]
    fn bad_version_is_rejected() {
        let mut bytes = serialize(&ArenaState::new(1, 0));
        bytes[0] = 0xFF; // corrupt version
        assert!(matches!(
            deserialize(&bytes),
            Err(SnapshotError::BadVersion(_))
        ));
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
