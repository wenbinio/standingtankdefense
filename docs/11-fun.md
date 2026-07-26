# 11 — Fun: Diagnosis and Targets

Everything before this document was about **correctness** — the sim is provably right, bit-for-bit against the Rust, across 55,200 ticks. None of that makes it fun. This document is the first honest look at whether the game is any good to play, measured rather than argued.

**Headline: as implemented, it is not a game you can lose, and that is the root of every other problem.**

## 11.1 What the data says

`cargo run -p preview --bin sweep` over 80 seeds:

| metric | measured | what it means |
| --- | --- | --- |
| Win rate | **92.5%** (74/80) | There is no failure state in practice |
| Survival, median | **1200 s = the cap** | Most runs simply hit the ceiling alive |
| Deaths under 30 s | **0/80** | No early pressure whatsoever |
| Earliest death, any seed | tick 13832 (**461 s**) | Nothing threatens you for the first 7½ minutes |
| `balanced` bot mean max HP | **244,815,927** | Numbers have left the readable range entirely |
| `glass-cannon` mean max HP | 2,423,425 | Same problem, two orders down |
| Arsenal in a sample run | **`Magic Bolt ×14`** | One weapon copied, not a build |

### The four failures, in order of how much they cost

1. **You cannot lose.** A 92.5% win rate in a *survival* roguelite means every decision is free. Nothing is at stake, so nothing you choose matters, so there is no tension and no reason to think. This single fact neutralizes the whole design — the shop is the game, and the shop is only interesting if a wrong pick can kill you.

2. **The numbers run away.** 244 million max HP is not a power fantasy, it is a readability failure. Multiplicative stacking (`[02] §2.1`) is the intended engine, but with nothing bounding it the curve leaves the range where a player can reason about trade-offs. "Do I need more HP?" stops being answerable.

3. **Offers are noise, not choices.** `shop.rs` draws **uniformly** over all 86 weapons and 91 modifiers into a fixed 4+4 grid (see `[10] F9`). A 500 g Common and a 3000 g Epic are equally likely. There is no rarity pressure, no escalation, no jackpot moment — the emotional beat that carries this entire genre. `[02] §2.5` specifies a rarity ladder driving draw weight; **it was never implemented.**

4. **Stacking beats building.** `Magic Bolt ×14` is the optimum because copies stack cleanly and nothing rewards breadth. The 86-weapon catalog collapses into "find the best one, buy it repeatedly."

### The harness is also measuring the wrong thing

`sweep` caps at **20 minutes** (36,000 ticks) and its header still talks about "the 15-min boss" — but `BOSS_SPAWN_TICK` is **30 minutes** (54,000). So the "92.5% win rate" is really *"92.5% survived two thirds of a run"*, and **no sweep has ever seen the boss fight.** The actual win condition — reach the boss, kill it with `Clear` — is entirely unmeasured. Fix the harness before trusting any tuning done against it.

## 11.2 Targets

Fun is not directly measurable, but its absence is. These are the numbers to tune toward, chosen for a run-based roguelite where a run is a *bet*:

| target | now | goal | why |
| --- | --- | --- | --- |
| Win rate (competent play) | 92.5% | **25–40%** | A win has to be worth something. Losing must be the common case. |
| Deaths in the first 25% of a run | 0% | **10–20%** | Early pressure makes early buys matter. |
| Deaths spread across the run | all at cap | **no single spike** | A difficulty *curve*, not a wall or a plateau. |
| Peak max HP | 2.4M–245M | **under ~500k** | Keep quantities in a range a player can compare. |
| Distinct weapons in a winning build | ~1–3 real | **5+** | Breadth should be competitive with stacking. |
| Offer rarity distribution | uniform | **weighted, escalating** | Jackpot moments; late shops feel different from early ones. |
| Boss reached | never measured | **measured, and a real fight** | It is the win condition. |

## 11.3 The work, in priority order

1. **Make it losable.** Raise the pressure curve until the win rate lands in the target band. This is the highest-value change by a wide margin and everything else is cosmetic without it.
2. **Implement rarity weighting.** The single biggest *feel* win: it turns eight random rows into a stream of escalating temptations. Needs building from scratch — `[10] F3` records that no weighting machinery exists.
3. **Bound the runaway curve.** Contain multiplicative stacking so numbers stay legible without removing the stacking fantasy.
4. **Reward breadth.** Make a five-weapon build competitive with `Magic Bolt ×14` — diminishing returns on copies, or synergy bonuses across damage types.
5. **Fix the harness first.** Extend `sweep` to run past the boss, report the death-time distribution and the boss outcome, and measure build diversity honestly. Tuning against a broken meter is worse than not tuning.

## 11.4 Constraint: balance changes are not free

The content tables are the **shared** source of truth (`[10] F0`). Changing them:

- moves `content_hash`, which **invalidates the whole R3 trace corpus**, so the traces must be regenerated and the Luau gate re-run;
- changes both the Steam and Roblox builds at once, which is the intent — this is balance, not a fork-only concern.

That is an acceptable cost and the pipeline handles it (`export-content`, then `export-traces`, then `trace_test`). But it means balance work must land as a deliberate, reviewed batch rather than as a trickle, and the gate must be green again before anything is called done.
