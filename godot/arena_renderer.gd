# ArenaRenderer — the single-arena WORLD layer (Main.tscn child). Draws the
# ground/ring/entities/trails/poofs in immediate mode, owns the render-only
# lighting + post rig (WorldEnvironment glow, CanvasModulate, PointLight2Ds,
# vignette grade), and runs the per-tick JUICE diff (hit flash, death poofs,
# muzzle, sparks, combat text + their placeholder SFX).
#
# Lives in the default (world) canvas under Main's Camera2D, so screen shake is
# the camera's offset — no manual `+ shake` on any blit. Everything here is
# render-only: it READS the sim via SimView and never writes it.
extends Node2D

# Everything in the world canvas is dimmed by this CanvasModulate ambient so
# the lights read. The UI layer sits on a CanvasLayer (outside this canvas) and
# compensates with the same modulate to keep its colors identical.
const AMBIENT_DIM := Color(0.62, 0.64, 0.7)

var view: SimView = null      # read-only sim view (wired by main.gd)
var fx: Fx = null             # shared juice bus (owned by main.gd)
var t := 0                    # frame counter (drives idle bob)
var clear_fx := 0             # Clear shockwave countdown (armed by main.gd)

# juice trackers (diffed once per sim tick in tick_juice())
var _prev := {}               # enemy id -> {pos:Vector2(world), hp:int}
var _flash := {}              # enemy id -> frames of hit-flash left
var _poofs := []              # [{wpos:Vector2, ttl:int, life:int}]
var _muzzle := 0
var _prev_proj := 0
var _proj_history := []       # recent frames of projectiles_pos() (motion trail)

# textures (manifest-driven; reloaded on theme cycle)
var tex := {}
var enemy_tex := []           # by kind, from ArtTheme.ENEMY_MANIFEST
var minion_tex := []          # by kind, from ArtTheme.MINION_MANIFEST
var _spark_tex: Texture2D     # hit_spark.svg, used for impact pops

# lighting / post rig (render-only)
var _world_env: WorldEnvironment
var _vignette: ColorRect      # screen-space vignette + danger color-grade
var _vig_mat: ShaderMaterial
var _tank_light: PointLight2D # tank floor light (aether-blue, idle pulse)
var _muzzle_light: PointLight2D
var _menace_light: PointLight2D  # red boss/fire-breather menace glow

func _ready() -> void:
	reload_theme()
	_setup_environment()

# (Re)load every world texture from the ArtTheme sprite manifest. Called at
# startup and again on theme cycle.
func reload_theme() -> void:
	tex = {
		"ground": ArtTheme.tex(ArtTheme.ENV_MANIFEST["ground"]),
		"ring":   ArtTheme.tex(ArtTheme.ENV_MANIFEST["ring"]),
		"tank":   ArtTheme.tank_tex(),
		"proj":   ArtTheme.tex(ArtTheme.PROJECTILE_MANIFEST["path"]),
		"clear":  ArtTheme.tex("fx/clear_shockwave.svg"),
		"muzzle": ArtTheme.tex("fx/muzzle_flash.svg"),
		"poof":   ArtTheme.tex("fx/death_poof.svg"),
	}
	enemy_tex = ArtTheme.enemy_textures()
	minion_tex = ArtTheme.minion_textures()
	_spark_tex = ArtTheme.tex("fx/hit_spark.svg")

# Reset every juice tracker for a fresh run (redeploy). `new_view`/`new_fx`
# replace the wrapped sim + FX bus so nothing leaks across runs.
func reset(new_view: SimView, new_fx: Fx) -> void:
	view = new_view
	fx = new_fx
	_prev = {}
	_flash = {}
	_poofs = []
	_muzzle = 0
	_prev_proj = 0
	_proj_history = []
	clear_fx = 0
	queue_redraw()

# Arm the Clear shockwave (called by main.gd the tick a Clear intent stepped).
func trigger_clear() -> void:
	clear_fx = 18

# Build the render-only lighting + post-processing rig in code. All cosmetic;
# nothing here touches the sim.
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
	cm.color = AMBIENT_DIM   # ~0.6 ambient, faintly cool
	add_child(cm)

	# Tank floor light — aether-blue; moved each frame.
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
	# The rig sits on its own CanvasLayer ABOVE the UI layer (layer 2 > 1), so
	# the grade covers arena + HUD exactly as before the module split. The
	# BackBufferCopy is the layer's first child: it captures the viewport
	# AFTER the world and UI canvases have rendered, then the shader ColorRect
	# reads that copy via SCREEN_TEXTURE.
	var cl := CanvasLayer.new()
	cl.layer = 2
	var bb := BackBufferCopy.new()
	bb.copy_mode = BackBufferCopy.COPY_MODE_VIEWPORT
	cl.add_child(bb)
	_vig_mat = ShaderMaterial.new()
	_vig_mat.shader = load("res://shaders/vignette_grade.gdshader")
	_vig_mat.set_shader_parameter("danger", 0.0)
	_vignette = ColorRect.new()
	_vignette.material = _vig_mat
	_vignette.color = Color(1, 1, 1, 1)
	_vignette.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_vignette.set_anchors_preset(Control.PRESET_FULL_RECT)
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

