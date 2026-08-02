//! Oracle traces for the R3 Luau transcription (`roblox/SIM-SPEC.md` §S6).
//!
//! The R3 done-condition is *byte-identical per-tick checksums* between the Luau
//! port and this Rust core. This module is the Rust half of that equality: it
//! drives [`crate::step`] with the deterministic [`Bot`], records the per-tick
//! `state_checksum`, and writes one JSON document per seed into
//! `roblox/test/traces/`.
//!
//! Invariants held here (they are the reason the corpus is trustworthy):
//! - **Observe, never mutate.** The bot reads `&ArenaState`; nothing in this
//!   module writes to the state or draws from a sim RNG stream. Removing the
//!   tracing would not change a single checksum.
//! - **No floats, no wall-clock, no platform RNG.** Every emitted number is an
//!   integer or a 16-digit hex `u64` (C4). Running the exporter twice produces
//!   byte-identical files.
//! - **`content_hash` is embedded**, so a trace generated against an older
//!   catalog fails loudly instead of silently disagreeing.
//! - **The field dump is not a second copy of `checksum()`.** [`fields`] mirrors
//!   `lib.rs::checksum` field-for-field, and [`checksum_of_fields`] re-folds the
//!   dump back into a `u64`; the test suite asserts the two agree on live states,
//!   so the debugging tool cannot drift away from the thing it explains.

use super::json::{int, obj, s, Value};
use crate::bot::{Bot, Challenge};
use crate::{checksum, content, step, ArenaState, Input, OfferKind};
use determinism::Checksum;

/// Trace document schema version (`SIM-SPEC.md` §S6).
pub const SCHEMA_VERSION: i64 = 1;

/// Every trace runs player 0. `player_id` feeds `Rng::derive` and the checksum,
/// but §S6 gives the trace document no field for it, so it is pinned rather than
/// varied — a varying value could not be recovered from the artifact.
pub const PLAYER_ID: u32 = 0;

/// Directory (repo-relative) holding the corpus.
pub const TRACE_DIR: &str = "roblox/test/traces";

// ===================== input encoding =====================

/// `Input` → the `code` integer stored in a trace's input log.
///
/// `code = kind * 256 + slot`, where `kind` is the `ids::Input` variant's
/// declaration index (`0` Noop, `1` BuyOffer, `2` Reroll, `3` Clear) and `slot`
/// is `0` for everything but `BuyOffer`. Decode with
/// `kind = code // 256; slot = code % 256`.
///
/// §S6 fixes the *field name* (`code`) but not the encoding; this is the
/// definition the corpus uses and it is restated in `MANIFEST.json`.
pub fn input_code(i: Input) -> i64 {
    match i {
        Input::Noop => 0,
        Input::BuyOffer { slot } => 256 + slot as i64,
        Input::Reroll => 2 * 256,
        Input::Clear => 3 * 256,
    }
}

/// Inverse of [`input_code`]. `None` for a code no variant claims.
pub fn input_from_code(code: i64) -> Option<Input> {
    let (kind, slot) = (code / 256, code % 256);
    match (kind, slot) {
        (0, 0) => Some(Input::Noop),
        (1, sl) => Some(Input::BuyOffer { slot: sl as u8 }),
        (2, 0) => Some(Input::Reroll),
        (3, 0) => Some(Input::Clear),
        _ => None,
    }
}

/// Human-readable rendering of an input code, for `--verbose`.
pub fn input_name(i: Input) -> String {
    match i {
        Input::Noop => "Noop".to_string(),
        Input::BuyOffer { slot } => format!("BuyOffer{{slot:{slot}}}"),
        Input::Reroll => "Reroll".to_string(),
        Input::Clear => "Clear".to_string(),
    }
}

// ===================== the corpus =====================

/// One trace in the corpus: a seed, a run length, and what it is here to cover.
pub struct TraceSpec {
    /// File stem under [`TRACE_DIR`] (the file is `<name>.json`).
    pub name: &'static str,
    pub seed: u64,
    pub ticks: u32,
    /// The bot's self-imposed purchase constraint. `Challenge::None` is the
    /// default survivor; the constrained modes exist because they steer the bot
    /// into weapon classes (and therefore combat phases — hazards, minions,
    /// frost/fire/stun) that the greedy default bot happens never to buy. The
    /// challenge only filters *purchases*; it never bends the sim, so a trace is
    /// still a plain `(seed, input log)` replay for the Luau side.
    pub challenge: Challenge,
    /// Editorial note carried into `MANIFEST.json` — why this trace exists.
    pub exercises: &'static str,
}

/// The Rust core's boss spawns at `content::BOSS_SPAWN_TICK` (54000 — 30 min).
/// A full-length trace runs 1200 ticks (40 s) past it, which is enough to cover
/// the boss escort flood, the boss's contact attrition and, for most seeds, the
/// tank's death inside the boss phase.
pub const FULL_TICKS: u32 = content::BOSS_SPAWN_TICK + 1200;

