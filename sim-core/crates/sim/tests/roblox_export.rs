//! Gates for the Roblox fork's generated artifacts (`roblox/CONTRACTS.md` C1/C4).
//!
//! Two things are proven here:
//! 1. the `(tag, a, b, c)` payload encoding is **lossless** for every enum value
//!    the catalog actually contains, and
//! 2. the checked-in `content.json` is **the Rust tables**, index for index
//!    (C1's "array index is identity" rule) — not a stale copy of them.

use sim::content::{
    self as c, Archetype, Attack, EnemyAbility, ModEffect, WeaponAbility, ENEMY_RANGED_DTYPE_SHIFT,
};
use sim::export::json::Value;
use sim::export::{all_artifacts, repo_root, OUTPUTS};

// ============================ round-trip ============================

fn rt_attack(a: Attack) {
    let (t, x, y, z) = a.words();
    assert_eq!(Attack::from_words(t, x, y, z), Some(a), "Attack round-trip: {a:?}");
}
fn rt_weapon_ability(a: WeaponAbility) {
    let (t, x, y, z) = a.words();
    assert_eq!(WeaponAbility::from_words(t, x, y, z), Some(a), "WeaponAbility round-trip: {a:?}");
}
fn rt_enemy_ability(a: EnemyAbility) {
    let (t, x, y, z) = a.words();
    assert_eq!(EnemyAbility::from_words(t, x, y, z), Some(a), "EnemyAbility round-trip: {a:?}");
}
fn rt_mod_effect(e: ModEffect) {
    let (t, x, y, z) = e.words();
    assert_eq!(ModEffect::from_words(t, x, y, z), Some(e), "ModEffect round-trip: {e:?}");
}

/// C1: "keep it lossless — `from_words(e.words()) == Some(e)` for every variant,
/// proven by a round-trip test over every catalog entry."
#[test]
fn every_catalog_payload_round_trips() {
    let mut attack_tags = std::collections::BTreeSet::new();
    let mut weapon_ability_tags = std::collections::BTreeSet::new();
    let mut enemy_ability_tags = std::collections::BTreeSet::new();
    let mut mod_effect_tags = std::collections::BTreeSet::new();

    for w in c::WEAPONS {
        rt_attack(w.attack);
        rt_weapon_ability(w.ability);
        attack_tags.insert(w.attack.words().0);
        weapon_ability_tags.insert(w.ability.words().0);
        // The `Attack` payload tag doubles as the damage-scope id (see
        // `constants.range_scope_*` / `rarity_scope_base` in content.json).
        assert_eq!(w.attack.words().0, c::attack_scope_id(w.attack));
    }
    for e in c::ENEMIES {
        rt_enemy_ability(e.ability);
        enemy_ability_tags.insert(e.ability.words().0);
        assert_eq!(Archetype::from_id(e.archetype.id()), Some(e.archetype));
    }
    for m in c::MODIFIERS {
        for e in m.effects {
            rt_mod_effect(*e);
            mod_effect_tags.insert(e.words().0);
        }
        if let Some(r) = m.ramp {
            rt_mod_effect(r.effect);
            mod_effect_tags.insert(r.effect.words().0);
        }
    }

    // The catalog happens to exercise EVERY variant of all four payload enums,
    // so the round-trip above is exhaustive, not merely representative. Pinning
    // that here means a newly added variant fails this test until it is both
    // used by the catalog and given a `from_words` arm.
    let seen = |set: &std::collections::BTreeSet<u8>| set.iter().copied().collect::<Vec<u8>>();
    assert_eq!(seen(&attack_tags), (0..=5).collect::<Vec<u8>>(), "Attack variants");
    assert_eq!(seen(&weapon_ability_tags), (0..=7).collect::<Vec<u8>>(), "WeaponAbility variants");
    assert_eq!(seen(&enemy_ability_tags), (0..=1).collect::<Vec<u8>>(), "EnemyAbility variants");
    assert_eq!(seen(&mod_effect_tags), (0..=47).collect::<Vec<u8>>(), "ModEffect variants");
}

