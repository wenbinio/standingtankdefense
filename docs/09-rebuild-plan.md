# [09] Full Rebuild Plan — Presentation Overhaul & Fix Program

**Status:** P0, P1, P3 (arena + net-view juice), P5 items 1–5 (Clear cooldown, timers/boss bar, pause+confirm, settings, net-view results), P4 systems layer, projectile art variety, and flipbook animation support all complete. Remaining: P2 art production (batches A–E; the strip-authoring contract lives in `theme.gd`), P4 sound assets/music, P5 minor items (keyboard tooltips, colorblind marks, weapon detail/range rings), P6 ship track. Standing caveat: all landed via static verification — first live editor run should regenerate `.translation` imports and eyeball the risks listed in each commit.

> **Fidelity program executed (2026-07, follows the fidelity-gap audit):** three waves aligning the game to the original Tower Survivors on the user-chosen axes (pacing/arc, weapons+upgrades, economy/rounds), IP-free. Wave 1: rarity-weighted mixed-slot shop, bonus-only income scaling, held Magic Treasure, source perk scoping; plus 15 engine mechanics the source content requires (combined attack shapes, fixed-rate waves, once-per-round activation, typed vulnerability, miss-chance, status-on-damaged armors, spikes extensions, opt-in Deep Freeze, rotating waves, free-reroll grants, conditional/self-scaling damage; snapshot v21). Wave 2: catalog re-anchored to the extraction — 14 same-name identity fixes, restored dropped weapons/upgrades, source cooldown/range tiers, dedupes → **96 weapons / 110 modifiers**. Wave 3: the source 15-minute arc restored (BOSS_SPAWN_TICK 27000, +20% step at 10:00, swift-end escalation, ramp-stop and shop-close at the boss, escort removed in favor of continuing scaled waves), boss re-statted to a 10-Clear window, full sweep retune to **21.2%** bot win-rate (band 17–23%, all archetypes viable), soak now proves a boss kill through the netcode. Deferred items — **all closed in a follow-up pass**: (1) Black Market is a true picker — `Input::BlackMarketPick` on the wire (input tag 4, docs/04 graduated, `wire_doc_sync` green), held-until-redeemed per the source text ("Uncommon Weapon or Spikes Damage Upgrade of your choosing"), director-validated with deterministic no-ops, bot pick policy, and a modal shop overlay with a dismissible pending badge; (2) game speed — host-set in the lobby, fixed at match start on `Msg::MatchStart` as exact integer tick rates (30/45/60/90), speed-aware clock sync, a Normal-vs-Hyper checksum-equality test proving cadence never touches state, integer tick-accumulator stepping in both views, and a single-player Speed setting; (3) Battle Fervor's +35% scoped to healing weapons (`WeaponDef::is_healing`, `HealingWeaponDamagePct`) per the source text. Snapshot v22; golden after the full program: `0x7b9690c6d0e661f0`; suite 338 tests. Produced from six parallel audits (art, VFX/game-feel, UI/UX/frontend, Rust sim/net core, audio, docs-vs-implementation) run against commit `e656163`, with the full test suite executed live (at audit time: 254 tests pass, M0 gate PASS, golden checksum `0x436067271f1956d8`).

> **P0 executed (2026-07):** all §9.1 defects fixed; docs truth pass landed with mechanical doc-sync tests; checksum coverage expanded beyond B4 (ten snapshot-carried fields total — the parity test found nine more of the same class), which deliberately re-baselined the golden checksum to `0x58d77dad0cc11999` per the documented runbook. Suite now **269 tests**, fmt/clippy/CI hygiene gates active. Pending: one Godot editor import pass to regenerate two stale `.translation` rows.

> **P1 executed (2026-07):** §9.3 event stream live end-to-end (record layout documented in `godot/rust/src/lib.rs`; `Projectile::weapon_kind` added → snapshot v20, golden re-baselined to `0x09058789ad2df90a`; suite 286 tests). Frontend restructured (main.gd 969→276 + arena_renderer/hud/shop/results/fx_overlay/sim_view modules), InputMap physical-key actions, `canvas_items` stretch, Camera2D shake, prev/curr tick interpolation, MultiMesh entity rendering, pooled FX. Audio systems layer (P4 items 1–5): jitter, limiter, ducking, `play_many` count scaling (wired for kills), crossfade + vertical-intensity music. Presentation consumes events — diff-inference deleted. Known follow-ups: Impact events carry a point, not a victim id (dense-clump flash mapping is approximate — candidate v2 event field); per-impact damage numbers may need a display threshold; all noted "needs live editor" risks from the restructure.

