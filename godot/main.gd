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

# UI fonts (loaded in _ready). _font is the body/HUD face; _font_head a heavier
# weight for headers. Falls back to ThemeDB if the theme resource is missing.
var _ui_theme: Theme = null
var _font: Font = null
var _font_head: Font = null

var tex := {}
var enemy_tex := []           # by kind: 0 grunt, 1 steam, 2 boss
var minion_tex := []          # summoned allies: 0 larvae, 1 spores
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
	_load_fonts()        # [UI stream] real font + theme; see _load_fonts()
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

# [UI stream] Load the Barlow Semi Condensed UI theme (real font, not the engine
# fallback). Kept in its own helper so it doesn't entangle with _load_textures().
func _load_fonts() -> void:
	_ui_theme = load("res://art/ui_theme.tres") as Theme
	# Body + header faces, each with a Noto Sans SC fallback chained in so the
	# custom-drawn HUD/shop renders CJK glyphs under zh-CN. Centralized on the
	# ArtTheme autoload (ui_font) so every draw site shares one CJK-capable face.
	_font = ArtTheme.ui_font(false)
	if _font == null:
		_font = ThemeDB.fallback_font
	# Heavier weight for headers; fall back to the body face if it won't load.
	_font_head = ArtTheme.ui_font(true)
	if _font_head == null:
		_font_head = _font

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
		ArtTheme.tex("enemies/squeakzilla_rat.svg"),  # 0 Squeakzilla
		ArtTheme.tex("enemies/fanged_death.svg"),     # 1 Fanged Death
		ArtTheme.tex("enemies/boss_hippo.svg"),       # 2 boss (The Hippocrate)
		ArtTheme.tex("enemies/doomduck.svg"),         # 3 Doomduck
		ArtTheme.tex("enemies/bacon_warthog.svg"),    # 4 Bacon
		ArtTheme.tex("enemies/bandit_rider.svg"),     # 5 Honk
		ArtTheme.tex("enemies/bonk_golem.svg"),       # 6 Bonk
		ArtTheme.tex("enemies/noperope_cobra.svg"),   # 7 Nope Rope
		ArtTheme.tex("enemies/poisonspitter.svg"),    # 8 Croak
		ArtTheme.tex("enemies/firebreather.svg"),     # 9 Spicy
		ArtTheme.tex("enemies/icebreather.svg"),      # 10 Popsicle
		ArtTheme.tex("enemies/target_dummy.svg"),     # 11 Dodo
	]
	minion_tex = [ArtTheme.tex("minions/larvae.svg"), ArtTheme.tex("minions/spores.svg")]
	frame_tex = [ArtTheme.tex("ui/frame_common.svg"), ArtTheme.tex("ui/frame_uncommon.svg"),
		ArtTheme.tex("ui/frame_rare.svg"), ArtTheme.tex("ui/frame_epic.svg")]
	_spark_tex = ArtTheme.tex("fx/hit_spark.svg")

func _unhandled_key_input(e: InputEvent) -> void:
	if not (e is InputEventKey) or not e.pressed or e.echo:
		return
	# While dead, the only live controls are Redeploy (Enter/Space) and Menu
	# (Esc); swallow the shop/number/reroll keys so a fresh run isn't dirtied.
	if sim != null and sim.is_dead():
		match e.keycode:
			KEY_ENTER, KEY_KP_ENTER, KEY_SPACE: _redeploy()
			KEY_ESCAPE: get_tree().change_scene_to_file("res://SkinSelect.tscn")
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
	# While dead, the only clickable target is the Redeploy button on the
	# results panel; shop rects are stale and must not fire.
	if sim != null and sim.is_dead():
		if redeploy_rect.has_point(e.position):
			_redeploy()
		return
	for i in shop_rects.size():
		if shop_rects[i].has_point(e.position):
			pending_code = 1; pending_slot = i; return
	if reroll_rect.has_point(e.position):
		pending_code = 2; return
	if clear_rect.has_point(e.position):
		pending_code = 3

# Results-panel "Redeploy" hit-target, recomputed by _draw_results each frame
# while dead and consulted by the click handler above.
var redeploy_rect := Rect2()