# --- world/screen mapping ---------------------------------------------------
func _scale() -> float:
	var c: Vector2 = get_viewport_rect().size * 0.5
	return minf(c.x, c.y) / 1700.0

func to_screen(wx: float, wy: float) -> Vector2:
	return get_viewport_rect().size * 0.5 + Vector2(wx * _scale(), -wy * _scale())

func _blit(tx: Texture2D, center: Vector2, size: float, mod := Color.WHITE) -> void:
	draw_texture_rect(tx, Rect2(center - Vector2(size, size) * 0.5, Vector2(size, size)), false, mod)

# Frame-rate cosmetic update: animate lights, drive the danger color-grade,
# and keep particles smooth above the 30 Hz sim tick. Shake is NOT applied
# here — the camera offset carries it for the whole world canvas.
func _process(_delta: float) -> void:
	if view == null:
		return

	# Tank floor light follows the tank (origin + idle bob) and pulses gently
	# in sync with the idle bob.
	if _tank_light:
		var origin := to_screen(0, 0)
		var tbob := sin(t * 0.14) * 2.0
		_tank_light.position = origin + Vector2(0, tbob)
		_tank_light.energy = 1.0 + sin(t * 0.14) * 0.18

	# Muzzle light: brief punch driven by the _muzzle timer.
	if _muzzle_light:
		var origin2 := to_screen(0, 0)
		_muzzle_light.position = origin2 + Vector2(0, -44)
		_muzzle_light.energy = lerpf(_muzzle_light.energy,
			2.4 if _muzzle > 0 else 0.0, 0.5)

	# Menace light hovers over the most dangerous enemy (boss, else fire/ice
	# breather, kind 9/10), pulsing red. Cheap single light.
	_update_menace_light()

	# A2: danger color-grade ramps with the round number (cosmetic read).
	if _vig_mat:
		var danger := clampf(float(view.round_num()) / 18.0, 0.0, 1.0)
		_vig_mat.set_shader_parameter("danger", danger)

	queue_redraw()

# Pick the scariest on-screen enemy and park a red light on it.
func _update_menace_light() -> void:
	if _menace_light == null:
		return
	var ep: PackedVector2Array = view.enemies_pos()
	var ek: PackedByteArray = view.enemies_kind()
	var boss: PackedByteArray = view.enemies_boss()
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
		var sp := to_screen(ep[best].x, ep[best].y)
		_menace_light.position = sp
		_menace_light.energy = lerpf(_menace_light.energy,
			0.7 + sin(t * 0.25) * 0.25, 0.2)
	else:
		_menace_light.energy = lerpf(_menace_light.energy, 0.0, 0.2)

# Once per sim tick (called by main.gd right after sim.step): diff this tick's
# enemies vs last to spawn hit-flashes / death-poofs / muzzle, AND fire the
# cosmetic juice bus (sparks, kill bursts, shake, hit-stop, combat text). All
# render-only — driven by sim reads, never feeding the sim back. The Audio
# calls in here are the "fast" one-way SFX hooks, moved intact from main.gd
# (they will be replaced by drained events in a later pass).
func tick_juice() -> void:
	t += 1
	var ids: PackedInt64Array = view.enemies_id()
	var pos: PackedVector2Array = view.enemies_pos()
	var hp: PackedInt32Array = view.enemies_hp_permille()
	var cur := {}
	for i in ids.size():
		var id := ids[i]
		var wp: Vector2 = pos[i] if i < pos.size() else Vector2.ZERO
		var h: int = hp[i] if i < hp.size() else 0
		cur[id] = {"pos": wp, "hp": h}
		if _prev.has(id) and h < _prev[id]["hp"]:
			_flash[id] = 6
			# AUDIO (render-only): an hp drop on a still-living enemy = an impact.
			Audio.play(&"hit")
			# impact spark pop + damage number at the hit location
			var sp := to_screen(wp.x, wp.y)
			fx.burst_sparks(sp, Color(2.2, 1.3, 0.6), 6, 200.0, 0.26)
			var dmg: int = int(_prev[id]["hp"]) - h   # permille drop (proxy)
			if dmg > 30:
				fx.combat_text(sp + Vector2(0, -28),
					str(maxi(1, dmg / 10)), Color(1.0, 0.86, 0.4), 16, 38.0)
	for id in _prev:
		if not cur.has(id):
			var wp2: Vector2 = _prev[id]["pos"]
			_poofs.append({"wpos": wp2, "ttl": 12, "life": 12})
			# AUDIO (render-only): an id that was here last tick and is gone now
			# = a death (the same signal that drives the death poof).
			Audio.play(&"enemy_death")
			# kill burst + shake + a "death" combat pop
			var sp2 := to_screen(wp2.x, wp2.y)
			fx.kill_burst(sp2, Color(2.4, 1.6, 0.7))
	_prev = cur

	# Projectile motion trail: keep a short history of position frames to blit
	# fading ghosts behind each orb.
	var ppos: PackedVector2Array = view.projectiles_pos()
	_proj_history.push_front(ppos)
	if _proj_history.size() > 5:
		_proj_history.resize(5)
	var pc: int = ppos.size()
	if pc > _prev_proj:
		_muzzle = 5
		# AUDIO (render-only): more projectiles on screen than last tick = a shot
		# was fired (the same delta that triggers the muzzle flash).
		Audio.play(&"fire")
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
		# AUDIO (render-only): clear_fx was just armed by the Clear input this tick.
		Audio.play(&"clear")
		fx.add_shake(0.35)
		fx.add_flash(Color(0.7, 0.85, 1.0, 0.35), 0.22)
		fx.add_hitstop(0.05)
	if clear_fx > 0:
		clear_fx -= 1

