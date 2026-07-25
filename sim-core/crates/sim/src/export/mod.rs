//! Build artifacts for the Roblox fork (`roblox/CONTRACTS.md` C1 + C4).
//!
//! Three files, all generated from the Rust tables so those tables stay the
//! single source of truth:
//!
//! | file | contract |
//! | --- | --- |
//! | `roblox/src/shared/content.json` | C1 — the full catalog |
//! | `roblox/test/vectors/fixed_vectors.json` | C4 — `Fixed` parity oracle |
//! | `roblox/test/vectors/rng_vectors.json` | C4 — `Rng` parity oracle |
//!
//! Invariants held here:
//! - **No floats anywhere in the output.** `Fixed` values are emitted as their
//!   raw `i64`; ratios stay as `(num, den)` integer operand pairs.
//! - **Every expected value is produced by running the Rust**, never derived by
//!   hand — the vector generators literally call `Fixed`/`Rng`.
//! - **Deterministic**: no wall-clock, no `HashMap` iteration, no platform RNG.
//!   The only randomness is a seeded [`Rng`] used to widen vector coverage.
//! - Engine-independent, dependency-free (see [`json`]).

pub mod json;

use crate::content as c;
use determinism::{Fixed, Rng};
use json::{int, ints, obj, s, Value};

// ===================== shared helpers =====================

/// The `(tag, a, b, c)` payload encoding shared by `Attack`, `WeaponAbility`,
/// `EnemyAbility` and `ModEffect` (C1).
fn words((tag, a, b, cc): (u8, i64, i64, i64)) -> Value {
    obj(vec![("tag", int(tag as i64)), ("a", int(a)), ("b", int(b)), ("c", int(cc))])
}

/// 16-digit lowercase hex, two's complement. Used for every `i64`/`u64` in the
/// **parity vectors**, where values reach `i64::MAX`/`MIN` — Luau numbers are
/// doubles and cannot hold those exactly, so they must arrive as a bit pattern
/// the port splits into limbs. (`content.json` needs none of this: every value
/// in the catalog is far inside the 53-bit safe range and stays a JSON number.)
fn hex(v: i64) -> Value {
    s(&format!("{:016x}", v as u64))
}
fn hex_u(v: u64) -> Value {
    s(&format!("{v:016x}"))
}

const HEX_NOTE: &str =
    "i64/u64 values are 16-digit lowercase hex, two's complement, big-endian; u32 values \
     and small counts are plain JSON integers. No floats appear anywhere in this file.";

// ===================== C1 — content.json =====================

