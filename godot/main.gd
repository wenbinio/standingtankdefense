# Standing Tank Defense — single-arena ROOT COORDINATOR. Drives the Rust
# `StSim` (deterministic 30 Hz ticks; the GAME SPEED pref only changes how many
# ticks run per wall-clock second — see the accumulator in _physics_process),
# owns the input-intent FIFO, routes InputMap actions + clicks, drains the sim
# event stream once per tick (fanning it to juice + audio), and wires the
# render modules (children of Main.tscn):
#   Camera              — screen shake via offset (world canvas only)
#   ArenaRenderer       — world drawing + lighting rig + event-driven juice
#   UiLayer/FxOverlay   — screen-space FX bus drawing + full-screen flash
#   UiLayer/Hud         — top bar + arsenal panel
#   UiLayer/Shop        — shop cards / reroll / clear (alive)
#   UiLayer/Results     — death panel + redeploy (dead)
#   UiLayer/BlackMarket — Black Market picker overlay + pending badge (alive)
#   UiLayer/PauseMenu   — pause/settings overlay (alive; owns input while open)
# All sim reads go through the SimView wrapper; only THIS file calls sim.step.
# Controls: click a shop card or press 1-8 to buy · R reroll · Space clear ·
# B reopen a dismissed Black Market picker · Esc pause menu (resume/settings/
# quit — quit is confirm-gated so a live run can't be abandoned by one
# keypress) · M multi-arena net demo (see project.godot [input]).
#
# PAUSE (single-player only): while the PauseMenu overlay is open this file
# stops calling sim.step() and stops draining the intent FIFO — the LOCAL,
# offline sim simply holds its tick counter (the classic freeze; safe because
# nobody else consumes this sim). The FX bus freezes too (fx.update skipped),
# so the pause is a true still frame behind the dimmed menu. NETPLAY MUST
# NEVER DO THIS: match.gd / the director model keeps every sim ticking —
# there is no global pause in docs/03's architecture.
extends Node2D

const BUY_ACTIONS: Array[StringName] = [
	&"ui_buy_1", &"ui_buy_2", &"ui_buy_3", &"ui_buy_4",
	&"ui_buy_5", &"ui_buy_6", &"ui_buy_7", &"ui_buy_8",
]

# Sim tick rate (mirrors sim::TICK_HZ) — the tick-accumulator's denominator.
const TICK_HZ := 30
# Black-Market pick intent codes (mirror StSim.step's input table; the overlay
# submits these through on_pick with slot = the chosen CATALOG index).
const BM_CODE_WEAPON := 4
const BM_CODE_UPGRADE := 5

var sim                        # StSim — the ONLY handle that ever steps
var view: SimView              # typed read-only wrapper every module consumes
# Input-intent FIFO: [code, slot] pairs (1 buy · 2 reroll · 3 clear ·
# 4/5 Black-Market pick) queued by the input handlers and drained ONE per SIM
# TICK in arrival order (NOT per frame — a multi-step game-speed frame drains
# one intent per step), so two inputs landing within the same tick no longer
# overwrite each other. Capped small so stale input can't buffer up.
const MAX_QUEUED_INTENTS := 4
var _intents: Array = []
# GAME SPEED (single-player, persisted pref): integer tick accumulator. Each
# 30 Hz physics frame banks Profile.ticks_per_second() (30/45/60/90) and the
# sim steps while a whole tick's worth (TICK_HZ) is banked. Exact integer
# math: Normal = 1 step/frame · Fast alternates 1/2 · Faster = 2 · Hyper = 3.
# Cadence only — every tick stays bit-identical to Normal speed.
var _tick_accum := 0
# The intent consumed THIS tick (exactly what sim.step() received); kept for the
# shop's pressed-state draw feedback. 0 = none.
var pending_code := 0
var pending_slot := 0

# --- audio (render-only; reads the drained event stream, never the sim) ------
# Gameplay SFX are driven by the SAME event Array the juice fan-out consumes
# (drained once per tick in _physics_process). The only remaining edge tracker
# is the death edge — your own death has no sim event, so is_dead() is diffed.
var _au_was_dead := false      # death edge -> tank_destroyed + defeat

