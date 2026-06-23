# DESLOPPIFY — Cleanup Backlog

A prioritized cleanup backlog for Standing Tank Defense, produced from a read-only
desloppify scan (2026-06-23). Findings come from a parallel review of the four
major areas — the Rust **sim** core, the Rust **net** crate, the **Godot** client,
and **tooling/docs/CI** — plus a workspace build + full test run.

**Baseline health (verified this scan):** `cargo build --workspace` ✅,
`cargo test --workspace` ✅ — 162 sim unit tests, 57 net unit tests, and all
integration/chaos/soak tests pass. Nothing here is on fire; this is debt, not
breakage. The protocol↔doc seam (`wire_doc_sync`) and the determinism discipline
are genuinely well-kept — most findings are at the edges.

**Legend**
- **Where** — file:line (precise where possible).
- **Why** — the cost of leaving it.
- **Fix** — concrete recommendation.
- **When** — `NOW` (safe, localized, behavior-preserving or additive) ·
  `DECISION` (needs your call first) · `WAIT` (batch with a larger pass / not yet relevant).

How to use: pick an item by ID. I'll do that one, re-run build/tests, and redisplay
this list with the item checked off.

---

## 1. Critical

### C1 — Checksum omits 3 modifier fields the snapshot serializes  ·  `NOW`
- **Where:** `sim-core/crates/sim/src/lib.rs:203-208` (checksum block ends at `weapon_count_scaling`) vs `sim-core/crates/sim/src/snapshot.rs:302-304` (which *does* write `dmg_per_maxhp_rate`, `dmg_per_bounty_rate`, `shield_active_dmg`).
- **Why:** These three fields drive live damage. Two arenas that differ only in them are behaviorally different yet produce an **identical `state_checksum`** — exactly the desync the checksum exists to catch. It also breaks the checksum↔snapshot "mirror" both files' docstrings claim to maintain. This is a determinism-invariant violation (sacred per CLAUDE.md).
- **Fix:** Add three `c.write_fixed(...)` calls after the `weapon_count_scaling` loop, in the same order as the snapshot. Add a test asserting that mutating each field changes the checksum (existing roundtrip tests compare full equality, so they don't catch a checksum-coverage hole).
- **When:** NOW — small, localized, no snapshot-layout change (so no `SNAPSHOT_VERSION` bump).

### C2 — Match-length drift: code says 30 min, every doc says ~15 min  ·  `DECISION`
- **Where:** Code: `sim-core/crates/sim/src/content.rs:1350` — `BOSS_SPAWN_TICK = 30 * 60 * 30 // 54000 — 30 min`. Docs (all ~15 min): `docs/01:19-27`, `docs/02:11,26-28,87-88,94`, `docs/06:32,39`, `README.md:22`, `TUTORIAL.md`.
- **Why:** Match duration is the single most player-visible pacing number, and it silently doubled in code. Every design/tutorial/README reference now misstates core pacing; balance work and copy are anchored to the wrong number.
- **Fix:** Decide the canonical length. If 30 min is intended, update the six doc references (and the 10-/15-min scaling-step language); if 15 min, fix `content.rs` and the wave `start_tick`s.
- **When:** DECISION — needs your design call before any edit.

### C3 — `content_hash` join-gate is fake end-to-end  ·  `DECISION` (then `NOW` for the gate)
- **Where:** `docs/05:207` claims content is "hashed into the `content_hash` used at join time," but no hashing fn exists in `crates/sim`; `content_hash` is a caller-supplied `u64` literal (`net/director.rs:107`, `net/lobby.rs:132`, `replay.rs`). The **director ignores `Join.content_hash` entirely** (`net/director.rs:175`, with a comment deferring it), while `Lobby::join` *does* compare it — two divergent join paths, and the authoritative one trusts any joiner.
- **Why:** This is the anti-desync version gate at the untrusted-client seam — the architecture's whole reason for existing. As-is, two builds with different catalogs pass if handed the same constant, and a mismatched-content client can reconnect mid-match and receive a snapshot it will mis-simulate.
- **Fix:** (a) Implement a real catalog hash in `sim` (hash the weapon/enemy/modifier tables into `content_hash`); (b) validate `content_hash == self.content_hash` in the director's `Join` handler and drop on mismatch. If you'd rather defer (a), at minimum mark `docs/05 §5.7` as "planned," so the spec stops asserting it's implemented.
- **When:** DECISION on the hash implementation; the director-side gate (b) is a safe additive NOW once the value is real.

---

## 2. Medium

### M1 — Unbounded allocation on wire decode (host abort / DoS vector)  ·  `NOW`
- **Where:** `sim-core/crates/net/src/wire.rs:135-138` (`bytes` reads a `u32` length then `to_vec()`), `wire.rs:228-229` (`MatchResult` decode does `Vec::with_capacity(n)` on an unvalidated `u32`).
- **Why:** `take` is bounds-checked so it can't over-read, but a malformed/corrupt peer can declare a length up to 4 GB and force a giant allocation; the `with_capacity(n)` path aborts the process outright. Latent today (transport is in-process) but becomes a real single-packet host-kill the moment networked transport lands — and this crate is the netcode foundation.
- **Fix:** Cap `n` against a sane maximum (e.g. `MAX_PARTY` for places, a snapshot-size ceiling for bytes) before allocating; return `WireError` past the cap.
- **When:** NOW — small, and the soonest the security model gets real.

### M2 — `deserialize` doesn't validate catalog indices (panic on corrupt snapshot)  ·  `WAIT`
- **Where:** `sim-core/crates/sim/src/snapshot.rs` deserialize path; every `content::WEAPONS[def as usize]` / `ENEMIES[..]` / `MODIFIERS[..]` is an unchecked index (e.g. `combat.rs`, `modifiers.rs`).
- **Why:** Indices are safe when the sim allocated them, but a corrupt/malicious snapshot with an out-of-range `def` panics on first access. Snapshots flow director→client today, so it's not yet an untrusted path — but reconnect/replay-from-peer is on the roadmap.
- **Fix:** Range-validate every `def` against catalog length inside `deserialize`, returning an error rather than panicking.
- **When:** WAIT — wire it in for the milestone that accepts peer/replay snapshots; pairs naturally with M1.

### M3 — Director inbox: inconsistent unknown-peer handling + no snapshot rate-limit  ·  `NOW`
- **Where:** `sim-core/crates/net/src/director.rs:200-229`. Every arm re-runs `if let Some(i) = self.index_of(from)`; the `Input` arm sends an `InputAck` (`:207`) **even when the peer is unknown**, while `Digest`/`Join` silently drop unknowns. Separately, a known peer can spam `Join` to repeatedly pull full snapshots on the reliable Bulk channel with no server-side throttle.
- **Why:** Behavior/info asymmetry (an unknown peer learns its seq was "accepted"), and an unbounded-Bulk-traffic lever — both at the trust boundary.
- **Fix:** Resolve `index_of(from)` once at the top of the loop body, `continue` on `None`, then handle the message uniformly. Track last-snapshot iter per player and throttle re-joins.
- **When:** NOW for the membership-check refactor; the throttle can WAIT until real transport.

### M4 — Hand-rolled codec scaffolding triplicated across three modules  ·  `NOW`
- **Where:** Near-identical reader/writer structs in `net/src/wire.rs` (`W`/`R`), `net/src/lobby.rs` (`LobbyW`/`LobbyR`), and `net/src/replay.rs` (`Reader` + free `encode_action`/`decode_action`). `replay.rs`'s action codec is a byte-for-byte duplicate of `wire.rs`'s `W::input`/`R::input`, and the `InputCode` tag mapping is defined twice.
- **Why:** Three little-endian codecs that must stay in lock-step; the duplicated `InputCode` tags can silently diverge and corrupt replays/inputs.
- **Fix:** Extract the `W`/`R` primitives into a shared zero-dep `codec` module; make `InputCode` encode/decode live in exactly one place. Roundtrip tests are the guard.
- **When:** NOW, but touches three files — do it as one isolated change.

### M5 — `main.gd` is a 949-line god-object  ·  `WAIT`
- **Where:** `godot/main.gd` (entire file) — sim-stepping (`_physics_process`), lighting/FX (`_process`), juice diffing, audio diffing, and the whole immediate-mode HUD/shop/results/tooltip renderer (`_draw_*`) all in one Node2D.
- **Why:** Any HUD tweak risks the sim loop and vice-versa; the file will only grow.
- **Fix:** Extract the immediate-mode HUD/shop/results renderer into a `Hud`/`ShopBar` helper (like the existing `Fx`), and pull the lighting rig into a reusable `ArenaLighting` helper `match.gd` can share.
- **When:** WAIT — cosmetic refactor; do it before the HUD grows further, not under deadline.

### M6 — Theme/cosmetic logic triplicated across screens  ·  `NOW`
- **Where:** `match.gd:74-111`, `lobby.gd:85-100`, `theme.gd:117-123`, `skin_select.gd:28-33`, `challenge_select.gd:23-28`. `_theme_base`/`_theme_load`/`_tank_for` (the "skins/<file> if exists else player_tank.svg" rule) and the 12-skin `PEER_SKINS`/peer-name rosters (`match.gd:61-65` vs `lobby.gd:40-44`) are copy-pasted verbatim.
- **Why:** The peer rosters and skin-resolution rule will drift between screens.
- **Fix:** Add `ArtTheme.tank_tex_for(theme_idx, skin_id)` and shared `PEER_SKINS`/`PEER_NAMES` consts (in `Profile` or `ArtTheme`); route all callers through them.
- **When:** NOW — pure behavior-preserving extraction.

### M7 — Audio buy/reroll detection trusts a gold-delta proxy  ·  `WAIT`
- **Where:** `godot/main.gd:505-512`. A buy is voiced only if `gold < gold_before` (a 0-cost/free item stays silent); a reroll is always voiced on `intent==2` even when the sim rejected it.
- **Why:** The renderer infers sim outcomes from one scalar instead of being told what happened — wrong/missing SFX on edge cases, and brittle to future free items.
- **Fix:** Have `StSim.step` return a small result code (bought/rerolled/rejected) the renderer reads.
- **When:** WAIT — needs a Rust-binding change; current placeholder audio is "good enough."

### M8 — Two `steam_appid.txt`, both the `480` placeholder  ·  `DECISION`
- **Where:** `/steam_appid.txt` and `/godot/steam_appid.txt`, both contain `480` (Valve's public Spacewar test ID).
- **Why:** Not a secret leak, but a duplicated placeholder that must be replaced before ship and can diverge. The Godot runtime needs its copy next to the binary; the root copy's purpose/consumer is unclear.
- **Fix:** Confirm whether the root copy is actually consumed; keep one canonical location, document why each exists, add a "replace before release" note.
- **When:** DECISION — needs a quick confirm of the root copy's consumer first.

### M9 — Doc↔code drift in the data-model/spec (cluster)  ·  `NOW` (doc-only)
- **Where & what:**
  - **PRNG named wrong:** `docs/05:183` says "PCG32 or xoshiro256\*\*"; code is **SplitMix64** (`determinism/src/lib.rs:11,154`). Any external replay-verifier built to the spec desyncs on every draw.
  - **Enemy/boss names not IP-cleansed in docs:** code renamed to original IP ("The Hippocrate", "Squeakzilla", etc.) but `docs/01,02,05,06`, `README.md:22`, `docs/04:41` still say "Samwise", "Fel Orc Grunt", "felorc_grunt" — partially defeating the `CREDITS.md` legal-hygiene claim.
  - **Tank/state schema stale:** `docs/05:26-34` shows `Fixed` fields and a `mana_shield{...}` object; code uses `i64`/`u32` and flat fields with no `active` (`state.rs:69-149`). Integer-vs-`Fixed` is determinism-relevant.
- **Why:** The spec misrepresents the authoritative shapes and names; readers can't trace docs to real content rows, and an alt-client built to spec breaks determinism.
- **Fix:** Refresh `docs/05 §5.6` (SplitMix64), the name references (or add a "formerly X" mapping table), and the Tank schema block to match `state.rs`.
- **When:** NOW — doc-only; batch into one pass. (Hold the name changes that overlap C2's doc pass.)

---

## 3. Nice-to-have

### Sim core
- **N1 — `Rng::derive` throwaway binding obscures a frozen step**  ·  `NOW` · `determinism/src/lib.rs:173-181`. `let mut r = Rng{state:s}; s = r.next_u64();` reads like `r` is reused. Replace with a direct expression and a comment that the exact sequence is frozen — must not change output.
- **N2 — `economy.rs` take-restore-clear is dead motion**  ·  `NOW` · `economy.rs:24,37-38`. `pending_kills` is taken, restored, then cleared; just clear the field directly.
- **N3 — `tick % cadence == 0` burst fragility**  ·  `WAIT` · `waves.rs:33,57`. At tick 0 (and the boss tick) every ungated/divisible entry fires at once — currently intended, but adding content can produce an unintended burst. Phase-offset or gate `start_tick >= 1`. Balance-adjacent; flag for the balance pass.
- **N4 — `Box::leak` in test helpers**  ·  `NOW` · `modifiers.rs:325`, `net/client.rs:374-380`. Test-only leaks to dodge the borrow checker; harmless but a copy-able bad pattern. Restructure to borrow/collect by value.
- **N5 — `% 5` mask silently mis-buckets `damage_type`**  ·  `NOW` · `modifiers.rs:94,146,300`. Hides an out-of-range type as type 0 instead of failing. `debug_assert!(dt < 5)` and drop the mask.
- **N6 — Linear `find` per projectile/minion**  ·  `WAIT` · `combat.rs:395-399,465,665`. O(projectiles×enemies)/tick; fine at M0 counts. If profiling shows it, binary-search the id-sorted enemy vec. Don't pre-optimize.

### Net
- **N7 — `Schedule::discard_before` is dead code**  ·  `NOW` · `schedule.rs:33`. Never called; its intended snapshot-adopt cleanup (`client.rs:148,165`) is missing. Either wire it in or delete it.
- **N8 — `Snapshot.tick` is redundant with the embedded arena tick**  ·  `WAIT` · `director.rs:217-224`, ignored by `client.rs:142`. Two sources of truth for "what tick is this snapshot." Drop the field or document it as advisory. Protocol change — batch with other `[04]` revisions.
- **N9 — Driver-counter naming: `iter`/`step`/`arena_tick`/`it`**  ·  `WAIT` · director/client/hub/tests. Three names for the tick-alignment clock raise cognitive load on the most important invariant. Standardize on `iter`.
- **N10 — `net::steam` duplicates the adapter's narrative**  ·  `NOW` · `net/src/steam.rs:8-18` vs `adapters/steam-transport/src/lib.rs:13-29`. The module is mostly a prose copy of the real adapter (plus a few constants the adapter depends on). Shrink it to the constants/flag-fn.

### Godot
- **N11 — Orphan `_cap_node.gd.uid` (no matching script)**  ·  `NOW` · `godot/_cap_node.gd.uid`. UID referenced nowhere; leftover from a deleted script. Delete it.
- **N12 — `Fx.draw()` doc signature is wrong**  ·  `NOW` · `fx.gd:13` documents `draw(canvas, to_screen, font)`; real signature is `draw(canvas, font)` (`fx.gd:151`). A future caller will pass a bad arg. Fix the doc block.
- **N13 — Achievement unlock `print()` in single-arena vs toast in multi**  ·  `NOW` · `main.gd:319` prints to console; `match.gd:166,196-203` shows a toast. The results panel (`main.gd:691-700`) already renders unlocks, so the print is redundant — drop it.
- **N14 — Magic numbers / duplicated world-scale constant**  ·  `WAIT` · `main.gd:516,535,615`, `match.gd:251,276`. The `1700.0` scale divisor and ring formula are duplicated across files and must agree silently. Hoist shared `WORLD_RADIUS`/`WORLD_SCALE_REF` consts.
- **N15 — No friendly error when the GDExtension class is missing**  ·  `WAIT` · `main.gd:62`, `match.gd:39,42`, `lobby.gd:56`. `StSim.new_match(...)` etc. throw "identifier not found" if the Rust core wasn't built. Add a `ClassDB.class_exists("StSim")` guard that shows a "build the Rust core first" screen.
- **N16 — Hardcoded key bindings + key hints baked into copy**  ·  `WAIT` · `main.gd`/`lobby.gd`/`match.gd` `match e.keycode` blocks; `[Enter]`/`[Space]` literals in translated strings. Can't be rebound and bypass accessibility. Migrate to named InputMap actions; derive displayed hints from them.
- **N17 — `tr("...%d...") % v` + concatenated translated fragments are fragile for zh-CN**  ·  `WAIT` · `main.gd:622,798-800`, `lobby.gd:263-264,379`, `match.gd:185-187,341`. Translators must preserve specifier order/count; concatenated control-hint fragments can't localize as units. Add a CI check for matching `%`-specifier counts per locale row; fold concatenated hints into single keys.
- **N18 — `master_volume` audio API is dead code**  ·  `WAIT` · `audio.gd:116-123`. No caller, no volume UI (only mute is wired). Expose a settings slider or drop the API.
- **N19 — Shop-category classification by English substring match**  ·  `WAIT` · `main.gd:921-929`. Categories derived by scraping English item text (works pre-`tr()` today). Have Rust `shop_meta` expose a category enum instead.
- **N20 — Inconsistent input handlers (`_input` vs `_unhandled_key_input`)**  ·  `WAIT` · menus use `_input`, gameplay uses `_unhandled_key_input`. Harmless now (no focusable Controls) but will swallow events once real Controls exist. Standardize on `_unhandled_key_input`.

### Tooling / docs / config
- **N21 — Python scripts: no error handling; `gen_catalog` clobbers curated `content.rs`**  ·  `WAIT` · `extract.py:51,63,106` use bare `open()` without `with`; `gen_catalog.py:206-223` `splice` does `src.index(marker)` (opaque `ValueError` if a GEN marker is missing) and overwrites curated `content.rs` with only a printed warning. Use `with`, wrap index lookups with a clear error, write a `.bak`. One-shot tooling, low blast radius.
- **N22 — `gen_catalog.py` hand-dedup tuple list is fragile**  ·  `WAIT` · `gen_catalog.py:88-99` hard-codes exact stat tuples of the 10 base weapons to skip; tuning any base weapon silently re-emits duplicates. Gated behind `--force`, so acceptable — document the coupling.
- **N23 — `CLAUDE.md` still frames the project as a spec package**  ·  `NOW` · `CLAUDE.md:7` ("Currently a specification package… moving toward implementation"). README was updated; CLAUDE.md wasn't. Update the framing.
- **N24 — README crate index incomplete**  ·  `NOW` · `README.md`. Omits the `preview` crate, the `net` submodules (`lobby/replay/results/steam`), and the `adapters/steam-transport` crate. Complete the table.
- **N25 — Godot version anchors inconsistent**  ·  `NOW` · `.gdextension:4` (`compatibility_maximum = 4.5`), `windows-release.yml:16` (`4.3-stable`), `TUTORIAL.md:16` ("tested on 4.4.1"). Not a bug, but three anchors — pick one source of truth and note it.

---

## Clean — verified, no action (context for the backlog)

- **Determinism discipline holds** beyond C1: no floats in any non-test hot path, no `HashMap` in checksum-feeding paths (`BTreeMap` throughout), all checksum/snapshot iteration is id-sorted, no wall-clock or platform RNG, `Fixed`/gold/stat math saturates or clamps. C1 is the lone hole.
- **Wire decode is genuinely bounds-safe** (`checked_add` + slice `get`, no `unwrap` on attacker bytes); `BuyOffer{slot}` is bounds-checked in `input.rs:33`; client `Snapshot` is not even an inbound wire variant. The M1/M2/M3 items are hardening at the edges, not open holes today.
- **The protocol↔doc seam is the best-kept in the repo** — `wire_doc_sync` mechanically enforces `docs/04` ↔ `wire.rs` in all three directions. A model for the other seams.
- **No tracked secrets/junk**; CI uses least-privilege scoped permissions; `.gitignore` coverage is reasonable.
- **Naming is consistently snake_case** in GDScript; immediate-mode rendering avoids fragile `get_node` paths by design.
