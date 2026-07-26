# R4–R6 — Phase 2 Contracts

Central seams for the shell, multiplayer, meta and platform layers. Owned centrally. **Flag gaps; do not widen them.** Background: `docs/10-roblox-fork-design.md` (F1–F9), `roblox/SIM-SPEC.md`, `roblox/CONTRACTS.md`.

R0–R3 are **done and verified**: the Luau sim reproduces the Rust bit-for-bit for 55,200 ticks (`roblox/test/trace_test.luau`). Everything here builds on a sim that is known-correct — do not modify `src/shared/sim/**` or `src/shared/{Fixed,Rng,Json,Checksum,Content,ContentData}.luau` except where a task explicitly owns a file.

## P0 — THE RULE (non-negotiable, learned the hard way)

**Never create, modify or delete any file under `/home/user/standingtankdefense/` that your task does not explicitly own — not even temporarily, not even if you intend to delete it.** Eight agents previously wrote throwaway stubs into the live tree and destroyed each other's work; two modules had to be recovered from scratch copies.

If you need a sandbox, copy what you need to
`/tmp/claude-0/-home-user-standingtankdefense/d7ede231-09e9-5d42-8af3-755591abd79a/scratchpad/<your-task-name>/`
and work there. Every sim module is real, landed and passing — you should not need stubs at all.

## P1 — Layout and ownership

```
roblox/src/shared/         sim core (DONE — do not touch)
roblox/src/shared/net/     Wire.luau            [Replication]
roblox/src/server/         Arena runner, Director, Meta   [Server / Meta]
roblox/src/client/         rendering + UI                 [Client]
roblox/build/              require-rewriting build step    [Platform]
```

Rojo maps `src/shared` → `ReplicatedStorage.Shared`, `src/server` → `ServerScriptService`, `src/client` → `StarterPlayerScripts` (`CONTRACTS.md` C5).

## P2 — The require problem (blocking, owned by Platform)

`src/shared/**` uses `require("../Fixed")` / `require("./State")`. That is the **only** form the standalone `luau` interpreter accepts, and Roblox accepts **none** of it — Roblox needs instance references (`require(script.Parent.Fixed)`).

Both must keep working: bare-interpreter execution is how the entire test suite and the R3 oracle gate run, and losing it would forfeit the correctness guarantee. So the resolution is a **build step that emits Roblox-shaped copies**, not an edit to the modules.

## P3 — Timeline selection (owned by Timeline task)

The sim currently runs the **Steam** timeline everywhere (`Arena.ROUND_TICKS = 900`, `Waves`/`Content` default to `TIMELINE.steam`). That is correct and must stay the default, because the R3 oracle is the Rust, which is hardcoded to Steam.

F1's Roblox timeline (boss 9000, 600-tick rounds, 900-tick ramp interval, gates rescaled by `tick_scale_num/den`) exists **only as data**. Activating it is a *runtime* choice carried on the state, never a compile-time swap:

- `state.timeline` is the single source of truth; `nil` means Steam.
- Nothing may hardcode either timeline. `Arena.ROUND_TICKS` must become a lookup.
- **The Steam path must remain byte-identical** — the R3 gate must still pass unchanged. That is the acceptance test.

## P4 — Wire contract (owned by Replication)

Server simulates all arenas (F6); clients render only. Each client receives **its own arena's view**, plus optionally one spectated arena (F4).

- Pack from `View.snapshot(state)` into a `buffer`. Never send tables of entities.
- Budget: `UnreliableRemoteEvent` caps at ~900 bytes/payload (dropped above 1000). Real peak is ~67 enemies / ~84 projectiles, so a 3-byte-per-entity delta encoding fits comfortably (`docs/09 §1.6`, `docs/10 §F8`).
- Send at **10–15 Hz**, not 30; clients interpolate.
- Reliable channel for low-rate truth: round clock, alive/dead, leaderboard, shop offers.
- `Wire.luau` lives in `src/shared/net/` and must be **pure** — no Roblox globals — so it is testable under the bare interpreter. The Roblox-specific send/receive glue lives in server/client files.

## P5 — Authority

Clients are never trusted. The only thing a client sends is an **input code** (`SIM-SPEC` §S6: `kind * 256 + slot`). The server validates and applies it. No client-authored positions, damage, gold or offers — ever.

## P6 — Report contract

Report: the public API you exposed; how you verified it (actually run things — the interpreter is at `/tmp/claude-0/-home-user-standingtankdefense/d7ede231-09e9-5d42-8af3-755591abd79a/scratchpad/luau/build/luau`, analyzer alongside it); what you could not verify without a real Roblox host, stated plainly; and any contract gap — flagged, not widened.
