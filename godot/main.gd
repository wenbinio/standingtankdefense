# Standing Tank Defense — single-arena view. Drives the Rust `StSim` (one
# deterministic tick per 30 Hz physics frame) and renders the "Gaslamp Bulwark"
# chibi set with juice: idle bob, hit flash, death poofs, muzzle flash, Clear FX.
# Controls: click a shop card or press 1-8 to buy · R reroll · Space clear ·
# Esc back to skin select · M multi-arena net demo.
extends Node2D

var sim
var pending_code := 0
var pending_slot := 0
var clear_fx := 0
var t := 0                    # frame counter (drives idle bob)

# juice trackers
var _prev := {}               # enemy id -> {pos:Vector2(world), hp:int}
var _flash := {}              # enemy id -> frames of hit-flash left
var _poofs := []              # [{wpos:Vector2, ttl:int, life:int}]
var _muzzle := 0
var _prev_proj := 0

var tex := {}
var enemy_tex := []           # by kind: 0 grunt, 1 steam, 2 boss
var minion_tex := []          # summoned allies: 0 skeleton, 1 infernal
var frame_tex := []           # by rarity 0..3

# --- juice / lighting (render-only) ---------------------------------------
var fx: Fx                    # reusable pooled FX + screen-shake/hitstop bus
var _spark_tex: Texture2D     # hit_spark.svg, used for impact pops
var _world_env: WorldEnvironment
var _vignette: ColorRect      # screen-space vignette + danger color-grade
var _vig_mat: ShaderMaterial
var _tank_light: PointLight2D # tank floor light (aether-blue, idle pulse)
var _muzzle_light: PointLight2D
var _menace_light: PointLight2D  # red boss/fire-breather menace glow
var _proj_history := []       # recent frames of projectiles_pos() (motion trail)
var _shake := Vector2.ZERO    # this-frame world-origin shake offset

# interactive shop hit-targets (recomputed each draw)
var shop_rects: Array[Rect2] = []
var reroll_rect := Rect2()
var clear_rect := Rect2()
var _recorded := false        # match-end achievements credited once

func _ready() -> void:
	randomize()
	sim = StSim.new_match(randi())
	fx = Fx.new()
	_load_textures()
	_setup_environment()

