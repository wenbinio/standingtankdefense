# Start-screen tank-skin selector. A grid gallery of every skin; unlocked ones
# are pickable, locked ones are dimmed and show the achievement that grants them.
# Selection persists via the Profile autoload; deploy loads the match.
extends Node2D

const COLS := 7

# Shared pause/settings overlay script, instanced here in settings-only mode
# ([O]; volumes/mute/language/screen-shake — same pane the in-run pause shows).
const PAUSE_MENU := preload("res://pause_menu.gd")

var font: Font
var sel := 0
var cards: Array[Rect2] = []
var thumbs := {}              # skin id -> Texture2D (cached up-front, per theme)
var _settings: Node2D = null  # lazily created PAUSE_MENU child (standalone mode)
# A6: snapshot of achievements unlocked since the gallery was last viewed —
# drawn as the "N NEW" badge + per-card NEW tags for THIS visit, then the
# backing profile list is cleared (viewing SkinSelect counts as seeing them).
var _new_ids: Array = []

func _ready() -> void:
	font = ArtTheme.ui_font(false)   # Barlow + Noto SC fallback (renders CJK)
	# Menu music bed (render-only; set_music dedupes, so re-entering is free).
	Audio.set_music("menu_theme.wav")
	# Apply the persisted (or default "en") UI language at the menu root.
	TranslationServer.set_locale(Profile.locale())
	for i in Profile.SKINS.size():
		if Profile.SKINS[i].id == Profile.selected:
			sel = i
	_new_ids = Profile.unseen_achievements.duplicate()
	Profile.mark_achievements_seen()
	_cache_thumbs()
	queue_redraw()

func _cache_thumbs() -> void:
	thumbs.clear()
	for s in Profile.SKINS:
		# One shared (theme, skin) resolver on ArtTheme replaces the local copy.
		thumbs[s.id] = ArtTheme.tank_tex_for(ArtTheme.active, s.id)

func _deploy() -> void:
	var s: Dictionary = Profile.SKINS[sel]
	if Profile.is_unlocked(s.id):
		Audio.play(&"ui_click")
		Profile.select(s.id)
		Profile.active_challenge_code = 0   # plain deploy = you play, free of any rule
		get_tree().change_scene_to_file("res://Main.tscn")
	else:
		Audio.play(&"ui_deny")              # locked skin: audible rejection

# InputMap actions (bindings in project.godot [input]); was raw keycodes in
# _input — moved to _unhandled_input like every other screen.
func _unhandled_input(e: InputEvent) -> void:
	# The settings overlay owns EVERY event while open (keys, clicks, motion for
	# slider drags). Redraw on close so a language flip re-resolves every tr().
	if _settings != null and _settings.is_open():
		_settings.handle_input(e)
		if not _settings.is_open():
			queue_redraw()
		return
	if e is InputEventMouseButton:
		if e.pressed and e.button_index == MOUSE_BUTTON_LEFT:
			for i in cards.size():
				if cards[i].has_point(e.position):
					if i == sel:
						_deploy()
					else:
						sel = i
						Audio.play(&"ui_move")
					queue_redraw()
		return
	if not (e is InputEventKey or e is InputEventJoypadButton):
		return
	var n := Profile.SKINS.size()
	if e.is_action_pressed(&"ui_nav_left"):
		sel = (sel - 1 + n) % n
		Audio.play(&"ui_move")
		queue_redraw()
	elif e.is_action_pressed(&"ui_nav_right"):
		sel = (sel + 1) % n
		Audio.play(&"ui_move")
		queue_redraw()
	elif e.is_action_pressed(&"ui_nav_up"):
		sel = (sel - COLS + n) % n
		Audio.play(&"ui_move")
		queue_redraw()
	elif e.is_action_pressed(&"ui_nav_down"):
		sel = (sel + COLS) % n
		Audio.play(&"ui_move")
		queue_redraw()
	elif e.is_action_pressed(&"ui_challenges"):
		Audio.play(&"ui_click")
		get_tree().change_scene_to_file("res://ChallengeSelect.tscn")
	elif e.is_action_pressed(&"ui_lobby"):
		Audio.play(&"ui_click")
		get_tree().change_scene_to_file("res://Lobby.tscn")   # multiplayer lobby (host-authoritative)
	elif e.is_action_pressed(&"ui_net_view"):
		Audio.play(&"ui_click")
		get_tree().change_scene_to_file("res://Match.tscn")   # multi-arena net demo
	elif e.is_action_pressed(&"ui_theme_cycle"):
		ArtTheme.cycle()
		_cache_thumbs()
		Audio.play(&"ui_click")
		queue_redraw()
	elif e.is_action_pressed(&"ui_language"):
		_toggle_language()
		Audio.play(&"ui_click")
	elif e.is_action_pressed(&"ui_difficulty"):
		# SP difficulty is a DEPLOY-TIME choice, so it lives here (not in the
		# pause-menu settings): cycle Easy → Normal → Hard, persisted.
		Profile.set_difficulty((Profile.difficulty() + 1) % Profile.DIFF_NAMES.size())
		Audio.play(&"ui_click")
		queue_redraw()
	elif e.is_action_pressed(&"ui_settings"):
		_open_settings()
	elif e.is_action_pressed(&"ui_dev_unlock"):
		Profile.unlock_all()          # dev: preview the gallery
		queue_redraw()
	elif e.is_action_pressed(&"ui_dev_reset"):
		Profile.reset()               # dev: relock everything
		sel = 0
		queue_redraw()
	elif e.is_action_pressed(&"ui_confirm"):
		_deploy()
	elif e.is_action_pressed(&"ui_back"):
		Audio.play(&"ui_back")
		get_tree().quit()

