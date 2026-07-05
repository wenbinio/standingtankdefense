# Black Market picker overlay (Main.tscn UiLayer; single-arena only). Shown
# while the sim holds a Black Market pick (StSim.black_market_pending() — "Buy
# 1 Uncommon Weapon or Spikes Damage Upgrade of your choosing. The Black Market
# lasts until a choice is made."). Immediate-mode drawn in the shop's visual
# language: two columns — Uncommon WEAPONS and Uncommon SPIKES UPGRADES — with
# keyboard nav (ui_nav_* / ui_confirm) and mouse clicks.
#
# NOT a pause: the sim keeps stepping behind the overlay (the held pick has no
# deadline). Esc / [B] / an outside click dismisses to a passive badge above
# the shop bar so the player can keep shopping; clicking the badge (or [B])
# reopens the picker. The overlay stays armed until the sim redeems the pick.
# Esc therefore still reaches the pause menu: the first press dismisses to the
# badge, the next one (with the overlay closed) pauses as usual.
#
# Owner contract (main.gd):
#   arm(w_idx, w_names, u_idx, u_names)   pending-edge: store lists ONCE + open
#   disarm()                              redeem-edge (or redeploy/death): gone
#   pick_failed()                         queued pick no-oped -> back to badge
#   is_open() / is_held()                 input routing / badge state
#   handle_input(e)                       forward every event while is_open()
#   badge_hit(pos) + reopen()             badge click routing (owner click pass)
#   on_pick: Callable(code, slot) -> bool queue intent 4/5 through main's FIFO;
#                                         false = FIFO full, overlay stays open
#
# READ-ONLY towards the sim: the choice lists arrive prebuilt from main.gd
# (SimView) on the pending edge, and the pick goes back as an intent through
# main's FIFO — this node never steps or queries the sim. No per-frame
# allocation in steady state: lists + hit-rect arrays are sized once in arm();
# _draw only writes into them (Rect2 is a value type).
extends Node2D

# Overlay states.
const CLOSED := 0     # no pick held by the sim
const OPEN := 1       # modal picker
const HELD := 2       # pick still held, overlay dismissed -> passive badge
const WAIT := 3       # pick intent queued -> draw nothing until the redeem edge

# Pick intent codes (mirror StSim.step's input table: 4 weapon · 5 upgrade).
const CODE_WEAPON := 4
const CODE_UPGRADE := 5

# Shop bar height (mirrors shop.gd's bar_h) — the badge parks just above it.
const SHOP_BAR_H := 162.0
const ROW_H := 34.0
const COL_W := 300.0
const PAD := 18.0

# Uncommon rarity green (shop.gd's _rarity_color(1)) — the eligible set is all
# Uncommon, so the overlay's chrome keys off it.
const UNCOMMON := Color(0.40, 0.80, 0.45)

# main.gd's intent hook: called with (code, slot = catalog index) on a pick.
var on_pick: Callable = Callable()

var _mode := CLOSED
var _col := 0                 # keyboard cursor: 0 weapons, 1 upgrades
var _row := 0
# Choice lists (parallel catalog-index / display-name pairs), set once per arm().
var _w_idx := PackedInt64Array()
var _w_names := PackedStringArray()
var _u_idx := PackedInt64Array()
var _u_names := PackedStringArray()
# Hit rects, recomputed by _draw and consumed by handle_input clicks; sized
# once per arm().
var _w_rects: Array[Rect2] = []
var _u_rects: Array[Rect2] = []
var _panel_rect := Rect2()
var badge_rect := Rect2()

var _font: Font = null
var _font_head: Font = null

func _ready() -> void:
	_font = ArtTheme.ui_font(false)
	if _font == null:
		_font = ThemeDB.fallback_font
	_font_head = ArtTheme.ui_font(true)
	if _font_head == null:
		_font_head = _font
	visible = false

func _process(_delta: float) -> void:
	if _mode == OPEN or _mode == HELD:
		queue_redraw()      # hover/pulse feedback; pause_menu.gd's pattern

# --- owner API -----------------------------------------------------------------

func is_open() -> bool:
	return _mode == OPEN

func is_held() -> bool:
	return _mode == HELD

# Pending-edge: adopt the (already-filtered, eligible-only) choice lists and
# open. Lists are stored as-is — built exactly once per pending edge.
func arm(w_idx: PackedInt64Array, w_names: PackedStringArray,
		u_idx: PackedInt64Array, u_names: PackedStringArray) -> void:
	_w_idx = w_idx
	_w_names = w_names
	_u_idx = u_idx
	_u_names = u_names
	_w_rects.resize(w_names.size())
	_u_rects.resize(u_names.size())
	_col = 0
	_row = 0
	_set_mode(OPEN)

