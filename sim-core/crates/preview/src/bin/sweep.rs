//! Balance measurement harness — a seed sweep over the real `sim::step` +
//! `sim::bot::Bot`, reported against the `docs/11-fun.md §11.2` target bands.
//!
//! Each seed is played to a REAL CONCLUSION: the tank dies, or the boss (`The
//! Hippocrate`, spawned at `content::BOSS_SPAWN_TICK` = 30 min) is killed, or the
//! hard tick cap is reached. The default cap sits `BOSS_FIGHT_ROOM` ticks PAST the
//! boss spawn so the boss fight — the actual win condition — is inside every run.
//! (The previous version of this file capped at 20 min and therefore never once
//! observed the boss; every "win rate" it printed was really "survived two thirds
//! of a run". Do not reintroduce a cap below `BOSS_SPAWN_TICK`.)
//!
//! What it reports, and why each one exists (`docs/11-fun.md`):
//!   - WIN RATE = boss killed / seeds. The win condition is killing the boss, not
//!     outliving a clock. "Survived to the cap with the boss alive" is reported as
//!     its own UNRESOLVED bucket, never as a win.
//!   - DEATH-TIME DISTRIBUTION in deciles of the 30-minute designed run (3:00
//!     each) plus a boss-phase bucket, so a difficulty *spike* or a *plateau* is
//!     visible at a glance instead of collapsing into one win/lose number.
//!   - BOSS OUTCOME separately: reached / not reached, killed / survived-but-not-
//!     killed, time-to-kill, and the boss HP left on runs that failed it.
//!   - PEAK AND FINAL MAGNITUDES (max HP, damage, gold), so the runaway-numbers
//!     problem is a tracked number instead of an anecdote.
//!   - BUILD SHAPE: distinct weapons vs total copies and the top weapon's share,
//!     so `Magic Bolt ×14` is visibly distinguishable from a real five-weapon build.
//!
//! Pure observation of the public arena state — it never bends the sim, and the
//! bot reads only the same surface a client sees. Deterministic: a run is a pure
//! function of its seed, results are re-sorted into seed order before anything is
//! aggregated, and no wall-clock or platform RNG touches anything reported.
//! `--threads` only changes how fast the identical numbers arrive.
//!
//!   sweep                            # seeds 0..80, cap = boss + 5 min
//!   sweep --seeds 200 --rows         # more seeds, print every run
//!   sweep --json                     # machine-readable (diff tuning runs)
//!   sweep --threads 1                # force sequential (determinism check)

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use sim::bot::{Archetype, Bot, Challenge};
use sim::{content, step, ArenaState, Input, TICK_HZ};

/// Ticks of headroom past the boss spawn. The boss has 33M HP and only `Clear`
/// (3M damage, 300-tick cooldown) hurts it, so even a perfect kill needs ~11
/// Clears ≈ 3300 ticks; 5 minutes leaves room for an imperfect but real fight to
/// resolve one way or the other.
const BOSS_FIGHT_ROOM: u32 = 5 * 60 * TICK_HZ;
/// Default hard cap: far enough past `BOSS_SPAWN_TICK` that the boss fight always
/// fits inside the run.
const DEFAULT_CAP_TICKS: u32 = content::BOSS_SPAWN_TICK + BOSS_FIGHT_ROOM;
const DEFAULT_SEEDS: u64 = 80;
/// The DESIGNED run length, used as the denominator for the death-time deciles and
/// the "first quarter" target. Pinned to the boss spawn (NOT the cap) so the
/// distribution stays comparable across `--cap` changes.
const RUN_REF_TICKS: u32 = content::BOSS_SPAWN_TICK;
const DECILES: usize = 10;

// ---- §11.2 target bands (docs/11-fun.md) ------------------------------------
const T_WIN_LO: f64 = 25.0;
const T_WIN_HI: f64 = 40.0;
const T_Q1_LO: f64 = 10.0;
const T_Q1_HI: f64 = 20.0;
const T_PEAK_HP_MAX: i64 = 500_000;
const T_DISTINCT_WEAPONS_MIN: f64 = 5.0;
/// PROXY for §11.2's "no single spike", which is stated qualitatively. This
/// harness makes it checkable by declaring a spike to be any single decile holding
/// more than this share of all deaths (a perfectly flat curve sits at 10%).
const T_MAX_DECILE_SHARE: f64 = 25.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    /// The tank died.
    Died,
    /// The boss was killed — the actual win condition.
    BossKilled,
    /// Cap reached alive with the boss still up (or never spawned).
    Unresolved,
}

impl Outcome {
    fn label(self) -> &'static str {
        match self {
            Outcome::Died => "DIED",
            Outcome::BossKilled => "WIN(boss)",
            Outcome::Unresolved => "UNRESOLVED",
        }
    }
    fn json(self) -> &'static str {
        match self {
            Outcome::Died => "died",
            Outcome::BossKilled => "boss_killed",
            Outcome::Unresolved => "unresolved",
        }
    }
}

struct Run {
    seed: u64,
    archetype: Archetype,
    outcome: Outcome,
    /// Tick the run ended (death tick, boss-kill tick, or the cap).
    end_tick: u32,
    end_secs: u32,

    // ---- boss ----
    /// The boss actually spawned and was observed alive.
    boss_reached: bool,
    /// Tick the boss died (set exactly when `outcome == BossKilled`).
    boss_kill_tick: Option<u32>,
    /// Boss HP remaining at the end of the run, when it was reached but not killed.
    boss_hp_left: Option<i64>,
    /// `Clear` activations that actually fired (off cooldown): total, and the
    /// subset inside the boss phase.
    clears: u32,
    clears_boss_phase: u32,

