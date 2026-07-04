//! Snapshot↔checksum parity — the tripwire for the B4 bug class.
//!
//! `snapshot::serialize` and `checksum` must cover the SAME state: a field that
//! rides the snapshot (i.e. survives reconnect/correction and can differ
//! between two arenas) but is absent from `checksum()` is a silent desync hole
//! — two clients could diverge in that field and the digest exchange would
//! never notice (exactly what happened with `Projectile::last_target_pos`).
//!
//! Method (deliberately plain, not clever): for every snapshot-carried mutable
//! field, build two states differing ONLY in that field and assert that BOTH
//! the serialized bytes AND the checksum differ. The byte assertion keeps the
//! test honest (the mutation really is snapshot-carried); the checksum
//! assertion is the parity property itself.
//!
//! When you add a field to `ArenaState` (or a nested struct): add it to
//! `snapshot.rs`, to `checksum()`, and to the list below. Transient intra-tick
//! flags (`tank_hit_this_tick`, `shield_broke_this_tick`) are excluded by
//! design — they are always `false` at a tick boundary and are neither
//! serialized nor checksummed. The render-event buffer (`ArenaState::events`,
//! `docs/09 §9.3`) is excluded the same way BY DESIGN: it is cleared at the top
//! of every `step()`, is a pure derivation of checksummed state, never crosses
//! the wire, and compares equal regardless of contents — so it cannot desync
//! and must never enter the snapshot or the checksum.

use determinism::{Fixed, Rng};
use sim::content::{ModEffect, StatusOnHit, WeaponAbility};
use sim::*;

/// A base arena with at least one of every entity/collection so per-element
/// fields can be mutated. Values are arbitrary but fixed.
fn base() -> ArenaState {
    let mut s = ArenaState::new(0xB4_B4B4, 1);
    let f = Fixed::from_int;
    s.weapons.push(WeaponInstance {
        instance_id: EntityId(100),
        def: 0,
        next_fire_tick: 5,
    });
    s.enemies
        .push(Enemy::new(EntityId(101), 0, 500, Vec2::new(f(10), f(-4))));
    s.projectiles.push(Projectile {
        id: EntityId(102),
        weapon_kind: 0,
        pos: Vec2::new(f(1), f(2)),
        target: EntityId(101),
        last_target_pos: Vec2::new(f(10), f(-4)),
        damage: 25,
        damage_type: 1,
        splash_radius: f(2),
        speed: f(9),
        on_hit: StatusOnHit::NONE,
        ability: WeaponAbility::None,
    });
    s.hazards.push(Hazard {
        id: EntityId(103),
        pos: Vec2::new(f(3), f(4)),
        dmg: 7,
        damage_type: 2,
        radius: 300,
        ticks_left: 45,
    });
    s.minions.push(Minion {
        id: EntityId(104),
        pos: Vec2::new(f(-2), f(6)),
        kind: 0,
        hp: 50,
        damage: 9,
        damage_type: 0,
        next_attack_tick: 12,
        expire_tick: 200,
    });
    s.sweeps.push(WaveSweep {
        id: EntityId(105),
        weapon_kind: 91,
        damage: 300,
        damage_type: 2,
        radius: f(450),
        angle_bam: 0,
        step_bam: 2048,
        ticks_left: 16,
        clockwise: false,
        on_hit: StatusOnHit::NONE,
        ability: WeaponAbility::None,
    });
    s.ramps.push(ActiveRamp {
        effect: ModEffect::IncomeFlat(3),
        interval_ticks: 900,
        next_apply: 900,
    });
    s.vuln_pulses.push(VulnPulse {
        magnitude: 1,
        range: 500,
        interval_ticks: 15,
        next_tick: 20,
    });
    s.modifiers.weapon_count_scaling.push(WeaponCountScale {
        weapon_def: 0,
        dmg_type: 1,
        per: Fixed::from_ratio(1, 100),
    });
    s.shop.offers.push(Offer {
        kind: OfferKind::Weapon,
        def: 0,
        cost: 100,
    });
    s.pending_kills.push(2);
    s
}

/// Two states differing only in `mutate` must differ in BOTH snapshot bytes
/// and checksum.
fn parity(name: &str, mutate: impl FnOnce(&mut ArenaState)) {
    parity2(name, |_| {}, mutate);
}

