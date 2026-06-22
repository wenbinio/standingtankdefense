# Multi-arena / net view. Runs the REAL netcode loop in `StMatch` (authoritative
# director + N clients + hub, bot-driven) and renders every player's
# authoritative shadow arena as a grid — the visual proof of the sharded-sim
# architecture: N independent arenas under one director, no entity replication.
extends Node2D

const N := 8                  # players in the demo match

# Relative paths under a theme folder (the locked art filename contract), in
# enemy-kind order for the cached enemy set and minion-kind order for minions.
const ENEMY_REL := [
	"enemies/fel_orc_grunt.svg", "enemies/steam_tank.svg",
	"enemies/samwise.svg", "enemies/fel_orc_peon.svg",
	"enemies/fel_orc_raider.svg", "enemies/bandit_rider.svg",
	"enemies/mountain_giant.svg", "enemies/fel_orc_warlock.svg",
	"enemies/poisonspitter.svg", "enemies/firebreather.svg",
	"enemies/icebreather.svg", "enemies/target_dummy.svg",
]
const MINION_REL := ["minions/skeleton.svg", "minions/infernal.svg"]

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
	m = StMatch.new_match(N, randi())
	# You are player 0; honor a chosen challenge so its achievement is earnable.
	if Profile.active_challenge_code != 0:
		m.set_challenge(0, Profile.active_challenge_code)
	_assign_cosmetics()
	_cache_theme_textures()
	_setup_environment()

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
		elif e.keycode == KEY_S:
			get_tree().change_scene_to_file("res://SkinSelect.tscn")
		elif e.keycode == KEY_ESCAPE:
			get_tree().quit()

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
	queue_redraw()

func _blit(tx: Texture2D, center: Vector2, size: float, mod := Color.WHITE) -> void:
	draw_texture_rect(tx, Rect2(center - Vector2(size, size) * 0.5, Vector2(size, size)), false, mod)

func _draw() -> void:
	if m == null:
		return
	var vp: Vector2 = get_viewport_rect().size
	var font := ThemeDB.fallback_font
	var n: int = m.player_count()

	# header
	var status := "MATCH OVER" if m.match_over() else "LIVE"
	draw_string(font, Vector2(16, 26),
		"MULTI-ARENA NET VIEW  —  %d sharded sims · 1 authoritative director · server tick %d · alive %d/%d  [%s]"
		% [n, m.server_tick(), m.alive_count(), n, status],
		HORIZONTAL_ALIGNMENT_LEFT, -1, 16, Color(0.82, 0.88, 0.96))
	var you := "YOU: %s  ·  [S] skins" % Profile.skin_def(Profile.selected).name
	if Profile.active_challenge_code != 0:
		for c in Profile.CHALLENGES:
			if c.code == Profile.active_challenge_code:
				you = "CHALLENGE: %s  ·  %s" % [c.name, you]
				break
	draw_string(font, Vector2(vp.x - mini(int(vp.x) - 20, 16 + you.length() * 7), 26),
		you, HORIZONTAL_ALIGNMENT_LEFT, -1, 13, Color(0.72, 0.66, 0.5))
	if not _toast.is_empty():
		draw_string(font, Vector2(16, vp.y - 16),
			"ACHIEVEMENT UNLOCKED:  " + ", ".join(_toast),
			HORIZONTAL_ALIGNMENT_LEFT, -1, 18, Color(1.0, 0.82, 0.4))

	# grid
	var cols: int = mini(n, 4)
	var rows: int = int(ceil(float(n) / cols))
	var pad := 8.0
	var top := 40.0
	var cw := (vp.x - pad * (cols + 1)) / cols
	var ch := (vp.y - top - pad * (rows + 1)) / rows
	for i in n:
		var col: int = i % cols
		var row: int = i / cols
		var rect := Rect2(pad + col * (cw + pad), top + pad + row * (ch + pad), cw, ch)
		_draw_cell(font, i, rect)

# A short uppercase tag for a theme index, for the per-cell theme pill.
func _theme_tag(theme_idx: int) -> String:
	match ArtTheme.themes[theme_idx]:
		"grimdark":         return "GRIMDARK"
		"gaslamp_bulwark":  return "GASLAMP"
	return ArtTheme.themes[theme_idx].to_upper()