# Start a fresh single-arena run in-place. Rebuilds the sim exactly as _ready()
# does — plain new_match(randi()), no challenge (single-arena never applies
# Profile.active_challenge_code) — then resets every render/juice/FX tracker so
# nothing leaks across runs. Clearing _recorded re-arms the once-per-run
# achievement latch so the new run credits its own play.
func _redeploy() -> void:
	sim = StSim.new_match(randi())
	_prev = {}
	_flash = {}
	_poofs = []
	_muzzle = 0
	_prev_proj = 0
	_proj_history = []
	_shake = Vector2.ZERO
	clear_fx = 0
	_recorded = false
	pending_code = 0
	pending_slot = 0
	fx = Fx.new()
	redeploy_rect = Rect2()
	queue_redraw()

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
		elif k == 6:              # Bonk
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
		fx.add_shake(0.025)   # tiny recoil kick on fire
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
		fx.add_shake(0.35)
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
	var font: Font = _font if _font else ThemeDB.fallback_font
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

	# summoned allies (Larvae / Spores) — drawn beneath the tank
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

func _draw_hud(font: Font, vp: Vector2) -> void:
	var head: Font = _font_head if _font_head else font
	var ta: PackedInt64Array = sim.tank()
	var eco: PackedInt64Array = sim.economy()
	draw_texture_rect(tex["panel"], Rect2(Vector2(12, 10), Vector2(330, 92)), false)
	_blit(tex["heart"], Vector2(40, 38), 30)
	draw_string(head, Vector2(60, 45), "%d / %d" % [maxi(ta[2], 0), ta[3]], HORIZONTAL_ALIGNMENT_LEFT, -1, 19, ArtTheme.ui("hp"))
	_blit(tex["coin"], Vector2(40, 74), 28)
	draw_string(head, Vector2(60, 81), "%d" % eco[0], HORIZONTAL_ALIGNMENT_LEFT, -1, 19, ArtTheme.ui("coin"))
	var gold_w := head.get_string_size("%d" % eco[0], HORIZONTAL_ALIGNMENT_LEFT, -1, 19).x
	draw_string(font, Vector2(60 + gold_w + 8, 81), "+%d/t" % eco[1], HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("accent_dim"))
	draw_string(head, Vector2(212, 45), tr("ROUND %d") % sim.round(), HORIZONTAL_ALIGNMENT_LEFT, -1, 16, ArtTheme.ui("header"))
	draw_string(font, Vector2(212, 81), tr("tick %d") % sim.tick(), HORIZONTAL_ALIGNMENT_LEFT, -1, 13, ArtTheme.ui("text_dim"))
	_draw_arsenal(font, head, vp)
	# While dead the bottom shop bar is frozen/irrelevant — replace it with the
	# results panel so the destroyed-run summary owns the screen.
	if sim.is_dead():
		_draw_results(font, head, vp)
	else:
		_draw_shop(font, vp)