/// `EnemyAbility::RangedAttack` packs two fields into operand `b`; prove the
/// packing survives the extremes of both, not just the catalog's small values.
#[test]
fn enemy_ranged_packing_is_lossless_at_the_edges() {
    for cooldown in [0u32, 1, 30, u32::MAX, u32::MAX - 1] {
        for damage_type in [0u8, 4, 255] {
            for range in [0i64, 900, i64::MAX, i64::MIN] {
                rt_enemy_ability(EnemyAbility::RangedAttack {
                    range,
                    cooldown_ticks: cooldown,
                    damage: -1,
                    damage_type,
                });
            }
        }
    }
    assert_eq!(ENEMY_RANGED_DTYPE_SHIFT, 32);
}

#[test]
fn unknown_tags_decode_to_none() {
    assert_eq!(Attack::from_words(6, 0, 0, 0), None);
    assert_eq!(WeaponAbility::from_words(8, 0, 0, 0), None);
    assert_eq!(EnemyAbility::from_words(2, 0, 0, 0), None);
    assert_eq!(ModEffect::from_words(48, 0, 0, 0), None);
    assert_eq!(Archetype::from_id(7), None);
}

// ============================ artifacts ============================

fn read_artifact(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}\nrun: cargo run -p sim --bin export-content", path.display()))
}

/// The checked-in files must be exactly what the generator produces right now.
/// This is what makes "the Rust tables are the single source of truth" true
/// rather than aspirational.
#[test]
fn checked_in_artifacts_are_current() {
    for (rel, expect) in OUTPUTS.iter().zip(all_artifacts()) {
        assert_eq!(
            read_artifact(rel),
            expect,
            "{rel} is stale — re-run: cargo run -p sim --bin export-content"
        );
    }
}

fn content() -> Value {
    Value::parse(&read_artifact(OUTPUTS[0])).expect("content.json parses")
}

fn payload(v: &Value) -> (u8, i64, i64, i64) {
    (v.at("tag").as_i64() as u8, v.at("a").as_i64(), v.at("b").as_i64(), v.at("c").as_i64())
}

/// C1: "**Array index is identity.** `weapons[i]` must correspond to weapon id
/// `i` exactly as the Rust indexes it."
#[test]
fn json_weapons_match_the_rust_table_by_index() {
    let doc = content();
    let arr = doc.at("weapons").as_arr();
    assert_eq!(arr.len(), c::WEAPONS.len(), "weapon count");
    for (i, (j, w)) in arr.iter().zip(c::WEAPONS).enumerate() {
        let ctx = || format!("weapons[{i}] ({})", w.name);
        assert_eq!(j.at("name").as_str(), w.name, "{}", ctx());
        assert_eq!(j.at("rarity").as_i64(), w.rarity as i64, "{}", ctx());
        assert_eq!(j.at("cost").as_i64(), w.cost, "{}", ctx());
        assert_eq!(j.at("damage").as_i64(), w.damage, "{}", ctx());
        assert_eq!(j.at("damage_type").as_i64(), w.damage_type as i64, "{}", ctx());
        assert_eq!(payload(j.at("attack")), w.attack.words(), "{}", ctx());
        assert_eq!(j.at("cooldown_ticks").as_i64(), w.cooldown_ticks as i64, "{}", ctx());
        assert_eq!(j.at("range").as_i64(), w.range, "{}", ctx());
        assert_eq!(j.at("proj_speed").as_i64(), w.proj_speed, "{}", ctx());
        let oh = j.at("on_hit");
        assert_eq!(oh.at("poison_dps").as_i64(), w.on_hit.poison_dps, "{}", ctx());
        assert_eq!(oh.at("poison_ticks").as_i64(), w.on_hit.poison_ticks as i64, "{}", ctx());
        assert_eq!(oh.at("frost_stacks").as_i64(), w.on_hit.frost_stacks as i64, "{}", ctx());
        assert_eq!(oh.at("fire_stacks").as_i64(), w.on_hit.fire_stacks as i64, "{}", ctx());
        assert_eq!(oh.at("stun_ticks").as_i64(), w.on_hit.stun_ticks as i64, "{}", ctx());
        assert_eq!(payload(j.at("ability")), w.ability.words(), "{}", ctx());
        // ... and the payload actually decodes back to the Rust value.
        let (t, a, b, cc) = payload(j.at("attack"));
        assert_eq!(Attack::from_words(t, a, b, cc), Some(w.attack), "{}", ctx());
        let (t, a, b, cc) = payload(j.at("ability"));
        assert_eq!(WeaponAbility::from_words(t, a, b, cc), Some(w.ability), "{}", ctx());
    }
}

