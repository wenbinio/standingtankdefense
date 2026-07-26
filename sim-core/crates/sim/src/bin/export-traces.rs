//! Generates the R3 oracle trace corpus (`roblox/SIM-SPEC.md` §S6).
//!
//! The Luau transcription is *done* when its per-tick checksum trace equals the
//! Rust's for every seed here. These files are that Rust side.
//!
//! ```text
//! cargo run -p sim --bin export-traces                        # write the corpus
//! cargo run -p sim --bin export-traces -- --check             # fail if stale
//! cargo run -p sim --bin export-traces -- --list              # index, no I/O
//! cargo run -p sim --bin export-traces -- --verbose 0 54000   # dump tick 54000 of seed 0
//! cargo run -p sim --bin export-traces -- --verbose 0 54000 --filter enemies[3]
//! cargo run -p sim --bin export-traces -- <repo-root>         # write under another root
//! ```
//!
//! Writes `roblox/test/traces/<name>.json` for every entry in
//! `export::trace::TRACES`, plus `roblox/test/traces/MANIFEST.json`.
//!
//! Deterministic: running it twice produces byte-identical files.
//!
//! **`--verbose <seed> <tick>` is the divergence-localizer.** It replays the
//! committed trace's own input log to that tick and prints every field
//! `checksum()` folds, one per line, as `path<TAB>kind<TAB>hex<TAB>decimal`.
//! Dump the same thing from the Luau side and `diff` the two: the first
//! differing line is the bug. Beats bisecting 55,200 ticks by hand.
//!
//! `--scan` is how the corpus was *chosen*: it sweeps seeds and reports which
//! sim behaviours each one reaches, so the committed set can be picked to leave
//! no phase untested rather than hoped into coverage.

use sim::bot::Challenge;
use sim::export::json::Value;
use sim::export::repo_root;
use sim::export::trace::{
    self, checksum_of_fields, fields, input_name, replay_to, TraceSpec, TRACES,
};
use sim::Input;
use std::collections::BTreeMap;
use std::path::PathBuf;

const USAGE: &str = "\
usage:
  export-traces [<repo-root>]                    write the trace corpus
  export-traces --check [<repo-root>]            exit 1 if any committed file is stale
  export-traces --list                           print the corpus index (no I/O)
  export-traces --verbose <seed> <tick> [opts]   dump every checksummed field after <tick>

  --verbose options:
    --filter <substr>   only print fields whose path contains <substr>
    --ticks <n>         override the trace length used for the <tick> bounds check

  export-traces --scan <seeds> <ticks> [<challenge-code>]
                        coverage sweep used to CHOOSE corpus seeds: runs each seed
                        and prints which sim behaviours it reached. Writes nothing.
