# Multi-arena / net view. Runs the REAL netcode loop in `StMatch` (authoritative
# director + N clients + hub, bot-driven) and renders every player's
# authoritative shadow arena as a grid — the visual proof of the sharded-sim
# architecture: N independent arenas under one director, no entity replication.
#
# Per-player sim reads go through the same typed SimView wrapper as the
# single-arena view (one wrapper per player index); only m.step() and the
# match-level meta (server_tick/alive_count/match_over) touch StMatch directly.
#
# JUICE (P3): one shared Fx bus + per-cell event-driven feedback (kill sparks,
# boss flashes, tank-hit shake, elimination stamps, round pulse). All of it is
# render-only and one-way — drained events + dead flags in, pixels/audio out.
# Budgets: YOUR cell gets the full treatment and the only combat audio; peer
# cells get capped, cheaper sparks (see the cap consts below).
extends Node2D

const N := 8                  # players in the demo match

var m
var _views: Array = []        # SimView per player index (read-only wrappers)

# Per-player cosmetic assignment, built once in _ready (stable across frames).
# players[i] = {"theme": <idx into ArtTheme.themes>, "skin": <skin id string>}.
var players: Array = []

# Per-theme texture cache, keyed by theme index. Each entry:
#   {"ground": Texture2D, "ring": Texture2D, "enemies": [Texture2D...], "minions": [...]}
# Paths + kind order come from the ArtTheme sprite manifest.
var _theme_tex: Array = []
# Per-player tank texture, keyed by player index (resolved from theme+skin).
var _player_tank: Array = []

# --- P3 juice (docs/09 §9.4-P3 item 12) --------------------------------------
# ONE shared Fx bus drawn over the whole grid (screen-space), plus tiny per-cell
# cosmetic timers in preallocated packed arrays. Strictly render-only, one-way:
# everything below READS drained sim events / dead flags; nothing writes the sim.
#
# HARD CAPS (8 cells x events per tick — peers must stay cheap):
#   YOUR cell (player 0): full treatment, but at most YOUR_KILL_FX_CAP kill
#     bursts per tick (extra kills still count toward the ONE aggregated
#     play_many voice). Boss kills always render (rare).
#   PEER cells: at most PEER_KILL_FX_CAP spark bursts per cell per tick, each a
#     small PEER_SPARKS_PER_KILL-spark burst — no shockwave, no shake, no audio.
#   Fx's own pools (512 sparks / 48 waves / 64 texts) are the global backstop:
#     emits beyond a pool cap are dropped, never allocated.
const YOUR_KILL_FX_CAP := 6        # kill bursts per tick in YOUR cell
const PEER_KILL_FX_CAP := 3        # spark bursts per PEER cell per tick
const PEER_SPARKS_PER_KILL := 5    # sparks per peer kill burst (yours: 8)

# Elimination stamp animation timings (seconds, render clock).
const ELIM_FLASH_T := 0.15         # white pop
const ELIM_DESAT_T := 0.40         # darken/desaturate ramp
const ELIM_STAMP_DELAY := 0.10     # stamp starts scaling in after this
const ELIM_STAMP_T := 0.35         # stamp overshoot->settle duration

var fx: Fx = null                  # the ONE shared juice bus
@onready var _camera: Camera2D = $Camera   # shake via offset (mirrors main.gd)

# --- P5 end-of-match results panel (docs/09 §9.4-P5 item 5) -------------------
# Appears RESULTS_DELAY seconds after match_over so the final juice (victory/
# defeat flash, last elimination stamp) reads first. Everything the panel draws
# is precomputed ONCE by _build_results at match_over — _draw formats nothing.
const RESULTS_DELAY := 2.2         # seconds from match_over to panel
const RESULTS_FADE := 0.3          # backdrop dim ease-in

var _from_lobby := false           # launched by the lobby (Session handoff)?
var _over_t := 0.0                 # seconds since match_over (render clock)
var _rows: Array = []              # results rows by placement, built once
var _res_sub := ""                 # cached panel strings (zero per-frame fmt)
var _res_btn_label := ""
var _res_menu_label := ""
var _res_hint := ""
var _rematch_rect := Rect2()       # click hit-targets (recomputed each _draw,
var _menu_rect := Rect2()          # results.gd's redeploy_rect pattern)

# Per-cell transient state — preallocated to player_count() in _ready, decayed
# in _process; nothing here resizes or allocates per frame.
var _cell_pulse := PackedFloat32Array()      # kill-feedback frame pulse ttl
var _cell_flash_ttl := PackedFloat32Array()  # cell fill-flash ttl
var _cell_flash_life := PackedFloat32Array()
var _cell_flash_col := PackedColorArray()
var _edge_ttl := PackedFloat32Array()        # your-cell tank-hit red edge ttl
var _elim_t := PackedFloat32Array()          # seconds since death; <0 = alive
var _was_dead := PackedByteArray()           # death edge detector
var _cell_spark := PackedColorArray()        # per-player HDR spark tint (theme accent)
var _last_round := -1                        # global round-pulse edge detector
var _round_pulse := 0.0                      # header pulse ttl

func _ready() -> void:
	randomize()
	# If we arrived from the lobby, honor its host-authoritative plan (host + peers,
	# planned seed) instead of the standalone demo's defaults; then clear the
	# hand-off so a later direct launch falls back to the demo behavior.
	if Session.lobby_players > 0:
		m = StMatch.new_match(Session.lobby_players + 1, Session.lobby_seed)
		_from_lobby = true   # remembered past clear() for the results panel's rematch
		Session.clear()
	else:
		m = StMatch.new_match(N, randi())
	# You are player 0; honor a chosen challenge so its achievement is earnable.
	if Profile.active_challenge_code != 0:
		m.set_challenge(0, Profile.active_challenge_code)
	_views.clear()
	for i in m.player_count():
		_views.append(SimView.of_match(m, i))
	_assign_cosmetics()
	_cache_theme_textures()
	_setup_environment()
	_init_juice()
	Audio.set_music("ambient_bed.wav")   # render-only ambient bed