# Centered run-summary panel shown on tank death. Reads the (still-valid) sim
# for round/stats/arsenal and Profile.last_unlocks for this run's achievements,
# and offers Redeploy (Enter/Space/click) or Menu (Esc). Cosmetic only; it never
# touches the sim. Sets `redeploy_rect` for the click handler.
func _draw_results(font: Font, head: Font, vp: Vector2) -> void:
	# Dim the arena behind the panel so the summary reads cleanly.
	draw_rect(Rect2(Vector2.ZERO, vp), Color(0.02, 0.02, 0.04, 0.55))

	var st: PackedInt64Array = sim.stats()
	var dmg: int = st[0] if st.size() > 0 else 0
	var gold: int = st[1] if st.size() > 1 else 0
	var bought: int = st[3] if st.size() > 3 else 0
	var rnd: int = sim.round()
	var owned: PackedStringArray = sim.arsenal_lines()

	# Panel geometry, centered.
	var pw := 560.0
	var unlock_n: int = Profile.last_unlocks.size()
	var ph := 372.0 + maxf(float(unlock_n), 0.0) * 22.0
	var px := vp.x * 0.5 - pw * 0.5
	var py := vp.y * 0.5 - ph * 0.5
	var panel := Rect2(Vector2(px, py), Vector2(pw, ph))
	draw_rect(panel, ArtTheme.ui("panel_bg"))
	draw_rect(panel, ArtTheme.ui("danger").darkened(0.5), false, 2.0)
	# Emissive top rule so it blooms under glow (boost danger to HDR for bloom).
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, 3)), ArtTheme.ui("danger") * 1.4)

	var cx := vp.x * 0.5
	# Title.
	var title := tr("TANK DESTROYED")
	var tw := head.get_string_size(title, HORIZONTAL_ALIGNMENT_LEFT, -1, 34).x
	draw_string(head, Vector2(cx - tw * 0.5, py + 48), title, HORIZONTAL_ALIGNMENT_LEFT, -1, 34, ArtTheme.ui("danger") * 1.4)
	var sub := tr("Run summary")
	var sw := font.get_string_size(sub, HORIZONTAL_ALIGNMENT_LEFT, -1, 15).x
	draw_string(font, Vector2(cx - sw * 0.5, py + 72), sub, HORIZONTAL_ALIGNMENT_LEFT, -1, 15, ArtTheme.ui("text_dim"))

	# Stat rows (label left, value right).
	var lx := px + 36.0
	var rx := px + pw - 36.0
	var ry := py + 110.0
	var rstep := 30.0
	_draw_stat_row(font, head, lx, rx, ry, tr("Round reached"), "%d" % rnd, ArtTheme.ui("header"))
	ry += rstep
	_draw_stat_row(font, head, lx, rx, ry, tr("Damage dealt"), "%d" % dmg, ArtTheme.ui("accent"))
	ry += rstep
	_draw_stat_row(font, head, lx, rx, ry, tr("Gold earned"), "%d" % gold, ArtTheme.ui("coin"))
	ry += rstep
	_draw_stat_row(font, head, lx, rx, ry, tr("Weapons bought"), "%d" % bought, ArtTheme.ui("text"))
	ry += rstep + 4.0

	# Owned arsenal, condensed onto one wrapped line. `owned` entries are
	# Rust-sourced "Name xN" lines; translate each name (preserving the xN tail).
	draw_string(head, Vector2(lx, ry), tr("ARSENAL"), HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("header"))
	ry += 20.0
	var ars := "  ·  ".join(_tr_arsenal(owned)) if owned.size() > 0 else tr("— nothing acquired —")
	draw_string(font, Vector2(lx, ry), ars, HORIZONTAL_ALIGNMENT_LEFT, pw - 72.0, 13, ArtTheme.ui("text"))
	ry += 30.0

	# Achievements unlocked this run.
	draw_string(head, Vector2(lx, ry), tr("ACHIEVEMENTS UNLOCKED"), HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("accent"))
	ry += 20.0
	if unlock_n == 0:
		draw_string(font, Vector2(lx, ry), tr("— none this run —"), HORIZONTAL_ALIGNMENT_LEFT, pw - 72.0, 13, ArtTheme.ui("text_dim"))
		ry += 22.0
	else:
		for id in Profile.last_unlocks:
			var nm: String = Profile.ach_def(id).get("name", id)
			draw_string(font, Vector2(lx, ry), "★ " + tr(nm), HORIZONTAL_ALIGNMENT_LEFT, pw - 72.0, 14, ArtTheme.ui("accent"))
			ry += 22.0

	# Redeploy button + Esc prompt.
	var btn_w := 220.0
	var btn_h := 40.0
	redeploy_rect = Rect2(cx - btn_w * 0.5, py + ph - 64.0, btn_w, btn_h)
	var mpos := get_viewport().get_mouse_position()
	var hovered := redeploy_rect.has_point(mpos)
	var btn_base := ArtTheme.ui("accent").darkened(0.7)
	var bbg := btn_base.lightened(0.08) if hovered else btn_base
	draw_rect(redeploy_rect, bbg)
	var btn_border := ArtTheme.ui("accent")
	btn_border.a = 0.7
	draw_rect(redeploy_rect, btn_border, false, 1.5)
	var blabel := tr("REDEPLOY")
	var blw := head.get_string_size(blabel, HORIZONTAL_ALIGNMENT_LEFT, -1, 18).x
	draw_string(head, redeploy_rect.position + Vector2(btn_w * 0.5 - blw * 0.5, 27), blabel, HORIZONTAL_ALIGNMENT_LEFT, -1, 18, ArtTheme.ui("accent").lightened(0.3))
	var prompt := tr("[Enter] Redeploy   ·   [Esc] Menu")
	var pwid := font.get_string_size(prompt, HORIZONTAL_ALIGNMENT_LEFT, -1, 14).x
	draw_string(font, Vector2(cx - pwid * 0.5, py + ph - 12.0), prompt, HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("text_dim"))

