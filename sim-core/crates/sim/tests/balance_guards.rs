//! BALANCE-PASS GUARD TESTS — codify the early-game design window (`docs/06`).
//!
//! Two deterministic scenarios pin the two ends of the "early grace, but punish the
//! naked eco-rush" target:
//!   (A) a NAKED eco-rush tank (no weapons, no defense items — only economy buys +
//!       the free Clear) is DEAD by tick ≤ 3600 (≤ 2 min @ 30 Hz);
//!   (B) a MODEST opener (a weapon or two + some HP/armor) is ALIVE past tick 3600.
//! The separating lever is OFFENSE: the early wave throughput sits just above what a
//! weaponless tank can survive on Clear alone, but well within what even a small
//! arsenal holds. All scenarios are pure functions of the seed (deterministic).
//!
//! NOTE on the naked case — was RED, now GREEN, and the cause was measured, not guessed.
//! The naked guard used to fail because of **income-as-HP-regen**: `Entangled Gold Mine`
//! (MODIFIERS[7]) carries `IncomeRegenPct(25, 100)`, which heals a fraction of EVERY
//! income award, stacks additively per copy with no cap, and is an ECONOMY modifier — so
//! `Challenge::EcoOnly` is allowed to buy it, and an all-economy build stacks it faster
//! than any other build in the game. Once ~20 copies landed the tank healed tens of
//! thousands of HP per tick against a ~10 HP/tick contact leak on a 24k pool, and no wave
//! schedule could touch it: across seeds 0..24 the 10 seeds that drew the item before tick
//! 3600 were EXACTLY the 10 that survived (`diag_naked_eco_rush_survival_cause`). The
//! guard was measuring a SHOP DRAW, not early wave pressure.
//!
//! Fixed in `economy::income_regen_tick_cap` — a per-tick ceiling on that heal, denominated
//! in the tank's Max HP and ramped in quartically over the run, so it is a rounding error in
//! the opening minutes and fully permissive by the boss. A FLAT fraction of Max HP was
//! measured across twelve values and cannot work: the naked eco-rush and a real build demand
//! sustain ~280x apart on identical 24,000 HP pools, so the window is empty from both ends
//! (see the doc comment on `income_regen_tick_cap`). With the ramp the punish lands on the
//! whole sample — 24/24 at the guard's own sample size and 239/240 at 240 seeds, against a
//! ≥75% bar — and every docs/11 §11.2 fun target is unchanged.
//!
//! The `docs/11 §11.8` sustain-parity pass then added a SECOND, independent bound on the
//! same modifier — `modifiers::INCOME_REGEN_SOFT`, a soft cap on the accumulated RATIO
//! rather than on the per-tick result — and steepened this ramp (POOLS 5→3, POW 4→5). Both
//! changes only tighten the naked eco-rush, so this guard's margin grew rather than shrank.
//! See the degeneracy block further down for what that pass was actually for.

use sim::bot::{Bot, Challenge};
use sim::{step, ArenaState, Input, WeaponInstance};

const DEADLINE: u32 = 3600; // ≤ 2 min @ 30 Hz — the eco-rush death deadline.

/// Run a NAKED eco-rush: strip the starting weapon, buy ONLY economy (EcoOnly), and
/// use the free Clear reactively when swarmed. Returns the tank's death tick (or the
/// cap if it somehow survives). Fully deterministic from `seed`.
fn run_naked_eco_rush(seed: u64, cap: u32) -> u32 {
    let mut s = ArenaState::new(seed, 0);
    s.weapons.clear(); // truly weaponless — no offense at all
    let mut bot = Bot::with_challenge(Challenge::EcoOnly);
    while s.tick < cap && !s.dead {
        // Economy buys from the eco-bot; Clear reactively when the board is full.
        let bot_a = Challenge::EcoOnly.filter(bot.decide(&s), &s);
        let clear_now = s.enemies.len() >= 8 && s.tick >= s.tank.clear_cooldown_end;
        let a = match bot_a {
            Input::Clear if !clear_now => Input::Noop,
            _ if clear_now => Input::Clear,
            other => other,
        };
        step(&mut s, a);
        // Defend the invariant: nothing can grant this tank a weapon.
        if !s.weapons.is_empty() {
            s.weapons.clear();
        }
    }
    s.tick
}

