# Multi-arena / net view. Runs the REAL netcode loop in `StMatch` (authoritative
# director + N clients + hub, bot-driven) and renders every player's
# authoritative shadow arena as a grid — the visual proof of the sharded-sim
# architecture: N independent arenas under one director, no entity replication.
extends Node2D

const N := 8                  # players in the demo match

var m
var tex := {}
var enemy_tex := []           # by kind: 0 grunt, 1 steam, 2 boss
var minion_tex := []          # summoned allies: 0 skeleton, 1 infernal

func _ready() -> void:
	randomize()
	m = StMatch.new_match(N, randi())
	# You are player 0; honor a chosen challenge so its achievement is earnable.
	if Profile.active_challenge_code != 0:
		m.set_challenge(0, Profile.active_challenge_code)
	_load_textures()

func _load_textures() -> void:
	tex = {
		"ground": ArtTheme.tex("env/arena_ground.svg"),
		"ring":   ArtTheme.tex("env/spawn_ring.svg"),
		"tank":   ArtTheme.tank_tex(),
		"coin":   ArtTheme.tex("ui/coin.svg"),
	}
	enemy_tex = [
		ArtTheme.tex("enemies/fel_orc_grunt.svg"), ArtTheme.tex("enemies/steam_tank.svg"),
		ArtTheme.tex("enemies/samwise.svg"), ArtTheme.tex("enemies/fel_orc_peon.svg"),
		ArtTheme.tex("enemies/fel_orc_raider.svg"), ArtTheme.tex("enemies/bandit_rider.svg"),
		ArtTheme.tex("enemies/mountain_giant.svg"), ArtTheme.tex("enemies/fel_orc_warlock.svg"),
		ArtTheme.tex("enemies/poisonspitter.svg"), ArtTheme.tex("enemies/firebreather.svg"),
		ArtTheme.tex("enemies/icebreather.svg"), ArtTheme.tex("enemies/target_dummy.svg"),
	]
	minion_tex = [ArtTheme.tex("minions/skeleton.svg"), ArtTheme.tex("minions/infernal.svg")]

var _recorded := false        # match-end achievements credited once
var _toast: Array = []        # newly-unlocked achievement names to flash

func _unhandled_key_input(e: InputEvent) -> void:
	if e is InputEventKey and e.pressed and not e.echo:
		if e.keycode == KEY_T:
			ArtTheme.cycle(); _load_textures()
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

func _draw_cell(font, i: int, r: Rect2) -> void:
	var arena: PackedInt64Array = m.arena(i)   # [x,y,hp,maxhp,rev,round,tick,dead]
	if arena.size() < 8:
		return
	var dead: bool = arena[7] != 0
	var center := r.position + Vector2(r.size.x * 0.5, r.size.y * 0.5 + 8.0)
	var scl: float = minf(r.size.x, r.size.y) * 0.42 / 1700.0

	# arena floor + spawn ring (clipped to cell)
	draw_texture_rect(tex["ground"], r, false)
	_blit(tex["ring"], center, 2.0 * 1500.0 * scl / 0.90)

	# enemies (small)
	var ep: PackedVector2Array = m.enemies_pos(i)
	var ek: PackedByteArray = m.enemies_kind(i)
	for j in ep.size():
		var kind: int = ek[j] if j < ek.size() else 0
		var sz := 56.0 if kind == 2 else (30.0 if kind == 6 else 20.0)
		var tx: Texture2D = enemy_tex[kind] if kind < enemy_tex.size() else null
		if tx:
			_blit(tx, center + Vector2(ep[j].x * scl, -ep[j].y * scl), sz)

	# summoned allies
	var mp: PackedVector2Array = m.minions_pos(i)
	var mk: PackedByteArray = m.minions_kind(i)
	for j in mp.size():
		var mkind: int = mk[j] if j < mk.size() else 0
		var mtx: Texture2D = minion_tex[mkind] if mkind < minion_tex.size() else null
		if mtx:
			_blit(mtx, center + Vector2(mp[j].x * scl, -mp[j].y * scl), 18.0)

	# tank
	_blit(tex["tank"], center, 40.0, Color(1, 1, 1, 0.5) if dead else Color.WHITE)

	# HP bar
	var hp := maxi(int(arena[2]), 0)
	var maxhp := maxi(int(arena[3]), 1)
	var bw := r.size.x - 16.0
	draw_rect(Rect2(r.position + Vector2(8, 8), Vector2(bw, 7)), Color(0, 0, 0, 0.55))
	draw_rect(Rect2(r.position + Vector2(8, 8), Vector2(bw * float(hp) / float(maxhp), 7)),
		Color(0.5, 0.5, 0.55) if dead else Color(0.33, 0.72, 1.0))

	# label + economy
	var eco: PackedInt64Array = m.economy(i)
	var gold: int = eco[0] if eco.size() > 0 else 0
	draw_string(font, r.position + Vector2(10, 34), "P%d" % (i + 1), HORIZONTAL_ALIGNMENT_LEFT, -1, 15, Color(0.9, 0.93, 0.98))
	draw_string(font, r.position + Vector2(46, 34), "R%d · %dg · %dw" % [arena[5], gold, m.weapon_count(i)],
		HORIZONTAL_ALIGNMENT_LEFT, -1, 13, Color(0.74, 0.66, 0.4))
	# per-player damage score (log-compressed so it never runs into the thousands)
	draw_string(font, Vector2(r.position.x + r.size.x - 104, r.position.y + 34), "SCORE %d" % _score(i),
		HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(1.0, 0.78, 0.35))

	# cell border
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