# One label/value row for the results panel.
func _draw_stat_row(font: Font, head: Font, lx: float, rx: float, y: float, label: String, value: String, vcol: Color) -> void:
	draw_string(font, Vector2(lx, y), label, HORIZONTAL_ALIGNMENT_LEFT, -1, 16, ArtTheme.ui("text_dim"))
	var vw := head.get_string_size(value, HORIZONTAL_ALIGNMENT_LEFT, -1, 18).x
	draw_string(head, Vector2(rx - vw, y + 1), value, HORIZONTAL_ALIGNMENT_LEFT, -1, 18, vcol)

# Translate each "Name xN" arsenal line: tr() the name, keep the " xN" tail
# (scaffolding). Used by the results-panel arsenal summary.
func _tr_arsenal(lines: PackedStringArray) -> PackedStringArray:
	var out: PackedStringArray = []
	for line in lines:
		var sp := line.rfind(" x")
		if sp > 0:
			out.append(tr(line.substr(0, sp)) + line.substr(sp))
		else:
			out.append(tr(line))
	return out

# C3 — Arsenal panel: a framed list (top-right) of owned weapons/mods with a
# header and right-aligned counts pulled from `sim.arsenal_lines()` ("Name xN").
func _draw_arsenal(font: Font, head: Font, vp: Vector2) -> void:
	var lines: PackedStringArray = sim.arsenal_lines()
	var pw := 234.0
	var px := vp.x - pw - 12.0
	var py := 12.0
	var row_h := 19.0
	var head_h := 26.0
	var ph := head_h + 8.0 + maxf(float(lines.size()), 1.0) * row_h + 6.0
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, ph)), ArtTheme.ui("panel_bg"))
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, ph)), ArtTheme.ui("panel_border"), false, 1.0)
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, head_h)), ArtTheme.ui("panel_border").darkened(0.4))
	draw_string(head, Vector2(px + 10, py + 18), tr("ARSENAL"), HORIZONTAL_ALIGNMENT_LEFT, -1, 15, ArtTheme.ui("header"))
	var ct := "%d" % lines.size()
	var ctw := font.get_string_size(ct, HORIZONTAL_ALIGNMENT_LEFT, -1, 14).x
	draw_string(font, Vector2(px + pw - ctw - 10, py + 18), ct, HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("text_dim"))
	var ay := py + head_h + 8.0
	if lines.is_empty():
		draw_string(font, Vector2(px + 10, ay + 12), tr("— nothing yet —"), HORIZONTAL_ALIGNMENT_LEFT, pw - 20, 13, ArtTheme.ui("text_dim"))
		return
	for line in lines:
		# Split "Name xN" so the count can be right-aligned for legibility.
		# `nm` is a Rust-sourced weapon/mod name — translate it; "xN" is scaffolding.
		var nm := line
		var cnt := ""
		var sp := line.rfind(" x")
		if sp > 0:
			nm = line.substr(0, sp)
			cnt = line.substr(sp + 1)   # "xN"
		draw_string(font, Vector2(px + 10, ay + 13), tr(nm), HORIZONTAL_ALIGNMENT_LEFT, pw - 56, 14, ArtTheme.ui("text"))
		if cnt != "":
			var cw := font.get_string_size(cnt, HORIZONTAL_ALIGNMENT_LEFT, -1, 14).x
			draw_string(font, Vector2(px + pw - cw - 10, ay + 13), cnt, HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("accent_dim"))
		ay += row_h