/// Run a MODEST opener: keep the starting Bow, add one weapon, and a little HP/armor
/// (≈ a couple of cheap defensive buys). Clear reactively; no further purchases.
/// Returns the death tick (or cap). Deterministic from `seed`.
fn run_modest_opener(seed: u64, cap: u32) -> u32 {
    let mut s = ArenaState::new(seed, 0);
    // + Mortar Launcher (def 1 — cheap reliable splash).
    let id = s.alloc_entity_id();
    s.weapons.push(WeaponInstance { instance_id: id, def: 1, next_fire_tick: 0 });
    // Modest defense: +12000 Max HP and +5 armor.
    s.tank.max_hp += 12_000;
    s.tank.hp = s.tank.max_hp;
    s.tank.armor += 5;
    while s.tick < cap && !s.dead {
        let a = if s.enemies.len() >= 14 && s.tick >= s.tank.clear_cooldown_end {
            Input::Clear
        } else {
            Input::Noop
        };
        step(&mut s, a);
    }
    s.tick
}

/// DIAGNOSTIC (ignored — evidence for the module note, not a gate). Prints, per seed,
/// the tick at which the eco bot first acquires income-as-HP-regen and whether the run
/// died by the deadline. BEFORE the `income_regen_tick_cap` ceiling the two columns
/// agreed on every seed (acquisition ⇒ survival, no acquisition ⇒ death at 1890–2410);
/// it is kept because it is the direct read-out of that coupling — every seed should now
/// die by the deadline whether or not it draws the item. Run with
/// `cargo test --release -p sim --test balance_guards -- --ignored --nocapture`.
#[test]
#[ignore]
fn diag_naked_eco_rush_survival_cause() {
    let mut acquired = 0u32;
    for seed in 0..24u64 {
        let mut s = ArenaState::new(seed, 0);
        s.weapons.clear();
        let mut bot = Bot::with_challenge(Challenge::EcoOnly);
        let mut first_regen_tick: Option<u32> = None;
        while s.tick < DEADLINE && !s.dead {
            let bot_a = Challenge::EcoOnly.filter(bot.decide(&s), &s);
            let clear_now = s.enemies.len() >= 8 && s.tick >= s.tank.clear_cooldown_end;
            let a = match bot_a {
                Input::Clear if !clear_now => Input::Noop,
                _ if clear_now => Input::Clear,
                other => other,
            };
            step(&mut s, a);
            if !s.weapons.is_empty() {
                s.weapons.clear();
            }
            if first_regen_tick.is_none() && s.economy.income_regen_pct > determinism::Fixed::ZERO {
                first_regen_tick = Some(s.tick);
            }
        }
        if first_regen_tick.is_some() {
            acquired += 1;
        }
        println!(
            "seed {seed:2}  income-regen acquired at {first_regen_tick:?}  end {}  dead {}",
            s.tick, s.dead
        );
    }
    println!("seeds acquiring income-as-HP-regen before tick {DEADLINE}: {acquired}/24");
}

