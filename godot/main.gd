# Standing Tank Defense — front-end. Owns NO game logic: it drives the Rust
# `StSim` (one deterministic tick per physics frame at 30 Hz) and renders the
# "Gaslamp Bulwark" sprite set. Controls: 1/2/3 buy slot · R reroll · Space clear.
extends Node2D

var sim                       # StSim (from the gdext binding)
var pending_code := 0         # action queued for the next tick (0 = Noop)
var pending_slot := 0
var clear_fx := 0             # frames remaining on the Clear shockwave FX

# Loaded SVG textures.
var tex := {}
var enemy_tex := []           # indexed by enemy kind: 0 grunt, 1 steam, 2 boss
var frame_tex := []           # indexed by rarity: 0..3

func _ready() -> void:
	randomize()
	sim = StSim.new_match(randi())
	tex = {
		"ground": load("res://art/env/arena_ground.svg"),
		"ring":   load("res://art/env/spawn_ring.svg"),
		"tank":   load("res://art/tank/player_tank.svg"),
		"proj":   load("res://art/projectiles/magic_orb.svg"),
		"clear":  load("res://art/fx/clear_shockwave.svg"),
		"coin":   load("res://art/ui/coin.svg"),
		"heart":  load("res://art/ui/heart.svg"),
		"weapon": load("res://art/ui/icon_weapon.svg"),
		"mod":    load("res://art/ui/icon_modifier.svg"),
		"panel":  load("res://art/ui/panel.svg"),
	}
	enemy_tex = [
		load("res://art/enemies/fel_orc_grunt.svg"),
		load("res://art/enemies/steam_tank.svg"),
		load("res://art/enemies/samwise.svg"),
	]
	frame_tex = [
		load("res://art/ui/frame_common.svg"),
		load("res://art/ui/frame_uncommon.svg"),
		load("res://art/ui/frame_rare.svg"),
		load("res://art/ui/frame_epic.svg"),
	]

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
		if pending_code == 3:
			clear_fx = 18          # trigger the Clear shockwave
	if clear_fx > 0:
		clear_fx -= 1
	pending_code = 0
	pending_slot = 0
	queue_redraw()

# ---- world units → screen pixels (tank at origin, +y up) ----
func _scale() -> float:
	var center: Vector2 = get_viewport_rect().size * 0.5
	return minf(center.x, center.y) / 1700.0

func _to_screen(wx: float, wy: float) -> Vector2:
	return get_viewport_rect().size * 0.5 + Vector2(wx * _scale(), -wy * _scale())

func _blit(t: Texture2D, center: Vector2, size: float, modulate := Color.WHITE) -> void:
	draw_texture_rect(t, Rect2(center - Vector2(size, size) * 0.5, Vector2(size, size)), false, modulate)

func _draw() -> void:
	if sim == null:
		return
	var vp: Vector2 = get_viewport_rect().size
	var font := ThemeDB.fallback_font
	var s := _scale()
	var origin := _to_screen(0, 0)

	# ---- battlefield ----
	draw_texture_rect(tex["ground"], Rect2(Vector2.ZERO, vp), false)
	var ring_d := 2.0 * 1500.0 * s / 0.90   # art ring sits at ~90% of half-canvas
	_blit(tex["ring"], origin, ring_d)

	# ---- Clear shockwave FX (under entities) ----
	if clear_fx > 0:
		var prog := 1.0 - float(clear_fx) / 18.0
		_blit(tex["clear"], origin, 200.0 + prog * (ring_d - 200.0),
			Color(1, 1, 1, 1.0 - prog * 0.7))

	# ---- projectiles ----
	for p in sim.projectiles_pos():
		_blit(tex["proj"], _to_screen(p.x, p.y), 26.0)

	# ---- enemies (sorted-ish: boss drawn big) ----
	var ep: PackedVector2Array = sim.enemies_pos()
	var ek: PackedByteArray = sim.enemies_kind()
	for i in ep.size():
		var kind := ek[i] if i < ek.size() else 0
		var t: Texture2D = enemy_tex[kind] if kind < enemy_tex.size() else enemy_tex[0]
		var size := 230.0 if kind == 2 else 74.0
		_blit(t, _to_screen(ep[i].x, ep[i].y), size)

	# ---- tank ----
	_blit(tex["tank"], origin, 124.0)

	# ---- HUD ----
	var t_arr: PackedInt64Array = sim.tank()
	var eco: PackedInt64Array = sim.economy()
	var hp: int = t_arr[2]
	var maxhp: int = t_arr[3]
	# top-left status panel
	draw_texture_rect(tex["panel"], Rect2(Vector2(12, 10), Vector2(330, 92)), false)
	_blit(tex["heart"], Vector2(40, 38), 30)
	draw_string(font, Vector2(60, 44), "%d / %d" % [maxi(hp, 0), maxhp], HORIZONTAL_ALIGNMENT_LEFT, -1, 18, Color(0.91, 0.89, 0.82))
	_blit(tex["coin"], Vector2(40, 74), 28)
	draw_string(font, Vector2(60, 80), "%d   (+%d/t)" % [eco[0], eco[1]], HORIZONTAL_ALIGNMENT_LEFT, -1, 18, Color(0.79, 0.64, 0.29))
	draw_string(font, Vector2(210, 44), "Round %d" % sim.round(), HORIZONTAL_ALIGNMENT_LEFT, -1, 16, Color(0.74, 0.84, 0.95))
	draw_string(font, Vector2(210, 80), "tick %d" % sim.tick(), HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(0.6, 0.64, 0.72))
	if sim.is_dead():
		draw_string(font, Vector2(vp.x * 0.5 - 150, vp.y * 0.5 - 220), "*** TANK DESTROYED ***", HORIZONTAL_ALIGNMENT_LEFT, -1, 28, Color(1.0, 0.35, 0.23))

	# ---- shop (left column) ----
	var names: PackedStringArray = sim.shop_names()
	var meta: PackedInt64Array = sim.shop_meta()   # [cost, flags, rarity] per offer
	draw_string(font, Vector2(16, 134), "SHOP   1/2/3 buy · R reroll · Space clear", HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(0.8, 0.8, 0.8))
	for i in names.size():
		var cost := meta[i * 3]
		var flags := meta[i * 3 + 1]
		var rarity := int(meta[i * 3 + 2])
		var is_weapon := (flags & 1) != 0
		var affordable := (flags & 2) != 0
		var y := 146 + i * 64
		var row := Color.WHITE if affordable else Color(0.45, 0.45, 0.5)
		# rarity frame + category icon
		_blit(frame_tex[clampi(rarity, 0, 3)], Vector2(42, y + 28), 56, row)
		_blit(tex["weapon"] if is_weapon else tex["mod"], Vector2(42, y + 28), 36, row)
		# label + cost
		draw_string(font, Vector2(80, y + 22), "[%d] %s" % [i + 1, names[i]], HORIZONTAL_ALIGNMENT_LEFT, 250, 15, row)
		draw_string(font, Vector2(80, y + 44), "%dg" % cost, HORIZONTAL_ALIGNMENT_LEFT, -1, 15, Color(0.79, 0.64, 0.29) if affordable else row)

	# ---- arsenal (top-right) ----
	var ay := 26
	var ax := vp.x - 250
	draw_string(font, Vector2(ax, ay - 4), "ARSENAL", HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(0.7, 0.9, 0.7))
	for line in sim.arsenal_lines():
		ay += 20
		draw_string(font, Vector2(ax, ay), line, HORIZONTAL_ALIGNMENT_LEFT, 240, 14, Color(0.7, 0.9, 0.7))