/// FNV-1a 64 over UTF-8 bytes — the same function `determinism::Checksum`
/// implements over words, applied bytewise so `content_hash` is reproducible by
/// any port from the artifact alone.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The catalog document, **without** its `content_hash` member. This is what
/// the hash is computed over (in [`Value::canonical`] form).
fn content_body() -> Value {
    let damage_matrix = Value::Arr(
        (0..5u8)
            .map(|dt| {
                ints((0..c::NUM_ARMOR_CLASSES as u8).map(|ac| c::damage_multiplier(dt, ac).raw()))
            })
            .collect(),
    );

    let weapons = Value::Arr(
        c::WEAPONS
            .iter()
            .map(|w| {
                obj(vec![
                    ("name", s(w.name)),
                    ("rarity", int(w.rarity as i64)),
                    ("cost", int(w.cost)),
                    ("damage", int(w.damage)),
                    ("damage_type", int(w.damage_type as i64)),
                    ("attack", words(w.attack.words())),
                    ("cooldown_ticks", int(w.cooldown_ticks as i64)),
                    ("range", int(w.range)),
                    ("proj_speed", int(w.proj_speed)),
                    (
                        "on_hit",
                        obj(vec![
                            ("poison_dps", int(w.on_hit.poison_dps)),
                            ("poison_ticks", int(w.on_hit.poison_ticks as i64)),
                            ("frost_stacks", int(w.on_hit.frost_stacks as i64)),
                            ("fire_stacks", int(w.on_hit.fire_stacks as i64)),
                            ("stun_ticks", int(w.on_hit.stun_ticks as i64)),
                        ]),
                    ),
                    ("ability", words(w.ability.words())),
                ])
            })
            .collect(),
    );

    let enemies = Value::Arr(
        c::ENEMIES
            .iter()
            .map(|e| {
                obj(vec![
                    ("name", s(e.name)),
                    ("base_hp", int(e.base_hp)),
                    ("move_speed", int(e.move_speed)),
                    ("contact_damage", int(e.contact_damage)),
                    ("bounty", int(e.bounty)),
                    ("armor_class", int(e.armor_class as i64)),
                    ("archetype", int(e.archetype.id() as i64)),
                    ("ability", words(e.ability.words())),
                    ("boss", Value::Bool(e.boss)),
                ])
            })
            .collect(),
    );

    let modifiers = Value::Arr(
        c::MODIFIERS
            .iter()
            .map(|m| {
                obj(vec![
                    ("name", s(m.name)),
                    ("rarity", int(m.rarity as i64)),
                    ("cost", int(m.cost)),
                    ("effects", Value::Arr(m.effects.iter().map(|e| words(e.words())).collect())),
                    (
                        "ramp",
                        match m.ramp {
                            None => Value::Null,
                            Some(r) => obj(vec![
                                ("effect", words(r.effect.words())),
                                ("interval_ticks", int(r.interval_ticks as i64)),
                            ]),
                        },
                    ),
                ])
            })
            .collect(),
    );

    let wave = |ws: &[c::WaveSpawn]| {
        Value::Arr(
            ws.iter()
                .map(|w| {
                    obj(vec![
                        ("enemy", int(w.enemy as i64)),
                        ("cadence_ticks", int(w.cadence_ticks as i64)),
                        ("start_tick", int(w.start_tick as i64)),
                    ])
                })
                .collect(),
        )
    };

    let spawn_ring = Value::Arr(
        c::SPAWN_RING
            .iter()
            .map(|p| obj(vec![("x", int(p.x.raw())), ("y", int(p.y.raw()))]))
            .collect(),
    );

    Value::Obj(vec![
        ("schema_version".into(), int(1)),
        ("damage_matrix".into(), damage_matrix),
        ("weapons".into(), weapons),
        ("enemies".into(), enemies),
        ("modifiers".into(), modifiers),
        ("waves".into(), wave(c::WAVE_M0)),
        // NOT in the C1 sketch — see the exporter's report. The boss phase is
        // unreachable without it, and it is the same `WaveSpawn` shape.
        ("boss_escort".into(), wave(c::BOSS_ESCORT)),
        // Likewise not in C1: spawn positions are content, and an arena cannot
        // place enemies without them. Fixed raws, no floats.
        ("spawn_ring".into(), spawn_ring),
        ("timeline".into(), timeline()),
        ("constants".into(), constants()),
    ])
}

// ---- timeline (C1 + `docs/10` F1) ------------------------------------------

/// Roblox run length: boss at 5 min @ 30 Hz (`docs/10` F1).
pub const ROBLOX_BOSS_SPAWN_TICK: u32 = 9000;
/// Roblox round: 20 s @ 30 Hz (`docs/10` F1) → 15 shop decisions per run.
pub const ROBLOX_ROUND_TICKS: u32 = 600;
/// Roblox difficulty ramp interval (`docs/10` F1). Deliberately NOT equal to the
/// round length: it is the Steam interval put through the same `/6` rescale, so
/// `boss_spawn_tick / ramp_interval` stays at Steam's **10** compounding steps.
/// Tying it to the 600-tick round instead would give 15 steps and move the
/// difficulty endpoint from ≈×5.56 to ≈×12.9 — a rebalance, not a rescale.
/// Shop cadence (20 s) and difficulty cadence (30 s) are simply independent.
pub const ROBLOX_RAMP_INTERVAL: u32 = rescale(c::RAMP_INTERVAL);

/// Rescale a Steam tick onto the Roblox timeline (`× 9000/54000`, i.e. `/6`).
/// Integer-only; every gate constant divides evenly.
const fn rescale(tick: u32) -> u32 {
    ((tick as u64 * ROBLOX_BOSS_SPAWN_TICK as u64) / c::BOSS_SPAWN_TICK as u64) as u32
}

