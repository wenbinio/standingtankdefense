# Standing Tank Defense — front-end. Owns NO game logic: it drives the Rust
# `StSim` (one deterministic tick per physics frame at 30 Hz) and draws whatever
# the sim reports. Controls: 1/2/3 buy shop slot · R reroll · Space clear.
extends Node2D

var sim                       # StSim (from the gdext binding)
var pending_code := 0         # action queued for the next tick (0 = Noop)
var pending_slot := 0

func _ready() -> void:
	randomize()
	# A random match each run; swap for a constant to reproduce a game exactly.
	sim = StSim.new_match(randi())

func _unhandled_key_input(event: InputEvent) -> void:
	if not (event is InputEventKey) or not event.pressed or event.echo:
		return
	match event.keycode:
		KEY_1: pending_code = 1; pending_slot = 0
		KEY_2: pending_code = 1; pending_slot = 1
		KEY_3: pending_code = 1; pending_slot = 2
		KEY_R: pending_code = 2
		KEY_SPACE: pending_code = 3
		KEY_ESCAPE: get_tree().quit()

func _physics_process(_delta: float) -> void:
	if sim == null:
		return
	if not sim.is_dead():
		sim.step(pending_code, pending_slot)
	pending_code = 0
	pending_slot = 0
	queue_redraw()

# World units → screen pixels. Tank sits at world origin; +y is up.
func _scale() -> float:
	var center: Vector2 = get_viewport_rect().size * 0.5
	return minf(center.x, center.y) / 1700.0

func _to_screen(wx: float, wy: float) -> Vector2:
	var center: Vector2 = get_viewport_rect().size * 0.5
	var s := _scale()
	return center + Vector2(wx * s, -wy * s)

func _draw() -> void:
	if sim == null:
		return
	var font := ThemeDB.fallback_font

	# Spawn ring (faint guide).
	draw_arc(_to_screen(0, 0), _scale() * 1500.0, 0, TAU, 64, Color(1, 1, 1, 0.08), 2.0)

	# Projectiles.
	for p in sim.projectiles_pos():
		draw_circle(_to_screen(p.x, p.y), 3.0, Color(1.0, 0.9, 0.3))

	# Enemies (boss bigger & gold).
	var ep: PackedVector2Array = sim.enemies_pos()
	var eb: PackedByteArray = sim.enemies_boss()
	for i in ep.size():
		var is_boss := i < eb.size() and eb[i] == 1
		var rad := 16.0 if is_boss else 7.0
		var col := Color(1.0, 0.55, 0.0) if is_boss else Color(0.9, 0.25, 0.25)
		draw_circle(_to_screen(ep[i].x, ep[i].y), rad, col)

	# Tank.
	var t: PackedInt64Array = sim.tank()
	var tp := _to_screen(float(t[0]), float(t[1]))
	draw_circle(tp, 12.0, Color(0.3, 0.8, 1.0))
	draw_arc(tp, 16.0, 0, TAU, 24, Color(0.3, 0.8, 1.0, 0.5), 2.0)

	# ---- HUD ----
	var eco: PackedInt64Array = sim.economy()
	var hp: int = t[2]
	var maxhp: int = t[3]
	var hud := "HP %d/%d    Gold %d (+%d/t)    Round %d    tick %d" % [maxi(hp, 0), maxhp, eco[0], eco[1], sim.round(), sim.tick()]
	if sim.is_dead():
		hud = "*** TANK DESTROYED ***    " + hud
	draw_string(font, Vector2(16, 26), hud, HORIZONTAL_ALIGNMENT_LEFT, -1, 18, Color.WHITE)

	# Shop.
	draw_string(font, Vector2(16, 56), "SHOP   (1/2/3 buy · R reroll · Space clear)", HORIZONTAL_ALIGNMENT_LEFT, -1, 15, Color(0.8, 0.8, 0.8))
	var names: PackedStringArray = sim.shop_names()
	var meta: PackedInt64Array = sim.shop_meta()
	for i in names.size():
		var cost := meta[i * 2]
		var affordable := (meta[i * 2 + 1] & 2) != 0
		var col := Color.WHITE if affordable else Color(0.5, 0.5, 0.5)
		draw_string(font, Vector2(16, 80 + i * 22), "[%d] %s  —  %dg" % [i + 1, names[i], cost], HORIZONTAL_ALIGNMENT_LEFT, -1, 15, col)

	# Arsenal (top-right).
	var y := 26
	var x := get_viewport_rect().size.x - 270
	for line in sim.arsenal_lines():
		draw_string(font, Vector2(x, y), line, HORIZONTAL_ALIGNMENT_LEFT, 260, 14, Color(0.7, 0.9, 0.7))
		y += 18
