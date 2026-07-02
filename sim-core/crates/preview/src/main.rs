//! Standing Tank Defense — a runnable, self-playing preview of the deterministic
//! sim core. It drives the real `sim::step` with a bot's `Input`s and renders the
//! public arena state as text — the same read-only surface the Godot front-end
//! will consume. Two modes:
//!
//!   preview                 # headless: prints a few spaced frames, then a summary
//!   preview --watch         # live: clears the screen each frame at ~30 Hz
//!
//! Flags: --seed <u64>  --every <ticks>  --frames <n>  --max-ticks <n>  --speed <n>

mod render;

use sim::bot::Bot;
use sim::{content, step, ArenaState, Input, OfferKind};

struct Args {
    watch: bool,
    seed: u64,
    every: u32,
    frames: u32,
    max_ticks: u32,
    speed: u32,
}

fn parse_args() -> Args {
    let mut a = Args {
        watch: false,
        seed: 0xA5A5_1234_DEAD_BEEF,
        every: 0, // 0 ⇒ pick a per-mode default below
        frames: 0,
        max_ticks: content::BOSS_SPAWN_TICK + 600, // run a bit past the boss
        speed: 1,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = || it.next().and_then(|v| v.parse::<u64>().ok());
        match arg.as_str() {
            "--watch" => a.watch = true,
            "--seed" => a.seed = val().unwrap_or(a.seed),
            "--every" => a.every = val().map(|v| v as u32).unwrap_or(0),
            "--frames" => a.frames = val().map(|v| v as u32).unwrap_or(0),
            "--max-ticks" => a.max_ticks = val().map(|v| v as u32).unwrap_or(a.max_ticks),
            "--speed" => a.speed = val().map(|v| v as u32).unwrap_or(1),
            "-h" | "--help" => {
                println!("preview [--watch] [--seed N] [--every T] [--frames K] [--max-ticks N] [--speed N]");
                std::process::exit(0);
            }
            _ => {}
        }
    }
    if a.every == 0 {
        a.every = if a.watch { 2 } else { 300 };
    }
    if a.frames == 0 {
        a.frames = if a.watch { u32::MAX } else { 7 };
    }
    a
}

fn main() {
    let args = parse_args();
    let mut s = ArenaState::new(args.seed, 0);
    let mut ai = Bot::default();
    let mut log: Vec<String> = vec!["match started".to_string()];

    let mut prev_round = s.round;
    let mut prev_weapons = s.weapons.len();
    let mut prev_boss = false;
    let mut frames_shown = 0u32;

    let push = |log: &mut Vec<String>, line: String| {
        log.push(line);
        let n = log.len();
        if n > 6 {
            log.drain(0..n - 6);
        }
    };

    for _ in 0..args.max_ticks {
        // Decide and apply one tick.
        let action = ai.decide(&s);
        let pre_gold = s.economy.gold;
        let pre_offers = s.shop.offers.clone();
        step(&mut s, action);

        // ---- detect notable events for the log ----
        if s.round > prev_round {
            push(&mut log, format!("round {}: new shop", s.round));
            prev_round = s.round;
        }
        if let Input::BuyOffer { slot } = action {
            let bought = s.weapons.len() > prev_weapons || s.economy.gold < pre_gold;
            if bought {
                if let Some(off) = pre_offers.get(slot as usize) {
                    let name = match off.kind {
                        OfferKind::Weapon => content::WEAPONS[off.def as usize].name,
                        OfferKind::Modifier => content::MODIFIERS[off.def as usize].name,
                    };
                    push(&mut log, format!("bought {}", name));
                }
            }
        } else if action == Input::Clear {
            push(&mut log, "CLEAR! board wiped".to_string());
        }
        prev_weapons = s.weapons.len();

        let boss_now = s
            .enemies
            .iter()
            .any(|e| content::ENEMIES[e.def as usize].boss);
        if boss_now && !prev_boss {
            push(
                &mut log,
                "*** BOSS: The Hippocrate has arrived! Bedside manner: terminal. ***".to_string(),
            );
        }
        prev_boss = boss_now;

        // ---- render on cadence ----
        if s.tick.is_multiple_of(args.every) {
            let f = render::frame(&sim::view::snapshot(&s), &log);
            if args.watch {
                print!("\x1b[2J\x1b[H{}", f);
                use std::io::Write;
                let _ = std::io::stdout().flush();
                std::thread::sleep(std::time::Duration::from_millis(
                    (1000 / sim::TICK_HZ * args.every / args.speed.max(1)) as u64,
                ));
            } else {
                println!("{}", f);
                frames_shown += 1;
                if frames_shown >= args.frames {
                    break;
                }
            }
        }

        if s.dead {
            let secs = s.tick / sim::TICK_HZ;
            push(
                &mut log,
                format!("TANK DESTROYED at {:02}:{:02}", secs / 60, secs % 60),
            );
            // Show one final frame, then stop.
            println!("{}", render::frame(&sim::view::snapshot(&s), &log));
            break;
        }
    }

    // Headless summary line (handy for CI / quick sanity).
    if !args.watch {
        let secs = s.tick / sim::TICK_HZ;
        println!(
            "summary: seed={:#x} ticks={} time={:02}:{:02} gold={} weapons={} enemies={} status={}",
            args.seed,
            s.tick,
            secs / 60,
            secs % 60,
            s.economy.gold,
            s.weapons.len(),
            s.enemies.len(),
            if s.dead { "DEAD" } else { "ALIVE" },
        );
    }
}