# --- juice / modules (render-only) -----------------------------------------
var fx: Fx                    # reusable pooled FX + screen-shake/hitstop bus
var _recorded := false        # match-end achievements credited once
# SP VICTORY LATCH: killing the boss (boss-flagged EV_ENEMY_KILLED) marks the
# run won — permanently. The run CONTINUES (swift-end waves keep coming) and
# the eventual death still shows the results panel, in its gold victory
# variant, with "won" recorded in the profile. Render-side only: the latch is
# derived from the drained event stream and never feeds back into the sim.
var _won := false
var _won_tick := -1           # sim tick of the boss kill (-1 = not yet)
# A11 death forensics (render-side only): ring of tank damage taken per tick
# over the trailing 10 s (300 ticks @ 30 Hz). Slot tick % window is overwritten
# every tick, so the ring always holds exactly the last window — no allocs.
const HIT_WINDOW_TICKS := 300
var _hit_ring := PackedInt64Array()
# P3.9: the results panel holds off ~0.8 s so the death FX sequence can play.
const RESULTS_DELAY_MS := 800
var _dead_since_ms := -1      # wall-clock ms of the is_dead() edge (-1 = alive)

# Black Market pending-edge tracker: the overlay arms (choice lists built
# ONCE) on the false->true edge and disarms on the redeem edge.
var _bm_was_pending := false

@onready var _camera: Camera2D = $Camera
@onready var _arena: Node2D = $ArenaRenderer
@onready var _fx_overlay: Node2D = $UiLayer/FxOverlay
@onready var _hud: Node2D = $UiLayer/Hud
@onready var _shop: Node2D = $UiLayer/Shop
@onready var _results: Node2D = $UiLayer/Results
@onready var _bm: Node2D = $UiLayer/BlackMarket
@onready var _pause_menu: Node2D = $UiLayer/PauseMenu

func _ready() -> void:
	randomize()
	# SP deploy honors the profile's difficulty preset (Easy/Normal/Hard over
	# the sim's ramp dial — authoritative sim state, SP-only; netplay's
	# StMatch/director path has no difficulty parameter and stays Normal).
	sim = StSim.new_match_with_difficulty(randi(), Profile.difficulty())
	view = SimView.of_sim(sim)
	fx = Fx.new()
	# The world canvas is dimmed by ArenaRenderer's CanvasModulate; the UI layer
	# lives outside that canvas, so give its nodes the same ambient modulate to
	# keep FX/HUD/shop colors identical to the pre-split rendering.
	for ui_node in [_fx_overlay, _hud, _shop, _results, _bm, _pause_menu]:
		ui_node.modulate = _arena.AMBIENT_DIM
	# Black Market picks route back through the SAME intent FIFO as buy/reroll.
	_bm.on_pick = _on_bm_pick
	# A12: the pause menu's confirm-gated "Restart Run" reuses the exact
	# _redeploy() path (the menu closes itself before calling).
	_pause_menu.on_restart = _redeploy
	_hit_ring.resize(HIT_WINDOW_TICKS)   # zero-filled by resize
	_wire_modules()
	# AUDIO (render-only): start the looping ambient bed.
	Audio.set_music_layers("match_base.wav", "match_combat.wav")
	_au_was_dead = false

# Hand every module its read-only view (+ the shared FX bus where needed) and
# sync visibility with the alive/dead state.
func _wire_modules() -> void:
	_arena.reset(view, fx)
	_fx_overlay.fx = fx
	_hud.view = view
	_shop.view = view
	_results.view = view
	_sync_dead_panels()

# Shop while alive, results panel while dead — the visibility switch that used
# to be _draw_hud's if/else. On the death EDGE this also arms the arena's
# staged death FX and starts the results-panel delay (~0.8 s) so the sequence
# reads before the panel covers it. Render-only wall-clock gating.
func _sync_dead_panels() -> void:
	var dead: bool = view.is_dead()
	if dead:
		if _dead_since_ms < 0:
			_dead_since_ms = Time.get_ticks_msec()
			_arena.trigger_tank_death()
	else:
		_dead_since_ms = -1
	_shop.visible = not dead
	_results.visible = dead and Time.get_ticks_msec() - _dead_since_ms >= RESULTS_DELAY_MS

