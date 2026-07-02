# ArenaRenderer — the single-arena WORLD layer (Main.tscn child). Draws the
# ground/ring/poofs in immediate mode, renders enemies + projectiles through
# per-texture MultiMeshes (P1.6), interpolates every entity between sim ticks
# (P1.5), owns the render-only lighting + post rig (WorldEnvironment glow,
# CanvasModulate, PointLight2Ds, vignette grade), and turns the drained sim
# event stream into juice (death poofs, kill bursts, impact sparks, real
# damage numbers, oriented muzzle flash).
#
# Event flow: main.gd drains view.take_events() exactly ONCE per sim tick and
# hands the decoded Array to tick_juice(events); nothing here re-queries the
# stream. Sim reads happen ONCE per tick into the snapshot arrays below; the
# per-frame fill/draw paths consume only those snapshots (near-zero per-frame
# allocation).
#
# Lives in the default (world) canvas under Main's Camera2D, so screen shake is
# the camera's offset — no manual `+ shake` on any blit. Everything here is
# render-only: it READS the sim via SimView and never writes it.
extends Node2D

# Everything in the world canvas is dimmed by this CanvasModulate ambient so
# the lights read. The UI layer sits on a CanvasLayer (outside this canvas) and
# compensates with the same modulate to keep its colors identical.
const AMBIENT_DIM := Color(0.62, 0.64, 0.7)

const FLASH_TICKS := 6            # hit-flash duration (sim ticks)
const MUZZLE_TICKS := 5           # muzzle flash duration (sim ticks)
const POOF_CAP := 64              # death poofs alive at once (pooled; smoke shares it)

# --- P3 caps (per-effect budgets so a 200-kill tick can't explode draws) ------
const GOLD_POPS_PER_TICK := 12    # "+Ng" bounty popups emitted per tick, max
const SPAWN_RINGS_PER_TICK := 6   # spawn-telegraph rings emitted per tick, max
const HAZARD_DRAW_CAP := 48       # hazard ground decals drawn, max
const RING_PULSE_TICKS := 15      # spawn-ring emissive pulse length (~0.5 s)
const BOSS_RAMP_TICKS := 15       # boss-entrance shake ramp length (~0.5 s)
const DEATH_SEQ_TICKS := 20       # tank death staged sequence length (~0.66 s)

# Damage-type -> hazard decal color (world-space ground circles). Indexed by
# content.rs DMG_*: 0 Normal, 1 Piercing, 2 Magic, 3 Siege, 4 Chaos. Siege
# (burning oil) rides >1 red so its rim blooms under glow.
const HAZARD_COLORS: Array[Color] = [
	Color(1.0, 0.85, 0.5),    # Normal   — dusty amber
	Color(0.7, 0.95, 1.15),   # Piercing — pale steel-cyan
	Color(0.6, 0.65, 1.4),    # Magic    — arcane violet-blue
	Color(1.55, 0.65, 0.3),   # Siege    — burning-oil ember
	Color(1.45, 0.45, 1.0),   # Chaos    — fel magenta
]

# Boss-death gold fountain: 5 fanned combat texts (deterministic offsets — no
# render RNG needed, and the fan reads as a fountain).
const FOUNTAIN_OFF: Array[Vector2] = [
	Vector2(-70, -8), Vector2(-36, -32), Vector2(0, -44),
	Vector2(36, -32), Vector2(70, -8),
]

var view: SimView = null      # read-only sim view (wired by main.gd)
var fx: Fx = null             # shared juice bus (owned by main.gd)
var t := 0                    # sim-tick counter (drives idle bob)
var clear_fx := 0             # Clear shockwave countdown (armed by main.gd)

# --- per-tick sim snapshots (P1.5 interpolation) -----------------------------
# Read ONCE per sim tick in _advance_snapshot(); the 60+ Hz fill/draw paths
# lerp prev→curr with Engine.get_physics_interpolation_fraction(). The *_idx
# Dictionaries map stable id -> array index (prev side only needs it: a new id
# with no prev entry draws at curr — never lerp-from-origin).
var _e_ids := PackedInt64Array()
var _e_pos := PackedVector2Array()
var _e_kind := PackedByteArray()
var _e_boss := PackedByteArray()
var _e_status := PackedByteArray()   # P3.5 status flags (frost/poison/fire/…)
var _e_idx := {}
var _e_prev_pos := PackedVector2Array()
var _e_prev_idx := {}
var _p_ids := PackedInt64Array()
var _p_pos := PackedVector2Array()
var _p_kind := PackedInt64Array()
var _p_target := PackedVector2Array()
var _p_idx := {}
var _p_prev_pos := PackedVector2Array()
var _p_prev_idx := {}
var _m_ids := PackedInt64Array()
var _m_pos := PackedVector2Array()
var _m_kind := PackedByteArray()
var _m_idx := {}
var _m_prev_pos := PackedVector2Array()
var _m_prev_idx := {}

# --- event-driven juice state -------------------------------------------------
var _flash := {}              # enemy id -> tick the hit-flash expires
var _muzzle := 0              # muzzle flash ticks left
var _muzzle_dir := Vector2.UP # screen-space fire direction (from ProjectileSpawned)
# Death-poof pool (dense parallel arrays, swap-remove; alloc-free after _ready).
var _poof_pos := PackedVector2Array()   # world position
var _poof_ttl := PackedInt32Array()
var _poof_life := PackedInt32Array()
var _poof_scale := PackedFloat32Array() # 1.0 normal · bigger for bosses
var _poof_gray := PackedByteArray()     # 1 = gray wreck smoke (rises, dimmed)
var _poof_n := 0

