//! Survival-curve seed sweep over the real `sim::step` + `sim::bot::Bot`.
//!
//! Runs seeds 0..N (default 80), each to the tank's death or a hard tick cap,
//! recording the death tick / survival seconds and whether the run "won" (i.e.
//! survived to the cap — this is an endless survival sim with no explicit
//! victory state, so reaching the cap alive is the closest analogue).
//!
//! Prints per-seed rows plus an aggregate table: survival buckets, mean/median/
//! min/max survival, win rate, the <30s death rate, and the global min death.
//! Pure observation of the public arena state — it never bends the sim, so it is
//! a faithful proxy for the survival curve. Compare BEFORE vs AFTER a bot change
//! by running this binary on each version (identical methodology, only the bot
//! decision logic differs).
//!
//!   sweep                 # seeds 0..80, cap = 20 min
//!   sweep --seeds 80 --cap 36000 --rows   # explicit knobs; --rows prints each seed

use sim::bot::Bot;
use sim::{step, ArenaState, TICK_HZ};

/// Survival cap: 20 minutes @ 30 Hz. Past the 15-min boss and the second scale
/// step, so anything reaching it has cleared the designed difficulty ramp.
const DEFAULT_CAP_TICKS: u32 = 20 * 60 * TICK_HZ;
const DEFAULT_SEEDS: u64 = 80;

struct Run {
    seed: u64,
    death_tick: u32,
    survived_secs: u32,
    won: bool,
}

fn run_seed(seed: u64, cap: u32) -> Run {
    let mut s = ArenaState::new(seed, 0);
    let mut bot = Bot::default();
    while s.tick < cap && !s.dead {
        let action = bot.decide(&s);
        step(&mut s, action);
    }
    Run {
        seed,
        death_tick: s.tick,
        survived_secs: s.tick / TICK_HZ,
        won: !s.dead, // reached the cap alive
    }
}

fn main() {
    let mut seeds = DEFAULT_SEEDS;
    let mut cap = DEFAULT_CAP_TICKS;
    let mut rows = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seeds" => seeds = it.next().and_then(|v| v.parse().ok()).unwrap_or(seeds),
            "--cap" => cap = it.next().and_then(|v| v.parse().ok()).unwrap_or(cap),
            "--rows" => rows = true,
            "-h" | "--help" => {
                println!("sweep [--seeds N] [--cap TICKS] [--rows]");
                return;
            }
            _ => {}
        }
    }

    let runs: Vec<Run> = (0..seeds).map(|seed| run_seed(seed, cap)).collect();

    if rows {
        println!("seed  death_tick  survived  result");
        for r in &runs {
            println!(
                "{:>4}  {:>10}  {:>5}s  {}",
                r.seed,
                r.death_tick,
                r.survived_secs,
                if r.won { "WON (cap)" } else { "DEAD" }
            );
        }
        println!();
    }

    // ---- buckets (by survival seconds) ----
    let mut b_lt5 = 0; // <5s
    let mut b_5_30 = 0; // 5-30s
    let mut b_30_180 = 0; // 30s-3min
    let mut b_180_600 = 0; // 3-10min
    let mut b_gt600 = 0; // >10min
    for r in &runs {
        let t = r.survived_secs;
        if t < 5 {
            b_lt5 += 1;
        } else if t < 30 {
            b_5_30 += 1;
        } else if t < 180 {
            b_30_180 += 1;
        } else if t < 600 {
            b_180_600 += 1;
        } else {
            b_gt600 += 1;
        }
    }

    let n = runs.len() as u32;
    let secs: Vec<u32> = {
        let mut v: Vec<u32> = runs.iter().map(|r| r.survived_secs).collect();
        v.sort_unstable();
        v
    };
    let mean = runs.iter().map(|r| r.survived_secs as u64).sum::<u64>() as f64 / n as f64;
    let median = if n % 2 == 1 {
        secs[(n / 2) as usize] as f64
    } else {
        (secs[(n / 2 - 1) as usize] as f64 + secs[(n / 2) as usize] as f64) / 2.0
    };
    let min = *secs.first().unwrap();
    let max = *secs.last().unwrap();
    let wins = runs.iter().filter(|r| r.won).count();
    // <30s death rate: died (not a cap-win) before 30s.
    let lt30 = runs.iter().filter(|r| !r.won && r.survived_secs < 30).count();
    let min_death_tick = runs
        .iter()
        .filter(|r| !r.won)
        .map(|r| r.death_tick)
        .min();

    let pct = |x: usize| 100.0 * x as f64 / n as f64;
    let pctc = |x: u32| 100.0 * x as f64 / n as f64;

    println!("=== survival sweep: {} seeds (0..{}), cap {}t = {}s ===", n, n, cap, cap / TICK_HZ);
    println!("buckets:");
    println!("  <5s      : {:>3}  ({:>5.1}%)", b_lt5, pctc(b_lt5));
    println!("  5-30s    : {:>3}  ({:>5.1}%)", b_5_30, pctc(b_5_30));
    println!("  30s-3min : {:>3}  ({:>5.1}%)", b_30_180, pctc(b_30_180));
    println!("  3-10min  : {:>3}  ({:>5.1}%)", b_180_600, pctc(b_180_600));
    println!("  >10min   : {:>3}  ({:>5.1}%)", b_gt600, pctc(b_gt600));
    println!("survival (s): mean {:.1}  median {:.1}  min {}  max {}", mean, median, min, max);
    println!("win rate (reached cap): {}/{} ({:.1}%)", wins, n, pct(wins));
    println!("<30s death rate: {}/{} ({:.1}%)", lt30, n, pct(lt30));
    match min_death_tick {
        Some(t) => println!("min death: tick {} = {:.2}s", t, t as f64 / TICK_HZ as f64),
        None => println!("min death: none (no deaths)"),
    }
}
