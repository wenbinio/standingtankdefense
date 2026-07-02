//! Determinism harness CLI. Exit 0 iff the M0 determinism gate holds.
use harness::*;

fn main() {
    let sc = m0_scenario();

    // 1. Identical replays must produce identical checksum traces.
    let a = run_trace(&sc);
    let b = run_trace(&sc);
    let identical = a == b;

    // 2. Sanity: changing the seed must change the outcome (not a constant).
    let mut sc2 = m0_scenario();
    sc2.master_seed ^= 0xFFFF_FFFF_FFFF_FFFF;
    let seed_sensitive = final_checksum(&sc2) != *a.last().unwrap();

    println!("ticks            = {}", sc.total_ticks);
    println!("final_checksum   = {:#018x}", a.last().unwrap());
    println!(
        "identical_runs   = {}",
        if identical { "PASS" } else { "FAIL" }
    );
    println!(
        "seed_sensitive   = {}",
        if seed_sensitive { "PASS" } else { "FAIL" }
    );

    if identical && seed_sensitive {
        println!("M0 DETERMINISM GATE: PASS");
    } else {
        println!("M0 DETERMINISM GATE: FAIL");
        std::process::exit(1);
    }
}