fn timeline_entry(
    round_ticks: u32,
    boss_spawn_tick: u32,
    ramp_interval: u32,
    gentle_ticks: u32,
    warn_ticks: u32,
    ramp_per_round: u32,
    step1: u32,
    step2: u32,
    cliff20: u32,
    cliff25: u32,
) -> Value {
    obj(vec![
        ("tick_hz", int(crate::TICK_HZ as i64)),
        ("round_ticks", int(round_ticks as i64)),
        ("boss_spawn_tick", int(boss_spawn_tick as i64)),
        ("ramp_interval", int(ramp_interval as i64)),
        // Intervals compounded before the boss = boss_spawn_tick / ramp_interval.
        // Both timelines are 10, which is what keeps the F1 rescale curve-neutral:
        // same compounding count → same difficulty endpoint, no retuning.
        ("ramp_intervals_to_boss", int((boss_spawn_tick / ramp_interval) as i64)),
        ("gentle_ticks", int(gentle_ticks as i64)),
        ("warn_ticks", int(warn_ticks as i64)),
        ("ramp_per_round", int(ramp_per_round as i64)),
        ("scale_step_1_tick", int(step1 as i64)),
        ("scale_step_2_tick", int(step2 as i64)),
        ("cliff_20_tick", int(cliff20 as i64)),
        ("cliff_25_tick", int(cliff25 as i64)),
    ])
}

fn timeline() -> Value {
    let steam = timeline_entry(
        crate::ROUND_TICKS,
        c::BOSS_SPAWN_TICK,
        c::RAMP_INTERVAL,
        c::GENTLE_TICKS,
        c::WARN_TICKS,
        c::RAMP_PER_ROUND,
        c::SCALE_STEP_1_TICK,
        c::SCALE_STEP_2_TICK,
        c::CLIFF_20_TICK,
        c::CLIFF_25_TICK,
    );
    // The gentle/warn split keeps its Steam PROPORTION of the interval
    // (4500:900 = 5:1), so the within-interval curve shape survives the rescale.
    let gentle = ROBLOX_RAMP_INTERVAL * c::GENTLE_TICKS / c::RAMP_INTERVAL;
    let roblox = timeline_entry(
        ROBLOX_ROUND_TICKS,
        ROBLOX_BOSS_SPAWN_TICK,
        ROBLOX_RAMP_INTERVAL,
        gentle,
        ROBLOX_RAMP_INTERVAL - gentle,
        ROBLOX_ROUND_TICKS,
        rescale(c::SCALE_STEP_1_TICK),
        rescale(c::SCALE_STEP_2_TICK),
        rescale(c::CLIFF_20_TICK),
        rescale(c::CLIFF_25_TICK),
    );
    obj(vec![("steam", steam), ("roblox", roblox)])
}

