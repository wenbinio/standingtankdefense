//! Sim→render event stream (`docs/09 §9.3`) — emission and determinism.
//!
//! The contract under test:
//! - events are a PURE DERIVATION of deterministic state: draining them (or
//!   not) can never change the checksum trajectory or the wire snapshot;
//! - every kind is emitted at its edge (kill vs despawn distinguished, impacts
//!   per detonation, round/boss/hazard/freeze/shield/bounty edges);
//! - the buffer is cleared at the top of every `step()` (undrained events are
//!   dropped, never accumulated).

use determinism::Fixed;
use sim::content;
use sim::*;

/// A quiet, mid-round arena: no wave cadence fires at tick 7, no starting
/// weapon, round already started (no `RoundStart` noise).
fn quiet_arena() -> ArenaState {
    let mut s = ArenaState::new(0xEE_EE, 0);
    s.weapons.clear();
    s.tick = 7; // 7 % 18 != 0, 7 % 95 != 0, below every gated wave entry
    s.round = 0;
    s
}

fn enemy_at(s: &mut ArenaState, def: u16, hp: i64, x: i64) -> EntityId {
    let id = s.alloc_entity_id();
    s.enemies.push(Enemy::new(
        id,
        def,
        hp,
        Vec2::new(Fixed::from_int(x), Fixed::ZERO),
    ));
    id
}

/// Push a projectile already sitting on `target` so it detonates next step.
fn proj_on_target(
    s: &mut ArenaState,
    target: EntityId,
    target_x: i64,
    damage: i64,
    splash: i64,
    on_hit: content::StatusOnHit,
    ability: content::WeaponAbility,
) {
    let id = s.alloc_entity_id();
    let pos = Vec2::new(Fixed::from_int(target_x), Fixed::ZERO);
    s.projectiles.push(Projectile {
        id,
        weapon_kind: 0,
        pos,
        target,
        last_target_pos: pos,
        damage,
        damage_type: content::DMG_SIEGE, // 1x vs armor 0 — keeps numbers exact
        splash_radius: Fixed::from_int(splash),
        speed: Fixed::from_int(10_000),
        on_hit,
        ability,
    });
}

// ---------------- determinism: drained vs never-drained ----------------

#[test]
fn drained_and_undrained_runs_have_identical_checksums_and_snapshots() {
    let scripted = |tick: u32| -> Input {
        match tick {
            5 => Input::BuyOffer { slot: 0 },
            50 => Input::BuyOffer { slot: 1 },
            200 => Input::Reroll,
            205 => Input::BuyOffer { slot: 0 },
            400 => Input::Clear,
            950 => Input::BuyOffer { slot: 0 },
            _ => Input::Noop,
        }
    };
    let mut drained = ArenaState::new(0xA5A5_1234_DEAD_BEEF, 0);
    let mut undrained = drained.clone();
    let mut total_events = 0usize;
    for tick in 0..2000u32 {
        step(&mut drained, scripted(tick));
        step(&mut undrained, scripted(tick));
        total_events += drained.events.take().len(); // drain every tick
        assert_eq!(
            checksum(&drained),
            checksum(&undrained),
            "checksum diverged at tick {tick}: draining events perturbed the sim"
        );
    }
    assert!(total_events > 0, "the run must actually produce events");
    // The wire snapshot is identical too — events never cross the wire.
    assert_eq!(
        snapshot::serialize(&drained),
        snapshot::serialize(&undrained)
    );
}

#[test]
fn events_are_excluded_from_checksum_snapshot_and_equality() {
    let a = ArenaState::new(1, 0);
    let mut b = a.clone();
    b.events.0.push(SimEvent::ShieldBroke);
    assert_eq!(checksum(&a), checksum(&b), "events must not feed checksum");
    assert_eq!(
        snapshot::serialize(&a),
        snapshot::serialize(&b),
        "events must not ride the snapshot"
    );
    assert_eq!(a, b, "events are not part of state identity");
}

// ---------------- per-kind emission ----------------