# --- P3 juice state (all render-only) ----------------------------------------
var _hurt := 0.0              # TankHit vignette spike (1 -> 0 over ~0.4 s)
var _hb_phase := 0.0          # low-HP heartbeat phase (0..1, wraps)
var _ring_pulse := 0          # spawn-ring emissive pulse ticks left
var _boss_ramp := 0           # boss-entrance shake ramp ticks left
var _death_seq := 0           # tank death staged-FX ticks left
var _boss_death_at := -1      # tick for boss-death stage 2 (-1 = none; 1 slot)
var _boss_death_pos := Vector2.ZERO   # screen pos captured at the kill
var _boss_death_bounty := 0
var _gold_pops := 0           # gold popups emitted THIS tick (capped)
var _frozen_alpha := 0.0      # interpolation alpha held during hit-stop
var _hitstop_was := false
# Hazard decal snapshot (read once per tick, capped, preallocated).
var _hz_pos := PackedVector2Array()
var _hz_r := PackedFloat32Array()
var _hz_left := PackedInt32Array()
var _hz_type := PackedByteArray()
var _hz_n := 0
# damage_type -> initial hazard ticks (from HazardPlaced), for the last-20%
# fade — hazards() records carry no id/total, so remember the type's typical
# lifetime instead.
var _hz_total := {}

# --- MultiMesh entity rendering (P1.6) -----------------------------------------
# One MultiMesh per texture: enemies get one per ENEMY_MANIFEST kind (per-kind
# draw size baked into the instance transform, so the boss needs no special
# path); projectiles share ONE MultiMesh because PROJECTILE_MANIFEST defines a
# single texture today — when it grows per-weapon entries, bucket by
# projectiles_kind() exactly like the enemy kinds. Buffers grow only;
# visible_instance_count trims the draw.
var _enemy_mmi: Array = []    # MultiMeshInstance2D per enemy kind
var _proj_mmi: MultiMeshInstance2D = null
var _fg: Node2D = null        # foreground canvas: minions + tank + muzzle
var _quad: ArrayMesh = null   # shared unit quad (scaled per instance)
var _kind_count := PackedInt32Array()   # per-frame per-kind tallies (reused)
var _kind_cursor := PackedInt32Array()

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
	_poof_pos.resize(POOF_CAP)
	_poof_ttl.resize(POOF_CAP)
	_poof_life.resize(POOF_CAP)
	_poof_scale.resize(POOF_CAP)
	_poof_gray.resize(POOF_CAP)
	_hz_pos.resize(HAZARD_DRAW_CAP)
	_hz_r.resize(HAZARD_DRAW_CAP)
	_hz_left.resize(HAZARD_DRAW_CAP)
	_hz_type.resize(HAZARD_DRAW_CAP)
	reload_theme()
	_setup_environment()
	_setup_entity_layers()

# (Re)load every world texture from the ArtTheme sprite manifest. Called at
# startup and again on theme cycle; retargets the MultiMesh textures in place.
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
	for k in _enemy_mmi.size():
		_enemy_mmi[k].texture = enemy_tex[k]
	if _proj_mmi:
		_proj_mmi.texture = tex["proj"]

# Reset every juice tracker + snapshot for a fresh run (redeploy).
# `new_view`/`new_fx` replace the wrapped sim + FX bus so nothing leaks across
# runs.
func reset(new_view: SimView, new_fx: Fx) -> void:
	view = new_view
	fx = new_fx
	_e_ids = PackedInt64Array()
	_e_pos = PackedVector2Array()
	_e_kind = PackedByteArray()
	_e_boss = PackedByteArray()
	_e_status = PackedByteArray()
	_e_idx = {}
	_e_prev_pos = PackedVector2Array()
	_e_prev_idx = {}
	_p_ids = PackedInt64Array()
	_p_pos = PackedVector2Array()
	_p_kind = PackedInt64Array()
	_p_target = PackedVector2Array()
	_p_idx = {}
	_p_prev_pos = PackedVector2Array()
	_p_prev_idx = {}
	_m_ids = PackedInt64Array()
	_m_pos = PackedVector2Array()
	_m_kind = PackedByteArray()
	_m_idx = {}
	_m_prev_pos = PackedVector2Array()
	_m_prev_idx = {}
	_flash = {}
	_muzzle = 0
	_muzzle_dir = Vector2.UP
	_poof_n = 0
	clear_fx = 0
	_hurt = 0.0
	_hb_phase = 0.0
	_ring_pulse = 0
	_boss_ramp = 0
	_death_seq = 0
	_boss_death_at = -1
	_boss_death_bounty = 0
	_gold_pops = 0
	_frozen_alpha = 0.0
	_hitstop_was = false
	_hz_n = 0
	_hz_total = {}
	for mmi in _enemy_mmi:
		mmi.multimesh.visible_instance_count = 0
	if _proj_mmi:
		_proj_mmi.multimesh.visible_instance_count = 0
	queue_redraw()

# Arm the Clear shockwave (called by main.gd the tick a Clear intent stepped).
func trigger_clear() -> void:
	clear_fx = 18

# Arm the tank-death staged sequence (called ONCE by main.gd on the is_dead()
# edge; the stages themselves play out tick-by-tick in tick_juice).
func trigger_tank_death() -> void:
	_death_seq = DEATH_SEQ_TICKS

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