fn constants() -> Value {
    obj(vec![
        ("tick_hz", int(crate::TICK_HZ as i64)),
        ("frost_max_stacks", int(c::FROST_MAX_STACKS as i64)),
        ("ramp_per_round", int(c::RAMP_PER_ROUND as i64)),
        ("boss_contact_cadence", int(c::BOSS_CONTACT_CADENCE as i64)),
        // Named ids (C1: "Named ids go in constants").
        ("starting_weapon", int(c::STARTING_WEAPON as i64)),
        ("death_engine", int(c::DEATH_ENGINE as i64)),
        ("boss", int(c::BOSS as i64)),
        // Table sizes — index-is-identity, so a port can assert its own load.
        ("num_weapons", int(c::WEAPONS.len() as i64)),
        ("num_enemies", int(c::ENEMIES.len() as i64)),
        ("num_modifiers", int(c::MODIFIERS.len() as i64)),
        ("num_damage_types", int(5)),
        ("num_armor_classes", int(c::NUM_ARMOR_CLASSES as i64)),
        ("num_scopes", int(c::NUM_SCOPES as i64)),
        // Damage types / armor classes (matrix indices).
        ("dmg_normal", int(c::DMG_NORMAL as i64)),
        ("dmg_piercing", int(c::DMG_PIERCING as i64)),
        ("dmg_magic", int(c::DMG_MAGIC as i64)),
        ("dmg_siege", int(c::DMG_SIEGE as i64)),
        ("dmg_chaos", int(c::DMG_CHAOS as i64)),
        ("armor_light", int(c::ARMOR_LIGHT as i64)),
        ("armor_medium", int(c::ARMOR_MEDIUM as i64)),
        ("armor_fortified", int(c::ARMOR_FORTIFIED as i64)),
        // Damage scopes: attack-class scope id == `Attack` payload tag (0..5),
        // then the two range buckets, then the four rarities.
        ("range_scope_short", int(6)),
        ("range_scope_long", int(7)),
        ("range_scope_short_max", int(600)),
        ("rarity_scope_base", int(8)),
        // Enemy archetype ids (C1 leaves this enum's encoding unspecified).
        ("archetype_swarm", int(0)),
        ("archetype_tank", int(1)),
        ("archetype_fast", int(2)),
        ("archetype_caster", int(3)),
        ("archetype_ranged", int(4)),
        ("archetype_boss", int(5)),
        ("archetype_inert", int(6)),
        // `EnemyAbility::RangedAttack` packs cooldown_ticks | damage_type << 32
        // into operand `b` (four fields, three operand slots).
        ("enemy_ranged_dtype_shift", int(c::ENEMY_RANGED_DTYPE_SHIFT as i64)),
        // `enemy_hp_mult` curve parameters, as (num, den) integer ratios.
        ("ramp_gentle_num", int(c::RAMP_GENTLE.0)),
        ("ramp_gentle_den", int(c::RAMP_GENTLE.1)),
        ("ramp_warn_num", int(c::RAMP_WARN.0)),
        ("ramp_warn_den", int(c::RAMP_WARN.1)),
        ("ramp_jump_num", int(c::RAMP_JUMP.0)),
        ("ramp_jump_den", int(c::RAMP_JUMP.1)),
        // Early-wave cadence knobs (the wave table references them by value).
        ("early_grunt_cadence", int(c::EARLY_GRUNT_CADENCE as i64)),
        ("early_peon_cadence", int(c::EARLY_PEON_CADENCE as i64)),
        ("early_raider_cadence", int(c::EARLY_RAIDER_CADENCE as i64)),
        ("early_bandit_cadence", int(c::EARLY_BANDIT_CADENCE as i64)),
        // Fixed-point shape (C2 must agree).
        ("fixed_frac_bits", int(Fixed::FRAC_BITS as i64)),
        ("fixed_one", int(Fixed::ONE.raw())),
    ])
}

/// The C1 catalog, with `content_hash` filled in.
pub fn content_document() -> Value {
    let body = content_body();
    let hash = fnv1a64(body.canonical().as_bytes());
    let Value::Obj(mut kv) = body else { unreachable!() };
    // `content_hash` sits directly after `schema_version` (C1 top-level shape).
    kv.insert(1, ("content_hash".to_string(), s(&format!("{hash:016x}"))));
    Value::Obj(kv)
}

/// `content_hash` = FNV-1a 64 over the canonical (compact, insertion-ordered)
/// serialization of the catalog with the `content_hash` member removed.
pub fn content_hash() -> u64 {
    fnv1a64(content_body().canonical().as_bytes())
}

/// The bytes written to `roblox/src/shared/content.json`.
pub fn content_json() -> String {
    content_document().pretty(2)
}

// ===================== C4 — Fixed parity vectors =====================

/// Curated raw `i64` operands: zero, ±1, the fractional boundary (±ONE and its
/// neighbours), 32-bit edges, the Q47.16 integer-range edge (2^47), and the
/// saturation boundaries themselves.
const RAWS: &[i64] = &[
    0,
    1,
    -1,
    2,
    -2,
    65_535,
    65_536,
    65_537,
    -65_535,
    -65_536,
    -65_537,
    100_000,
    -100_000,
    1_234_567,
    -1_234_567,
    0x7fff_ffff,
    -0x8000_0000,
    1 << 40,
    -(1 << 40),
    1 << 47,
    -(1 << 47),
    i64::MAX,
    i64::MAX - 1,
    i64::MIN,
    i64::MIN + 1,
];