# Preallocate every per-cell timer array once (see the caps block above).
func _init_juice() -> void:
	fx = Fx.new()
	var n: int = m.player_count()
	_cell_pulse.resize(n)
	_cell_pulse.fill(0.0)
	_cell_flash_ttl.resize(n)
	_cell_flash_ttl.fill(0.0)
	_cell_flash_life.resize(n)
	_cell_flash_life.fill(0.0)
	_cell_flash_col.resize(n)
	_edge_ttl.resize(n)
	_edge_ttl.fill(0.0)
	_elim_t.resize(n)
	_elim_t.fill(-1.0)
	_was_dead.resize(n)
	_was_dead.fill(0)

# --- Per-player cosmetic assignment (engine-only, never feeds the sim) --------
# Player 0 is YOU (your live theme + selected skin). Players 1..N-1 simulate
# other lobby members: a deterministic, varied spread that cycles BOTH themes
# across the stable ArtTheme.PEER_SKIN_ROTATION so the grid shows a range of
# distinct looks — like 8 different people each picked their own cosmetics.
func _assign_cosmetics() -> void:
	players.clear()
	players.append({"theme": ArtTheme.active, "skin": Profile.selected})
	# Simulated peers, so unlock gating doesn't apply to the preview.
	var rotation: Array = ArtTheme.PEER_SKIN_ROTATION
	var theme_count: int = ArtTheme.themes.size()
	for i in range(1, N):
		# Alternate themes so BOTH always appear; offset from your theme so the
		# first peer already contrasts with your cell.
		var theme_idx: int = (ArtTheme.active + i) % theme_count
		var skin_id: String = rotation[(i - 1) % rotation.size()]
		players.append({"theme": theme_idx, "skin": skin_id})

# Cache BOTH theme texture sets up front (there are only 2), plus each player's
# tank keyed by player index. Loads strictly by explicit (theme, path) through
# the ArtTheme manifest; ArtTheme.active is never read for loading nor mutated.
func _cache_theme_textures() -> void:
	_theme_tex.clear()
	for t in ArtTheme.themes.size():
		_theme_tex.append({
			"ground": ArtTheme.tex_of(t, ArtTheme.ENV_MANIFEST["ground"]),
			"ring":   ArtTheme.tex_of(t, ArtTheme.ENV_MANIFEST["ring"]),
			"enemies": ArtTheme.enemy_textures_of(t),
			"minions": ArtTheme.minion_textures_of(t),
		})
	_player_tank.clear()
	for p in players:
		_player_tank.append(ArtTheme.tank_tex_for(p["theme"], p["skin"]))
	# Per-player HDR spark tint (theme accent boosted past 1.0 so sparks bloom),
	# precomputed here so the per-event hot path does zero theme-dict lookups.
	# Rebuilt with the caches on theme cycle.
	_cell_spark.resize(players.size())
	for i in players.size():
		var ac: Color = ArtTheme.ui_of(players[i]["theme"], "accent")
		_cell_spark[i] = Color(ac.r * 2.2, ac.g * 2.2, ac.b * 2.2)

# Cheap render-only glow parity with the single-arena view: a WorldEnvironment
# with bloom so emissive (>1.0) pixels — tanks, the spawn rings — bloom. No
# per-cell PointLight2D (too many cells); we lean on additive bloom instead.
func _setup_environment() -> void:
	var env := Environment.new()
	env.background_mode = Environment.BG_CANVAS
	env.glow_enabled = true
	env.glow_intensity = 0.7
	env.glow_strength = 1.0
	env.glow_bloom = 0.1
	env.glow_blend_mode = Environment.GLOW_BLEND_MODE_ADDITIVE
	env.glow_hdr_threshold = 1.05
	env.glow_hdr_scale = 2.0
	var we := WorldEnvironment.new()
	we.environment = env
	add_child(we)
	var cm := CanvasModulate.new()
	cm.color = Color(0.74, 0.76, 0.82)   # gentle cool dim; lets bloom read
	add_child(cm)

var _recorded := false        # match-end achievements credited once
var _toast: Array = []        # newly-unlocked achievement names to flash

func _unhandled_input(e: InputEvent) -> void:
	# Results-panel click targets (mirrors main.gd's _handle_click routing);
	# rects are only live while the panel is actually displayed.
	if e is InputEventMouseButton:
		if e.pressed and e.button_index == MOUSE_BUTTON_LEFT and _results_visible():
			if _rematch_rect.has_point(e.position):
				_rematch()
			elif _menu_rect.has_point(e.position):
				get_tree().change_scene_to_file("res://SkinSelect.tscn")
		return
	if not (e is InputEventKey or e is InputEventJoypadButton):
		return
	# [Enter/Space] on the results panel: rematch (gated on the panel being up,
	# so it can never fire mid-match).
	if _results_visible() and e.is_action_pressed(&"ui_confirm"):
		_rematch()
		return
	if e.is_action_pressed(&"ui_theme_cycle"):
		# Cycling YOUR theme re-assigns your cell (player 0) and refreshes the
		# peer spread + tank cache so the grid stays consistent.
		ArtTheme.cycle()
		_assign_cosmetics()
		_cache_theme_textures()
	elif e.is_action_pressed(&"ui_skins") or e.is_action_pressed(&"ui_back"):
		# Both S and Esc back out to the skin-select menu (no in-match quit).
		get_tree().change_scene_to_file("res://SkinSelect.tscn")