# --- input (InputMap actions; physical-key bindings live in project.godot) ---
func _unhandled_input(e: InputEvent) -> void:
	# [N] mute is a global UX toggle (render-only) — handled first so it works
	# alive, on the results panel AND inside the pause menu (whose settings pane
	# mirrors the state live). Plays a confirm blip when unmuting.
	if (e is InputEventKey or e is InputEventJoypadButton) \
			and e.is_action_pressed(&"ui_mute"):
		var muted := Audio.toggle_mute()
		if not muted:
			Audio.play(&"ui_move")
		return
	# While the pause/settings overlay is open it owns EVERY remaining event —
	# keys, clicks, and mouse motion (slider drags). Nothing below (shop input,
	# scene changes) can fire. The overlay is a plain child node of THIS scene,
	# not a global: the net view (match.gd) has no PauseMenu and can never pause.
	if _pause_menu.is_open():
		_pause_menu.handle_input(e)
		return
	# Black Market picker: while OPEN it owns the remaining events (modal for
	# INPUT only — the sim keeps stepping behind it). It never blocks the pause
	# path: Esc dismisses it to the badge, so the next Esc lands below and
	# opens the pause menu as usual.
	if _bm.is_open():
		_bm.handle_input(e)
		return
	if e is InputEventMouseButton:
		if e.pressed and e.button_index == MOUSE_BUTTON_LEFT:
			_handle_click(e.position)
		return
	if not (e is InputEventKey or e is InputEventJoypadButton):
		return
	# While dead, the only live controls are Redeploy (confirm) and Menu (back);
	# swallow the shop/number/reroll actions so a fresh run isn't dirtied.
	if sim != null and view.is_dead():
		if e.is_action_pressed(&"ui_confirm"):
			_redeploy()
		elif e.is_action_pressed(&"ui_back"):
			get_tree().change_scene_to_file("res://SkinSelect.tscn")
		return
	# Number row 1..8 buys the matching shop slot.
	for i in BUY_ACTIONS.size():
		if e.is_action_pressed(BUY_ACTIONS[i]):
			_queue_intent(1, i)
			return
	if e.is_action_pressed(&"ui_reroll"):
		_queue_intent(2)
	elif e.is_action_pressed(&"ui_clear"):
		_queue_intent(3)
	elif e.is_action_pressed(&"ui_black_market"):
		# [B] reopens a dismissed-but-held Black Market picker (while the
		# picker is open, [B]/Esc dismiss it inside _bm.handle_input above).
		if _bm.is_held():
			_bm.reopen()
	elif e.is_action_pressed(&"ui_theme_cycle"):
		_cycle_theme()
	elif e.is_action_pressed(&"ui_net_view"):
		get_tree().change_scene_to_file("res://Match.tscn")   # multi-arena net demo
	elif e.is_action_pressed(&"ui_back"):
		# Esc while alive PAUSES (was: instant scene change — the run-abandon
		# footgun). Quit lives inside the menu behind a confirm step. Dead-state
		# Esc (above) keeps its direct back-to-menu behavior on the results panel.
		_pause_menu.open_pause()

# Route a left-click by the modules' hit-targets. While dead, the only
# clickable target is the Redeploy button on the results panel; shop rects are
# stale and must not fire.
func _handle_click(pos: Vector2) -> void:
	if sim != null and view.is_dead():
		if _results.redeploy_hit(pos):
			_redeploy()
		return
	# Held Black Market pick: the shop-area badge reopens the picker.
	if _bm.badge_hit(pos):
		_bm.reopen()
		return
	var card: int = _shop.card_at(pos)
	if card >= 0:
		_queue_intent(1, card)
		return
	if _shop.reroll_hit(pos):
		_queue_intent(2)
		return
	if _shop.clear_hit(pos):
		_queue_intent(3)