/// The corpus, ordered by run length so the Luau side can bring modules up on
/// the cheap traces first. Two kinds of entry live here:
///
/// - **Narrative traces** (`smoke` / `short` / `mid` / `full` / `purist`): whole
///   matches, chosen from a reconnaissance sweep so the set spans the bot
///   archetypes, three death timings (mid-game, late-game, in-boss-phase) and
///   one seed that survives the whole run.
/// - **Coverage probes** (the 1.2k–2.4k-tick entries): seeds picked with
///   `--scan` purely because they *reach a behaviour* the narrative traces stopped
///   touching. A behaviour has to OCCUR to be covered, not run to completion, so
///   these are deliberately the shortest seed found that arms the flag —
///   `manifest_describes_the_committed_corpus` is satisfied for a few thousand
///   ticks instead of another 55,200-tick replay through the interpreter.
///
/// When a balance change moves what the bot buys, that test goes red naming the
/// behaviours that fell out; re-pick with
/// `export-traces --scan <seeds> <ticks> [<challenge>]` and replace the probes.
/// Do not delete the flag — an unreachable behaviour is a finding about the
/// *game*, not a licence to stop testing it.
///
/// Trimming this list is a one-line edit; every consumer (writer, `--check`,
/// `MANIFEST.json`, the replay test) is driven from it.
pub const TRACES: &[TraceSpec] = &[
    TraceSpec {
        name: "smoke-seed-4",
        seed: 4,
        ticks: 900,
        challenge: Challenge::None,
        exercises: "SMOKE (fastest inner loop, ~20 KB): tick-0 shop generation, the first \
                    round boundary at 900, opening buys, early spawns and projectile flight. \
                    No status effect lands this early, so a divergence here is in the core \
                    loop rather than in Status. Start here when bringing a module up.",
    },
    TraceSpec {
        name: "purist4-seed-3045",
        seed: 3045,
        ticks: 1200,
        challenge: Challenge::Purist(4),
        exercises: "PROBE, HAZARDS + PENDING PERK (1200 ticks): a Wave-class purist that lays \
                    a mine field and arms a duplicator/voucher inside the first 40 s. Second, \
                    cheap cover for Combat.tickHazards and for the Input.apply pending-perk \
                    branch, both of which otherwise ride on a single 55200-tick purist trace.",
    },
    TraceSpec {
        name: "short-seed-17620",
        seed: 17620,
        ticks: 1200,
        challenge: Challenge::None,
        exercises: "PROBE, AURA (1200 ticks): second, independent cover for the damage/poison \
                    aura (Blight Aura -> Combat.tickAura) — the cheapest seed found that arms \
                    an aura cadence at all. Also carries poison DoT and fire stacks.",
    },
    TraceSpec {
        name: "short-seed-8855",
        seed: 8855,
        ticks: 2100,
        challenge: Challenge::None,
        exercises: "PROBE, AURA + PENDING PERK (2100 ticks): buys Blight Aura, so it reaches \
                    Combat.tickAura with a live cadence; also arms a PendingPerk and stacks \
                    vulnerability. Paired with short-seed-17620 so no aura bug can hide.",
    },
    TraceSpec {
        name: "short-seed-13430",
        seed: 13430,
        ticks: 2100,
        challenge: Challenge::None,
        exercises: "PROBE, DEEP FREEZE (2100 ticks): frost stacks reach FROST_MAX_STACKS and \
                    convert into freeze_ticks, the Deep-Freeze payoff in Status.tick. Also \
                    arms a PendingPerk.",
    },
    TraceSpec {
        name: "short-seed-19260",
        seed: 19260,
        ticks: 2100,
        challenge: Challenge::None,
        exercises: "PROBE, DEEP FREEZE II (2100 ticks): second, independent cover for \
                    freeze_ticks and FROST_MAX_STACKS, on a different build from \
                    short-seed-13430. Also the cheapest poison-DoT cover in the corpus.",
    },
    TraceSpec {
        name: "short-seed-5660",
        seed: 5660,
        ticks: 2100,
        challenge: Challenge::None,
        exercises: "PROBE, REVIVE + MANA SHIELD + VULN PULSE II (2100 ticks): second, \
                    independent cover for all three behaviours short-seed-12283 carries. \
                    Those three rode on a single seed, so one balance nudge could have taken \
                    out the revive branch, the shield and Status.pulse together; this splits \
                    that risk across two unrelated builds.",
    },
    TraceSpec {
        name: "short-seed-12283",
        seed: 12283,
        ticks: 2400,
        challenge: Challenge::None,
        exercises: "PROBE, REVIVE + MANA SHIELD + VULN PULSE (2400 ticks): buys a revive (so \
                    economy.resolve_deaths has a revive branch to take), a mana shield, and a \
                    Vulnerability-Pulse aura driving Status.pulse; also poisons enemies, which \
                    gives poison DoT a second cover.",
    },
    TraceSpec {
        name: "short-seed-3",
        seed: 3,
        ticks: 6000,
        challenge: Challenge::None,
        exercises: "SHORT inner loop: 6 round boundaries, a lean build's weapon floor, the \
                    economy-snowball modifier window, and the first Clear activations",
    },
    TraceSpec {
        name: "mid-seed-12",
        seed: 12,
        ticks: 18000,
        challenge: Challenge::None,
        exercises: "MEDIUM: through the 10-min roster step (scale_step_1_tick), 3 difficulty \
                    ramp intervals, a balanced build mid-snowball, fire stacks + stuns + \
                    vulnerability stacks on a 160-enemy board",
    },
    TraceSpec {
        name: "full-seed-0",
        seed: 0,
        ticks: FULL_TICKS,
        challenge: Challenge::None,
        exercises: "FULL baseline, MINIONS: a summoner build (the only default-bot trace \
                    that fields minions, so it drives Combat.tick_minions without a purist); \
                    dies at death_tick 7114, so ~48k ticks of the post-death short-circuit \
                    follow, boss tick included",
    },
    TraceSpec {
        name: "full-seed-5",
        seed: 5,
        ticks: FULL_TICKS,
        challenge: Challenge::None,
        exercises: "FULL, MID-GAME DEATH at death_tick 16551 — ~38.6k ticks of the post-death \
                    short-circuit. The boss tick passes while dead, so it also proves waves \
                    and every other phase stay frozen after death. Reaches spikes retaliation \
                    and fire stacks on the way there",
    },
    TraceSpec {
        name: "full-seed-14",
        seed: 14,
        ticks: FULL_TICKS,
        challenge: Challenge::None,
        exercises: "FULL, LATE DEATH at death_tick 32956 — the longest-running default build \
                    that still dies before the boss, so it holds a 200-enemy board through 36 \
                    rounds of difficulty ramp (the widest wave/targeting workload of any \
                    default trace) and also drives the time-scaling ramps",
    },
    TraceSpec {
        name: "full-seed-67",
        seed: 67,
        ticks: FULL_TICKS,
        challenge: Challenge::None,
        exercises: "FULL, FAT ARSENAL: the widest default-bot weapon count (19 buys), so it is \
                    the trace that stresses per-tick weapon iteration, arsenal-synergy scaling \
                    and weapon_count_scaling; dies at death_tick 21020",
    },
    TraceSpec {
        name: "full-seed-189",
        seed: 189,
        ticks: FULL_TICKS,
        challenge: Challenge::None,
        exercises: "FULL, DEATH INSIDE THE RAMP TAIL at death_tick 44092, and the heaviest \
                    board of any default trace (290 enemies): the frost/stun corner of \
                    Status.tick under load, plus 136 Clear activations",
    },
    TraceSpec {
        name: "full-seed-272",
        seed: 272,
        ticks: FULL_TICKS,
        challenge: Challenge::None,
        exercises: "FULL, NEVER DIES on the default bot — no short-circuit anywhere, so all 21 \
                    phases run on all 55200 ticks, including the whole boss phase and the \
                    escort flood. The one default trace that both survives and spawns the \
                    boss; also drives the time-scaling ramps",
    },
    TraceSpec {
        name: "full-seed-311",
        seed: 311,
        ticks: FULL_TICKS,
        challenge: Challenge::None,
        exercises: "FULL, SECOND MINION COVER: minions plus spikes retaliation, time-scaling \
                    ramps and stuns on a starved economy (the lowest final_gold of any default \
                    trace); dies early at death_tick 7720",
    },
    TraceSpec {
        name: "purist3-seed-7",
        seed: 7,
        ticks: FULL_TICKS,
        challenge: Challenge::Purist(3),
        exercises: "FULL, WEAPON ABILITIES: an Area-class purist buys Shroom Doom, so this is \
                    the only trace that drives Combat.tickMinions across the boss phase. ~9.6k \
                    input events (the purist rerolls to fish), which also stresses \
                    Shop.generateOffers and the reroll economy",
    },
    TraceSpec {
        name: "purist4-seed-8",
        seed: 8,
        ticks: FULL_TICKS,
        challenge: Challenge::Purist(4),
        exercises: "FULL, HAZARDS + POISON: a Wave-class purist (Boom Bloom mine fields, \
                    Bloody Spikes stacking) — the only FULL-length cover for \
                    Combat.tickHazards and for poison DoT, plus the stacking-spikes round \
                    reset. purist4-seed-3045 is the cheap second cover for hazards",
    },
];