    // ---- magnitudes ----
    peak_max_hp: i64,
    final_max_hp: i64,
    final_armor: i64,
    /// Largest player damage dealt within a single tick (a burst magnitude).
    peak_tick_damage: i64,
    total_damage: i64,
    /// Largest gold balance ever held.
    peak_gold: i64,
    total_gold_earned: i64,

    // ---- build shape ----
    /// Distinct weapon DEFS owned at the end.
    distinct_weapons: usize,
    /// Total weapon INSTANCES owned at the end (copies included).
    total_weapons: usize,
    /// Copies of the single most-owned weapon, and its name.
    top_weapon_count: usize,
    top_weapon: &'static str,
}

impl Run {
    /// Share of the arsenal held by the single most-copied weapon, in percent.
    /// 100% means the build is one weapon stacked; a genuine five-weapon build
    /// sits far lower.
    fn top_share_pct(&self) -> f64 {
        if self.total_weapons == 0 {
            0.0
        } else {
            100.0 * self.top_weapon_count as f64 / self.total_weapons as f64
        }
    }
}

fn run_seed(seed: u64, cap: u32, challenge: Challenge) -> Run {
    let mut s = ArenaState::new(seed, 0);
    let mut bot = Bot::with_challenge(challenge);

    let mut peak_max_hp = s.tank.max_hp;
    let mut peak_gold = s.economy.gold;
    let mut peak_tick_damage = 0i64;
    let mut clears = 0u32;
    let mut clears_boss_phase = 0u32;
    let mut boss_reached = false;
    let mut boss_kill_tick: Option<u32> = None;
    let mut boss_hp_left: Option<i64> = None;

    while s.tick < cap && !s.dead && boss_kill_tick.is_none() {
        // Apply the challenge's authoritative buy-filter too, so a slot purchase
        // can never violate the playstyle even if the shop shifted — keeps the
        // measured run a faithful pure-eco / constrained run.
        let action = challenge.filter(bot.decide(&s), &s);
        // A `Clear` counts only when it actually fires (off cooldown) — the same
        // condition `input::apply` uses, read from public state before the step.
        if matches!(action, Input::Clear) && s.tick >= s.tank.clear_cooldown_end {
            clears += 1;
            if s.tick >= content::BOSS_SPAWN_TICK {
                clears_boss_phase += 1;
            }
        }
        let dmg_before = s.total_damage_dealt;
        step(&mut s, action);

        peak_max_hp = peak_max_hp.max(s.tank.max_hp);
        peak_gold = peak_gold.max(s.economy.gold);
        peak_tick_damage = peak_tick_damage.max(s.total_damage_dealt - dmg_before);

        // Boss tracking. The boss is removed from `enemies` when it dies, and the
        // sim freezes on tank death, so "was present, now absent, tank alive" is
        // exactly a boss kill.
        if s.tick >= content::BOSS_SPAWN_TICK {
            match s.enemies.iter().find(|e| e.def == content::BOSS) {
                Some(b) => {
                    boss_reached = true;
                    boss_hp_left = Some(b.hp);
                }
                None if boss_reached && !s.dead => {
                    boss_kill_tick = Some(s.tick);
                    boss_hp_left = None;
                }
                None => {}
            }
        }
    }

    let outcome = if s.dead {
        Outcome::Died
    } else if boss_kill_tick.is_some() {
        Outcome::BossKilled
    } else {
        Outcome::Unresolved
    };

    // ---- build shape: distinct defs, copies, top weapon ----
    let mut defs: Vec<u16> = s.weapons.iter().map(|w| w.def).collect();
    defs.sort_unstable();
    let mut distinct = 0usize;
    let mut top_count = 0usize;
    let mut top_def: Option<u16> = None;
    let mut i = 0usize;
    while i < defs.len() {
        let d = defs[i];
        let mut j = i;
        while j < defs.len() && defs[j] == d {
            j += 1;
        }
        distinct += 1;
        if j - i > top_count {
            top_count = j - i;
            top_def = Some(d);
        }
        i = j;
    }

    Run {
        seed,
        // The default bot's per-match archetype is a pure function of the seed.
        archetype: Archetype::for_seed(seed),
        outcome,
        end_tick: s.tick,
        end_secs: s.tick / TICK_HZ,
        boss_reached,
        boss_kill_tick,
        boss_hp_left,
        clears,
        clears_boss_phase,
        peak_max_hp,
        final_max_hp: s.tank.max_hp,
        final_armor: s.tank.armor,
        peak_tick_damage,
        total_damage: s.total_damage_dealt,
        peak_gold,
        total_gold_earned: s.total_gold_earned,
        distinct_weapons: distinct,
        total_weapons: s.weapons.len(),
        top_weapon_count: top_count,
        top_weapon: top_def.map_or("-", |d| content::WEAPONS[d as usize].name),
    }
}

/// Play every seed, optionally across `threads` workers. Results are re-sorted
/// into seed order before anything is aggregated, so the reported numbers are
/// identical for any thread count (each run is an independent pure function of
/// its seed and shares no state).
fn run_all(seed_start: u64, seeds: u64, cap: u32, challenge: Challenge, threads: usize) -> Vec<Run> {
    let list: Vec<u64> = (seed_start..seed_start + seeds).collect();
    if threads <= 1 || list.len() <= 1 {
        return list.into_iter().map(|s| run_seed(s, cap, challenge)).collect();
    }
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<(usize, Run)>> = Mutex::new(Vec::with_capacity(list.len()));
    std::thread::scope(|sc| {
        for _ in 0..threads.min(list.len()) {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= list.len() {
                    break;
                }
                let r = run_seed(list[i], cap, challenge);
                out.lock().unwrap().push((i, r));
            });
        }
    });
    let mut v = out.into_inner().unwrap();
    v.sort_by_key(|(i, _)| *i);
    v.into_iter().map(|(_, r)| r).collect()
}

