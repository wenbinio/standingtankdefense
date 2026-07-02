# Standing Tank Defense — single-arena ROOT COORDINATOR. Drives the Rust
# `StSim` (one deterministic tick per 30 Hz physics frame), owns the
# input-intent FIFO, routes InputMap actions + clicks, drains the sim event
# stream once per tick (fanning it to juice + audio), and wires the render
# modules (children of Main.tscn):
#   Camera            — screen shake via offset (world canvas only)
#   ArenaRenderer     — world drawing + lighting rig + event-driven juice
#   UiLayer/FxOverlay — screen-space FX bus drawing + full-screen flash
#   UiLayer/Hud       — top bar + arsenal panel
#   UiLayer/Shop      — shop cards / reroll / clear (alive)
#   UiLayer/Results   — death panel + redeploy (dead)
# All sim reads go through the SimView wrapper; only THIS file calls sim.step.
# Controls: click a shop card or press 1-8 to buy · R reroll · Space clear ·
# Esc back to skin select · M multi-arena net demo (see project.godot [input]).
extends Node2D

const BUY_ACTIONS: Array[StringName] = [
	&"ui_buy_1", &"ui_buy_2", &"ui_buy_3", &"ui_buy_4",
	&"ui_buy_5", &"ui_buy_6", &"ui_buy_7", &"ui_buy_8",
]

var sim                        # StSim — the ONLY handle that ever steps
var view: SimView              # typed read-only wrapper every module consumes
# Input-intent FIFO: [code, slot] pairs (1 buy · 2 reroll · 3 clear) queued by
# the input handlers and drained ONE per 30 Hz physics tick in arrival order,
# so two inputs landing within the same tick no longer overwrite each other.
# Capped small so stale input can't buffer up.
const MAX_QUEUED_INTENTS := 4
var _intents: Array = []
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

@onready var _camera: Camera2D = $Camera
@onready var _arena: Node2D = $ArenaRenderer
@onready var _fx_overlay: Node2D = $UiLayer/FxOverlay
@onready var _hud: Node2D = $UiLayer/Hud
@onready var _shop: Node2D = $UiLayer/Shop
@onready var _results: Node2D = $UiLayer/Results

func _ready() -> void:
	randomize()
	sim = StSim.new_match(randi())
	view = SimView.of_sim(sim)
	fx = Fx.new()
	# The world canvas is dimmed by ArenaRenderer's CanvasModulate; the UI layer
	# lives outside that canvas, so give its nodes the same ambient modulate to
	# keep FX/HUD/shop colors identical to the pre-split rendering.
	for ui_node in [_fx_overlay, _hud, _shop, _results]:
		ui_node.modulate = _arena.AMBIENT_DIM
	_wire_modules()
	# AUDIO (render-only): start the looping ambient bed.
	Audio.set_music("ambient_bed.wav")
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
# to be _draw_hud's if/else.
func _sync_dead_panels() -> void:
	var dead: bool = view.is_dead()
	_shop.visible = not dead
	_results.visible = dead

# --- input (InputMap actions; physical-key bindings live in project.godot) ---
func _unhandled_input(e: InputEvent) -> void:
	if e is InputEventMouseButton:
		if e.pressed and e.button_index == MOUSE_BUTTON_LEFT:
			_handle_click(e.position)
		return
	if not (e is InputEventKey or e is InputEventJoypadButton):
		return
	# [N] mute is a global UX toggle (render-only) — handled before the dead-guard
	# so it works on the results panel too. Plays a confirm blip when unmuting.
	if e.is_action_pressed(&"ui_mute"):
		var muted := Audio.toggle_mute()
		if not muted:
			Audio.play(&"ui_move")
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
	elif e.is_action_pressed(&"ui_theme_cycle"):
		_cycle_theme()
	elif e.is_action_pressed(&"ui_net_view"):
		get_tree().change_scene_to_file("res://Match.tscn")   # multi-arena net demo
	elif e.is_action_pressed(&"ui_back"):
		get_tree().change_scene_to_file("res://SkinSelect.tscn")

# Route a left-click by the modules' hit-targets. While dead, the only
# clickable target is the Redeploy button on the results panel; shop rects are
# stale and must not fire.
func _handle_click(pos: Vector2) -> void:
	if sim != null and view.is_dead():
		if _results.redeploy_hit(pos):
			_redeploy()
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
func _queue_intent(code: int, slot: int = 0) -> void:
	if _intents.size() < MAX_QUEUED_INTENTS:
		_intents.append([code, slot])