# Build the MultiMesh entity layers + the foreground canvas. CHILD ORDER IS
# DRAW ORDER on top of this node's own _draw (ground/ring/clear/poofs):
# projectiles under enemies under the foreground (minions/tank/muzzle) —
# the same stacking the old single _draw produced.
func _setup_entity_layers() -> void:
	_quad = _unit_quad_mesh()
	_proj_mmi = _make_mmi(tex["proj"])
	_enemy_mmi = []
	for k in enemy_tex.size():
		_enemy_mmi.append(_make_mmi(enemy_tex[k]))
	_kind_count.resize(enemy_tex.size())
	_kind_cursor.resize(enemy_tex.size())
	# Foreground canvas: a plain Node2D whose `draw` signal we feed, so
	# minions/tank/muzzle stack ABOVE the MultiMeshes without a second script.
	_fg = Node2D.new()
	_fg.draw.connect(_draw_foreground)
	add_child(_fg)

# A unit quad (centered, canvas-space UVs) shared by every MultiMesh; the
# per-instance Transform2D carries draw size + rotation.
func _unit_quad_mesh() -> ArrayMesh:
	var verts := PackedVector2Array([
		Vector2(-0.5, -0.5), Vector2(0.5, -0.5),
		Vector2(0.5, 0.5), Vector2(-0.5, 0.5),
	])
	var uvs := PackedVector2Array([
		Vector2(0, 0), Vector2(1, 0), Vector2(1, 1), Vector2(0, 1),
	])
	var indices := PackedInt32Array([0, 1, 2, 0, 2, 3])
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = verts
	arrays[Mesh.ARRAY_TEX_UV] = uvs
	arrays[Mesh.ARRAY_INDEX] = indices
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	return mesh

# One MultiMeshInstance2D per texture; 2D transforms + per-instance colors
# (hit flash; colors are float so >1 channels still bloom under glow).
func _make_mmi(texture: Texture2D) -> MultiMeshInstance2D:
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_2D
	mm.use_colors = true
	mm.mesh = _quad
	mm.instance_count = 0
	mm.visible_instance_count = 0
	var mmi := MultiMeshInstance2D.new()
	mmi.multimesh = mm
	mmi.texture = texture
	add_child(mmi)
	return mmi

# Grow-only instance buffer: instance_count only ever rises (with headroom so
# growth is rare); visible_instance_count trims the actual draw each frame.
func _ensure_capacity(mm: MultiMesh, needed: int) -> void:
	if mm.instance_count < needed:
		mm.instance_count = maxi(needed + (needed >> 1), 16)

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

# Interpolation weight for this render frame (0 = previous tick, 1 = current).
# P3.10 hit-stop: while fx.hitstop_active(), the alpha is CLAMPED to its value
# at freeze start, so every interpolated entity holds its exact pose for the
# freeze frame. Render-only — the sim keeps stepping at 30 Hz underneath and
# positions snap forward when the freeze releases (the classic hit-stop pop).
func _lerp_alpha() -> float:
	var a := clampf(Engine.get_physics_interpolation_fraction(), 0.0, 1.0)
	if fx != null and fx.hitstop_active():
		if not _hitstop_was:
			_frozen_alpha = a
			_hitstop_was = true
		return _frozen_alpha
	_hitstop_was = false
	return a

# Interpolated world position for entity `id` at current index `i`. A new id
# (no prev entry) draws at curr — never lerp-from-origin.
func _lerp_pos(prev_idx: Dictionary, prev_pos: PackedVector2Array,
		cur_pos: PackedVector2Array, id: int, i: int, alpha: float) -> Vector2:
	var pidx: int = prev_idx.get(id, -1)
	if pidx < 0 or pidx >= prev_pos.size():
		return cur_pos[i]
	return prev_pos[pidx].lerp(cur_pos[i], alpha)

# Frame-rate cosmetic update: animate lights, drive the danger color-grade,
# refill the MultiMesh instance buffers at the interpolated positions, and
# repaint. Shake is NOT applied here — the camera offset carries it for the
# whole world canvas.
func _process(delta: float) -> void:
	if view == null:
		return

	# Tank floor light follows the tank (origin + idle bob) and pulses gently
	# in sync with the idle bob.
	if _tank_light:
		var origin := to_screen(0, 0)
		var tbob := sin(t * 0.14) * 2.0
		_tank_light.position = origin + Vector2(0, tbob)
		_tank_light.energy = 1.0 + sin(t * 0.14) * 0.18

	# Muzzle light: brief punch driven by the _muzzle timer, parked at the
	# muzzle position along the actual firing direction.
	if _muzzle_light:
		var origin2 := to_screen(0, 0)
		_muzzle_light.position = origin2 + _muzzle_dir * 44.0
		_muzzle_light.energy = lerpf(_muzzle_light.energy,
			2.4 if _muzzle > 0 else 0.0, 0.5)

	# Menace light hovers over the most dangerous enemy (boss, else fire/ice
	# breather, kind 9/10), pulsing red. Cheap single light.
	_update_menace_light()

	# A2: danger color-grade ramps with the round number (cosmetic read).
	if _vig_mat:
		var danger := clampf(float(view.round_num()) / 18.0, 0.0, 1.0)
		_vig_mat.set_shader_parameter(&"danger", danger)
		# P3.1: TankHit red vignette spike, decays over ~0.4 s.
		if _hurt > 0.0:
			_hurt = maxf(0.0, _hurt - delta / 0.4)
		# P3.13: low-HP heartbeat — below 33% HP the vignette thumps, deeper and
		# faster as HP drops. Composes with (never replaces) the danger grade.
		var hb := 0.0
		var mhp := view.tank_max_hp()
		if mhp > 0 and not view.is_dead():
			var ratio := clampf(float(view.tank_hp()) / float(mhp), 0.0, 1.0)
			var severity := clampf((0.33 - ratio) / 0.33, 0.0, 1.0)
			if severity > 0.0:
				_hb_phase = fmod(_hb_phase + delta * (0.9 + severity * 1.1), 1.0)
				# lub-dub: sharp decay thump + a smaller echo at phase 0.3
				var thump := exp(-9.0 * _hb_phase) \
					+ 0.55 * exp(-9.0 * absf(_hb_phase - 0.3))
				hb = severity * 0.55 * clampf(thump, 0.0, 1.0)
			else:
				_hb_phase = 0.0
		_vig_mat.set_shader_parameter(&"hurt", _hurt)
		_vig_mat.set_shader_parameter(&"heartbeat", hb)

	# P1.5/P1.6: refill the instanced entity layers at interpolated positions.
	var alpha := _lerp_alpha()
	_fill_enemy_instances(alpha)
	_fill_projectile_instances(alpha)

	queue_redraw()
	if _fg:
		_fg.queue_redraw()