# Cycle the active ArtTheme and have every module re-resolve its textures.
func _cycle_theme() -> void:
	ArtTheme.cycle()
	_arena.reload_theme()
	_hud.reload_theme()
	_shop.reload_theme()

# Enqueue an input intent for the sim, preserving arrival order. Intents beyond
# the small cap are dropped (better than buffering seconds of stale clicks).
# Returns whether the intent was actually queued (the Black Market overlay
# stays open on a full-FIFO false so the pick is never silently lost).
func _queue_intent(code: int, slot: int = 0) -> bool:
	if _intents.size() >= MAX_QUEUED_INTENTS:
		return false
	_intents.append([code, slot])
	return true

# Black Market overlay pick callback: queue (code 4/5, slot = catalog index)
# through the SAME FIFO as buy/reroll/clear — the sim still receives exactly
# one (code, slot) per tick.
func _on_bm_pick(code: int, slot: int) -> bool:
	return _queue_intent(code, slot)

# Start a fresh single-arena run in-place. Rebuilds the sim exactly as _ready()
# does — new_match_with_difficulty(randi(), profile preset), no challenge
# (single-arena never applies Profile.active_challenge_code) — then resets
# every render/juice/FX tracker so nothing leaks across runs. Clearing
# _recorded re-arms the once-per-run achievement latch so the new run credits
# its own play.
func _redeploy() -> void:
	sim = StSim.new_match_with_difficulty(randi(), Profile.difficulty())
	view = SimView.of_sim(sim)
	fx = Fx.new()
	_recorded = false
	# Fresh run: the victory latch re-arms.
	_won = false
	_won_tick = -1
	_intents.clear()
	pending_code = 0
	pending_slot = 0
	_shop.pending_code = 0
	_shop.pending_slot = 0
	_tick_accum = 0
	# Fresh sim holds no Black Market pick: reset the overlay + edge tracker.
	_bm.disarm()
	_bm_was_pending = false
	# Re-arm the death-edge audio tracker for the fresh run.
	_au_was_dead = false
	# Fresh run: empty damage ring + stale death report dropped.
	_hit_ring.fill(0)
	_results.clear_report()
	_wire_modules()

func _physics_process(_delta: float) -> void:
	if sim == null:
		return
	# PAUSE GATE (single-player only): overlay open -> no sim.step, no intent
	# drain, no event fan-out, and the tick accumulator does not grow — a pause
	# freezes stepping AT ANY GAME SPEED. The local sim's tick counter/checksum
	# hold exactly where they were; queued intents (≤4) stay queued and resume
	# in order. Netplay must never gate a shared sim like this (see header note).
	if _pause_menu.is_open():
		return
	if view.is_dead():
		# Frozen run (results panel): no stepping at any speed. Keep the old
		# once-per-frame cadence for the render/audio trackers.
		_tick_accum = 0
		_drain_one_intent()
		_shop.pending_code = pending_code
		_shop.pending_slot = pending_slot
		if not _recorded:
			# Credit your own run's achievements + personal records from how you
			# actually played, and build the results panel's death report ONCE
			# (all result strings composed here at the death edge, never per frame).
			_recorded = true
			var rec: Dictionary = view.stats_record()
			# SP victory = the boss died on this run (latched above). "won"
			# feeds the profile record + the sole_survivor-style framing.
			rec["won"] = _won
			var prev_best_round: int = Profile.best_round   # before this run lands
			# RECORDS GUARD: bests/achievements/history only land on Normal —
			# an Easy run must not set farmable PBs and a Hard run must not
			# demand them (the SkinSelect hint says so). Easy/Hard still get
			# the full results panel, just with no record side-effects.
			var on_normal: bool = Profile.difficulty() == Profile.DIFF_NORMAL
			var unlocks: Array = Profile.record_match(rec) if on_normal else []
			var bests: Dictionary = \
				Profile.record_run(rec, _mmss(view.tick())) if on_normal else {}
			_results.set_report(_build_death_report(rec, unlocks, bests, prev_best_round))
			# Death-moment celebration (A1/A6): gold NEW BEST banner (round beats
			# damage beats gold when several land), then achievement banners —
			# rarer, so they win the single fx banner slot. &"victory" sting for
			# both (the orchestrator re-points sting names after the audio pass).
			if bool(bests.get("round", false)):
				fx.banner(tr("NEW BEST ROUND!"), Color(1.8, 1.5, 0.6), 1.8, 48)
			elif bool(bests.get("damage", false)):
				fx.banner(tr("NEW BEST DAMAGE!"), Color(1.8, 1.5, 0.6), 1.8, 48)
			elif bool(bests.get("gold", false)):
				fx.banner(tr("NEW BEST GOLD!"), Color(1.8, 1.5, 0.6), 1.8, 48)
			if bests.values().has(true):
				Audio.play(&"victory")
			for id in unlocks:
				fx.banner("★ " + tr(String(Profile.ach_def(id).get("name", id))),
					Color(1.9, 1.6, 0.7), 2.2, 44)
				Audio.play(&"victory")
		_arena.tick_juice([])
		_update_audio([], pending_code, view.gold())
		_sync_black_market()
		_sync_dead_panels()
		return
	# GAME SPEED accumulator (exact integers): bank ticks_per_second per 30 Hz
	# physics frame, run one full sim tick per banked TICK_HZ. Each iteration
	# is a complete tick — one drained intent, one step, one event fan-out —
	# so every per-tick invariant survives multi-step frames unchanged.
	_tick_accum += Profile.ticks_per_second()
	while _tick_accum >= TICK_HZ and not view.is_dead():
		_tick_accum -= TICK_HZ
		_step_one_tick()
	_sync_black_market()
	_sync_dead_panels()