func _physics_process(_delta: float) -> void:
	if m == null:
		return
	m.step()
	# JUICE (render-only, one-way): drain each live cell's event stream, detect
	# death/round edges. Nothing below writes the sim.
	_drain_events()
	_detect_eliminations()
	_detect_round_pulse()
	# Credit "you" (player 0) once the match is decided. Cosmetic only.
	if not _recorded and m.match_over():
		_recorded = true
		var you: SimView = _views[0]
		var rec: Dictionary = you.stats_record()
		rec["won"] = you.placement() == 1
		for id in Profile.record_match(rec):
			_toast.append(Profile.ach_def(id).get("name", id))
		# AUDIO (render-only): voice the outcome for "you" once, reading the
		# authoritative placement. Strictly one-way — no sim write.
		Audio.play(&"victory" if rec["won"] else &"defeat")
		if rec["won"]:
			# Full-screen flash is reserved for your own death/victory; this is
			# the victory half (your death half lives in _detect_eliminations).
			fx.add_flash(Color(1.0, 0.9, 0.6, 0.35), 0.5)
		# Build the results-panel rows once, now that the director has finalized
		# every placement. The panel itself appears after RESULTS_DELAY.
		_build_results()

# Frame-rate cosmetic update: advance the FX bus, feed its shake into the
# camera offset (whole grid shakes as one — mirrors main.gd's camera pattern),
# and decay the per-cell timers. No allocation here.
func _process(delta: float) -> void:
	if m == null or fx == null:
		return
	fx.update(delta)
	if _camera:
		_camera.offset = fx.shake_offset()
	if _recorded:
		_over_t += delta   # render clock toward (and past) the results panel
	if _round_pulse > 0.0:
		_round_pulse = maxf(0.0, _round_pulse - delta)
	for i in _elim_t.size():
		if _cell_pulse[i] > 0.0:
			_cell_pulse[i] = maxf(0.0, _cell_pulse[i] - delta)
		if _cell_flash_ttl[i] > 0.0:
			_cell_flash_ttl[i] = maxf(0.0, _cell_flash_ttl[i] - delta)
		if _edge_ttl[i] > 0.0:
			_edge_ttl[i] = maxf(0.0, _edge_ttl[i] - delta)
		if _elim_t[i] >= 0.0:
			_elim_t[i] += delta
	queue_redraw()

# Drain StMatch.take_events(i) for every live cell (read-and-clear at the
# binding — exactly once per tick per cell) and fan out kill/hit/round/boss
# juice. Budget rule: YOUR cell (index 0) gets the full treatment + the only
# audio; peer cells get capped, cheaper sparks and stay silent (8 arenas of
# full audio would be mush). Caps are the consts at the top of the file.
func _drain_events() -> void:
	var vp: Vector2 = get_viewport_rect().size
	var n: int = m.player_count()
	var your_kills := 0
	for i in n:
		if i >= _views.size():
			break
		var view: SimView = _views[i]
		if view == null or not view.is_valid() or view.is_dead():
			continue
		var events: Array = view.take_events()
		if events.is_empty():
			continue
		var r := _cell_rect(i, n, vp)
		var center := r.position + Vector2(r.size.x * 0.5, r.size.y * 0.5 + 8.0)
		var scl: float = minf(r.size.x, r.size.y) * 0.42 / 1700.0
		var is_you: bool = i == 0
		var spark_col: Color = _cell_spark[i] if i < _cell_spark.size() else Color(2.2, 1.3, 0.6)
		var bursts := 0   # per-cell per-tick kill-burst budget
		for ev in events:
			match ev.kind:
				SimView.EV_ENEMY_KILLED:
					var p := center + Vector2(ev.x * scl, -ev.y * scl)
					if ev.boss:
						# Boss kill: bigger burst + cell flash (rare, uncapped).
						if is_you:
							fx.burst_sparks(p, Color(2.6, 1.8, 0.8), 26, 420.0, 0.55)
							fx.shockwave(p, Color(2.4, 1.6, 0.7, 0.9),
								minf(r.size.x, r.size.y) * 0.30, 0.5)
							fx.add_shake(0.3)
						else:
							fx.burst_sparks(p, spark_col, 12, 300.0, 0.45)
						_flash_cell(i, Color(1.0, 0.85, 0.5, 0.28), 0.3)
						_cell_pulse[i] = 0.22
					else:
						var cap := YOUR_KILL_FX_CAP if is_you else PEER_KILL_FX_CAP
						if bursts < cap:
							bursts += 1
							if is_you:
								fx.burst_sparks(p, Color(2.2, 1.3, 0.6), 8, 260.0, 0.35)
							else:
								fx.burst_sparks(p, spark_col,
									PEER_SPARKS_PER_KILL, 190.0, 0.28)
						_cell_pulse[i] = maxf(_cell_pulse[i], 0.16)
					if is_you:
						your_kills += 1   # counts ALL kills, not just drawn ones
				SimView.EV_TANK_HIT:
					# Your-cell feel only; peer tank hits render via their HP bar.
					if is_you:
						fx.add_shake(0.15)
						_edge_ttl[i] = 0.25
						Audio.play(&"tank_hit")
				SimView.EV_ROUND_START:
					if is_you:
						# Subtle ring pulse in your cell + the round SFX.
						fx.shockwave(center, Color(spark_col.r, spark_col.g, spark_col.b, 0.35),
							minf(r.size.x, r.size.y) * 0.38, 0.6)
						Audio.play(&"round_start")
				SimView.EV_BOSS_SPAWNED:
					if is_you:
						_flash_cell(i, Color(1.0, 0.3, 0.25, 0.30), 0.45)
						# boss_spawn is in Audio.DUCK_EVENTS, so this play() also
						# ducks the music bed — no separate duck() call needed.
						Audio.play(&"boss_spawn")
				_:
					# Impact/ProjectileSpawned etc.: deliberately NOT hooked
					# here (fire/hit audio+FX x8 arenas would be noise).
					pass
	# ONE aggregated, count-scaled kill voice per tick for YOUR cell.
	if your_kills > 0:
		Audio.play_many(&"enemy_death", your_kills)