/// Deterministic coverage widener. Seeded [`Rng`] — never a platform RNG.
struct Spread(Rng);
impl Spread {
    fn new(seed: u64) -> Spread {
        Spread(Rng::from_seed(seed))
    }
    /// Full-range `i64`.
    fn wide(&mut self) -> i64 {
        self.0.next_u64() as i64
    }
    /// `i64` within `±2^bits`, so results mostly land *inside* the range and
    /// exercise real arithmetic rather than pure saturation.
    fn narrow(&mut self, bits: u32) -> i64 {
        let v = (self.0.next_u64() >> (64 - bits)) as i64;
        if self.0.next_u64() & 1 == 0 {
            v
        } else {
            -v
        }
    }
}

fn binary_cases(op: impl Fn(i64, i64) -> Option<i64>, seed: u64) -> Value {
    let mut out = Vec::new();
    let push = |a: i64, b: i64, out: &mut Vec<Value>| {
        if let Some(r) = op(a, b) {
            out.push(obj(vec![("a", hex(a)), ("b", hex(b)), ("out", hex(r))]));
        }
    };
    for &a in RAWS {
        for &b in RAWS {
            push(a, b, &mut out);
        }
    }
    let mut sp = Spread::new(seed);
    for _ in 0..96 {
        let (a, b) = (sp.wide(), sp.wide());
        push(a, b, &mut out);
    }
    for _ in 0..96 {
        let (a, b) = (sp.narrow(38), sp.narrow(38));
        push(a, b, &mut out);
    }
    Value::Arr(out)
}

fn unary_cases(op: impl Fn(i64) -> i64, extra: &[i64], seed: u64) -> Value {
    let mut out = Vec::new();
    let push = |a: i64, out: &mut Vec<Value>| {
        out.push(obj(vec![("a", hex(a)), ("out", hex(op(a)))]));
    };
    for &a in RAWS {
        push(a, &mut out);
    }
    for &a in extra {
        push(a, &mut out);
    }
    let mut sp = Spread::new(seed);
    for _ in 0..64 {
        let a = sp.wide();
        push(a, &mut out);
    }
    Value::Arr(out)
}

/// The `floor_to_int` set, deliberately DENSE around negative multiples of ONE
/// — arithmetic shift floors toward −∞, and a port that truncates toward zero
/// diverges exactly here.
fn floor_cases() -> Value {
    let mut xs: Vec<i64> = RAWS.to_vec();
    for k in -8i64..=8 {
        let base = k * 65_536;
        xs.extend([base - 1, base, base + 1, base + 32_768, base - 32_768]);
    }
    let mut t = -1_000_000i64;
    while t <= 1_000_000 {
        xs.push(t);
        t += 37_117;
    }
    let mut sp = Spread::new(0x_f100_1234_5678_9abc);
    for _ in 0..64 {
        xs.push(sp.wide());
    }
    for _ in 0..64 {
        xs.push(sp.narrow(24));
    }
    Value::Arr(
        xs.into_iter()
            .map(|x| obj(vec![("x", hex(x)), ("out", hex(Fixed::from_raw(x).floor_to_int()))]))
            .collect(),
    )
}

fn from_int_cases() -> (Value, Value) {
    let mut is: Vec<i64> = vec![
        0,
        1,
        -1,
        2,
        -2,
        100,
        -100,
        1000,
        -1000,
        65_535,
        -65_535,
        65_536,
        -65_536,
        1_000_000,
        -1_000_000,
        1 << 30,
        -(1 << 30),
        1 << 40,
        -(1 << 40),
        (1 << 46) - 1,
        -((1 << 46) - 1),
    ];
    let mut sp = Spread::new(0x_f1f1_0000_0000_0001);
    for _ in 0..64 {
        is.push(sp.narrow(46));
    }
    // Values whose `i << 16` sheds high bits. Rust's `<<` discards them
    // silently (it is NOT saturating, unlike mul/div) — a real behavioural
    // difference a port must decide about deliberately, so these ship in their
    // own list rather than mixed into the main one.
    let wrapping: Vec<i64> = vec![1 << 47, -(1 << 47), 1 << 50, i64::MAX, i64::MIN, i64::MAX - 1];

    let mk = |v: &[i64]| {
        Value::Arr(
            v.iter()
                .map(|&i| obj(vec![("i", hex(i)), ("out", hex(Fixed::from_int(i).raw()))]))
                .collect(),
        )
    };
    (mk(&is), mk(&wrapping))
}

