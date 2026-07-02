# Hud — the single-arena top bar (HP / gold / income / round / tick) and the
# arsenal panel (Main.tscn, UI layer). Immediate-mode _draw on its own
# CanvasItem so HUD redraws are scoped away from the arena. Read-only: every
# value comes through the SimView wrapper.
extends Node2D

var view: SimView = null      # wired by main.gd

var _font: Font = null
var _font_head: Font = null
var _panel_tex: Texture2D
var _heart_tex: Texture2D
var _coin_tex: Texture2D

func _ready() -> void:
	# Body + header faces with the Noto Sans SC fallback chained in (CJK-safe).
	_font = ArtTheme.ui_font(false)
	if _font == null:
		_font = ThemeDB.fallback_font
	_font_head = ArtTheme.ui_font(true)
	if _font_head == null:
		_font_head = _font
	reload_theme()

func reload_theme() -> void:
	_panel_tex = ArtTheme.tex("ui/panel.svg")
	_heart_tex = ArtTheme.tex("ui/heart.svg")
	_coin_tex = ArtTheme.tex("ui/coin.svg")

func _process(_delta: float) -> void:
	queue_redraw()

func _blit(tx: Texture2D, center: Vector2, size: float, mod := Color.WHITE) -> void:
	draw_texture_rect(tx, Rect2(center - Vector2(size, size) * 0.5, Vector2(size, size)), false, mod)

func _draw() -> void:
	if view == null:
		return
	var vp: Vector2 = get_viewport_rect().size
	var font: Font = _font
	var head: Font = _font_head
	draw_texture_rect(_panel_tex, Rect2(Vector2(12, 10), Vector2(330, 92)), false)
	_blit(_heart_tex, Vector2(40, 38), 30)
	draw_string(head, Vector2(60, 45), "%d / %d" % [maxi(view.tank_hp(), 0), view.tank_max_hp()],
		HORIZONTAL_ALIGNMENT_LEFT, -1, 19, ArtTheme.ui("hp"))
	_blit(_coin_tex, Vector2(40, 74), 28)
	draw_string(head, Vector2(60, 81), "%d" % view.gold(), HORIZONTAL_ALIGNMENT_LEFT, -1, 19, ArtTheme.ui("coin"))
	var gold_w := head.get_string_size("%d" % view.gold(), HORIZONTAL_ALIGNMENT_LEFT, -1, 19).x
	draw_string(font, Vector2(60 + gold_w + 8, 81), "+%d/t" % view.income(), HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("accent_dim"))
	draw_string(head, Vector2(212, 45), tr("ROUND %d") % view.round_num(), HORIZONTAL_ALIGNMENT_LEFT, -1, 16, ArtTheme.ui("header"))
	draw_string(font, Vector2(212, 81), tr("tick %d") % view.tick(), HORIZONTAL_ALIGNMENT_LEFT, -1, 13, ArtTheme.ui("text_dim"))
	_draw_arsenal(font, head, vp)

# C3 — Arsenal panel: a framed list (top-right) of owned weapons/mods with a
# header and right-aligned counts pulled from `view.arsenal_lines()` ("Name xN").
func _draw_arsenal(font: Font, head: Font, vp: Vector2) -> void:
	var lines: PackedStringArray = view.arsenal_lines()
	var pw := 234.0
	var px := vp.x - pw - 12.0
	var py := 12.0
	var row_h := 19.0
	var head_h := 26.0
	var ph := head_h + 8.0 + maxf(float(lines.size()), 1.0) * row_h + 6.0
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, ph)), ArtTheme.ui("panel_bg"))
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, ph)), ArtTheme.ui("panel_border"), false, 1.0)
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, head_h)), ArtTheme.ui("panel_border").darkened(0.4))
	draw_string(head, Vector2(px + 10, py + 18), tr("ARSENAL"), HORIZONTAL_ALIGNMENT_LEFT, -1, 15, ArtTheme.ui("header"))
	var ct := "%d" % lines.size()
	var ctw := font.get_string_size(ct, HORIZONTAL_ALIGNMENT_LEFT, -1, 14).x
	draw_string(font, Vector2(px + pw - ctw - 10, py + 18), ct, HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("text_dim"))
	var ay := py + head_h + 8.0
	if lines.is_empty():
		draw_string(font, Vector2(px + 10, ay + 12), tr("— nothing yet —"), HORIZONTAL_ALIGNMENT_LEFT, pw - 20, 13, ArtTheme.ui("text_dim"))
		return
	for line in lines:
		# Split "Name xN" so the count can be right-aligned for legibility.
		# `nm` is a Rust-sourced weapon/mod name — translate it; "xN" is scaffolding.
		var nm := line
		var cnt := ""
		var sp := line.rfind(" x")
		if sp > 0:
			nm = line.substr(0, sp)
			cnt = line.substr(sp + 1)   # "xN"
		draw_string(font, Vector2(px + 10, ay + 13), tr(nm), HORIZONTAL_ALIGNMENT_LEFT, pw - 56, 14, ArtTheme.ui("text"))
		if cnt != "":
			var cw := font.get_string_size(cnt, HORIZONTAL_ALIGNMENT_LEFT, -1, 14).x
			draw_string(font, Vector2(px + pw - cw - 10, ay + 13), cnt, HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("accent_dim"))
		ay += row_h