# Build the render-only lighting + post-processing rig in code (Main.tscn is a
# bare Node2D). All cosmetic; nothing here touches the sim.
func _setup_environment() -> void:
	# --- A1: real bloom via WorldEnvironment glow -------------------------
	var env := Environment.new()
	env.background_mode = Environment.BG_CANVAS
	env.glow_enabled = true
	env.glow_intensity = 0.9
	env.glow_strength = 1.05
	env.glow_bloom = 0.15
	env.glow_blend_mode = Environment.GLOW_BLEND_MODE_ADDITIVE
	# Only pixels brighter than ~1.0 (HDR) bloom — the Color(2.4,..) hit-flash,
	# bright FX sparks and the painted SVG halos. Keeps the base art clean.
	env.glow_hdr_threshold = 1.0
	env.glow_hdr_scale = 2.0
	# Bias the blur pyramid toward mid levels for a soft, wide bloom.
	env.set_glow_level(1, 0.0)
	env.set_glow_level(2, 0.4)
	env.set_glow_level(3, 0.8)
	env.set_glow_level(4, 1.0)
	env.set_glow_level(5, 0.6)
	_world_env = WorldEnvironment.new()
	_world_env.environment = env
	add_child(_world_env)

	# --- A3: CanvasModulate dims the floor so lights read -----------------
	var cm := CanvasModulate.new()
	cm.color = Color(0.62, 0.64, 0.7)   # ~0.6 ambient, faintly cool
	add_child(cm)

	# Tank floor light — aether-blue, parented under Main; moved each frame.
	_tank_light = PointLight2D.new()
	_tank_light.texture = _radial_light_tex(256)
	_tank_light.color = Color(0.42, 0.66, 1.0)
	_tank_light.energy = 1.1
	_tank_light.texture_scale = 3.4
	_tank_light.blend_mode = Light2D.BLEND_MODE_ADD
	add_child(_tank_light)

	# Muzzle flash light — brief punch reusing the _muzzle timer.
	_muzzle_light = PointLight2D.new()
	_muzzle_light.texture = _radial_light_tex(128)
	_muzzle_light.color = Color(0.5, 0.74, 1.0)
	_muzzle_light.energy = 0.0
	_muzzle_light.texture_scale = 2.0
	_muzzle_light.blend_mode = Light2D.BLEND_MODE_ADD
	add_child(_muzzle_light)

	# Cheap red menace light that hovers over the nastiest enemy on screen.
	_menace_light = PointLight2D.new()
	_menace_light.texture = _radial_light_tex(192)
	_menace_light.color = Color(1.0, 0.32, 0.22)
	_menace_light.energy = 0.0
	_menace_light.texture_scale = 2.6
	_menace_light.blend_mode = Light2D.BLEND_MODE_ADD
	add_child(_menace_light)

	# --- A2: screen-space vignette + escalating danger grade --------------
	# BackBufferCopy captures the arena so the shader can read SCREEN_TEXTURE.
	var bb := BackBufferCopy.new()
	bb.copy_mode = BackBufferCopy.COPY_MODE_VIEWPORT
	add_child(bb)
	_vig_mat = ShaderMaterial.new()
	_vig_mat.shader = load("res://shaders/vignette_grade.gdshader")
	_vig_mat.set_shader_parameter("danger", 0.0)
	_vignette = ColorRect.new()
	_vignette.material = _vig_mat
	_vignette.color = Color(1, 1, 1, 1)
	_vignette.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_vignette.set_anchors_preset(Control.PRESET_FULL_RECT)
	# Layer above the arena but the HUD (drawn by the UI agent in _draw) renders
	# on the Node2D canvas; this CanvasLayer sits between arena and HUD intent.
	var cl := CanvasLayer.new()
	cl.layer = 0
	cl.add_child(_vignette)
	add_child(cl)

# A soft radial gradient texture for PointLight2D (bright center -> transparent).
func _radial_light_tex(size: int) -> Texture2D:
	var img := Image.create(size, size, false, Image.FORMAT_RGBA8)
	var c := float(size) * 0.5
	for y in size:
		for x in size:
			var d := Vector2(x - c, y - c).length() / c
			var a := clampf(1.0 - d, 0.0, 1.0)
			a = a * a                       # soft falloff
			img.set_pixel(x, y, Color(1, 1, 1, a))
	return ImageTexture.create_from_image(img)