fn from_ratio_cases() -> (Value, Value) {
    let nums: [i64; 15] =
        [0, 1, -1, 2, -2, 3, -3, 7, -7, 10, -10, 100, -100, 1_000_000, -1_000_000];
    let dens: [i64; 12] = [1, -1, 2, -2, 3, -3, 5, -5, 60, 100, 1000, 65_536];
    let mut ok = Vec::new();
    let mut wrapping = Vec::new();
    let push = |n: i64, d: i64, ok: &mut Vec<Value>, wr: &mut Vec<Value>| {
        if d == 0 {
            return;
        }
        let exact = ((n as i128) << Fixed::FRAC_BITS) / d as i128;
        let e = obj(vec![
            ("num", hex(n)),
            ("den", hex(d)),
            ("out", hex(Fixed::from_ratio(n, d).raw())),
        ]);
        // `from_ratio` narrows with `as i64` (truncating), NOT `saturating` —
        // the asymmetry with mul/div is exactly why these are separated.
        if exact > i64::MAX as i128 || exact < i64::MIN as i128 {
            wr.push(e);
        } else {
            ok.push(e);
        }
    };
    for &n in &nums {
        for &d in &dens {
            push(n, d, &mut ok, &mut wrapping);
        }
    }
    let mut sp = Spread::new(0x_f1f1_0000_0000_0002);
    for _ in 0..96 {
        let (n, d) = (sp.narrow(40), sp.narrow(20));
        push(n, d, &mut ok, &mut wrapping);
    }
    for &n in &[i64::MAX, i64::MIN, 1 << 47, -(1 << 47), i64::MAX - 1] {
        for &d in &[1i64, -1, 3] {
            push(n, d, &mut ok, &mut wrapping);
        }
    }
    (Value::Arr(ok), Value::Arr(wrapping))
}

fn scale_cases() -> Value {
    let vs: [i64; 14] = [
        0,
        1,
        -1,
        75,
        -75,
        1000,
        -1000,
        45_000,
        1_000_000,
        -1_000_000,
        i64::MAX,
        i64::MIN,
        1 << 40,
        -(1 << 40),
    ];
    let mut out = Vec::new();
    for &x in RAWS {
        for &v in &vs {
            out.push(obj(vec![
                ("x", hex(x)),
                ("v", hex(v)),
                ("out", hex(Fixed::from_raw(x).scale_i64(v))),
            ]));
        }
    }
    let mut sp = Spread::new(0x_f1f1_0000_0000_0003);
    for _ in 0..96 {
        let (x, v) = (sp.narrow(40), sp.narrow(30));
        out.push(obj(vec![
            ("x", hex(x)),
            ("v", hex(v)),
            ("out", hex(Fixed::from_raw(x).scale_i64(v))),
        ]));
    }
    Value::Arr(out)
}

fn sqrt_cases() -> Value {
    // `Fixed::sqrt` asserts on negatives, so the vectors are non-negative only.
    let mut xs: Vec<i64> = RAWS.iter().copied().filter(|v| *v >= 0).collect();
    xs.extend([
        3,
        4,
        9,
        16,
        4 * 65_536,
        9 * 65_536,
        100 * 65_536,
        10_000 * 65_536,
        65_536 * 65_536,
        (1 << 46) + 12_345,
    ]);
    let mut sp = Spread::new(0x_f1f1_0000_0000_0004);
    for _ in 0..64 {
        xs.push((sp.0.next_u64() >> 1) as i64);
    }
    for _ in 0..32 {
        xs.push((sp.0.next_u64() >> 32) as i64);
    }
    Value::Arr(
        xs.into_iter()
            .map(|x| obj(vec![("a", hex(x)), ("out", hex(Fixed::from_raw(x).sqrt().raw()))]))
            .collect(),
    )
}