# Pick the scariest on-screen enemy (from this tick's snapshot) and park a red
# light on it.
func _update_menace_light() -> void:
	if _menace_light == null:
		return
	var best := -1
	var best_score := 0
	for i in _e_pos.size():
		var k: int = _e_kind[i] if i < _e_kind.size() else 0
		var is_boss: bool = i < _e_boss.size() and _e_boss[i] != 0
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
		var sp := to_screen(_e_pos[best].x, _e_pos[best].y)
		_menace_light.position = sp
		_menace_light.energy = lerpf(_menace_light.energy,
			0.7 + sin(t * 0.25) * 0.25, 0.2)
	else:
		_menace_light.energy = lerpf(_menace_light.energy, 0.0, 0.2)

# ============================ per-sim-tick path ==============================

# Once per sim tick (called by main.gd right after sim.step + take_events):
# advance the interpolation snapshots, then turn this tick's drained events
# into juice (poofs, bursts, sparks, damage numbers, muzzle). All render-only.
# All SFX for these events live in main.gd's _update_audio, fed the SAME
# drained array — nothing here touches the Audio bus.
func tick_juice(events: Array) -> void:
	t += 1
	_gold_pops = 0
	_advance_snapshot()

	# TankHit aggregates across the tick: many contact hits landing in one tick
	# produce ONE combined number + ONE flash/shake (self-capping by design).
	var tank_dmg := 0
	for ev in events:
		match ev.kind:
			SimView.EV_ENEMY_KILLED:
				_on_enemy_killed(ev)
			SimView.EV_IMPACT:
				_on_impact(ev)
			SimView.EV_PROJECTILE_SPAWNED:
				_on_projectile_spawned(ev)
			SimView.EV_ENEMY_DESPAWNED:
				# Deliberately NOTHING: a contact self-destruct is not a kill —
				# no poof, no kill burst (this fixes the old fake-death juice
				# from snapshot diffing).
				pass
			SimView.EV_TANK_HIT:
				tank_dmg += int(ev.damage)
			SimView.EV_ROUND_START:
				# P3.8: round banner slam + spawn-ring emissive pulse (~0.5 s).
				fx.banner(tr("ROUND %d") % int(ev.round),
					Color(1.5, 1.6, 1.9), 1.1, 54)
				_ring_pulse = RING_PULSE_TICKS
			SimView.EV_BOSS_SPAWNED:
				# P3.8: boss entrance — banner, ~0.5 s shake ramp, zoom punch.
				# The music duck already rides main.gd's boss_spawn SFX
				# (Audio.DUCK_EVENTS), so no extra duck() call here.
				fx.banner(tr("BOSS"), Color(2.0, 0.5, 0.4), 1.4, 72)
				fx.zoom_punch(0.04, 0.35)
				_boss_ramp = BOSS_RAMP_TICKS
			SimView.EV_FREEZE_PROC:
				# P3.5: small white shatter burst on the frozen enemy.
				var fi: int = _e_idx.get(ev.id, -1)
				if fi >= 0 and fi < _e_pos.size():
					fx.burst_sparks(to_screen(_e_pos[fi].x, _e_pos[fi].y),
						Color(1.8, 2.0, 2.4), 7, 170.0, 0.3)
			SimView.EV_HAZARD_PLACED:
				# Remember this type's lifetime for the decal's last-20% fade
				# (hazards() records carry ticks_left but not the total).
				_hz_total[int(ev.damage_type)] = int(ev.ticks)
			SimView.EV_GOLD_BOUNTY:
				# Per-tick aggregate — deliberately NOT a popup (it would
				# double-count the per-kill bounty texts). No coin-tick SFX
				# asset exists in Audio.EVENTS, so no audio either.
				pass
			SimView.EV_HAZARD_EXPIRED, SimView.EV_SHIELD_BROKE:
				pass   # decals expire via the hazards() snapshot; shield P3.11+
			_:
				pass

	# P3.1: tank-hit feedback (red edge flash + vignette spike + shake + number).
	if tank_dmg > 0:
		_hurt = 1.0
		fx.add_flash(Color(0.9, 0.12, 0.1, 0.28), 0.18)
		fx.add_shake(0.2)
		fx.combat_text(to_screen(0, 0) + Vector2(0, -46),
			str(tank_dmg), Color(1.6, 0.35, 0.3), 17, 44.0)

	# P3.2 stage 2 of the boss death (~0.27 s after the kill): second shockwave,
	# second spark burst, gold fountain.
	if _boss_death_at == t:
		_boss_death_at = -1
		fx.shockwave(_boss_death_pos, Color(2.2, 1.4, 0.5, 0.9), 320.0, 0.6)
		fx.burst_sparks(_boss_death_pos, Color(2.4, 1.5, 0.6), 24, 360.0, 0.55)
		fx.add_shake(0.3)
		@warning_ignore("integer_division")
		var share := maxi(1, _boss_death_bounty / 5)
		for i in FOUNTAIN_OFF.size():
			fx.combat_text(_boss_death_pos + FOUNTAIN_OFF[i], "+%dg" % share,
				Color(1.9, 1.5, 0.45), 14, 60.0 + float(i) * 6.0)

	# P3.8: boss-entrance shake ramp (small trauma per tick for ~0.5 s).
	if _boss_ramp > 0:
		_boss_ramp -= 1
		fx.add_shake(0.06)

	if _ring_pulse > 0:
		_ring_pulse -= 1

	# P3.9: tank death staged sequence — flash/slow-mo at t0, then two more
	# spark bursts across ~0.6 s; afterwards the wreck smolders (below).
	if _death_seq > 0:
		var origin := to_screen(0, 0)
		if _death_seq == DEATH_SEQ_TICKS:
			fx.add_flash(Color(1, 1, 1, 0.85), 0.3)
			fx.add_shake(0.5)
			fx.add_slowmo(0.35, 0.3)
			fx.burst_sparks(origin, Color(2.4, 1.7, 0.7), 26, 380.0, 0.55)
			fx.shockwave(origin, Color(2.2, 1.5, 0.6, 0.9), 220.0, 0.5)
		elif _death_seq == 13:
			fx.burst_sparks(origin + Vector2(-26, 10), Color(2.2, 1.2, 0.5),
				16, 300.0, 0.45)
			fx.add_shake(0.25)
		elif _death_seq == 6:
			fx.burst_sparks(origin + Vector2(30, -14), Color(2.2, 1.2, 0.5),
				16, 300.0, 0.45)
			fx.add_shake(0.25)
		_death_seq -= 1

	# P3.9: gray smoke poofs looping on the wreck while dead (shares the poof
	# pool; deterministic-looking sin/cos jitter — no render RNG).
	if view.is_dead() and (t % 9) == 0 and _poof_n < POOF_CAP:
		var jit := Vector2(sin(t * 0.7) * 60.0, cos(t * 1.3) * 45.0)
		_poof_pos[_poof_n] = jit
		_poof_ttl[_poof_n] = 24
		_poof_life[_poof_n] = 24
		_poof_scale[_poof_n] = 1.3
		_poof_gray[_poof_n] = 1
		_poof_n += 1

	# hit-flash expiry (entries are absolute expiry ticks)
	if not _flash.is_empty():
		for id in _flash.keys():
			if _flash[id] <= t:
				_flash.erase(id)

	# death-poof decay (dense pool, swap-remove)
	var i := 0
	while i < _poof_n:
		_poof_ttl[i] -= 1
		if _poof_ttl[i] <= 0:
			_poof_n -= 1
			_poof_pos[i] = _poof_pos[_poof_n]
			_poof_ttl[i] = _poof_ttl[_poof_n]
			_poof_life[i] = _poof_life[_poof_n]
			_poof_scale[i] = _poof_scale[_poof_n]
			_poof_gray[i] = _poof_gray[_poof_n]
			continue
		i += 1

	if _muzzle > 0:
		_muzzle -= 1

	# Clear (Space) is a big event: shockwave already drawn; add shake + flash
	# (the SFX plays from main.gd where the Clear intent is consumed).
	if clear_fx == 18:
		fx.add_shake(0.35)
		fx.add_flash(Color(0.7, 0.85, 1.0, 0.35), 0.22)
		fx.add_hitstop(0.05)
	if clear_fx > 0:
		clear_fx -= 1