// ===================== running a trace =====================

/// Coverage actually observed while running a trace. Recorded by *reading* the
/// state after each tick; it never influences the sim. Reported in
/// `MANIFEST.json` so the Luau side can see what a trace really covers rather
/// than trusting the editorial note.
#[derive(Clone, Debug, Default)]
pub struct Coverage {
    pub buys: u32,
    pub rerolls: u32,
    pub clears: u32,
    pub weapon_buys: u32,
    pub modifier_buys: u32,
    pub weapons_bought: u32,
    pub death_tick: Option<u32>,
    pub reached_boss_phase: bool,
    pub boss_spawned: bool,
    pub max_enemies: usize,
    pub max_projectiles: usize,
    pub saw_hazard: bool,
    pub saw_minion: bool,
    pub saw_ramp: bool,
    pub saw_vuln_pulse: bool,
    pub saw_pending_perk: bool,
    pub saw_poison: bool,
    pub saw_frost: bool,
    pub saw_fire: bool,
    pub saw_stun: bool,
    pub saw_freeze: bool,
    pub saw_vuln_stacks: bool,
    pub saw_aura: bool,
    pub saw_revive: bool,
    pub saw_mana_shield: bool,
    pub saw_spikes: bool,
    pub final_gold: i64,
    pub total_damage_dealt: i64,
}

