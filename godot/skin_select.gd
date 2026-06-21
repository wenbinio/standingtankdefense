# Start-screen tank-skin selector. A gallery of every skin; unlocked ones are
# pickable, locked ones are dimmed and show the achievement that grants them.
# Selection persists via the Profile autoload; deploy loads the match.
extends Node2D

var font: Font
var sel := 0
var cards: Array[Rect2] = []
var thumbs := {}              # skin id -> Texture2D (cached up-front, per theme)

func _ready() -> void:
	font = ThemeDB.fallback_font
	for i in Profile.SKINS.size():
		if Profile.SKINS[i].id == Profile.selected:
			sel = i
	_cache_thumbs()
	queue_redraw()

func _cache_thumbs() -> void:
	thumbs.clear()
	for s in Profile.SKINS:
		thumbs[s.id] = _load_skin_tex(s)

func _load_skin_tex(s: Dictionary) -> Texture2D:
	if s.file != "":
		var p: String = ArtTheme.base() + "tank/" + s.file
		if ResourceLoader.exists(p):
			return load(p)
	return ArtTheme.tex("tank/player_tank.svg")

func _deploy() -> void:
	var s: Dictionary = Profile.SKINS[sel]
	if Profile.is_unlocked(s.id):
		Profile.select(s.id)
		get_tree().change_scene_to_file("res://Match.tscn")

func _input(e: InputEvent) -> void:
	var n := Profile.SKINS.size()
	if e is InputEventKey and e.pressed and not e.echo:
		match e.keycode:
			KEY_LEFT, KEY_A:  sel = (sel - 1 + n) % n; queue_redraw()
			KEY_RIGHT, KEY_D: sel = (sel + 1) % n; queue_redraw()
			KEY_T: ArtTheme.cycle(); _cache_thumbs(); queue_redraw()
			KEY_U: Profile.unlock_all(); queue_redraw()         # dev: preview the gallery
			KEY_R: Profile.reset(); sel = 0; queue_redraw()     # dev: relock everything
			KEY_ENTER, KEY_KP_ENTER, KEY_SPACE: _deploy()
			KEY_ESCAPE: get_tree().quit()
	elif e is InputEventMouseButton and e.pressed and e.button_index == MOUSE_BUTTON_LEFT:
		for i in cards.size():
			if cards[i].has_point(e.position):
				if i == sel: _deploy()
				else: sel = i
				queue_redraw()

func _draw() -> void:
	var vp: Vector2 = get_viewport_rect().size
	draw_rect(Rect2(Vector2.ZERO, vp), Color(0.035, 0.04, 0.055))
	draw_string(font, Vector2(40, 58), "SELECT YOUR TANK",
		HORIZONTAL_ALIGNMENT_LEFT, -1, 34, Color(0.9, 0.93, 0.98))
	draw_string(font, Vector2(40, 86),
		"Skins unlock through achievements.   [←/→] choose    [Enter] deploy    [T] theme    [U] dev-unlock",
		HORIZONTAL_ALIGNMENT_LEFT, -1, 15, Color(0.58, 0.62, 0.7))

	cards.clear()
	var n := Profile.SKINS.size()
	var pad := 22.0
	var cw := (vp.x - 80.0 - pad * (n - 1)) / n
	var ch := minf(cw * 1.5, vp.y - 220.0)
	var top := 130.0
	for i in n:
		var s: Dictionary = Profile.SKINS[i]
		var r := Rect2(40.0 + i * (cw + pad), top, cw, ch)
		cards.append(r)
		var unlocked: bool = Profile.is_unlocked(s.id)
		var chosen: bool = s.id == Profile.selected

		draw_rect(r, Color(0.10, 0.11, 0.14) if unlocked else Color(0.07, 0.075, 0.09))
		# thumbnail
		var t: Texture2D = thumbs.get(s.id)
		if t:
			var ts := cw * 0.74
			var tp := r.position + Vector2((cw - ts) * 0.5, 16.0)
			draw_texture_rect(t, Rect2(tp, Vector2(ts, ts)), false,
				Color.WHITE if unlocked else Color(0.16, 0.16, 0.19))
		# name
		draw_string(font, r.position + Vector2(12, ch - 64), s.name,
			HORIZONTAL_ALIGNMENT_LEFT, cw - 20, 18,
			Color(0.92, 0.94, 0.98) if unlocked else Color(0.5, 0.5, 0.56))
		if unlocked:
			draw_string(font, r.position + Vector2(12, ch - 40), s.blurb,
				HORIZONTAL_ALIGNMENT_LEFT, cw - 20, 12, Color(0.58, 0.62, 0.7))
			if chosen:
				draw_string(font, r.position + Vector2(12, ch - 18), "✓ EQUIPPED",
					HORIZONTAL_ALIGNMENT_LEFT, -1, 13, Color(0.4, 0.85, 0.55))
		else:
			var a := Profile.ach_def(s.unlock)
			draw_string(font, r.position + Vector2(12, ch - 42), "LOCKED — " + String(a.get("name", "")),
				HORIZONTAL_ALIGNMENT_LEFT, cw - 20, 13, Color(0.85, 0.52, 0.32))
			draw_string(font, r.position + Vector2(12, ch - 22), String(a.get("desc", "")),
				HORIZONTAL_ALIGNMENT_LEFT, cw - 20, 11, Color(0.55, 0.5, 0.5))
		# selection outline
		if i == sel:
			draw_rect(r, Color(0.42, 0.72, 1.0) if unlocked else Color(0.85, 0.45, 0.32), false, 3.0)

	# deploy bar
	var sd: Dictionary = Profile.SKINS[sel]
	var ok: bool = Profile.is_unlocked(sd.id)
	var msg := "[ DEPLOY: %s ]" % sd.name if ok else "[ %s is locked ]" % sd.name
	draw_string(font, Vector2(40, vp.y - 28), msg,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 20,
		Color(0.5, 0.9, 0.6) if ok else Color(0.8, 0.45, 0.4))