func _load_textures() -> void:
	tex = {
		"ground": ArtTheme.tex("env/arena_ground.svg"),
		"ring":   ArtTheme.tex("env/spawn_ring.svg"),
		"tank":   ArtTheme.tank_tex(),
		"proj":   ArtTheme.tex("projectiles/magic_orb.svg"),
		"clear":  ArtTheme.tex("fx/clear_shockwave.svg"),
		"muzzle": ArtTheme.tex("fx/muzzle_flash.svg"),
		"poof":   ArtTheme.tex("fx/death_poof.svg"),
		"coin":   ArtTheme.tex("ui/coin.svg"),
		"heart":  ArtTheme.tex("ui/heart.svg"),
		"weapon": ArtTheme.tex("ui/icon_weapon.svg"),
		"mod":    ArtTheme.tex("ui/icon_modifier.svg"),
		"panel":  ArtTheme.tex("ui/panel.svg"),
	}
	enemy_tex = [
		ArtTheme.tex("enemies/fel_orc_grunt.svg"),    # 0 grunt
		ArtTheme.tex("enemies/steam_tank.svg"),       # 1 steam tank
		ArtTheme.tex("enemies/samwise.svg"),          # 2 boss
		ArtTheme.tex("enemies/fel_orc_peon.svg"),     # 3 peon
		ArtTheme.tex("enemies/fel_orc_raider.svg"),   # 4 raider
		ArtTheme.tex("enemies/bandit_rider.svg"),     # 5 bandit rider
		ArtTheme.tex("enemies/mountain_giant.svg"),   # 6 mountain giant
		ArtTheme.tex("enemies/fel_orc_warlock.svg"),  # 7 warlock
		ArtTheme.tex("enemies/poisonspitter.svg"),    # 8 poisonspitter
		ArtTheme.tex("enemies/firebreather.svg"),     # 9 firebreather
		ArtTheme.tex("enemies/icebreather.svg"),      # 10 icebreather
		ArtTheme.tex("enemies/target_dummy.svg"),     # 11 target dummy
	]
	minion_tex = [ArtTheme.tex("minions/skeleton.svg"), ArtTheme.tex("minions/infernal.svg")]
	frame_tex = [ArtTheme.tex("ui/frame_common.svg"), ArtTheme.tex("ui/frame_uncommon.svg"),
		ArtTheme.tex("ui/frame_rare.svg"), ArtTheme.tex("ui/frame_epic.svg")]
	_spark_tex = ArtTheme.tex("fx/hit_spark.svg")

func _unhandled_key_input(e: InputEvent) -> void:
	if not (e is InputEventKey) or not e.pressed or e.echo:
		return
	# Number row 1..8 buys the matching shop slot.
	if e.keycode >= KEY_1 and e.keycode <= KEY_8:
		pending_code = 1
		pending_slot = e.keycode - KEY_1
		return
	match e.keycode:
		KEY_R: pending_code = 2
		KEY_SPACE: pending_code = 3
		KEY_T: ArtTheme.cycle(); _load_textures()
		KEY_M: get_tree().change_scene_to_file("res://Match.tscn")   # multi-arena net demo
		KEY_ESCAPE: get_tree().change_scene_to_file("res://SkinSelect.tscn")

func _unhandled_input(e: InputEvent) -> void:
	if not (e is InputEventMouseButton) or not e.pressed or e.button_index != MOUSE_BUTTON_LEFT:
		return
	for i in shop_rects.size():
		if shop_rects[i].has_point(e.position):
			pending_code = 1; pending_slot = i; return
	if reroll_rect.has_point(e.position):
		pending_code = 2; return
	if clear_rect.has_point(e.position):
		pending_code = 3

func _physics_process(_delta: float) -> void:
	if sim == null:
		return
	t += 1
	if not sim.is_dead():
		sim.step(pending_code, pending_slot)
		if pending_code == 3:
			clear_fx = 18
	elif not _recorded:
		# Credit your own run's achievements from how you actually played.
		_recorded = true
		var st: PackedInt64Array = sim.stats()
		for id in Profile.record_match({
			"damage": st[0] if st.size() > 0 else 0,
			"gold": st[1] if st.size() > 1 else 0,
			"round": sim.round(),
			"won": false,                                   # single-arena: no opponents
			"attack_mask": st[2] if st.size() > 2 else 0,
			"weapons_bought": st[3] if st.size() > 3 else 0,
			"economy_buys": st[4] if st.size() > 4 else 0,
		}):
			print("Achievement unlocked: ", Profile.ach_def(id).get("name", id))
	_update_juice()
	if clear_fx > 0:
		clear_fx -= 1
	pending_code = 0
	pending_slot = 0
	queue_redraw()