#[test]
fn json_enemies_match_the_rust_table_by_index() {
    let doc = content();
    let arr = doc.at("enemies").as_arr();
    assert_eq!(arr.len(), c::ENEMIES.len(), "enemy count");
    for (i, (j, e)) in arr.iter().zip(c::ENEMIES).enumerate() {
        let ctx = || format!("enemies[{i}] ({})", e.name);
        assert_eq!(j.at("name").as_str(), e.name, "{}", ctx());
        assert_eq!(j.at("base_hp").as_i64(), e.base_hp, "{}", ctx());
        assert_eq!(j.at("move_speed").as_i64(), e.move_speed, "{}", ctx());
        assert_eq!(j.at("contact_damage").as_i64(), e.contact_damage, "{}", ctx());
        assert_eq!(j.at("bounty").as_i64(), e.bounty, "{}", ctx());
        assert_eq!(j.at("armor_class").as_i64(), e.armor_class as i64, "{}", ctx());
        assert_eq!(j.at("archetype").as_i64(), e.archetype.id() as i64, "{}", ctx());
        assert_eq!(j.at("boss").as_bool(), e.boss, "{}", ctx());
        let (t, a, b, cc) = payload(j.at("ability"));
        assert_eq!(EnemyAbility::from_words(t, a, b, cc), Some(e.ability), "{}", ctx());
    }
}

#[test]
fn json_modifiers_match_the_rust_table_by_index() {
    let doc = content();
    let arr = doc.at("modifiers").as_arr();
    assert_eq!(arr.len(), c::MODIFIERS.len(), "modifier count");
    for (i, (j, m)) in arr.iter().zip(c::MODIFIERS).enumerate() {
        let ctx = || format!("modifiers[{i}] ({})", m.name);
        assert_eq!(j.at("name").as_str(), m.name, "{}", ctx());
        assert_eq!(j.at("rarity").as_i64(), m.rarity as i64, "{}", ctx());
        assert_eq!(j.at("cost").as_i64(), m.cost, "{}", ctx());
        let effects = j.at("effects").as_arr();
        assert_eq!(effects.len(), m.effects.len(), "{} effect count", ctx());
        for (k, (je, e)) in effects.iter().zip(m.effects).enumerate() {
            let (t, a, b, cc) = payload(je);
            assert_eq!(ModEffect::from_words(t, a, b, cc), Some(*e), "{} effects[{k}]", ctx());
        }
        match m.ramp {
            None => assert!(j.at("ramp").is_null(), "{} ramp", ctx()),
            Some(r) => {
                let jr = j.at("ramp");
                let (t, a, b, cc) = payload(jr.at("effect"));
                assert_eq!(ModEffect::from_words(t, a, b, cc), Some(r.effect), "{} ramp", ctx());
                assert_eq!(
                    jr.at("interval_ticks").as_i64(),
                    r.interval_ticks as i64,
                    "{} ramp interval",
                    ctx()
                );
            }
        }
    }
}