/// General form: apply `ma` to one copy of the base and `mb` to the other
/// (for fields that need a non-default base on both sides, e.g. `pending_perk`
/// sub-fields).
fn parity2(name: &str, ma: impl FnOnce(&mut ArenaState), mb: impl FnOnce(&mut ArenaState)) {
    let mut a = base();
    let mut b = base();
    ma(&mut a);
    mb(&mut b);
    assert_ne!(
        snapshot::serialize(&a),
        snapshot::serialize(&b),
        "{name}: mutation is not snapshot-carried — update snapshot.rs or this test"
    );
    assert_ne!(
        checksum(&a),
        checksum(&b),
        "{name}: snapshot-carried field is MISSING from checksum() (B4 bug class)"
    );
}

#[test]
fn header_and_bookkeeping_fields_feed_checksum() {
    parity("tick", |s| s.tick = 9);
    parity("round", |s| s.round = 3);
    parity("master_seed", |s| s.master_seed ^= 1);
    parity("player_id", |s| s.player_id = 7);
    parity("next_entity_id", |s| s.next_entity_id += 1);
    parity("dead", |s| s.dead = true);
    parity("death_tick", |s| s.death_tick = Some(5));
    parity("pending_kills[0]", |s| s.pending_kills[0] = 3);
    parity("pending_kills.len", |s| s.pending_kills.push(4));
    parity("total_damage_dealt", |s| s.total_damage_dealt += 1);
    parity("total_gold_earned", |s| s.total_gold_earned += 1);
    parity("bought_attack_mask", |s| s.bought_attack_mask |= 1);
    parity("weapons_bought", |s| s.weapons_bought += 1);
    parity("economy_purchases", |s| s.economy_purchases += 1);
    parity("rng_spawn", |s| s.rng_spawn = Rng::from_seed(1));
    parity("rng_targeting", |s| s.rng_targeting = Rng::from_seed(2));
    parity("rng_shop", |s| s.rng_shop = Rng::from_seed(3));
    parity("rng_reroll", |s| s.rng_reroll = Rng::from_seed(4));
    parity("rng_proc", |s| s.rng_proc = Rng::from_seed(5));
}