/// The bytes written to `roblox/test/vectors/fixed_vectors.json`.
pub fn fixed_vectors_json() -> String {
    fixed_vectors_document().pretty(2)
}

pub fn fixed_vectors_document() -> Value {
    let f = Fixed::from_raw;
    let (from_int, from_int_wrapping) = from_int_cases();
    let (from_ratio, from_ratio_wrapping) = from_ratio_cases();
    obj(vec![
        ("schema_version", int(1)),
        ("encoding", s(HEX_NOTE)),
        ("frac_bits", int(Fixed::FRAC_BITS as i64)),
        ("one", int(Fixed::ONE.raw())),
        ("from_int", from_int),
        ("from_int_wrapping", from_int_wrapping),
        ("from_ratio", from_ratio),
        ("from_ratio_wrapping", from_ratio_wrapping),
        ("floor_to_int", floor_cases()),
        ("neg", unary_cases(|a| (-f(a)).raw(), &[], 0x_f1f1_0000_0000_0005)),
        ("add", binary_cases(|a, b| Some((f(a) + f(b)).raw()), 0x_f1f1_0000_0000_0006)),
        ("sub", binary_cases(|a, b| Some((f(a) - f(b)).raw()), 0x_f1f1_0000_0000_0007)),
        ("mul", binary_cases(|a, b| Some(f(a).mul(f(b)).raw()), 0x_f1f1_0000_0000_0008)),
        // `div` by zero panics in Rust (i128 division); no vector covers it.
        (
            "div",
            binary_cases(
                |a, b| (b != 0).then(|| f(a).div(f(b)).raw()),
                0x_f1f1_0000_0000_0009,
            ),
        ),
        ("scale_i64", scale_cases()),
        ("sqrt", sqrt_cases()),
    ])
}

// ===================== C4 — Rng parity vectors =====================

/// Seeds chosen to cover 0, small, the golden-ratio increment itself, high-bit
/// patterns, and `u64::MAX`.
const SEEDS: &[u64] = &[
    0,
    1,
    2,
    12_345,
    0x9E37_79B9_7F4A_7C15,
    0xDEAD_BEEF_CAFE_F00D,
    0xFFFF_FFFF_0000_0000,
    u64::MAX,
];

/// `below` bounds: 0 (the early-out), powers and non-powers of two, and bounds
/// large enough that Lemire's rejection branch fires roughly half the time —
/// that branch is where an "equivalent but different" port desynchronizes.
const BELOW_NS: &[u32] = &[
    0,
    1,
    2,
    3,
    5,
    6,
    7,
    8,
    10,
    100,
    1000,
    65_535,
    0x4000_0001,
    0x8000_0000,
    0x8000_0001,
    0xC000_0000,
    0xFFFF_FFFF,
];

const CHANCE_PS: &[(u32, u32)] = &[(0, 1), (1, 1), (1, 2), (1, 3), (5, 200), (25, 100), (3, 7)];