# Death-edge detector: starts the elimination stamp animation the tick a cell's
# dead flag first reads true. Your own death gets the bigger treatment (the
# full-screen flash budget + heavy shake); at most ONE tank_destroyed voice per
# tick even if several arenas fall together. The existing victory/defeat flow
# at match_over stays untouched.
func _detect_eliminations() -> void:
	var sfx_done := false
	for i in _was_dead.size():
		var view: SimView = _views[i] if i < _views.size() else null
		var d: bool = view != null and view.is_valid() and view.is_dead()
		if d and _was_dead[i] == 0:
			_elim_t[i] = 0.0
			if i == 0:
				fx.add_flash(Color(1, 1, 1, 0.4), 0.3)
				fx.add_shake(0.6)
			if not sfx_done:
				sfx_done = true
				# Muted stinger (play() carries jitter; ducks via DUCK_EVENTS).
				Audio.play(&"tank_destroyed")
		_was_dead[i] = 1 if d else 0

# Global round counter edge -> brief header pulse so the match pacing reads.
func _detect_round_pulse() -> void:
	var rmax := 0
	for view in _views:
		if view != null and view.is_valid():
			rmax = maxi(rmax, view.round_num())
	if _last_round < 0:
		_last_round = rmax          # first tick: seed without pulsing
	elif rmax > _last_round:
		_last_round = rmax
		_round_pulse = 0.4

# Arm a short fill-flash over cell i (boss kill/spawn, tinted per event).
func _flash_cell(i: int, col: Color, life: float) -> void:
	if i >= _cell_flash_ttl.size():
		return
	_cell_flash_col[i] = col
	_cell_flash_ttl[i] = life
	_cell_flash_life[i] = life

func _blit(tx: Texture2D, center: Vector2, size: float, mod := Color.WHITE) -> void:
	draw_texture_rect(tx, Rect2(center - Vector2(size, size) * 0.5, Vector2(size, size)), false, mod)

func _draw() -> void:
	if m == null:
		return
	var vp: Vector2 = get_viewport_rect().size
	var font := ArtTheme.ui_font(false)   # Barlow + Noto SC fallback (renders CJK)
	var n: int = m.player_count()

	# header — pulses (brighter + slightly larger) for a beat when the global
	# round counter advances, so the match pacing reads at a glance.
	var status := tr("MATCH OVER") if m.match_over() else tr("LIVE")
	var hk := clampf(_round_pulse / 0.4, 0.0, 1.0)
	var hcol := Color(0.82, 0.88, 0.96).lerp(Color(1.35, 1.30, 1.05), hk)
	draw_string(font, Vector2(16, 26),
		tr("MULTI-ARENA NET VIEW  —  %d sharded sims · 1 authoritative director · server tick %d · alive %d/%d  [%s]")
		% [n, m.server_tick(), m.alive_count(), n, status],
		HORIZONTAL_ALIGNMENT_LEFT, -1, 16 + int(3.0 * hk), hcol)
	var you := tr("YOU: %s  ·  [S] skins") % tr(Profile.skin_def(Profile.selected).name)
	if Profile.active_challenge_code != 0:
		for c in Profile.CHALLENGES:
			if c.code == Profile.active_challenge_code:
				you = tr("CHALLENGE: %s  ·  %s") % [tr(c.name), you]
				break
	# Right-align by measuring the rendered width (length()*px breaks with CJK),
	# clamped so an over-long line still starts on screen.
	var you_w: float = font.get_string_size(you, HORIZONTAL_ALIGNMENT_LEFT, -1, 13).x
	draw_string(font, Vector2(maxf(vp.x - 16.0 - you_w, 20.0), 26),
		you, HORIZONTAL_ALIGNMENT_LEFT, -1, 13, Color(0.72, 0.66, 0.5))
	# Featured layout via _cell_rect (shared with the event drain): player 0
	# (YOU) gets a large panel on the left taking ~60% of the width and the full
	# height under the header; the other n-1 peers wrap into a tidy 2-column
	# grid filling the remaining ~40% on the right.
	for i in n:
		_draw_cell(font, i, _cell_rect(i, n, vp), i == 0)

	# The shared FX bus draws OVER the whole grid (emitter positions were
	# computed in screen space at drain time), then the full-screen flash —
	# reserved for your own death/victory — on top of everything.
	fx.draw(self, font)
	var fc := fx.flash_color()
	if fc.a > 0.001:
		draw_rect(Rect2(Vector2.ZERO, vp), fc)

	# End-of-match results panel over the dimmed final grid (the sims stay
	# rendered beneath; the match_over gate lives in _results_visible).
	if _results_visible():
		_draw_results(font, vp)

	# Achievement toasts draw LAST so they stay visible above the results panel.
	if not _toast.is_empty():
		# Each toast entry is an achievement name (in the translation table).
		var toast_names: Array = []
		for t in _toast:
			toast_names.append(tr(t))
		draw_string(font, Vector2(16, vp.y - 16),
			tr("ACHIEVEMENT UNLOCKED:  %s") % ", ".join(toast_names),
			HORIZONTAL_ALIGNMENT_LEFT, -1, 18, Color(1.0, 0.82, 0.4))