#[test]
fn tank_fields_feed_checksum() {
    let f = Fixed::from_int;
    parity("tank.hp", |s| s.tank.hp += 1);
    parity("tank.max_hp", |s| s.tank.max_hp += 1);
    parity("tank.pos.x", move |s| s.tank.pos.x = f(1));
    parity("tank.pos.y", move |s| s.tank.pos.y = f(1));
    parity("tank.clear_cooldown_end", |s| {
        s.tank.clear_cooldown_end = 30
    });
    parity("tank.armor", |s| s.tank.armor += 1);
    parity("tank.dodge_num", |s| s.tank.dodge_num += 1);
    parity("tank.dodge_den", |s| s.tank.dodge_den += 1);
    parity("tank.mana_shield", |s| s.tank.mana_shield += 1);
    parity("tank.mana_shield_max", |s| s.tank.mana_shield_max += 1);
    parity("tank.mana_regen_per_tick", |s| {
        s.tank.mana_regen_per_tick += 1
    });
    parity("tank.hp_regen_per_tick", |s| s.tank.hp_regen_per_tick += 1);
    parity("tank.spikes_damage", |s| s.tank.spikes_damage += 1);
    parity("tank.spikes_mult", move |s| s.tank.spikes_mult = f(3));
    parity("tank.shield_active_dr", |s| {
        s.tank.shield_active_dr = Fixed::from_ratio(1, 4)
    });
    parity("tank.heal_on_damaged", |s| s.tank.heal_on_damaged += 1);
    parity("tank.heal_on_kill", |s| s.tank.heal_on_kill += 1);
    parity("tank.mana_on_kill", |s| s.tank.mana_on_kill += 1);
    parity("tank.heal_on_poison", |s| s.tank.heal_on_poison += 1);
    parity("tank.healing_mult", move |s| s.tank.healing_mult = f(2));
    parity("tank.missing_hp_heal_pct", |s| {
        s.tank.missing_hp_heal_pct = Fixed::from_ratio(1, 10)
    });
    parity("tank.revives", |s| s.tank.revives += 1);
    parity("tank.revive_bonus_hp", |s| s.tank.revive_bonus_hp += 1);
    parity("tank.shieldbreak_stun_range", |s| {
        s.tank.shieldbreak_stun_range = 1200
    });
    parity("tank.shieldbreak_stun_ticks", |s| {
        s.tank.shieldbreak_stun_ticks = 15
    });
    parity("tank.spikes_poison_dps", |s| s.tank.spikes_poison_dps = 2);
    parity("tank.spikes_poison_ticks", |s| {
        s.tank.spikes_poison_ticks = 90
    });
    parity("tank.spikes_stack_per", |s| s.tank.spikes_stack_per = 20);
    parity("tank.spikes_stacks", |s| s.tank.spikes_stacks = 7);
    parity("tank.spikes_stacks_max", |s| s.tank.spikes_stacks_max = 25);
    parity("tank.aura_range", |s| s.tank.aura_range = 600);
    parity("tank.aura_cadence", |s| s.tank.aura_cadence = 30);
    parity("tank.aura_damage", |s| s.tank.aura_damage = 200);
    parity("tank.aura_poison_dps", |s| s.tank.aura_poison_dps = 2);
    parity("tank.aura_poison_ticks", |s| s.tank.aura_poison_ticks = 90);
    parity("tank.aura_tick", |s| s.tank.aura_tick = 17);
    // EXPANSION E3 fidelity-mechanics tank state.
    parity("tank.regen_bonus_per_tick", |s| {
        s.tank.regen_bonus_per_tick = Fixed::from_ratio(1, 150)
    });
    parity("tank.regen_carry", |s| {
        s.tank.regen_carry = Fixed::from_ratio(1, 2)
    });
    parity("tank.deep_freeze", |s| s.tank.deep_freeze = true);
    parity("tank.retaliate_frost", |s| s.tank.retaliate_frost = 2);
    parity("tank.retaliate_fire", |s| s.tank.retaliate_fire = 20);
    parity("tank.spikes_first_hit", |s| s.tank.spikes_first_hit = 240);
    parity("tank.spikes_dr_rate", |s| {
        s.tank.spikes_dr_rate = Fixed::from_ratio(1, 20)
    });
    parity("tank.dmg_taken_to_spikes", |s| {
        s.tank.dmg_taken_to_spikes = Fixed::from_ratio(3, 10)
    });
}

#[test]
fn economy_fields_feed_checksum() {
    let f = Fixed::from_int;
    parity("economy.gold", |s| s.economy.gold += 1);
    parity("economy.income_per_tick", |s| {
        s.economy.income_per_tick += 1
    });
    parity("economy.income_mult", move |s| s.economy.income_mult = f(2));
    parity("economy.income_regen_pct", |s| {
        s.economy.income_regen_pct = Fixed::from_ratio(1, 5)
    });
    parity("economy.bounty_mult", move |s| s.economy.bounty_mult = f(2));
    parity("economy.bounty_proc_chance_pct", |s| {
        s.economy.bounty_proc_chance_pct = 10
    });
    parity("economy.bounty_proc_bonus", move |s| {
        s.economy.bounty_proc_bonus = f(2)
    });
    parity("economy.gold_per_damage", |s| {
        s.economy.gold_per_damage = Fixed::from_ratio(1, 100)
    });
    parity("economy.income_shield_pct", |s| {
        s.economy.income_shield_pct = Fixed::from_ratio(1, 5)
    });
    parity("economy.rerolls_remaining", |s| {
        s.economy.rerolls_remaining += 1
    });
    parity("economy.reroll_cost", |s| s.economy.reroll_cost += 1);
    parity("economy.treasure_pool", |s| s.economy.treasure_pool += 250);
}