// ---- small deterministic stats helpers (integer nearest-rank percentiles) ----

fn sorted_i64(vals: impl Iterator<Item = i64>) -> Vec<i64> {
    let mut v: Vec<i64> = vals.collect();
    v.sort_unstable();
    v
}

/// Nearest-rank percentile of a pre-sorted slice (`p` in 0..=100). Integer index
/// arithmetic only, no interpolation — exactly reproducible.
fn pctl(sorted: &[i64], p: usize) -> i64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = (p * (sorted.len() - 1) + 50) / 100;
    sorted[idx.min(sorted.len() - 1)]
}

fn mean_i64(v: &[i64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64
}

fn mean_f64(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().sum::<f64>() / v.len() as f64
}

/// `IN` / `OUT` verdict for a metric against its §11.2 band.
fn verdict(v: f64, lo: f64, hi: f64) -> &'static str {
    if v < lo {
        "OUT(low)"
    } else if v > hi {
        "OUT(high)"
    } else {
        "IN"
    }
}

fn fmt_secs(t: u32) -> String {
    format!("{}:{:02}", t / 60, t % 60)
}

/// Compact magnitude for the human table (12.30M / 244.8k / 950). Past 1e15 the
/// suffixes stop being meaningful, so it falls back to exponent form — a runaway
/// stat should LOOK wrong rather than read as a tidy number.
fn mag(v: i64) -> String {
    let a = v.unsigned_abs();
    if a >= 1_000_000_000_000_000 {
        format!("{:.2e}", v as f64)
    } else if a >= 1_000_000_000_000 {
        format!("{:.2}T", v as f64 / 1e12)
    } else if a >= 1_000_000_000 {
        format!("{:.2}B", v as f64 / 1e9)
    } else if a >= 1_000_000 {
        format!("{:.2}M", v as f64 / 1e6)
    } else if a >= 1_000 {
        format!("{:.1}k", v as f64 / 1e3)
    } else {
        format!("{}", v)
    }
}

fn arch_name(a: Archetype) -> &'static str {
    match a {
        Archetype::GlassCannon => "glass-cannon",
        Archetype::Tanky => "tanky",
        Archetype::EcoPivot => "eco-pivot",
        Archetype::Balanced => "balanced",
    }
}

fn challenge_name(c: Challenge) -> String {
    match c {
        Challenge::None => "none".into(),
        Challenge::Purist(k) => format!("purist({})", k),
        Challenge::NoEconomy => "no-economy".into(),
        Challenge::JackOfAll => "jack-of-all".into(),
        Challenge::EcoOnly => "eco-only".into(),
    }
}

// ---- aggregation ------------------------------------------------------------

struct Agg {
    n: usize,
    died: usize,
    boss_killed: usize,
    unresolved: usize,
    /// Deaths per decile of the 30-min designed run, plus `[DECILES]` = boss phase.
    death_decile: [usize; DECILES + 1],
    deaths_total: usize,
    deaths_first_quarter: usize,
    /// Death ticks, sorted.
    death_ticks: Vec<i64>,
    /// End time in seconds for every run (deaths and survivors alike), sorted.
    end_secs: Vec<i64>,
    boss_reached: usize,
    ttk_secs: Vec<i64>,
    boss_hp_left: Vec<i64>,
    clears_boss: Vec<i64>,
}

fn aggregate(runs: &[Run]) -> Agg {
    let mut a = Agg {
        n: runs.len(),
        died: 0,
        boss_killed: 0,
        unresolved: 0,
        death_decile: [0; DECILES + 1],
        deaths_total: 0,
        deaths_first_quarter: 0,
        death_ticks: Vec::new(),
        end_secs: Vec::new(),
        boss_reached: 0,
        ttk_secs: Vec::new(),
        boss_hp_left: Vec::new(),
        clears_boss: Vec::new(),
    };
    for r in runs {
        match r.outcome {
            Outcome::Died => a.died += 1,
            Outcome::BossKilled => a.boss_killed += 1,
            Outcome::Unresolved => a.unresolved += 1,
        }
        if r.outcome == Outcome::Died {
            a.deaths_total += 1;
            a.death_ticks.push(r.end_tick as i64);
            let bucket = if r.end_tick >= RUN_REF_TICKS {
                DECILES // boss phase
            } else {
                ((r.end_tick as u64 * DECILES as u64) / RUN_REF_TICKS as u64) as usize
            };
            a.death_decile[bucket.min(DECILES)] += 1;
            if r.end_tick < RUN_REF_TICKS / 4 {
                a.deaths_first_quarter += 1;
            }
        }
        if r.boss_reached {
            a.boss_reached += 1;
            a.clears_boss.push(r.clears_boss_phase as i64);
            if let Some(hp) = r.boss_hp_left {
                a.boss_hp_left.push(hp);
            }
        }
        if let Some(t) = r.boss_kill_tick {
            a.ttk_secs.push(((t - content::BOSS_SPAWN_TICK) / TICK_HZ) as i64);
        }
        a.end_secs.push(r.end_secs as i64);
    }
    a.death_ticks.sort_unstable();
    a.end_secs.sort_unstable();
    a.ttk_secs.sort_unstable();
    a.boss_hp_left.sort_unstable();
    a.clears_boss.sort_unstable();
    a
}

/// Largest single-decile share of all deaths, in percent (the spike proxy).
fn max_decile_share(a: &Agg) -> f64 {
    if a.deaths_total == 0 {
        return 0.0;
    }
    let m = a.death_decile.iter().copied().max().unwrap_or(0);
    100.0 * m as f64 / a.deaths_total as f64
}

// ---- human report -----------------------------------------------------------

