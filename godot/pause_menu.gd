# Pause + settings overlay (Main.tscn UiLayer; also embedded by skin_select.gd
# in settings-only mode). Immediate-mode drawn like results.gd/skin_select.gd:
# ArtTheme.ui colors, centered panel, keyboard (up/down/confirm) + mouse.
#
# SINGLE-PLAYER ONLY. This node exists only where the whole sim is private and
# offline (Main.tscn's local run; SkinSelect has no sim at all). While open,
# main.gd simply stops calling sim.step() — legal because nobody else consumes
# that sim. The net view (match.gd / the director model) must NEVER gate its
# sims on a local pause: remote peers keep ticking; docs/03's model has no
# global pause. This script therefore exposes no global/autoload state — the
# only way to reach it is owning a node instance and routing input to it.
#
# Owner contract (main.gd / skin_select.gd):
#   is_open() -> bool          gate your sim step / input handling on this
#   open_pause()               open at the pause menu (Resume/Restart/Settings/Quit)
#   open_settings()            open straight at the settings pane
#   handle_input(e)            forward EVERY InputEvent while is_open()
#                              (keys, clicks, and mouse motion for slider drag)
#   standalone_settings = true settings-only mode (no pause entries; back = close)
#
# COSMETIC/CADENCE ONLY: it never touches a sim. All settings it edits are
# persisted prefs (Audio volumes/mute -> [audio] in user://profile.cfg via
# audio.gd; language + screen shake + game speed -> [profile] via profile.gd).
# Game Speed only changes how often main.gd calls sim.step() (30/45/60/90
# ticks/s); every tick stays bit-identical, so determinism is untouched. It
# applies live: main.gd re-reads Profile.ticks_per_second() every physics frame.
extends Node2D

# Panes.
const CLOSED := 0
const MENU := 1             # Resume / Restart Run / Settings / Quit to Menu
const CONFIRM := 2          # one-step "really quit? run is live" gate
const SETTINGS := 3         # volumes / mute / language / screen shake
const CONFIRM_RESTART := 4  # one-step "really restart? run is live" gate (A12)

# Settings rows (index into the fixed row tables below).
const ROW_MASTER := 0
const ROW_SFX := 1
const ROW_MUSIC := 2
const ROW_MUTE := 3
const ROW_LANG := 4
const ROW_SHAKE := 5
const ROW_SPEED := 6
const ROW_BACK := 7
const SETTINGS_ROWS := 8
const MENU_ROWS := 4
const CONFIRM_ROWS := 2
const MAX_ROWS := 8            # fixed hit-rect capacity (largest pane)

const VOL_STEP := 0.05         # arrow-key volume increment (5%)

# Game-speed labels/multipliers by Profile.game_speed() code (labels are
# translation-table keys; the speed itself is CADENCE-only — see profile.gd).
const SPEED_NAMES := ["Normal", "Fast", "Faster", "Hyper"]
const SPEED_MULTS := ["1.0", "1.5", "2.0", "3.0"]

# Settings-only mode (SkinSelect): open_* lands on SETTINGS and "back" closes
# the overlay instead of returning to the pause menu.
var standalone_settings := false

# A12: owner-wired restart hook (main.gd sets this to its _redeploy). The menu
# closes itself FIRST, so the pause gate is fully released before the fresh
# run starts. Unset (invalid) in standalone/settings-only embeds.
var on_restart := Callable()

var _mode := CLOSED
var _sel := 0                  # keyboard cursor within the active pane
var _drag_row := -1            # volume row being mouse-dragged (-1 = none)

var _font: Font = null
var _font_head: Font = null

# Hit rects, recomputed by _draw each frame and consumed by handle_input.
# Preallocated to fixed capacity (no per-frame Array churn; Rect2 is a value).
var _row_rects: Array[Rect2] = []
var _slider_rects: Array[Rect2] = []      # bar rects for ROW_MASTER..ROW_MUSIC

func _ready() -> void:
	_font = ArtTheme.ui_font(false)
	if _font == null:
		_font = ThemeDB.fallback_font
	_font_head = ArtTheme.ui_font(true)
	if _font_head == null:
		_font_head = _font
	_row_rects.resize(MAX_ROWS)
	_slider_rects.resize(3)
	visible = false