#[test]
fn modifier_fields_feed_checksum() {
    let f = Fixed::from_int;
    let pct = |n, d| Fixed::from_ratio(n, d);
    parity("modifiers.add_global", move |s| {
        s.modifiers.add_global = pct(1, 10)
    });
    for i in 0..5 {
        parity(&format!("modifiers.add_by_type[{i}]"), move |s| {
            s.modifiers.add_by_type[i] = pct(1, 10)
        });
    }
    for i in 0..12 {
        parity(&format!("modifiers.add_by_scope[{i}]"), move |s| {
            s.modifiers.add_by_scope[i] = pct(1, 10)
        });
    }
    parity("modifiers.mul_global", move |s| {
        s.modifiers.mul_global = f(2)
    });
    parity("modifiers.attack_speed", move |s| {
        s.modifiers.attack_speed = pct(1, 10)
    });
    parity("modifiers.vs_stunned", move |s| {
        s.modifiers.vs_stunned = pct(1, 10)
    });
    parity("modifiers.vs_poisoned", move |s| {
        s.modifiers.vs_poisoned = pct(1, 10)
    });
    parity("modifiers.poison_dmg_mult", move |s| {
        s.modifiers.poison_dmg_mult = f(2)
    });
    parity("modifiers.stun_dur_mult", move |s| {
        s.modifiers.stun_dur_mult = f(2)
    });
    parity("modifiers.weapon_count_scaling[0].weapon_def", |s| {
        s.modifiers.weapon_count_scaling[0].weapon_def = 5
    });
    parity("modifiers.weapon_count_scaling[0].dmg_type", |s| {
        s.modifiers.weapon_count_scaling[0].dmg_type = 3
    });
    parity("modifiers.weapon_count_scaling[0].per", move |s| {
        s.modifiers.weapon_count_scaling[0].per = pct(1, 10)
    });
    parity("modifiers.dmg_per_maxhp_rate", move |s| {
        s.modifiers.dmg_per_maxhp_rate = pct(1, 10)
    });
    parity("modifiers.dmg_per_bounty_rate", move |s| {
        s.modifiers.dmg_per_bounty_rate = pct(1, 10)
    });
    parity("modifiers.shield_active_dmg", move |s| {
        s.modifiers.shield_active_dmg = pct(1, 10)
    });
    // EXPANSION E3 fidelity-mechanics aggregates.
    parity("modifiers.frost_strength_mult", move |s| {
        s.modifiers.frost_strength_mult = f(2)
    });
    parity("modifiers.fire_dmg_mult", move |s| {
        s.modifiers.fire_dmg_mult = f(2)
    });
    parity("modifiers.fire_explosion_mult", move |s| {
        s.modifiers.fire_explosion_mult = f(2)
    });
    parity("modifiers.bounce_barrage_pct", move |s| {
        s.modifiers.bounce_barrage_pct = pct(1, 4)
    });
    parity("modifiers.healthy_dmg", move |s| {
        s.modifiers.healthy_dmg = pct(7, 20)
    });
}

