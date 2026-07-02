# Multi-arena / net view. Runs the REAL netcode loop in `StMatch` (authoritative
# director + N clients + hub, bot-driven) and renders every player's
# authoritative shadow arena as a grid — the visual proof of the sharded-sim
# architecture: N independent arenas under one director, no entity replication.
extends Node2D

const N := 8                  # players in the demo match

# Relative paths under a theme folder (the locked art filename contract), in
# enemy-kind order for the cached enemy set and minion-kind order for minions.
const ENEMY_REL := [
	"enemies/squeakzilla_rat.svg", "enemies/fanged_death.svg",
	"enemies/boss_hippo.svg", "enemies/doomduck.svg",
	"enemies/bacon_warthog.svg", "enemies/bandit_rider.svg",
	"enemies/bonk_golem.svg", "enemies/noperope_cobra.svg",
	"enemies/poisonspitter.svg", "enemies/firebreather.svg",
	"enemies/icebreather.svg", "enemies/target_dummy.svg",
]
const MINION_REL := ["minions/larvae.svg", "minions/spores.svg"]

var m

# Per-player cosmetic assignment, built once in _ready (stable across frames).
# players[i] = {"theme": <idx into ArtTheme.themes>, "skin": <skin id string>}.
var players: Array = []

# Per-theme texture cache, keyed by theme index. Each entry:
#   {"ground": Texture2D, "ring": Texture2D, "enemies": [Texture2D...], "minions": [...]}
var _theme_tex: Array = []
# Per-player tank texture, keyed by player index (resolved from theme+skin).
var _player_tank: Array = []

func _ready() -> void:
	randomize()
	# If we arrived from the lobby, honor its host-authoritative plan (host + peers,
	# planned seed) instead of the standalone demo's defaults; then clear the
	# hand-off so a later direct launch falls back to the demo behavior.
	if Session.lobby_players > 0:
		m = StMatch.new_match(Session.lobby_players + 1, Session.lobby_seed)
		Session.clear()
	else:
		m = StMatch.new_match(N, randi())
	# You are player 0; honor a chosen challenge so its achievement is earnable.
	if Profile.active_challenge_code != 0:
		m.set_challenge(0, Profile.active_challenge_code)
	_assign_cosmetics()
	_cache_theme_textures()
	_setup_environment()
	Audio.set_music("ambient_bed.wav")   # render-only ambient bed

# --- Per-player cosmetic assignment (engine-only, never feeds the sim) --------
# Player 0 is YOU (your live theme + selected skin). Players 1..N-1 simulate
# other lobby members: a deterministic, varied spread that cycles BOTH themes
# across a stable rotation of skin ids so the grid shows a range of distinct
# looks — like 8 different people each picked their own cosmetics.
func _assign_cosmetics() -> void:
	players.clear()
	players.append({"theme": ArtTheme.active, "skin": Profile.selected})
	# A fixed, hand-picked rotation of skin ids (stable order, visibly varied).
	# These are simulated peers, so unlock gating doesn't apply to the preview.
	var rotation := [
		"deadeye", "spicy_meatball", "octo_blaster", "disco_doom",
		"tidal_terry", "bouncy_boi", "stone_broke", "franken_tank",
		"gore_hound", "chilly_willy", "sir_toots", "lord_spookington",
	]
	var theme_count: int = ArtTheme.themes.size()
	for i in range(1, N):
		# Alternate themes so BOTH always appear; offset from your theme so the
		# first peer already contrasts with your cell.
		var theme_idx: int = (ArtTheme.active + i) % theme_count
		var skin_id: String = rotation[(i - 1) % rotation.size()]
		players.append({"theme": theme_idx, "skin": skin_id})

# --- Theme path helpers (mirror ArtTheme WITHOUT mutating its global state) ---
func _theme_base(theme_idx: int) -> String:
	return "res://art/themes/%s/" % ArtTheme.themes[theme_idx]

func _theme_load(theme_idx: int, rel: String) -> Texture2D:
	return load(_theme_base(theme_idx) + rel)