# Rotate the snapshots: this tick's arrays become prev, then read the sim ONCE
# for the new curr. PackedArrays are copy-on-write, so the prev hand-off is a
# cheap reference move; the id->index Dictionary is built once per tick.
func _advance_snapshot() -> void:
	_e_prev_pos = _e_pos
	_e_prev_idx = _e_idx
	_e_ids = view.enemies_id()
	_e_pos = view.enemies_pos()
	_e_kind = view.enemies_kind()
	_e_boss = view.enemies_boss()
	_e_status = view.enemies_status()
	var eidx := {}
	for i in _e_ids.size():
		eidx[_e_ids[i]] = i
	_e_idx = eidx

	# P3.13: spawn telegraph — an id in curr with no prev entry just spawned;
	# pop a small ring at its position. Capped per tick; the first snapshot
	# after a reset (empty prev) is skipped so a redeploy doesn't ring the
	# whole field.
	if fx != null and not _e_prev_idx.is_empty():
		var rings := 0
		for i in _e_ids.size():
			if rings >= SPAWN_RINGS_PER_TICK:
				break
			if not _e_prev_idx.has(_e_ids[i]):
				fx.shockwave(to_screen(_e_pos[i].x, _e_pos[i].y),
					Color(1.3, 1.45, 1.7, 0.5), 26.0, 0.25)
				rings += 1

	# P3.6: hazard decal snapshot (read once per tick, hard-capped). The
	# accessor decodes to an Array of small Dictionaries — a per-TICK
	# allocation like take_events(), never per-frame.
	var hz: Array = view.hazards()
	_hz_n = mini(hz.size(), HAZARD_DRAW_CAP)
	for i in _hz_n:
		var h: Dictionary = hz[i]
		_hz_pos[i] = Vector2(float(h.x), float(h.y))
		_hz_r[i] = float(h.radius)
		_hz_left[i] = int(h.ticks_left)
		_hz_type[i] = int(h.damage_type)

	_p_prev_pos = _p_pos
	_p_prev_idx = _p_idx
	_p_ids = view.projectiles_id()
	_p_pos = view.projectiles_pos()
	_p_kind = view.projectiles_kind()
	_p_target = view.projectiles_target()
	var pidx := {}
	for i in _p_ids.size():
		pidx[_p_ids[i]] = i
	_p_idx = pidx

	_m_prev_pos = _m_pos
	_m_prev_idx = _m_idx
	_m_ids = view.minions_id()
	_m_pos = view.minions_pos()
	_m_kind = view.minions_kind()
	var midx := {}
	for i in _m_ids.size():
		midx[_m_ids[i]] = i
	_m_idx = midx