# The one source of truth for cell geometry, used by BOTH _draw and the event
# drain so juice lands exactly where the cell renders. Pure function of
# (index, player count, viewport); allocates nothing but the returned Rect2.
func _cell_rect(i: int, n: int, vp: Vector2) -> Rect2:
	var pad := 8.0
	var top := 40.0
	var avail_w := vp.x - pad * 3.0          # outer-left, center gutter, outer-right
	var avail_h := vp.y - top - pad * 2.0
	var big_w := avail_w * 0.60
	var x0 := pad
	var y0 := top + pad
	if i == 0:
		# YOU — one tall featured panel on the left.
		return Rect2(x0, y0, big_w, avail_h)
	# Peers — a 2-column grid on the right, rows sized to fit n-1 cells.
	var peers: int = n - 1
	var pcols: int = 2 if peers > 1 else 1
	var prows: int = int(ceil(float(peers) / pcols))
	var k: int = i - 1
	var col: int = k % pcols
	@warning_ignore("integer_division")
	var row: int = k / pcols
	var pcw := (avail_w - big_w - pad * (pcols - 1)) / pcols
	var pch := (avail_h - pad * (prows - 1)) / prows
	return Rect2(x0 + big_w + pad + col * (pcw + pad), y0 + row * (pch + pad), pcw, pch)