#[test]
fn json_waves_and_matrix_match_the_rust_tables() {
    let doc = content();
    for (key, table) in [("waves", c::WAVE_M0), ("boss_escort", c::BOSS_ESCORT)] {
        let arr = doc.at(key).as_arr();
        assert_eq!(arr.len(), table.len(), "{key} count");
        for (i, (j, w)) in arr.iter().zip(table).enumerate() {
            assert_eq!(j.at("enemy").as_i64(), w.enemy as i64, "{key}[{i}].enemy");
            assert_eq!(j.at("cadence_ticks").as_i64(), w.cadence_ticks as i64, "{key}[{i}]");
            assert_eq!(j.at("start_tick").as_i64(), w.start_tick as i64, "{key}[{i}]");
            assert!((w.enemy as usize) < c::ENEMIES.len(), "{key}[{i}] enemy id in range");
        }
    }

    let m = doc.at("damage_matrix").as_arr();
    assert_eq!(m.len(), 5, "one row per damage type");
    for (dt, row) in m.iter().enumerate() {
        let row = row.as_arr();
        assert_eq!(row.len(), c::NUM_ARMOR_CLASSES);
        for (ac, cell) in row.iter().enumerate() {
            assert_eq!(
                cell.as_i64(),
                c::damage_multiplier(dt as u8, ac as u8).raw(),
                "damage_matrix[{dt}][{ac}] raw Fixed"
            );
        }
    }

    let ring = doc.at("spawn_ring").as_arr();
    assert_eq!(ring.len(), c::SPAWN_RING.len());
    for (i, (j, p)) in ring.iter().zip(c::SPAWN_RING).enumerate() {
        assert_eq!(j.at("x").as_i64(), p.x.raw(), "spawn_ring[{i}].x");
        assert_eq!(j.at("y").as_i64(), p.y.raw(), "spawn_ring[{i}].y");
    }
}

/// C1: "Named ids (`STARTING_WEAPON`, `DEATH_ENGINE`, `BOSS`) go in `constants`."
#[test]
fn named_ids_and_constants_agree_with_rust() {
    let doc = content();
    let k = doc.at("constants");
    let get = |name: &str| k.at(name).as_i64();

    assert_eq!(get("starting_weapon"), c::STARTING_WEAPON as i64);
    assert_eq!(get("death_engine"), c::DEATH_ENGINE as i64);
    assert_eq!(get("boss"), c::BOSS as i64);
    // The named ids must actually index the entries the Rust means by them.
    assert_eq!(
        doc.at("weapons").as_arr()[c::STARTING_WEAPON as usize].at("name").as_str(),
        c::WEAPONS[c::STARTING_WEAPON as usize].name
    );
    assert_eq!(
        doc.at("weapons").as_arr()[c::DEATH_ENGINE as usize].at("name").as_str(),
        c::WEAPONS[c::DEATH_ENGINE as usize].name
    );
    assert!(doc.at("enemies").as_arr()[c::BOSS as usize].at("boss").as_bool());

    assert_eq!(get("num_weapons"), c::WEAPONS.len() as i64);
    assert_eq!(get("num_enemies"), c::ENEMIES.len() as i64);
    assert_eq!(get("num_modifiers"), c::MODIFIERS.len() as i64);
    assert_eq!(get("frost_max_stacks"), c::FROST_MAX_STACKS as i64);
    assert_eq!(get("ramp_per_round"), c::RAMP_PER_ROUND as i64);
    assert_eq!(get("boss_contact_cadence"), c::BOSS_CONTACT_CADENCE as i64);
    assert_eq!(get("num_scopes"), c::NUM_SCOPES as i64);
    assert_eq!(get("num_armor_classes"), c::NUM_ARMOR_CLASSES as i64);
    assert_eq!(get("ramp_gentle_num"), c::RAMP_GENTLE.0);
    assert_eq!(get("ramp_gentle_den"), c::RAMP_GENTLE.1);
    assert_eq!(get("ramp_warn_num"), c::RAMP_WARN.0);
    assert_eq!(get("ramp_warn_den"), c::RAMP_WARN.1);
    assert_eq!(get("ramp_jump_num"), c::RAMP_JUMP.0);
    assert_eq!(get("ramp_jump_den"), c::RAMP_JUMP.1);
    assert_eq!(get("fixed_frac_bits"), determinism::Fixed::FRAC_BITS as i64);
    assert_eq!(get("fixed_one"), determinism::Fixed::ONE.raw());
    assert_eq!(get("enemy_ranged_dtype_shift"), ENEMY_RANGED_DTYPE_SHIFT as i64);
}