";

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }
    if args.iter().any(|a| a == "--list") {
        list();
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--verbose") {
        return verbose(&args[i + 1..]);
    }
    if let Some(i) = args.iter().position(|a| a == "--scan") {
        scan(&args[i + 1..]);
        return Ok(());
    }

    let mut check = false;
    let mut root: Option<PathBuf> = None;
    for arg in &args {
        match arg.as_str() {
            "--check" => check = true,
            other => root = Some(PathBuf::from(other)),
        }
    }
    let root = root.unwrap_or_else(repo_root);

    let mut stale = Vec::new();
    for (rel, body) in trace::all_traces() {
        let path = root.join(&rel);
        let current = std::fs::read_to_string(&path).ok();
        let matches = current.as_deref() == Some(body.as_str());
        if check {
            if !matches {
                stale.push(rel);
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

    if check {
        if !stale.is_empty() {
            eprintln!("stale traces (re-run without --check): {stale:?}");
            std::process::exit(1);
        }
        println!("traces up to date ({} files)", TRACES.len() + 1);
    }
    println!("content_hash = {:016x}", sim::export::content_hash());
    Ok(())
}

fn list() {
    println!("{:<20}  {:>6}  {:>7}  {:<12}  what it exercises", "name", "seed", "ticks", "bot");
    for t in TRACES {
        println!(
            "{:<20}  {:>6}  {:>7}  {:<12}  {}",
            t.name,
            t.seed,
            t.ticks,
            trace::challenge_name(t.challenge),
            t.exercises
        );
    }
    println!("\ncontent_hash = {:016x}", sim::export::content_hash());
}

/// `--scan <seeds> <ticks> [<challenge-code>]` — the tool used to *pick* the
/// corpus. Prints one row per seed showing which behaviours it reached, plus a
/// union row: any column that is empty in the union is a phase the corpus cannot
/// test, and the corpus should be re-picked until it fills.
fn scan(rest: &[String]) {
    let seeds: u64 = rest.first().and_then(|v| v.parse().ok()).unwrap_or(32);
    let ticks: u32 = rest.get(1).and_then(|v| v.parse().ok()).unwrap_or(12000);
    let code: i64 = rest.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
    let challenge = Challenge::from_code(code);

    let head = trace::Coverage::default();
    print!("{:>5} {:>8} {:>5} {:>5} {:>5}", "seed", "death", "buys", "wpn", "mod");
    for (n, _) in head.flags() {
        print!(" {:>9}", n);
    }
    println!();

    let mut union = vec![false; head.flags().len()];
    for seed in 0..seeds {
        let c = trace::scan(seed, ticks, challenge);
        print!(
            "{:>5} {:>8} {:>5} {:>5} {:>5}",
            seed,
            c.death_tick.map(|t| t.to_string()).unwrap_or_else(|| "-".into()),
            c.buys,
            c.weapon_buys,
            c.modifier_buys
        );
        for (i, (_, v)) in c.flags().iter().enumerate() {
            union[i] |= *v;
            print!(" {:>9}", if *v { "X" } else { "." });
        }
        println!();
    }
    print!("{:>5} {:>8} {:>5} {:>5} {:>5}", "UNION", "", "", "", "");
    for (i, (_, _)) in head.flags().iter().enumerate() {
        print!(" {:>9}", if union[i] { "X" } else { "MISSING" });
    }
    println!(
        "\n\nbot = {} (challenge code {code}); {ticks} ticks/seed",
        trace::challenge_name(challenge)
    );
}

/// A committed trace, read back so `--verbose` can explain the *file* rather
/// than a fresh bot run.
struct Committed {
    log: BTreeMap<u32, Input>,
    checksums: Vec<String>,
}

/// Read and parse the committed trace for `spec`, if it is on disk and valid.
/// A missing or malformed file is not fatal — `--verbose` falls back to a
/// bot-driven replay and says so.
fn read_committed(spec: &TraceSpec) -> Option<Committed> {
    let src = std::fs::read_to_string(repo_root().join(trace::trace_path(spec))).ok()?;
    let doc = Value::parse(&src).ok()?;
    let mut log = BTreeMap::new();
    for e in doc.get("inputs")?.as_arr() {
        let inp = trace::input_from_code(e.get("code")?.as_i64())?;
        log.insert(e.get("tick")?.as_i64() as u32, inp);
    }
    let checksums =
        doc.get("checksums")?.as_arr().iter().map(|v| v.as_str().to_string()).collect();
    Some(Committed { log, checksums })
}

/// `--verbose <seed> <tick> [--filter <substr>] [--ticks <n>]`.
fn verbose(rest: &[String]) -> std::io::Result<()> {
    let mut positional: Vec<&String> = Vec::new();
    let mut filter: Option<String> = None;
    let mut ticks_override: Option<u32> = None;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--filter" => filter = it.next().cloned(),
            "--ticks" => ticks_override = it.next().and_then(|v| v.parse().ok()),
            _ => positional.push(a),
        }
    }
    if positional.len() < 2 {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let seed: u64 = match positional[0].parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("bad seed {:?}", positional[0]);
            std::process::exit(2);
        }
    };
    let tick: u32 = match positional[1].parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("bad tick {:?}", positional[1]);
            std::process::exit(2);
        }
    };

    let spec: Option<&TraceSpec> = TRACES.iter().find(|t| t.seed == seed);
    let corpus_ticks = ticks_override.or(spec.map(|t| t.ticks));
    if let Some(n) = corpus_ticks {
        if tick >= n {
            eprintln!(
                "tick {tick} is past the end of seed {seed}'s trace ({n} ticks, last index {})",
                n - 1
            );
            std::process::exit(2);
        }
    }

    // `checksums[tick]` is taken AFTER tick `tick` completes, i.e. after `tick+1`
    // steps have run. Replaying to `tick + 1` reproduces exactly that state.
    //
    // Prefer replaying the COMMITTED input log — it is what the Luau side does,
    // and it stays correct for traces whose bot ran under a challenge. Fall back
    // to re-running the bot only when there is no file to read.
    let committed = spec.and_then(read_committed);
    let (st, last_input, cs) = match &committed {
        Some(c) => trace::replay_inputs(seed, tick + 1, &c.log),
        None => replay_to(seed, tick + 1, spec.map(|t| t.challenge).unwrap_or(Challenge::None)),
    };
    let all = fields(&st);
    assert_eq!(
        checksum_of_fields(&all),
        cs,
        "field dump does not re-fold to checksum() — the dump has drifted; fix export::trace::fields"
    );

    println!("# seed          {seed}");
    println!("# player_id     {}", trace::PLAYER_ID);
    println!("# tick          {tick}   (state AFTER this tick completed = checksums[{tick}])");
    println!("# input@tick    {}  (code {})", input_name(last_input), trace::input_code(last_input));
    println!("# checksum      {cs:016x}");
    println!("# content_hash  {:016x}", sim::export::content_hash());
    match spec {
        Some(t) => println!(
            "# trace         {} ({} ticks, bot {})",
            t.name,
            t.ticks,
            trace::challenge_name(t.challenge)
        ),
        None => println!("# trace         (seed not in the corpus; replayed with the default bot)"),
    }
    match &committed {
        Some(c) => {
            let want = c.checksums.get(tick as usize).map(String::as_str).unwrap_or("<missing>");
            let ok = want == format!("{cs:016x}");
            println!(
                "# committed     {want}  ({})",
                if ok { "MATCHES — the file is fresh" } else { "STALE — re-run export-traces" }
            );
        }
        None => println!("# committed     (no file on disk; nothing to cross-check)"),
    }
    println!("# fields        {}", all.len());
    println!("# format        path<TAB>kind<TAB>hex<TAB>decimal");
    if let Some(f) = &filter {
        println!("# filter        {f:?}");
    }
    println!("#");

    let mut shown = 0usize;
    for f in &all {
        if filter.as_ref().is_some_and(|q| !f.path.contains(q.as_str())) {
            continue;
        }
        println!("{}", f.line());
        shown += 1;
    }
    if shown == 0 {
        eprintln!("no field matched the filter; run without --filter to see all {}", all.len());
        std::process::exit(1);
    }
    Ok(())
}