# Frame-rate cosmetic update: advance the FX bus, animate lights, drive the
# danger color-grade, and keep particles smooth above the 30 Hz sim tick.
func _process(delta: float) -> void:
	if sim == null or fx == null:
		return
	fx.update(delta)
	_shake = fx.shake_offset()

	# Tank floor light follows the tank (origin + idle bob) and pulses gently
	# in sync with the idle bob.
	if _tank_light:
		var origin := _to_screen(0, 0)
		var tbob := sin(t * 0.14) * 2.0
		_tank_light.position = origin + Vector2(0, tbob) + _shake
		_tank_light.energy = 1.0 + sin(t * 0.14) * 0.18

	# Muzzle light: brief punch driven by the _muzzle timer.
	if _muzzle_light:
		var origin2 := _to_screen(0, 0)
		_muzzle_light.position = origin2 + Vector2(0, -44) + _shake
		_muzzle_light.energy = lerpf(_muzzle_light.energy,
			2.4 if _muzzle > 0 else 0.0, 0.5)

	# Menace light hovers over the most dangerous enemy (boss, else fire/ice
	# breather, kind 9/10), pulsing red. Cheap single light.
	_update_menace_light()

	# A2: danger color-grade ramps with the round number (cosmetic read).
	if _vig_mat:
		var danger := clampf(float(sim.round()) / 18.0, 0.0, 1.0)
		_vig_mat.set_shader_parameter("danger", danger)

	queue_redraw()

# Pick the scariest on-screen enemy and park a red light on it.
func _update_menace_light() -> void:
	if _menace_light == null:
		return
	var ep: PackedVector2Array = sim.enemies_pos()
	var ek: PackedByteArray = sim.enemies_kind()
	var boss: PackedByteArray = sim.enemies_boss()
	var best := -1
	var best_score := 0
	for i in ep.size():
		var k: int = ek[i] if i < ek.size() else 0
		var is_boss: bool = i < boss.size() and boss[i] != 0
		var score := 0
		if is_boss or k == 2:
			score = 3
		elif k == 9 or k == 10:   # fire-/ice-breather
			score = 2
		elif k == 6:              # mountain giant
			score = 1
		if score > best_score:
			best_score = score
			best = i
	if best >= 0:
		var sp := _to_screen(ep[best].x, ep[best].y) + _shake
		_menace_light.position = sp
		_menace_light.energy = lerpf(_menace_light.energy,
			0.7 + sin(t * 0.25) * 0.25, 0.2)
	else:
		_menace_light.energy = lerpf(_menace_light.energy, 0.0, 0.2)

# Diff this tick's enemies vs last to spawn hit-flashes / death-poofs / muzzle,
# AND fire the cosmetic juice bus (sparks, kill bursts, shake, hit-stop, combat
# text). All render-only — driven by sim reads, never feeding the sim back.
func _update_juice() -> void:
	var ids: PackedInt64Array = sim.enemies_id()
	var pos: PackedVector2Array = sim.enemies_pos()
	var hp: PackedInt32Array = sim.enemies_hp_permille()
	var cur := {}
	for i in ids.size():
		var id := ids[i]
		var wp: Vector2 = pos[i] if i < pos.size() else Vector2.ZERO
		var h: int = hp[i] if i < hp.size() else 0
		cur[id] = {"pos": wp, "hp": h}
		if _prev.has(id) and h < _prev[id]["hp"]:
			_flash[id] = 6
			# impact spark pop + damage number at the hit location
			var sp := _to_screen(wp.x, wp.y)
			fx.burst_sparks(sp, Color(2.2, 1.3, 0.6), 6, 200.0, 0.26)
			var dmg: int = int(_prev[id]["hp"]) - h   # permille drop (proxy)
			if dmg > 30:
				fx.combat_text(sp + Vector2(0, -28),
					str(maxi(1, dmg / 10)), Color(1.0, 0.86, 0.4), 16, 38.0)
	for id in _prev:
		if not cur.has(id):
			var wp2: Vector2 = _prev[id]["pos"]
			_poofs.append({"wpos": wp2, "ttl": 12, "life": 12})
			# kill burst + shake + a "death" combat pop
			var sp2 := _to_screen(wp2.x, wp2.y)
			fx.kill_burst(sp2, Color(2.4, 1.6, 0.7))
	_prev = cur

	# Projectile motion trail: keep a short history of position frames to blit
	# fading ghosts behind each orb.
	var ppos: PackedVector2Array = sim.projectiles_pos()
	_proj_history.push_front(ppos)
	if _proj_history.size() > 5:
		_proj_history.resize(5)
	var pc: int = ppos.size()
	if pc > _prev_proj:
		_muzzle = 5
		fx.add_shake(0.05)   # tiny recoil kick on fire
	_prev_proj = pc
	for id in _flash.keys():
		_flash[id] -= 1
		if _flash[id] <= 0:
			_flash.erase(id)
	for p in _poofs:
		p["ttl"] -= 1
	_poofs = _poofs.filter(func(p): return p["ttl"] > 0)
	if _muzzle > 0:
		_muzzle -= 1

	# Clear (Space) is a big event: shockwave already drawn; add shake + flash.
	if clear_fx == 18:
		fx.add_shake(0.5)
		fx.add_flash(Color(0.7, 0.85, 1.0, 0.35), 0.22)
		fx.add_hitstop(0.05)