#[test]
fn round_start_is_emitted_at_round_boundaries() {
    let mut s = ArenaState::new(2, 0);
    // Keep the tank alive across the whole no-input round (contact damage
    // would otherwise kill it and freeze the arena before the boundary).
    s.tank.max_hp = i64::MAX / 4;
    s.tank.hp = s.tank.max_hp;
    step(&mut s, Input::Noop);
    assert!(
        s.events
            .as_slice()
            .contains(&SimEvent::RoundStart { round: 0 }),
        "first tick opens round 0"
    );
    // A mid-round tick emits no RoundStart.
    step(&mut s, Input::Noop);
    assert!(
        !s.events
            .as_slice()
            .iter()
            .any(|e| matches!(e, SimEvent::RoundStart { .. })),
        "mid-round ticks emit no RoundStart"
    );
    // Advance to the round-1 boundary: the step that PROCESSES tick 900 opens it.
    while s.tick < ROUND_TICKS {
        step(&mut s, Input::Noop);
    }
    step(&mut s, Input::Noop);
    assert!(
        s.events
            .as_slice()
            .contains(&SimEvent::RoundStart { round: 1 }),
        "round boundary emits RoundStart"
    );
}

#[test]
fn projectile_kill_emits_killed_impact_and_bounty_but_no_despawn() {
    let mut s = quiet_arena();
    let eid = enemy_at(&mut s, 0, 100, 1000); // far from the tank
    proj_on_target(
        &mut s,
        eid,
        1000,
        200,
        0,
        content::StatusOnHit::NONE,
        content::WeaponAbility::None,
    );
    step(&mut s, Input::Noop);
    let evs = s.events.as_slice();
    let bounty = content::ENEMIES[0].bounty;
    assert!(
        evs.iter().any(|e| matches!(
            e,
            SimEvent::EnemyKilled { kind: 0, boss: false, fire_explosion_radius: 0, bounty: b, .. }
            if *b == bounty
        )),
        "kill must emit EnemyKilled with the catalog bounty: {evs:?}"
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, SimEvent::EnemyDespawned { .. })),
        "a kill is not a despawn"
    );
    assert!(
        evs.iter().any(|e| matches!(
            e,
            SimEvent::Impact {
                damage: 200,
                splash_radius: 0,
                ..
            }
        )),
        "detonation must emit Impact: {evs:?}"
    );
    assert!(
        evs.iter().any(|e| matches!(
            e,
            SimEvent::GoldBounty { amount } if *amount == bounty
        )),
        "paid bounty must emit GoldBounty: {evs:?}"
    );
}

#[test]
fn contact_selfdestruct_emits_despawn_and_tankhit_but_no_kill() {
    let mut s = quiet_arena();
    let eid = enemy_at(&mut s, 0, 10_000, 3); // inside one move step of the tank
    step(&mut s, Input::Noop);
    let evs = s.events.as_slice();
    assert!(
        evs.contains(&SimEvent::EnemyDespawned { id: eid.0 }),
        "contact self-destruct must emit EnemyDespawned: {evs:?}"
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, SimEvent::EnemyKilled { .. })),
        "a despawn is not a kill"
    );
    let contact = content::ENEMIES[0].contact_damage;
    assert!(
        evs.contains(&SimEvent::TankHit { damage: contact }),
        "landed contact hit must emit TankHit: {evs:?}"
    );
    assert!(
        !evs.iter().any(|e| matches!(e, SimEvent::GoldBounty { .. })),
        "no bounty for a self-destruct"
    );
}

#[test]
fn dodged_hit_emits_no_tankhit() {
    let mut s = quiet_arena();
    s.tank.dodge_num = 100;
    s.tank.dodge_den = 100; // guaranteed dodge
    enemy_at(&mut s, 0, 10_000, 3);
    step(&mut s, Input::Noop);
    assert!(
        !s.events
            .as_slice()
            .iter()
            .any(|e| matches!(e, SimEvent::TankHit { .. })),
        "a dodged hit never lands, so it must not emit TankHit"
    );
}

#[test]
fn splash_impact_reports_its_radius() {
    let mut s = quiet_arena();
    let eid = enemy_at(&mut s, 0, 100, 1000);
    proj_on_target(
        &mut s,
        eid,
        1000,
        300,
        300,
        content::StatusOnHit::NONE,
        content::WeaponAbility::None,
    );
    step(&mut s, Input::Noop);
    assert!(
        s.events.as_slice().iter().any(|e| matches!(
            e,
            SimEvent::Impact {
                x: 1000,
                y: 0,
                damage: 300,
                splash_radius: 300,
                ..
            }
        )),
        "splash detonation must carry its radius: {:?}",
        s.events.as_slice()
    );
}