# Redeem-edge / redeploy / death: everything gone (badge included).
func disarm() -> void:
	badge_rect = Rect2()
	_set_mode(CLOSED)

# Esc/[B]/outside click while open: down to the passive badge.
func dismiss() -> void:
	if _mode == OPEN:
		_set_mode(HELD)

# Badge click / [B] while held: back up to the picker.
func reopen() -> void:
	if _mode == HELD:
		Audio.play(&"ui_move")
		_set_mode(OPEN)

# The queued pick was no-oped by the sim (pending survived its tick). Should
# not happen — the lists only ever contain eligible indices — but the overlay
# must not stay invisible forever if it does.
func pick_failed() -> void:
	if _mode == WAIT:
		_set_mode(HELD)

func badge_hit(pos: Vector2) -> bool:
	return _mode == HELD and badge_rect.has_point(pos)

# Route one InputEvent here while is_open() (mirrors pause_menu.gd's contract:
# the overlay never reads global input itself).
func handle_input(e: InputEvent) -> void:
	if e is InputEventMouseButton:
		var mb := e as InputEventMouseButton
		if mb.pressed and mb.button_index == MOUSE_BUTTON_LEFT:
			_click(mb.position)
		return
	if not (e is InputEventKey or e is InputEventJoypadButton):
		return
	if e.is_action_pressed(&"ui_back") or e.is_action_pressed(&"ui_black_market"):
		dismiss()
	elif e.is_action_pressed(&"ui_nav_left"):
		_set_col(0)
	elif e.is_action_pressed(&"ui_nav_right"):
		_set_col(1)
	elif e.is_action_pressed(&"ui_nav_up"):
		_move(-1)
	elif e.is_action_pressed(&"ui_nav_down"):
		_move(1)
	elif e.is_action_pressed(&"ui_confirm"):
		_pick()

# --- cursor / pick ---------------------------------------------------------------

func _set_mode(m: int) -> void:
	_mode = m
	visible = m == OPEN or m == HELD
	queue_redraw()

func _col_size(c: int) -> int:
	return _w_names.size() if c == 0 else _u_names.size()

func _move(dir: int) -> void:
	var n := _col_size(_col)
	if n <= 0:
		return
	_row = posmod(_row + dir, n)
	Audio.play(&"ui_move")
	queue_redraw()

func _set_col(c: int) -> void:
	if c == _col or _col_size(c) <= 0:
		return
	_col = c
	_row = clampi(_row, 0, _col_size(c) - 1)
	Audio.play(&"ui_move")
	queue_redraw()

# Submit the current selection through main.gd's intent FIFO: code 4 (weapon)
# or 5 (upgrade), slot = the CATALOG index from black_market_choices. On
# success the overlay goes quiet (WAIT) until the redeem edge disarms it.
func _pick() -> void:
	var idx_arr := _w_idx if _col == 0 else _u_idx
	if _row < 0 or _row >= idx_arr.size():
		return
	var code := CODE_WEAPON if _col == 0 else CODE_UPGRADE
	if on_pick.is_valid() and bool(on_pick.call(code, int(idx_arr[_row]))):
		_set_mode(WAIT)

func _click(pos: Vector2) -> void:
	for i in _w_rects.size():
		if _w_rects[i].has_point(pos):
			_col = 0
			_row = i
			_pick()
			return
	for i in _u_rects.size():
		if _u_rects[i].has_point(pos):
			_col = 1
			_row = i
			_pick()
			return
	# Click outside the panel: dismiss to the badge so the shop below becomes
	# clickable again on the very next press.
	if not _panel_rect.has_point(pos):
		dismiss()

# --- drawing -----------------------------------------------------------------------

func _draw() -> void:
	match _mode:
		OPEN:
			_draw_panel()
		HELD:
			_draw_badge()