func _scale() -> float:
	var c: Vector2 = get_viewport_rect().size * 0.5
	return minf(c.x, c.y) / 1700.0

func _to_screen(wx: float, wy: float) -> Vector2:
	return get_viewport_rect().size * 0.5 + Vector2(wx * _scale(), -wy * _scale())

func _blit(tx: Texture2D, center: Vector2, size: float, mod := Color.WHITE) -> void:
	draw_texture_rect(tx, Rect2(center - Vector2(size, size) * 0.5, Vector2(size, size)), false, mod)

func _draw() -> void:
	if sim == null:
		return
	var vp: Vector2 = get_viewport_rect().size
	var font := ThemeDB.fallback_font
	var s := _scale()
	# Screen-shake offset (render-only) applied to the whole arena layer.
	var sh := _shake
	var origin := _to_screen(0, 0) + sh

	draw_texture_rect(tex["ground"], Rect2(Vector2.ZERO, vp), false)
	var ring_d := 2.0 * 1500.0 * s / 0.90
	_blit(tex["ring"], origin, ring_d)

	# Upgraded Clear shockwave: brighter (blooms) + a second trailing ring.
	if clear_fx > 0:
		var prog := 1.0 - float(clear_fx) / 18.0
		var ca := 1.0 - prog * 0.7
		# emissive (>1) so it blooms under glow
		_blit(tex["clear"], origin, 200.0 + prog * (ring_d - 200.0), Color(1.8, 2.0, 2.4, ca))
		if prog > 0.15:
			_blit(tex["clear"], origin, 200.0 + (prog - 0.15) * (ring_d - 200.0),
				Color(1.2, 1.5, 2.0, ca * 0.5))

	# Projectile motion trail (fading ghosts) under the live orbs.
	for hidx in range(_proj_history.size() - 1, 0, -1):
		var frame: PackedVector2Array = _proj_history[hidx]
		var ta := (1.0 - float(hidx) / float(_proj_history.size())) * 0.45
		for p in frame:
			_blit(tex["proj"], _to_screen(p.x, p.y) + sh, 26.0 * (1.0 - 0.08 * hidx),
				Color(1.0, 1.0, 1.2, ta))
	for p in sim.projectiles_pos():
		_blit(tex["proj"], _to_screen(p.x, p.y) + sh, 26.0, Color(1.5, 1.5, 1.9))

	# death poofs (under enemies)
	for poof in _poofs:
		var pr := 1.0 - float(poof["ttl"]) / float(poof["life"])
		var wp: Vector2 = poof["wpos"]
		_blit(tex["poof"], _to_screen(wp.x, wp.y) + sh, 38.0 + pr * 42.0, Color(1, 1, 1, 1.0 - pr))

	# enemies with idle bob + hit flash
	var ep: PackedVector2Array = sim.enemies_pos()
	var ek: PackedByteArray = sim.enemies_kind()
	var eid: PackedInt64Array = sim.enemies_id()
	for i in ep.size():
		var kind: int = ek[i] if i < ek.size() else 0
		var id: int = eid[i] if i < eid.size() else 0
		var bob := sin(t * 0.18 + float(id % 997) * 0.7) * 3.0
		var tx: Texture2D = enemy_tex[kind] if kind < enemy_tex.size() else enemy_tex[0]
		if tx == null:
			continue
		var size := 230.0 if kind == 2 else (118.0 if kind == 6 else 74.0)
		var mod := Color(2.4, 2.4, 2.4) if _flash.has(id) else Color.WHITE
		_blit(tx, _to_screen(ep[i].x, ep[i].y) + Vector2(0, bob) + sh, size, mod)

	# summoned allies (skeletons / infernals) — drawn beneath the tank
	var mp: PackedVector2Array = sim.minions_pos()
	var mk: PackedByteArray = sim.minions_kind()
	for i in mp.size():
		var k: int = mk[i] if i < mk.size() else 0
		var mtx: Texture2D = minion_tex[k] if k < minion_tex.size() else null
		if mtx:
			var mbob := sin(t * 0.2 + float(i) * 1.3) * 3.0
			_blit(mtx, _to_screen(mp[i].x, mp[i].y) + Vector2(0, mbob) + sh, 64.0)

	# tank with gentle bob + muzzle flash
	var tbob := sin(t * 0.14) * 2.0
	_blit(tex["tank"], origin + Vector2(0, tbob), 124.0)
	if _muzzle > 0:
		# emissive muzzle flash blooms; spark texture adds bite
		_blit(tex["muzzle"], origin + Vector2(0, -44 + tbob), 64.0,
			Color(1.8, 2.0, 2.6, float(_muzzle) / 5.0))
		if _spark_tex:
			_blit(_spark_tex, origin + Vector2(0, -44 + tbob), 40.0,
				Color(2.2, 1.6, 0.9, float(_muzzle) / 5.0))

	# Pooled custom-_draw FX (impact sparks, kill bursts, shockwaves, combat
	# text) — all in screen space, so drawn after the world layer.
	if fx:
		fx.draw(self, font)
		# Full-screen flash pulse on big hits / Clear.
		var fc := fx.flash_color()
		if fc.a > 0.001:
			draw_rect(Rect2(Vector2.ZERO, vp), fc)

	_draw_hud(font, vp)