/// C1: "`timeline.roblox` carries the F1 rescale (boss at tick 9000, 600-tick
/// rounds). Both timelines ship so the fork's divergence is visible in one place."
#[test]
fn timeline_carries_both_schedules() {
    let doc = content();
    let t = doc.at("timeline");

    let steam = t.at("steam");
    assert_eq!(steam.at("boss_spawn_tick").as_i64(), c::BOSS_SPAWN_TICK as i64);
    assert_eq!(steam.at("round_ticks").as_i64(), sim::ROUND_TICKS as i64);
    assert_eq!(steam.at("ramp_interval").as_i64(), c::RAMP_INTERVAL as i64);
    assert_eq!(steam.at("gentle_ticks").as_i64(), c::GENTLE_TICKS as i64);
    assert_eq!(steam.at("warn_ticks").as_i64(), c::WARN_TICKS as i64);
    assert_eq!(steam.at("scale_step_1_tick").as_i64(), c::SCALE_STEP_1_TICK as i64);
    assert_eq!(steam.at("scale_step_2_tick").as_i64(), c::SCALE_STEP_2_TICK as i64);
    assert_eq!(steam.at("cliff_20_tick").as_i64(), c::CLIFF_20_TICK as i64);
    assert_eq!(steam.at("cliff_25_tick").as_i64(), c::CLIFF_25_TICK as i64);
    assert_eq!(steam.at("ramp_intervals_to_boss").as_i64(), 10);

    // `docs/10` F1: 5-minute run, 20 s rounds. The ramp interval is deliberately
    // NOT the round length — it is the Steam interval under the same /6 rescale,
    // so shop cadence (20 s) and difficulty cadence (30 s) are independent.
    let rbx = t.at("roblox");
    assert_eq!(rbx.at("boss_spawn_tick").as_i64(), 9000);
    assert_eq!(rbx.at("round_ticks").as_i64(), 600);
    assert_eq!(rbx.at("ramp_interval").as_i64(), 900);
    assert_eq!(rbx.at("ramp_per_round").as_i64(), rbx.at("round_ticks").as_i64());
    // 15 shop decisions per run (F1).
    assert_eq!(
        rbx.at("boss_spawn_tick").as_i64() / rbx.at("round_ticks").as_i64(),
        15
    );
    // THE curve-neutrality invariant behind F1's "rescale, not rebalance" claim:
    // the ramp compounds the SAME number of times on both timelines, so the
    // difficulty endpoint is unchanged and no per-interval factor needs retuning.
    // Tying ramp_interval to the 600-tick round would make this 15 vs 10 and move
    // the endpoint from ≈×5.56 to ≈×12.9. If this assert ever fails, F1 in
    // `docs/10` is no longer true and the ramp needs an explicit Roblox retune.
    assert_eq!(
        rbx.at("ramp_intervals_to_boss").as_i64(),
        steam.at("ramp_intervals_to_boss").as_i64(),
        "F1 curve-neutrality: ramp must compound equally on both timelines"
    );
    assert_eq!(rbx.at("ramp_interval").as_i64() * 6, steam.at("ramp_interval").as_i64());
    // The within-interval shape survives the rescale (gentle:warn stays 5:1).
    assert_eq!(rbx.at("gentle_ticks").as_i64(), 750);
    assert_eq!(rbx.at("warn_ticks").as_i64(), 150);
    assert_eq!(
        rbx.at("gentle_ticks").as_i64() + rbx.at("warn_ticks").as_i64(),
        rbx.at("ramp_interval").as_i64()
    );
    // Gates rescale by the same 1/6 factor as the boss tick.
    for key in ["scale_step_1_tick", "scale_step_2_tick", "cliff_20_tick", "cliff_25_tick"] {
        assert_eq!(rbx.at(key).as_i64() * 6, steam.at(key).as_i64(), "{key} rescale");
    }
    // Both schedules run on the sim's one clock.
    assert_eq!(steam.at("tick_hz").as_i64(), sim::TICK_HZ as i64);
    assert_eq!(rbx.at("tick_hz").as_i64(), sim::TICK_HZ as i64);
}

