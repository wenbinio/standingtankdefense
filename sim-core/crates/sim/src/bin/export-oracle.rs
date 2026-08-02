//! Regenerates the `modifiers`/`economy` oracle consumed by
//! `roblox/test/modifiers_test.luau` (`roblox/CONTRACTS.md` C4).
//!
//! ```text
//! cargo run -p sim --bin export-oracle            # writes into the repo
//! cargo run -p sim --bin export-oracle -- --check # fails if it is stale
//! cargo run -p sim --bin export-oracle -- <dir>   # write under another root
//! ```
//!
//! Writes (relative to the repo root):
//! - `roblox/test/OracleData.luau`  (C4 — the whole oracle document)
//!
//! Deterministic: running it twice produces a byte-identical file.
//!
//! The generator itself is `sim::export::oracle`, INSIDE the lib — that is the
//! point. `modifiers::apply_ramps` and every `economy::*` entry point are
//! `pub(crate)`, so the previous out-of-repo generator could not reach them and
//! `ramp_cases`/`econ_cases` had to be carried forward by hand between passes. They
//! went stale exactly as you would expect. Nothing here widens a visibility; the
//! generator simply moved to where the crate already lets it call.

use sim::export::oracle::{oracle_data_luau, ORACLE_OUTPUT};
use sim::export::repo_root;
use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let mut check = false;
    let mut root: Option<PathBuf> = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--check" => check = true,
            "-h" | "--help" => {
                println!("usage: export-oracle [--check] [<repo-root>]");
                return Ok(());
            }
            other => root = Some(PathBuf::from(other)),
        }
    }
    let root = root.unwrap_or_else(repo_root);

    let body = oracle_data_luau();
    let path = root.join(ORACLE_OUTPUT);
    let current = std::fs::read_to_string(&path).ok();
    let matches = current.as_deref() == Some(body.as_str());

    if check {
        if !matches {
            eprintln!("stale oracle (re-run without --check): {ORACLE_OUTPUT}");
            std::process::exit(1);
        }
        println!("oracle up to date");
    } else if matches {
        println!("  unchanged  {ORACLE_OUTPUT}");
    } else {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, &body)?;
        println!("  wrote      {ORACLE_OUTPUT}  ({} bytes)", body.len());
    }

    println!("content_hash = {:016x}", sim::export::content_hash());
    Ok(())
}
