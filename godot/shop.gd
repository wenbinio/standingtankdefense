# Shop — the single-arena bottom shop bar (Main.tscn, UI layer): every offer as
# a clickable card, the reroll/clear buttons, hover/press feedback and the
# hover tooltip. Immediate-mode _draw on its own CanvasItem. Read-only: offers
# come through SimView.shop_offers(); the actual buy/reroll/clear INTENTS are
# queued by main.gd (this module only reports hit-tests and draws state).
extends Node2D

# Sim tick rate (mirrors sim::TICK_HZ) — ticks → seconds for the Clear cooldown
# readout. Render-only arithmetic.
const TICK_HZ := 30

var view: SimView = null      # wired by main.gd
# The intent consumed THIS tick (exactly what sim.step() received), pushed by
# main.gd for the pressed-state draw feedback. 0 = none.
var pending_code := 0
var pending_slot := 0

# interactive hit-targets (recomputed each draw; consumed by main.gd's click
# handler via card_at / reroll_hit / clear_hit)
var shop_rects: Array[Rect2] = []
var reroll_rect := Rect2()
var clear_rect := Rect2()

var _font: Font = null
var _font_head: Font = null
var _weapon_tex: Texture2D
var _mod_tex: Texture2D
var frame_tex := []           # by rarity 0..3

func _ready() -> void:
	_font = ArtTheme.ui_font(false)
	if _font == null:
		_font = ThemeDB.fallback_font
	_font_head = ArtTheme.ui_font(true)
	if _font_head == null:
		_font_head = _font
	reload_theme()

func reload_theme() -> void:
	_weapon_tex = ArtTheme.tex("ui/icon_weapon.svg")
	_mod_tex = ArtTheme.tex("ui/icon_modifier.svg")
	frame_tex = [ArtTheme.tex("ui/frame_common.svg"), ArtTheme.tex("ui/frame_uncommon.svg"),
		ArtTheme.tex("ui/frame_rare.svg"), ArtTheme.tex("ui/frame_epic.svg")]

func _process(_delta: float) -> void:
	queue_redraw()

# --- hit-tests for main.gd's click routing ------------------------------------
# Index of the shop card under `pos`, or -1.
func card_at(pos: Vector2) -> int:
	for i in shop_rects.size():
		if shop_rects[i].has_point(pos):
			return i
	return -1

func reroll_hit(pos: Vector2) -> bool:
	return reroll_rect.has_point(pos)

func clear_hit(pos: Vector2) -> bool:
	return clear_rect.has_point(pos)

func _blit(tx: Texture2D, center: Vector2, size: float, mod := Color.WHITE) -> void:
	draw_texture_rect(tx, Rect2(center - Vector2(size, size) * 0.5, Vector2(size, size)), false, mod)