// ===================== The single-modifier degeneracy (docs/11 §11.8) =========
//
// `MODIFIERS[7]` (`Entangled Gold Mine`) carries `IncomeRegenPct(25,100)`. The
// finding in §11.8 is that owning it, not anything else about the build, decided
// the run:
//
//     BEFORE, 80 seeds   own income-as-HP-regen  won 26 of 37   (70.3%)
//                        do not                  won  1 of 43   ( 2.3%)
//
// i.e. the headline 33.8% win rate was approximately that item's DRAW rate, and
// 96% of every win in the game came from one shop draw. After the sustain-parity
// pass (pool-fed HP/shield regen + a soft cap on the income ratio):
//
//     AFTER,  80 seeds   own it   15 of 37 (40.5%)   do not  12 of 43 (27.9%)
//     AFTER, 240 seeds   own it   38 of 106 (35.8%)  do not  40 of 134 (29.9%)
//
// — a 67.9-point advantage reduced to 5.9, and a majority (51.3% at 240 seeds) of
// all wins now coming from builds that never owned the item.
//
// The diagnostic below is the reusable read-out of exactly that partition, so the
// number can be re-checked after any balance change rather than re-derived from a
// throwaway script. It plays the SAME run the `sweep` harness plays — reference
// bot, real conclusion, win == boss killed — and additionally records the peak of
// every COMPETING sustain stat, which is what tuning actually needs: it answers
// "what do the alternatives reach in a real build" and not merely "did the gap
// close".
//
// WHAT IT REVEALED, and what any future pass here should know: the reference bot
// spends its entire modifier budget (`bot::MODIFIER_BUY_CAP`) inside the first ~3
// minutes, and shop offers are not consumed, so a round resolves to ~150 copies of
// ONE item. A run is therefore largely decided by which modifier sat in its opening
// shop. That is why the partition was so extreme, and it is why the remaining gap is
// hard to close from the content tables alone: the median run buys NO defensive item
// at any point (see the `maxHP med` column), and no amount of tuning helps a build
// that owns none of the thing being tuned.
//
// Run with:
//   cargo test --release -p sim --test balance_guards -- --ignored --nocapture
// Seed count is overridable:  SEEDS=240 cargo test …
// Per-seed rows:              ROWS=1 cargo test …

/// Ticks of headroom past the boss spawn — mirrors `preview::sweep`'s
/// `BOSS_FIGHT_ROOM` so the two harnesses measure the same run.
const BOSS_FIGHT_ROOM: u32 = 5 * 60 * 30;

/// Everything the partition needs from one played-out run.
struct SustainRun {
    seed: u64,
    /// The boss was killed — the actual win condition.
    won: bool,
    /// The run ever held `income_regen_pct > 0`.
    owns_income_regen: bool,
    /// Peak of each competing sustain path, so a tuning pass can see what the
    /// alternatives actually reach rather than what the catalog nominally offers.
    peak_max_hp: i64,
    peak_hp_regen: i64,
    peak_shield_pool: i64,
    peak_armor: i64,
    heal_on_kill: i64,
    dodge_pct: i64,
    /// Deepest NET single-tick HP loss seen, in per-mille of the pool at that
    /// tick. This is the sustain DEMAND: a build dies to a tick it cannot undo, so
    /// any competing sustain has to be worth about this much per tick.
    deepest_drop_permille: i64,
    /// HP at the moment of death, in per-mille of the pool — how deep the killing
    /// blow went past zero (the "resurrection" depth of `docs/11 §11.8`).
    death_depth_permille: i64,
    /// Ratios, in per-mille, of the three multiplicative sustain riders.
    income_regen_permille: i64,
    missing_hp_permille: i64,
    healing_permille: i64,
    /// Copies bought of each catalog modifier, indexed by def. What the build
    /// ACTUALLY acquired, which is the other half of "why did it lose".
    mod_buys: Vec<u16>,
}

