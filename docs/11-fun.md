# 11 — Fun: Diagnosis and Targets

Everything before this document was about **correctness** — the sim is provably right, bit-for-bit against the Rust, across 55,200 ticks. None of that makes it fun. This document is the first honest look at whether the game is any good to play, measured rather than argued.

**Headline: the first thirty minutes are a plateau nobody dies on, and then the boss deletes 82% of all runs in a thirteen-second window. The game is not too easy or too hard — it is shapeless.**

## 11.1 What the data says

> **This section was rewritten after the harness was fixed. The original diagnosis below the line was wrong** — it was measured with a `sweep` that capped at 20 minutes while the boss spawns at 30, so it never observed the win condition and reported "survived two thirds of a run" as a win. Everything in this subsection is measured against the rebuilt harness: **win = boss actually killed**, cap 63,000 ticks, 80 seeds (confirmed stable at 240).

| metric | measured | §11.2 target | verdict |
| --- | --- | --- | --- |
| Win rate (boss killed) | **16.2%** (13/80) | 25–40% | too **hard** |
| Deaths in first quarter (<7:30) | **0.0%** | 10–20% | no early stakes |
| Largest single death decile | **81.8%** | <25% | a wall, not a curve |
| Peak max HP — median | **24,000** (the starting value) | <500k | passes, degenerately |
| Peak max HP — worst seed | **3.72 B** | <500k | severe tail |
| Distinct weapons, winning builds | **3.2** | ≥5 | concentration wins |

### The real failures, in order of how much they cost

1. **The boss is a wall, and it is the entire difficulty curve.** **81.8% of every death in the game happens inside a ~13-second window at 30:13.** Deciles one through ten hold *twelve deaths between them*; the boss phase holds fifty-four. The first thirty minutes are a plateau nobody dies on, and then a cliff.

2. **The boss is binary, not a fight.** Winners *always* spend exactly 11 Clears — the theoretical minimum for 33M HP at 3M per Clear — with a TTK of 100–109 s. Losers almost always land exactly **one** Clear and die with 30M of 33M remaining. There is no partial progress, no comeback, no "nearly had it". You either chain every cooldown perfectly or you are deleted, and which of those happens is settled long before the boss appears.

3. **Nothing is at stake for the first 7 minutes 41 seconds.** Zero deaths before that point across 240 seeds. Every early purchase is consequence-free.

4. **Breadth is anti-correlated with winning.** All runs average 4 distinct weapons and 14 copies, with the top weapon at 66.6% of the arsenal. *Winning* runs are **narrower** — 3.2 distinct, 76.0% top share. Concentration is not merely tolerated, it is the winning strategy, which makes an 86-weapon catalog mostly decoration.

5. **Runaway magnitudes are a tail, not the norm.** Median peak max HP is **24,000 — the starting value**, meaning over half of all runs never buy a single Max-HP item. Meanwhile the worst seeds reach 3.72 B max HP and 1.61e15 lifetime damage. So the curve is simultaneously *flat for most players* and *broken for a few*, which needs a tail fix, not a global damp.

6. **The Clear outdamages the arsenal.** Median peak per-tick damage is 3.09M, essentially `CLEAR_DAMAGE` (3M). In a median run **the biggest damage event is the free cooldown ability, not anything the player chose to buy.**

---

### Superseded: the original diagnosis, and why it was wrong

The first pass reported a 92.5% win rate, 244M mean max HP, and no deaths before 7½ minutes at a 20-minute cap — concluding "the game is not losable". That reading came from a harness that stopped 10 minutes before the boss and counted survival-to-cap as victory. Measured to a real conclusion the game is the *opposite* of too easy: 16.2% of runs win.

The lesson is worth keeping: **the meter was broken in a direction that inverted the conclusion**, and three agents were briefed off it before anyone noticed. Fix the instrument before tuning the thing.

## 11.2 Targets

Fun is not directly measurable, but its absence is. These are the numbers to tune toward, chosen for a run-based roguelite where a run is a *bet*:

| target | now | goal | why |
| --- | --- | --- | --- |
| Win rate (competent play) | 16.2% | **25–40%** | Currently too punishing; a win must be reachable, not a lottery. |
| Deaths in the first 25% of a run | 0.0% | **10–20%** | Early pressure makes early buys matter. |
| Largest single death decile | 81.8% (the boss) | **under 25%** | A difficulty *curve*, not a 30-minute plateau then a cliff. |
| Peak max HP (tail) | up to 3.72B | **under ~500k** | Median is already 24k; it is the tail that needs bounding. |
| Distinct weapons in a winning build | 3.2 | **5+** | Breadth is currently anti-correlated with winning. |
| Offer rarity distribution | uniform | **weighted, escalating** | Jackpot moments; late shops feel different from early ones. |
| Boss outcome | 85% reach it, 19% of those kill it | **a real fight, with partial progress** | Today it is binary: 11 Clears or deleted. |

## 11.3 The work, in priority order

0. **Fix the harness.** ✅ **Done** — and it inverted the diagnosis. `sweep` now runs past the boss, wins only on a boss kill, and reports death deciles, boss outcome, magnitude tails and build shape.

1. **Redistribute the difficulty, don't amplify it.** The single highest-value change. Move pressure *out* of the boss wall and *into* the empty first two-thirds, so deaths spread across the run instead of piling into thirteen seconds. Net win rate should rise from 16.2% into 25–40% while the largest death decile falls under 25%. This is a *shape* problem, and treating it as a difficulty-level problem will make it worse in both directions at once.

2. **Make the boss a fight.** It is currently a pass/fail check on whether you can chain 11 Clears. It needs partial progress, a comeback path, and an outcome other than "perfect or deleted" — and probably damage sources other than the one free ability.

3. **Implement rarity weighting.** ✅ **Done** — Epic went from a flat ~12% at all times to 2% early and 21% late, and earliest death dropped from 7:41 to 3:25.

4. **Reward breadth.** Winning builds are *narrower* than average (3.2 distinct, 76% top-weapon share), so breadth is anti-correlated with winning. Add upside for covering damage types rather than taxing copies.

5. **Bound the magnitude tail.** Median peak HP is the starting value; the worst seed is 3.72 B. Needs a soft cap that bites at the extreme and is invisible at p90, not a global damp.

6. **Make the arsenal matter more than the free ability.** Median peak per-tick damage ≈ `CLEAR_DAMAGE`. In half of all runs the biggest hit is the thing the player didn't buy.

## 11.4 Constraint: balance changes are not free

The content tables are the **shared** source of truth (`[10] F0`). Changing them:

- moves `content_hash`, which **invalidates the whole R3 trace corpus**, so the traces must be regenerated and the Luau gate re-run;
- changes both the Steam and Roblox builds at once, which is the intent — this is balance, not a fork-only concern.

That is an acceptable cost and the pipeline handles it (`export-content`, then `export-traces`, then `trace_test`). But it means balance work must land as a deliberate, reviewed batch rather than as a trickle, and the gate must be green again before anything is called done.