#[test]
fn weapon_fire_emits_projectile_spawned_with_the_weapon_kind() {
    let mut s = quiet_arena();
    let wid = s.alloc_entity_id();
    s.weapons.push(WeaponInstance {
        instance_id: wid,
        def: content::STARTING_WEAPON,
        next_fire_tick: 0,
    });
    enemy_at(&mut s, 0, 1_000_000, 100);
    step(&mut s, Input::Noop);
    assert!(
        s.events.as_slice().iter().any(|e| matches!(
            e,
            SimEvent::ProjectileSpawned {
                weapon_kind: content::STARTING_WEAPON,
                x: 0,
                y: 0,
                ..
            }
        )),
        "firing must emit ProjectileSpawned: {:?}",
        s.events.as_slice()
    );
    // The live projectile carries the same deterministic render kind.
    assert!(s
        .projectiles
        .iter()
        .all(|p| p.weapon_kind == content::STARTING_WEAPON));
}

#[test]
fn boss_spawn_emits_boss_spawned_with_its_id() {
    let mut s = ArenaState::new(3, 0);
    s.tick = content::BOSS_SPAWN_TICK;
    s.round = content::BOSS_SPAWN_TICK / ROUND_TICKS;
    step(&mut s, Input::Noop);
    let boss_id = s
        .enemies
        .iter()
        .find(|e| e.def == content::BOSS)
        .expect("boss on the board")
        .id;
    assert!(
        s.events
            .as_slice()
            .contains(&SimEvent::BossSpawned { id: boss_id.0 }),
        "boss tick must emit BossSpawned: {:?}",
        s.events.as_slice()
    );
}

#[test]
fn shield_break_emits_shield_broke_even_without_the_stun_perk() {
    let mut s = quiet_arena();
    s.tank.mana_shield = 5;
    s.tank.mana_shield_max = 5;
    assert_eq!(s.tank.shieldbreak_stun_range, 0, "no Energy Pulse owned");
    enemy_at(&mut s, 0, 10_000, 3); // contact hit drains the tiny shield
    step(&mut s, Input::Noop);
    assert!(
        s.events.as_slice().contains(&SimEvent::ShieldBroke),
        "the >0→0 shield edge must emit ShieldBroke: {:?}",
        s.events.as_slice()
    );
}

#[test]
fn deep_freeze_payoff_emits_freeze_proc() {
    let mut s = quiet_arena();
    let eid = enemy_at(&mut s, 0, 1_000_000, 1000);
    s.enemies[0].status.frost_stacks = content::FROST_MAX_STACKS - 1;
    s.enemies[0].status.frost_ticks = 150;
    proj_on_target(
        &mut s,
        eid,
        1000,
        1,
        0,
        content::StatusOnHit {
            frost_stacks: 1, // reaches the cap → freeze
            ..content::StatusOnHit::NONE
        },
        content::WeaponAbility::None,
    );
    step(&mut s, Input::Noop);
    assert!(
        s.events
            .as_slice()
            .contains(&SimEvent::FreezeProc { id: eid.0 }),
        "reaching FROST_MAX_STACKS must emit FreezeProc: {:?}",
        s.events.as_slice()
    );
    assert!(s.enemies[0].status.freeze_ticks > 0, "enemy actually froze");
}