func _draw_hud(font, vp: Vector2) -> void:
	var ta: PackedInt64Array = sim.tank()
	var eco: PackedInt64Array = sim.economy()
	draw_texture_rect(tex["panel"], Rect2(Vector2(12, 10), Vector2(330, 92)), false)
	_blit(tex["heart"], Vector2(40, 38), 30)
	draw_string(font, Vector2(60, 44), "%d / %d" % [maxi(ta[2], 0), ta[3]], HORIZONTAL_ALIGNMENT_LEFT, -1, 18, Color(0.91, 0.89, 0.82))
	_blit(tex["coin"], Vector2(40, 74), 28)
	draw_string(font, Vector2(60, 80), "%d   (+%d/t)" % [eco[0], eco[1]], HORIZONTAL_ALIGNMENT_LEFT, -1, 18, Color(0.79, 0.64, 0.29))
	draw_string(font, Vector2(210, 44), "Round %d" % sim.round(), HORIZONTAL_ALIGNMENT_LEFT, -1, 16, Color(0.74, 0.84, 0.95))
	draw_string(font, Vector2(210, 80), "tick %d" % sim.tick(), HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(0.6, 0.64, 0.72))
	if sim.is_dead():
		draw_string(font, Vector2(vp.x * 0.5 - 150, vp.y * 0.5 - 220), "*** TANK DESTROYED ***", HORIZONTAL_ALIGNMENT_LEFT, -1, 28, Color(1.0, 0.35, 0.23))

	var ay := 26
	var ax := vp.x - 250
	draw_string(font, Vector2(ax, ay - 4), "ARSENAL", HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(0.7, 0.9, 0.7))
	for line in sim.arsenal_lines():
		ay += 20
		draw_string(font, Vector2(ax, ay), line, HORIZONTAL_ALIGNMENT_LEFT, 240, 14, Color(0.7, 0.9, 0.7))

	_draw_shop(font, vp)