/// Play one seed to a real conclusion with the reference bot (no challenge).
fn play_to_conclusion(seed: u64) -> SustainRun {
    use determinism::Fixed;
    let cap = sim::content::BOSS_SPAWN_TICK + BOSS_FIGHT_ROOM;
    let mut s = ArenaState::new(seed, 0);
    let mut bot = Bot::default();
    let mut owns_income_regen = false;
    let mut peak_max_hp = s.tank.max_hp;
    let mut peak_hp_regen = s.tank.hp_regen_per_tick;
    let mut peak_shield_pool = s.tank.mana_shield_max;
    let mut peak_armor = s.tank.armor;
    let mut boss_reached = false;
    let mut won = false;
    let mut mod_buys = vec![0u16; sim::content::MODIFIERS.len()];
    let mut deepest_drop_permille = 0i64;
    while s.tick < cap && !s.dead && !won {
        let a = bot.decide(&s);
        // Record the modifier the bot is about to buy (a `BuyOffer` lands iff the
        // tank can afford it — see `input::apply`), so the build's actual shopping
        // list is observable without reaching into the bot's private state.
        if let Input::BuyOffer { slot } = a {
            if let Some(off) = s.shop.offers.get(slot as usize) {
                if s.economy.gold >= off.cost && matches!(off.kind, sim::OfferKind::Modifier) {
                    mod_buys[off.def as usize] += 1;
                }
            }
        }
        let hp_before = s.tank.hp;
        step(&mut s, a);
        let drop = hp_before - s.tank.hp;
        if drop > 0 && s.tank.max_hp > 0 {
            deepest_drop_permille =
                deepest_drop_permille.max(drop.saturating_mul(1000) / s.tank.max_hp);
        }
        owns_income_regen |= s.economy.income_regen_pct > Fixed::ZERO;
        peak_max_hp = peak_max_hp.max(s.tank.max_hp);
        peak_hp_regen = peak_hp_regen.max(s.tank.hp_regen_per_tick);
        peak_shield_pool = peak_shield_pool.max(s.tank.mana_shield_max);
        peak_armor = peak_armor.max(s.tank.armor);
        // Boss present → absent while alive is exactly a boss kill (same rule the
        // sweep harness uses; the boss is removed from `enemies` on death and the
        // sim freezes on tank death).
        if s.tick >= sim::content::BOSS_SPAWN_TICK {
            match s.enemies.iter().any(|e| e.def == sim::content::BOSS) {
                true => boss_reached = true,
                false => won = boss_reached && !s.dead,
            }
        }
    }
    SustainRun {
        seed,
        won,
        owns_income_regen,
        peak_max_hp,
        peak_hp_regen,
        peak_shield_pool,
        peak_armor,
        heal_on_kill: s.tank.heal_on_kill,
        dodge_pct: if s.tank.dodge_den > 0 { s.tank.dodge_num as i64 * 100 / s.tank.dodge_den as i64 } else { 0 },
        deepest_drop_permille,
        death_depth_permille: if s.tank.max_hp > 0 { s.tank.hp.saturating_mul(1000) / s.tank.max_hp } else { 0 },
        income_regen_permille: s.economy.income_regen_pct.scale_i64(1000),
        missing_hp_permille: s.tank.missing_hp_heal_pct.scale_i64(1000),
        healing_permille: s.tank.healing_mult.scale_i64(1000),
        mod_buys,
    }
}

/// Play `seeds` runs across `threads` workers and return them in SEED order, so the
/// aggregate is a pure function of the seed range (threading changes only speed).
fn play_range(seeds: u64) -> Vec<SustainRun> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let next = std::sync::atomic::AtomicU64::new(0);
    let out = std::sync::Mutex::new(Vec::with_capacity(seeds as usize));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let seed = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if seed >= seeds {
                    break;
                }
                let r = play_to_conclusion(seed);
                out.lock().unwrap().push(r);
            });
        }
    });
    let mut v = out.into_inner().unwrap();
    v.sort_by_key(|r| r.seed);
    v
}

fn winners_of(runs: &[SustainRun]) -> Vec<&SustainRun> {
    runs.iter().filter(|r| r.won).collect()
}

fn median(mut v: Vec<i64>) -> i64 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[v.len() / 2]
}

fn pct(v: &[i64], p: usize) -> i64 {
    if v.is_empty() {
        return 0;
    }
    let mut v = v.to_vec();
    v.sort_unstable();
    v[(v.len() * p / 100).min(v.len() - 1)]
}