func _draw() -> void:
	if view == null:
		return
	var vp: Vector2 = get_viewport_rect().size
	var font: Font = _font
	var head: Font = _font_head
	var offers: Array = view.shop_offers()
	var gold: int = view.gold()
	var free_rr: int = view.free_rerolls()
	var rr_cost: int = view.reroll_cost()
	var mpos := get_viewport().get_mouse_position()

	var bar_h := 162.0
	var y0 := vp.y - bar_h
	draw_rect(Rect2(Vector2(0, y0), Vector2(vp.x, bar_h)), ArtTheme.ui("panel_bg"))
	draw_rect(Rect2(Vector2(0, y0), Vector2(vp.x, 2)), ArtTheme.ui("panel_border"))
	var shop_hdr := tr("SHOP")
	draw_string(head, Vector2(16, y0 + 20), shop_hdr, HORIZONTAL_ALIGNMENT_LEFT, -1, 15, ArtTheme.ui("header"))
	var sw := head.get_string_size(shop_hdr, HORIZONTAL_ALIGNMENT_LEFT, -1, 15).x
	# Help line + the [N] mute state indicator (render-only UX reflection).
	var audio_hint := tr("[N] sound: off") if Audio.is_muted() else tr("[N] sound: on")
	draw_string(font, Vector2(16 + sw + 10, y0 + 20),
		tr("click a card or press [1-8] to buy  ·  refreshes every round (30s)  ·  buy as many as you can afford") + "  ·  " + audio_hint,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 13, ArtTheme.ui("text_dim"))

	var n := offers.size()
	var split := 4              # slots 0-3 are weapons, 4-7 are economy/passives/spikes
	var btn_w := 156.0
	var left := 14.0
	var top := y0 + 54.0
	var ch := bar_h - 64.0
	var gap := 8.0
	var group_gap := 30.0
	var area := vp.x - left - btn_w - 16.0
	var cw := (area - group_gap - gap * (n - 2)) / maxf(n, 1)

	shop_rects.clear()
	var x := left
	for i in n:
		if i == split:
			# divider between the two groups
			draw_rect(Rect2(Vector2(x - group_gap * 0.5 - gap * 0.5, top - 18), Vector2(2, ch + 18)), ArtTheme.ui("panel_border"))
			x += group_gap - gap
		var r := Rect2(x, top, cw, ch)
		shop_rects.append(r)
		var offer: Dictionary = offers[i]
		var cost: int = offer["cost"]
		var rarity: int = offer["rarity"]
		var is_weapon: bool = offer["is_weapon"]
		var affordable: bool = offer["affordable"]
		var tip: String = offer["tip"]
		var fg := ArtTheme.ui("text") if affordable else ArtTheme.ui("text_dim").darkened(0.2)
		# C5 hover/press feedback (read-only — uses the same rects/inputs as the handlers).
		var hovered := r.has_point(mpos)
		var pressed := pending_code == 1 and pending_slot == i
		var bg := ArtTheme.ui("panel_bg").lightened(0.06) if affordable else ArtTheme.ui("panel_bg")
		bg.a = 1.0
		if pressed:
			bg = bg.lightened(0.10) if affordable else bg
		elif hovered and affordable:
			bg = bg.lightened(0.06)
		draw_rect(r, bg)
		var rc := _rarity_color(rarity)
		draw_rect(Rect2(r.position, Vector2(r.size.x, 3)), rc if affordable else rc.darkened(0.55))
		# category pip (top-right): weapon vs economy/passive/spike
		var cat := _shop_category(is_weapon, offer["name"], tip)
		draw_rect(Rect2(r.position + Vector2(cw - 12, 6), Vector2(7, 7)), cat[1] if affordable else (cat[1] as Color).darkened(0.5))
		var icx := r.position + Vector2(cw * 0.5, 26)
		_blit(frame_tex[clampi(rarity, 0, 3)], icx, 46, fg)
		_blit(_weapon_tex if is_weapon else _mod_tex, icx, 30, fg)
		draw_string(head, r.position + Vector2(6, 16), "%d" % (i + 1), HORIZONTAL_ALIGNMENT_LEFT, -1, 13, ArtTheme.ui("text_dim"))
		# Rust-sourced name/tip: translate at the draw boundary (category logic
		# above still keys off the English text).
		draw_string(head, r.position + Vector2(6, ch - 38), _fit(font, tr(offer["name"]), 13, cw - 12), HORIZONTAL_ALIGNMENT_LEFT, cw - 10, 13, fg)
		# C2 self-describing effect line (the mechanical tip), truncated to fit.
		var eff_col := ArtTheme.ui("text_dim") if affordable else ArtTheme.ui("text_dim").darkened(0.3)
		draw_string(font, r.position + Vector2(6, ch - 22), _fit(font, tr(tip), 11, cw - 12), HORIZONTAL_ALIGNMENT_LEFT, cw - 10, 11, eff_col)
		# cost (left) + rarity word (right), measured for clean right-alignment.
		draw_string(head, r.position + Vector2(6, ch - 5), "%dg" % cost, HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("coin") if affordable else ArtTheme.ui("accent_dim"))
		var cat_lbl := tr(cat[0])
		draw_string(font, r.position + Vector2(cw - 8 - font.get_string_size(cat_lbl, HORIZONTAL_ALIGNMENT_LEFT, -1, 10).x, ch - 6), cat_lbl, HORIZONTAL_ALIGNMENT_LEFT, -1, 10, (cat[1] as Color).lightened(0.1) if affordable else ArtTheme.ui("text_dim").darkened(0.2))
		if i == 0:
			draw_string(head, Vector2(x, top - 6), tr("WEAPONS"), HORIZONTAL_ALIGNMENT_LEFT, -1, 12, ArtTheme.ui("header"))
		elif i == split:
			draw_string(head, Vector2(x, top - 6), tr("ECONOMY · PASSIVES · SPIKES"), HORIZONTAL_ALIGNMENT_LEFT, -1, 12, ArtTheme.ui("accent"))
		x += cw + gap

	var bx := vp.x - btn_w - 8.0
	reroll_rect = Rect2(bx, top, btn_w, ch * 0.5 - 4.0)
	clear_rect = Rect2(bx, top + ch * 0.5 + 4.0, btn_w, ch * 0.5 - 4.0)
	var rr_ok := free_rr > 0 or gold >= rr_cost
	var rr_label := tr("[R] REROLL  free x%d") % free_rr if free_rr > 0 else tr("[R] REROLL  %dg") % rr_cost
	var rr_on := ArtTheme.ui("header").darkened(0.7)
	var rr_bg := _btn_bg(rr_on, ArtTheme.ui("panel_border").darkened(0.45), rr_ok, reroll_rect.has_point(mpos), pending_code == 2)
	draw_rect(reroll_rect, rr_bg)
	if reroll_rect.has_point(mpos) and rr_ok:
		var rr_outline := ArtTheme.ui("header")
		rr_outline.a = 0.5
		draw_rect(reroll_rect, rr_outline, false, 1.0)
	draw_string(head, reroll_rect.position + Vector2(12, reroll_rect.size.y * 0.5 + 5), rr_label,
		HORIZONTAL_ALIGNMENT_LEFT, btn_w - 18, 14, ArtTheme.ui("header") if rr_ok else ArtTheme.ui("text_dim"))
	# Clear button: two visual states driven by the sim's real cooldown (the sim
	# already ignores Clear while cooling — this makes the button LOOK disabled).
	var cd_left: int = view.clear_ready_in()
	var cd_total: int = maxi(view.clear_cooldown_total(), 1)
	var cl_ready := cd_left <= 0
	var cl_on := ArtTheme.ui("danger").darkened(0.7)
	var cl_off := ArtTheme.ui("panel_bg").lightened(0.03)
	var cl_bg := _btn_bg(cl_on, cl_off, cl_ready, clear_rect.has_point(mpos), pending_code == 3)
	draw_rect(clear_rect, cl_bg)
	if cl_ready:
		# Subtle ready pulse (render-only clock; never feeds the sim).
		var cl_pulse := ArtTheme.ui("danger")
		cl_pulse.a = 0.30 + 0.18 * sin(Time.get_ticks_msec() * 0.004)
		draw_rect(clear_rect, cl_pulse, false, 1.0)
		if clear_rect.has_point(mpos):
			var cl_outline := ArtTheme.ui("danger")
			cl_outline.a = 0.5
			draw_rect(clear_rect, cl_outline, false, 1.0)
		draw_string(head, clear_rect.position + Vector2(12, clear_rect.size.y * 0.5 + 5), tr("[Space] CLEAR"),
			HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("danger"))
	else:
		# Cooling: dimmed button, a left-to-right recharge fill, and the seconds
		# remaining — no hover affordance while it cannot fire.
		var frac := 1.0 - float(cd_left) / float(cd_total)
		var cl_fill := ArtTheme.ui("danger").darkened(0.55)
		cl_fill.a = 0.55
		draw_rect(Rect2(clear_rect.position, Vector2(clear_rect.size.x * frac, clear_rect.size.y)), cl_fill)
		draw_string(head, clear_rect.position + Vector2(12, clear_rect.size.y * 0.5 + 5), tr("[Space] CLEAR"),
			HORIZONTAL_ALIGNMENT_LEFT, btn_w - 52, 14, ArtTheme.ui("text_dim"))
		var cl_secs := "%ds" % ceili(float(cd_left) / float(TICK_HZ))
		var csw := head.get_string_size(cl_secs, HORIZONTAL_ALIGNMENT_LEFT, -1, 14).x
		draw_string(head, clear_rect.position + Vector2(clear_rect.size.x - csw - 10, clear_rect.size.y * 0.5 + 5),
			cl_secs, HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("danger").lightened(0.15))

	# Hover tooltip: flavor + mechanical tip for the card under the cursor.
	for i in shop_rects.size():
		if shop_rects[i].has_point(mpos):
			draw_rect(shop_rects[i], Color(1, 1, 1, 0.05))
			var hov := ArtTheme.ui("text")
			hov.a = 0.6
			draw_rect(shop_rects[i], hov, false, 1.0)
			var offer2: Dictionary = offers[i]
			_draw_tooltip(font, vp, shop_rects[i], offer2["name"], offer2["flavor"], offer2["tip"])
			break