# Bottom shop bar: every offer this round as a clickable card, plus reroll and
# clear. Slots persist until rerolled or the round refreshes (every 30s), so you
# can keep buying from them. Click a card or press its number to buy.
func _draw_shop(font, vp: Vector2) -> void:
	var names: PackedStringArray = sim.shop_names()
	var meta: PackedInt64Array = sim.shop_meta()
	var eco: PackedInt64Array = sim.economy()
	var gold: int = eco[0] if eco.size() > 0 else 0
	var free_rr: int = eco[2] if eco.size() > 2 else 0
	var rr_cost: int = eco[3] if eco.size() > 3 else 0

	var bar_h := 152.0
	var y0 := vp.y - bar_h
	draw_rect(Rect2(Vector2(0, y0), Vector2(vp.x, bar_h)), Color(0.06, 0.07, 0.09, 0.93))
	draw_rect(Rect2(Vector2(0, y0), Vector2(vp.x, 2)), Color(0.22, 0.26, 0.32))
	draw_string(font, Vector2(16, y0 + 20),
		"SHOP — click a card or press [1-8] to buy  ·  refreshes every round (30s)  ·  buy as many as you can afford",
		HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(0.78, 0.82, 0.9))

	var n := names.size()
	var split := 4              # slots 0-3 are weapons, 4-7 are economy/passives/spikes
	var btn_w := 156.0
	var left := 14.0
	var top := y0 + 52.0
	var ch := bar_h - 62.0
	var gap := 8.0
	var group_gap := 30.0
	var area := vp.x - left - btn_w - 16.0
	var cw := (area - group_gap - gap * (n - 2)) / maxf(n, 1)

	shop_rects.clear()
	var x := left
	for i in n:
		if i == split:
			# divider between the two groups
			draw_rect(Rect2(Vector2(x - group_gap * 0.5 - gap * 0.5, top - 18), Vector2(2, ch + 18)), Color(0.2, 0.23, 0.29))
			x += group_gap - gap
		var r := Rect2(x, top, cw, ch)
		shop_rects.append(r)
		var cost: int = meta[i * 3]
		var flags: int = meta[i * 3 + 1]
		var rarity := int(meta[i * 3 + 2])
		var is_weapon := (flags & 1) != 0
		var affordable := (flags & 2) != 0
		var fg := Color.WHITE if affordable else Color(0.42, 0.42, 0.48)
		draw_rect(r, Color(0.11, 0.12, 0.15) if affordable else Color(0.075, 0.08, 0.10))
		var rc := _rarity_color(rarity)
		draw_rect(Rect2(r.position, Vector2(r.size.x, 3)), rc if affordable else rc.darkened(0.55))
		var icx := r.position + Vector2(cw * 0.5, 30)
		_blit(frame_tex[clampi(rarity, 0, 3)], icx, 50, fg)
		_blit(tex["weapon"] if is_weapon else tex["mod"], icx, 32, fg)
		draw_string(font, r.position + Vector2(6, 16), "%d" % (i + 1), HORIZONTAL_ALIGNMENT_LEFT, -1, 13, Color(0.5, 0.55, 0.62))
		draw_string(font, r.position + Vector2(6, ch - 24), names[i], HORIZONTAL_ALIGNMENT_LEFT, cw - 12, 12, fg)
		draw_string(font, r.position + Vector2(6, ch - 6), "%dg" % cost, HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(0.86, 0.71, 0.33) if affordable else Color(0.6, 0.4, 0.4))
		if i == 0:
			draw_string(font, Vector2(x, top - 6), "WEAPONS", HORIZONTAL_ALIGNMENT_LEFT, -1, 12, Color(0.62, 0.72, 0.86))
		elif i == split:
			draw_string(font, Vector2(x, top - 6), "ECONOMY · PASSIVES · SPIKES", HORIZONTAL_ALIGNMENT_LEFT, -1, 12, Color(0.78, 0.7, 0.5))
		x += cw + gap

	var bx := vp.x - btn_w - 8.0
	reroll_rect = Rect2(bx, top, btn_w, ch * 0.5 - 4.0)
	clear_rect = Rect2(bx, top + ch * 0.5 + 4.0, btn_w, ch * 0.5 - 4.0)
	var rr_ok := free_rr > 0 or gold >= rr_cost
	var rr_label := "REROLL  free x%d" % free_rr if free_rr > 0 else "REROLL  %dg" % rr_cost
	draw_rect(reroll_rect, Color(0.13, 0.16, 0.2) if rr_ok else Color(0.09, 0.1, 0.12))
	draw_string(font, reroll_rect.position + Vector2(12, reroll_rect.size.y * 0.5 + 5), "[R] " + rr_label,
		HORIZONTAL_ALIGNMENT_LEFT, btn_w - 18, 14, Color(0.72, 0.86, 0.96) if rr_ok else Color(0.5, 0.5, 0.55))
	draw_rect(clear_rect, Color(0.2, 0.13, 0.13))
	draw_string(font, clear_rect.position + Vector2(12, clear_rect.size.y * 0.5 + 5), "[Space] CLEAR",
		HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(0.96, 0.62, 0.52))

	# Hover tooltip: flavor + mechanical tip for the card under the cursor.
	var desc: PackedStringArray = sim.shop_desc()
	var mp := get_viewport().get_mouse_position()
	for i in shop_rects.size():
		if shop_rects[i].has_point(mp):
			draw_rect(shop_rects[i], Color(1, 1, 1, 0.06))
			var fl := desc[i * 2] if i * 2 < desc.size() else ""
			var tp := desc[i * 2 + 1] if i * 2 + 1 < desc.size() else ""
			_draw_tooltip(font, vp, shop_rects[i], names[i], fl, tp)
			break

