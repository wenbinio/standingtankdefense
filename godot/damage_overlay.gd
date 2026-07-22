# DamageOverlay — the hold-[Tab] per-weapon damage table (DPS meter). A small
# UiLayer module (Main.tscn) in the hud.gd immediate-mode style: read-only over
# the SimView wrapper, zero writes to the sim.
#
# Behavior:
# - Shown ONLY while the player holds the `ui_damage_overlay` action (physical
#   Tab). main.gd feeds `held` once per frame (false while the pause menu is
#   open); the overlay works while alive AND on the frozen death state.
# - Rows are the sim's authoritative per-source damage ledger via
#   SimView.damage_rows(): every owned weapon that dealt damage, plus the
#   pseudo sources (Spikes / Clear / Other). Ranked by total, top MAX_ROWS
#   shown, the rest aggregated into one "+N more" line.
# - The table is REBUILT AT MOST ONCE PER SECOND while held (and on the show
#   edge), never per frame — drawing iterates prebuilt row dicts.
extends Node2D

const MAX_ROWS := 12          # ranked rows before the "+N more" aggregate
const REBUILD_MS := 1000      # rebuild cadence while held (~1/s)

# Damage-type id → row colour (0 Normal · 1 Piercing · 2 Magic · 3 Siege ·
# 4 Chaos; 255 = pseudo row → neutral). Matches the arsenal panel's type
# naming (hud.gd ARS_DMG_KEYS); colours chosen to read on the dark panel.
const TYPE_COLORS: Array[Color] = [
	Color(0.75, 0.75, 0.78),   # Normal — steel gray
	Color(0.95, 0.82, 0.42),   # Piercing — fletching gold
	Color(0.55, 0.72, 1.00),   # Magic — arcane blue
	Color(0.98, 0.62, 0.34),   # Siege — blast orange
	Color(0.80, 0.52, 0.95),   # Chaos — fel purple
]
const PSEUDO_COLOR := Color(0.55, 0.62, 0.58)   # Spikes/Clear/Other rows

var view: SimView = null      # wired by main.gd
var held := false             # fed by main.gd once per frame (pause-gated)

var _font: Font = null
var _font_head: Font = null
var _was_held := false
var _built_ms := -REBUILD_MS  # last rebuild time (wall-clock; cosmetic only)
# Prebuilt display rows [{t, cnt, dmg, share_permille, col}] + header totals.
var _rows: Array = []
var _total := 0

func _ready() -> void:
	_font = ArtTheme.ui_font(false)
	if _font == null:
		_font = ThemeDB.fallback_font
	_font_head = ArtTheme.ui_font(true)
	if _font_head == null:
		_font_head = _font

func _process(_delta: float) -> void:
	if view == null:
		return
	var now := Time.get_ticks_msec()
	if held and (not _was_held or now - _built_ms >= REBUILD_MS):
		_built_ms = now
		_rebuild()
	if held != _was_held:
		_was_held = held
		queue_redraw()
	elif held:
		queue_redraw()

# Rank the ledger rows (desc by total; ties by ascending source id for a
# stable order), cap at MAX_ROWS, aggregate the tail. Show-edge / ~1 Hz only.
func _rebuild() -> void:
	var raw: Array = view.damage_rows()
	_rows = []
	_total = 0
	for r in raw:
		_total += int(r["total"])
	raw.sort_custom(func(a, b) -> bool:
		var ta: int = a["total"]
		var tb: int = b["total"]
		return ta > tb if ta != tb else int(a["source"]) < int(b["source"]))
	var shown: int = mini(raw.size(), MAX_ROWS)
	for i in shown:
		var r: Dictionary = raw[i]
		var dt := int(r["damage_type"])
		var cnt := int(r["count"])
		_rows.append({
			"t": tr(String(r["name"])),
			"cnt": ("x%d" % cnt) if cnt > 0 else "",
			"dmg": _fmt(int(r["total"])),
			"share_permille": _permille(int(r["total"])),
			"col": TYPE_COLORS[dt] if dt >= 0 and dt < TYPE_COLORS.size() else PSEUDO_COLOR,
		})
	if raw.size() > shown:
		var rest := 0
		for i in range(shown, raw.size()):
			rest += int(raw[i]["total"])
		_rows.append({
			"t": tr("+%d more") % (raw.size() - shown),
			"cnt": "",
			"dmg": _fmt(rest),
			"share_permille": _permille(rest),
			"col": PSEUDO_COLOR,
		})