# Button background tint for the three hover/press states (C5).
func _btn_bg(on: Color, off: Color, enabled: bool, hovered: bool, pressed: bool) -> Color:
	if not enabled:
		return off
	if pressed:
		return on.lightened(0.14)
	if hovered:
		return on.lightened(0.07)
	return on

# Truncate `s` with an ellipsis so it fits within `max_w` at `size` px (C2/C8).
func _fit(font: Font, s: String, size: int, max_w: float) -> String:
	if s == "" or font.get_string_size(s, HORIZONTAL_ALIGNMENT_LEFT, -1, size).x <= max_w:
		return s
	var ell := "…"
	while s.length() > 1 and font.get_string_size(s + ell, HORIZONTAL_ALIGNMENT_LEFT, -1, size).x > max_w:
		s = s.substr(0, s.length() - 1)
	return s.strip_edges() + ell

# Category label + pip color (C2). Weapons are one bucket; modifiers are split
# into economy / spike / passive by keywords in the name/tip (cosmetic only).
func _shop_category(is_weapon: bool, nm: String, tip: String) -> Array:
	if is_weapon:
		return ["WEAPON", Color(0.46, 0.62, 0.92)]
	var hay := (nm + " " + tip).to_lower()
	if hay.contains("gold") or hay.contains("income") or hay.contains("interest") or hay.contains("econ"):
		return ["ECONOMY", Color(0.85, 0.72, 0.34)]
	if hay.contains("spike") or hay.contains("thorn") or hay.contains("reflect"):
		return ["SPIKE", Color(0.86, 0.46, 0.4)]
	return ["PASSIVE", Color(0.62, 0.78, 0.5)]