func _draw_tooltip(font, vp: Vector2, card: Rect2, nm: String, flavor: String, tip: String) -> void:
	var w := 380.0
	var h := 92.0
	var x := clampf(card.position.x + card.size.x * 0.5 - w * 0.5, 8.0, vp.x - w - 8.0)
	var y := card.position.y - h - 12.0
	draw_rect(Rect2(Vector2(x, y), Vector2(w, h)), Color(0.04, 0.05, 0.07, 0.97))
	draw_rect(Rect2(Vector2(x, y), Vector2(w, h)), Color(0.32, 0.36, 0.44), false, 2.0)
	draw_string(font, Vector2(x + 14, y + 26), nm, HORIZONTAL_ALIGNMENT_LEFT, w - 28, 17, Color(0.96, 0.92, 0.8))
	draw_multiline_string(font, Vector2(x + 14, y + 48), flavor, HORIZONTAL_ALIGNMENT_LEFT, w - 28, 13, 2, Color(0.72, 0.76, 0.85))
	draw_string(font, Vector2(x + 14, y + h - 12), tip, HORIZONTAL_ALIGNMENT_LEFT, w - 28, 13, Color(0.6, 0.82, 0.62))

func _rarity_color(r: int) -> Color:
	match r:
		1: return Color(0.40, 0.80, 0.45)   # uncommon
		2: return Color(0.35, 0.60, 1.00)   # rare
		3: return Color(0.78, 0.46, 0.96)   # epic
		_: return Color(0.60, 0.60, 0.66)   # common