func _process(_delta: float) -> void:
	if _mode != CLOSED:
		queue_redraw()      # hover/drag feedback; same pattern as results.gd

# --- owner API -----------------------------------------------------------------

func is_open() -> bool:
	return _mode != CLOSED

func open_pause() -> void:
	Audio.play(&"ui_click")
	_enter(SETTINGS if standalone_settings else MENU)

func open_settings() -> void:
	Audio.play(&"ui_click")
	_enter(SETTINGS)

# Route one InputEvent here while is_open(). The overlay owns everything the
# owner forwards; it never reads global input itself, so it can't leak into
# other scenes (the net view never instantiates it).
func handle_input(e: InputEvent) -> void:
	if e is InputEventMouseMotion:
		if _drag_row >= 0:
			_drag_to(e.position)
		return
	if e is InputEventMouseButton:
		var mb := e as InputEventMouseButton
		if mb.button_index == MOUSE_BUTTON_LEFT:
			if mb.pressed:
				_click(mb.position)
			else:
				_drag_row = -1
		return
	if not (e is InputEventKey or e is InputEventJoypadButton):
		return
	if e.is_action_pressed(&"ui_back"):
		_back()
	elif e.is_action_pressed(&"ui_nav_up"):
		_move_sel(-1)
	elif e.is_action_pressed(&"ui_nav_down"):
		_move_sel(1)
	elif e.is_action_pressed(&"ui_nav_left"):
		_adjust(-1)
	elif e.is_action_pressed(&"ui_nav_right"):
		_adjust(1)
	elif e.is_action_pressed(&"ui_confirm"):
		_activate(_sel)

# --- pane state ------------------------------------------------------------------

func _enter(mode: int) -> void:
	_mode = mode
	_sel = 0
	_drag_row = -1
	visible = mode != CLOSED
	queue_redraw()

func _close() -> void:
	_enter(CLOSED)

# Esc / back: settings -> menu (or close when standalone), confirm -> menu,
# menu -> resume (close). Esc-while-paused therefore resumes.
func _back() -> void:
	Audio.play(&"ui_back")
	match _mode:
		SETTINGS:
			if standalone_settings:
				_close()
			else:
				_enter(MENU)
		CONFIRM:
			_enter(MENU)
		CONFIRM_RESTART:
			_enter(MENU)
		MENU:
			_close()

func _row_count() -> int:
	match _mode:
		MENU: return MENU_ROWS
		CONFIRM: return CONFIRM_ROWS
		CONFIRM_RESTART: return CONFIRM_ROWS
		SETTINGS: return SETTINGS_ROWS
	return 0

func _move_sel(dir: int) -> void:
	var n := _row_count()
	if n <= 0:
		return
	_sel = (_sel + dir + n) % n
	Audio.play(&"ui_move")
	queue_redraw()

# Confirm / click on row `i` of the active pane.
func _activate(i: int) -> void:
	match _mode:
		MENU:
			Audio.play(&"ui_click")
			match i:
				0: _close()                                   # Resume
				1: _enter(CONFIRM_RESTART)                    # Restart -> confirm gate
				2: _enter(SETTINGS)                           # Settings
				3: _enter(CONFIRM)                            # Quit -> confirm gate
		CONFIRM:
			Audio.play(&"ui_click")
			match i:
				0: _close()                                   # Keep Playing (resume)
				1: get_tree().change_scene_to_file("res://SkinSelect.tscn")
		CONFIRM_RESTART:
			match i:
				0: _close()                                   # Keep Playing (resume)
				1:
					# Close FIRST (releases the pause gate + resets pane state),
					# then hand off to the owner's _redeploy path.
					_close()
					if on_restart.is_valid():
						on_restart.call()
		SETTINGS:
			match i:
				ROW_MUTE:
					var muted: bool = Audio.toggle_mute()
					if not muted:
						Audio.play(&"ui_move")                # audible unmute confirm
				ROW_LANG:
					_toggle_language()
					Audio.play(&"ui_click")
				ROW_SHAKE:
					Profile.set_screen_shake(not Profile.screen_shake())
					Audio.play(&"ui_click")
				ROW_SPEED:
					# Enter cycles forward; left/right in _adjust go both ways.
					Profile.set_game_speed(posmod(Profile.game_speed() + 1, SPEED_NAMES.size()))
					Audio.play(&"ui_click")
				ROW_BACK:
					_back()                                   # voices ui_back itself
				_:
					pass   # volume rows adjust via left/right or drag, not confirm