# Bottom shop bar: every offer this round as a clickable card, plus reroll and
# clear. Slots persist until rerolled or the round refreshes (every 30s), so you
# can keep buying from them. Click a card or press its number to buy.
func _draw_shop(font: Font, vp: Vector2) -> void:
	var head: Font = _font_head if _font_head else font
	var names: PackedStringArray = sim.shop_names()
	var meta: PackedInt64Array = sim.shop_meta()
	var desc: PackedStringArray = sim.shop_desc()
	var eco: PackedInt64Array = sim.economy()
	var gold: int = eco[0] if eco.size() > 0 else 0
	var free_rr: int = eco[2] if eco.size() > 2 else 0
	var rr_cost: int = eco[3] if eco.size() > 3 else 0
	var mpos := get_viewport().get_mouse_position()

	var bar_h := 162.0
	var y0 := vp.y - bar_h
	draw_rect(Rect2(Vector2(0, y0), Vector2(vp.x, bar_h)), ArtTheme.ui("panel_bg"))
	draw_rect(Rect2(Vector2(0, y0), Vector2(vp.x, 2)), ArtTheme.ui("panel_border"))
	var shop_hdr := tr("SHOP")
	draw_string(head, Vector2(16, y0 + 20), shop_hdr, HORIZONTAL_ALIGNMENT_LEFT, -1, 15, ArtTheme.ui("header"))
	var sw := head.get_string_size(shop_hdr, HORIZONTAL_ALIGNMENT_LEFT, -1, 15).x
	draw_string(font, Vector2(16 + sw + 10, y0 + 20),
		tr("click a card or press [1-8] to buy  ·  refreshes every round (30s)  ·  buy as many as you can afford"),
		HORIZONTAL_ALIGNMENT_LEFT, -1, 13, ArtTheme.ui("text_dim"))

	var n := names.size()
	var split := 4              # slots 0-3 are weapons, 4-7 are economy/passives/spikes
	var btn_w := 156.0
	var left := 14.0
	var top := y0 + 54.0
	var ch := bar_h - 64.0
	var gap := 8.0
	var group_gap := 30.0
	var area := vp.x - left - btn_w - 16.0
	var cw := (area - group_gap - gap * (n - 2)) / maxf(n, 1)

	shop_rects.clear()
	var x := left
	for i in n:
		if i == split:
			# divider between the two groups
			draw_rect(Rect2(Vector2(x - group_gap * 0.5 - gap * 0.5, top - 18), Vector2(2, ch + 18)), ArtTheme.ui("panel_border"))
			x += group_gap - gap
		var r := Rect2(x, top, cw, ch)
		shop_rects.append(r)
		var cost: int = meta[i * 3]
		var flags: int = meta[i * 3 + 1]
		var rarity := int(meta[i * 3 + 2])
		var is_weapon := (flags & 1) != 0
		var affordable := (flags & 2) != 0
		var tip := desc[i * 2 + 1] if i * 2 + 1 < desc.size() else ""
		var fg := ArtTheme.ui("text") if affordable else ArtTheme.ui("text_dim").darkened(0.2)
		# C5 hover/press feedback (read-only — uses the same rects/inputs as the handlers).
		var hovered := r.has_point(mpos)
		var pressed := pending_code == 1 and pending_slot == i
		var bg := ArtTheme.ui("panel_bg").lightened(0.06) if affordable else ArtTheme.ui("panel_bg")
		bg.a = 1.0
		if pressed:
			bg = bg.lightened(0.10) if affordable else bg
		elif hovered and affordable:
			bg = bg.lightened(0.06)
		draw_rect(r, bg)
		var rc := _rarity_color(rarity)
		draw_rect(Rect2(r.position, Vector2(r.size.x, 3)), rc if affordable else rc.darkened(0.55))
		# category pip (top-right): weapon vs economy/passive/spike
		var cat := _shop_category(is_weapon, names[i], tip)
		draw_rect(Rect2(r.position + Vector2(cw - 12, 6), Vector2(7, 7)), cat[1] if affordable else (cat[1] as Color).darkened(0.5))
		var icx := r.position + Vector2(cw * 0.5, 26)
		_blit(frame_tex[clampi(rarity, 0, 3)], icx, 46, fg)
		_blit(tex["weapon"] if is_weapon else tex["mod"], icx, 30, fg)
		draw_string(head, r.position + Vector2(6, 16), "%d" % (i + 1), HORIZONTAL_ALIGNMENT_LEFT, -1, 13, ArtTheme.ui("text_dim"))
		# Rust-sourced name/tip: translate at the draw boundary (category logic
		# above still keys off the English text).
		draw_string(head, r.position + Vector2(6, ch - 38), _fit(font, tr(names[i]), 13, cw - 12), HORIZONTAL_ALIGNMENT_LEFT, cw - 10, 13, fg)
		# C2 self-describing effect line (the mechanical tip), truncated to fit.
		var eff_col := ArtTheme.ui("text_dim") if affordable else ArtTheme.ui("text_dim").darkened(0.3)
		draw_string(font, r.position + Vector2(6, ch - 22), _fit(font, tr(tip), 11, cw - 12), HORIZONTAL_ALIGNMENT_LEFT, cw - 10, 11, eff_col)
		# cost (left) + rarity word (right), measured for clean right-alignment.
		draw_string(head, r.position + Vector2(6, ch - 5), "%dg" % cost, HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("coin") if affordable else ArtTheme.ui("accent_dim"))
		var cat_lbl := tr(cat[0])
		draw_string(font, r.position + Vector2(cw - 8 - font.get_string_size(cat_lbl, HORIZONTAL_ALIGNMENT_LEFT, -1, 10).x, ch - 6), cat_lbl, HORIZONTAL_ALIGNMENT_LEFT, -1, 10, (cat[1] as Color).lightened(0.1) if affordable else ArtTheme.ui("text_dim").darkened(0.2))
		if i == 0:
			draw_string(head, Vector2(x, top - 6), tr("WEAPONS"), HORIZONTAL_ALIGNMENT_LEFT, -1, 12, ArtTheme.ui("header"))
		elif i == split:
			draw_string(head, Vector2(x, top - 6), tr("ECONOMY · PASSIVES · SPIKES"), HORIZONTAL_ALIGNMENT_LEFT, -1, 12, ArtTheme.ui("accent"))
		x += cw + gap

	var bx := vp.x - btn_w - 8.0
	reroll_rect = Rect2(bx, top, btn_w, ch * 0.5 - 4.0)
	clear_rect = Rect2(bx, top + ch * 0.5 + 4.0, btn_w, ch * 0.5 - 4.0)
	var rr_ok := free_rr > 0 or gold >= rr_cost
	var rr_label := tr("REROLL  free x%d") % free_rr if free_rr > 0 else tr("REROLL  %dg") % rr_cost
	var rr_on := ArtTheme.ui("header").darkened(0.7)
	var rr_bg := _btn_bg(rr_on, ArtTheme.ui("panel_border").darkened(0.45), rr_ok, reroll_rect.has_point(mpos), pending_code == 2)
	draw_rect(reroll_rect, rr_bg)
	if reroll_rect.has_point(mpos) and rr_ok:
		var rr_outline := ArtTheme.ui("header")
		rr_outline.a = 0.5
		draw_rect(reroll_rect, rr_outline, false, 1.0)
	draw_string(head, reroll_rect.position + Vector2(12, reroll_rect.size.y * 0.5 + 5), "[R] " + rr_label,
		HORIZONTAL_ALIGNMENT_LEFT, btn_w - 18, 14, ArtTheme.ui("header") if rr_ok else ArtTheme.ui("text_dim"))
	var cl_on := ArtTheme.ui("danger").darkened(0.7)
	var cl_bg := _btn_bg(cl_on, cl_on, true, clear_rect.has_point(mpos), pending_code == 3)
	draw_rect(clear_rect, cl_bg)
	if clear_rect.has_point(mpos):
		var cl_outline := ArtTheme.ui("danger")
		cl_outline.a = 0.5
		draw_rect(clear_rect, cl_outline, false, 1.0)
	draw_string(head, clear_rect.position + Vector2(12, clear_rect.size.y * 0.5 + 5), tr("[Space] CLEAR"),
		HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("danger"))

	# Hover tooltip: flavor + mechanical tip for the card under the cursor.
	for i in shop_rects.size():
		if shop_rects[i].has_point(mpos):
			draw_rect(shop_rects[i], Color(1, 1, 1, 0.05))
			var hov := ArtTheme.ui("text")
			hov.a = 0.6
			draw_rect(shop_rects[i], hov, false, 1.0)
			var fl := desc[i * 2] if i * 2 < desc.size() else ""
			var tp := desc[i * 2 + 1] if i * 2 + 1 < desc.size() else ""
			_draw_tooltip(font, vp, shop_rects[i], names[i], fl, tp)
			break