impl Coverage {
    /// The behaviour flags, for the seed scanner and the corpus-coverage test.
    /// Every one of these gates a phase or branch the Luau transcription has to
    /// get right; a flag no trace sets is a hole in the oracle.
    pub fn flags(&self) -> [(&'static str, bool); 16] {
        [
            ("boss", self.boss_spawned),
            ("death", self.death_tick.is_some()),
            ("hazard", self.saw_hazard),
            ("minion", self.saw_minion),
            ("ramp", self.saw_ramp),
            ("vulnpulse", self.saw_vuln_pulse),
            ("perk", self.saw_pending_perk),
            ("poison", self.saw_poison),
            ("frost", self.saw_frost),
            ("fire", self.saw_fire),
            ("stun", self.saw_stun),
            ("freeze", self.saw_freeze),
            ("vulnstack", self.saw_vuln_stacks),
            ("aura", self.saw_aura),
            ("revive", self.saw_revive),
            ("shield", self.saw_mana_shield),
        ]
    }
}

/// A completed trace run.
pub struct TraceRun {
    pub spec_name: &'static str,
    pub seed: u64,
    pub ticks: u32,
    /// `(tick, input)` for every tick whose input was **not** `Noop`.
    pub inputs: Vec<(u32, Input)>,
    /// `checksums[i]` is `checksum(state)` after tick `i` completed.
    pub checksums: Vec<u64>,
    pub coverage: Coverage,
}

/// Run one trace: drive `step()` with [`Bot`] for `spec.ticks` ticks, recording
/// the input log and the per-tick checksum.
///
/// The bot is asked for a decision on **every** tick (including after death,
/// where it returns `Noop`), so its internal cooldown advances exactly as it
/// would in a live match and the input log is reproducible from the seed alone.
pub fn run(spec: &TraceSpec) -> TraceRun {
    let mut st = ArenaState::new(spec.seed, PLAYER_ID);
    let mut bot = Bot::with_challenge(spec.challenge);
    let mut inputs = Vec::new();
    let mut checksums = Vec::with_capacity(spec.ticks as usize);
    let mut cov = Coverage::default();

    for tick in 0..spec.ticks {
        // The authoritative buy-filter is applied on top of the bot's decision,
        // exactly as the director/client does (`bot::Challenge::filter`), so a
        // constrained trace is the input log a real constrained run would emit.
        let inp = spec.challenge.filter(bot.decide(&st), &st);
        match inp {
            Input::Noop => {}
            other => {
                match other {
                    Input::BuyOffer { slot } => {
                        cov.buys += 1;
                        // Classify against the shop the buy lands on, read
                        // BEFORE `step` consumes it.
                        match st.shop.offers.get(slot as usize).map(|o| o.kind) {
                            Some(OfferKind::Weapon) => cov.weapon_buys += 1,
                            Some(OfferKind::Modifier) => cov.modifier_buys += 1,
                            None => {}
                        }
                    }
                    Input::Reroll => cov.rerolls += 1,
                    Input::Clear => cov.clears += 1,
                    Input::Noop => unreachable!(),
                }
                inputs.push((tick, other));
            }
        }
        step(&mut st, inp);
        checksums.push(checksum(&st));
        observe(&st, &mut cov);
    }

    cov.death_tick = st.death_tick;
    cov.reached_boss_phase = spec.ticks > content::BOSS_SPAWN_TICK;
    cov.weapons_bought = st.weapons_bought;
    cov.final_gold = st.economy.gold;
    cov.total_damage_dealt = st.total_damage_dealt;

    TraceRun {
        spec_name: spec.name,
        seed: spec.seed,
        ticks: spec.ticks,
        inputs,
        checksums,
        coverage: cov,
    }
}

/// Coverage-only run, used by `--scan` to pick corpus seeds. Same loop as
/// [`run`] minus the (memory-hungry) checksum vector.
pub fn scan(seed: u64, ticks: u32, challenge: Challenge) -> Coverage {
    let mut st = ArenaState::new(seed, PLAYER_ID);
    let mut bot = Bot::with_challenge(challenge);
    let mut cov = Coverage::default();
    for _ in 0..ticks {
        let inp = challenge.filter(bot.decide(&st), &st);
        if let Input::BuyOffer { slot } = inp {
            cov.buys += 1;
            match st.shop.offers.get(slot as usize).map(|o| o.kind) {
                Some(OfferKind::Weapon) => cov.weapon_buys += 1,
                Some(OfferKind::Modifier) => cov.modifier_buys += 1,
                None => {}
            }
        }
        step(&mut st, inp);
        observe(&st, &mut cov);
    }
    cov.death_tick = st.death_tick;
    cov.reached_boss_phase = ticks > content::BOSS_SPAWN_TICK;
    cov.weapons_bought = st.weapons_bought;
    cov.final_gold = st.economy.gold;
    cov.total_damage_dealt = st.total_damage_dealt;
    cov
}

/// Read-only coverage accounting. Takes `&ArenaState` by design: a `&mut` here
/// would be a licence to perturb the sim.
fn observe(st: &ArenaState, cov: &mut Coverage) {
    cov.max_enemies = cov.max_enemies.max(st.enemies.len());
    cov.max_projectiles = cov.max_projectiles.max(st.projectiles.len());
    cov.saw_hazard |= !st.hazards.is_empty();
    cov.saw_minion |= !st.minions.is_empty();
    cov.saw_ramp |= !st.ramps.is_empty();
    cov.saw_vuln_pulse |= !st.vuln_pulses.is_empty();
    cov.saw_pending_perk |= st.pending_perk.is_some();
    cov.boss_spawned |= st.enemies.iter().any(|e| e.def == content::BOSS);
    for e in &st.enemies {
        cov.saw_poison |= e.status.poison_ticks > 0;
        cov.saw_frost |= e.status.frost_stacks > 0;
        cov.saw_fire |= e.status.fire_stacks > 0;
        cov.saw_stun |= e.status.stun_ticks > 0;
        cov.saw_freeze |= e.status.freeze_ticks > 0;
        cov.saw_vuln_stacks |= e.status.vuln_stacks > 0;
    }
    cov.saw_aura |= st.tank.aura_cadence > 0;
    cov.saw_revive |= st.tank.revives > 0;
    cov.saw_mana_shield |= st.tank.mana_shield_max > 0;
    cov.saw_spikes |= st.tank.spikes_damage > 0 || st.tank.spikes_stacks > 0;
}

// ===================== the trace document =====================

/// 16-digit lowercase hex (C4).
fn hex_u(v: u64) -> Value {
    s(&format!("{v:016x}"))
}

/// The §S6 trace document for a completed run.
pub fn trace_document(run: &TraceRun) -> Value {
    obj(vec![
        ("schema_version", int(SCHEMA_VERSION)),
        ("seed", int(run.seed as i64)),
        ("content_hash", s(&format!("{:016x}", super::content_hash()))),
        ("encoding", s(ENCODING_NOTE)),
        (
            "inputs",
            Value::Arr(
                run.inputs
                    .iter()
                    .map(|(t, i)| {
                        obj(vec![("tick", int(*t as i64)), ("code", int(input_code(*i)))])
                    })
                    .collect(),
            ),
        ),
        ("checksums", Value::Arr(run.checksums.iter().map(|c| hex_u(*c)).collect())),
    ])
}

/// The one field this exporter adds beyond §S6's four. It documents the two
/// things a consumer cannot recover from the artifact — the `code` encoding and
/// the fact that the input log is *sparse* — and follows the convention already
/// set by the C4 vector files, which carry an `encoding` string for the same
/// reason.
pub const ENCODING_NOTE: &str =
    "checksums[i] = state_checksum AFTER tick i completes, as 16-digit lowercase hex u64. \
     inputs is SPARSE: it lists only ticks whose input was not Noop; every unlisted tick in \
     0..#checksums is Noop. code = kind*256 + slot, kind in {0:Noop, 1:BuyOffer, 2:Reroll, \
     3:Clear} (the ids::Input declaration order), slot is 0 except for BuyOffer. player_id is \
     0 for every trace. No floats appear anywhere in this file.";

/// Bytes written for one trace.
///
/// `pretty(1)` expands only the top-level object, so `checksums` stays a single
/// compact line. A per-line layout would add ~25 % to a corpus that is already
/// several megabytes, and the human entry point for a divergence is
/// `--verbose`, not reading the array.
pub fn trace_json(run: &TraceRun) -> String {
    trace_document(run).pretty(1)
}

/// Repo-relative path of a trace file.
pub fn trace_path(spec: &TraceSpec) -> String {
    format!("{TRACE_DIR}/{}.json", spec.name)
}

// ===================== the manifest =====================

fn yes(b: bool) -> Value {
    Value::Bool(b)
}

fn manifest_entry(spec: &TraceSpec, run: &TraceRun, bytes: usize) -> Value {
    let c = &run.coverage;
    obj(vec![
        ("name", s(spec.name)),
        ("file", s(&format!("{}.json", spec.name))),
        ("seed", int(spec.seed as i64)),
        ("ticks", int(spec.ticks as i64)),
        ("bytes", int(bytes as i64)),
        ("bot", s(challenge_name(spec.challenge))),
        ("input_events", int(run.inputs.len() as i64)),
        ("exercises", s(spec.exercises)),
        (
            "coverage",
            obj(vec![
                ("buys", int(c.buys as i64)),
                ("weapon_buys", int(c.weapon_buys as i64)),
                ("modifier_buys", int(c.modifier_buys as i64)),
                ("rerolls", int(c.rerolls as i64)),
                ("clears", int(c.clears as i64)),
                ("weapons_bought", int(c.weapons_bought as i64)),
                ("death_tick", match c.death_tick {
                    Some(t) => int(t as i64),
                    None => Value::Null,
                }),
                ("reached_boss_phase", yes(c.reached_boss_phase)),
                ("boss_spawned", yes(c.boss_spawned)),
                ("max_enemies", int(c.max_enemies as i64)),
                ("max_projectiles", int(c.max_projectiles as i64)),
                ("saw_hazards", yes(c.saw_hazard)),
                ("saw_minions", yes(c.saw_minion)),
                ("saw_ramps", yes(c.saw_ramp)),
                ("saw_vuln_pulses", yes(c.saw_vuln_pulse)),
                ("saw_pending_perk", yes(c.saw_pending_perk)),
                ("saw_poison", yes(c.saw_poison)),
                ("saw_frost", yes(c.saw_frost)),
                ("saw_fire", yes(c.saw_fire)),
                ("saw_stun", yes(c.saw_stun)),
                ("saw_freeze", yes(c.saw_freeze)),
                ("saw_vuln_stacks", yes(c.saw_vuln_stacks)),
                ("saw_damage_aura", yes(c.saw_aura)),
                ("saw_revive", yes(c.saw_revive)),
                ("saw_mana_shield", yes(c.saw_mana_shield)),
                ("saw_spikes", yes(c.saw_spikes)),
                ("final_gold", int(c.final_gold)),
                ("total_damage_dealt", int(c.total_damage_dealt)),
            ]),
        ),
    ])
}

/// Stable name for a bot challenge, for `MANIFEST.json` and `--list`.
pub fn challenge_name(c: Challenge) -> &'static str {
    match c {
        Challenge::None => "default",
        Challenge::Purist(0) => "purist-0",
        Challenge::Purist(1) => "purist-1",
        Challenge::Purist(2) => "purist-2",
        Challenge::Purist(3) => "purist-3",
        Challenge::Purist(4) => "purist-4",
        Challenge::Purist(_) => "purist-5",
        Challenge::NoEconomy => "no-economy",
        Challenge::JackOfAll => "jack-of-all",
        Challenge::EcoOnly => "eco-only",
    }
}