fn print_human(runs: &[Run], a: &Agg, cap: u32, challenge: Challenge, threads: usize, rows: bool) {
    let n = a.n as f64;
    let pct = |x: usize| 100.0 * x as f64 / n;

    if rows {
        println!("seed  end_tick  end_time  outcome     boss  ttk    distinct/copies  top weapon      peakHP     earned");
        for r in runs {
            println!(
                "{:>4}  {:>8}  {:>8}  {:<10}  {:<4}  {:<5}  {:>8}/{:<6}  {:<14}  {:>9}  {:>9}",
                r.seed,
                r.end_tick,
                fmt_secs(r.end_secs),
                r.outcome.label(),
                if r.boss_reached { "yes" } else { "no" },
                match r.boss_kill_tick {
                    Some(t) => fmt_secs((t - content::BOSS_SPAWN_TICK) / TICK_HZ),
                    None => "-".into(),
                },
                r.distinct_weapons,
                r.total_weapons,
                r.top_weapon,
                mag(r.peak_max_hp),
                mag(r.total_gold_earned),
            );
        }
        println!();
    }

    println!(
        "=== balance sweep — {} seeds ({}..{}), challenge {} ===",
        a.n,
        runs.first().map_or(0, |r| r.seed),
        runs.last().map_or(0, |r| r.seed + 1),
        challenge_name(challenge)
    );
    println!(
        "cap {}t ({}), boss spawns {}t ({}), {} worker thread(s)",
        cap,
        fmt_secs(cap / TICK_HZ),
        content::BOSS_SPAWN_TICK,
        fmt_secs(content::BOSS_SPAWN_TICK / TICK_HZ),
        threads
    );
    println!("targets in brackets are docs/11-fun.md §11.2\n");

    // ---- headline, each beside its target band ----
    let win = pct(a.boss_killed);
    let q1 = 100.0 * a.deaths_first_quarter as f64 / n;
    let peak_hp_sorted = sorted_i64(runs.iter().map(|r| r.peak_max_hp));
    let peak_hp_med = pctl(&peak_hp_sorted, 50);
    let spike = max_decile_share(a);
    let winners: Vec<&Run> = runs.iter().filter(|r| r.outcome == Outcome::BossKilled).collect();
    let win_distinct = if winners.is_empty() {
        0.0
    } else {
        winners.iter().map(|r| r.distinct_weapons as f64).sum::<f64>() / winners.len() as f64
    };

    println!("HEADLINE                                       measured        target");
    println!(
        "  win rate (boss killed)                  {:>6} ({:>5.1}%)   [{:.0}-{:.0}%]      {}",
        a.boss_killed,
        win,
        T_WIN_LO,
        T_WIN_HI,
        verdict(win, T_WIN_LO, T_WIN_HI)
    );
    println!(
        "  deaths in first quarter (<{})         {:>6} ({:>5.1}%)   [{:.0}-{:.0}%]      {}",
        fmt_secs(RUN_REF_TICKS / 4 / TICK_HZ),
        a.deaths_first_quarter,
        q1,
        T_Q1_LO,
        T_Q1_HI,
        verdict(q1, T_Q1_LO, T_Q1_HI)
    );
    println!(
        "  biggest death decile (spike PROXY)             {:>6.1}%   [<{:.0}%]        {}",
        spike,
        T_MAX_DECILE_SHARE,
        if a.deaths_total == 0 {
            "n/a (no deaths)"
        } else if spike <= T_MAX_DECILE_SHARE {
            "IN"
        } else {
            "OUT(spike)"
        }
    );
    // The runaway-numbers problem (§11.1) is an OUTLIER problem — a tidy median
    // hides the one seed that reached 3.7B — so the verdict rides the WORST run
    // and the median is printed beside it for context.
    let peak_hp_max = *peak_hp_sorted.last().unwrap_or(&0);
    println!(
        "  peak max HP (worst run; median {:>7})    {:>7}   [<{}]      {}",
        mag(peak_hp_med),
        mag(peak_hp_max),
        mag(T_PEAK_HP_MAX),
        if peak_hp_max <= T_PEAK_HP_MAX { "IN" } else { "OUT(high)" }
    );
    println!(
        "  distinct weapons, winning builds (mean)       {:>7.1}   [>={:.0}]         {}",
        win_distinct,
        T_DISTINCT_WEAPONS_MIN,
        if winners.is_empty() {
            "n/a (no wins)"
        } else if win_distinct >= T_DISTINCT_WEAPONS_MIN {
            "IN"
        } else {
            "OUT(low)"
        }
    );

    // ---- outcomes ----
    println!("\noutcomes:");
    println!("  died                 : {:>3}  ({:>5.1}%)", a.died, pct(a.died));
    println!(
        "  boss killed (WIN)    : {:>3}  ({:>5.1}%)",
        a.boss_killed,
        pct(a.boss_killed)
    );
    println!(
        "  unresolved at cap    : {:>3}  ({:>5.1}%)   alive at the cap, boss not killed",
        a.unresolved,
        pct(a.unresolved)
    );

    // ---- death-time distribution ----
    let dec_ticks = RUN_REF_TICKS / DECILES as u32;
    println!(
        "\ndeath-time distribution ({} deaths; deciles of the {} designed run, {} each):",
        a.deaths_total,
        fmt_secs(RUN_REF_TICKS / TICK_HZ),
        fmt_secs(dec_ticks / TICK_HZ)
    );
    let dshare = |x: usize| {
        if a.deaths_total == 0 {
            0.0
        } else {
            100.0 * x as f64 / a.deaths_total as f64
        }
    };
    for d in 0..DECILES {
        let from = d as u32 * dec_ticks;
        let to = from + dec_ticks;
        let c = a.death_decile[d];
        println!(
            "  D{:<2} {:>5}-{:<6} {:>3}  ({:>5.1}% of deaths, {:>5.1}% of runs)  {}",
            d + 1,
            fmt_secs(from / TICK_HZ),
            fmt_secs(to / TICK_HZ),
            c,
            dshare(c),
            pct(c),
            "#".repeat(c.min(40))
        );
    }
    let bp = a.death_decile[DECILES];
    println!(
        "  BOSS {:>5}+       {:>3}  ({:>5.1}% of deaths, {:>5.1}% of runs)  {}",
        fmt_secs(RUN_REF_TICKS / TICK_HZ),
        bp,
        dshare(bp),
        pct(bp),
        "#".repeat(bp.min(40))
    );
    if a.deaths_total > 0 {
        println!(
            "  death time: min {}  p25 {}  median {}  p75 {}  max {}",
            fmt_secs(a.death_ticks[0] as u32 / TICK_HZ),
            fmt_secs(pctl(&a.death_ticks, 25) as u32 / TICK_HZ),
            fmt_secs(pctl(&a.death_ticks, 50) as u32 / TICK_HZ),
            fmt_secs(pctl(&a.death_ticks, 75) as u32 / TICK_HZ),
            fmt_secs(*a.death_ticks.last().unwrap() as u32 / TICK_HZ),
        );
    } else {
        println!("  (no deaths — there is no failure state at these settings)");
    }
    println!(
        "  run length (s): mean {:.0}  median {}  min {}  max {}",
        mean_i64(&a.end_secs),
        pctl(&a.end_secs, 50),
        a.end_secs.first().copied().unwrap_or(0),
        a.end_secs.last().copied().unwrap_or(0)
    );

    // ---- boss ----
    println!(
        "\nboss (The Hippocrate, {} HP, plated):",
        mag(content::ENEMIES[content::BOSS as usize].base_hp)
    );
    println!(
        "  reached              : {:>3}/{} ({:>5.1}%)",
        a.boss_reached,
        a.n,
        pct(a.boss_reached)
    );
    println!(
        "  killed               : {:>3}/{} ({:>5.1}% of all runs, {:>5.1}% of runs that reached it)",
        a.boss_killed,
        a.n,
        pct(a.boss_killed),
        if a.boss_reached == 0 {
            0.0
        } else {
            100.0 * a.boss_killed as f64 / a.boss_reached as f64
        }
    );
    println!(
        "  reached, not killed  : {:>3}  (died to it, or ran out the cap)",
        a.boss_reached - a.boss_killed
    );
    if !a.ttk_secs.is_empty() {
        println!(
            "  time-to-kill (s)     : min {}  median {}  max {}",
            a.ttk_secs[0],
            pctl(&a.ttk_secs, 50),
            a.ttk_secs.last().unwrap()
        );
    } else {
        println!("  time-to-kill (s)     : n/a (never killed)");
    }
    if !a.boss_hp_left.is_empty() {
        let base = content::ENEMIES[content::BOSS as usize].base_hp.max(1);
        let med = pctl(&a.boss_hp_left, 50);
        println!(
            "  boss HP left (failed): median {} ({:.0}% of {}), min {}, max {}",
            mag(med),
            100.0 * med as f64 / base as f64,
            mag(base),
            mag(a.boss_hp_left[0]),
            mag(*a.boss_hp_left.last().unwrap())
        );
    }
    if !a.clears_boss.is_empty() {
        let base = content::ENEMIES[content::BOSS as usize].base_hp;
        println!(
            "  Clears in boss phase : median {}  max {}  ({} Clear-equivalents if Clear were the only source)",
            pctl(&a.clears_boss, 50),
            a.clears_boss.last().unwrap(),
            (base + 2_999_999) / 3_000_000
        );
    }

    // ---- magnitudes ----
    println!("\nmagnitudes (peak = highest seen during the run; final = at run end):");
    let rowm = |label: &str, peak: Vec<i64>, fin: Vec<i64>| {
        let p = sorted_i64(peak.into_iter());
        let f = sorted_i64(fin.into_iter());
        println!(
            "  {:<22} peak: median {:>9}  p90 {:>9}  max {:>9}   |  final: median {:>9}  max {:>9}",
            label,
            mag(pctl(&p, 50)),
            mag(pctl(&p, 90)),
            mag(*p.last().unwrap_or(&0)),
            mag(pctl(&f, 50)),
            mag(*f.last().unwrap_or(&0)),
        );
    };
    rowm(
        "max HP",
        runs.iter().map(|r| r.peak_max_hp).collect(),
        runs.iter().map(|r| r.final_max_hp).collect(),
    );
    rowm(
        "damage (tick | total)",
        runs.iter().map(|r| r.peak_tick_damage).collect(),
        runs.iter().map(|r| r.total_damage).collect(),
    );
    rowm(
        "gold (held | earned)",
        runs.iter().map(|r| r.peak_gold).collect(),
        runs.iter().map(|r| r.total_gold_earned).collect(),
    );
    let armor = sorted_i64(runs.iter().map(|r| r.final_armor));
    println!(
        "  {:<22} final: median {:>9}  max {:>9}",
        "armor",
        mag(pctl(&armor, 50)),
        mag(*armor.last().unwrap_or(&0))
    );

    // ---- build shape ----
    println!("\nbuild shape (arsenal at run end — stacking vs breadth):");
    let shape = |label: &str, set: &[&Run]| {
        if set.is_empty() {
            println!("  {:<16}: (none)", label);
            return;
        }
        let dist = sorted_i64(set.iter().map(|r| r.distinct_weapons as i64));
        let tot = sorted_i64(set.iter().map(|r| r.total_weapons as i64));
        let share: Vec<f64> = set.iter().map(|r| r.top_share_pct()).collect();
        println!(
            "  {:<16}: {:>3} runs  distinct median {:>2} (mean {:>4.1})  copies median {:>3}  top-weapon share mean {:>5.1}%  max {:>5.1}%",
            label,
            set.len(),
            pctl(&dist, 50),
            mean_i64(&dist),
            pctl(&tot, 50),
            mean_f64(&share),
            share.iter().cloned().fold(0.0, f64::max),
        );
    };
    let all: Vec<&Run> = runs.iter().collect();
    shape("all runs", &all);
    shape("winners (boss)", &winners);
    let losers: Vec<&Run> = runs.iter().filter(|r| r.outcome == Outcome::Died).collect();
    shape("deaths", &losers);
    // Which weapon dominates arsenals, and how often.
    let mut tops: Vec<&'static str> = runs.iter().map(|r| r.top_weapon).collect();
    tops.sort_unstable();
    let mut best: (usize, &'static str) = (0, "-");
    let mut i = 0usize;
    while i < tops.len() {
        let mut j = i;
        while j < tops.len() && tops[j] == tops[i] {
            j += 1;
        }
        if j - i > best.0 {
            best = (j - i, tops[i]);
        }
        i = j;
    }
    println!(
        "  most common top weapon: {} (dominates {}/{} arsenals)",
        best.1, best.0, a.n
    );

    // ---- per-archetype ----
    println!("\nper-archetype (the bot's build identity, seeded per match):");
    for arch in [
        Archetype::GlassCannon,
        Archetype::Tanky,
        Archetype::EcoPivot,
        Archetype::Balanced,
    ] {
        let g: Vec<&Run> = runs.iter().filter(|r| r.archetype == arch).collect();
        if g.is_empty() {
            println!("  {:<12}: 0 seeds", arch_name(arch));
            continue;
        }
        let wins = g.iter().filter(|r| r.outcome == Outcome::BossKilled).count();
        let php = sorted_i64(g.iter().map(|r| r.peak_max_hp));
        let ends = sorted_i64(g.iter().map(|r| r.end_secs as i64));
        println!(
            "  {:<12}: {:>2} seeds  wins {:>2} ({:>5.1}%)  boss reached {:>2}  median end {:>6}  distinct wpn {:>4.1}  peak HP med {:>9}",
            arch_name(arch),
            g.len(),
            wins,
            100.0 * wins as f64 / g.len() as f64,
            g.iter().filter(|r| r.boss_reached).count(),
            fmt_secs(pctl(&ends, 50) as u32),
            g.iter().map(|r| r.distinct_weapons as f64).sum::<f64>() / g.len() as f64,
            mag(pctl(&php, 50)),
        );
    }
}

// ---- JSON report ------------------------------------------------------------

/// Minimal hand-rolled JSON (the workspace has zero external dependencies, and a
/// sweep report is not worth adding one). Every key is a fixed literal; the only
/// variable strings are content names, escaped here.
fn jstr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn f2(v: f64) -> String {
    format!("{:.2}", v)
}

fn print_json(runs: &[Run], a: &Agg, cap: u32, challenge: Challenge, threads: usize, rows: bool) {
    let n = a.n as f64;
    let pct = |x: usize| 100.0 * x as f64 / n;
    let win = pct(a.boss_killed);
    let q1 = 100.0 * a.deaths_first_quarter as f64 / n;
    let spike = max_decile_share(a);
    let peak_hp = sorted_i64(runs.iter().map(|r| r.peak_max_hp));
    let winners: Vec<&Run> = runs.iter().filter(|r| r.outcome == Outcome::BossKilled).collect();
    let win_distinct = if winners.is_empty() {
        0.0
    } else {
        winners.iter().map(|r| r.distinct_weapons as f64).sum::<f64>() / winners.len() as f64
    };

    println!("{{");
    println!("  \"schema\": \"sweep/1\",");
    println!("  \"config\": {{");
    println!("    \"seed_start\": {},", runs.first().map_or(0, |r| r.seed));
    println!("    \"seeds\": {},", a.n);
    println!("    \"cap_ticks\": {},", cap);
    println!("    \"boss_spawn_tick\": {},", content::BOSS_SPAWN_TICK);
    println!("    \"run_ref_ticks\": {},", RUN_REF_TICKS);
    println!("    \"tick_hz\": {},", TICK_HZ);
    println!("    \"threads\": {},", threads);
    println!("    \"challenge\": {}", jstr(&challenge_name(challenge)));
    println!("  }},");

    println!(
        "  \"outcomes\": {{ \"died\": {}, \"boss_killed\": {}, \"unresolved\": {} }},",
        a.died, a.boss_killed, a.unresolved
    );

    println!("  \"headline\": {{");
    println!(
        "    \"win_rate_pct\": {{ \"value\": {}, \"target_min\": {}, \"target_max\": {}, \"verdict\": {} }},",
        f2(win),
        T_WIN_LO,
        T_WIN_HI,
        jstr(verdict(win, T_WIN_LO, T_WIN_HI))
    );
    println!(
        "    \"first_quarter_death_pct\": {{ \"value\": {}, \"target_min\": {}, \"target_max\": {}, \"verdict\": {} }},",
        f2(q1),
        T_Q1_LO,
        T_Q1_HI,
        jstr(verdict(q1, T_Q1_LO, T_Q1_HI))
    );
    println!(
        "    \"max_death_decile_share_pct\": {{ \"value\": {}, \"target_max\": {}, \"is_proxy\": true }},",
        f2(spike),
        T_MAX_DECILE_SHARE
    );
    println!(
        "    \"peak_max_hp\": {{ \"median\": {}, \"worst\": {}, \"target_max\": {}, \"verdict\": {} }},",
        pctl(&peak_hp, 50),
        peak_hp.last().copied().unwrap_or(0),
        T_PEAK_HP_MAX,
        jstr(if peak_hp.last().copied().unwrap_or(0) <= T_PEAK_HP_MAX { "IN" } else { "OUT(high)" })
    );
    println!(
        "    \"winner_distinct_weapons_mean\": {{ \"value\": {}, \"target_min\": {} }}",
        f2(win_distinct),
        T_DISTINCT_WEAPONS_MIN
    );
    println!("  }},");

    let dec_ticks = RUN_REF_TICKS / DECILES as u32;
    println!("  \"death_distribution\": {{");
    println!("    \"deaths\": {},", a.deaths_total);
    println!("    \"first_quarter_deaths\": {},", a.deaths_first_quarter);
    println!("    \"deciles\": [");
    for d in 0..DECILES {
        println!(
            "      {{ \"index\": {}, \"from_tick\": {}, \"to_tick\": {}, \"deaths\": {} }}{}",
            d,
            d as u32 * dec_ticks,
            (d as u32 + 1) * dec_ticks,
            a.death_decile[d],
            if d + 1 < DECILES { "," } else { "" }
        );
    }
    println!("    ],");
    println!("    \"boss_phase_deaths\": {},", a.death_decile[DECILES]);
    println!(
        "    \"death_tick_min\": {},",
        a.death_ticks.first().copied().unwrap_or(-1)
    );
    println!(
        "    \"death_tick_median\": {},",
        if a.death_ticks.is_empty() { -1 } else { pctl(&a.death_ticks, 50) }
    );
    println!(
        "    \"death_tick_max\": {}",
        a.death_ticks.last().copied().unwrap_or(-1)
    );
    println!("  }},");

    println!(
        "  \"run_length_secs\": {{ \"mean\": {}, \"median\": {}, \"min\": {}, \"max\": {} }},",
        f2(mean_i64(&a.end_secs)),
        pctl(&a.end_secs, 50),
        a.end_secs.first().copied().unwrap_or(0),
        a.end_secs.last().copied().unwrap_or(0)
    );

    println!("  \"boss\": {{");
    println!(
        "    \"base_hp\": {},",
        content::ENEMIES[content::BOSS as usize].base_hp
    );
    println!("    \"reached\": {},", a.boss_reached);
    println!("    \"killed\": {},", a.boss_killed);
    println!("    \"reached_not_killed\": {},", a.boss_reached - a.boss_killed);
    println!(
        "    \"kill_rate_given_reached_pct\": {},",
        f2(if a.boss_reached == 0 {
            0.0
        } else {
            100.0 * a.boss_killed as f64 / a.boss_reached as f64
        })
    );
    println!("    \"ttk_secs_min\": {},", a.ttk_secs.first().copied().unwrap_or(-1));
    println!(
        "    \"ttk_secs_median\": {},",
        if a.ttk_secs.is_empty() { -1 } else { pctl(&a.ttk_secs, 50) }
    );
    println!("    \"ttk_secs_max\": {},", a.ttk_secs.last().copied().unwrap_or(-1));
    println!(
        "    \"hp_left_median\": {},",
        if a.boss_hp_left.is_empty() { -1 } else { pctl(&a.boss_hp_left, 50) }
    );
    println!(
        "    \"clears_boss_phase_median\": {}",
        if a.clears_boss.is_empty() { -1 } else { pctl(&a.clears_boss, 50) }
    );
    println!("  }},");

    let stat = |name: &str, v: Vec<i64>, last: bool| {
        let s = sorted_i64(v.into_iter());
        println!(
            "    \"{}\": {{ \"median\": {}, \"p90\": {}, \"max\": {} }}{}",
            name,
            pctl(&s, 50),
            pctl(&s, 90),
            s.last().copied().unwrap_or(0),
            if last { "" } else { "," }
        );
    };
    println!("  \"magnitudes\": {{");
    stat("peak_max_hp", runs.iter().map(|r| r.peak_max_hp).collect(), false);
    stat("final_max_hp", runs.iter().map(|r| r.final_max_hp).collect(), false);
    stat("peak_tick_damage", runs.iter().map(|r| r.peak_tick_damage).collect(), false);
    stat("total_damage", runs.iter().map(|r| r.total_damage).collect(), false);
    stat("peak_gold_held", runs.iter().map(|r| r.peak_gold).collect(), false);
    stat("total_gold_earned", runs.iter().map(|r| r.total_gold_earned).collect(), false);
    stat("final_armor", runs.iter().map(|r| r.final_armor).collect(), true);
    println!("  }},");

    println!("  \"build_shape\": {{");
    let shape = |name: &str, set: &[&Run], last: bool| {
        let dist = sorted_i64(set.iter().map(|r| r.distinct_weapons as i64));
        let tot = sorted_i64(set.iter().map(|r| r.total_weapons as i64));
        let share: Vec<f64> = set.iter().map(|r| r.top_share_pct()).collect();
        println!(
            "    \"{}\": {{ \"runs\": {}, \"distinct_median\": {}, \"distinct_mean\": {}, \"copies_median\": {}, \"top_share_mean_pct\": {} }}{}",
            name,
            set.len(),
            pctl(&dist, 50),
            f2(mean_i64(&dist)),
            pctl(&tot, 50),
            f2(mean_f64(&share)),
            if last { "" } else { "," }
        );
    };
    let all: Vec<&Run> = runs.iter().collect();
    let losers: Vec<&Run> = runs.iter().filter(|r| r.outcome == Outcome::Died).collect();
    shape("all", &all, false);
    shape("winners", &winners, false);
    shape("deaths", &losers, true);
    println!("  }},");

    println!("  \"archetypes\": [");
    let arches = [
        Archetype::GlassCannon,
        Archetype::Tanky,
        Archetype::EcoPivot,
        Archetype::Balanced,
    ];
    for (i, arch) in arches.iter().enumerate() {
        let g: Vec<&Run> = runs.iter().filter(|r| r.archetype == *arch).collect();
        let wins = g.iter().filter(|r| r.outcome == Outcome::BossKilled).count();
        let ends = sorted_i64(g.iter().map(|r| r.end_secs as i64));
        let php = sorted_i64(g.iter().map(|r| r.peak_max_hp));
        println!(
            "    {{ \"name\": {}, \"seeds\": {}, \"wins\": {}, \"boss_reached\": {}, \"end_secs_median\": {}, \"distinct_weapons_mean\": {}, \"peak_max_hp_median\": {} }}{}",
            jstr(arch_name(*arch)),
            g.len(),
            wins,
            g.iter().filter(|r| r.boss_reached).count(),
            pctl(&ends, 50),
            f2(if g.is_empty() {
                0.0
            } else {
                g.iter().map(|r| r.distinct_weapons as f64).sum::<f64>() / g.len() as f64
            }),
            pctl(&php, 50),
            if i + 1 < arches.len() { "," } else { "" }
        );
    }
    println!("  ]{}", if rows { "," } else { "" });

    if rows {
        println!("  \"runs\": [");
        for (i, r) in runs.iter().enumerate() {
            println!(
                "    {{ \"seed\": {}, \"archetype\": {}, \"outcome\": {}, \"end_tick\": {}, \"boss_reached\": {}, \"boss_kill_tick\": {}, \"boss_hp_left\": {}, \"clears\": {}, \"clears_boss_phase\": {}, \"peak_max_hp\": {}, \"final_max_hp\": {}, \"peak_tick_damage\": {}, \"total_damage\": {}, \"peak_gold\": {}, \"total_gold_earned\": {}, \"distinct_weapons\": {}, \"total_weapons\": {}, \"top_weapon\": {}, \"top_weapon_count\": {} }}{}",
                r.seed,
                jstr(arch_name(r.archetype)),
                jstr(r.outcome.json()),
                r.end_tick,
                r.boss_reached,
                r.boss_kill_tick.map_or(-1i64, |t| t as i64),
                r.boss_hp_left.unwrap_or(-1),
                r.clears,
                r.clears_boss_phase,
                r.peak_max_hp,
                r.final_max_hp,
                r.peak_tick_damage,
                r.total_damage,
                r.peak_gold,
                r.total_gold_earned,
                r.distinct_weapons,
                r.total_weapons,
                jstr(r.top_weapon),
                r.top_weapon_count,
                if i + 1 < runs.len() { "," } else { "" }
            );
        }
        println!("  ]");
    }
    println!("}}");
}

fn main() {
    let mut seeds = DEFAULT_SEEDS;
    let mut seed_start = 0u64;
    let mut cap = DEFAULT_CAP_TICKS;
    let mut rows = false;
    let mut json = false;
    let mut threads = 0usize; // 0 ⇒ auto
    let mut challenge = Challenge::None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seeds" => seeds = it.next().and_then(|v| v.parse().ok()).unwrap_or(seeds),
            "--seed-start" => {
                seed_start = it.next().and_then(|v| v.parse().ok()).unwrap_or(seed_start)
            }
            "--cap" => cap = it.next().and_then(|v| v.parse().ok()).unwrap_or(cap),
            "--rows" => rows = true,
            "--json" => json = true,
            "--threads" => threads = it.next().and_then(|v| v.parse().ok()).unwrap_or(threads),
            // Drive the bot under a self-imposed playstyle (challenge code, see
            // `Challenge::from_code`): 9 = pure-economy (weapons forbidden). The
            // default is the greedy survivor (code 0/None).
            "--challenge" => {
                challenge = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .map(Challenge::from_code)
                    .unwrap_or(challenge)
            }
            "--eco" => challenge = Challenge::EcoOnly, // shorthand for --challenge 9
            "-h" | "--help" => {
                println!("sweep [--seeds N] [--seed-start S] [--cap TICKS] [--rows] [--json]");
                println!("      [--threads N] [--challenge CODE | --eco]");
                println!();
                println!("Plays each seed to a real conclusion (death / boss killed / cap) and");
                println!("reports it against the docs/11-fun.md §11.2 target bands.");
                println!(
                    "Default cap {} ticks ({}) — {} past the {} boss spawn.",
                    DEFAULT_CAP_TICKS,
                    fmt_secs(DEFAULT_CAP_TICKS / TICK_HZ),
                    fmt_secs(BOSS_FIGHT_ROOM / TICK_HZ),
                    fmt_secs(content::BOSS_SPAWN_TICK / TICK_HZ)
                );
                return;
            }
            _ => {}
        }
    }
    if seeds == 0 {
        eprintln!("sweep: --seeds must be >= 1");
        std::process::exit(2);
    }
    if cap < content::BOSS_SPAWN_TICK {
        eprintln!(
            "sweep: WARNING cap {} is BELOW the boss spawn ({}) — no run can reach the win\n\
             condition, so the win rate reads 0% by construction. This is the exact\n\
             mismeasurement docs/11-fun.md calls out; pass --cap >= {} for a real number.",
            cap,
            content::BOSS_SPAWN_TICK,
            DEFAULT_CAP_TICKS
        );
    }
    // Thread count affects only speed, never results (see `run_all`).
    // `available_parallelism` reads the machine's CPU budget — not a clock, not RNG.
    if threads == 0 {
        threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    }
    threads = threads.clamp(1, seeds as usize);

    let runs = run_all(seed_start, seeds, cap, challenge, threads);
    let agg = aggregate(&runs);
    if json {
        print_json(&runs, &agg, cap, challenge, threads, rows);
    } else {
        print_human(&runs, &agg, cap, challenge, threads, rows);
    }
}