func _draw_cell(font, i: int, r: Rect2, is_big: bool = false) -> void:
	var view: SimView = _views[i] if i < _views.size() else null
	if view == null or not view.is_valid():
		return
	var dead: bool = view.is_dead()
	var center := r.position + Vector2(r.size.x * 0.5, r.size.y * 0.5 + 8.0)
	var scl: float = minf(r.size.x, r.size.y) * 0.42 / 1700.0

	# This player's chosen cosmetics drive every texture AND its UI chrome: each
	# cell paints in its own player's theme palette via ArtTheme.ui_of(theme,...).
	var pc: Dictionary = players[i] if i < players.size() else {"theme": 0, "skin": "ol_reliable"}
	var theme_idx: int = pc["theme"]
	var tset: Dictionary = _theme_tex[theme_idx]
	var is_you: bool = i == 0

	# Per-cell palette (this player's theme).
	var c_accent: Color = ArtTheme.ui_of(theme_idx, "accent")
	var c_text: Color = ArtTheme.ui_of(theme_idx, "text")
	var c_dim: Color = ArtTheme.ui_of(theme_idx, "text_dim")
	var c_header: Color = ArtTheme.ui_of(theme_idx, "header")
	var c_hp: Color = ArtTheme.ui_of(theme_idx, "hp")
	var c_danger: Color = ArtTheme.ui_of(theme_idx, "danger")
	var c_coin: Color = ArtTheme.ui_of(theme_idx, "coin")
	var c_border: Color = ArtTheme.ui_of(theme_idx, "panel_border")
	# Dead cells desaturate/darken the HP color so a destroyed bar reads as gone.
	var hp_fill: Color = Color(0.5, 0.5, 0.55) if dead else c_hp
	# UI scale: the big featured cell gets larger type/markers; peers stay legible.
	var us := 1.55 if is_big else 1.0

	# arena floor + spawn ring (this player's theme, clipped to cell)
	draw_texture_rect(tset["ground"], r, false)
	_blit(tset["ring"], center, 2.0 * 1500.0 * scl / 0.90)

	# enemies (small) — this player's theme art, net-view sizes from the manifest
	var theme_enemies: Array = tset["enemies"]
	var ep: PackedVector2Array = view.enemies_pos()
	var ek: PackedByteArray = view.enemies_kind()
	for j in ep.size():
		var kind: int = ek[j] if j < ek.size() else 0
		var sz := ArtTheme.enemy_draw_size(kind, true)
		var tx: Texture2D = theme_enemies[kind] if kind < theme_enemies.size() else null
		if tx:
			_blit(tx, center + Vector2(ep[j].x * scl, -ep[j].y * scl), sz)

	# summoned allies — this player's theme art
	var theme_minions: Array = tset["minions"]
	var mp: PackedVector2Array = view.minions_pos()
	var mk: PackedByteArray = view.minions_kind()
	for j in mp.size():
		var mkind: int = mk[j] if j < mk.size() else 0
		var mtx: Texture2D = theme_minions[mkind] if mkind < theme_minions.size() else null
		if mtx:
			_blit(mtx, center + Vector2(mp[j].x * scl, -mp[j].y * scl),
				ArtTheme.minion_draw_size(mkind, true))

	# tank — this player's (theme, skin) texture. Living tanks get a faint
	# emissive lift so they bloom under glow, echoing the single-arena tank light.
	var tank_tx: Texture2D = _player_tank[i] if i < _player_tank.size() else null
	if tank_tx:
		_blit(tank_tx, center, ArtTheme.tank_draw_size(true),
			Color(1, 1, 1, 0.5) if dead else Color(1.18, 1.22, 1.35))

	# HP bar — fill in this player's theme HP color (dead → desaturated).
	var hp := maxi(view.tank_hp(), 0)
	var maxhp := maxi(view.tank_max_hp(), 1)
	var bh := 7.0 * us
	var bw := r.size.x - 16.0
	draw_rect(Rect2(r.position + Vector2(8, 8), Vector2(bw, bh)), Color(0, 0, 0, 0.55))
	draw_rect(Rect2(r.position + Vector2(8, 8), Vector2(bw * float(hp) / float(maxhp), bh)), hp_fill)

	# label + economy (round / gold / weapon count preserved). P# + skin name read
	# in theme text; your own cell keeps the accent so it pops.
	var gold: int = view.gold()
	var rnd: int = view.round_num()
	var top_y := r.position.y + 26.0 + bh
	var pcol := c_accent if is_you else c_text
	draw_string(font, Vector2(r.position.x + 10, top_y), "P%d" % (i + 1),
		HORIZONTAL_ALIGNMENT_LEFT, -1, int(15 * us), pcol)
	# round/gold/weapon metadata: gold figure in the theme coin color, rest dim.
	var meta_x := r.position.x + 10.0 + (44.0 * us)
	draw_string(font, Vector2(meta_x, top_y), "R%d · " % rnd,
		HORIZONTAL_ALIGNMENT_LEFT, -1, int(13 * us), c_dim)
	var rw: float = font.get_string_size("R%d · " % rnd, HORIZONTAL_ALIGNMENT_LEFT, -1, int(13 * us)).x
	draw_string(font, Vector2(meta_x + rw, top_y), "%dg" % gold,
		HORIZONTAL_ALIGNMENT_LEFT, -1, int(13 * us), c_coin)
	var gw: float = font.get_string_size("%dg" % gold, HORIZONTAL_ALIGNMENT_LEFT, -1, int(13 * us)).x
	draw_string(font, Vector2(meta_x + rw + gw, top_y), " · %dw" % view.weapon_count(),
		HORIZONTAL_ALIGNMENT_LEFT, -1, int(13 * us), c_dim)
	# per-player damage score (log-compressed so it never runs into the thousands)
	var score_txt := tr("SCORE %d") % _score(i)
	var sw: float = font.get_string_size(score_txt, HORIZONTAL_ALIGNMENT_LEFT, -1, int(14 * us)).x
	draw_string(font, Vector2(r.position.x + r.size.x - sw - 8.0, top_y), score_txt,
		HORIZONTAL_ALIGNMENT_LEFT, -1, int(14 * us), c_accent)

	# net legibility: who picked what. Skin name line in theme text; your own cell
	# keeps the accent and the ★ YOU mark so the grid reads like a lobby of choices.
	var skin_name: String = tr(Profile.skin_def(pc["skin"]).name)
	var name_sz := int(13 * us)
	var who := ("P%d · %s" % [i + 1, skin_name]) + ("  " + tr("★ YOU") if is_you else "")
	draw_string(font, Vector2(r.position.x + 10, r.position.y + r.size.y - 12), who,
		HORIZONTAL_ALIGNMENT_LEFT, -1, name_sz, c_accent if is_you else c_text)
	# theme pill, bottom-right — the pill itself reads as THAT theme's color: fill
	# from accent (dimmed), border + text from the theme's header/accent.
	var tag := tr(ArtTheme.theme_tag(theme_idx))
	var pill_fs := int(11 * us)
	var tag_w: float = font.get_string_size(tag, HORIZONTAL_ALIGNMENT_LEFT, -1, pill_fs).x + 14.0
	var pill_h := 18.0 * us
	var pill := Rect2(r.position.x + r.size.x - tag_w - 8.0, r.position.y + r.size.y - pill_h - 8.0, tag_w, pill_h)
	draw_rect(pill, Color(c_accent.r, c_accent.g, c_accent.b, 0.22))
	draw_rect(pill, c_header, false, 1.0)
	draw_string(font, Vector2(pill.position.x + 7.0, pill.position.y + pill_h - 5.0), tag,
		HORIZONTAL_ALIGNMENT_LEFT, -1, pill_fs, c_header)

	# cell fill flash (boss kill / boss spawn) — brief tinted wash over the cell.
	if i < _cell_flash_ttl.size() and _cell_flash_ttl[i] > 0.0:
		var fcol := _cell_flash_col[i]
		var fk := _cell_flash_ttl[i] / maxf(_cell_flash_life[i], 0.0001)
		draw_rect(r, Color(fcol.r, fcol.g, fcol.b, fcol.a * fk))

	# cell border — your cell gets a brighter, thicker accent + glow-y double frame
	# in YOUR theme's accent; peers get their own theme's panel_border.
	if is_you:
		draw_rect(r, c_accent, false, 4.0)
		draw_rect(Rect2(r.position + Vector2(3, 3), r.size - Vector2(6, 6)),
			Color(c_accent.r, c_accent.g, c_accent.b, 0.35), false, 1.5)
	else:
		draw_rect(r, c_border, false, 2.0)

	# kill-feedback frame pulse: the cell frame briefly brightens (HDR accent so
	# it blooms) when this arena scores kills. One rect, decays in _process.
	if i < _cell_pulse.size() and _cell_pulse[i] > 0.0:
		var pk := _cell_pulse[i] / 0.22
		draw_rect(r, Color(c_accent.r * 1.6, c_accent.g * 1.6, c_accent.b * 1.6, 0.8 * pk),
			false, 2.0 + 2.0 * pk)

	# your-cell tank-hit feedback: red edge flash (fill stays clear so the fight
	# under it remains readable; the shake already sells the impact).
	if i < _edge_ttl.size() and _edge_ttl[i] > 0.0:
		var ek := _edge_ttl[i] / 0.25
		draw_rect(Rect2(r.position + Vector2(1.5, 1.5), r.size - Vector2(3, 3)),
			Color(1.8, 0.25, 0.2, 0.85 * ek), false, 3.0 + 3.0 * ek)

	# elimination stamp — replaces the old instant dead overlay with a short
	# animation: white pop -> desaturate over ELIM_DESAT_T -> placement stamp
	# scaling in (overshoot then settle). Driven by _elim_t (render clock); a
	# cell dead before the edge detector saw it (et<0) renders fully settled.
	if dead:
		var et: float = _elim_t[i] if i < _elim_t.size() and _elim_t[i] >= 0.0 else 10.0
		if et < ELIM_FLASH_T:
			draw_rect(r, Color(1, 1, 1, 0.85 * (1.0 - et / ELIM_FLASH_T)))
		var dk := clampf(et / ELIM_DESAT_T, 0.0, 1.0)
		draw_rect(r, Color(0, 0, 0, 0.5 * dk))
		var st := clampf((et - ELIM_STAMP_DELAY) / ELIM_STAMP_T, 0.0, 1.0)
		if st > 0.0:
			var place: int = view.placement()
			var txt := tr("OUT") if place == 0 else "#%d" % place
			# Overshoot-then-settle: scale eases 2.2 -> 1.0 through easeOutBack,
			# which dips just under 1.0 before landing — the "stamp" feel.
			var dead_fs := int(26 * us * lerpf(2.2, 1.0, _ease_out_back(st)))
			var da := clampf(st * 3.0, 0.0, 1.0)
			var dw: float = font.get_string_size(txt, HORIZONTAL_ALIGNMENT_LEFT, -1, dead_fs).x
			draw_string(font, center - Vector2(dw * 0.5, dead_fs * 0.25), txt,
				HORIZONTAL_ALIGNMENT_LEFT, -1, dead_fs,
				Color(c_danger.r, c_danger.g, c_danger.b, da))