# --- event handlers -----------------------------------------------------------

# EnemyKilled {x, y, enemy_kind, boss, bounty, fire_radius}: death poof at the
# reported spot + kill burst + gold bounty popup (capped per tick). A boss kill
# opens the P3.2 multi-stage death: stage 1 here (white flash + double
# shockwave + sparks + 0.15 s hit-stop + slow-mo), stage 2 ~8 ticks later in
# tick_juice (second wave/burst + gold fountain). ev.fire_radius stays a hook.
func _on_enemy_killed(ev: Dictionary) -> void:
	var wp := Vector2(float(ev.x), float(ev.y))
	var sp := to_screen(wp.x, wp.y)
	if _poof_n < POOF_CAP:
		_poof_pos[_poof_n] = wp
		_poof_ttl[_poof_n] = 12
		_poof_life[_poof_n] = 12
		_poof_scale[_poof_n] = 2.2 if ev.boss else 1.0
		_poof_gray[_poof_n] = 0
		_poof_n += 1
	if ev.boss:
		fx.add_flash(Color(1, 1, 1, 0.7), 0.25)
		fx.burst_sparks(sp, Color(2.6, 1.8, 0.8), 30, 420.0, 0.6)
		fx.shockwave(sp, Color(2.4, 1.6, 0.7, 0.9), 200.0, 0.5)
		fx.shockwave(sp, Color(1.6, 1.9, 2.4, 0.8), 120.0, 0.4)
		fx.add_shake(0.5)
		fx.add_hitstop(0.15)
		fx.add_slowmo(0.4, 0.35)
		_boss_death_at = t + 8   # single slot: same-tick double boss merges
		_boss_death_pos = sp
		_boss_death_bounty = int(ev.bounty)
	else:
		fx.kill_burst(sp, Color(2.4, 1.6, 0.7))
		# P3.3: "+Ng" base-bounty popup — smaller + distinct from damage
		# numbers, capped so a wave wipe can't flood the text pool.
		if int(ev.bounty) > 0 and _gold_pops < GOLD_POPS_PER_TICK:
			_gold_pops += 1
			fx.combat_text(sp + Vector2(14, -12), "+%dg" % int(ev.bounty),
				Color(1.5, 1.2, 0.35), 12, 26.0)

# Impact {x, y, damage, damage_type, splash_radius}: sparks + the REAL damage
# number (replaces the old hp-permille proxy), and arm the hit-flash on the
# struck enemies (all inside the splash radius, else the nearest one).
func _on_impact(ev: Dictionary) -> void:
	var wp := Vector2(float(ev.x), float(ev.y))
	var sp := to_screen(wp.x, wp.y)
	fx.burst_sparks(sp, Color(2.2, 1.3, 0.6), 6, 200.0, 0.26)
	if ev.damage > 0:
		fx.combat_text(sp + Vector2(0, -28),
			str(ev.damage), Color(1.0, 0.86, 0.4), 16, 38.0)
	_mark_impact_flash(wp, float(ev.splash_radius))

# Arm the hit-flash for the enemies an Impact actually touched. The event
# carries a point + splash radius (world units), not victim ids, so map it to
# ids via this tick's snapshot: splash flashes everything inside the radius
# (+ a small pad for the one tick of post-hit movement), single-target flashes
# the nearest enemy within a small window.
func _mark_impact_flash(wp: Vector2, splash_r: float) -> void:
	var expiry := t + FLASH_TICKS
	if splash_r > 0.0:
		var r := splash_r + 40.0
		var r2 := r * r
		for i in _e_pos.size():
			if _e_pos[i].distance_squared_to(wp) <= r2:
				_flash[_e_ids[i]] = expiry
	else:
		var best := -1
		var best_d2 := 120.0 * 120.0
		for i in _e_pos.size():
			var d2 := _e_pos[i].distance_squared_to(wp)
			if d2 < best_d2:
				best_d2 = d2
				best = i
		if best >= 0:
			_flash[_e_ids[best]] = expiry