#[test]
fn entity_fields_feed_checksum() {
    let f = Fixed::from_int;
    // Weapons.
    parity("weapons[0].instance_id", |s| {
        s.weapons[0].instance_id = EntityId(200)
    });
    parity("weapons[0].def", |s| s.weapons[0].def = 1);
    parity("weapons[0].next_fire_tick", |s| {
        s.weapons[0].next_fire_tick += 1
    });
    parity("weapons.len", |s| {
        s.weapons.push(WeaponInstance {
            instance_id: EntityId(201),
            def: 0,
            next_fire_tick: 0,
        })
    });
    // Enemies (+ status).
    parity("enemies[0].id", |s| s.enemies[0].id = EntityId(210));
    parity("enemies[0].def", |s| s.enemies[0].def = 1);
    parity("enemies[0].hp", |s| s.enemies[0].hp += 1);
    parity("enemies[0].pos.x", move |s| s.enemies[0].pos.x = f(11));
    parity("enemies[0].pos.y", move |s| s.enemies[0].pos.y = f(11));
    parity("enemies[0].status.poison_dps", |s| {
        s.enemies[0].status.poison_dps = 2
    });
    parity("enemies[0].status.poison_ticks", |s| {
        s.enemies[0].status.poison_ticks = 30
    });
    parity("enemies[0].status.frost_stacks", |s| {
        s.enemies[0].status.frost_stacks = 1
    });
    parity("enemies[0].status.frost_ticks", |s| {
        s.enemies[0].status.frost_ticks = 30
    });
    parity("enemies[0].status.fire_stacks", |s| {
        s.enemies[0].status.fire_stacks = 1
    });
    parity("enemies[0].status.vuln_stacks", |s| {
        s.enemies[0].status.vuln_stacks = 1
    });
    parity("enemies[0].status.stun_ticks", |s| {
        s.enemies[0].status.stun_ticks = 10
    });
    parity("enemies[0].status.freeze_ticks", |s| {
        s.enemies[0].status.freeze_ticks = 10
    });
    parity("enemies[0].status.obscure_pct", |s| {
        s.enemies[0].status.obscure_pct = 25
    });
    parity("enemies[0].status.obscure_ticks", |s| {
        s.enemies[0].status.obscure_ticks = 90
    });
    for i in 0..5 {
        parity(&format!("enemies[0].status.vuln_by_type[{i}]"), move |s| {
            s.enemies[0].status.vuln_by_type[i] = 5
        });
    }
    parity("enemies[0].status.hit_tank", |s| {
        s.enemies[0].status.hit_tank = true
    });
    // Projectiles (incl. the original B4 field).
    parity("projectiles[0].id", |s| s.projectiles[0].id = EntityId(220));
    parity("projectiles[0].weapon_kind", |s| {
        s.projectiles[0].weapon_kind = 7
    });
    parity("projectiles[0].pos.x", move |s| {
        s.projectiles[0].pos.x = f(5)
    });
    parity("projectiles[0].pos.y", move |s| {
        s.projectiles[0].pos.y = f(5)
    });
    parity("projectiles[0].target", |s| {
        s.projectiles[0].target = EntityId(0)
    });
    parity("projectiles[0].last_target_pos.x", move |s| {
        s.projectiles[0].last_target_pos.x = f(1)
    });
    parity("projectiles[0].last_target_pos.y", move |s| {
        s.projectiles[0].last_target_pos.y = f(1)
    });
    parity("projectiles[0].damage", |s| s.projectiles[0].damage += 1);
    parity("projectiles[0].damage_type", |s| {
        s.projectiles[0].damage_type = 2
    });
    parity("projectiles[0].splash_radius", move |s| {
        s.projectiles[0].splash_radius = f(4)
    });
    parity("projectiles[0].speed", move |s| {
        s.projectiles[0].speed = f(4)
    });
    parity("projectiles[0].on_hit.poison_dps", |s| {
        s.projectiles[0].on_hit.poison_dps = 2
    });
    parity("projectiles[0].on_hit.poison_ticks", |s| {
        s.projectiles[0].on_hit.poison_ticks = 30
    });
    parity("projectiles[0].on_hit.frost_stacks", |s| {
        s.projectiles[0].on_hit.frost_stacks = 1
    });
    parity("projectiles[0].on_hit.fire_stacks", |s| {
        s.projectiles[0].on_hit.fire_stacks = 1
    });
    parity("projectiles[0].on_hit.stun_ticks", |s| {
        s.projectiles[0].on_hit.stun_ticks = 10
    });
    parity("projectiles[0].ability", |s| {
        s.projectiles[0].ability = WeaponAbility::LifeDrain { per_hit: 5 }
    });
    // Hazards.
    parity("hazards[0].id", |s| s.hazards[0].id = EntityId(230));
    parity("hazards[0].pos.x", move |s| s.hazards[0].pos.x = f(7));
    parity("hazards[0].pos.y", move |s| s.hazards[0].pos.y = f(7));
    parity("hazards[0].dmg", |s| s.hazards[0].dmg += 1);
    parity("hazards[0].damage_type", |s| s.hazards[0].damage_type = 3);
    parity("hazards[0].radius", |s| s.hazards[0].radius += 1);
    parity("hazards[0].ticks_left", |s| s.hazards[0].ticks_left += 1);
    // Minions.
    parity("minions[0].id", |s| s.minions[0].id = EntityId(240));
    parity("minions[0].pos.x", move |s| s.minions[0].pos.x = f(8));
    parity("minions[0].pos.y", move |s| s.minions[0].pos.y = f(8));
    parity("minions[0].kind", |s| s.minions[0].kind = 1);
    parity("minions[0].hp", |s| s.minions[0].hp += 1);
    parity("minions[0].damage", |s| s.minions[0].damage += 1);
    parity("minions[0].damage_type", |s| s.minions[0].damage_type = 2);
    parity("minions[0].next_attack_tick", |s| {
        s.minions[0].next_attack_tick += 1
    });
    parity("minions[0].expire_tick", |s| s.minions[0].expire_tick += 1);
    // Rotating-wave sweeps.
    parity("sweeps[0].id", |s| s.sweeps[0].id = EntityId(250));
    parity("sweeps[0].weapon_kind", |s| s.sweeps[0].weapon_kind = 1);
    parity("sweeps[0].damage", |s| s.sweeps[0].damage += 1);
    parity("sweeps[0].damage_type", |s| s.sweeps[0].damage_type = 4);
    parity("sweeps[0].radius", move |s| s.sweeps[0].radius = f(300));
    parity("sweeps[0].angle_bam", |s| s.sweeps[0].angle_bam = 2048);
    parity("sweeps[0].step_bam", |s| s.sweeps[0].step_bam = 4096);
    parity("sweeps[0].ticks_left", |s| s.sweeps[0].ticks_left += 1);
    parity("sweeps[0].clockwise", |s| s.sweeps[0].clockwise = true);
    parity("sweeps[0].on_hit.frost_stacks", |s| {
        s.sweeps[0].on_hit.frost_stacks = 3
    });
    parity("sweeps[0].ability", |s| {
        s.sweeps[0].ability = WeaponAbility::HealOnAttack { amount: 80 }
    });
    parity("sweeps.len", |s| {
        let sw = s.sweeps[0];
        s.sweeps.push(WaveSweep {
            id: EntityId(251),
            ..sw
        })
    });
}

