# Standing Tank Defense — Roblox fork

The Roblox build is a **fork, not a port** (`docs/09` Part 3 Option C, locked in `docs/10`). It shares the content catalog, the stacking engine and the sharded-arena architecture with the Steam build; it does not share the run structure, the competitive layer or the progression model.

- Design decisions: [`docs/10-roblox-fork-design.md`](../docs/10-roblox-fork-design.md)
- Feasibility background: [`docs/09-roblox-port-assessment.md`](../docs/09-roblox-port-assessment.md)
- **Interface contracts (read before touching a seam):** [`CONTRACTS.md`](CONTRACTS.md)

---

## Layout

```
roblox/
  default.project.json   Rojo project — maps dist/, never src/
  CONTRACTS.md           the seams — owned centrally
  README.md              this file
  build/                 the require-rewriting build step (see below)
  src/shared/            Fixed.luau, Rng.luau, Content.luau, content.json   -> ReplicatedStorage.Shared
  src/server/            Arena, Director                                     -> ServerScriptService.Server
  src/client/            render + UI                                         -> StarterPlayerScripts.Client
  dist/                  GENERATED Roblox-shaped copy of src/ — what Rojo syncs
  bench/                 R2 throughput harness                               -> ServerScriptService.Bench
  test/                  parity runner + vectors                             -> ServerStorage.Test
```

`dist/server`, `dist/client` and `test` are declared **optional** in the Rojo project, so the place builds before those milestones land. `bench/` and `dist/shared` are not optional — a missing `dist/shared` is Rojo telling you that you forgot to run the build.

Per `CONTRACTS.md` §C5, nothing in `src/shared/` may reference a Roblox global (`game`, `workspace`, `task`, `Instance`). That layer — and the measurement core of the bench — runs unmodified under a bare `luau` interpreter, which is how both are tested in CI.

---

## The build step: `src/` → `dist/`

`src/**` is written for the **standalone Luau interpreter**, which resolves modules by relative string path:

```lua
local Fixed = require("../Fixed")
```

That is the only form the bare interpreter accepts, and Roblox accepts none of it — Roblox needs an *instance reference*, `require(script.Parent.Parent.Fixed)`.

Both have to keep working, and the sources are not negotiable: bare-interpreter execution is how the whole test suite and the R3 correctness oracle run (`test/trace_test.luau` proves the Luau sim reproduces the Rust bit-for-bit for 55,200 ticks). Editing the modules to use Roblox requires would forfeit that guarantee. So the sources are never touched. A build step reads `src/**` and emits Roblox-shaped **copies** into `dist/**`, rewriting each require into the instance reference implied by the file's position in the Rojo tree. **Rojo syncs `dist/`, never `src/`.**

```bash
python3 roblox/build/build.py                 # build + verify   <- the one you want
python3 roblox/build/build.py --exec-check    # + run the emitted graph (needs `luau`)
python3 roblox/build/build.py --check         # CI: fail if dist/ is stale
python3 roblox/build/selftest.py              # tests for the rewriter itself
```

Python 3.8+, standard library only, no install step. `luau` / `luau-analyze` are found on `PATH` or via `$LUAU` / `$LUAU_ANALYZE`.

**Run it after every edit under `src/`.** `--check` is the CI gate that catches a forgotten rebuild.

### What it rewrites

| source | emitted |
| --- | --- |
| `require("./State")` | `require(script.Parent.State)` |
| `require("../Fixed")` | `require(script.Parent.Parent.Fixed)` |
| `pcall(require, "../Content")` | `pcall(require, script.Parent.Parent.Content)` |
| `require("../shared/sim/Arena")` from `src/server/` | `require(game:GetService("ReplicatedStorage"):WaitForChild("Shared"):WaitForChild("sim"):WaitForChild("Arena"))` |

Within one Rojo root the emitted reference is relative (`script.Parent…`). Crossing services it names the service and `WaitForChild`s each step down, because a client script can run before `ReplicatedStorage`'s children have replicated. Instance names that are not valid identifiers are bracket-indexed. The instance layout is read out of `default.project.json` itself, so the build step and the Rojo project cannot drift apart.

Everything else is copied verbatim (`content.json`, `.meta.json` sidecars). Line numbers shift by the three-line generated banner; `--!strict` / `--!native` directives are kept on line 1 where Luau requires them.