/// DIAGNOSTIC (ignored — the headline measurement for `docs/11 §11.8`, not a gate).
/// Win rate PARTITIONED by whether the run ever acquired income-as-HP-regen. The
/// number that matters is the last line: the share of all wins that came from builds
/// WITHOUT the item. At the §11.8 baseline it was 1/27 ≈ 4%; a game where no single
/// modifier decides the run puts it near half. Currently 44.4% at 80 seeds and 51.3%
/// at 240 (80 is noisy — prefer 240 for any judgement).
#[test]
#[ignore]
fn diag_win_rate_partitioned_by_income_regen() {
    let seeds: u64 = std::env::var("SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(80);
    let runs = play_range(seeds);

    let (own, not): (Vec<&SustainRun>, Vec<&SustainRun>) =
        runs.iter().partition(|r| r.owns_income_regen);
    let own_wins = own.iter().filter(|r| r.won).count();
    let not_wins = not.iter().filter(|r| r.won).count();
    let wins = own_wins + not_wins;

    let rate = |w: usize, n: usize| if n == 0 { 0.0 } else { 100.0 * w as f64 / n as f64 };
    println!("\n=== win rate partitioned by income-as-HP-regen — {seeds} seeds ===");
    println!(
        "  owns income-regen     : {own_wins:3}/{:<3} ({:5.1}%)",
        own.len(),
        rate(own_wins, own.len())
    );
    println!(
        "  does NOT own it       : {not_wins:3}/{:<3} ({:5.1}%)",
        not.len(),
        rate(not_wins, not.len())
    );
    println!("  overall win rate      : {wins:3}/{seeds:<3} ({:5.1}%)", rate(wins, seeds as usize));
    println!(
        "  gap (owners − others) : {:5.1} points",
        rate(own_wins, own.len()) - rate(not_wins, not.len())
    );
    println!(
        "  >>> share of WINS from builds WITHOUT income-regen: {not_wins}/{wins} ({:5.1}%)",
        rate(not_wins, wins.max(1))
    );

    // What the competing sustain paths actually reach, split the same way — the
    // input a tuning pass needs. If the alternatives are all pinned at their
    // starting values, "raise the alternatives" has nothing to raise.
    for (label, set) in [("owners", &own), ("non-owners", &not)] {
        let f = |g: fn(&SustainRun) -> i64| set.iter().map(|r| g(r)).collect::<Vec<_>>();
        println!(
            "  {label:<11} peak: maxHP med {:>9} p90 {:>9} | hpRegen med {:>7} p90 {:>8} | shield med {:>7} p90 {:>8} | armor med {:>5} | healOnKill med {:>4} | dodge med {:>3}%",
            median(f(|r| r.peak_max_hp)),
            pct(&f(|r| r.peak_max_hp), 90),
            median(f(|r| r.peak_hp_regen)),
            pct(&f(|r| r.peak_hp_regen), 90),
            median(f(|r| r.peak_shield_pool)),
            pct(&f(|r| r.peak_shield_pool), 90),
            median(f(|r| r.peak_armor)),
            median(f(|r| r.heal_on_kill)),
            median(f(|r| r.dodge_pct)),
        );
    }
    // What the winners actually bought, versus what everyone bought. A sustain path
    // that never appears in a winning shopping list is not "weak", it is unbought.
    {
        let tally = |set: &[&SustainRun]| {
            let mut t = vec![0u32; sim::content::MODIFIERS.len()];
            for r in set {
                for (i, n) in r.mod_buys.iter().enumerate() {
                    t[i] += *n as u32;
                }
            }
            t
        };
        let all: Vec<&SustainRun> = runs.iter().collect();
        let t_all = tally(&all);
        let t_win = tally(&winners_of(&runs));
        let mut idx: Vec<usize> = (0..t_all.len()).collect();
        idx.sort_by_key(|&i| std::cmp::Reverse(t_all[i]));
        println!("  most-bought modifiers (copies across all runs | across winners):");
        for &i in idx.iter().take(12) {
            if t_all[i] == 0 {
                break;
            }
            println!(
                "    {:>5} | {:>5}  {}",
                t_all[i],
                t_win[i],
                sim::content::MODIFIERS[i].name
            );
        }
    }
    // Per-seed rows (ROWS=1) — for reading which build shape survives what.
    if std::env::var("ROWS").is_ok() {
        println!("  seed  win  peakHP   regen  shield  armor    hok  dodge   ir‰  miss‰  heal‰   drop‰  death‰");
        for r in &runs {
            println!(
                "  {:4}  {:>3}  {:>6}  {:>6}  {:>6}  {:>5}  {:>5}  {:>4}%  {:>7}  {:>5}  {:>5}  {:>7}  {:>8}",
                r.seed,
                if r.won { "W" } else { "." },
                r.peak_max_hp,
                r.peak_hp_regen,
                r.peak_shield_pool,
                r.peak_armor,
                r.heal_on_kill,
                r.dodge_pct,
                r.income_regen_permille,
                r.missing_hp_permille,
                r.healing_permille,
                r.deepest_drop_permille,
                r.death_depth_permille,
            );
        }
    }
    // Winners only — the shape of a build that actually gets there.
    let winners: Vec<&SustainRun> = runs.iter().filter(|r| r.won).collect();
    let g = |h: fn(&SustainRun) -> i64| winners.iter().map(|r| h(r)).collect::<Vec<_>>();
    println!(
        "  winners     peak: maxHP med {} | hpRegen med {} | shield med {} | armor med {}",
        median(g(|r| r.peak_max_hp)),
        median(g(|r| r.peak_hp_regen)),
        median(g(|r| r.peak_shield_pool)),
        median(g(|r| r.peak_armor)),
    );
}

#[test]
fn naked_eco_rush_is_dead_by_deadline_on_a_representative_seed() {
    // Seed 2 is representative of the punish landing on the mechanic this guard is meant
    // to test — contact leak accumulating past a 300-tick Clear. It dies at tick 2190,
    // i.e. 1410 ticks (47 s) inside the deadline, mid-pack for the sample (deaths now
    // span 1890–2476 across 240 seeds). It never draws income-as-HP-regen, so it was the
    // one seed whose result is unchanged by the `income_regen_tick_cap` ceiling; seed 0,
    // which draws the heal at tick 907, used to be unkillable and now dies at 2085.
    let death = run_naked_eco_rush(2, 56_000);
    assert!(
        death <= DEADLINE,
        "naked eco-rush must be DEAD by tick {DEADLINE}, died at {death}"
    );
}

#[test]
fn naked_eco_rush_punish_holds_for_the_broad_majority() {
    // Across a 24-seed sample, the eco-rush punish must land on the large majority
    // (≥ 75%). GREEN with margin since `economy::income_regen_tick_cap` bounded the
    // income-as-HP-regen heal: 24/24, and still 100% at 48, 96 and 240 seeds (the bar
    // stays at 75% precisely so it measures a broad majority rather than this sample).
    // The bar was never lowered to meet the mechanic; the mechanic was bounded.
    let n = 24u64;
    let dead_by_deadline = (0..n).filter(|&seed| run_naked_eco_rush(seed, 56_000) <= DEADLINE).count();
    assert!(
        dead_by_deadline * 100 >= (n as usize) * 75,
        "eco-rush punish must hold for ≥75% of seeds; only {dead_by_deadline}/{n} died by tick {DEADLINE}"
    );
}

#[test]
fn modest_opener_survives_past_the_deadline() {
    // A modest opener (Bow + one weapon + a little HP/armor) must establish board
    // control and live PAST the deadline on EVERY sampled seed — the other end of the
    // design window (early grace for a real build must not collapse).
    for seed in 0..16u64 {
        let death = run_modest_opener(seed, 56_000);
        assert!(
            death > DEADLINE,
            "modest opener (seed {seed}) must survive past tick {DEADLINE}, died at {death}"
        );
    }
}