func _draw_tooltip(font: Font, vp: Vector2, card: Rect2, nm: String, flavor: String, tip: String) -> void:
	var head: Font = _font_head if _font_head else font
	var w := 380.0
	var h := 96.0
	var x := clampf(card.position.x + card.size.x * 0.5 - w * 0.5, 8.0, vp.x - w - 8.0)
	var y := card.position.y - h - 12.0
	draw_rect(Rect2(Vector2(x, y), Vector2(w, h)), ArtTheme.ui("panel_bg"))
	draw_rect(Rect2(Vector2(x, y), Vector2(w, h)), ArtTheme.ui("panel_border"), false, 2.0)
	# Rust-sourced name / flavor / tip — translate at the draw boundary.
	draw_string(head, Vector2(x + 14, y + 26), tr(nm), HORIZONTAL_ALIGNMENT_LEFT, w - 28, 17, ArtTheme.ui("accent"))
	draw_multiline_string(font, Vector2(x + 14, y + 48), tr(flavor), HORIZONTAL_ALIGNMENT_LEFT, w - 28, 13, 2, ArtTheme.ui("text"))
	draw_string(font, Vector2(x + 14, y + h - 12), tr(tip), HORIZONTAL_ALIGNMENT_LEFT, w - 28, 13, ArtTheme.ui("text_dim"))

func _rarity_color(r: int) -> Color:
	match r:
		1: return Color(0.40, 0.80, 0.45)   # uncommon
		2: return Color(0.35, 0.60, 1.00)   # rare
		3: return Color(0.78, 0.46, 0.96)   # epic
		_: return Color(0.60, 0.60, 0.66)   # common