### What it refuses to do

The failure mode worth preventing is a require that survives the rewrite and only explodes inside Studio, so nothing is trusted:

- Every rewritten require must resolve to a module that was **actually emitted**. Unresolvable, or resolving outside every build root — the build fails with `file:line`.
- After emitting, every generated require expression is **re-parsed out of the emitted text** by an independent evaluator and walked against an instance tree rebuilt from the emitted files. Correct by construction, then checked anyway.
- A relative-path string literal that is *not* in a require position fails the build rather than being emitted as something that silently breaks.
- A require whose path is computed (`require("./v/" .. name)`) fails the build. It cannot be resolved statically and will not be guessed at.
- Every emitted file is parsed by `luau-analyze`. Any `SyntaxError` fails. Diagnostics are compared **differentially against the source file**, so a pre-existing lint is not laundered into a build failure and a newly-introduced one cannot hide. The only tolerated new diagnostics are the unknown-type family that necessarily follows from an instance require the analyzer cannot resolve without a Rojo sourcemap.
- `--exec-check` goes further and *executes* every emitted module under the standalone interpreter against an emulated instance tree, proving the rewritten graph actually loads. It separates "the rewrite named something that does not exist" (fatal) from "this module needs a real Roblox runtime" (reported, not fatal).

Requires that are already instance references are passed through untouched and reported in the summary — never silently.

### `dist/` is generated

Treat it as build output: **do not edit it, and do not commit it.** `roblox/dist/` is in the repo `.gitignore`. It is a byte-for-byte function of `src/` plus `default.project.json`, `--check` proves that in CI, and committing it would double every sim diff and let a stale copy drift from `src/`.

## Syncing to Studio