---

## 9.0 Scope decision: what "rebuild" means here

The Rust core (sim + netcode) is **not** rebuilt. The audit verdict is unambiguous: determinism discipline is exemplary (no floats, no wall-clock, no unseeded RNG, no unordered iteration in any checksum path), seams are clean, all milestone tests pass, and performance headroom is ~250× real-time. Rebuilding it would burn the project's most valuable, hardest-won asset for nothing. It gets **targeted hardening** (§9.6, §9.9) instead.

What *is* rebuilt is the entire presentation layer and its plumbing:

| Layer | Verdict | Action |
|---|---|---|
| Sim + net core (Rust) | Excellent; 2 minor nits | **Keep**; harden + extend the view seam |
| Sim→render seam | Insufficient (diff-inference, no events, no projectile ids, no interpolation data, 56 redundant snapshot rebuilds/frame) | **Extend** (event stream — the keystone item) |
| Frontend structure (GDScript) | 949-line god-file, zero Control nodes, zero signals, raw keycodes, index-poking into packed arrays | **Restructure** before any art lands |
| Art | 94 static SVGs, two themes (one style-drifted off its own locked bible), zero animation, dead assets, no marketing art | **Rebuild** against a single identity + new asset contract |
| VFX / game feel | Good prototype bones (trauma shake, HDR bloom, vignette), but alloc-heavy fake pooling, no interpolation, inert hit-stop, whole mechanics invisible (statuses, hazards, fire explosions) | **Rebuild** on GPUParticles2D + MultiMesh |
| Audio | 15 procedural placeholder WAVs, one 8-second drone as all music, silent menus/net-view, no settings UI | **Rebuild** (systems first, then CC0 assets + music) |
| UX completeness | No settings, no pause, Esc quits app from Match, Clear cooldown invisible, no boss HP bar, no net-view results | **Complete** |
| Docs | Five docs contradict the shipped game (15-min/Samwise vs 30-min/Hippocrate); milestone status under-reported | **Truth pass** |

Locked decisions (`CLAUDE.md`) — Godot 4 + Rust GDExtension, sharded-sim netcode, SDR deployment, netcode-first ordering — are all respected. Two locked items have **drifted and are flagged, not silently revised**, in §9.2.

---

## 9.1 Confirmed defects (fix regardless of any rebuild)

Found and verified by the audits; all are small.

| # | Defect | Evidence | Size |
|---|---|---|---|
| B1 | `Profile._save()` writes a fresh ConfigFile with only `[profile]`, destroying the `[audio]` section `audio.gd` persists to the same `user://profile.cfg`. Any achievement/skin/locale change after muting wipes audio prefs. | `profile.gd:179-184` vs `audio.gd:217-222` | S |
| B2 | Four `tr()` keys never match the CSV: `"REROLL  free x%d"` / `"REROLL  %dg"` (CSV has `[R] `-prefixed keys) and `"[N] sound: on/off"` (absent). Reroll button and mute hint never translate. | `main.gd:868`, `main.gd:797` vs `locale/game_translations.csv:192-193` | S |
| B3 | Esc in the net view quits the entire app instead of navigating back. | `match.gd:145` | S |
| B4 | Checksum blind spot: `Projectile::last_target_pos` is behavior-relevant and snapshotted but not checksummed. | `state.rs:280`, `sim/src/lib.rs:274-297` | S |
| B5 | `Client::pending` grows unboundedly on lost `InputAck`s; `Schedule::discard_before` never called after corrections. | `client.rs:44,141-171` | S |
| B6 | Director acks `Input` from peers not in the match, and answers every mid-match `Join` with an unthrottled full-snapshot serialize (amplification vector). | `director.rs:180-211` | S |
| B7 | CJK-broken right-alignment via `you.length() * 7` px estimate. | `match.gd:194` | S |
| B8 | Dead assets/code: `projectiles/{arrow,axe,boulder}.svg` never loaded (every weapon fires the magic orb), orphaned `palette.svg.import` + `ui_theme.tres`/`_ui_theme`, stale `fx.gd` doc header, stale `main.gd` header comment. | `main.gd:185`, `main.gd:33,168`, `fx.gd:11-13` | S |
| B9 | `cargo fmt --check` fails (325 diffs); CI gates neither fmt nor clippy, and never `cargo check`s `godot/rust` or `adapters/steam-transport` on push — the GDExtension can silently break until a release tag. | `.github/workflows/ci.yml` | S |
| B10 | Two clicks/keys within one 30 Hz tick silently overwrite `pending_code`/`pending_slot` (earlier intent dropped). | `main.gd:253-259,293-326` | S |