# Button background tint for the three hover/press states (C5).
func _btn_bg(on: Color, off: Color, enabled: bool, hovered: bool, pressed: bool) -> Color:
	if not enabled:
		return off
	if pressed:
		return on.lightened(0.14)
	if hovered:
		return on.lightened(0.07)
	return on

# Truncate `s` with an ellipsis so it fits within `max_w` at `size` px (C2/C8).
func _fit(font: Font, s: String, size: int, max_w: float) -> String:
	if s == "" or font.get_string_size(s, HORIZONTAL_ALIGNMENT_LEFT, -1, size).x <= max_w:
		return s
	var ell := "…"
	while s.length() > 1 and font.get_string_size(s + ell, HORIZONTAL_ALIGNMENT_LEFT, -1, size).x > max_w:
		s = s.substr(0, s.length() - 1)
	return s.strip_edges() + ell

# Category label + pip color (C2). Weapons are one bucket; modifiers are split
# into economy / spike / passive by keywords in the name/tip (cosmetic only).
func _shop_category(is_weapon: bool, nm: String, tip: String) -> Array:
	if is_weapon:
		return ["WEAPON", Color(0.46, 0.62, 0.92)]
	var hay := (nm + " " + tip).to_lower()
	if hay.contains("gold") or hay.contains("income") or hay.contains("interest") or hay.contains("econ"):
		return ["ECONOMY", Color(0.85, 0.72, 0.34)]
	if hay.contains("spike") or hay.contains("thorn") or hay.contains("reflect"):
		return ["SPIKE", Color(0.86, 0.46, 0.4)]
	return ["PASSIVE", Color(0.62, 0.78, 0.5)]