func _permille(part: int) -> int:
	@warning_ignore("integer_division")
	return (part * 1000) / _total if _total > 0 else 0

# Compact damage figure: 12345 → "12.3K", 4200000 → "4.2M", 1.5e9 → "1.5B".
func _fmt(n: int) -> String:
	if n >= 1_000_000_000:
		return "%.1fB" % (float(n) / 1_000_000_000.0)
	if n >= 1_000_000:
		return "%.1fM" % (float(n) / 1_000_000.0)
	if n >= 10_000:
		return "%.1fK" % (float(n) / 1_000.0)
	return "%d" % n

func _draw() -> void:
	if not held or view == null:
		return
	var vp: Vector2 = get_viewport_rect().size
	var font: Font = _font
	var head: Font = _font_head
	var pw := 460.0
	var head_h := 30.0
	var row_h := 22.0
	var rows := maxf(float(_rows.size()), 1.0)
	var ph := head_h + 10.0 + rows * row_h + 10.0
	var px := (vp.x - pw) * 0.5
	var py := maxf((vp.y - ph) * 0.38, 56.0)   # above center, below the boss strip
	# Panel (hud.gd palette) over a dim backdrop so the table reads mid-combat.
	draw_rect(Rect2(Vector2(px - 6, py - 6), Vector2(pw + 12, ph + 12)), Color(0, 0, 0, 0.45))
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, ph)), ArtTheme.ui("panel_bg"))
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, ph)), ArtTheme.ui("panel_border"), false, 1.0)
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, head_h)), ArtTheme.ui("panel_border").darkened(0.4))
	draw_string(head, Vector2(px + 12, py + 20), tr("DAMAGE DEALT"),
		HORIZONTAL_ALIGNMENT_LEFT, -1, 15, ArtTheme.ui("header"))
	var tot := _fmt(_total)
	var tw := head.get_string_size(tot, HORIZONTAL_ALIGNMENT_LEFT, -1, 15).x
	draw_string(head, Vector2(px + pw - tw - 12, py + 20), tot,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 15, ArtTheme.ui("accent"))
	var ay := py + head_h + 10.0
	if _rows.is_empty():
		draw_string(font, Vector2(px + 12, ay + 14), tr("— nothing yet —"),
			HORIZONTAL_ALIGNMENT_LEFT, pw - 24, 13, ArtTheme.ui("text_dim"))
		return
	# Columns: [name + xN] [share bar] [pct] [damage].
	var bar_x := px + 214.0
	var bar_w := 130.0
	var pct_x := px + 352.0
	for row in _rows:
		var col: Color = row["col"]
		draw_string(font, Vector2(px + 12, ay + 14), String(row["t"]),
			HORIZONTAL_ALIGNMENT_LEFT, 164, 14, ArtTheme.ui("text"))
		if String(row["cnt"]) != "":
			draw_string(font, Vector2(px + 180, ay + 14), String(row["cnt"]),
				HORIZONTAL_ALIGNMENT_LEFT, -1, 12, ArtTheme.ui("text_dim"))
		var share: float = float(row["share_permille"]) / 1000.0
		var bar := Rect2(Vector2(bar_x, ay + 4.0), Vector2(bar_w, 10.0))
		draw_rect(bar, ArtTheme.ui("panel_border").darkened(0.5))
		if share > 0.0:
			draw_rect(Rect2(bar.position, Vector2(bar.size.x * clampf(share, 0.0, 1.0), bar.size.y)), col)
		draw_string(font, Vector2(pct_x, ay + 14), "%d%%" % int(round(share * 100.0)),
			HORIZONTAL_ALIGNMENT_LEFT, -1, 12, ArtTheme.ui("text_dim"))
		var dmg := String(row["dmg"])
		var dw := font.get_string_size(dmg, HORIZONTAL_ALIGNMENT_LEFT, -1, 14).x
		draw_string(font, Vector2(px + pw - dw - 12, ay + 14), dmg,
			HORIZONTAL_ALIGNMENT_LEFT, -1, 14, col)
		ay += row_h