---

## 9.2 Flagged drift on locked decisions (decide, don't silently patch)

1. **"Content is data, validated against [05] schemas and hashed into `content_hash`."** Reality: content is compiled Rust (`sim/src/content.rs`, 86 weapons / 91 modifiers) and the only content hash in use is the placeholder `DEMO_CONTENT_HASH = 0xC0DE_C0DE` (`godot/rust/src/lib.rs:251`), which defeats the version-drift gate the join handshake exists for. **Recommendation:** keep compiled-Rust content (it has served the balance/test workflow well) but amend docs/05 §5.7 to say so, and compute a real `content_hash` over the compiled catalog (a deterministic serialization of the tables). Externalized JSON/TOML remains a future option, not a prerequisite.
2. **Gaslamp Bulwark's "LOCKED" STYLE.md** no longer matches its own sprites: commit `a19199b` rewrote the 15 core sprites in a rubber-hose cartoon style (different ink color, palette, and 320px canvases vs the bible's 96/192 contract), clashing with the 12 chibi-era skins. §9.5 resolves this by electing grimdark as the identity; the gaslamp bible must be re-authored or the theme retired to legacy — either way the "LOCKED" label as written is void and the docs must say which.
3. **README/CLAUDE.md framing** ("currently a specification package") mis-states the repo — there is a playable Windows build in `demos/`. Fix in the truth pass (§9.4).

---

## 9.3 The keystone interface: sim→render event stream (define first, fan out after)

Every presentation workstream (VFX, audio, art rotation/animation, damage numbers) is currently starved by the same gap: GDScript infers events by diffing snapshot arrays, which is lossy (spawn+kill in one tick invisible, multi-hits collapsed, muzzle flashes mis-fire) and expensive (every accessor rebuilds a full `RenderView`; ~56 rebuilds/frame in the 8-arena view). Both the VFX and Rust audits independently converged on the same design. **This contract is defined centrally before any presentation work is delegated.**

**Design (one-way, render-only, deterministic by construction):**

- `ArenaState` gains a transient `events: Vec<SimEvent>` buffer, **cleared at the top of each `step()`**, appended during step phases, **excluded from `checksum()` and `snapshot()`** (same class as the existing `tank_hit_this_tick` intra-tick state). Events are a pure function of deterministic state; emission cannot perturb the sim. The netcode never ships them.
- GDExtension exposes `take_events() -> PackedInt64Array` of flat integer records `[kind, a, b, c, d, e]`, read-and-clear, mirroring the existing `Packed*Array` marshaling style.
- **Canonical event kinds (v1):** `EnemyKilled{x, y, kind, boss_flag, bounty, fire_explosion_radius}` · `EnemyDespawned{id}` (disambiguates kill vs off-round expiry) · `Impact{x, y, damage, damage_type, splash_radius}` · `ProjectileSpawned{weapon_kind, x, y, target_x, target_y}` · `TankHit{damage}` · `RoundStart{round}` · `BossSpawned{id}` · `HazardPlaced{x, y, radius, ticks, damage_type}` / `HazardExpired{id}` · `FreezeProc{id}` · `ShieldBroke` · `GoldBounty{amount}`. All payloads integer (positions are already `floor_to_int`).
- **View extensions:** `enemies_status()` (packed status-flag bytes: frost/poison/fire/vuln/stun/freeze — all exist in sim state but never reach the view), `projectiles_id()`/`projectiles_kind()` (requires deterministic id assignment on `Projectile`; if the id joins authoritative state it must enter `checksum()` and bump the snapshot version — golden re-baseline per the documented procedure §9.9-D4), `hazards()`, `minions_id()`.
- **Perf fix bundled in:** cache one `RenderView` per tick inside the GDExtension (rebuilt in `step()`, served from cache by all accessors). This alone removes the dominant per-frame cost in both views.
- **Consumption rule for GDScript:** events drive FX/audio; snapshots drive state display. The fragile `_prev` diff dictionaries in `main.gd:395-441` are deleted, not maintained in parallel.

