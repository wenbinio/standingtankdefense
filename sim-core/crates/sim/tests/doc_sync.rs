//! Mechanical drift guard between the shipped sim content and the docs
//! (`docs/02`, `docs/04`, `docs/05`, `docs/06`) — the match-arc twin of
//! `net/tests/wire_doc_sync.rs` (which guards the wire catalog in `docs/04`).
//!
//! History: five docs described a 15-minute arc and a boss the shipped game
//! didn't have; a "truth pass" then rewrote them around an interim 30-minute
//! arc; and the source-fidelity program has now RESTORED the source's 15-minute
//! arc (docs/01 §1.2) as both spec and implementation. The one doc that stayed
//! true throughout (`docs/04`'s wire catalog) was the one with a test, so the
//! pattern is extended: this test fails the build if the docs drift from
//! `sim::content` again on
//!   1. the 15-minute match arc (`BOSS_SPAWN_TICK`, the 10:00 `SCALE_STEP_TICK`,
//!      and the post-15:00 swift end),
//!   2. the boss identity (display name derived from `ENEMIES[BOSS]`, and no
//!      resurrection of the WC3 source's boss name as *this* game's boss), and
//!   3. the catalog counts (derived from the `WEAPONS`/`MODIFIERS` tables, so
//!      growing the catalog forces the doc number to move with it).
//!
//! Robust-not-brittle: all doc checks are plain substring presence, so prose
//! may evolve freely as long as the load-bearing facts still appear.

use std::fs;
use std::path::PathBuf;

use sim::content::{BOSS, BOSS_SPAWN_TICK, ENEMIES, MODIFIERS, SCALE_STEP_TICK, WEAPONS};

/// Read a doc from the repo `docs/` dir (CARGO_MANIFEST_DIR = sim-core/crates/sim).
fn doc(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../docs")
        .join(name)
        .canonicalize()
        .unwrap_or_else(|e| panic!("canonicalize docs/{name}: {e}"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn boss_name() -> &'static str {
    let def = &ENEMIES[BOSS as usize];
    assert!(def.boss, "ENEMIES[BOSS] is not flagged as the boss");
    def.name
}

/// The arc constants themselves: the restored SOURCE arc — a 15-minute match
/// with the +20% scaling step at 10:00 and the boss (plus the swift-end
/// escalation, shop close, and ramp stop) at 15:00. If a deliberate redesign
/// changes these, the docs listed in the tests below must be updated in the
/// same change.
#[test]
fn match_arc_constants_are_the_source_15_minute_arc() {
    assert_eq!(
        BOSS_SPAWN_TICK,
        15 * 60 * 30,
        "boss spawn is 15 min @ 30 Hz (the source match length, docs/01 §1.2)"
    );
    assert_eq!(
        SCALE_STEP_TICK,
        10 * 60 * 30,
        "the +20% scaling step is 10 min @ 30 Hz (source changelog)"
    );
    // Exactly one boss in the roster, and it's the one `BOSS` points at.
    assert_eq!(
        ENEMIES.iter().filter(|e| e.boss).count(),
        1,
        "expected exactly one boss in ENEMIES"
    );
}

#[test]
fn design_doc_states_the_shipped_arc_and_boss() {
    let d02 = doc("02-game-design.md");
    assert!(
        d02.contains(boss_name()),
        "docs/02 must name the shipped boss ({:?})",
        boss_name()
    );
    assert!(
        d02.contains("15 min"),
        "docs/02 must state the 15-minute match arc"
    );
    assert!(
        d02.contains("10 min") || d02.contains("10:00"),
        "docs/02 must state the 10-minute (+20%) scaling step"
    );
    assert!(
        d02.contains("swift end"),
        "docs/02 must describe the post-15:00 swift-end escalation"
    );
}

#[test]
fn protocol_doc_states_the_shipped_arc_and_boss() {
    let d04 = doc("04-protocol-and-messages.md");
    assert!(
        d04.contains(boss_name()),
        "docs/04's match state machine must name the shipped boss ({:?})",
        boss_name()
    );
    assert!(
        d04.contains("15:00") && d04.contains("BOSS_SPAWN_TICK"),
        "docs/04 must place the BOSS transition at 15:00 (BOSS_SPAWN_TICK)"
    );
    assert!(
        !d04.to_lowercase().contains("samwise"),
        "docs/04 describes the shipped protocol; the WC3 source's boss name \
         must not reappear here (source-map analysis belongs in docs/01)"
    );
}

#[test]
fn data_model_doc_states_the_shipped_boss_and_spawn_tick() {
    let d05 = doc("05-data-model.md");
    assert!(
        d05.contains(boss_name()),
        "docs/05 §5.4 must name the shipped boss ({:?})",
        boss_name()
    );
    assert!(
        d05.contains(&BOSS_SPAWN_TICK.to_string()),
        "docs/05 must state the boss spawn tick ({BOSS_SPAWN_TICK})"
    );
    assert!(
        !d05.to_lowercase().contains("samwise"),
        "docs/05 schemas describe the shipped game; the WC3 source's boss \
         name must not reappear here"
    );
    assert!(
        d05.contains("Q47.16"),
        "docs/05 must state the implemented fixed-point format (Q47.16)"
    );
    // §5.7 amendment states the compiled-catalog sizes; derive them so growth
    // in content.rs forces the doc to move.
    let counts = format!("{} weapons / {} modifiers", WEAPONS.len(), MODIFIERS.len());
    assert!(
        d05.contains(&counts) && d05.contains(&format!("{}-entry enemy roster", ENEMIES.len())),
        "docs/05 §5.7 must state the current compiled-catalog sizes \
         ({counts} / {}-entry enemy roster)",
        ENEMIES.len()
    );
}

#[test]
fn roadmap_doc_states_the_shipped_catalog_and_arc() {
    let d06 = doc("06-roadmap-risks-testing.md");
    // Derived, not hardcoded: growing the catalog moves this string, forcing
    // the doc's headline counts to move with it.
    let counts = format!("{} weapons / {} modifiers", WEAPONS.len(), MODIFIERS.len());
    assert!(
        d06.contains(&counts),
        "docs/06 must state the current catalog counts ({counts})"
    );
    assert!(
        d06.contains(boss_name()),
        "docs/06 must name the shipped boss ({:?})",
        boss_name()
    );
    assert!(
        d06.contains("swift end"),
        "docs/06 must describe the post-15:00 swift-end escalation"
    );
    assert!(
        !d06.to_lowercase().contains("samwise"),
        "docs/06 milestones describe the shipped game; the WC3 source's boss \
         name must not reappear here"
    );
}
