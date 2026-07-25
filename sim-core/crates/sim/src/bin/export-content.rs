//! Regenerates the Roblox fork's build artifacts from the Rust tables.
//!
//! ```text
//! cargo run -p sim --bin export-content            # writes into the repo
//! cargo run -p sim --bin export-content -- --check # fails if they are stale
//! cargo run -p sim --bin export-content -- <dir>   # write under another root
//! ```
//!
//! Writes (relative to the repo root):
//! - `roblox/src/shared/content.json`          (C1)
//! - `roblox/test/vectors/fixed_vectors.json`  (C4)
//! - `roblox/test/vectors/rng_vectors.json`    (C4)
//!
//! Deterministic: running it twice produces byte-identical files.

use sim::export::{all_artifacts, repo_root, OUTPUTS};
use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let mut check = false;
    let mut root: Option<PathBuf> = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--check" => check = true,
            "-h" | "--help" => {
                println!("usage: export-content [--check] [<repo-root>]");
                return Ok(());
            }
            other => root = Some(PathBuf::from(other)),
        }
    }
    let root = root.unwrap_or_else(repo_root);

    let mut stale = Vec::new();
    for (rel, body) in OUTPUTS.iter().zip(all_artifacts()) {
        let path = root.join(rel);
        let current = std::fs::read_to_string(&path).ok();
        let matches = current.as_deref() == Some(body.as_str());
        if check {
            if !matches {
                stale.push(*rel);
            }
            continue;
        }
        if matches {
            println!("  unchanged  {rel}");
            continue;
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, &body)?;
        println!("  wrote      {rel}  ({} bytes)", body.len());
    }

    if check && !stale.is_empty() {
        eprintln!("stale artifacts (re-run without --check): {stale:?}");
        std::process::exit(1);
    }
    println!("content_hash = {:016x}", sim::export::content_hash());
    Ok(())
}