# Left/right on a settings row: nudge sliders, flip toggles.
func _adjust(dir: int) -> void:
	if _mode != SETTINGS:
		return
	if _sel <= ROW_MUSIC:
		_set_vol(_sel, _vol(_sel) + VOL_STEP * dir)
		Audio.play(&"ui_move")     # audible level feedback (rides the new volume)
	elif _sel == ROW_SPEED:
		# Directional cycle through Normal/Fast/Faster/Hyper (wraps both ways).
		Profile.set_game_speed(posmod(Profile.game_speed() + dir, SPEED_NAMES.size()))
	elif _sel == ROW_MUTE or _sel == ROW_LANG or _sel == ROW_SHAKE:
		_activate(_sel)
	queue_redraw()

# --- mouse -----------------------------------------------------------------------

func _click(pos: Vector2) -> void:
	# Volume bars first: press starts a drag (released in handle_input).
	if _mode == SETTINGS:
		for row in 3:
			if _slider_rects[row].grow(6.0).has_point(pos):
				_sel = row
				_drag_row = row
				_drag_to(pos)
				return
	for i in _row_count():
		if _row_rects[i].has_point(pos):
			_sel = i
			_activate(i)
			return

func _drag_to(pos: Vector2) -> void:
	if _drag_row < 0:
		return
	var bar := _slider_rects[_drag_row]
	if bar.size.x <= 0.0:
		return
	_set_vol(_drag_row, (pos.x - bar.position.x) / bar.size.x)
	queue_redraw()

# --- setting accessors -------------------------------------------------------------

# Volume rows map straight onto Audio's linear 0..1 getters/setters; every set
# persists automatically ([audio] section of user://profile.cfg).
func _vol(row: int) -> float:
	match row:
		ROW_MASTER: return Audio.master_volume()
		ROW_SFX: return Audio.sfx_volume()
		ROW_MUSIC: return Audio.music_volume()
	return 0.0

func _set_vol(row: int, v: float) -> void:
	v = clampf(v, 0.0, 1.0)
	match row:
		ROW_MASTER: Audio.set_master_volume(v)
		ROW_SFX: Audio.set_sfx_volume(v)
		ROW_MUSIC: Audio.set_music_volume(v)

# EN <-> zh_CN flip, persisted — the exact pattern skin_select.gd uses. Every
# tr()'d string re-resolves on the next redraw.
func _toggle_language() -> void:
	var next := "zh_CN" if TranslationServer.get_locale().begins_with("en") else "en"
	TranslationServer.set_locale(next)
	Profile.set_locale_pref(next)
	queue_redraw()

# Active-language label, always in its own script (never translated).
func _lang_label() -> String:
	return "简体中文" if TranslationServer.get_locale().begins_with("zh") else "English"

# --- drawing -----------------------------------------------------------------------

func _draw() -> void:
	if _mode == CLOSED:
		return
	var vp: Vector2 = get_viewport_rect().size
	# Dim the scene behind the panel (same wash as the results panel).
	draw_rect(Rect2(Vector2.ZERO, vp), Color(0.02, 0.02, 0.04, 0.55))
	match _mode:
		MENU:
			_draw_menu(vp)
		CONFIRM:
			_draw_confirm(vp, tr("QUIT TO MENU?"),
				tr("This run is still live — quitting abandons it."), tr("Quit to Menu"))
		CONFIRM_RESTART:
			_draw_confirm(vp, tr("RESTART RUN?"),
				tr("This run is still live — restarting abandons it."), tr("Restart Run"))
		SETTINGS:
			_draw_settings(vp)

# Drawn-panel chrome shared by all panes: bg + border + emissive top rule
# (results.gd's look, keyed on accent instead of danger).
func _panel(px: float, py: float, pw: float, ph: float, rule: Color) -> void:
	var panel := Rect2(Vector2(px, py), Vector2(pw, ph))
	draw_rect(panel, ArtTheme.ui("panel_bg"))
	draw_rect(panel, ArtTheme.ui("panel_border"), false, 2.0)
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, 3)), rule * 1.4)   # HDR: blooms

