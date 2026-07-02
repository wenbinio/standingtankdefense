//! Mechanical drift guard between the wire protocol and `docs/04`.
//!
//! `net::wire::TRANSMITTED_MESSAGES` is the single source of truth for "which
//! messages actually go on the wire". This test fails the build if any of three
//! things drift apart:
//!   1. `TRANSMITTED_MESSAGES` vs the real `Msg` enum variants (1:1, source-parsed).
//!   2. The doc's transmitted-message catalog table vs `TRANSMITTED_MESSAGES`.
//!   3. The doc still describing a name as transmitted that isn't on the wire,
//!      without the explicit "not transmitted" disclaimer.
//!
//! Robust-not-brittle: the doc check is plain substring presence, so prose may
//! evolve freely as long as every real message name still appears.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use net::wire::TRANSMITTED_MESSAGES;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Parse the `Msg` enum variant names directly out of `src/wire.rs` so the test
/// reflects the *actual* enum, not a second hand-maintained list. We scan the
/// `pub enum Msg { ... }` block and take each top-level variant identifier (the
/// first token of a line that begins a variant, ignoring `//` comments, the
/// `enum`/brace lines, and section markers).
fn msg_variants_from_source() -> BTreeSet<String> {
    let src = fs::read_to_string(manifest_dir().join("src/wire.rs")).expect("read src/wire.rs");

    let start = src
        .find("pub enum Msg {")
        .expect("`pub enum Msg {` not found in wire.rs");
    let after = &src[start + "pub enum Msg {".len()..];
    // Find the brace that closes the enum, accounting for the inner `{ ... }`
    // of struct-like variants (e.g. `MatchStart { .. }`).
    let mut depth = 1usize;
    let mut end = after.len();
    for (i, c) in after.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    let body = &after[..end];

    // Each top-level variant is declared on its own line, with all fields inline
    // (e.g. `MatchStart { start_tick: u32, master_seed: u64 },`). A variant line
    // is one whose first identifier is UpperCamel; struct fields are lowercase
    // and so are skipped, and `//` comment / section-marker lines are ignored.
    let mut variants = BTreeSet::new();
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let ident: String = line
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if ident
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false)
        {
            variants.insert(ident);
        }
    }
    assert!(
        !variants.is_empty(),
        "failed to parse any variants from `enum Msg`"
    );
    variants
}

#[test]
fn transmitted_messages_match_msg_enum() {
    let canonical: BTreeSet<String> = TRANSMITTED_MESSAGES.iter().map(|s| s.to_string()).collect();
    let actual = msg_variants_from_source();

    let missing_from_const: Vec<_> = actual.difference(&canonical).collect();
    let missing_from_enum: Vec<_> = canonical.difference(&actual).collect();

    assert!(
        missing_from_const.is_empty(),
        "`Msg` has variants not listed in `TRANSMITTED_MESSAGES`: {missing_from_const:?}. \
         Add them to the const (and to the doc catalog)."
    );
    assert!(
        missing_from_enum.is_empty(),
        "`TRANSMITTED_MESSAGES` names messages that are not `Msg` variants: {missing_from_enum:?}. \
         Remove them from the const or add the variant."
    );

    // No accidental duplicates in the const.
    assert_eq!(
        TRANSMITTED_MESSAGES.len(),
        canonical.len(),
        "`TRANSMITTED_MESSAGES` contains duplicate names"
    );
}

fn protocol_doc() -> String {
    // CARGO_MANIFEST_DIR = sim-core/crates/net ; doc lives at repo `docs/`.
    let path = manifest_dir()
        .join("../../../docs/04-protocol-and-messages.md")
        .canonicalize()
        .expect("canonicalize docs/04 path");
    fs::read_to_string(path).expect("read docs/04-protocol-and-messages.md")
}

#[test]
fn doc_catalogs_every_transmitted_message() {
    let doc = protocol_doc();
    let missing: Vec<&&str> = TRANSMITTED_MESSAGES
        .iter()
        .filter(|name| !doc.contains(**name))
        .collect();
    assert!(
        missing.is_empty(),
        "docs/04 is missing transmitted message name(s) from its catalog: {missing:?}. \
         The doc must name every wire `Msg` variant."
    );
}

#[test]
fn doc_marks_seed_derived_messages_as_not_transmitted() {
    // These appear in the doc as game concepts but are DERIVED from seed+tick
    // and never sent. If the doc names them it must also disclaim them, so a
    // future reader can't mistake them for wire messages.
    let doc = protocol_doc();
    for derived in ["RoundStart", "ShopOffer", "BossSpawn"] {
        if doc.contains(derived) {
            assert!(
                doc.contains("DERIVED"),
                "docs/04 mentions seed-derived `{derived}` but is missing the \
                 explicit \"DERIVED ... not transmitted\" disclaimer."
            );
        }
    }
}
