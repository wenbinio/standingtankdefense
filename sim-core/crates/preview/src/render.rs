//! Text renderer for the live `ArenaState`. Reads only public sim state — this
//! is exactly the data a Godot front-end will pull each frame to draw the arena,
//! HUD, and shop. Kept engine-agnostic and allocation-light.

use sim::{content, ArenaState, OfferKind};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Arena glyph grid size (characters).
const GRID_W: usize = 49;
const GRID_H: usize = 19;
/// World half-extent (units). The spawn ring sits at 1500, so 1600 leaves a rim.
const WORLD: i64 = 1600;

/// Map a world coordinate to a grid cell (y points up). Returns `None` if off-grid.
fn to_cell(x: i64, y: i64) -> Option<(usize, usize)> {
    if !(-WORLD..=WORLD).contains(&x) || !(-WORLD..=WORLD).contains(&y) {
        return None;
    }
    let col = ((x + WORLD) * (GRID_W as i64 - 1) / (2 * WORLD)) as usize;
    let row = ((WORLD - y) * (GRID_H as i64 - 1) / (2 * WORLD)) as usize;
    Some((col.min(GRID_W - 1), row.min(GRID_H - 1)))
}

/// Compose one full frame for the given state, with a recent-event log.
pub fn frame(s: &ArenaState, log: &[String]) -> String {
    let mut grid = [[' '; GRID_W]; GRID_H];

    // Projectiles first (lowest priority), then enemies, then the tank on top.
    for p in &s.projectiles {
        if let Some((c, r)) = to_cell(p.pos.x.floor_to_int(), p.pos.y.floor_to_int()) {
            grid[r][c] = '.';
        }
    }
    for e in &s.enemies {
        if let Some((c, r)) = to_cell(e.pos.x.floor_to_int(), e.pos.y.floor_to_int()) {
            let edef = &content::ENEMIES[e.def as usize];
            grid[r][c] = if edef.boss {
                'B'
            } else {
                // First letter of the enemy name, lowercased, as its glyph.
                edef.name.chars().next().unwrap_or('o').to_ascii_lowercase()
            };
        }
    }
    if let Some((c, r)) = to_cell(s.tank.pos.x.floor_to_int(), s.tank.pos.y.floor_to_int()) {
        grid[r][c] = '@';
    }

    let inner = GRID_W; // box inner width is the grid width
    let mut out = String::with_capacity(GRID_H * (GRID_W + 4) + 1024);

    let bar = |hp: i64, max: i64, width: usize| -> String {
        let filled = if max > 0 {
            ((hp.max(0) as i128 * width as i128) / max as i128) as usize
        } else {
            0
        };
        let filled = filled.min(width);
        format!("{}{}", "#".repeat(filled), "-".repeat(width - filled))
    };

    let secs = s.tick / sim::TICK_HZ;
    let status = if s.dead { "DESTROYED" } else { "ALIVE" };

    // ---- header / HUD -------------------------------------------------------
    let _ = writeln!(out, "+{}+", "=".repeat(inner));
    let _ = writeln!(out, "|{:^width$}|", "STANDING TANK DEFENSE — live preview", width = inner);
    let _ = writeln!(out, "+{}+", "-".repeat(inner));
    let _ = writeln!(
        out,
        "| {:<width$}|",
        format!(
            "t={:02}:{:02}  tick {}  round {}  [{}]",
            secs / 60, secs % 60, s.tick, s.round, status
        ),
        width = inner - 1
    );
    let _ = writeln!(
        out,
        "| {:<width$}|",
        format!(
            "HP [{}] {}/{}",
            bar(s.tank.hp, s.tank.max_hp, 18),
            s.tank.hp.max(0),
            s.tank.max_hp
        ),
        width = inner - 1
    );
    let revives = if s.tank.revives > 0 {
        format!("  revives {}", s.tank.revives)
    } else {
        String::new()
    };
    let _ = writeln!(
        out,
        "| {:<width$}|",
        format!(
            "gold {} (+{}/t)  weapons {}  enemies {}{}",
            s.economy.gold,
            s.economy.income_mult.scale_i64(s.economy.income_per_tick),
            s.weapons.len(),
            s.enemies.len(),
            revives
        ),
        width = inner - 1
    );

    // ---- arena --------------------------------------------------------------
    let _ = writeln!(out, "+{}+", "-".repeat(inner));
    for row in &grid {
        let line: String = row.iter().collect();
        let _ = writeln!(out, "|{}|", line);
    }

    // ---- shop ---------------------------------------------------------------
    let _ = writeln!(out, "+{} SHOP {}+", "-".repeat(2), "-".repeat(inner - 8));
    for (i, off) in s.shop.offers.iter().enumerate() {
        let (name, owned) = match off.kind {
            OfferKind::Weapon => (content::WEAPONS[off.def as usize].name, ""),
            OfferKind::Modifier => (content::MODIFIERS[off.def as usize].name, ""),
        };
        let mark = if off.cost > s.economy.gold { "x" } else { "$" };
        let _ = writeln!(
            out,
            "| {:<width$}|",
            format!("[{}] {:<28} {:>6}g {}{}", i, trunc(name, 28), off.cost, mark, owned),
            width = inner - 1
        );
    }

    // ---- arsenal (counts by weapon) ----------------------------------------
    let _ = writeln!(out, "+{} ARSENAL {}+", "-".repeat(2), "-".repeat(inner - 11));
    let mut counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    for w in &s.weapons {
        *counts.entry(content::WEAPONS[w.def as usize].name).or_insert(0) += 1;
    }
    let arsenal = if counts.is_empty() {
        "(none)".to_string()
    } else {
        counts
            .iter()
            .map(|(n, c)| format!("{}x{}", trunc(n, 14), c))
            .collect::<Vec<_>>()
            .join("  ")
    };
    for line in wrap(&arsenal, inner - 2) {
        let _ = writeln!(out, "| {:<width$}|", line, width = inner - 1);
    }

    // ---- event log ----------------------------------------------------------
    let _ = writeln!(out, "+{} LOG {}+", "-".repeat(2), "-".repeat(inner - 7));
    for line in log {
        let _ = writeln!(out, "| {:<width$}|", trunc(line, inner - 2), width = inner - 1);
    }
    let _ = writeln!(out, "+{}+", "=".repeat(inner));
    out
}

/// Truncate a string to `n` display chars (ASCII), adding nothing.
fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

/// Word-wrap to `width` columns (greedy, whitespace-split).
fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if !cur.is_empty() && cur.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}
