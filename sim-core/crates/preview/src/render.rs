//! Text renderer driven by [`sim::view::RenderView`] — the engine-agnostic
//! snapshot the Godot front-end also consumes. Proves the view contract carries
//! everything a renderer needs. Allocation-light; no sim internals touched.

use sim::view::RenderView;
use std::fmt::Write as _;

/// Arena glyph grid size (characters).
const GRID_W: usize = 49;
const GRID_H: usize = 19;
/// World half-extent (units). The spawn ring sits at 1500, so 1600 leaves a rim.
const WORLD: i64 = 1600;

/// Map a world coordinate to a grid cell (y points up). `None` if off-grid.
fn to_cell(x: i64, y: i64) -> Option<(usize, usize)> {
    if !(-WORLD..=WORLD).contains(&x) || !(-WORLD..=WORLD).contains(&y) {
        return None;
    }
    let col = ((x + WORLD) * (GRID_W as i64 - 1) / (2 * WORLD)) as usize;
    let row = ((WORLD - y) * (GRID_H as i64 - 1) / (2 * WORLD)) as usize;
    Some((col.min(GRID_W - 1), row.min(GRID_H - 1)))
}

/// Compose one full frame from a view snapshot and a recent-event log.
pub fn frame(v: &RenderView, log: &[String]) -> String {
    let mut grid = [[' '; GRID_W]; GRID_H];

    // Projectiles (lowest priority), then enemies, then the tank on top.
    for p in &v.projectiles {
        if let Some((c, r)) = to_cell(p.x, p.y) {
            grid[r][c] = '.';
        }
    }
    for e in &v.enemies {
        if let Some((c, r)) = to_cell(e.x, e.y) {
            grid[r][c] = if e.boss {
                'B'
            } else {
                e.name.chars().next().unwrap_or('o').to_ascii_lowercase()
            };
        }
    }
    if let Some((c, r)) = to_cell(v.tank.x, v.tank.y) {
        grid[r][c] = '@';
    }

    let inner = GRID_W;
    let mut out = String::with_capacity(GRID_H * (GRID_W + 4) + 1024);

    let bar = |hp: i64, max: i64, width: usize| -> String {
        let filled = if max > 0 {
            (((hp.max(0) as i128) * width as i128) / max as i128) as usize
        } else {
            0
        };
        let filled = filled.min(width);
        format!("{}{}", "#".repeat(filled), "-".repeat(width - filled))
    };

    let secs = v.tick / sim::TICK_HZ;
    let status = if v.dead { "DESTROYED" } else { "ALIVE" };

    // ---- header / HUD -------------------------------------------------------
    let _ = writeln!(out, "+{}+", "=".repeat(inner));
    let _ = writeln!(out, "|{:^width$}|", "STANDING TANK DEFENSE — live preview", width = inner);
    let _ = writeln!(out, "+{}+", "-".repeat(inner));
    let _ = writeln!(
        out,
        "| {:<width$}|",
        format!("t={:02}:{:02}  tick {}  round {}  [{}]", secs / 60, secs % 60, v.tick, v.round, status),
        width = inner - 1
    );
    let _ = writeln!(
        out,
        "| {:<width$}|",
        format!("HP [{}] {}/{}", bar(v.tank.hp, v.tank.max_hp, 18), v.tank.hp.max(0), v.tank.max_hp),
        width = inner - 1
    );
    let revives = if v.tank.revives > 0 {
        format!("  revives {}", v.tank.revives)
    } else {
        String::new()
    };
    let _ = writeln!(
        out,
        "| {:<width$}|",
        format!(
            "gold {} (+{}/t)  weapons {}  enemies {}{}",
            v.economy.gold,
            v.economy.income_per_tick,
            v.arsenal.iter().map(|a| a.count).sum::<u32>(),
            v.enemies.len(),
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
    for off in &v.shop {
        let mark = if off.affordable { "$" } else { "x" };
        let _ = writeln!(
            out,
            "| {:<width$}|",
            format!("[{}] {:<28} {:>6}g {}", off.slot, trunc(off.name, 28), off.cost, mark),
            width = inner - 1
        );
    }

    // ---- arsenal ------------------------------------------------------------
    let _ = writeln!(out, "+{} ARSENAL {}+", "-".repeat(2), "-".repeat(inner - 11));
    let arsenal = if v.arsenal.is_empty() {
        "(none)".to_string()
    } else {
        v.arsenal
            .iter()
            .map(|a| format!("{}x{}", trunc(a.name, 14), a.count))
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

/// Truncate to `n` display chars (ASCII), appending an ellipsis when cut.
fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

/// Greedy word-wrap to `width` columns.
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