# Standard easeOutBack (overshoots slightly past 1.0 near the end, then
# settles) — drives the elimination stamp's scale-in.
func _ease_out_back(t: float) -> float:
	var c1 := 1.70158
	var c3 := c1 + 1.0
	var u := t - 1.0
	return 1.0 + c3 * u * u * u + c1 * u * u

# Compressed damage score for the per-player tracker: log-scaled so it climbs
# steadily but never runs into the thousands (raw damage reaches the millions).
func _score(i: int) -> int:
	var dmg: float = float(_views[i].damage_dealt()) if i < _views.size() else 0.0
	if dmg < 1.0:
		return 0
	return int(round(30.0 * log(1.0 + dmg) / log(10.0)))

# --- P5 results panel (docs/09 §9.4-P5 item 5) ---------------------------------
# The panel is gated on match_over (it can never appear mid-match): _recorded
# only flips inside the `m.match_over()` branch, and _over_t only advances after.
func _results_visible() -> bool:
	return _recorded and _over_t >= RESULTS_DELAY

# Build every string the results panel draws, ONCE, at match_over — the director
# has finalized all placements by then. WIN/LOSS uses the results semantics from
# net::results::MatchStats: win = top half of the lobby, ceil cutoff (1st..4th
# of 8 WIN, 5th..8th LOSS) — placement itself stays the director's 1-based rank.
func _build_results() -> void:
	var n: int = m.player_count()
	@warning_ignore("integer_division")
	var win_cutoff: int = (n + 1) / 2
	_rows.clear()
	for i in n:
		var view: SimView = _views[i] if i < _views.size() else null
		var place: int = view.placement() if view != null and view.is_valid() else 0
		var pc: Dictionary = players[i] if i < players.size() else {"skin": "ol_reliable"}
		var nm: String = "P%d · %s" % [i + 1, tr(Profile.skin_def(pc["skin"]).name)]
		if i == 0:
			nm += "  " + tr("★ YOU")
		_rows.append({
			"place_txt": ("#%d" % place) if place > 0 else "—",
			"sort": place if place > 0 else 99,   # unresolved sorts last (shouldn't happen)
			"name": nm,
			"is_you": i == 0,
			"won": place > 0 and place <= win_cutoff,
			"result_txt": tr("WIN") if place > 0 and place <= win_cutoff else tr("LOSS"),
			"score_txt": "%d" % _score(i),
			"weapons_txt": "%d" % (view.weapon_count() if view != null else 0),
		})
	_rows.sort_custom(func(a, b): return a["sort"] < b["sort"])
	_res_sub = tr("You placed #%d of %d") % [_views[0].placement(), n]
	_res_menu_label = tr("MENU")
	if _from_lobby:
		_res_btn_label = tr("BACK TO LOBBY")
		_res_hint = tr("[Enter] Back to Lobby   ·   [Esc / S] Menu")
	else:
		_res_btn_label = tr("REMATCH")
		_res_hint = tr("[Enter] Rematch   ·   [Esc / S] Menu")

# Rematch semantics: a lobby-launched match goes BACK TO THE LOBBY — lobby.gd
# reads nothing from Session on _ready and the Session plan is one-shot, so a
# re-plan forged here would bypass StLobby's host-authoritative try_start;
# "play again" is re-ready + start. A direct demo/challenge launch reloads
# Match.tscn: Session is clear (fresh demo seed via randi()) and the active
# challenge persists in the Profile autoload, so it stays applied.
func _rematch() -> void:
	if _from_lobby:
		get_tree().change_scene_to_file("res://Lobby.tscn")
	else:
		get_tree().reload_current_scene()