# Open the shared settings pane (pause_menu.gd, settings-only mode) on top of
# the gallery. Created lazily once, reused across opens; drawn above the cards
# (last child). No sim exists on this screen, so "pause" semantics don't apply.
func _open_settings() -> void:
	if _settings == null:
		_settings = PAUSE_MENU.new()
		_settings.standalone_settings = true
		add_child(_settings)
	_settings.open_settings()

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
	var unlocked_txt := tr("%d / %d unlocked") % [have, n]
	draw_string(font, Vector2(330, 46), unlocked_txt,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 16, Color(0.55, 0.78, 0.6))
	if not _new_ids.is_empty():
		# A6: "N NEW" badge beside the unlock tally for freshly earned
		# achievements (cleared for next visit — this viewing counts as seen).
		var badge := tr("%d NEW") % _new_ids.size()
		var bx := 330.0 + font.get_string_size(unlocked_txt, HORIZONTAL_ALIGNMENT_LEFT, -1, 16).x + 16.0
		var bw := font.get_string_size(badge, HORIZONTAL_ALIGNMENT_LEFT, -1, 14).x
		draw_rect(Rect2(bx - 6.0, 46.0 - 15.0, bw + 12.0, 20.0), Color(0.85, 0.67, 0.30, 0.22))
		draw_string(font, Vector2(bx, 46), badge, HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(0.95, 0.8, 0.4))
	draw_string(font, Vector2(36, 72),
		tr("Unlock skins via achievements — purist runs (one weapon type), no-economy, and more.   [arrows] move   [Enter] play   [C] challenges   [L] lobby   [M] net demo   [T] theme   [U] dev-unlock"),
		HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(0.56, 0.6, 0.68))
	# Language toggle hint + current language (own line; label shown in its
	# script) + the settings-pane key + the SP difficulty cycler (deploy-time
	# choice; records only land on Normal, hinted in amber on Easy/Hard).
	var prefs := "[G] %s   ·   [O] %s   ·   %s" % [_lang_label(), tr("Settings"),
		tr("[H] Difficulty: %s") % tr(Profile.difficulty_name())]
	draw_string(font, Vector2(36, 90), prefs,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(0.62, 0.72, 0.9))
	if Profile.difficulty() != Profile.DIFF_NORMAL:
		var hx := 36.0 + font.get_string_size(prefs, HORIZONTAL_ALIGNMENT_LEFT, -1, 14).x + 16.0
		draw_string(font, Vector2(hx, 90), tr("records: Normal only"),
			HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color(0.85, 0.67, 0.30))

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
		# A7: quantifiable-goal progress vs your best-ever numbers (threshold
		# achievements only; constraint ones have no partial progress to show).
		var g: Dictionary = Profile.goal_progress(s.unlock, Profile.best_rec())
		if not g.is_empty():
			draw_string(font, r.position + Vector2(8, ch - 4),
				"%s / %s" % [Profile.fmt_num(int(g["current"])), Profile.fmt_num(int(g["target"]))],
				HORIZONTAL_ALIGNMENT_LEFT, cw - 12, 10, Color(0.85, 0.72, 0.4))
	# A6: freshly-earned tag on cards whose unlock is in this visit's snapshot.
	if _new_ids.has(s.unlock):
		var nb := tr("NEW")
		var nbw := font.get_string_size(nb, HORIZONTAL_ALIGNMENT_LEFT, -1, 11).x
		draw_string(font, r.position + Vector2(cw - nbw - 8.0, 18.0), nb,
			HORIZONTAL_ALIGNMENT_LEFT, -1, 11, Color(0.95, 0.8, 0.4))
	# selection outline
	if i == sel:
		draw_rect(r, Color(0.42, 0.72, 1.0) if unlocked else Color(0.85, 0.45, 0.32), false, 3.0)