func _draw_tooltip(font: Font, vp: Vector2, card: Rect2, nm: String, flavor: String, tip: String) -> void:
	var head: Font = _font_head if _font_head else font
	var w := 380.0
	var h := 96.0
	var x := clampf(card.position.x + card.size.x * 0.5 - w * 0.5, 8.0, vp.x - w - 8.0)
	var y := card.position.y - h - 12.0
	draw_rect(Rect2(Vector2(x, y), Vector2(w, h)), ArtTheme.ui("panel_bg"))
	draw_rect(Rect2(Vector2(x, y), Vector2(w, h)), ArtTheme.ui("panel_border"), false, 2.0)
	# Rust-sourced name / flavor / tip — translate at the draw boundary.
	draw_string(head, Vector2(x + 14, y + 26), tr(nm), HORIZONTAL_ALIGNMENT_LEFT, w - 28, 17, ArtTheme.ui("accent"))
	draw_multiline_string(font, Vector2(x + 14, y + 48), tr(flavor), HORIZONTAL_ALIGNMENT_LEFT, w - 28, 13, 2, ArtTheme.ui("text"))
	draw_string(font, Vector2(x + 14, y + h - 12), tr(tip), HORIZONTAL_ALIGNMENT_LEFT, w - 28, 13, ArtTheme.ui("text_dim"))

func _rarity_color(r: int) -> Color:
	match r:
		1: return Color(0.40, 0.80, 0.45)   # uncommon
		2: return Color(0.35, 0.60, 1.00)   # rare
		3: return Color(0.78, 0.46, 0.96)   # epic
		_: return Color(0.60, 0.60, 0.66)   # common
