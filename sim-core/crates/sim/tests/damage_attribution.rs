//! Damage-attribution ledger invariants over REAL runs (the DPS meter).
//!
//! The ledger (`ArenaState::damage_by_weapon`) must stay a pure re-bucketing
//! of the scoreboard: every damage site records the same amounts to both, so
//! at every tick boundary `Σ ledger == total_damage_dealt`. The view mirrors
//! the ledger row-for-row (`RenderView::damage_by_weapon`), cross-checked
//! against `stats.damage_dealt`.

use sim::{
    checksum, snapshot, step, view, ArenaState, Input, DMG_SRC_CLEAR, DMG_SRC_OTHER, DMG_SRC_SPIKES,
};

/// A varied scripted run: buys, rerolls, and periodic Clears so every common
/// damage path (projectiles, instants, DoT, Clear, spikes) gets exercised.
fn scripted_input(tick: u32) -> Input {
    match tick {
        5 | 40 | 125 | 700 | 1300 => Input::BuyOffer { slot: 0 },
        60 | 950 => Input::BuyOffer { slot: 1 },
        120 | 900 => Input::Reroll,
        t if t % 400 == 300 => Input::Clear,
        _ => Input::Noop,
    }
}

#[test]
fn ledger_sums_to_the_scoreboard_all_run_long() {
    let mut s = ArenaState::new(0xD9_5EED, 0);
    for tick in 0..3000u32 {
        step(&mut s, scripted_input(tick));
        let ledger: i64 = s.damage_by_weapon.values().sum();
        assert_eq!(
            ledger, s.total_damage_dealt,
            "ledger != scoreboard at tick {tick}"
        );
    }
    assert!(s.total_damage_dealt > 0, "the run dealt damage at all");
    assert!(
        s.damage_by_weapon.keys().any(|&k| k < 0xFF00),
        "at least one REAL weapon row accumulated"
    );
    assert!(
        s.damage_by_weapon.contains_key(&DMG_SRC_CLEAR),
        "the scripted Clears accumulated under the CLEAR pseudo row"
    );
}

#[test]
fn view_rows_mirror_the_ledger_and_cross_check_stats() {
    let mut s = ArenaState::new(0xD9_5EED, 0);
    for tick in 0..1500u32 {
        step(&mut s, scripted_input(tick));
    }
    let v = view::snapshot(&s);
    assert_eq!(v.damage_by_weapon.len(), s.damage_by_weapon.len());
    let mut sum = 0i64;
    let mut last_source = -1i64;
    for row in &v.damage_by_weapon {
        // Rows arrive in strictly ascending source order (stable ranking base).
        assert!((row.source as i64) > last_source, "rows not sorted");
        last_source = row.source as i64;
        assert_eq!(
            Some(&row.total),
            s.damage_by_weapon.get(&row.source),
            "row mirrors the ledger"
        );
        assert!(!row.name.is_empty());
        match row.source {
            DMG_SRC_SPIKES | DMG_SRC_CLEAR | DMG_SRC_OTHER => {
                assert_eq!(row.damage_type, 255, "pseudo rows are type-neutral");
                assert_eq!(row.count, 0, "pseudo rows own no weapon copies");
            }
            def => {
                assert!(row.damage_type <= 4);
                let owned = s.weapons.iter().filter(|w| w.def == def).count() as u32;
                assert_eq!(row.count, owned, "owned count resolves live");
            }
        }
        sum += row.total;
    }
    // THE cross-check: the meter's rows sum to the existing stats figure.
    assert_eq!(sum, v.stats.damage_dealt);
}

#[test]
fn ledger_survives_snapshot_roundtrip_and_checksums() {
    // The ledger rides the snapshot (v23) and feeds the checksum: a restored
    // arena continues with identical attribution.
    let mut s = ArenaState::new(0xABBA, 1);
    for tick in 0..1000u32 {
        step(&mut s, scripted_input(tick));
    }
    assert!(!s.damage_by_weapon.is_empty());
    let back = snapshot::deserialize(&snapshot::serialize(&s)).unwrap();
    assert_eq!(s.damage_by_weapon, back.damage_by_weapon);
    assert_eq!(checksum(&s), checksum(&back));
}