const MANIFEST_HOWTO: &str =
    "Regenerate: cargo run -p sim --bin export-traces. Verify freshness: \
     cargo run -p sim --bin export-traces -- --check. Localize a divergence: \
     cargo run -p sim --bin export-traces -- --verbose <seed> <tick> [--filter <substr>], \
     which dumps every checksummed field after that tick as \
     'path<TAB>kind<TAB>hex<TAB>decimal' — diff it against the same dump from the Luau side.";

const MANIFEST_TIMELINE: &str =
    "These traces run the RUST core's timeline (content.json timeline.steam): 900-tick rounds, \
     difficulty ramp every 5400 ticks, boss at tick 54000. The Roblox F1 rescale \
     (timeline.roblox: 600-tick rounds, boss at 9000) is NOT implemented in the Rust sim, so a \
     Luau port that adopts it cannot match this oracle. Transcribe against the Steam timeline \
     first, prove checksum parity, then rescale.";

/// `MANIFEST.json` — the corpus index the Luau side reads to pick a trace.
///
/// `coverage_union` is the corpus's real reach: a `false` there names a sim
/// behaviour **no trace exercises**, so a Luau bug in it would pass the gate.
/// A committed manifest with anything false is a hole to close (re-pick seeds
/// with `--scan`), not a detail to skim past; a test asserts it stays empty.
pub fn manifest_document(entries: Vec<Value>, union: Vec<(&'static str, bool)>) -> Value {
    let missing: Vec<Value> =
        union.iter().filter(|(_, v)| !*v).map(|(n, _)| s(n)).collect();
    obj(vec![
        ("schema_version", int(SCHEMA_VERSION)),
        ("content_hash", s(&format!("{:016x}", super::content_hash()))),
        ("trace_schema_version", int(SCHEMA_VERSION)),
        ("player_id", int(PLAYER_ID as i64)),
        ("tick_hz", int(crate::TICK_HZ as i64)),
        ("round_ticks", int(crate::ROUND_TICKS as i64)),
        ("boss_spawn_tick", int(content::BOSS_SPAWN_TICK as i64)),
        ("timeline", s(MANIFEST_TIMELINE)),
        ("encoding", s(ENCODING_NOTE)),
        ("howto", s(MANIFEST_HOWTO)),
        (
            "coverage_union",
            Value::Obj(union.iter().map(|(n, v)| (n.to_string(), yes(*v))).collect()),
        ),
        ("coverage_missing", Value::Arr(missing)),
        ("traces", Value::Arr(entries)),
    ])
}

/// Repo-relative path of the manifest.
pub fn manifest_path() -> String {
    format!("{TRACE_DIR}/MANIFEST.json")
}

/// Generate the whole corpus: `(repo-relative path, file body)` for every trace
/// plus `MANIFEST.json`, in a stable order. Pure — no I/O, no clock.
pub fn all_traces() -> Vec<(String, String)> {
    let mut out = Vec::with_capacity(TRACES.len() + 1);
    let mut entries = Vec::with_capacity(TRACES.len());
    let mut union: Vec<(&'static str, bool)> =
        Coverage::default().flags().iter().map(|(n, _)| (*n, false)).collect();
    for spec in TRACES {
        let run = run(spec);
        let body = trace_json(&run);
        for (i, (_, v)) in run.coverage.flags().iter().enumerate() {
            union[i].1 |= *v;
        }
        entries.push(manifest_entry(spec, &run, body.len()));
        out.push((trace_path(spec), body));
    }
    out.push((manifest_path(), manifest_document(entries, union).pretty(3)));
    out
}

// ===================== field dump (`--verbose`) =====================

/// How a checksummed field was fed to the accumulator. Display-only: every kind
/// ends up as `Checksum::write_u64` of the value's `u64` bit pattern, so the tag
/// exists to make a dump readable, not to change the fold.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    U32,
    U64,
    I64,
    /// A `Fixed` raw `i64` (Q47.16).
    Fixed,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::U32 => "u32",
            Kind::U64 => "u64",
            Kind::I64 => "i64",
            Kind::Fixed => "fixed",
        }
    }
}

/// One checksummed field: its dotted path, how it was written, and its bits.
#[derive(Clone, Debug)]
pub struct Field {
    pub path: String,
    pub kind: Kind,
    /// The exact `u64` handed to `Checksum::write_u64`.
    pub bits: u64,
}

impl Field {
    /// The value rendered in its natural type (signed for `i64`/`fixed`).
    pub fn decimal(&self) -> String {
        match self.kind {
            Kind::U32 | Kind::U64 => format!("{}", self.bits),
            Kind::I64 | Kind::Fixed => format!("{}", self.bits as i64),
        }
    }
    /// `path<TAB>kind<TAB>hex<TAB>decimal` — the diffable line format.
    pub fn line(&self) -> String {
        format!("{}\t{}\t{:016x}\t{}", self.path, self.kind.as_str(), self.bits, self.decimal())
    }
}