**Done-condition:** golden determinism harness still passes with events enabled and drained (checksum unchanged unless projectile ids are added to authoritative state, in which case one documented re-baseline); a harness test asserts `checksum(state)` is invariant whether or not events are drained.

---

## 9.4 Phase plan

Phases are dependency-ordered. Within a phase, tasks marked ∥ are independent and dispatched to parallel agents; tasks touching shared files run in worktree isolation and are integrated centrally.

### P0 — Truth & stabilization (1 batch; all S/M, no dependencies)

1. Fix confirmed defects B1–B3, B5–B8, B10 (§9.1). ∥
2. B4 checksum fix + a mechanical snapshot↔checksum field-parity test. ∥
3. B9 CI: `cargo fmt` (one-time 325-diff fix) + `clippy -D warnings` + `cargo check` of `godot/rust` and `adapters/steam-transport` on every push. ∥
4. Docs truth pass ∥ :
   - Reconcile the 30-min arc / The Hippocrate across README, docs/02/04/05/06, sim-core/README (docs/06 currently contradicts itself in one file).
   - docs/06 milestone truth: M0–M4 shipped (with the Steam-meta bullet of M4 and the Steam half of M2/M3 explicitly open), M5 partial with evidence.
   - TUTORIAL.md / godot/README controls refresh ([C]/[L]/[G]/[N], Lobby section); fix stale `locale/README` "not yet applied" note.
   - README + CLAUDE.md framing ("specification package" → playable build) and doc index (add TUTORIAL, CREDITS, demos/, adapters/, 07a, this doc).
   - Extend mechanical doc-sync tests beyond `wire_doc_sync.rs`: assert `BOSS_SPAWN_TICK`, catalog counts, boss name quoted in docs. The one doc that stayed true (docs/04) is the one with a test.

**Gate:** suite green, fmt/clippy green, docs no longer contradict the binary.

### P1 — Engineering foundations (blocks all presentation work)