fn rng_block(seed: u64) -> Value {
    const N: usize = 16;

    let mut r = Rng::from_seed(seed);
    let u64s: Vec<Value> = (0..N).map(|_| hex_u(r.next_u64())).collect();
    let u64_state = r.state();

    let mut r = Rng::from_seed(seed);
    let u32s: Vec<Value> = (0..N).map(|_| int(r.next_u32() as i64)).collect();
    let u32_state = r.state();

    let below = Value::Arr(
        BELOW_NS
            .iter()
            .map(|&n| {
                let mut r = Rng::from_seed(seed);
                let outs: Vec<Value> = (0..8).map(|_| int(r.below(n) as i64)).collect();
                obj(vec![
                    ("n", int(n as i64)),
                    ("out", Value::Arr(outs)),
                    // The decisive field: a port that draws a different NUMBER
                    // of words gets the same visible outputs but a wrong state.
                    ("state_after", hex_u(r.state())),
                ])
            })
            .collect(),
    );

    let chance = Value::Arr(
        CHANCE_PS
            .iter()
            .map(|&(num, den)| {
                let mut r = Rng::from_seed(seed);
                let outs: Vec<Value> =
                    (0..N).map(|_| Value::Bool(r.chance(num, den))).collect();
                obj(vec![
                    ("num", int(num as i64)),
                    ("den", int(den as i64)),
                    ("out", Value::Arr(outs)),
                    ("state_after", hex_u(r.state())),
                ])
            })
            .collect(),
    );

    obj(vec![
        ("seed", hex_u(seed)),
        (
            "next_u64",
            obj(vec![("out", Value::Arr(u64s)), ("state_after", hex_u(u64_state))]),
        ),
        (
            "next_u32",
            obj(vec![("out", Value::Arr(u32s)), ("state_after", hex_u(u32_state))]),
        ),
        ("below", below),
        ("chance", chance),
    ])
}

fn derive_cases() -> Value {
    let masters: [u64; 4] = [0, 1, 0xDEAD_BEEF_CAFE_F00D, u64::MAX];
    let coords: [(u32, u32, u32); 8] = [
        (0, 0, 0),
        (1, 0, 0),
        (0, 1, 0),
        (0, 0, 1),
        (3, 2, 7),
        (7, 5, 29),
        (u32::MAX, u32::MAX, u32::MAX),
        (2, 0, 1000),
    ];
    let mut out = Vec::new();
    for &m in &masters {
        for &(p, purpose, round) in &coords {
            let mut r = Rng::derive(m, p, purpose, round);
            let state = r.state();
            let firsts: Vec<Value> = (0..8).map(|_| hex_u(r.next_u64())).collect();
            out.push(obj(vec![
                ("master", hex_u(m)),
                ("player", int(p as i64)),
                ("purpose", int(purpose as i64)),
                ("round", int(round as i64)),
                ("state", hex_u(state)),
                ("next_u64", Value::Arr(firsts)),
            ]));
        }
    }
    Value::Arr(out)
}

pub fn rng_vectors_document() -> Value {
    obj(vec![
        ("schema_version", int(1)),
        ("encoding", s(HEX_NOTE)),
        ("seeds", Value::Arr(SEEDS.iter().map(|&s| rng_block(s)).collect())),
        ("derive", derive_cases()),
    ])
}

/// The bytes written to `roblox/test/vectors/rng_vectors.json`.
pub fn rng_vectors_json() -> String {
    rng_vectors_document().pretty(4)
}

// ===================== output paths =====================

/// Repo-relative paths of the three generated artifacts, in the order
/// [`content_json`], [`fixed_vectors_json`], [`rng_vectors_json`].
pub const OUTPUTS: [&str; 3] = [
    "roblox/src/shared/content.json",
    "roblox/test/vectors/fixed_vectors.json",
    "roblox/test/vectors/rng_vectors.json",
];

/// The three artifacts' contents, in [`OUTPUTS`] order.
pub fn all_artifacts() -> [String; 3] {
    [content_json(), fixed_vectors_json(), rng_vectors_json()]
}

/// Repository root, resolved from the crate manifest at COMPILE time so the
/// exporter and its tests agree regardless of the caller's working directory.
pub fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .canonicalize()
        .expect("resolve repo root from CARGO_MANIFEST_DIR")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_byte_stable() {
        assert_eq!(content_json(), content_json());
        assert_eq!(fixed_vectors_json(), fixed_vectors_json());
        assert_eq!(rng_vectors_json(), rng_vectors_json());
    }

    #[test]
    fn no_floats_are_emitted() {
        // The parser errors on any number carrying a '.'/'e', so a successful
        // reparse of each artifact IS the "no floats anywhere" proof (C1).
        for (path, a) in OUTPUTS.iter().zip(all_artifacts()) {
            Value::parse(&a).unwrap_or_else(|e| panic!("{path}: {e}"));
        }
    }
}