# Resolve a tank texture for (theme, skin) by the same rule as ArtTheme.tank_tex:
# tank/skins/<file> if it exists in THAT theme, else that theme's player_tank.svg.
func _tank_for(theme_idx: int, skin_id: String) -> Texture2D:
	var f: String = Profile.skin_def(skin_id).file
	if f != "":
		var p := _theme_base(theme_idx) + "tank/" + f
		if ResourceLoader.exists(p):
			return load(p)
	return _theme_load(theme_idx, "tank/player_tank.svg")

# Cache BOTH theme texture sets up front (there are only 2), plus each player's
# tank keyed by player index. Loads strictly by explicit path; ArtTheme.active
# is never read for loading nor mutated here.
func _cache_theme_textures() -> void:
	_theme_tex.clear()
	for t in ArtTheme.themes.size():
		var enemies: Array = []
		for rel in ENEMY_REL:
			enemies.append(_theme_load(t, rel))
		var minions: Array = []
		for rel in MINION_REL:
			minions.append(_theme_load(t, rel))
		_theme_tex.append({
			"ground": _theme_load(t, "env/arena_ground.svg"),
			"ring":   _theme_load(t, "env/spawn_ring.svg"),
			"enemies": enemies,
			"minions": minions,
		})
	_player_tank.clear()
	for p in players:
		_player_tank.append(_tank_for(p["theme"], p["skin"]))

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

func _unhandled_key_input(e: InputEvent) -> void:
	if e is InputEventKey and e.pressed and not e.echo:
		if e.keycode == KEY_T:
			# Cycling YOUR theme re-assigns your cell (player 0) and refreshes the
			# peer spread + tank cache so the grid stays consistent.
			ArtTheme.cycle(); _assign_cosmetics(); _cache_theme_textures()
		elif e.keycode == KEY_S or e.keycode == KEY_ESCAPE:
			# Both S and Esc back out to the skin-select menu (no in-match quit).
			get_tree().change_scene_to_file("res://SkinSelect.tscn")

func _physics_process(_delta: float) -> void:
	if m == null:
		return
	m.step()
	# Credit "you" (player 0) once the match is decided. Cosmetic only.
	if not _recorded and m.match_over():
		_recorded = true
		var st: PackedInt64Array = m.stats(0)
		var arena: PackedInt64Array = m.arena(0)
		var rec := {
			"damage": st[0] if st.size() > 0 else 0,
			"gold": st[1] if st.size() > 1 else 0,
			"round": arena[5] if arena.size() > 5 else 0,
			"won": m.placement(0) == 1,
			"attack_mask": st[2] if st.size() > 2 else 0,
			"weapons_bought": st[3] if st.size() > 3 else 0,
			"economy_buys": st[4] if st.size() > 4 else 0,
		}
		for id in Profile.record_match(rec):
			_toast.append(Profile.ach_def(id).get("name", id))
		# AUDIO (render-only): voice the outcome for "you" once, reading the
		# authoritative placement. Strictly one-way — no sim write.
		Audio.play(&"victory" if rec["won"] else &"defeat")
	queue_redraw()

func _blit(tx: Texture2D, center: Vector2, size: float, mod := Color.WHITE) -> void:
	draw_texture_rect(tx, Rect2(center - Vector2(size, size) * 0.5, Vector2(size, size)), false, mod)