func _draw_cell(font, i: int, r: Rect2) -> void:
	var arena: PackedInt64Array = m.arena(i)   # [x,y,hp,maxhp,rev,round,tick,dead]
	if arena.size() < 8:
		return
	var dead: bool = arena[7] != 0
	var center := r.position + Vector2(r.size.x * 0.5, r.size.y * 0.5 + 8.0)
	var scl: float = minf(r.size.x, r.size.y) * 0.42 / 1700.0

	# This player's chosen cosmetics drive every texture in the cell.
	var pc: Dictionary = players[i] if i < players.size() else {"theme": 0, "skin": "ol_reliable"}
	var theme_idx: int = pc["theme"]
	var tset: Dictionary = _theme_tex[theme_idx]
	var is_you: bool = i == 0

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

	# HP bar
	var hp := maxi(int(arena[2]), 0)
	var maxhp := maxi(int(arena[3]), 1)
	var bw := r.size.x - 16.0
	draw_rect(Rect2(r.position + Vector2(8, 8), Vector2(bw, 7)), Color(0, 0, 0, 0.55))
	draw_rect(Rect2(r.position + Vector2(8, 8), Vector2(bw * float(hp) / float(maxhp), 7)),
		Color(0.5, 0.5, 0.55) if dead else Color(0.33, 0.72, 1.0))

	# label + economy (round / gold / weapon count preserved)
	var eco: PackedInt64Array = m.economy(i)
	var gold: int = eco[0] if eco.size() > 0 else 0
	var pcol := Color(1.0, 0.9, 0.45) if is_you else Color(0.9, 0.93, 0.98)
	draw_string(font, r.position + Vector2(10, 34), "P%d" % (i + 1), HORIZONTAL_ALIGNMENT_LEFT, -1, 15, pcol)
	draw_string(font, r.position + Vector2(46, 34), "R%d · %dg · %dw" % [arena[5], gold, m.weapon_count(i)],
		HORIZONTAL_ALIGNMENT_LEFT, -1, 13, Color(0.74, 0.66, 0.4))
	# per-player damage score (log-compressed so it never runs into the thousands)
	draw_string(font, Vector2(r.position.x + r.size.x - 104, r.position.y + 34), "SCORE %d" % _score(i),
		HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(1.0, 0.78, 0.35))

	# net legibility: who picked what. Skin name line + a small theme pill, so
	# the grid reads like a lobby of cosmetic choices.
	var skin_name: String = Profile.skin_def(pc["skin"]).name
	var who := ("P%d · %s" % [i + 1, skin_name]) + ("  ★ YOU" if is_you else "")
	draw_string(font, Vector2(r.position.x + 10, r.position.y + r.size.y - 12), who,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 13, Color(1.0, 0.95, 0.7) if is_you else Color(0.86, 0.9, 0.96))
	# theme pill, bottom-right
	var tag := _theme_tag(theme_idx)
	var tag_w: float = tag.length() * 8.0 + 12.0
	var pill := Rect2(r.position.x + r.size.x - tag_w - 8.0, r.position.y + r.size.y - 26.0, tag_w, 18.0)
	var pill_col := Color(0.30, 0.16, 0.18, 0.85) if theme_idx == 0 else Color(0.16, 0.22, 0.30, 0.85)
	draw_rect(pill, pill_col)
	draw_rect(pill, Color(0.7, 0.7, 0.75, 0.5), false, 1.0)
	draw_string(font, Vector2(pill.position.x + 6.0, pill.position.y + 14.0), tag,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 11, Color(0.92, 0.94, 0.98))

	# cell border — your cell gets a brighter accent + glow-y double frame.
	if is_you:
		draw_rect(r, Color(1.0, 0.82, 0.32, 1.0), false, 3.0)
		draw_rect(Rect2(r.position + Vector2(2, 2), r.size - Vector2(4, 4)), Color(1.0, 0.9, 0.5, 0.35), false, 1.0)
	else:
		draw_rect(r, Color(0.06, 0.07, 0.09, 1.0), false, 2.0)

	# dead overlay + placement
	if dead:
		draw_rect(r, Color(0, 0, 0, 0.5))
		var place: int = m.placement(i)
		var txt := "OUT" if place == 0 else "#%d" % place
		draw_string(font, center - Vector2(22, 6), txt, HORIZONTAL_ALIGNMENT_LEFT, -1, 26, Color(1.0, 0.4, 0.28))

# Compressed damage score for the per-player tracker: log-scaled so it climbs
# steadily but never runs into the thousands (raw damage reaches the millions).
func _score(i: int) -> int:
	var st: PackedInt64Array = m.stats(i)
	var dmg: float = float(st[0]) if st.size() > 0 else 0.0
	if dmg < 1.0:
		return 0
	return int(round(30.0 * log(1.0 + dmg) / log(10.0)))