/// `content_hash` must be reproducible from the artifact alone: FNV-1a 64 over
/// the canonical serialization of the document minus its own `content_hash`.
#[test]
fn content_hash_is_self_verifying() {
    let doc = content();
    let hash = doc.at("content_hash").as_str().to_string();
    assert_eq!(hash.len(), 16);
    assert!(hash.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    assert_eq!(hash, format!("{:016x}", sim::export::content_hash()));

    let Value::Obj(kv) = doc else { panic!("top level must be an object") };
    let body = Value::Obj(kv.into_iter().filter(|(k, _)| k != "content_hash").collect());
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in body.canonical().as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    assert_eq!(format!("{h:016x}"), hash, "content_hash must cover the canonical body");
}

// ============================ parity vectors ============================

fn from_hex_i64(v: &Value) -> i64 {
    let t = v.as_str();
    assert_eq!(t.len(), 16, "hex operand must be 16 digits: {t:?}");
    u64::from_str_radix(t, 16).expect("hex operand") as i64
}

/// C4: the expected values must be what the Rust actually computes. Re-running
/// every `Fixed` case against `determinism` proves the file is an oracle and not
/// a hand-derived guess.
#[test]
fn fixed_vectors_reproduce_the_rust() {
    use determinism::Fixed as F;
    let doc = Value::parse(&read_artifact(OUTPUTS[1])).expect("fixed_vectors.json parses");
    assert_eq!(doc.at("frac_bits").as_i64(), F::FRAC_BITS as i64);
    assert_eq!(doc.at("one").as_i64(), F::ONE.raw());

    let mut n = 0usize;
    for key in ["from_int", "from_int_wrapping"] {
        for e in doc.at(key).as_arr() {
            assert_eq!(from_hex_i64(e.at("out")), F::from_int(from_hex_i64(e.at("i"))).raw());
            n += 1;
        }
    }
    for key in ["from_ratio", "from_ratio_wrapping"] {
        for e in doc.at(key).as_arr() {
            let (num, den) = (from_hex_i64(e.at("num")), from_hex_i64(e.at("den")));
            assert_eq!(from_hex_i64(e.at("out")), F::from_ratio(num, den).raw());
            n += 1;
        }
    }
    for e in doc.at("floor_to_int").as_arr() {
        assert_eq!(from_hex_i64(e.at("out")), F::from_raw(from_hex_i64(e.at("x"))).floor_to_int());
        n += 1;
    }
    for e in doc.at("neg").as_arr() {
        assert_eq!(from_hex_i64(e.at("out")), (-F::from_raw(from_hex_i64(e.at("a")))).raw());
        n += 1;
    }
    for (key, op) in [
        ("add", (|a: F, b: F| a + b) as fn(F, F) -> F),
        ("sub", |a, b| a - b),
        ("mul", |a: F, b: F| a.mul(b)),
        ("div", |a: F, b: F| a.div(b)),
    ] {
        for e in doc.at(key).as_arr() {
            let (a, b) = (from_hex_i64(e.at("a")), from_hex_i64(e.at("b")));
            assert_eq!(from_hex_i64(e.at("out")), op(F::from_raw(a), F::from_raw(b)).raw(), "{key}");
            n += 1;
        }
    }
    for e in doc.at("scale_i64").as_arr() {
        let (x, v) = (from_hex_i64(e.at("x")), from_hex_i64(e.at("v")));
        assert_eq!(from_hex_i64(e.at("out")), F::from_raw(x).scale_i64(v));
        n += 1;
    }
    for e in doc.at("sqrt").as_arr() {
        assert_eq!(from_hex_i64(e.at("out")), F::from_raw(from_hex_i64(e.at("a"))).sqrt().raw());
        n += 1;
    }
    assert!(n > 3000, "fixed vectors must be dense (got {n})");

    // The single most likely port divergence: `floor_to_int` is an ARITHMETIC
    // SHIFT (floors toward -inf), not truncation toward zero.
    let negatives = doc
        .at("floor_to_int")
        .as_arr()
        .iter()
        .filter(|e| {
            let x = from_hex_i64(e.at("x"));
            x < 0 && x % 65_536 != 0
        })
        .count();
    assert!(negatives > 50, "floor_to_int must densely cover negative non-multiples of ONE");
}

/// C4: `below` must reproduce the Rust rejection strategy exactly — same
/// outputs AND the same number of words drawn (hence `state_after`).
#[test]
fn rng_vectors_reproduce_the_rust() {
    use determinism::Rng;
    let doc = Value::parse(&read_artifact(OUTPUTS[2])).expect("rng_vectors.json parses");
    let u64_of = |v: &Value| u64::from_str_radix(v.as_str(), 16).expect("hex u64");

    for blk in doc.at("seeds").as_arr() {
        let seed = u64_of(blk.at("seed"));

        let mut r = Rng::from_seed(seed);
        for out in blk.at("next_u64").at("out").as_arr() {
            assert_eq!(u64_of(out), r.next_u64(), "next_u64 seed {seed:#x}");
        }
        assert_eq!(u64_of(blk.at("next_u64").at("state_after")), r.state());

        let mut r = Rng::from_seed(seed);
        for out in blk.at("next_u32").at("out").as_arr() {
            assert_eq!(out.as_i64(), r.next_u32() as i64, "next_u32 seed {seed:#x}");
        }
        assert_eq!(u64_of(blk.at("next_u32").at("state_after")), r.state());

        for case in blk.at("below").as_arr() {
            let n = case.at("n").as_i64() as u32;
            let mut r = Rng::from_seed(seed);
            for out in case.at("out").as_arr() {
                let got = r.below(n);
                assert_eq!(out.as_i64(), got as i64, "below({n}) seed {seed:#x}");
                assert!(n == 0 || got < n, "below({n}) out of range");
            }
            assert_eq!(u64_of(case.at("state_after")), r.state(), "below({n}) word count");
        }

        for case in blk.at("chance").as_arr() {
            let (num, den) = (case.at("num").as_i64() as u32, case.at("den").as_i64() as u32);
            let mut r = Rng::from_seed(seed);
            for out in case.at("out").as_arr() {
                assert_eq!(out.as_bool(), r.chance(num, den), "chance({num},{den})");
            }
            assert_eq!(u64_of(case.at("state_after")), r.state());
        }
    }

    for case in doc.at("derive").as_arr() {
        let master = u64_of(case.at("master"));
        let p = case.at("player").as_i64() as u32;
        let purpose = case.at("purpose").as_i64() as u32;
        let round = case.at("round").as_i64() as u32;
        let mut r = Rng::derive(master, p, purpose, round);
        assert_eq!(u64_of(case.at("state")), r.state(), "derive state");
        for out in case.at("next_u64").as_arr() {
            assert_eq!(u64_of(out), r.next_u64(), "derive stream");
        }
    }

    // Bounds large enough that Lemire's rejection loop fires must be present —
    // that branch is where an "equivalent" port silently desynchronizes.
    let ns: Vec<i64> = doc.at("seeds").as_arr()[0]
        .at("below")
        .as_arr()
        .iter()
        .map(|c| c.at("n").as_i64())
        .collect();
    assert!(ns.contains(&0), "below(0) early-out must be covered");
    assert!(ns.iter().any(|n| *n > 0x8000_0000), "high-rejection bound must be covered");
}
