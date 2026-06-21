# Challenge picker. Pick a self-imposed rule (or free play); the preview's
# player-0 bot honors it so the matching achievement — and its skin — is
# earnable on demand. Deploy launches the match with the rule applied; the
# achievement is granted at match-end by Profile.record_match as usual.
extends Node2D

var font: Font
var sel := 0                 # 0 = free play, 1..N = CHALLENGES[sel-1]
var rows: Array[Rect2] = []
var thumbs := {}             # skin id -> Texture2D

func _ready() -> void:
	font = ThemeDB.fallback_font
	# Resume on the currently-armed challenge, if any.
	for i in Profile.CHALLENGES.size():
		if Profile.CHALLENGES[i].code == Profile.active_challenge_code:
			sel = i + 1
	for c in Profile.CHALLENGES:
		var sk: Dictionary = Profile.skin_for_ach(c.ach)
		thumbs[sk.id] = _tex(sk)
	queue_redraw()

func _tex(s: Dictionary) -> Texture2D:
	if s.file != "":
		var p: String = ArtTheme.base() + "tank/" + s.file
		if ResourceLoader.exists(p):
			return load(p)
	return ArtTheme.tex("tank/player_tank.svg")

func _count() -> int:
	return Profile.CHALLENGES.size() + 1   # + free play

func _deploy() -> void:
	Profile.active_challenge_code = 0 if sel == 0 else int(Profile.CHALLENGES[sel - 1].code)
	get_tree().change_scene_to_file("res://Match.tscn")

func _input(e: InputEvent) -> void:
	var n := _count()
	if e is InputEventKey and e.pressed and not e.echo:
		match e.keycode:
			KEY_UP, KEY_W:    sel = (sel - 1 + n) % n; queue_redraw()
			KEY_DOWN, KEY_S:  sel = (sel + 1) % n; queue_redraw()
			KEY_ENTER, KEY_KP_ENTER, KEY_SPACE: _deploy()
			KEY_ESCAPE, KEY_C: get_tree().change_scene_to_file("res://SkinSelect.tscn")
	elif e is InputEventMouseButton and e.pressed and e.button_index == MOUSE_BUTTON_LEFT:
		for i in rows.size():
			if rows[i].has_point(e.position):
				if i == sel: _deploy()
				else: sel = i
				queue_redraw()

func _draw() -> void:
	var vp: Vector2 = get_viewport_rect().size
	draw_rect(Rect2(Vector2.ZERO, vp), Color(0.035, 0.04, 0.055))
	draw_string(font, Vector2(40, 50), "CHOOSE A CHALLENGE",
		HORIZONTAL_ALIGNMENT_LEFT, -1, 30, Color(0.9, 0.93, 0.98))
	draw_string(font, Vector2(40, 78),
		"Your tank's bot will honor the rule — clear it to unlock the reward skin.   [↑/↓] move   [Enter] deploy   [C/Esc] back",
		HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(0.56, 0.6, 0.68))

	rows.clear()
	var n := _count()
	var top := 100.0
	var rh := minf(60.0, (vp.y - top - 24.0) / n)
	for i in n:
		var r := Rect2(40.0, top + i * rh, vp.x - 80.0, rh - 6.0)
		rows.append(r)
		if i == 0:
			_draw_freeplay(r, i == sel)
		else:
			_draw_challenge(Profile.CHALLENGES[i - 1], r, rh, i == sel)

func _draw_freeplay(r: Rect2, on: bool) -> void:
	draw_rect(r, Color(0.10, 0.11, 0.14))
	draw_string(font, r.position + Vector2(76, r.size.y * 0.5 + 2), "Free Play",
		HORIZONTAL_ALIGNMENT_LEFT, -1, 18, Color(0.92, 0.94, 0.98))
	draw_string(font, r.position + Vector2(220, r.size.y * 0.5 + 2),
		"No rule — just deploy with your selected skin.",
		HORIZONTAL_ALIGNMENT_LEFT, -1, 13, Color(0.56, 0.6, 0.68))
	if on:
		draw_rect(r, Color(0.42, 0.72, 1.0), false, 3.0)

func _draw_challenge(c: Dictionary, r: Rect2, rh: float, on: bool) -> void:
	var sk: Dictionary = Profile.skin_for_ach(c.ach)
	var done: bool = Profile.has_ach(c.ach)
	draw_rect(r, Color(0.11, 0.12, 0.15) if not done else Color(0.10, 0.14, 0.11))
	# reward skin thumbnail
	var t: Texture2D = thumbs.get(sk.id)
	if t:
		var ts := rh - 14.0
		draw_texture_rect(t, Rect2(r.position + Vector2(6, 4), Vector2(ts, ts)), false)
	var cy := r.position.y + rh * 0.5
	draw_string(font, Vector2(r.position.x + 76, cy - 6), c.name,
		HORIZONTAL_ALIGNMENT_LEFT, 280, 17, Color(0.92, 0.94, 0.98))
	draw_string(font, Vector2(r.position.x + 76, cy + 13), c.rule,
		HORIZONTAL_ALIGNMENT_LEFT, 360, 12, Color(0.58, 0.62, 0.7))
	# reward + status (right side)
	var rx := r.position.x + r.size.x - 240.0
	draw_string(font, Vector2(rx, cy - 6), "Unlocks: " + String(sk.name),
		HORIZONTAL_ALIGNMENT_LEFT, 230, 13, Color(0.7, 0.74, 0.82))
	draw_string(font, Vector2(rx, cy + 13), ("✓ EARNED" if done else "locked"),
		HORIZONTAL_ALIGNMENT_LEFT, -1, 12,
		Color(0.4, 0.85, 0.55) if done else Color(0.7, 0.55, 0.4))
	if on:
		draw_rect(r, Color(0.42, 0.72, 1.0), false, 3.0)