# Centered end-of-match summary over the dimmed final grid: every player by
# final placement, WIN/LOSS per the top-half rule, score + weapons bought, and
# the Rematch/Menu actions (keyboard + click). Visual language mirrors
# results.gd (panel_bg body, emissive top rule, darkened-accent button). Draws
# ONLY strings cached by _build_results — no per-frame formatting.
func _draw_results(font: Font, vp: Vector2) -> void:
	var head: Font = ArtTheme.ui_font(true)   # cached per weight in ArtTheme
	var k := clampf((_over_t - RESULTS_DELAY) / RESULTS_FADE, 0.0, 1.0)
	# Dim the finished arenas behind the panel (they keep rendering beneath).
	draw_rect(Rect2(Vector2.ZERO, vp), Color(0.02, 0.02, 0.04, 0.62 * k))

	var n := _rows.size()
	var row_h := 26.0
	var pw := 640.0
	var ph := 226.0 + row_h * float(n)
	var px := vp.x * 0.5 - pw * 0.5
	var py := vp.y * 0.5 - ph * 0.5
	var panel := Rect2(Vector2(px, py), Vector2(pw, ph))
	draw_rect(panel, ArtTheme.ui("panel_bg"))
	draw_rect(panel, ArtTheme.ui("panel_border"), false, 2.0)
	# Emissive top rule so it blooms under glow (accent: outcome-neutral header).
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, 3)), ArtTheme.ui("accent") * 1.4)

	var cx := vp.x * 0.5
	var title := tr("MATCH RESULTS")
	var tw := head.get_string_size(title, HORIZONTAL_ALIGNMENT_LEFT, -1, 32).x
	draw_string(head, Vector2(cx - tw * 0.5, py + 46), title,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 32, ArtTheme.ui("accent") * 1.3)
	var subw := font.get_string_size(_res_sub, HORIZONTAL_ALIGNMENT_LEFT, -1, 15).x
	draw_string(font, Vector2(cx - subw * 0.5, py + 70), _res_sub,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 15, ArtTheme.ui("text_dim"))

	# Column header row (numeric columns right-aligned by measured width).
	var lx := px + 30.0
	var name_x := lx + 52.0
	var res_x := px + pw - 236.0
	var score_rx := px + pw - 128.0
	var wpn_rx := px + pw - 34.0
	var hy := py + 100.0
	var c_head: Color = ArtTheme.ui("header")
	draw_string(head, Vector2(lx, hy), "#", HORIZONTAL_ALIGNMENT_LEFT, -1, 12, c_head)
	draw_string(head, Vector2(name_x, hy), tr("PLAYER"), HORIZONTAL_ALIGNMENT_LEFT, -1, 12, c_head)
	draw_string(head, Vector2(res_x, hy), tr("RESULT"), HORIZONTAL_ALIGNMENT_LEFT, -1, 12, c_head)
	var sco_h := tr("SCORE")
	draw_string(head, Vector2(score_rx - head.get_string_size(sco_h, HORIZONTAL_ALIGNMENT_LEFT, -1, 12).x, hy),
		sco_h, HORIZONTAL_ALIGNMENT_LEFT, -1, 12, c_head)
	var wpn_h := tr("WEAPONS")
	draw_string(head, Vector2(wpn_rx - head.get_string_size(wpn_h, HORIZONTAL_ALIGNMENT_LEFT, -1, 12).x, hy),
		wpn_h, HORIZONTAL_ALIGNMENT_LEFT, -1, 12, c_head)

	# Placement rows (already sorted best -> worst by _build_results).
	var c_text: Color = ArtTheme.ui("text")
	var c_dim: Color = ArtTheme.ui("text_dim")
	var c_accent: Color = ArtTheme.ui("accent")
	var c_win := Color(0.42, 0.85, 0.55)          # lobby's ready-green
	var c_loss: Color = ArtTheme.ui("danger")
	c_loss.a = 0.85
	for j in n:
		var row: Dictionary = _rows[j]
		var y := hy + 26.0 + row_h * float(j)
		if row["is_you"]:
			# Subtle accent wash so YOUR row pops (matches your-cell highlight).
			draw_rect(Rect2(px + 8.0, y - 17.0, pw - 16.0, row_h - 2.0),
				Color(c_accent.r, c_accent.g, c_accent.b, 0.12))
		var rc: Color = c_accent if row["is_you"] else c_text
		draw_string(head, Vector2(lx, y), row["place_txt"], HORIZONTAL_ALIGNMENT_LEFT, -1, 15, rc)
		draw_string(font, Vector2(name_x, y), row["name"], HORIZONTAL_ALIGNMENT_LEFT, -1, 15, rc)
		draw_string(head, Vector2(res_x, y), row["result_txt"],
			HORIZONTAL_ALIGNMENT_LEFT, -1, 14, c_win if row["won"] else c_loss)
		var scw := font.get_string_size(row["score_txt"], HORIZONTAL_ALIGNMENT_LEFT, -1, 15).x
		draw_string(font, Vector2(score_rx - scw, y), row["score_txt"],
			HORIZONTAL_ALIGNMENT_LEFT, -1, 15, c_accent if row["is_you"] else c_dim)
		var ww := font.get_string_size(row["weapons_txt"], HORIZONTAL_ALIGNMENT_LEFT, -1, 15).x
		draw_string(font, Vector2(wpn_rx - ww, y), row["weapons_txt"],
			HORIZONTAL_ALIGNMENT_LEFT, -1, 15, c_dim)

	# Actions: primary Rematch/Back-to-Lobby + secondary Menu, hover-lit click
	# targets (results.gd's button pattern) with the keyboard hint beneath.
	var btn_h := 40.0
	var by := py + ph - 66.0
	var bw1 := 250.0
	var bw2 := 130.0
	var gap := 16.0
	_rematch_rect = Rect2(cx - (bw1 + gap + bw2) * 0.5, by, bw1, btn_h)
	_menu_rect = Rect2(_rematch_rect.position.x + bw1 + gap, by, bw2, btn_h)
	var mpos := get_viewport().get_mouse_position()
	var btn_base: Color = c_accent.darkened(0.7)
	draw_rect(_rematch_rect, btn_base.lightened(0.08) if _rematch_rect.has_point(mpos) else btn_base)
	var btn_border := c_accent
	btn_border.a = 0.7
	draw_rect(_rematch_rect, btn_border, false, 1.5)
	var blw := head.get_string_size(_res_btn_label, HORIZONTAL_ALIGNMENT_LEFT, -1, 17).x
	draw_string(head, _rematch_rect.position + Vector2(bw1 * 0.5 - blw * 0.5, 26),
		_res_btn_label, HORIZONTAL_ALIGNMENT_LEFT, -1, 17, c_accent.lightened(0.3))
	var menu_base := Color(0.14, 0.14, 0.18)
	draw_rect(_menu_rect, menu_base.lightened(0.06) if _menu_rect.has_point(mpos) else menu_base)
	draw_rect(_menu_rect, ArtTheme.ui("panel_border"), false, 1.5)
	var mlw := head.get_string_size(_res_menu_label, HORIZONTAL_ALIGNMENT_LEFT, -1, 16).x
	draw_string(head, _menu_rect.position + Vector2(bw2 * 0.5 - mlw * 0.5, 26),
		_res_menu_label, HORIZONTAL_ALIGNMENT_LEFT, -1, 16, c_text)
	var hintw := font.get_string_size(_res_hint, HORIZONTAL_ALIGNMENT_LEFT, -1, 14).x
	draw_string(font, Vector2(cx - hintw * 0.5, py + ph - 12.0), _res_hint,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 14, c_dim)