/// Accumulator that records what it folds. Mirrors `Checksum`'s API so [`fields`]
/// can be a transcription of `lib.rs::checksum` with names attached, rather than
/// a second, independently-drifting implementation of it.
struct Dump {
    out: Vec<Field>,
    prefix: String,
}

impl Dump {
    fn new() -> Dump {
        Dump { out: Vec::new(), prefix: String::new() }
    }
    fn path(&self, name: &str) -> String {
        if self.prefix.is_empty() {
            name.to_string()
        } else if name.is_empty() {
            self.prefix.clone()
        } else {
            format!("{}.{}", self.prefix, name)
        }
    }
    fn u32(&mut self, name: &str, v: u32) {
        let path = self.path(name);
        self.out.push(Field { path, kind: Kind::U32, bits: v as u64 });
    }
    fn u64(&mut self, name: &str, v: u64) {
        let path = self.path(name);
        self.out.push(Field { path, kind: Kind::U64, bits: v });
    }
    fn i64(&mut self, name: &str, v: i64) {
        let path = self.path(name);
        self.out.push(Field { path, kind: Kind::I64, bits: v as u64 });
    }
    fn fixed(&mut self, name: &str, v: determinism::Fixed) {
        let path = self.path(name);
        self.out.push(Field { path, kind: Kind::Fixed, bits: v.raw() as u64 });
    }
    /// Run `f` with `seg` pushed onto the dotted path.
    fn scope(&mut self, seg: &str, f: impl FnOnce(&mut Dump)) {
        let nested = self.path(seg);
        let saved = core::mem::replace(&mut self.prefix, nested);
        f(self);
        self.prefix = saved;
    }
    /// Emit the `(tag, a, b, c)` payload of a `words()`-encoded enum, matching
    /// the four writes `checksum()` performs for it.
    fn words(&mut self, name: &str, w: (u8, i64, i64, i64)) {
        self.scope(name, |d| {
            d.u32("tag", w.0 as u32);
            d.i64("a", w.1);
            d.i64("b", w.2);
            d.i64("c", w.3);
        });
    }
}

