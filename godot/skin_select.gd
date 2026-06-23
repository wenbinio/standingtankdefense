# Start-screen tank-skin selector. A grid gallery of every skin; unlocked ones
# are pickable, locked ones are dimmed and show the achievement that grants them.
# Selection persists via the Profile autoload; deploy loads the match.
extends Node2D

const COLS := 7

var font: Font
var sel := 0
var cards: Array[Rect2] = []
var thumbs := {}              # skin id -> Texture2D (cached up-front, per theme)

func _ready() -> void:
	font = ArtTheme.ui_font(false)   # Barlow + Noto SC fallback (renders CJK)
	# Apply the persisted (or default "en") UI language at the menu root.
	TranslationServer.set_locale(Profile.locale())
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
		Profile.active_challenge_code = 0   # plain deploy = you play, free of any rule
		get_tree().change_scene_to_file("res://Main.tscn")

func _input(e: InputEvent) -> void:
	var n := Profile.SKINS.size()
	if e is InputEventKey and e.pressed and not e.echo:
		match e.keycode:
			KEY_LEFT, KEY_A:  sel = (sel - 1 + n) % n; queue_redraw()
			KEY_RIGHT, KEY_D: sel = (sel + 1) % n; queue_redraw()
			KEY_UP, KEY_W:    sel = (sel - COLS + n) % n; queue_redraw()
			KEY_DOWN, KEY_S:  sel = (sel + COLS) % n; queue_redraw()
			KEY_C: get_tree().change_scene_to_file("res://ChallengeSelect.tscn")
			KEY_L: get_tree().change_scene_to_file("res://Lobby.tscn")   # multiplayer lobby (host-authoritative)
			KEY_M: get_tree().change_scene_to_file("res://Match.tscn")   # multi-arena net demo
			KEY_T: ArtTheme.cycle(); _cache_thumbs(); queue_redraw()
			KEY_G: _toggle_language()
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

# Flip the UI language between English and Simplified Chinese, persist it, and
# redraw so every tr()'d string re-resolves. Render-layer only.
func _toggle_language() -> void:
	var next := "zh_CN" if TranslationServer.get_locale().begins_with("en") else "en"
	TranslationServer.set_locale(next)
	Profile.set_locale_pref(next)
	queue_redraw()

# Human-readable name of the active UI language (not in the translation table;
# always shown in its own script).
func _lang_label() -> String:
	return "简体中文" if TranslationServer.get_locale().begins_with("zh") else "English"

func _draw() -> void:
	var vp: Vector2 = get_viewport_rect().size
	draw_rect(Rect2(Vector2.ZERO, vp), Color(0.035, 0.04, 0.055))
	var n := Profile.SKINS.size()
	var have := 0
	for s in Profile.SKINS:
		if Profile.is_unlocked(s.id): have += 1
	draw_string(font, Vector2(36, 46), tr("SELECT YOUR TANK"),
		HORIZONTAL_ALIGNMENT_LEFT, -1, 30, Color(0.9, 0.93, 0.98))
	draw_string(font, Vector2(330, 46), tr("%d / %d unlocked") % [have, n],
		HORIZONTAL_ALIGNMENT_LEFT, -1, 16, Color(0.55, 0.78, 0.6))
	draw_string(font, Vector2(36, 72),
		tr("Unlock skins via achievements — purist runs (one weapon type), no-economy, and more.   [arrows] move   [Enter] play   [C] challenges   [L] lobby   [M] net demo   [T] theme   [U] dev-unlock"),
		HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(0.56, 0.6, 0.68))
	# Language toggle hint + current language (own line; label shown in its script).
	draw_string(font, Vector2(36, 90), "[G] %s" % _lang_label(),
		HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(0.62, 0.72, 0.9))

	cards.clear()
	var rows := int(ceil(float(n) / COLS))
	var mx := 36.0
	var top := 92.0
	var bottom := 46.0
	var pad := 12.0
	var cw := (vp.x - 2.0 * mx - pad * (COLS - 1)) / COLS
	var ch := (vp.y - top - bottom - pad * (rows - 1)) / rows
	for i in n:
		var s: Dictionary = Profile.SKINS[i]
		var col := i % COLS
		var row := i / COLS
		var r := Rect2(mx + col * (cw + pad), top + row * (ch + pad), cw, ch)
		cards.append(r)
		_draw_card(s, r, cw, ch, i)

	# deploy bar
	var sd: Dictionary = Profile.SKINS[sel]
	var ok: bool = Profile.is_unlocked(sd.id)
	var msg := tr("[ DEPLOY: %s ]") % tr(sd.name) if ok else tr("[ LOCKED: %s — %s ]") % [tr(sd.name), tr(Profile.ach_def(sd.unlock).get("desc", ""))]
	draw_string(font, Vector2(mx, vp.y - 16), msg,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 18,
		Color(0.5, 0.9, 0.6) if ok else Color(0.8, 0.45, 0.4))

func _draw_card(s: Dictionary, r: Rect2, cw: float, ch: float, i: int) -> void:
	var unlocked: bool = Profile.is_unlocked(s.id)
	var chosen: bool = s.id == Profile.selected
	draw_rect(r, Color(0.10, 0.11, 0.14) if unlocked else Color(0.07, 0.075, 0.09))
	# thumbnail
	var t: Texture2D = thumbs.get(s.id)
	if t:
		var ts := minf(cw * 0.82, ch * 0.56)
		var tp := r.position + Vector2((cw - ts) * 0.5, 8.0)
		draw_texture_rect(t, Rect2(tp, Vector2(ts, ts)), false,
			Color.WHITE if unlocked else Color(0.16, 0.16, 0.19))
	# name (skin names live in the translation table)
	draw_string(font, r.position + Vector2(8, ch - 56), tr(s.name),
		HORIZONTAL_ALIGNMENT_LEFT, cw - 12, 15,
		Color(0.92, 0.94, 0.98) if unlocked else Color(0.5, 0.5, 0.56))
	if unlocked:
		draw_string(font, r.position + Vector2(8, ch - 36), tr(s.blurb),
			HORIZONTAL_ALIGNMENT_LEFT, cw - 12, 11, Color(0.56, 0.6, 0.68))
		if chosen:
			draw_string(font, r.position + Vector2(8, ch - 14), tr("✓ EQUIPPED"),
				HORIZONTAL_ALIGNMENT_LEFT, -1, 12, Color(0.4, 0.85, 0.55))
	else:
		var a := Profile.ach_def(s.unlock)
		draw_string(font, r.position + Vector2(8, ch - 38), tr("LOCKED: %s") % tr(String(a.get("name", ""))),
			HORIZONTAL_ALIGNMENT_LEFT, cw - 12, 12, Color(0.85, 0.55, 0.34))
		draw_string(font, r.position + Vector2(8, ch - 18), tr(String(a.get("desc", ""))),
			HORIZONTAL_ALIGNMENT_LEFT, cw - 12, 10, Color(0.55, 0.5, 0.5))
	# selection outline
	if i == sel:
		draw_rect(r, Color(0.42, 0.72, 1.0) if unlocked else Color(0.85, 0.45, 0.32), false, 3.0)
