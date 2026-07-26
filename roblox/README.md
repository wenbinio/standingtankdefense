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

### Standalone — no Roblox required

Any Luau interpreter with `buffer`, `bit32` and require-by-string. Build one from [luau-lang/luau](https://github.com/luau-lang/luau) (`cmake --build . --target Luau.Repl.CLI`), then:

```bash
cd roblox/bench
luau -O2 throughput.luau              # ~30 s
luau -O2 throughput.luau -a 0.15      # shorter: ~0.15 s per timed measurement
luau -O2 --codegen throughput.luau    # with native codegen, the closest proxy for --!native
```

It auto-detects the absence of Roblox and runs immediately. It picks up the real `../src/shared/Fixed.luau` if present and falls back to `bench/FixedStub.luau` otherwise, printing which one it used.

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

## What the bench measures

| Section | Question |
| --- | --- |
| 0 | What *is* the worst-case wave? Steady-state population derived from the real cadences. |
| 1 | Per-tick cost of one arena, and the 8-arena-at-30 Hz extrapolation as a % of budget. |
| 1b | Which phase spends the time. |
| 2 | What exactness costs: per-op and whole-workload, exact `Fixed` vs plain doubles. |
| 3 | Replication: bytes/tick for 300 packed positions vs the ~900 B `UnreliableRemoteEvent` cap. |
| 4 | The gate: PASS/FAIL per backend, serial and parallel. |
| 5 | The caveats that qualify all of the above. |

**The workload is sized from the real tables, not invented.** Spawn cadences, ring radii, movement speeds, the damage matrix and the 20-weapon arsenal are transcribed from `sim-core/crates/sim/src/{content.rs,waves.rs}`, and the per-tick phases mirror `sim::step`'s phase order. Population is derived by Little's law over the real cadences (`pop = Σ 1/cadence × radius/speed`) rather than guessed.

That derivation is checked against the Rust. Driving the real `sim::step` with the built-in `sim::bot::Bot` across 24 seeds to the boss tick and recording per-tick maxima gives a **peak of 67 live enemies**; the model puts the boss-escort steady state at **67.6**. Same scan: max 84 projectiles, max 20 weapons, 0 hazards, 0 minions, mean 14.3 enemies, and 2.5 µs/tick in Rust (averaged over whole runs, so at a much lower mean population than the bench pins — an order-of-magnitude anchor, not a like-for-like comparison).

Three numeric backends run the *identical* algorithm:

- **`exact`** — the real §C2 `Fixed` (four 16-bit limb tables, bit-identical to Rust).
- **`api-dbl`** — the same §C2 API over a plain-double representation. This is the escape hatch `docs/10 §F5` explicitly reserves: *"the module's internals can be swapped to plain doubles without touching a single caller."*
- **`raw-dbl`** — no `Fixed` at all. The floor.

---

## Measured results

Standalone `luau -O2`, Intel Xeon @ 2.10 GHz, 4 cores. One representative run; the `exact` figures move ±30 % between runs (see caveats). **Read the caveats below before quoting any of this.**

### The gate

Budget: one 30 Hz sim tick is **33.33 ms** of wall clock for all 8 arenas. A server also replicates, runs physics and runs every other script, so the honest ceiling for simulation is taken as **30 % of that = 10.00 ms**.

| backend | scenario | serial (1 thread) | parallel (8 Actors) |
| --- | --- | --- | --- |
| `exact` | realistic E=68 W=20 | **376 %** FAIL | 47 % PASS |
| `exact` | worst case E=300 W=20 | **1566 %** FAIL | **196 %** FAIL |
| `api-dbl` | realistic E=68 W=20 | 8.6 % PASS | 1.1 % PASS |
| `api-dbl` | worst case E=300 W=20 | 48 % PASS | 6.0 % PASS |
| `raw-dbl` | realistic E=68 W=20 | 1.6 % PASS | 0.2 % PASS |
| `raw-dbl` | worst case E=300 W=20 | 7.1 % PASS | 0.9 % PASS |

### Per-tick cost, one arena

| backend | E | W | ms/tick | 8 arenas serial | % of 33.33 ms | with `--codegen` |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `exact` | 68 | 20 | 4.698 | 37.58 | 113 % | 2.124 |
| `exact` | 300 | 20 | 19.576 | 156.61 | 470 % | 9.360 |
| `exact` | 675 | 20 | 43.487 | 347.90 | 1044 % | 21.440 |
| `api-dbl` | 68 | 20 | 0.108 | 0.86 | 2.6 % | 0.040 |
| `api-dbl` | 300 | 20 | 0.600 | 4.80 | 14.4 % | 0.172 |
| `api-dbl` | 675 | 20 | 1.939 | 15.51 | 46.5 % | 0.465 |
| `raw-dbl` | 300 | 20 | 0.088 | 0.71 | 2.1 % | 0.033 |

E=68 is the measured Rust peak. E=300 is `docs/09 §1.6`'s stated worst case. E=675 is the analytic ceiling with a max-Frost build (25 stacks → 10 % speed floor → 10× lifetimes).

Native codegen buys a consistent **≈2.1×**.

### What exactness costs

| op | `exact` ns | native double ns | ratio |
| --- | ---: | ---: | ---: |
| add | 245 | 1.09 | 225× |
| sub | 228 | 1.45 | 157× |
| **mul** | **736** | 0.95 | **778×** |
| **div** | **17 718** | 1.24 | **14 236×** |
| scaleI64 | 945 | 4.74 | 199× |
| sqrt | 1 853 | 3.31 | 559× |
| fromRatio | 18 656 | 1.47 | 12 657× |
| compare | 171 | 1.59 | 108× |

Whole workload at E=300/W=20: `exact` is **221×** raw doubles; `api-dbl` is **6.8×**. The exactness tax — same API, different representation — is **33×**.

`Fixed.div` and `Fixed.fromRatio` (which is implemented on top of `div`) are **24× the cost of `Fixed.mul`**, and they dominate. The cause is algorithmic, not incidental: `divMag` is a bit-by-bit restoring binary division, 80 iterations each doing a 5-limb shift, compare and conditional subtract. That is a correct and defensible first implementation, and it is also the single biggest lever in the whole system.

### Where the time goes (E=300, W=20, `exact`)

| phase | ms/tick | share |
| --- | ---: | ---: |
| move | 13.87 | 72 % |
| fire | 4.91 | 26 % |
| proj | 0.33 | 1.7 % |
| checksum | 0.07 | 0.4 % |
| ranged / status / reap / spawn | <0.01 each | ~0 % |

`move` is 72 % because `Vec2::step_toward` does two `Fixed::div` per enemy per tick. 300 enemies × 2 divs × 17.7 µs ≈ 10.6 ms — the whole phase, in two operations.

`fire` performs **1670 weapon-range scans per tick** at E=300/W=20, far above the naive `W×E/cooldown`. `combat.rs::fire_weapons` deliberately does *not* advance a weapon's cooldown when nothing is in range ("do not fire, do not advance cooldown"), so a short-range weapon on a board whose enemies are all still marching in re-scans every enemy every tick. That converts an amortised cost into a true `O(W×E)`.

### Replication

300 entities, one arena per client (`docs/09 §1.6`; F4's spectated arena makes it two). `UnreliableRemoteEvent` usable payload ~900 B, dropped above 1000 B. Positions live in ±1800 units, so a whole-unit `i16` is lossless.

| encoding | B/entity | total | payloads | entities/payload |
| --- | ---: | ---: | ---: | ---: |
| id u16 + type u8 + x,y i16 + hp u8 | 8 | 2400 | 3 | 112 |
| type u8 + x,y i16 + hp u8 | 6 | 1800 | 2 | 150 |
| type u8 + x,y i16 | 5 | 1500 | 2 | 180 |
| x,y i16 | 4 | 1200 | 2 | 225 |
| dx,dy i8 + flags u8 (delta) | 3 | 900 | **1** | 300 |

At 5 B/entity and 15 Hz: **22.0 KB/s per client**, 175.8 KB/s server-side for 8 clients. Packing costs **0.014 ms** for 300 entities — 0.16 % of one core for all 8 arenas at 15 Hz, i.e. free.

**Replication is not the problem.** No encoding fits 300 entities in one 900 B payload except the 3 B delta form, but 2 payloads at 15 Hz is unremarkable, and the delta encoding gets it to 1 if wanted. The bandwidth is an order of magnitude below the 100 KB/s the community 2000-NPC stress test cited in `docs/09 §1.3` sustains.

---

## The verdict on R2

**Simulating 8 arenas at 30 Hz is comfortably affordable. Doing it through the exact `Fixed` emulation is not.**

- With `raw-dbl` or `api-dbl` arithmetic the gate passes with an order of magnitude to spare, at every population up to the analytic Frost-build ceiling, even serialised onto a single thread.
- With `exact` arithmetic the gate fails serially at every population, and fails even in the perfectly-parallel case at E=300.
- The gap is **33×** and it is concentrated in two operations, `div` and `fromRatio`.

This is not a verdict against the architecture — the sharded-arena design is fine and the algorithmic shape is cheap. It is a verdict about one module, and `docs/10 §F5` already anticipated it: *"If the throughput spike shows fixed-point math is the bottleneck, the module's internals can be swapped to plain doubles without touching a single caller."* The spike shows exactly that.

Before taking that swap, three cheaper things are worth trying, in order — they may preserve the seed-for-seed correctness oracle (`docs/09 §1.5`), which the swap forfeits:

1. **Replace `divMag` with limb-wise division** (Knuth Algorithm D, or a reciprocal-multiply for the common case). Restoring binary division is the worst available algorithm here; a 10–20× win on `div` alone is realistic, and it moves `exact` from 470 % to roughly 100 % of a whole tick budget at E=300.
2. **Remove `div` from the hot path.** `step_toward`'s two divisions per enemy per tick are the whole `move` phase. A reciprocal computed once per enemy, or a normalised-direction cache, replaces two divides with one divide plus two multiplies.
3. **Kill the `O(W×E)` rescan** in `fire_weapons` with a spatial grid or a single shared per-tick distance pass. This is backend-independent and helps all three.

Only if those fall short should the representation change. `--!native` is worth roughly 2.1× on top of any of them, and is free.

**Status: R2 is INCONCLUSIVE pending a Studio run — but the decision it gates is already clear.** The standalone numbers cannot be the gate result (see caveats), yet the 33× ratio between backends is a property of the code, not of the host, and will not invert on Roblox hardware.

---

## Caveats — what these numbers do and do not prove

- **This is not a Roblox server.** The standalone interpreter is the same language but not the same configuration: Roblox ships its own Luau build with different FFlags, its own allocator and a sandbox layer. Treat the absolute numbers as an order of magnitude. The *ratios between backends* are far more robust than the absolute values.
- **The `exact` rows are noisy — roughly ±30 % run to run.** The limb representation allocates a table per operation, so the `exact` backend is GC-bound and its timings depend on heap state. Observed across repeat runs, `exact` at E=68/W=20 ranged 292–376 % of the 30 % budget serially. The conclusion is insensitive to that spread; individual digits are not. `api-dbl` and `raw-dbl` allocate nothing on the hot path and are stable to a few percent.
- **The hardware is a build container** (Xeon @ 2.10 GHz, 4 cores), not a Roblox server.
- **No `--!native`.** The `--codegen` column is the closest available proxy and shows ≈2.1×; real Roblox native codegen may differ.
- **No Actor scheduling.** Parallel Luau has real per-task dispatch and `synchronize` barrier costs this cannot see, and the worker pool is sized by the host machine — it is not guaranteed to be 8. The "parallel" column is a ceiling, the "serial" column a floor. **The serial column is the honest planning number.** Only the Studio harness (`run.server.luau`) can measure the truth, and it is written but *unvalidated* — it has never been executed against a real Roblox VM.
- **A Roblox server is not idle.** Replication, physics, character streaming and every other script share the frame. Hence the 30 % budget column.
- **Population is pinned**, so the cost/entity curve is clean, but a real arena oscillates: `Clear` wipes the board and it refills.
- **Not modelled:** hazards, minions, auras, vulnerability pulses and Fire chain explosions. All were zero across the 24-seed Rust profile, but a build stacking them adds `O(H×E)` and `O(M×E)` passes.
- `bench/FixedStub.luau` is a **stub** and must never ship. It exists so the bench runs before R0 lands and so the `api-dbl` comparison has something to measure. Its correctness gaps are listed in its header.

## Reproducing

```bash
# 1. Entity-count ground truth from the Rust sim. `preview` prints live enemy
#    counts and the ending arsenal at sampled frames (this is where the
#    20-weapon loadout in the bench comes from):
cd sim-core && cargo build --release -p preview
./target/release/preview --max-ticks 54600 --every 5400 --frames 12
#    The 24-seed per-tick MAXIMA (67 enemies / 84 projectiles / 20 weapons /
#    0 hazards / 0 minions) came from a throwaway driver that steps
#    `sim::step` with `sim::bot::Bot` and records maxima. It is not checked in
#    — ~40 lines against the public `ArenaState` fields reproduces it.

# 2. The bench, standalone
cd roblox/bench
luau -O2 throughput.luau              # interpreter — the numbers above
luau -O2 --codegen throughput.luau    # native codegen column

# 3. The bench, in Roblox (the only run that can settle the gate)
rojo serve roblox/default.project.json   # connect Studio, press Play, read the Output window
```
