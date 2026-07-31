//! Attribution profile for `sim::step` cost. Runs the reference bot for a fixed
//! tick budget and reports wall time both raw and NORMALISED by the workload
//! actually simulated (enemy-ticks, weapon-fire scans, projectile-ticks), so
//! "more enemies" can be told apart from "each tick got more expensive".
//!
//! Ticks after the tank dies are NOT counted (`sim::step` early-returns), so all
//! rates are per LIVE tick.
//!
//! Usage: cargo run -p harness --release --example step_profile [seeds] [ticks]

use std::time::Instant;

use sim::bot::Bot;
use sim::{step, ArenaState};

fn main() {
    let mut args = std::env::args().skip(1);
    let seeds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(8);
    let ticks: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30_000);
    // The per-tick timer is opt-in: two `Instant::now()` calls per tick are not
    // free next to a 7 us tick, so the headline wall figure must be taken without
    // it and the bucket breakdown read as shape, not as absolute cost.
    let bucketed: bool = args.next().is_some_and(|s| s == "buckets");

    let mut tot_secs = 0.0f64;
    let mut live_ticks: u128 = 0;
    let mut enemy_ticks: u128 = 0;
    let mut proj_ticks: u128 = 0;
    let mut weapon_ticks: u128 = 0;
    // Scans a naive `fire_weapons` would do: one pass over the enemy list for
    // every weapon considered this tick.
    let mut scan_pairs: u128 = 0;
    let mut peak_e = 0usize;
    let mut peak_w = 0usize;
    let mut alive = 0u32;
    // Per-tick cost bucketed by board size, so "a full board is expensive" can be
    // told apart from "every tick is expensive". Bucket i covers E in [25i, 25i+25).
    const BUCKETS: usize = 12;
    let mut bucket_ns = [0u128; BUCKETS];
    let mut bucket_n = [0u64; BUCKETS];

    for seed in 0..seeds {
        let mut s = ArenaState::new(0xC0FFEE + seed, 0);
        let mut bot = Bot::default();
        let mut n = 0u32;
        let t0 = Instant::now();
        for _ in 0..ticks {
            let inp = bot.decide(&s);
            let t1 = if bucketed { Some(Instant::now()) } else { None };
            step(&mut s, inp);
            n += 1;
            let e = s.enemies.len();
            if let Some(t1) = t1 {
                let b = (e / 25).min(BUCKETS - 1);
                bucket_ns[b] += t1.elapsed().as_nanos();
                bucket_n[b] += 1;
            }
            let w = s.weapons.len();
            enemy_ticks += e as u128;
            proj_ticks += s.projectiles.len() as u128;
            weapon_ticks += w as u128;
            scan_pairs += (e * w) as u128;
            peak_e = peak_e.max(e);
            peak_w = peak_w.max(w);
            if s.tank.hp <= 0 {
                break;
            }
        }
        tot_secs += t0.elapsed().as_secs_f64();
        live_ticks += n as u128;
        if s.tank.hp > 0 {
            alive += 1;
        }
        println!("  seed {seed}: live_ticks={n} final_E={} W={}", s.enemies.len(), s.weapons.len());
    }

    println!("seeds={seeds} cap={ticks} survived={alive}/{seeds} live_ticks={live_ticks}");
    println!(
        "wall            : {tot_secs:.3} s   ({:.3} us / live tick)",
        tot_secs * 1e6 / live_ticks.max(1) as f64
    );
    println!("enemy-ticks     : {enemy_ticks}  (peak E={peak_e}, mean E={:.1})", enemy_ticks as f64 / live_ticks.max(1) as f64);
    println!("projectile-ticks: {proj_ticks}  (mean P={:.1})", proj_ticks as f64 / live_ticks.max(1) as f64);
    println!("weapon-ticks    : {weapon_ticks}  (peak W={peak_w}, mean W={:.1})", weapon_ticks as f64 / live_ticks.max(1) as f64);
    println!("W*E scan pairs  : {scan_pairs}");
    println!("ns per enemy-tick   : {:.1}", tot_secs * 1e9 / enemy_ticks.max(1) as f64);
    println!("ns per W*E pair     : {:.2}", tot_secs * 1e9 / scan_pairs.max(1) as f64);
    println!("cost by board size (us/tick, in-loop timer):");
    for b in 0..BUCKETS {
        if bucket_n[b] == 0 {
            continue;
        }
        println!(
            "  E {:>3}..{:<3} : {:8.3} us/tick   ({} ticks)",
            b * 25,
            b * 25 + 24,
            bucket_ns[b] as f64 / 1000.0 / bucket_n[b] as f64,
            bucket_n[b]
        );
    }
}