# ONE complete sim tick: drain exactly one queued intent (the FIFO invariant —
# one (code, slot) per SIM TICK, not per frame), step, drain the event stream
# EXACTLY ONCE, and fan it out to the arena juice + audio hooks. Nobody
# re-queries the stream (take_events is read-and-clear at the binding).
func _step_one_tick() -> void:
	_drain_one_intent()
	# Mirror the consumed intent to the shop for its pressed-state feedback
	# (last step of a multi-step frame wins — a 1-frame cosmetic).
	_shop.pending_code = pending_code
	_shop.pending_slot = pending_slot
	# Capture pre-step gold so the audio hook can tell a successful buy (gold
	# actually dropped) from a no-op click; and the pre-step Black Market flag
	# so a consumed pick can be classified below. Read-only.
	var au_gold_before: int = view.gold()
	var bm_was_held: bool = view.black_market_pending()
	sim.step(pending_code, pending_slot)
	var events: Array = view.take_events()
	if pending_code == 3:
		# Clear is input-driven (no sim event): arm the shockwave + voice it
		# here, the one place the consumed intent is known.
		_arena.trigger_clear()
		Audio.play(&"clear")
	elif pending_code == BM_CODE_WEAPON or pending_code == BM_CODE_UPGRADE:
		# Black Market pick consumed this tick: redeemed (pending dropped —
		# voice the free "buy") or no-oped by the sim (pending survived; should
		# not happen since the overlay only offers eligible indices — resurface
		# the badge so the pick is never stranded invisible).
		if bm_was_held and not view.black_market_pending():
			Audio.play(&"buy")
		elif bm_was_held:
			_bm.pick_failed()
	# A11 forensics ring: bank this tick's incoming tank damage (0 most ticks —
	# the write itself is what expires the stale slot from one window ago).
	# Same pass: watch for the boss-flagged kill that latches SP victory.
	var tank_dmg := 0
	for ev in events:
		if ev.kind == SimView.EV_TANK_HIT:
			tank_dmg += int(ev.damage)
		elif ev.kind == SimView.EV_ENEMY_KILLED and bool(ev.boss) and not _won:
			# SP VICTORY (docs/02 §2.7's prestige goal, rendered): latch — the
			# run keeps going against the swift-end tide, but it is now a win.
			# COMPOSITION with the existing boss-death juice: arena_renderer
			# already fires the kill's flash/shockwaves/slow-mo (stage 1) and
			# the delayed gold fountain (stage 2) from this same event — those
			# stay untouched; this adds the ONE gold banner + victory sting on
			# top, guarded by the latch so nothing can ever double-fire.
			_won = true
			_won_tick = view.tick()
			fx.banner(tr("BOSS SLAIN — YOU SURVIVED THE ARC"),
				Color(1.9, 1.6, 0.7), 2.6, 46)
			Audio.play(&"victory")
	_hit_ring[view.tick() % HIT_WINDOW_TICKS] = tank_dmg
	_arena.tick_juice(events)
	_update_audio(events, pending_code, au_gold_before)