Requires [Rojo](https://rojo.space) 7.3+ (the `{"optional": ...}` path form) and Roblox Studio.

```bash
python3 roblox/build/build.py              # ALWAYS first — Rojo reads dist/

# live sync into an open Studio place
rojo serve roblox/default.project.json     # then connect from the Rojo Studio plugin

# or build a standalone place file
rojo build roblox/default.project.json -o StandingTankDefense.rbxlx
```

## Running the throughput bench (milestone R2)

There are **two** benches, and they answer different questions.

| file | what it runs | what it answers |
| --- | --- | --- |
| `bench/realsim.luau` | the **shipped sim** — `Arena.step` driven by `Bot.decide`, whole matches, both timelines | **does R2 clear?** and what population does the game actually reach |
| `bench/throughput.luau` | a **model** of the sim: a struct-of-arrays mini-arena at a pinned population | what does exact fixed-point cost per entity, `exact` vs `api-dbl` vs `raw-dbl`, and replication bytes |

`realsim.luau` is the authority on the gate. `throughput.luau` is the authority on the arithmetic. Quote the first for verdicts and the second for ratios.

### Standalone — no Roblox required

Any Luau interpreter with `buffer`, `bit32` and require-by-string. Build one from [luau-lang/luau](https://github.com/luau-lang/luau) (`cmake --build . --target Luau.Repl.CLI`), then:

```bash
cd roblox/bench
luau -O2 realsim.luau                 # the gate: 4 seeds x both timelines, ~15 min
luau -O2 realsim.luau -a 1            # one seed per timeline, ~4 min
luau -O2 throughput.luau              # the model: ~30 s
luau -O2 throughput.luau -a 0.15      # shorter: ~0.15 s per timed measurement
luau -O2 --codegen throughput.luau    # with native codegen, the closest proxy for --!native
```

Both auto-detect the absence of Roblox and run immediately. `throughput.luau` picks up the real `../src/shared/Fixed.luau` if present and falls back to `bench/FixedStub.luau` otherwise, printing which one it used; `realsim.luau` always uses the real modules, because running the shipped sim is the entire point.

### In Studio / on a live server

`bench/` maps to `ServerScriptService.Bench`. `run.server.luau` runs on start and does two things:

1. The same report, inside Roblox's own Luau VM.
2. **Eight arenas on eight parallel `Actor`s**, measuring wall clock for all eight — the number the R2 gate is actually about, and the one a standalone interpreter cannot produce.

`throughput.luau` never auto-runs under Roblox; it only returns its module table. To drive it yourself:

```lua
local Bench = require(game.ServerScriptService.Bench.throughput)
Bench.run({ targetSec = 0.15, yield = function() task.wait() end })
```

Pass `yield` on a live server — the measurement core is pure Luau with no yield points, and without it a long run starves the scheduler.

## What the benches measure

`realsim.luau` reports, per timeline: the population the shipped sim reaches (mean / p50 / p90 / p99 / peak), the cost of `Arena.step` (mean, p90, p99, worst sustained second, worst single tick), the cost bucketed by live-enemy count, and the PASS/FAIL gate table serial and parallel.

`throughput.luau` reports:

| Section | Question |
| --- | --- |
| 0 | The analytic wave model — now a **lower bound** on population, and printed as one. |
| 1 | Per-tick cost of one arena at a pinned population, as a % of budget. |
| 1b | Which phase spends the time. |
| 2 | What exactness costs: per-op and whole-workload, exact `Fixed` vs plain doubles. |
| 3 | Replication: bytes/tick for 325 packed positions vs the ~900 B `UnreliableRemoteEvent` cap. |
| 4 | PASS/FAIL per backend, serial and parallel, at the measured populations. |
| 5 | The caveats that qualify all of the above. |

**The workload is sized from the real tables, not invented.** Spawn cadences, ring radii, movement speeds, the damage matrix and the 20-weapon arsenal are transcribed from `sim-core/crates/sim/src/{content.rs,waves.rs}`, and the per-tick phases mirror `sim::step`'s phase order. Population is derived by Little's law over the real cadences (`pop = Σ 1/cadence × radius/speed`) rather than guessed.

That derivation **used to be** checked against the Rust and used to hold: 24 seeds of `sim::step` under `sim::bot::Bot` peaked at **67 live enemies** against the model's **67.6**.

**It no longer holds, and that is the whole story of the R2 re-measurement.** The `[11]` balance pass did not touch a single spawn cadence — `WAVE_M0` and `BOSS_ESCORT` are byte-identical to what the model reads. It ramped enemy **HP** (`content::enemy_hp_mult`) instead, so enemies now survive far longer than spawn-to-contact, and Little's law over spawn *rate* is blind to a kill *rate*. Running the shipped `Arena.step` under the shipped `Bot` (`bench/realsim.luau`, 4 seeds, whole matches) measures **mean 87, p99 270, peak 325** live enemies on the Steam timeline. The model is now a **lower bound**, not an estimate, and `throughput.luau`'s section 0 prints that in place of the old "the model is sound".

Three numeric backends run the *identical* algorithm:

- **`exact`** — the real §C2 `Fixed` (four 16-bit limb tables, bit-identical to Rust).
- **`api-dbl`** — the same §C2 API over a plain-double representation. This is the escape hatch `docs/10 §F5` explicitly reserves: *"the module's internals can be swapped to plain doubles without touching a single caller."*
- **`raw-dbl`** — no `Fixed` at all. The floor.

---

## Measured results

Standalone `luau -O2`, build container (Xeon @ 2.10 GHz, 4 cores), no `--!native`. **Read the caveats below before quoting any of this.**

These numbers replace an earlier set that was stale twice over: it predated both the `Fixed.div` rewrite and the `[11]` balance pass, and it was sized against a 67-enemy board that no longer exists.

### The gate — measured on the shipped sim (`bench/realsim.luau`)

`Arena.step` driven by `Bot.decide`, four seeds, whole matches. Budget: one 30 Hz tick is **33.33 ms** for all 8 arenas; the honest simulation ceiling is **30 % of that = 10.00 ms**, because a Roblox server frame is not ours alone.

| timeline | statistic | ms/arena/tick | serial (8 on 1 thread) | parallel (8 Actors) |
| --- | --- | ---: | ---: | ---: |
| steam | mean tick | 1.670 | **133.6 %** FAIL | 16.7 % PASS |
| steam | p99 tick | 9.331 | **746.5 %** FAIL | 93.3 % PASS |
| steam | worst sustained second | 14.366 | **1149.3 %** FAIL | **143.7 %** FAIL |
| steam | worst single tick | 30.078 | **2406.2 %** FAIL | **300.8 %** FAIL |
| roblox | mean tick | 0.867 | 69.3 % PASS | 8.7 % PASS |
| roblox | p99 tick | 3.128 | **250.3 %** FAIL | 31.3 % PASS |
| roblox | worst sustained second | 3.105 | **248.4 %** FAIL | 31.0 % PASS |
| roblox | worst single tick | 6.416 | **513.3 %** FAIL | 64.2 % PASS |

`steam` is the 30-minute schedule the R3 oracle traces are recorded against. `roblox` is F1's five-minute schedule — **the one this fork ships**, and the one the gate is really about.

### The population the gate is sized against

| timeline | enemies mean | p50 | p90 | p99 | peak | projectiles peak |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| steam | 86.9 | 73 | 200 | 270 | **325** | 20 |
| roblox | 79.1 | 70 | 169 | 233 | **261** | 7 |

Against `docs/10 §F8`'s **peak 66–67 / mean 14.3**: the peak is ~5× and the mean ~6× what the section was sized against. Projectiles went the other way — 84 → 20 — because the arsenal the reference bot ends with is now mostly instant-attack.

### Per-entity arithmetic cost (`bench/throughput.luau`, the model)

Pinned population, identical algorithm across three numeric backends. Percentages are of the 30 % (10.00 ms) ceiling, 8 arenas serial.

| backend | E=105 W=20 | E=218 W=20 | E=282 W=20 | E=325 W=40 | E=675 W=20 |
| --- | ---: | ---: | ---: | ---: | ---: |
| `exact` (§C2 `Fixed`) | 1.462 ms — 116.9 % | 2.936 — 234.9 % | 3.664 — 293.1 % | 6.726 — 538.1 % | 9.014 — 721.1 % |
| `api-dbl` (§F5 hatch) | 0.184 — 14.7 % | 0.423 — 33.8 % | 0.642 — 51.3 % | 1.390 — 111.2 % | 1.841 — 147.3 % |
| `raw-dbl` (floor) | 0.029 — 2.4 % | 0.055 — 4.4 % | 0.077 — 6.1 % | 0.104 — 8.3 % | 0.185 — 14.8 % |

E=105/282/325 are the measured mean/p99/peak. E=218 is the reference-bot peak `[10] §F8`'s invalidation note quotes. E=675 is the analytic max-Frost ceiling.

### What exactness costs

| op | `exact` ns | native double ns | ratio |
| --- | ---: | ---: | ---: |
| add | 189 | 1.40 | 135× |
| sub | 247 | 1.02 | 243× |
| mul | 245 | 0.62 | 396× |
| div | 367 | 0.77 | 476× |
| scaleI64 | 445 | 4.32 | 103× |
| **sqrt** | **1 747** | 2.37 | **737×** |
| fromInt | 277 | — | — |
| **fromRatio** | **693** | 0.82 | **843×** |
| compare | 122 | 0.31 | 391× |

Whole workload at E=282/W=20: `exact` is **48×** raw doubles and `api-dbl` is **8.4×**, so the exactness tax — same API, different representation — is now **6×**, down from 33× before the `div` rewrite.

`div` is no longer the problem; **`sqrt` and `fromRatio` are.** `sqrt` at 1 747 ns is the most expensive single operation measured, and `Vec2::step_toward` calls it once per enemy per tick. That figure is also 8.7× what `[10] §F8`'s post-optimization table records for `sqrt` (200 ns) and deserves a look on its own — either the operand mix in the microbenchmark changed, or something regressed.

### Where the time goes (E=282, W=20, `exact`, the model)

| phase | ms/tick | share |
| --- | ---: | ---: |
| fire | 2.124 | 55.3 % |
| move | 1.595 | 41.6 % |
| checksum | 0.060 | 1.6 % |
| proj | 0.041 | 1.1 % |
| ranged / status / reap / spawn | <0.01 each | ~0.5 % |

The model still performs **1 570 weapon-range scans per tick** at E=282/W=20 because it still contains the `O(W×E)` rescan. **The shipped sim no longer does** — see the optimization section below — but the model keeps it deliberately, so its numbers stay comparable with every earlier run of that file.

### Replication

325 entities (the measured peak), one arena per client (`docs/09 §1.6`; F4's spectated arena makes it two). `UnreliableRemoteEvent` usable payload ~900 B, dropped above 1000 B.

| encoding | B/entity | total | payloads |
| --- | ---: | ---: | ---: |
| id u16 + type u8 + x,y i16 + hp u8 | 8 | 2600 | 3 |
| type u8 + x,y i16 + hp u8 | 6 | 1950 | 3 |
| type u8 + x,y i16 | 5 | 1625 | 2 |
| x,y i16 | 4 | 1300 | 2 |
| dx,dy i8 + flags u8 (delta) | 3 | 975 | 2 |

At 5 B/entity and 15 Hz: **23.8 KB/s per client**, 190.4 KB/s server-side for 8. Packing costs **0.013 ms** for 325 entities — 0.16 % of one core for all 8 arenas at 15 Hz. **Replication is still not the constraint.**

---

## Sim optimizations landed against these numbers

Three algorithmic fixes were ported from the Rust into the Luau sim. All three are **behaviour-preserving by construction**, and the proof is that the R3 gate passes *unchanged* — `smoke-seed-4`, `mid-seed-12`, `full-seed-0` and `full-seed-14` (55 200 ticks each) all still match the Rust checksum on every tick, alongside `parity.luau` (7 309 assertions) and `differential.luau` (1 769 712 comparisons).

| fix | file | effect |
| --- | --- | --- |
| Bounded k-smallest bounce chain, replacing a whole-board sort | `Combat.luau` | −0.41 % `Fixed` calls |
| Per-tick distance table + `nearestSq` `O(1)` range rejection | `Combat.luau` | −1.59 % |
| `moveSpeedMult` / `vulnerabilityMult` no-status fast paths, `reapDead` early-out and constant hoists, `Status.tick` no-timer skip | `Status.luau` | −3.08 % |
| **total** | | **−5.08 % `Fixed` calls, 1.086× wall clock** |

Two things are worth recording because they are not obvious:

- **Work was counted, not just timed.** Wall clock on a shared build container moves ±5 % run to run, which is larger than any of these fixes individually. Counting calls into the `Fixed` API instead gives a deterministic, reproducible measure of work — the Luau analogue of the Rust's instruction counts — and the three contributions above are exactly additive in it.
- **One of the Rust optimizations does not port as written.** `combat.rs` builds its per-tick distance table *eagerly* at the top of `fire_weapons`. Transcribed literally, that was measured as a net **loss** in Luau: +2.2 % `Fixed` calls, because the table is built on every tick while the scan it replaces only ever ran for weapons that were off cooldown. A `Fixed` multiply costs ~1000× the branch it saves, so Rust can afford to precompute unconditionally and Luau cannot. Building it on the *first ready weapon* keeps every saving and turns +2.2 % into −1.59 %.

---

## The verdict on R2

**R2 does not clear as measured, and should be recorded as FAIL-serial rather than as passed.**

- On the **`roblox` timeline the fork actually ships**, the mean tick fits serially (69.3 % of the conservative ceiling) but **sustained peaks do not** (248 % for the worst second). Every statistic fits in the parallel column, including the worst single tick at 64.2 %.
- On the **`steam` timeline** nothing fits serially, and the peaks do not fit even in the perfectly-parallel case.
- Truth is between the two columns, and **the serial column is the honest planning number** — Roblox sizes the parallel-Luau worker pool from the host and does not guarantee 8 threads.

The gap is not large, and it is not architectural. In rough order of cost-to-benefit:

1. **`--!native` on the arena Actor.** Not applied here at all; `--codegen` (the closest proxy) was worth ≈2.1× on this workload historically. That alone would put the `roblox` worst-second at 83–166 % — i.e. plausibly inside the budget, but *not demonstrated*.
2. **`sqrt`.** 1 747 ns, called once per enemy per tick by `Vec2::step_toward`, and 8.7× the figure `[10] §F8` records. The single largest unexplained cost in the profile.
3. **A spatial grid for `fire`**, which is 55 % of the model's tick and still `O(W×E)` in the candidate scan even after the `nearestSq` rejection.
4. **The §F5 escape hatch.** `api-dbl` passes serially at every measured population except the E=325/W=40 peak. It is a 6× win and it forfeits the seed-for-seed Rust oracle, so it is the last resort, not the first.
5. **Fewer than 8 arenas per server**, or a sub-30 Hz arena tick, which are product decisions rather than engineering ones.

**Status: R2 remains INCONCLUSIVE pending a Studio run — but it is now inconclusive on real numbers rather than stale ones, and the honest reading of those numbers is that it does not currently clear.**

## Caveats — what these numbers do and do not prove

- **This is not a Roblox server.** The standalone interpreter is the same language but not the same configuration: Roblox ships its own Luau build with different FFlags, its own allocator and a sandbox layer. Treat the absolute numbers as an order of magnitude. The *ratios between backends* are far more robust than the absolute values.
- **The `exact` rows are noisy.** The limb representation allocates a table per operation, so the `exact` backend is GC-bound and its timings depend on heap state — and this container is shared, so whole-match `realsim.luau` means moved ±5 % across three interleaved repeats of the identical build. That spread is larger than any single optimization landed above, which is why the optimization table is denominated in **counted `Fixed` calls** (deterministic) rather than in milliseconds. `api-dbl` and `raw-dbl` allocate nothing on the hot path and are stable to a few percent.
- **The hardware is a build container** (Xeon @ 2.10 GHz, 4 cores), not a Roblox server.
- **No `--!native`.** The `--codegen` column is the closest available proxy and shows ≈2.1×; real Roblox native codegen may differ.
- **No Actor scheduling.** Parallel Luau has real per-task dispatch and `synchronize` barrier costs this cannot see, and the worker pool is sized by the host machine — it is not guaranteed to be 8. The "parallel" column is a ceiling, the "serial" column a floor. **The serial column is the honest planning number.** Only the Studio harness (`run.server.luau`) can measure the truth, and it is written but *unvalidated* — it has never been executed against a real Roblox VM.
- **A Roblox server is not idle.** Replication, physics, character streaming and every other script share the frame. Hence the 30 % budget column.
- **`throughput.luau` pins population**, so its cost/entity curve is clean but artificial; a real arena oscillates as `Clear` wipes the board and it refills. `realsim.luau` does not pin anything — it reports the board the sim actually reaches, which is why it, not the model, is the gate.
- **The reference `Bot` is not a human.** It fires `Clear` off cooldown, which wipes the board and therefore *caps* a population a human could exceed by holding it. Two of the four seeds also die before the boss (at ticks 7 785 and 38 359 on the Steam timeline), so the peak figures come from the seeds that survive.
- **`realsim.luau` times `Arena.step` only.** `Bot.decide` is timed separately (0.0016 ms/tick — negligible, and only a backfill-bot seat pays it), and `View.snapshot` / `Wire.packDelta` are not in these ticks at all; replication is measured separately.
- **Not modelled:** hazards, minions, auras, vulnerability pulses and Fire chain explosions. All were zero across the 24-seed Rust profile, but a build stacking them adds `O(H×E)` and `O(M×E)` passes.
- `bench/FixedStub.luau` is a **stub** and must never ship. It exists so the bench runs before R0 lands and so the `api-dbl` comparison has something to measure. Its correctness gaps are listed in its header.

## Reproducing

```bash
# 1. The gate and the population it is sized against. This IS the ground truth
#    now — it runs the shipped sim, so it needs no Rust build and cannot drift
#    from what the game does:
cd roblox/bench
luau -O2 realsim.luau                 # 4 seeds x both timelines, ~15 min

# 2. The arithmetic model (backend ratios, per-op costs, replication)
luau -O2 throughput.luau              # interpreter — the numbers above
luau -O2 --codegen throughput.luau    # native codegen column

# 3. The correctness gates the optimizations above are justified by
cd ../test
luau -O2 differential.luau            # 1,769,712 comparisons vs FixedRef
tmp=$(mktemp -d); cat vectors/*.json > "$tmp/all"; split -b 100000 "$tmp/all" "$tmp/c."
args=(); for f in "$tmp"/c.*; do args+=("$(cat "$f")"); done
luau parity.luau -a "${args[@]}"      # 7,309 assertions vs the Rust
tmp=$(mktemp -d); split -b 100000 traces/full-seed-0.json "$tmp/c."
args=(); for f in "$tmp"/c.*; do args+=("$(cat "$f")"); done
luau trace_test.luau -a "${args[@]}"  # 55,200 ticks, per-tick checksum vs Rust

# 4. The bench, in Roblox (the only run that can settle the gate)
rojo serve roblox/default.project.json   # connect Studio, press Play, read the Output window
```