func _draw() -> void:
	if m == null:
		return
	var vp: Vector2 = get_viewport_rect().size
	var font := ArtTheme.ui_font(false)   # Barlow + Noto SC fallback (renders CJK)
	var n: int = m.player_count()

	# header
	var status := tr("MATCH OVER") if m.match_over() else tr("LIVE")
	draw_string(font, Vector2(16, 26),
		tr("MULTI-ARENA NET VIEW  —  %d sharded sims · 1 authoritative director · server tick %d · alive %d/%d  [%s]")
		% [n, m.server_tick(), m.alive_count(), n, status],
		HORIZONTAL_ALIGNMENT_LEFT, -1, 16, Color(0.82, 0.88, 0.96))
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
	if not _toast.is_empty():
		# Each toast entry is an achievement name (in the translation table).
		var toast_names: Array = []
		for t in _toast:
			toast_names.append(tr(t))
		draw_string(font, Vector2(16, vp.y - 16),
			tr("ACHIEVEMENT UNLOCKED:  %s") % ", ".join(toast_names),
			HORIZONTAL_ALIGNMENT_LEFT, -1, 18, Color(1.0, 0.82, 0.4))

	# Featured layout: player 0 (YOU) gets a large panel on the left taking ~60%
	# of the width and the full height under the header; the other n-1 peers wrap
	# into a tidy 2-column grid filling the remaining ~40% on the right. The big
	# panel is far more than 2x the linear size of a peer cell, so it reads as the
	# clear focal point while all arenas stay visible without overlap.
	var pad := 8.0
	var top := 40.0
	var avail_w := vp.x - pad * 3.0          # outer-left, center gutter, outer-right
	var avail_h := vp.y - top - pad * 2.0
	var big_w := avail_w * 0.60
	var peers_w := avail_w - big_w
	var x0 := pad
	var y0 := top + pad

	# YOU — one tall featured panel on the left.
	var big := Rect2(x0, y0, big_w, avail_h)
	_draw_cell(font, 0, big, true)

	# Peers — a 2-column grid on the right, rows sized to fit n-1 cells.
	var peers: int = n - 1
	if peers > 0:
		var pcols: int = 2 if peers > 1 else 1
		var prows: int = int(ceil(float(peers) / pcols))
		var px0 := x0 + big_w + pad
		var pcw := (peers_w - pad * (pcols - 1)) / pcols
		var pch := (avail_h - pad * (prows - 1)) / prows
		for k in peers:
			var i: int = k + 1
			var col: int = k % pcols
			var row: int = k / pcols
			var rect := Rect2(px0 + col * (pcw + pad), y0 + row * (pch + pad), pcw, pch)
			_draw_cell(font, i, rect, false)

# A short uppercase tag for a theme index, for the per-cell theme pill.
func _theme_tag(theme_idx: int) -> String:
	match ArtTheme.themes[theme_idx]:
		"grimdark":         return "GRIMDARK"
		"gaslamp_bulwark":  return "GASLAMP"
	return ArtTheme.themes[theme_idx].to_upper()

