// Independent integrator probe of the M1 correction loop.
use harness::shadow::{replay_from_snapshot, ShadowRunner};
use harness::{input_at, m0_scenario};
use sim::snapshot::serialize;
use sim::{checksum, ArenaState, Input};

fn main() {
    let sc = m0_scenario();
    let mut r = ShadowRunner::new(sc.master_seed, 0, 30);

    // run to 205 (between digest boundaries 180 and 210)
    for tick in 0..205 { r.step(input_at(&sc, tick)); }
    println!("pre-injection : in_sync={} corrections={}", r.in_sync(), r.corrections);

    // inject a desync into the CLIENT only
    r.client.economy.gold += 777;
    r.client.tank.hp -= 123;
    println!("post-injection: in_sync={} (expect false)", r.in_sync());

    // step across the next digest boundary (210)
    for tick in 205..211 { r.step(input_at(&sc, tick)); }
    println!("after boundary: in_sync={} corrections={} at_tick={:?} bytes_match={}",
        r.in_sync(), r.corrections, r.last_correction_tick,
        serialize(&r.client) == serialize(&r.shadow));

    // reconnect/replay independent check
    let mut ref_sim = ArenaState::new(sc.master_seed, 0);
    for tick in 0..600 { sim::step(&mut ref_sim, input_at(&sc, tick)); }
    let snap = serialize(&ref_sim);
    let inputs: Vec<Input> = (600..1500).map(|t| input_at(&sc, t)).collect();
    for tick in 600..1500 { sim::step(&mut ref_sim, input_at(&sc, tick)); }
    let rebuilt = replay_from_snapshot(&snap, &inputs);
    println!("reconnect     : replay_matches={}", checksum(&rebuilt) == checksum(&ref_sim));
}