func _draw_panel() -> void:
	var vp: Vector2 = get_viewport_rect().size
	badge_rect = Rect2()
	var rows_max: int = maxi(_w_names.size(), _u_names.size())
	var pw := PAD * 3.0 + COL_W * 2.0
	var ph := 124.0 + float(rows_max) * ROW_H + 40.0
	var px := vp.x * 0.5 - pw * 0.5
	# Center in the space above the shop bar (the fight + shop stay visible and
	# LIVE behind the panel — this is a picker, not a pause).
	var py := maxf((vp.y - SHOP_BAR_H - ph) * 0.5, 8.0)
	_panel_rect = Rect2(px, py, pw, ph)
	# Chrome: the shop/pause panel language — bg, border, emissive top rule in
	# the Uncommon green so it blooms like the other panels' accent rules.
	draw_rect(_panel_rect, ArtTheme.ui("panel_bg"))
	draw_rect(_panel_rect, ArtTheme.ui("panel_border"), false, 2.0)
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, 3)), UNCOMMON * 1.4)
	var cx := px + pw * 0.5
	var title := tr("BLACK MARKET")
	var tw := _font_head.get_string_size(title, HORIZONTAL_ALIGNMENT_LEFT, -1, 26).x
	draw_string(_font_head, Vector2(cx - tw * 0.5, py + 38.0), title,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 26, UNCOMMON * 1.35)
	var sub := tr("Buy 1 Uncommon weapon or Spikes upgrade of your choosing — free."
		+ " The market waits until you choose.")
	var sw := _font.get_string_size(sub, HORIZONTAL_ALIGNMENT_LEFT, -1, 13).x
	draw_string(_font, Vector2(cx - minf(sw, pw - 24.0) * 0.5, py + 60.0), sub,
		HORIZONTAL_ALIGNMENT_LEFT, pw - 24.0, 13, ArtTheme.ui("text_dim"))
	# Column headers in the shop's category pip colors (weapon blue, spike red).
	var wx := px + PAD
	var ux := px + PAD * 2.0 + COL_W
	var hy := py + 92.0
	draw_string(_font_head, Vector2(wx, hy), tr("UNCOMMON WEAPONS"),
		HORIZONTAL_ALIGNMENT_LEFT, -1, 13, Color(0.46, 0.62, 0.92))
	draw_string(_font_head, Vector2(ux, hy), tr("SPIKES UPGRADES"),
		HORIZONTAL_ALIGNMENT_LEFT, -1, 13, Color(0.86, 0.46, 0.4))
	var top := hy + 10.0
	var mpos := get_viewport().get_mouse_position()
	_draw_column(0, wx, top, _w_names, _w_rects, mpos)
	_draw_column(1, ux, top, _u_names, _u_rects, mpos)
	var hint := tr("[←→] column   [↑↓] select   [Enter] pick   [Esc] later")
	var hw := _font.get_string_size(hint, HORIZONTAL_ALIGNMENT_LEFT, -1, 13).x
	draw_string(_font, Vector2(cx - hw * 0.5, py + ph - 14.0), hint,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 13, ArtTheme.ui("text_dim"))

# One choice column: hover/selected feedback in the shop-card style, with the
# Uncommon rarity strip on each entry's left edge. Writes this column's hit
# rects in place (preallocated by arm()).
func _draw_column(col: int, x: float, top: float, names: PackedStringArray,
		rects: Array[Rect2], mpos: Vector2) -> void:
	for i in names.size():
		var r := Rect2(x, top + float(i) * ROW_H, COL_W, ROW_H - 6.0)
		rects[i] = r
		var selected := _col == col and _row == i
		var hovered := r.has_point(mpos)
		var bg := ArtTheme.ui("panel_bg").lightened(0.06)
		if selected:
			bg = bg.lightened(0.08)
		elif hovered:
			bg = bg.lightened(0.05)
		draw_rect(r, bg)
		draw_rect(Rect2(r.position, Vector2(3, r.size.y)), UNCOMMON)
		if selected or hovered:
			var bc := UNCOMMON
			bc.a = 0.9 if selected else 0.4
			draw_rect(r, bc, false, 2.0 if selected else 1.0)
		# Rust-sourced catalog name: translate at the draw boundary (the names
		# are all existing shop-name keys in the translation table).
		draw_string(_font, r.position + Vector2(12.0, r.size.y * 0.5 + 5.0), tr(names[i]),
			HORIZONTAL_ALIGNMENT_LEFT, COL_W - 20.0, 14,
			ArtTheme.ui("text") if selected or hovered else ArtTheme.ui("text_dim"))

# Passive shop-area badge while the pick is held but the overlay is dismissed:
# parked just above the shop bar's right edge, sized to the rendered string
# (zh-CN safe), with the shop Clear button's ready-pulse border. Clickable —
# main.gd routes clicks here via badge_hit(); [B] also reopens.
func _draw_badge() -> void:
	var vp: Vector2 = get_viewport_rect().size
	var label := tr("★ BLACK MARKET — pick pending  [B]")
	var lw := _font.get_string_size(label, HORIZONTAL_ALIGNMENT_LEFT, -1, 13).x
	badge_rect = Rect2(vp.x - lw - 24.0 - 12.0, vp.y - SHOP_BAR_H - 38.0, lw + 24.0, 28.0)
	var hovered := badge_rect.has_point(get_viewport().get_mouse_position())
	var bg: Color = ArtTheme.ui("panel_bg")
	draw_rect(badge_rect, bg.lightened(0.08) if hovered else bg)
	var bc := UNCOMMON
	bc.a = 0.45 + 0.30 * (0.5 + 0.5 * sin(Time.get_ticks_msec() * 0.004))
	draw_rect(badge_rect, bc, false, 1.5)
	draw_string(_font, badge_rect.position + Vector2(12.0, 19.0), label,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 13, UNCOMMON.lightened(0.15))