#[test]
fn arena_collections_and_shop_feed_checksum() {
    parity("ramps[0].effect", |s| {
        s.ramps[0].effect = ModEffect::IncomeFlat(4)
    });
    parity("ramps[0].interval_ticks", |s| {
        s.ramps[0].interval_ticks += 1
    });
    parity("ramps[0].next_apply", |s| s.ramps[0].next_apply += 1);
    parity("vuln_pulses[0].magnitude", |s| {
        s.vuln_pulses[0].magnitude += 1
    });
    parity("vuln_pulses[0].range", |s| s.vuln_pulses[0].range += 1);
    parity("vuln_pulses[0].interval_ticks", |s| {
        s.vuln_pulses[0].interval_ticks += 1
    });
    parity("vuln_pulses[0].next_tick", |s| {
        s.vuln_pulses[0].next_tick += 1
    });
    parity("pending_perk presence", |s| {
        s.pending_perk = Some(PendingPerk {
            rarity: 255,
            scope: PerkScope::Any,
            extra_copies: 0,
            free: false,
        })
    });
    let with_perk = |s: &mut ArenaState| {
        s.pending_perk = Some(PendingPerk {
            rarity: 0,
            scope: PerkScope::Any,
            extra_copies: 0,
            free: false,
        })
    };
    parity2("pending_perk.rarity", with_perk, |s| {
        s.pending_perk = Some(PendingPerk {
            rarity: 1,
            scope: PerkScope::Any,
            extra_copies: 0,
            free: false,
        })
    });
    parity2("pending_perk.scope", with_perk, |s| {
        s.pending_perk = Some(PendingPerk {
            rarity: 0,
            scope: PerkScope::WeaponOrSpikes,
            extra_copies: 0,
            free: false,
        })
    });
    parity2("pending_perk.extra_copies", with_perk, |s| {
        s.pending_perk = Some(PendingPerk {
            rarity: 0,
            scope: PerkScope::Any,
            extra_copies: 1,
            free: false,
        })
    });
    parity2("pending_perk.free", with_perk, |s| {
        s.pending_perk = Some(PendingPerk {
            rarity: 0,
            scope: PerkScope::Any,
            extra_copies: 0,
            free: true,
        })
    });
    parity("shop.shop_seq", |s| s.shop.shop_seq += 1);
    parity("shop.offers[0].kind", |s| {
        s.shop.offers[0].kind = OfferKind::Modifier
    });
    parity("shop.offers[0].def", |s| s.shop.offers[0].def = 1);
    parity("shop.offers[0].cost", |s| s.shop.offers[0].cost += 1);
    parity("shop.offers.len", |s| {
        s.shop.offers.push(Offer {
            kind: OfferKind::Weapon,
            def: 2,
            cost: 50,
        })
    });
}