#[test]
fn hazard_placement_and_expiry_emit_events() {
    // Placement: a projectile whose ability drops a hazard on the enemy it hit.
    let mut s = quiet_arena();
    let eid = enemy_at(&mut s, 0, 1_000_000, 1000);
    proj_on_target(
        &mut s,
        eid,
        1000,
        1,
        0,
        content::StatusOnHit::NONE,
        content::WeaponAbility::Hazard {
            dmg: 10,
            radius: 300,
            ticks: 60,
        },
    );
    step(&mut s, Input::Noop);
    assert!(
        s.events.as_slice().iter().any(|e| matches!(
            e,
            SimEvent::HazardPlaced {
                x: 1000,
                y: 0,
                radius: 300,
                ticks: 60,
                ..
            }
        )),
        "hazard drop must emit HazardPlaced: {:?}",
        s.events.as_slice()
    );
    assert_eq!(s.hazards.len(), 1);

    // Expiry: a hazard with one tick left dies this step.
    let mut s = quiet_arena();
    let hid = s.alloc_entity_id();
    s.hazards.push(Hazard {
        id: hid,
        pos: Vec2::ZERO,
        dmg: 1,
        damage_type: content::DMG_SIEGE,
        radius: 100,
        ticks_left: 1,
    });
    step(&mut s, Input::Noop);
    assert!(s.hazards.is_empty(), "hazard expired");
    assert!(
        s.events
            .as_slice()
            .contains(&SimEvent::HazardExpired { id: hid.0 }),
        "expiry must emit HazardExpired: {:?}",
        s.events.as_slice()
    );
}

#[test]
fn clear_kills_emit_enemy_killed() {
    let mut s = quiet_arena();
    enemy_at(&mut s, 0, 5_000, 800);
    enemy_at(&mut s, 1, 5_000, 900);
    step(&mut s, Input::Clear);
    let kills: Vec<u16> = s
        .events
        .as_slice()
        .iter()
        .filter_map(|e| match e {
            SimEvent::EnemyKilled { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect();
    assert_eq!(kills, vec![0, 1], "Clear announces every kill in id order");
    assert!(
        s.events
            .as_slice()
            .iter()
            .any(|e| matches!(e, SimEvent::GoldBounty { .. })),
        "Clear kills pay (and announce) bounty"
    );
}

#[test]
fn spikes_kill_emits_enemy_killed() {
    let mut s = quiet_arena();
    s.tank.spikes_damage = 100;
    let contact = enemy_at(&mut s, 0, 10_000, 3); // triggers the hit
    let victim = enemy_at(&mut s, 0, 50, 100); // dies to the retaliation
    step(&mut s, Input::Noop);
    let evs = s.events.as_slice();
    assert!(
        evs.contains(&SimEvent::EnemyDespawned { id: contact.0 }),
        "the rammer despawns: {evs:?}"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, SimEvent::EnemyKilled { kind: 0, .. })),
        "the spikes victim is a real kill: {evs:?}"
    );
    assert!(
        s.enemies.iter().all(|e| e.id != victim),
        "the victim left the board"
    );
}

#[test]
fn fire_death_explosion_radius_rides_the_kill_event() {
    let mut s = quiet_arena();
    let eid = enemy_at(&mut s, 0, 10, 1000);
    s.enemies[0].status.fire_stacks = 50; // explodes for floor(50/5) = 10
    proj_on_target(
        &mut s,
        eid,
        1000,
        100,
        0,
        content::StatusOnHit::NONE,
        content::WeaponAbility::None,
    );
    step(&mut s, Input::Noop);
    assert!(
        s.events.as_slice().iter().any(|e| matches!(
            e,
            SimEvent::EnemyKilled {
                fire_explosion_radius: 300,
                ..
            }
        )),
        "a Fire-stacked death carries its explosion radius: {:?}",
        s.events.as_slice()
    );
}

#[test]
fn undrained_events_are_dropped_on_the_next_step() {
    let mut s = quiet_arena();
    let eid = enemy_at(&mut s, 0, 100, 1000);
    proj_on_target(
        &mut s,
        eid,
        1000,
        200,
        0,
        content::StatusOnHit::NONE,
        content::WeaponAbility::None,
    );
    step(&mut s, Input::Noop);
    assert!(
        !s.events.as_slice().is_empty(),
        "the kill tick produced events"
    );
    step(&mut s, Input::Noop); // never drained — the next step clears them
    assert!(
        !s.events
            .as_slice()
            .iter()
            .any(|e| matches!(e, SimEvent::EnemyKilled { .. })),
        "undrained events must be dropped, not accumulated"
    );
}

#[test]
fn dead_arena_emits_nothing() {
    let mut s = quiet_arena();
    s.dead = true;
    s.events.0.push(SimEvent::ShieldBroke); // stale, pretend-undrained
    step(&mut s, Input::Noop);
    assert!(
        s.events.as_slice().is_empty(),
        "a frozen (dead) arena never re-serves events"
    );
}