func _draw() -> void:
	if view == null:
		return
	var vp: Vector2 = get_viewport_rect().size
	var s := _scale()
	var origin := to_screen(0, 0)

	# Ground: aspect-correct COVER (scale uniformly until both axes are filled,
	# center, crop the overflow) — replaces the old non-uniform full-viewport
	# stretch of the square arena texture.
	var ground: Texture2D = tex["ground"]
	if ground:
		var gsz := Vector2(ground.get_size())
		if gsz.x > 0.0 and gsz.y > 0.0:
			var cover := maxf(vp.x / gsz.x, vp.y / gsz.y)
			var dsz := gsz * cover
			draw_texture_rect(ground, Rect2((vp - dsz) * 0.5, dsz), false)
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
	var proj_size := ArtTheme.projectile_draw_size()
	for hidx in range(_proj_history.size() - 1, 0, -1):
		var frame: PackedVector2Array = _proj_history[hidx]
		var ta := (1.0 - float(hidx) / float(_proj_history.size())) * 0.45
		for p in frame:
			_blit(tex["proj"], to_screen(p.x, p.y), proj_size * (1.0 - 0.08 * hidx),
				Color(1.0, 1.0, 1.2, ta))
	for p in view.projectiles_pos():
		_blit(tex["proj"], to_screen(p.x, p.y), proj_size, Color(1.5, 1.5, 1.9))

	# death poofs (under enemies)
	for poof in _poofs:
		var pr := 1.0 - float(poof["ttl"]) / float(poof["life"])
		var wp: Vector2 = poof["wpos"]
		_blit(tex["poof"], to_screen(wp.x, wp.y), 38.0 + pr * 42.0, Color(1, 1, 1, 1.0 - pr))

	# enemies with idle bob + hit flash (sizes from the sprite manifest)
	var ep: PackedVector2Array = view.enemies_pos()
	var ek: PackedByteArray = view.enemies_kind()
	var eid: PackedInt64Array = view.enemies_id()
	for i in ep.size():
		var kind: int = ek[i] if i < ek.size() else 0
		var id: int = eid[i] if i < eid.size() else 0
		var bob := sin(t * 0.18 + float(id % 997) * 0.7) * 3.0
		var tx: Texture2D = enemy_tex[kind] if kind < enemy_tex.size() else enemy_tex[0]
		if tx == null:
			continue
		var size := ArtTheme.enemy_draw_size(kind)
		var mod := Color(2.4, 2.4, 2.4) if _flash.has(id) else Color.WHITE
		_blit(tx, to_screen(ep[i].x, ep[i].y) + Vector2(0, bob), size, mod)

	# summoned allies (Larvae / Spores) — drawn beneath the tank
	var mp: PackedVector2Array = view.minions_pos()
	var mk: PackedByteArray = view.minions_kind()
	for i in mp.size():
		var k: int = mk[i] if i < mk.size() else 0
		var mtx: Texture2D = minion_tex[k] if k < minion_tex.size() else null
		if mtx:
			var mbob := sin(t * 0.2 + float(i) * 1.3) * 3.0
			_blit(mtx, to_screen(mp[i].x, mp[i].y) + Vector2(0, mbob),
				ArtTheme.minion_draw_size(k))

	# tank with gentle bob + muzzle flash
	var tbob := sin(t * 0.14) * 2.0
	_blit(tex["tank"], origin + Vector2(0, tbob), ArtTheme.tank_draw_size())
	if _muzzle > 0:
		# emissive muzzle flash blooms; spark texture adds bite
		_blit(tex["muzzle"], origin + Vector2(0, -44 + tbob), 64.0,
			Color(1.8, 2.0, 2.6, float(_muzzle) / 5.0))
		if _spark_tex:
			_blit(_spark_tex, origin + Vector2(0, -44 + tbob), 40.0,
				Color(2.2, 1.6, 0.9, float(_muzzle) / 5.0))