# ProjectileSpawned {weapon_kind, x, y, target_x, target_y}: muzzle flash
# oriented from the tank toward the target + a tiny recoil kick. The fire SFX
# plays from main.gd off the same event.
func _on_projectile_spawned(ev: Dictionary) -> void:
	_muzzle = MUZZLE_TICKS
	# World-space aim → screen-space direction (y flips across the mapping).
	var d := Vector2(float(ev.target_x - ev.x), -float(ev.target_y - ev.y))
	if d.length_squared() > 0.0001:
		_muzzle_dir = d.normalized()
	fx.add_shake(0.025)   # tiny recoil kick on fire

# ============================ per-frame fill/draw ============================

# Refill the per-kind enemy MultiMeshes: interpolated position + idle bob in
# the instance transform (per-kind draw size baked into its scale — the boss
# kind's 230 px comes straight from the manifest, no special path), hit-flash
# in the instance color. Two passes over the snapshot with reused tally
# arrays; zero heap allocation.
func _fill_enemy_instances(alpha: float) -> void:
	var nk := _enemy_mmi.size()
	if nk == 0:
		return
	for k in nk:
		_kind_count[k] = 0
	var n := _e_ids.size()
	for i in n:
		var k: int = _e_kind[i] if i < _e_kind.size() else 0
		if k >= nk:
			k = 0
		_kind_count[k] += 1
	for k in nk:
		var mm: MultiMesh = _enemy_mmi[k].multimesh
		_ensure_capacity(mm, _kind_count[k])
		_kind_cursor[k] = 0
	# Cache the world→screen mapping once for the pass.
	var vp := get_viewport_rect().size
	var s := _scale()
	var center := vp * 0.5
	for i in n:
		var k2: int = _e_kind[i] if i < _e_kind.size() else 0
		if k2 >= nk:
			k2 = 0
		var id: int = _e_ids[i]
		var wpos := _lerp_pos(_e_prev_idx, _e_prev_pos, _e_pos, id, i, alpha)
		var bob := sin(t * 0.18 + float(id % 997) * 0.7) * 3.0
		var spos := Vector2(center.x + wpos.x * s, center.y - wpos.y * s + bob)
		var size := ArtTheme.enemy_draw_size(k2)
		var mm2: MultiMesh = _enemy_mmi[k2].multimesh
		var cur := _kind_cursor[k2]
		mm2.set_instance_transform_2d(cur,
			Transform2D(0.0, Vector2(size, size), 0.0, spos))
		# Per-instance color (>1 channels bloom under glow). Composition order:
		# hit-flash WINS outright (brief, FLASH_TICKS), then stun/freeze
		# white-hold, then the multiplicative status tint stack:
		#   bit0 frost -> icy blue · bit1 poison -> green pulse ·
		#   bit2 fire -> ember orange (HDR red, blooms) · bit3 vuln -> faint
		#   purple · bit4 stun / bit5 freeze -> white flash-hold.
		var col := Color.WHITE
		if _flash.has(id):
			col = Color(2.4, 2.4, 2.4)
		else:
			var st: int = _e_status[i] if i < _e_status.size() else 0
			if st & 0x30:          # stun / freeze: held white flash
				col = Color(1.9, 1.9, 2.0)
			elif st != 0:
				var cr := 1.0
				var cg := 1.0
				var cb := 1.0
				if st & 1:         # frost
					cr *= 0.62
					cg *= 0.84
					cb *= 1.25
				if st & 2:         # poison (slow green pulse, tips over 1.0)
					var pg := 1.3 + 0.35 * sin(t * 0.35 + float(id % 61))
					cr *= 0.62
					cg *= pg
					cb *= 0.62
				if st & 4:         # fire ember glow (HDR red blooms)
					cr *= 1.75
					cg *= 0.95
					cb *= 0.55
				if st & 8:         # vulnerability
					cr *= 1.08
					cg *= 0.82
					cb *= 1.18
				col = Color(cr, cg, cb)
		mm2.set_instance_color(cur, col)
		_kind_cursor[k2] = cur + 1
	for k in nk:
		_enemy_mmi[k].multimesh.visible_instance_count = _kind_count[k]

# Refill the projectile MultiMesh: interpolated position, rotation along the
# travel direction, and the bright-orb instance color (>1 blooms — this keeps
# the old live-orb look; the ghost-trail draws are gone, real trails are P3.7).
func _fill_projectile_instances(alpha: float) -> void:
	if _proj_mmi == null:
		return
	var mm := _proj_mmi.multimesh
	var n := _p_ids.size()
	_ensure_capacity(mm, n)
	var vp := get_viewport_rect().size
	var s := _scale()
	var center := vp * 0.5
	var size := ArtTheme.projectile_draw_size()
	for i in n:
		var id: int = _p_ids[i]
		var wpos := _lerp_pos(_p_prev_idx, _p_prev_pos, _p_pos, id, i, alpha)
		var spos := Vector2(center.x + wpos.x * s, center.y - wpos.y * s)
		# Face the travel direction: prev→curr when we have a prev sample,
		# else toward the sim-reported target (fresh spawns).
		var dir := Vector2.ZERO
		var pidx: int = _p_prev_idx.get(id, -1)
		if pidx >= 0 and pidx < _p_prev_pos.size():
			dir = _p_pos[i] - _p_prev_pos[pidx]
		if dir.length_squared() < 0.0001 and i < _p_target.size():
			dir = _p_target[i] - _p_pos[i]
		var rot := Vector2(dir.x, -dir.y).angle() if dir.length_squared() > 0.0001 else 0.0
		mm.set_instance_transform_2d(i,
			Transform2D(rot, Vector2(size, size), 0.0, spos))
		mm.set_instance_color(i, Color(1.5, 1.5, 1.9))
	mm.visible_instance_count = n