# Centered title on the panel.
func _title(cx: float, y: float, text: String, col: Color) -> void:
	var tw := _font_head.get_string_size(text, HORIZONTAL_ALIGNMENT_LEFT, -1, 30).x
	draw_string(_font_head, Vector2(cx - tw * 0.5, y), text,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 30, col * 1.4)

# One menu-style button row; registers its hit rect. Matches results.gd's
# REDEPLOY button (hover lighten, accent border, selected = full border).
func _button(i: int, r: Rect2, label: String, danger: bool = false) -> void:
	_row_rects[i] = r
	var mpos := get_viewport().get_mouse_position()
	var hovered := r.has_point(mpos)
	var acc: Color = ArtTheme.ui("danger") if danger else ArtTheme.ui("accent")
	var base := acc.darkened(0.7)
	draw_rect(r, base.lightened(0.08) if hovered or i == _sel else base)
	var border := acc
	border.a = 0.9 if i == _sel else 0.4
	draw_rect(r, border, false, 2.0 if i == _sel else 1.0)
	var lw := _font_head.get_string_size(label, HORIZONTAL_ALIGNMENT_LEFT, -1, 18).x
	draw_string(_font_head, r.position + Vector2(r.size.x * 0.5 - lw * 0.5, r.size.y * 0.5 + 6),
		label, HORIZONTAL_ALIGNMENT_LEFT, -1, 18, acc.lightened(0.3))

# Centered dim hint line at the panel foot.
func _hint(cx: float, y: float, text: String) -> void:
	var hw := _font.get_string_size(text, HORIZONTAL_ALIGNMENT_LEFT, -1, 13).x
	draw_string(_font, Vector2(cx - hw * 0.5, y), text,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 13, ArtTheme.ui("text_dim"))

func _draw_menu(vp: Vector2) -> void:
	var pw := 360.0
	var ph := 356.0
	var px := vp.x * 0.5 - pw * 0.5
	var py := vp.y * 0.5 - ph * 0.5
	_panel(px, py, pw, ph, ArtTheme.ui("accent"))
	var cx := vp.x * 0.5
	_title(cx, py + 46.0, tr("PAUSED"), ArtTheme.ui("header"))
	var bw := 250.0
	var bh := 42.0
	var by := py + 80.0
	_button(0, Rect2(cx - bw * 0.5, by, bw, bh), tr("Resume"))
	_button(1, Rect2(cx - bw * 0.5, by + 56.0, bw, bh), tr("Restart Run"))
	_button(2, Rect2(cx - bw * 0.5, by + 112.0, bw, bh), tr("Settings"))
	_button(3, Rect2(cx - bw * 0.5, by + 168.0, bw, bh), tr("Quit to Menu"), true)
	_hint(cx, py + ph - 14.0, tr("[↑↓] select   [Enter] confirm   [Esc] resume"))

# Shared confirm-gate pane (Quit / Restart): title + one body line + Keep
# Playing vs the destructive action (danger-styled).
func _draw_confirm(vp: Vector2, title: String, body: String, go_label: String) -> void:
	var pw := 460.0
	var ph := 236.0
	var px := vp.x * 0.5 - pw * 0.5
	var py := vp.y * 0.5 - ph * 0.5
	_panel(px, py, pw, ph, ArtTheme.ui("danger"))
	var cx := vp.x * 0.5
	_title(cx, py + 46.0, title, ArtTheme.ui("danger"))
	var bw2 := _font.get_string_size(body, HORIZONTAL_ALIGNMENT_LEFT, -1, 15).x
	draw_string(_font, Vector2(cx - bw2 * 0.5, py + 78.0), body,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 15, ArtTheme.ui("text"))
	var bw := 250.0
	var bh := 42.0
	_button(0, Rect2(cx - bw * 0.5, py + 100.0, bw, bh), tr("Keep Playing"))
	_button(1, Rect2(cx - bw * 0.5, py + 156.0, bw, bh), go_label, true)
	_hint(cx, py + ph - 14.0, tr("[↑↓] select   [Enter] confirm   [Esc] back"))

