# Results — the centered run-summary panel shown on tank death (Main.tscn, UI
# layer; main.gd toggles visibility with the shop). Reads the (still-valid) sim
# through SimView for round/stats/arsenal and Profile.last_unlocks for this
# run's achievements, and offers Redeploy (Enter/Space/click) or Menu (Esc).
# Cosmetic only; it never touches the sim. Sets `redeploy_rect` for main.gd's
# click handler (via redeploy_hit()).
extends Node2D

var view: SimView = null      # wired by main.gd

# Results-panel "Redeploy" hit-target, recomputed by _draw each frame while
# dead and consulted by main.gd's click handler.
var redeploy_rect := Rect2()

var _font: Font = null
var _font_head: Font = null

func _ready() -> void:
	_font = ArtTheme.ui_font(false)
	if _font == null:
		_font = ThemeDB.fallback_font
	_font_head = ArtTheme.ui_font(true)
	if _font_head == null:
		_font_head = _font

func _process(_delta: float) -> void:
	queue_redraw()

func redeploy_hit(pos: Vector2) -> bool:
	return redeploy_rect.has_point(pos)

func _draw() -> void:
	if view == null:
		return
	var vp: Vector2 = get_viewport_rect().size
	var font: Font = _font
	var head: Font = _font_head

	# Dim the arena behind the panel so the summary reads cleanly.
	draw_rect(Rect2(Vector2.ZERO, vp), Color(0.02, 0.02, 0.04, 0.55))

	var rec: Dictionary = view.stats_record()
	var dmg: int = rec["damage"]
	var gold: int = rec["gold"]
	var bought: int = rec["weapons_bought"]
	var rnd: int = view.round_num()
	var owned: PackedStringArray = view.arsenal_lines()

	# Panel geometry, centered.
	var pw := 560.0
	var unlock_n: int = Profile.last_unlocks.size()
	var ph := 372.0 + maxf(float(unlock_n), 0.0) * 22.0
	var px := vp.x * 0.5 - pw * 0.5
	var py := vp.y * 0.5 - ph * 0.5
	var panel := Rect2(Vector2(px, py), Vector2(pw, ph))
	draw_rect(panel, ArtTheme.ui("panel_bg"))
	draw_rect(panel, ArtTheme.ui("danger").darkened(0.5), false, 2.0)
	# Emissive top rule so it blooms under glow (boost danger to HDR for bloom).
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, 3)), ArtTheme.ui("danger") * 1.4)

	var cx := vp.x * 0.5
	# Title.
	var title := tr("TANK DESTROYED")
	var tw := head.get_string_size(title, HORIZONTAL_ALIGNMENT_LEFT, -1, 34).x
	draw_string(head, Vector2(cx - tw * 0.5, py + 48), title, HORIZONTAL_ALIGNMENT_LEFT, -1, 34, ArtTheme.ui("danger") * 1.4)
	var sub := tr("Run summary")
	var sw := font.get_string_size(sub, HORIZONTAL_ALIGNMENT_LEFT, -1, 15).x
	draw_string(font, Vector2(cx - sw * 0.5, py + 72), sub, HORIZONTAL_ALIGNMENT_LEFT, -1, 15, ArtTheme.ui("text_dim"))

	# Stat rows (label left, value right).
	var lx := px + 36.0
	var rx := px + pw - 36.0
	var ry := py + 110.0
	var rstep := 30.0
	_draw_stat_row(font, head, lx, rx, ry, tr("Round reached"), "%d" % rnd, ArtTheme.ui("header"))
	ry += rstep
	_draw_stat_row(font, head, lx, rx, ry, tr("Damage dealt"), "%d" % dmg, ArtTheme.ui("accent"))
	ry += rstep
	_draw_stat_row(font, head, lx, rx, ry, tr("Gold earned"), "%d" % gold, ArtTheme.ui("coin"))
	ry += rstep
	_draw_stat_row(font, head, lx, rx, ry, tr("Weapons bought"), "%d" % bought, ArtTheme.ui("text"))
	ry += rstep + 4.0

	# Owned arsenal, condensed onto one wrapped line. `owned` entries are
	# Rust-sourced "Name xN" lines; translate each name (preserving the xN tail).
	draw_string(head, Vector2(lx, ry), tr("ARSENAL"), HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("header"))
	ry += 20.0
	var ars := "  ·  ".join(_tr_arsenal(owned)) if owned.size() > 0 else tr("— nothing acquired —")
	draw_string(font, Vector2(lx, ry), ars, HORIZONTAL_ALIGNMENT_LEFT, pw - 72.0, 13, ArtTheme.ui("text"))
	ry += 30.0

	# Achievements unlocked this run.
	draw_string(head, Vector2(lx, ry), tr("ACHIEVEMENTS UNLOCKED"), HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("accent"))
	ry += 20.0
	if unlock_n == 0:
		draw_string(font, Vector2(lx, ry), tr("— none this run —"), HORIZONTAL_ALIGNMENT_LEFT, pw - 72.0, 13, ArtTheme.ui("text_dim"))
		ry += 22.0
	else:
		for id in Profile.last_unlocks:
			var nm: String = Profile.ach_def(id).get("name", id)
			draw_string(font, Vector2(lx, ry), "★ " + tr(nm), HORIZONTAL_ALIGNMENT_LEFT, pw - 72.0, 14, ArtTheme.ui("accent"))
			ry += 22.0

	# Redeploy button + Esc prompt.
	var btn_w := 220.0
	var btn_h := 40.0
	redeploy_rect = Rect2(cx - btn_w * 0.5, py + ph - 64.0, btn_w, btn_h)
	var mpos := get_viewport().get_mouse_position()
	var hovered := redeploy_rect.has_point(mpos)
	var btn_base := ArtTheme.ui("accent").darkened(0.7)
	var bbg := btn_base.lightened(0.08) if hovered else btn_base
	draw_rect(redeploy_rect, bbg)
	var btn_border := ArtTheme.ui("accent")
	btn_border.a = 0.7
	draw_rect(redeploy_rect, btn_border, false, 1.5)
	var blabel := tr("REDEPLOY")
	var blw := head.get_string_size(blabel, HORIZONTAL_ALIGNMENT_LEFT, -1, 18).x
	draw_string(head, redeploy_rect.position + Vector2(btn_w * 0.5 - blw * 0.5, 27), blabel, HORIZONTAL_ALIGNMENT_LEFT, -1, 18, ArtTheme.ui("accent").lightened(0.3))
	var prompt := tr("[Enter] Redeploy   ·   [Esc] Menu")
	var pwid := font.get_string_size(prompt, HORIZONTAL_ALIGNMENT_LEFT, -1, 14).x
	draw_string(font, Vector2(cx - pwid * 0.5, py + ph - 12.0), prompt, HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("text_dim"))

# One label/value row for the results panel.
func _draw_stat_row(font: Font, head: Font, lx: float, rx: float, y: float, label: String, value: String, vcol: Color) -> void:
	draw_string(font, Vector2(lx, y), label, HORIZONTAL_ALIGNMENT_LEFT, -1, 16, ArtTheme.ui("text_dim"))
	var vw := head.get_string_size(value, HORIZONTAL_ALIGNMENT_LEFT, -1, 18).x
	draw_string(head, Vector2(rx - vw, y + 1), value, HORIZONTAL_ALIGNMENT_LEFT, -1, 18, vcol)

# Translate each "Name xN" arsenal line: tr() the name, keep the " xN" tail
# (scaffolding). Used by the results-panel arsenal summary.
func _tr_arsenal(lines: PackedStringArray) -> PackedStringArray:
	var out: PackedStringArray = []
	for line in lines:
		var sp := line.rfind(" x")
		if sp > 0:
			out.append(tr(line.substr(0, sp)) + line.substr(sp))
		else:
			out.append(tr(line))
	return out