# Background pass (this node's own canvas, UNDER the MultiMeshes): ground,
# spawn ring, Clear shockwave, death poofs.
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
	# P3.8: spawn-ring emissive pulse on RoundStart (~0.5 s; >1 color blooms).
	if _ring_pulse > 0:
		var rk := float(_ring_pulse) / float(RING_PULSE_TICKS)
		_blit(tex["ring"], origin, ring_d * (1.0 + (1.0 - rk) * 0.015),
			Color(1.6, 1.7, 2.0, rk * 0.55))

	# P3.6: hazard ground decals — pulsing translucent damage-type-colored
	# circles (gameplay legibility: mines/burning oil were invisible). Fades
	# out across the last ~20% of the hazard's lifetime.
	for i in _hz_n:
		var hc: Color = HAZARD_COLORS[_hz_type[i] % HAZARD_COLORS.size()]
		var hsp := to_screen(_hz_pos[i].x, _hz_pos[i].y)
		var hr := maxf(_hz_r[i] * s, 6.0)
		var pulse := 0.5 + 0.5 * sin(t * 0.22 + float(i) * 1.7)
		@warning_ignore("integer_division")
		var fade_win := maxi(1, int(_hz_total.get(int(_hz_type[i]), 90)) / 5)
		var fade := clampf(float(_hz_left[i]) / float(fade_win), 0.0, 1.0)
		draw_circle(hsp, hr, Color(hc.r, hc.g, hc.b, (0.10 + 0.07 * pulse) * fade))
		draw_arc(hsp, hr, 0.0, TAU, 40,
			Color(hc.r, hc.g, hc.b, (0.35 + 0.25 * pulse) * fade), 2.0, true)

	# Upgraded Clear shockwave: brighter (blooms) + a second trailing ring.
	if clear_fx > 0:
		var prog := 1.0 - float(clear_fx) / 18.0
		var ca := 1.0 - prog * 0.7
		# emissive (>1) so it blooms under glow
		_blit(tex["clear"], origin, 200.0 + prog * (ring_d - 200.0), Color(1.8, 2.0, 2.4, ca))
		if prog > 0.15:
			_blit(tex["clear"], origin, 200.0 + (prog - 0.15) * (ring_d - 200.0),
				Color(1.2, 1.5, 2.0, ca * 0.5))

	# death poofs (under the enemy MultiMeshes); gray entries are P3.9 wreck
	# smoke — dimmed, and they RISE as they expand instead of sitting still.
	for i in _poof_n:
		var pr := 1.0 - float(_poof_ttl[i]) / float(_poof_life[i])
		var wp := _poof_pos[i]
		var psp := to_screen(wp.x, wp.y)
		var pcol := Color(1, 1, 1, 1.0 - pr)
		if _poof_gray[i] != 0:
			psp.y -= pr * 26.0
			pcol = Color(0.45, 0.46, 0.5, (1.0 - pr) * 0.85)
		_blit(tex["poof"], psp, (38.0 + pr * 42.0) * _poof_scale[i], pcol)

# Foreground pass (drawn on _fg, ABOVE the MultiMeshes): minions, tank, and
# the oriented muzzle flash.
func _draw_foreground() -> void:
	if view == null or _fg == null:
		return
	var alpha := _lerp_alpha()
	var origin := to_screen(0, 0)

	# summoned allies (Larvae / Spores) — interpolated, beneath the tank
	for i in _m_ids.size():
		var k: int = _m_kind[i] if i < _m_kind.size() else 0
		var mtx: Texture2D = minion_tex[k] if k < minion_tex.size() else null
		if mtx == null:
			continue
		var wp := _lerp_pos(_m_prev_idx, _m_prev_pos, _m_pos, _m_ids[i], i, alpha)
		var mbob := sin(t * 0.2 + float(i) * 1.3) * 3.0
		var msize := ArtTheme.minion_draw_size(k)
		_fg.draw_texture_rect(mtx,
			Rect2(to_screen(wp.x, wp.y) + Vector2(0, mbob) - Vector2(msize, msize) * 0.5,
				Vector2(msize, msize)), false)

	# tank with gentle bob (immobile — interpolation is a fixed point)
	var tbob := sin(t * 0.14) * 2.0
	var tsize := ArtTheme.tank_draw_size()
	_fg.draw_texture_rect(tex["tank"],
		Rect2(origin + Vector2(0, tbob) - Vector2(tsize, tsize) * 0.5,
			Vector2(tsize, tsize)), false)

	# muzzle flash, oriented along the actual firing direction (event-driven)
	if _muzzle > 0:
		var mpos := origin + Vector2(0, tbob) + _muzzle_dir * 44.0
		# The flash texture is authored pointing up; rotate up onto _muzzle_dir.
		var rot := _muzzle_dir.angle() + PI / 2.0
		var ka := float(_muzzle) / float(MUZZLE_TICKS)
		_fg.draw_set_transform(mpos, rot, Vector2.ONE)
		# emissive muzzle flash blooms; spark texture adds bite
		_fg.draw_texture_rect(tex["muzzle"], Rect2(Vector2(-32, -32), Vector2(64, 64)),
			false, Color(1.8, 2.0, 2.6, ka))
		if _spark_tex:
			_fg.draw_texture_rect(_spark_tex, Rect2(Vector2(-20, -20), Vector2(40, 40)),
				false, Color(2.2, 1.6, 0.9, ka))
		_fg.draw_set_transform(Vector2.ZERO, 0.0, Vector2.ONE)