# Start a fresh single-arena run in-place. Rebuilds the sim exactly as _ready()
# does — plain new_match(randi()), no challenge (single-arena never applies
# Profile.active_challenge_code) — then resets every render/juice/FX tracker so
# nothing leaks across runs. Clearing _recorded re-arms the once-per-run
# achievement latch so the new run credits its own play.
func _redeploy() -> void:
	sim = StSim.new_match(randi())
	view = SimView.of_sim(sim)
	fx = Fx.new()
	_recorded = false
	_intents.clear()
	pending_code = 0
	pending_slot = 0
	_shop.pending_code = 0
	_shop.pending_slot = 0
	# Re-arm the death-edge audio tracker for the fresh run.
	_au_was_dead = false
	_wire_modules()

func _physics_process(_delta: float) -> void:
	if sim == null:
		return
	# Drain exactly ONE queued intent this tick (FIFO — earlier of two same-tick
	# inputs is no longer lost; the later one simply runs next tick). What the
	# sim receives per tick is unchanged: a single (code, slot) pair.
	if _intents.is_empty():
		pending_code = 0
		pending_slot = 0
	else:
		var intent: Array = _intents.pop_front()
		pending_code = intent[0]
		pending_slot = intent[1]
	# Mirror the consumed intent to the shop for its pressed-state feedback.
	_shop.pending_code = pending_code
	_shop.pending_slot = pending_slot
	# Capture the input intent + pre-step gold so the audio hook can tell a
	# successful buy (gold actually dropped) from a no-op click. Read-only.
	var au_intent := pending_code
	var au_gold_before: int = view.gold()
	# This tick's sim→render events, drained EXACTLY ONCE right after step and
	# fanned out below to (a) the arena juice and (b) the audio hooks. Nobody
	# re-queries the stream (take_events is read-and-clear at the binding).
	var events: Array = []
	if not view.is_dead():
		sim.step(pending_code, pending_slot)
		events = view.take_events()
		if pending_code == 3:
			# Clear is input-driven (no sim event): arm the shockwave + voice it
			# here, the one place the consumed intent is known.
			_arena.trigger_clear()
			Audio.play(&"clear")
	elif not _recorded:
		# Credit your own run's achievements from how you actually played.
		_recorded = true
		var rec: Dictionary = view.stats_record()
		rec["won"] = false                                  # single-arena: no opponents
		for id in Profile.record_match(rec):
			print("Achievement unlocked: ", Profile.ach_def(id).get("name", id))
	_arena.tick_juice(events)
	_update_audio(events, au_intent, au_gold_before)
	_sync_dead_panels()

# Frame-rate cosmetic update: advance the FX bus and feed its shake into the
# camera offset (the world canvas shakes as one; the UI CanvasLayer doesn't).
func _process(delta: float) -> void:
	if sim == null or fx == null:
		return
	fx.update(delta)
	if _camera:
		_camera.offset = fx.shake_offset()

# AUDIO (render-only): every gameplay SFX now fires from the drained event
# stream — the SAME Array the juice fan-out consumed, decoded once by SimView.
# STRICTLY ONE-WAY: nothing here calls sim.step() or otherwise writes the sim;
# audio cannot influence determinism.
#   `events`     = this tick's drained sim events (empty while dead)
#   `intent`     = this tick's pending_code (1 buy · 2 reroll · 3 clear · else none)
#   `gold_before`= gold sampled BEFORE the step, to confirm a buy actually spent.
func _update_audio(events: Array, intent: int, gold_before: int) -> void:
	# --- death edge: tank_destroyed + defeat (single-arena has no victory).
	# Your own death has NO sim event, so this edge tracker stays.
	var dead: bool = view.is_dead()
	if dead and not _au_was_dead:
		Audio.play(&"tank_destroyed")
		Audio.play(&"defeat")
	_au_was_dead = dead
	if dead:
		return   # frozen run: no further gameplay SFX while on the results panel

	# --- event-driven SFX (Audio's per-event throttle absorbs bursts) --------
	for ev in events:
		match ev.kind:
			SimView.EV_ENEMY_KILLED:
				Audio.play(&"enemy_death")
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

	# --- economy actions: input-driven (no sim event for buy/reroll) ---------
	if intent == 1 and view.gold() < gold_before:
		# A buy that spent gold this tick (a click on an unaffordable card spends
		# nothing, so it stays silent).
		Audio.play(&"buy")
	elif intent == 2:
		# Reroll: refreshes the shop whether free or paid, so always voice it.
		Audio.play(&"reroll")
