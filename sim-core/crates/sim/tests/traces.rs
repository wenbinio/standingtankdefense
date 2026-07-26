//! The oracle's own guard rails (`roblox/SIM-SPEC.md` §S6).
//!
//! Seven agents are transcribing this crate into Luau against
//! `roblox/test/traces/`. A trace that silently stops describing what `step()`
//! actually does is worse than no trace at all — it would make a *correct* Luau
//! port look broken (or, worse, bless a wrong one). These tests are what stops
//! that:
//!
//! - [`committed_traces_replay_bit_identically_through_step`] takes the
//!   **committed files** and replays their input logs through the real
//!   `step()`/`checksum()`, asserting every one of the ~600k checksums. Any
//!   change to simulation behaviour fails here.
//! - [`committed_traces_are_fresh`] regenerates the corpus and byte-compares,
//!   so a behaviour change cannot be "fixed" by leaving stale files in place.
//! - [`manifest_describes_the_committed_corpus`] keeps the index honest and
//!   fails if the corpus stops covering a sim behaviour.

use sim::export::json::Value;
use sim::export::trace::{
    all_traces, input_from_code, manifest_path, trace_path, Coverage, PLAYER_ID, SCHEMA_VERSION,
    TRACES,
};
use sim::export::{content_hash, repo_root};
use sim::{checksum, step, ArenaState, Input};
use std::collections::BTreeMap;

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}\n\
             run `cargo run -p sim --bin export-traces` to generate the corpus",
            path.display()
        )
    })
}

/// Parse a committed trace. `Value::parse` rejects any number carrying `.`/`e`,
/// so a successful parse is also the "no floats in the trace" proof.
fn parse(rel: &str) -> Value {
    Value::parse(&read(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// THE gate. Replays every committed trace's input log through the real
/// `step()` and asserts the checksum after each tick, exactly as the Luau side
/// will. Nothing here calls the bot: the file must be self-contained.
#[test]
fn committed_traces_replay_bit_identically_through_step() {
    let want_hash = format!("{:016x}", content_hash());
    for spec in TRACES {
        let rel = trace_path(spec);
        let doc = parse(&rel);

        assert_eq!(doc.at("schema_version").as_i64(), SCHEMA_VERSION, "{rel}: schema_version");
        assert_eq!(doc.at("seed").as_i64() as u64, spec.seed, "{rel}: seed");
        assert_eq!(
            doc.at("content_hash").as_str(),
            want_hash,
            "{rel}: content_hash is stale — the catalog moved under the trace; \
             re-run `cargo run -p sim --bin export-traces`"
        );

        // The input log is SPARSE: unlisted ticks are Noop.
        let mut inputs: BTreeMap<u32, Input> = BTreeMap::new();
        for e in doc.at("inputs").as_arr() {
            let tick = e.at("tick").as_i64();
            let code = e.at("code").as_i64();
            let inp = input_from_code(code)
                .unwrap_or_else(|| panic!("{rel}: tick {tick} has unknown input code {code}"));
            assert!(
                tick >= 0 && (tick as u32) < spec.ticks,
                "{rel}: input tick {tick} is outside 0..{}",
                spec.ticks
            );
            assert!(
                inputs.insert(tick as u32, inp).is_none(),
                "{rel}: duplicate input at tick {tick}"
            );
            assert_ne!(inp, Input::Noop, "{rel}: tick {tick} — a sparse log must not list Noop");
        }

        let expected = doc.at("checksums").as_arr();
        assert_eq!(expected.len(), spec.ticks as usize, "{rel}: checksum count");

        let mut st = ArenaState::new(spec.seed, PLAYER_ID);
        for tick in 0..spec.ticks {
            let inp = inputs.get(&tick).copied().unwrap_or(Input::Noop);
            step(&mut st, inp);
            let got = format!("{:016x}", checksum(&st));
            assert_eq!(
                got,
                expected[tick as usize].as_str(),
                "{rel}: checksum diverged at tick {tick}\n\
                 localize it with: cargo run -p sim --bin export-traces -- --verbose {} {tick}",
                spec.seed
            );
        }
    }
}

/// The anti-rot gate: what `export-traces` would write today must be exactly
/// what is committed. Equivalent to `export-traces --check`, run in CI by
/// `cargo test`. It also proves generation is stable *across processes*, since
/// the committed bytes came from an earlier run.
#[test]
fn committed_traces_are_fresh() {
    let stale: Vec<String> = all_traces()
        .into_iter()
        .filter(|(rel, body)| {
            std::fs::read_to_string(repo_root().join(rel)).ok().as_deref() != Some(body.as_str())
        })
        .map(|(rel, _)| rel)
        .collect();
    assert!(
        stale.is_empty(),
        "stale trace artifacts: {stale:?}\n\
         re-run `cargo run -p sim --bin export-traces`"
    );
}

/// The manifest must index the real corpus, and must not admit a coverage hole:
/// an empty `coverage_union` entry is a sim behaviour the oracle cannot test, so
/// a Luau bug there would sail through the gate.
#[test]
fn manifest_describes_the_committed_corpus() {
    let m = parse(&manifest_path());
    assert_eq!(m.at("schema_version").as_i64(), SCHEMA_VERSION);
    assert_eq!(m.at("content_hash").as_str(), format!("{:016x}", content_hash()));
    assert_eq!(m.at("player_id").as_i64(), PLAYER_ID as i64);

    let listed = m.at("traces").as_arr();
    assert_eq!(listed.len(), TRACES.len(), "manifest lists a different number of traces");
    for (entry, spec) in listed.iter().zip(TRACES) {
        assert_eq!(entry.at("name").as_str(), spec.name);
        assert_eq!(entry.at("seed").as_i64() as u64, spec.seed);
        assert_eq!(entry.at("ticks").as_i64() as u32, spec.ticks);
        assert_eq!(
            entry.at("file").as_str(),
            format!("{}.json", spec.name),
            "manifest file name must match the trace on disk"
        );
    }

    let missing: Vec<&str> = m.at("coverage_missing").as_arr().iter().map(|v| v.as_str()).collect();
    assert!(
        missing.is_empty(),
        "the corpus exercises no trace covering: {missing:?} — \
         re-pick seeds with `export-traces --scan <seeds> <ticks> [<challenge>]`"
    );
    let union = m.at("coverage_union");
    for (name, _) in Coverage::default().flags() {
        assert!(union.at(name).as_bool(), "coverage_union.{name} is false");
    }
}