/// Every field `lib.rs::checksum` folds, in the exact order it folds them.
///
/// **This must stay a line-for-line mirror of `checksum()`.** It is not merely a
/// debugging convenience: `checksum_of_fields(&fields(s)) == checksum(s)` is
/// asserted by the test suite over live states, so if `checksum()` gains, loses
/// or reorders a field and this does not, the tests fail.
pub fn fields(st: &ArenaState) -> Vec<Field> {
    let mut d = Dump::new();
    d.u32("tick", st.tick);
    d.u32("round", st.round);
    d.u64("master_seed", st.master_seed);
    d.u32("player_id", st.player_id);

    d.scope("tank", |d| {
        d.i64("hp", st.tank.hp);
        d.i64("max_hp", st.tank.max_hp);
        d.fixed("pos.x", st.tank.pos.x);
        d.fixed("pos.y", st.tank.pos.y);
        d.u32("clear_cooldown_end", st.tank.clear_cooldown_end);
        d.i64("armor", st.tank.armor);
        d.u32("dodge_num", st.tank.dodge_num);
        d.u32("dodge_den", st.tank.dodge_den);
        d.i64("mana_shield", st.tank.mana_shield);
        d.i64("mana_shield_max", st.tank.mana_shield_max);
        d.i64("mana_regen_per_tick", st.tank.mana_regen_per_tick);
        d.i64("hp_regen_per_tick", st.tank.hp_regen_per_tick);
        d.i64("spikes_damage", st.tank.spikes_damage);
        d.fixed("spikes_mult", st.tank.spikes_mult);
        d.i64("heal_on_kill", st.tank.heal_on_kill);
        d.i64("heal_on_poison", st.tank.heal_on_poison);
        d.fixed("healing_mult", st.tank.healing_mult);
        d.fixed("missing_hp_heal_pct", st.tank.missing_hp_heal_pct);
        d.u32("revives", st.tank.revives);
        d.i64("revive_bonus_hp", st.tank.revive_bonus_hp);
        d.i64("shieldbreak_stun_range", st.tank.shieldbreak_stun_range);
        d.u32("shieldbreak_stun_ticks", st.tank.shieldbreak_stun_ticks);
        d.i64("spikes_poison_dps", st.tank.spikes_poison_dps);
        d.u32("spikes_poison_ticks", st.tank.spikes_poison_ticks);
        d.i64("spikes_stack_per", st.tank.spikes_stack_per);
        d.u32("spikes_stacks", st.tank.spikes_stacks);
        d.u32("spikes_stacks_max", st.tank.spikes_stacks_max);
        d.i64("aura_range", st.tank.aura_range);
        d.u32("aura_cadence", st.tank.aura_cadence);
        d.i64("aura_damage", st.tank.aura_damage);
        d.i64("aura_poison_dps", st.tank.aura_poison_dps);
        d.u32("aura_poison_ticks", st.tank.aura_poison_ticks);
        d.u32("aura_tick", st.tank.aura_tick);
    });

    d.scope("economy", |d| {
        d.i64("gold", st.economy.gold);
        d.i64("income_per_tick", st.economy.income_per_tick);
        d.fixed("income_mult", st.economy.income_mult);
        d.fixed("income_regen_pct", st.economy.income_regen_pct);
        d.fixed("bounty_mult", st.economy.bounty_mult);
        d.i64("bounty_proc_chance_pct", st.economy.bounty_proc_chance_pct);
        d.fixed("bounty_proc_bonus", st.economy.bounty_proc_bonus);
        d.fixed("gold_per_damage", st.economy.gold_per_damage);
        d.fixed("income_shield_pct", st.economy.income_shield_pct);
        d.u32("rerolls_remaining", st.economy.rerolls_remaining);
        d.i64("reroll_cost", st.economy.reroll_cost);
    });

    d.u32("next_entity_id", st.next_entity_id);
    d.u32("dead", st.dead as u32);
    d.u32("death_tick", st.death_tick.unwrap_or(u32::MAX));
    d.u32("pending_kills.len", st.pending_kills.len() as u32);
    for (i, k) in st.pending_kills.iter().enumerate() {
        d.u32(&format!("pending_kills[{i}]"), *k as u32);
    }
    d.i64("total_damage_dealt", st.total_damage_dealt);
    d.i64("total_gold_earned", st.total_gold_earned);

    d.u64("rng_spawn", st.rng_spawn.state());
    d.u64("rng_targeting", st.rng_targeting.state());
    d.u64("rng_shop", st.rng_shop.state());
    d.u64("rng_reroll", st.rng_reroll.state());
    d.u64("rng_proc", st.rng_proc.state());

    // Weapons, id-sorted (the index below is the SORTED position, matching the
    // order `checksum()` walks — not the position in `s.weapons`).
    let mut w: Vec<&crate::WeaponInstance> = st.weapons.iter().collect();
    w.sort_by_key(|x| x.instance_id);
    d.u32("weapons.len", w.len() as u32);
    for (i, x) in w.iter().enumerate() {
        d.scope(&format!("weapons[{i}]"), |d| {
            d.u32("instance_id", x.instance_id.0);
            d.u32("def", x.def as u32);
            d.u32("next_fire_tick", x.next_fire_tick);
        });
    }

    d.scope("modifiers", |d| {
        d.fixed("add_global", st.modifiers.add_global);
        for (i, a) in st.modifiers.add_by_type.iter().enumerate() {
            d.fixed(&format!("add_by_type[{i}]"), *a);
        }
        for (i, a) in st.modifiers.add_by_scope.iter().enumerate() {
            d.fixed(&format!("add_by_scope[{i}]"), *a);
        }
        d.fixed("mul_global", st.modifiers.mul_global);
        d.fixed("attack_speed", st.modifiers.attack_speed);
        d.fixed("vs_stunned", st.modifiers.vs_stunned);
        d.fixed("vs_poisoned", st.modifiers.vs_poisoned);
        d.fixed("poison_dmg_mult", st.modifiers.poison_dmg_mult);
        d.fixed("stun_dur_mult", st.modifiers.stun_dur_mult);
        d.u32("weapon_count_scaling.len", st.modifiers.weapon_count_scaling.len() as u32);
        for (i, r) in st.modifiers.weapon_count_scaling.iter().enumerate() {
            d.scope(&format!("weapon_count_scaling[{i}]"), |d| {
                d.u32("weapon_def", r.weapon_def as u32);
                d.u32("dmg_type", r.dmg_type as u32);
                d.fixed("per", r.per);
            });
        }
    });

    d.u32("ramps.len", st.ramps.len() as u32);
    for (i, r) in st.ramps.iter().enumerate() {
        d.scope(&format!("ramps[{i}]"), |d| {
            d.words("effect", r.effect.words());
            d.u32("interval_ticks", r.interval_ticks);
            d.u32("next_apply", r.next_apply);
        });
    }

    d.u32("vuln_pulses.len", st.vuln_pulses.len() as u32);
    for (i, p) in st.vuln_pulses.iter().enumerate() {
        d.scope(&format!("vuln_pulses[{i}]"), |d| {
            d.u32("magnitude", p.magnitude as u32);
            d.i64("range", p.range);
            d.u32("interval_ticks", p.interval_ticks);
            d.u32("next_tick", p.next_tick);
        });
    }

    match st.pending_perk {
        Some(p) => d.scope("pending_perk", |d| {
            d.u32("present", 1);
            d.u32("rarity", p.rarity as u32);
            d.u32("extra_copies", p.extra_copies);
            d.u32("free", p.free as u32);
        }),
        None => d.u32("pending_perk.present", 0),
    }

    d.u32("shop.shop_seq", st.shop.shop_seq);
    d.u32("shop.offers.len", st.shop.offers.len() as u32);
    for (i, o) in st.shop.offers.iter().enumerate() {
        d.scope(&format!("shop.offers[{i}]"), |d| {
            d.u32("kind", match o.kind {
                OfferKind::Weapon => 0,
                OfferKind::Modifier => 1,
            });
            d.u32("def", o.def as u32);
            d.i64("cost", o.cost);
        });
    }

    let mut e: Vec<&crate::Enemy> = st.enemies.iter().collect();
    e.sort_by_key(|x| x.id);
    d.u32("enemies.len", e.len() as u32);
    for (i, x) in e.iter().enumerate() {
        d.scope(&format!("enemies[{i}]"), |d| {
            d.u32("id", x.id.0);
            d.u32("def", x.def as u32);
            d.i64("hp", x.hp);
            d.fixed("pos.x", x.pos.x);
            d.fixed("pos.y", x.pos.y);
            d.i64("status.poison_dps", x.status.poison_dps);
            d.u32("status.poison_ticks", x.status.poison_ticks);
            d.u32("status.frost_stacks", x.status.frost_stacks as u32);
            d.u32("status.frost_ticks", x.status.frost_ticks);
            d.u32("status.fire_stacks", x.status.fire_stacks as u32);
            d.u32("status.vuln_stacks", x.status.vuln_stacks as u32);
            d.u32("status.stun_ticks", x.status.stun_ticks);
            d.u32("status.freeze_ticks", x.status.freeze_ticks);
        });
    }

    let mut p: Vec<&crate::Projectile> = st.projectiles.iter().collect();
    p.sort_by_key(|x| x.id);
    d.u32("projectiles.len", p.len() as u32);
    for (i, x) in p.iter().enumerate() {
        d.scope(&format!("projectiles[{i}]"), |d| {
            d.u32("id", x.id.0);
            d.fixed("pos.x", x.pos.x);
            d.fixed("pos.y", x.pos.y);
            d.u32("target", x.target.0);
            d.i64("damage", x.damage);
            d.u32("damage_type", x.damage_type as u32);
            d.fixed("splash_radius", x.splash_radius);
            d.fixed("speed", x.speed);
            d.i64("on_hit.poison_dps", x.on_hit.poison_dps);
            d.u32("on_hit.poison_ticks", x.on_hit.poison_ticks);
            d.u32("on_hit.frost_stacks", x.on_hit.frost_stacks as u32);
            d.u32("on_hit.fire_stacks", x.on_hit.fire_stacks as u32);
            d.u32("on_hit.stun_ticks", x.on_hit.stun_ticks);
            d.words("ability", x.ability.words());
        });
    }

    let mut hz: Vec<&crate::Hazard> = st.hazards.iter().collect();
    hz.sort_by_key(|x| x.id);
    d.u32("hazards.len", hz.len() as u32);
    for (i, x) in hz.iter().enumerate() {
        d.scope(&format!("hazards[{i}]"), |d| {
            d.u32("id", x.id.0);
            d.fixed("pos.x", x.pos.x);
            d.fixed("pos.y", x.pos.y);
            d.i64("dmg", x.dmg);
            d.u32("damage_type", x.damage_type as u32);
            d.i64("radius", x.radius);
            d.u32("ticks_left", x.ticks_left);
        });
    }

    let mut mn: Vec<&crate::Minion> = st.minions.iter().collect();
    mn.sort_by_key(|x| x.id);
    d.u32("minions.len", mn.len() as u32);
    for (i, x) in mn.iter().enumerate() {
        d.scope(&format!("minions[{i}]"), |d| {
            d.u32("id", x.id.0);
            d.fixed("pos.x", x.pos.x);
            d.fixed("pos.y", x.pos.y);
            d.u32("kind", x.kind as u32);
            d.i64("hp", x.hp);
            d.i64("damage", x.damage);
            d.u32("damage_type", x.damage_type as u32);
            d.u32("next_attack_tick", x.next_attack_tick);
            d.u32("expire_tick", x.expire_tick);
        });
    }

    d.out
}