func _draw_cell(font, i: int, r: Rect2, is_big: bool = false) -> void:
	var arena: PackedInt64Array = m.arena(i)   # [x,y,hp,maxhp,rev,round,tick,dead]
	if arena.size() < 8:
		return
	var dead: bool = arena[7] != 0
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

	# enemies (small) — this player's theme art
	var theme_enemies: Array = tset["enemies"]
	var ep: PackedVector2Array = m.enemies_pos(i)
	var ek: PackedByteArray = m.enemies_kind(i)
	for j in ep.size():
		var kind: int = ek[j] if j < ek.size() else 0
		var sz := 56.0 if kind == 2 else (30.0 if kind == 6 else 20.0)
		var tx: Texture2D = theme_enemies[kind] if kind < theme_enemies.size() else null
		if tx:
			_blit(tx, center + Vector2(ep[j].x * scl, -ep[j].y * scl), sz)

	# summoned allies — this player's theme art
	var theme_minions: Array = tset["minions"]
	var mp: PackedVector2Array = m.minions_pos(i)
	var mk: PackedByteArray = m.minions_kind(i)
	for j in mp.size():
		var mkind: int = mk[j] if j < mk.size() else 0
		var mtx: Texture2D = theme_minions[mkind] if mkind < theme_minions.size() else null
		if mtx:
			_blit(mtx, center + Vector2(mp[j].x * scl, -mp[j].y * scl), 18.0)

	# tank — this player's (theme, skin) texture. Living tanks get a faint
	# emissive lift so they bloom under glow, echoing the single-arena tank light.
	var tank_tx: Texture2D = _player_tank[i] if i < _player_tank.size() else null
	if tank_tx:
		_blit(tank_tx, center, 40.0, Color(1, 1, 1, 0.5) if dead else Color(1.18, 1.22, 1.35))

	# HP bar — fill in this player's theme HP color (dead → desaturated).
	var hp := maxi(int(arena[2]), 0)
	var maxhp := maxi(int(arena[3]), 1)
	var bh := 7.0 * us
	var bw := r.size.x - 16.0
	draw_rect(Rect2(r.position + Vector2(8, 8), Vector2(bw, bh)), Color(0, 0, 0, 0.55))
	draw_rect(Rect2(r.position + Vector2(8, 8), Vector2(bw * float(hp) / float(maxhp), bh)), hp_fill)

	# label + economy (round / gold / weapon count preserved). P# + skin name read
	# in theme text; your own cell keeps the accent so it pops.
	var eco: PackedInt64Array = m.economy(i)
	var gold: int = eco[0] if eco.size() > 0 else 0
	var top_y := r.position.y + 26.0 + bh
	var pcol := c_accent if is_you else c_text
	draw_string(font, Vector2(r.position.x + 10, top_y), "P%d" % (i + 1),
		HORIZONTAL_ALIGNMENT_LEFT, -1, int(15 * us), pcol)
	# round/gold/weapon metadata: gold figure in the theme coin color, rest dim.
	var meta_x := r.position.x + 10.0 + (44.0 * us)
	draw_string(font, Vector2(meta_x, top_y), "R%d · " % arena[5],
		HORIZONTAL_ALIGNMENT_LEFT, -1, int(13 * us), c_dim)
	var rw: float = font.get_string_size("R%d · " % arena[5], HORIZONTAL_ALIGNMENT_LEFT, -1, int(13 * us)).x
	draw_string(font, Vector2(meta_x + rw, top_y), "%dg" % gold,
		HORIZONTAL_ALIGNMENT_LEFT, -1, int(13 * us), c_coin)
	var gw: float = font.get_string_size("%dg" % gold, HORIZONTAL_ALIGNMENT_LEFT, -1, int(13 * us)).x
	draw_string(font, Vector2(meta_x + rw + gw, top_y), " · %dw" % m.weapon_count(i),
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
	var tag := tr(_theme_tag(theme_idx))
	var pill_fs := int(11 * us)
	var tag_w: float = font.get_string_size(tag, HORIZONTAL_ALIGNMENT_LEFT, -1, pill_fs).x + 14.0
	var pill_h := 18.0 * us
	var pill := Rect2(r.position.x + r.size.x - tag_w - 8.0, r.position.y + r.size.y - pill_h - 8.0, tag_w, pill_h)
	draw_rect(pill, Color(c_accent.r, c_accent.g, c_accent.b, 0.22))
	draw_rect(pill, c_header, false, 1.0)
	draw_string(font, Vector2(pill.position.x + 7.0, pill.position.y + pill_h - 5.0), tag,
		HORIZONTAL_ALIGNMENT_LEFT, -1, pill_fs, c_header)

	# cell border — your cell gets a brighter, thicker accent + glow-y double frame
	# in YOUR theme's accent; peers get their own theme's panel_border.
	if is_you:
		draw_rect(r, c_accent, false, 4.0)
		draw_rect(Rect2(r.position + Vector2(3, 3), r.size - Vector2(6, 6)),
			Color(c_accent.r, c_accent.g, c_accent.b, 0.35), false, 1.5)
	else:
		draw_rect(r, c_border, false, 2.0)

	# dead overlay + placement — placement text tinted to this theme's danger color.
	if dead:
		draw_rect(r, Color(0, 0, 0, 0.5))
		var place: int = m.placement(i)
		var txt := tr("OUT") if place == 0 else "#%d" % place
		var dead_fs := int(26 * us)
		var dw: float = font.get_string_size(txt, HORIZONTAL_ALIGNMENT_LEFT, -1, dead_fs).x
		draw_string(font, center - Vector2(dw * 0.5, dead_fs * 0.25), txt,
			HORIZONTAL_ALIGNMENT_LEFT, -1, dead_fs, c_danger)

# Compressed damage score for the per-player tracker: log-scaled so it climbs
# steadily but never runs into the thousands (raw damage reaches the millions).
func _score(i: int) -> int:
	var st: PackedInt64Array = m.stats(i)
	var dmg: float = float(st[0]) if st.size() > 0 else 0.0
	if dmg < 1.0:
		return 0
	return int(round(30.0 * log(1.0 + dmg) / log(10.0)))