# Pop exactly ONE queued intent (FIFO — earlier of two same-tick inputs is no
# longer lost; the later one simply runs next tick). What the sim receives per
# tick is unchanged: a single (code, slot) pair.
func _drain_one_intent() -> void:
	if _intents.is_empty():
		pending_code = 0
		pending_slot = 0
	else:
		var intent: Array = _intents.pop_front()
		pending_code = intent[0]
		pending_slot = intent[1]

# Assemble the render-side death report ONCE at the death edge (A1/A2/A7/A11):
# everything the results panel shows beyond the raw stats — near-miss timing,
# the final-10-seconds forensics, next goals, and the personal-best context.
# Reads only the frozen sim view + Profile; results.set_report composes the
# display strings from this exactly once.
func _build_death_report(rec: Dictionary, unlocks: Array, bests: Dictionary,
		prev_best_round: int) -> Dictionary:
	# Census of enemy kinds on screen at the death moment -> dominant kind.
	var counts := {}
	for k in view.enemies_kind():
		counts[int(k)] = int(counts.get(int(k), 0)) + 1
	var top_kind := -1
	var top_n := 0
	for k in counts:
		if int(counts[k]) > top_n:
			top_kind = int(k)
			top_n = int(counts[k])
	var recent := 0
	for d in _hit_ring:
		recent += int(d)
	var boss: Dictionary = view.boss_info()
	return {
		"rec": rec,
		"unlocks": unlocks,
		"bests": bests,
		"won": _won,                        # SP victory latch (boss killed)
		"won_tick": _won_tick,              # boss-kill tick (victory clock base)
		"records_off": Profile.difficulty() != Profile.DIFF_NORMAL,
		"prev_best_round": prev_best_round,
		"tick": view.tick(),
		"boss_spawn_tick": view.boss_spawn_tick(),
		"boss_hp_permille": int(boss.get("hp_permille", -1)),   # -1 = no boss up
		"top_kind": top_kind,
		"top_kind_count": top_n,
		"recent_damage": recent,
		"total_runs": Profile.total_runs,   # post-increment (drives tip rotation)
		"goals": Profile.nearest_goals(rec),
	}

# "mm:ss" for a tick count (floor'd to whole seconds; hud.gd's formatter).
func _mmss(ticks: int) -> String:
	@warning_ignore("integer_division")
	var secs := maxi(ticks, 0) / TICK_HZ
	@warning_ignore("integer_division")
	return "%02d:%02d" % [secs / 60, secs % 60]

# Black Market pending-edge sync (once per physics frame, after all steps):
# on false->true the overlay arms — the eligible choice lists are built HERE,
# exactly once per edge (never per frame); on true->false (pick redeemed, or
# death hiding the run) it disarms, badge included.
func _sync_black_market() -> void:
	var pending: bool = not view.is_dead() and view.black_market_pending()
	if pending and not _bm_was_pending:
		_bm.arm(
			view.black_market_choices(true), view.black_market_choice_names(true),
			view.black_market_choices(false), view.black_market_choice_names(false))
	elif _bm_was_pending and not pending:
		_bm.disarm()
	_bm_was_pending = pending