func _draw_settings(vp: Vector2) -> void:
	var pw := 560.0
	var ph := 444.0
	var px := vp.x * 0.5 - pw * 0.5
	var py := vp.y * 0.5 - ph * 0.5
	_panel(px, py, pw, ph, ArtTheme.ui("header"))
	var cx := vp.x * 0.5
	_title(cx, py + 46.0, tr("SETTINGS"), ArtTheme.ui("header"))
	var lx := px + 36.0
	var rx := px + pw - 36.0
	var ry := py + 96.0
	var rstep := 40.0
	_settings_row(ROW_MASTER, lx, rx, ry, tr("Master Volume"))
	ry += rstep
	_settings_row(ROW_SFX, lx, rx, ry, tr("SFX Volume"))
	ry += rstep
	_settings_row(ROW_MUSIC, lx, rx, ry, tr("Music Volume"))
	ry += rstep
	_settings_row(ROW_MUTE, lx, rx, ry, tr("Mute"))
	ry += rstep
	_settings_row(ROW_LANG, lx, rx, ry, tr("Language"))
	ry += rstep
	_settings_row(ROW_SHAKE, lx, rx, ry, tr("Screen Shake"))
	ry += rstep
	_settings_row(ROW_SPEED, lx, rx, ry, tr("Game Speed"))
	ry += rstep + 8.0
	var bw := 200.0
	_button(ROW_BACK, Rect2(cx - bw * 0.5, ry - 14.0, bw, 38.0), tr("Back"))
	_hint(cx, py + ph - 14.0, tr("[↑↓] select   [←→] adjust   [Enter] apply   [Esc] back"))

# One settings row: label left, control right (slider bar for volume rows, a
# value pill for toggles). Registers row + slider hit rects.
func _settings_row(row: int, lx: float, rx: float, y: float, label: String) -> void:
	var selected := _sel == row
	var r := Rect2(lx - 12.0, y - 22.0, rx - lx + 24.0, 32.0)
	_row_rects[row] = r
	if selected:
		var hl := ArtTheme.ui("accent")
		hl.a = 0.12
		draw_rect(r, hl)
		draw_string(_font_head, Vector2(lx - 10.0, y), "▸",
			HORIZONTAL_ALIGNMENT_LEFT, -1, 16, ArtTheme.ui("accent"))
	draw_string(_font, Vector2(lx + 8.0, y), label, HORIZONTAL_ALIGNMENT_LEFT, -1, 16,
		ArtTheme.ui("text") if selected else ArtTheme.ui("text_dim"))
	if row <= ROW_MUSIC:
		# Volume slider: bar + fill + right-aligned percentage.
		var bar := Rect2(rx - 250.0, y - 12.0, 190.0, 12.0)
		_slider_rects[row] = bar
		var v := _vol(row)
		draw_rect(bar, ArtTheme.ui("panel_border").darkened(0.3))
		if v > 0.0:
			draw_rect(Rect2(bar.position, Vector2(bar.size.x * v, bar.size.y)),
				ArtTheme.ui("accent") if selected else ArtTheme.ui("accent_dim"))
		draw_rect(bar, ArtTheme.ui("panel_border"), false, 1.0)
		var pct := "%d%%" % int(round(v * 100.0))
		var pctw := _font_head.get_string_size(pct, HORIZONTAL_ALIGNMENT_LEFT, -1, 15).x
		draw_string(_font_head, Vector2(rx - pctw, y), pct,
			HORIZONTAL_ALIGNMENT_LEFT, -1, 15, ArtTheme.ui("text"))
	else:
		var value := ""
		var on := false
		match row:
			ROW_MUTE:
				on = Audio.is_muted()
				value = tr("On") if on else tr("Off")
			ROW_LANG:
				on = true
				value = _lang_label()
			ROW_SHAKE:
				on = Profile.screen_shake()
				value = tr("On") if on else tr("Off")
			ROW_SPEED:
				# e.g. "Fast ×1.5" — accent-lit whenever off the Normal default.
				var sp: int = Profile.game_speed()
				on = sp > 0
				value = "%s ×%s" % [tr(SPEED_NAMES[sp]), SPEED_MULTS[sp]]
		var vw := _font_head.get_string_size(value, HORIZONTAL_ALIGNMENT_LEFT, -1, 15).x
		draw_string(_font_head, Vector2(rx - vw, y), value, HORIZONTAL_ALIGNMENT_LEFT, -1, 15,
			ArtTheme.ui("accent") if on else ArtTheme.ui("text_dim"))