/// Re-fold a field dump into the `u64` it came from. Equal to `checksum(s)` for
/// `fields(s)` — that equality is the drift guard on the dump.
pub fn checksum_of_fields(f: &[Field]) -> u64 {
    let mut c = Checksum::new();
    for x in f {
        c.write_u64(x.bits);
    }
    c.finish()
}

/// Replay `seed` for `ticks` ticks by re-running the bot, returning the state
/// **after** tick `ticks - 1` completed, plus the input applied on that last
/// tick and the resulting checksum. `ticks == 0` yields the pre-tick-0 state.
///
/// This is exactly the loop [`run`] uses — including the challenge, which must
/// be passed or a constrained trace replays as a different match entirely.
/// Prefer [`replay_inputs`] when a committed trace exists: replaying the
/// recorded log is what the Luau side does, and it cannot drift from the file.
pub fn replay_to(seed: u64, ticks: u32, challenge: Challenge) -> (ArenaState, Input, u64) {
    let mut st = ArenaState::new(seed, PLAYER_ID);
    let mut bot = Bot::with_challenge(challenge);
    let mut last = Input::Noop;
    for _ in 0..ticks {
        last = challenge.filter(bot.decide(&st), &st);
        step(&mut st, last);
    }
    let cs = checksum(&st);
    (st, last, cs)
}

/// Replay `seed` for `ticks` ticks from an explicit sparse input log — the same
/// thing a trace consumer does. `log` maps tick → input; every unlisted tick is
/// `Noop`. No bot is involved, so this works for any trace regardless of how its
/// inputs were produced.
pub fn replay_inputs(
    seed: u64,
    ticks: u32,
    log: &std::collections::BTreeMap<u32, Input>,
) -> (ArenaState, Input, u64) {
    let mut st = ArenaState::new(seed, PLAYER_ID);
    let mut last = Input::Noop;
    for tick in 0..ticks {
        last = log.get(&tick).copied().unwrap_or(Input::Noop);
        step(&mut st, last);
    }
    let cs = checksum(&st);
    (st, last, cs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_codes_round_trip() {
        let mut all = vec![Input::Noop, Input::Reroll, Input::Clear];
        for slot in 0..=255u8 {
            all.push(Input::BuyOffer { slot });
        }
        let mut seen = std::collections::BTreeSet::new();
        for i in all {
            let c = input_code(i);
            assert!(seen.insert(c), "input code {c} is not unique");
            assert_eq!(input_from_code(c), Some(i), "round trip failed for {i:?}");
        }
        // Codes with no variant must decode to None, not to a silent Noop.
        assert_eq!(input_from_code(1), None);
        assert_eq!(input_from_code(2 * 256 + 1), None);
        assert_eq!(input_from_code(4 * 256), None);
    }

    /// The drift guard: the `--verbose` dump must re-fold into `checksum()`
    /// exactly, on states rich enough to reach every branch of both functions.
    #[test]
    fn field_dump_refolds_to_the_real_checksum() {
        for seed in [0u64, 6, 14] {
            let mut st = ArenaState::new(seed, PLAYER_ID);
            let mut bot = Bot::default();
            for tick in 0..3000u32 {
                let inp = bot.decide(&st);
                step(&mut st, inp);
                if tick % 7 == 0 || tick < 40 {
                    let f = fields(&st);
                    assert_eq!(
                        checksum_of_fields(&f),
                        checksum(&st),
                        "field dump diverged from checksum() at seed {seed} tick {tick}"
                    );
                }
            }
        }
    }

    /// A dump is useless if two different fields share a path.
    /// Replaying a recorded input log must land on the same state as re-running
    /// the bot that produced it — otherwise a committed trace and the
    /// `--verbose` dump would describe different matches.
    #[test]
    fn replaying_a_recorded_log_matches_rerunning_the_bot() {
        for spec in [
            &TraceSpec {
                name: "t",
                seed: 9,
                ticks: 2000,
                challenge: Challenge::None,
                exercises: "",
            },
            &TraceSpec {
                name: "t",
                seed: 7,
                ticks: 2000,
                challenge: Challenge::Purist(3),
                exercises: "",
            },
        ] {
            let r = run(spec);
            let log: std::collections::BTreeMap<u32, Input> = r.inputs.iter().copied().collect();
            let (_, _, cs) = replay_inputs(spec.seed, spec.ticks, &log);
            assert_eq!(cs, *r.checksums.last().unwrap(), "seed {}", spec.seed);
        }
    }

    #[test]
    fn field_paths_are_unique() {
        let (st, _, _) = replay_to(9, 1500, Challenge::None);
        let f = fields(&st);
        assert!(f.len() > 200, "state was too thin to be a meaningful check");
        let mut seen = std::collections::BTreeSet::new();
        for x in &f {
            assert!(seen.insert(x.path.clone()), "duplicate field path {:?}", x.path);
        }
    }

    #[test]
    fn corpus_specs_are_well_formed() {
        let mut names = std::collections::BTreeSet::new();
        let mut seeds = std::collections::BTreeSet::new();
        for t in TRACES {
            assert!(names.insert(t.name), "duplicate trace name {}", t.name);
            assert!(seeds.insert(t.seed), "duplicate seed {}", t.seed);
            assert!(t.ticks > 0);
        }
        assert!(TRACES.len() >= 8, "the corpus must cover at least 8 seeds");
        assert!(
            TRACES.iter().any(|t| t.ticks > content::BOSS_SPAWN_TICK),
            "at least one trace must reach the boss phase"
        );
    }
}