# Frame-rate cosmetic update: advance the FX bus and feed its shake + zoom
# punch into the camera (the world canvas moves as one; the UI CanvasLayer
# doesn't). The camera anchors top-left, so a center-locked zoom needs the
# offset compensated by vp/2 * (1 - 1/z); at z == 1 that term is zero and only
# the shake remains.
func _process(delta: float) -> void:
	if sim == null or fx == null:
		return
	# Paused: the FX bus freezes too (pools/timers hold in place) so the pause
	# reads as a true still frame behind the dimmed menu — chosen over "FX keep
	# animating" so nothing decays or expires while the player is away.
	if _pause_menu.is_open():
		return
	fx.update(delta)
	if _camera:
		# Screen-shake pref (reduce motion, persisted via Profile): when off, the
		# camera pins to identity — no shake offset AND no zoom punch. This is
		# the single point where main.gd applies fx camera motion, so the toggle
		# covers all of it without touching fx.gd.
		var shake_on: bool = Profile.screen_shake()
		var z: float = fx.zoom_scale() if shake_on else 1.0
		_camera.zoom = Vector2(z, z)
		_camera.offset = (fx.shake_offset() if shake_on else Vector2.ZERO) \
			+ get_viewport_rect().size * 0.5 * (1.0 - 1.0 / z)

# AUDIO (render-only): every gameplay SFX now fires from the drained event
# stream — the SAME Array the juice fan-out consumed, decoded once by SimView.
# STRICTLY ONE-WAY: nothing here calls sim.step() or otherwise writes the sim;
# audio cannot influence determinism.
#   `events`     = this tick's drained sim events (empty while dead)
#   `intent`     = this tick's pending_code (1 buy · 2 reroll · 3 clear ·
#                  4/5 Black-Market pick, voiced in _step_one_tick · else none)
#   `gold_before`= gold sampled BEFORE the step, to confirm a buy actually spent.
func _update_audio(events: Array, intent: int, gold_before: int) -> void:
	# --- death edge: tank_destroyed always; the defeat sting only on a LOST
	# run — a boss-slaying (latched-victory) run already had its victory sting
	# at the kill and ends on the gold results panel, not a defeat note.
	# Your own death has NO sim event, so this edge tracker stays.
	var dead: bool = view.is_dead()
	if dead and not _au_was_dead:
		Audio.play(&"tank_destroyed")
		if not _won:
			Audio.play(&"defeat")
	_au_was_dead = dead
	if dead:
		return   # frozen run: no further gameplay SFX while on the results panel

	# --- event-driven SFX (Audio's per-event throttle absorbs bursts) --------
	# Kills aggregate per tick into ONE count-scaled voice (Audio.play_many):
	# a 50-kill wave wipe sounds bigger than a single kill instead of identical.
	var kills := 0
	for ev in events:
		match ev.kind:
			SimView.EV_ENEMY_KILLED:
				kills += 1
			SimView.EV_ENEMY_DESPAWNED:
				# Deliberately silent: contact self-destructs are not kills
				# (this fixes the old fake death sound from snapshot diffing).
				pass
			SimView.EV_IMPACT:
				Audio.play(&"hit")
			SimView.EV_PROJECTILE_SPAWNED:
				Audio.play(&"fire")
			SimView.EV_TANK_HIT:
				Audio.play(&"tank_hit")
			SimView.EV_ROUND_START:
				Audio.play(&"round_start")
			SimView.EV_BOSS_SPAWNED:
				Audio.play(&"boss_spawn")
			SimView.EV_GOLD_BOUNTY:
				# P3 hook: gold-pickup chime (no asset in Audio.EVENTS yet).
				pass
			_:
				pass   # Hazard*/FreezeProc/ShieldBroke: P3 SFX
	if kills > 0:
		Audio.play_many(&"enemy_death", kills)

	# --- economy actions: input-driven (no sim event for buy/reroll) ---------
	if intent == 1 and view.gold() < gold_before:
		# A buy that spent gold this tick (a click on an unaffordable card spends
		# nothing, so it stays silent).
		Audio.play(&"buy")
	elif intent == 2:
		# Reroll: refreshes the shop whether free or paid, so always voice it.
		Audio.play(&"reroll")