1. **Event stream + view extensions + cached RenderView** per §9.3 (Rust; M). Sequenced first — it is the interface everything else consumes.
2. **Frontend restructure** (GDScript; L): split `main.gd` (949 lines) into ArenaRenderer / HUD / Shop / Results modules; typed `SimView` wrapper naming every packed-array field (kills the `tank()[2]`/`arena[7]` index-poking repeated ~20×); hoist the 4× duplicated theme/skin-resolution helpers and the duplicated `ENEMY_REL` tables + size constants into `ArtTheme` behind a **single sprite manifest** (path, frames, fps, draw_size per entity kind — the art rebuild's target surface).
3. **Input layer** (M): InputMap actions replacing raw `keycode` matching everywhere (fixes non-QWERTY; enables controller support); keyboard-accessible card selection.
4. **Display foundation** (S): `canvas_items` stretch mode + font-size audit (UI is currently unreadable at 4K, marginal at 720p); Camera2D introduced (unifies shake, enables zoom punch, removes the manual `+ shake` on every blit).
5. **Render interpolation** (M): prev/curr tick position lerp for enemies/projectiles/tank/minions using stable ids — the single biggest perceived-smoothness win (30 Hz snapping today).
6. **Entity rendering via MultiMeshInstance2D** (M): per-kind atlas, per-instance color (hit-flash/status tints), one draw call per texture; replaces per-entity `draw_texture_rect` and the 6-draws-per-projectile ghost trails. `fx.gd` keeps `_draw` only for text and rings, with real ring-buffer pooling instead of Dictionary-alloc + `Array.filter()` per frame.

**Gate:** game plays identically (sim untouched, checksum green), smooth at 60+ Hz, and a stress scene with 500 enemies + 200 projectiles holds frame rate.

### P2 — Art rebuild (parallel with P3/P4 once P1 lands)

**Identity decision:** **grimdark ("Ashen Vigil") is the visual identity** — it is the one theme whose sprites actually obey their style bible. Gaslamp is retired to legacy or re-bibled later; no new money/time spent on two styles.

**Asset contract (new, replaces the drifted per-theme contracts):** author at 2× max draw size — enemies 160², large 256², boss 512², tank+skins 256², minions 128², projectiles 64², FX flipbooks 128²/frame, ground 2048² tileable, UI icons 64², 9-patch panels. Mipmaps ON for everything drawn below native size; atlas-packed; sprites authored facing up with **rotation actually implemented** via `draw_set_transform`/MultiMesh instance rotation (the current contract promises it; no draw call rotates anything).

Production list, priority order:

| Batch | Contents | ~Frames/files |
|---|---|---|
| A (P0-art) | 12 enemies × (4-frame walk + 3-frame death), boss × (walk/attack/death + 1 damage state), 13 tanks × (idle bob frames + 2-frame fire recoil) | ~170 frames |
| B (P0-art) | 4+ weapon-class projectiles **wired up** (arrow/axe/boulder exist and are dead today), 4–6 FX flipbooks (muzzle, impact, death, clear, summon) replacing primitive circles | ~40 |
| C (P1-art) | Arena: tileable ground w/ center focal wear, spawn-ring redesign, 8–12 scatter props, tank damage states (smoke/scorch at HP thresholds — currently 5% HP looks identical to full) | ~20 |
| D (P1-art) | UI kit: 9-patch panels/buttons, 8 weapon icons + 6 modifier icons (today one generic icon serves all ~8 shop offers), rarity frames refresh, lobby + skin-gallery card art, results header | ~30 |
| E (P2-art) | Identity & platform: logo/wordmark, title screen, app icon (export preset has none), Steam capsule set, 25 achievement icons, skin portraits | ~35 |

**Gate per batch:** assets land through the sprite manifest only (no new hardcoded tables); a review pass against the style bible before merge (worktree isolation makes this gate clean).

### P3 — VFX & game feel (consumes §9.3 events; parallel with P2/P4)

Priority order from the VFX audit, sized:

1. Tank-hit feedback: red edge flash + directional vignette pulse + shake + HP-bar damage chunk (currently **audio-only**). S
2. Enemy death v2: kind-colored GPUParticles2D one-shot bursts (persistent pre-warmed emitters, repositioned per event — no node churn); fire-stack deaths get radius-sized orange explosion (currently invisible mechanic); boss death gets multi-stage flash/ring/slow-mo/gold fountain instead of the same 14 sparks as a rat. M
3. Real damage numbers + gold popups from `Impact`/`GoldBounty` events (current numbers are an HP-permille proxy — wrong on bosses). S
4. Muzzle flash v2: per-weapon-kind color/shape, oriented at the target from `ProjectileSpawned` (today: one generic flash at a hardcoded offset, aimed at nothing). S
5. Status-effect overlays: frost tint + crystals, poison drips, fire ember glow, freeze flash/shatter via `enemies_status()` + MultiMesh instance color. M
6. **Hazard-zone decals** (pulsing damage-typed ground circles) — a gameplay-*legibility* fix, not just juice: hazards are completely invisible today. S
7. Projectile trails v2: GPUParticles2D trails / Line2D ribbons keyed by projectile id. M
8. Round banner + spawn-ring pulse; boss entrance (banner, shake ramp, zoom via Camera2D). S+M
9. Tank death sequence: staged explosions, white flash, render-side slow-mo, wreck smoke, delayed results. M
10. Working hit-stop: consume the existing-but-inert `hitstop_active()` to dip render interpolation alpha + particle `speed_scale` for 2–4 frames on big kills — sim cadence untouched (safe in netplay). S
11. Buy/upgrade flourish: card fly-out, arsenal pop, coin tick-down. S
12. **match.gd juice pass:** the 8-arena net view has essentially zero FX today — per-cell kill sparks (budget-capped), own-cell shake/flash, elimination stamp animation, round pulse. All per-player getters already exist. M
13. Low-HP heartbeat vignette (the existing shader's `danger` tracks round, not HP) + spawn telegraphs. S

**Determinism guardrails carried through every task:** events are drained, never fed back; all FX state lives render-side; hit-stop/slow-mo affect interpolation only; nothing writes to the sim. The existing code already models this discipline — keep it.

### P4 — Audio rebuild (parallel with P2/P3)

Systems first (assets drop into them):

1. Settings UI wiring: master/SFX/music sliders (the API exists, nothing calls it) + per-bus setters; persists via the single-owner config fixed in B1. S
2. Pitch/volume randomization per play (±5–10% pitch, ±2 dB) — highest-value fix for sample repetition. S
3. Master limiter + music duck (Godot bus effects) in `default_bus_layout.tres`. S
4. Mass-event handling: replace the drop-all MIN_GAP with per-tick aggregation from `EnemyKilled` events — one voice scaled by kill count (a 50-kill wipe currently sounds identical to one kill). M
5. Music manager: crossfading `set_music`, 2–3 vertical intensity stems mixed by sim-read danger (enemy count / boss flag / round). M
6. Wire the silent surfaces: menu click/back/hover (lobby/skin/challenge scenes have zero Audio calls), elimination stinger + placement sound in the net view, achievement-toast pop. S
7. Optional: X-position stereo panning in the arena. S

Assets — target ~45–55 SFX + 4–6 music pieces per the audit's category table (weapon-family fire ×12–15, impacts ×7, deaths ×5, economy/UI ×9, round-flow ×7, stingers ×6, ambient ×2; music = menu theme, match base+combat+boss stems, victory/defeat outros). **Sourcing: CC0-first** (Kenney, Sonniss, freesound CC0) to keep CREDITS.md bookkeeping nil for a free game; commission only music if budget appears; keep `gen_sfx.py` for dev reproducibility and replace outputs asset-by-asset.

### P5 — UX completion (after P1; interleaves with P2–P4)

1. **Clear cooldown indicator + disabled state** — a promised core mechanic ("the only thing that hurts the boss") is currently invisible. S
2. Round countdown timer replacing the meaningless `tick %d`, shop-refresh timer, **boss HP bar**. M
3. Pause/confirm layer: pause menu in the arena; confirmation before Esc/M abandons a live run; Match Esc navigates back (B3). S
4. Settings screen: volume, language, mute, **reduce-motion/shake toggle**, fullscreen/UI scale. M
5. Net-view end-of-match results panel (placements, stats, rematch/back-to-lobby — today: a toast, and the lobby is one-way). M
6. Challenge-spectate explainer banner ("your bot is playing this rule") — first-time users currently think the game is playing itself. S
7. Colorblind-safe rarity marks (text label alongside color strip); reroll-escalation display; keyboard tooltips. S
8. Weapon detail in Arsenal (type/range/DPS) + range rings — needs a small view extension. M
9. Guarded `ArtTheme.tex()` + a real "GDExtension missing" boot error screen; navigation router replacing scattered `change_scene_to_file` literals. S

### P6 — Hardening & ship track (independent of presentation; can run in parallel throughout)

Not "rebuild" work, but the audits say it plainly: **the headline promise — multiplayer over Steam — has never run between two machines.** Everything network-shaped is in-process over the test Hub with a fabricated lobby.

1. **Steam live bring-up (L, the critical path):** wire auth tickets, connection-status callbacks, lobby-gated accept (the three `TODO(steam-runtime)` stubs); compose `SteamBootstrap` + `SteamTransport` + `Director`/`Client` into a real match driver behind the existing `Transport` trait; real `ISteamMatchmaking` lobby replacing the simulated peers; 2-machine smoke test on App 480, then live host-migration test.
2. Director hardening: correction-rate counter → suspicion → kick threshold (docs/06 §6.3 exit criteria); B6 fixes. M
3. Real `content_hash` per §9.2-1. M
4. Long soak: multi-hour nightly with RSS ceiling assertion; explicit entity cap (docs/06 R3 mitigation, currently implicit only). M
5. Release pack: registered free App ID (swap both `steam_appid.txt`), `ISteamUserStats` mapping of the 25 local achievements, replay submission + verifier host decision, store assets (from P2 batch E), iconed/signed export presets, release runbook + versioned demo zips. L
6. Opportunistic: split `combat.rs` (1358 non-test lines) into fire/projectile/enemy-attack submodules while in there for the event stream. S–M

### New-contributor docs (folded into phases, listed once)

- **Binding API doc** for `StSim`/`StMatch`/`StLobby` — the entire sim↔Godot contract is currently discoverable only by reading `godot/rust/src/lib.rs`. Write it when §9.3 lands (the contract changes then anyway).
- **Event catalog** (§9.3 kinds) + art contract/manifest guide + "how to add an enemy/skin/theme" — with P1.
- Content-authoring guide (`content.rs` conventions, `descriptions.rs` voice, locale CSV keying) + the **golden-checksum re-baseline procedure** (performed six times in history, never written down) — with P0 docs pass.
- Balance methodology (`preview`/`sweep`/`balance_guards` workflow) — extracted from docs/06 prose. S

---

## 9.5 Dependency graph & delegation map

```
P0 truth/stabilization ──────────────┐            (all ∥, 1 batch)
                                     ▼
P1.1 event stream (Rust) ──► P1.2 frontend restructure ──► P2 art ∥ P3 vfx ∥ P4 audio ∥ P5 ux
        │                    (P1.3–P1.6 ∥ with P1.2)              (interleave; shared-file work
        └──► binding API doc                                       in worktrees, central merge)
P6 ship track ──────────────── runs in parallel with everything after P0
```

Orchestration rules for execution (per CLAUDE.md):
- §9.3's event contract and the P1.2 sprite manifest are **defined centrally before fan-out**; agents build against them, never redefine them.
- P2 art batches, P3 effects, P4 audio items, and P5 UX items are small, independently verifiable tasks — one agent each, parallel, worktree-isolated where they touch `main.gd`-descendant modules or `content.rs`.
- Every phase gate re-runs: `cargo test --workspace`, the M0 harness gate, fmt/clippy, and (post-P1) the 500-entity stress scene.
- Balance-affecting changes: none are planned in this program; any agent proposing one is escalated, not merged.

## 9.6 Rough effort totals

| Phase | Size |
|---|---|
| P0 | ~12 S + 1 M (one dispatch wave) |
| P1 | 1 L + 3 M + 2 S (critical path: P1.1 → P1.2) |
| P2 | ~295 asset frames/files in 5 batches + S/M wiring each |
| P3 | 13 effects: 7 S + 6 M |
| P4 | 5 S + 2 M systems + ~50 assets + 4–6 music pieces |
| P5 | 5 S + 4 M |
| P6 | 2 L + 3 M + 1 S (Steam bring-up is the schedule risk) |

## 9.7 Risks

| Risk | Mitigation |
|---|---|
| Event stream accidentally perturbs determinism | Events cleared per-step, excluded from checksum/snapshot; harness test asserts checksum invariance drained-vs-undrained; golden gate on every push |
| Projectile-id addition changes the golden checksum | Expected, one-time, documented re-baseline; the procedure gets written down first (it has been done six times undocumented) |
| Art/VFX agents colliding in the frontend | P1.2 restructure lands **first**; worktree isolation + central merge for anything touching shared modules |
| Two-theme cost creep | Identity decision (§9.4-P2) is explicit: grimdark only; gaslamp frozen |
| Steam bring-up surprises (adapter never ran live) | Start P6.1 immediately in parallel — it needs no presentation work; App 480 smoke test de-risks before the release pack |
| Doc drift recurring after the truth pass | Mechanical doc-sync tests extended (the only doc that stayed correct is the one with a test) |

## 9.8 Acceptance criteria (program-level)

1. All 254+ tests green; M0 gate green; fmt/clippy green in CI on every push, including `godot/rust` + adapter checks.
2. Stress scene (500 enemies, 200 projectiles, FX active) ≥ 60 fps; net view (8 arenas) ≥ 60 fps with the P3.12 juice pass.
3. Every sim mechanic visible: statuses, hazards, fire explosions, boss progress (HP bar), Clear cooldown.
4. Zero raw-keycode input; playable on controller; UI legible 720p→4K.
5. One coherent art identity, all entities animated (walk/death minimum), rotation implemented, no dead assets, manifest-driven.
6. Audio: settings UI, no sample machine-gunning, music with intensity layers, no silent screens.
7. Docs match the binary; new-contributor docs for the three seams exist.
8. (Ship track) Two real machines complete a match over Steam, host migration included.
