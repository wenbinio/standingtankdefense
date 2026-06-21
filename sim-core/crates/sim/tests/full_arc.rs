//! M4 systems-integration: drive the FULL `step()` pipeline (all 11 phases —
//! shop, input, spawning with scaling, all attack types, status ticking,
//! defensive layer, economy, boss) for an entire match to the Samwise spawn,
//! and prove the whole thing composes deterministically without panic
//! (overflow-checks are on, so this also guards the large-number paths).

use sim::{checksum, content, step, ArenaState, Input};

fn scripted(tick: u32) -> Input {
    match tick {
        5 => Input::BuyOffer { slot: 0 },
        50 => Input::BuyOffer { slot: 1 },
        120 => Input::BuyOffer { slot: 2 },
        300 => Input::Reroll,
        305 => Input::BuyOffer { slot: 0 },
        900 => Input::BuyOffer { slot: 1 },
        1800 => Input::Clear,
        9000 => Input::BuyOffer { slot: 0 },
        _ => Input::Noop,
    }
}

fn run_to_boss() -> (u64, bool, usize) {
    let mut s = ArenaState::new(0xFACE_B055, 0);
    // Survive to the boss so the full late-game arc executes.
    s.tank.hp = 1_000_000_000_000;
    s.tank.max_hp = s.tank.hp;
    for tick in 0..(content::BOSS_SPAWN_TICK + 100) {
        step(&mut s, scripted(tick));
    }
    let boss_present = s
        .enemies
        .iter()
        .any(|e| content::ENEMIES[e.def as usize].boss);
    (checksum(&s), boss_present, s.weapons.len())
}

#[test]
fn full_match_arc_is_deterministic_and_reaches_the_boss() {
    let a = run_to_boss();
    let b = run_to_boss();
    assert_eq!(a.0, b.0, "the full match arc must be byte-deterministic");
    assert!(a.1, "Samwise must be present after the boss tick");
    assert!(a.2 > 1, "the scripted buys should have added weapons");
}
