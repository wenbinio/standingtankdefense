# Hud — the single-arena top bar (HP bar / gold / income / round + next-round
# countdown / match clock), the top-center boss strip (pre-boss countdown, then
# the boss HP bar), and the arsenal panel (Main.tscn, UI layer). Immediate-mode
# _draw on its own CanvasItem so HUD redraws are scoped away from the arena.
# Read-only: every value comes through the SimView wrapper.
extends Node2D

# Sim tick rate (mirrors sim::TICK_HZ) — converts view ticks to seconds for the
# match clock / countdowns. Render-only arithmetic.
const TICK_HZ := 30
# Boss countdown appears this many ticks before the boss spawn (last 3 minutes).
const BOSS_WARN_TICKS := 3 * 60 * TICK_HZ

var view: SimView = null      # wired by main.gd

var _font: Font = null
var _font_head: Font = null
var _panel_tex: Texture2D
var _heart_tex: Texture2D
var _coin_tex: Texture2D

# --- render-side display state (cosmetic; never written back to the sim) -----
# Fighting-game "damage chunk": a white bar segment that lags the real HP on
# drops (holds briefly, then catches up over ~0.5 s) so multi-hits read.
var _hp_chunk := 0.0
var _hp_chunk_hold := 0.0
# Smooth-lerped boss HP display (permille) so individual Clear hits read as a
# visible drain instead of an instant jump.
var _boss_disp := 0.0
var _boss_up := false

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

func _process(delta: float) -> void:
	_tick_display_state(delta)
	queue_redraw()

# Advance the cosmetic display trackers (HP damage chunk + boss bar lerp).
# Read-only over the view; frame-rate driven, so it never touches determinism.
func _tick_display_state(delta: float) -> void:
	if view == null or not view.is_valid():
		return
	# Damage chunk: snap up on heal/redeploy, lag down on damage.
	var hp := float(maxi(view.tank_hp(), 0))
	if hp >= _hp_chunk:
		_hp_chunk = hp
		_hp_chunk_hold = 0.0
	else:
		_hp_chunk_hold += delta
		if _hp_chunk_hold > 0.15:   # brief hold so the chunk is visible at all
			# Exponential catch-up, ~95% closed in ~0.5 s.
			_hp_chunk = maxf(hp, _hp_chunk - (_hp_chunk - hp) * minf(delta * 6.0, 1.0))
	# Boss bar: snap to the real value on appear, then chase it smoothly.
	var bi: Dictionary = view.boss_info()
	if bi.is_empty():
		_boss_up = false
	else:
		var target := float(bi["hp_permille"])
		if not _boss_up:
			_boss_disp = target
			_boss_up = true
		else:
			_boss_disp += (target - _boss_disp) * minf(delta * 8.0, 1.0)

# "mm:ss" for a tick count (floor'd to whole seconds).
func _mmss(ticks: int) -> String:
	@warning_ignore("integer_division")
	var secs := maxi(ticks, 0) / TICK_HZ
	@warning_ignore("integer_division")
	return "%02d:%02d" % [secs / 60, secs % 60]

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
	_draw_hp_bar(Rect2(Vector2(60, 51), Vector2(140, 5)))
	_blit(_coin_tex, Vector2(40, 74), 28)
	draw_string(head, Vector2(60, 81), "%d" % view.gold(), HORIZONTAL_ALIGNMENT_LEFT, -1, 19, ArtTheme.ui("coin"))
	var gold_w := head.get_string_size("%d" % view.gold(), HORIZONTAL_ALIGNMENT_LEFT, -1, 19).x
	draw_string(font, Vector2(60 + gold_w + 8, 81), "+%d/t" % view.income(), HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("accent_dim"))
	draw_string(head, Vector2(212, 45), tr("ROUND %d") % view.round_num(), HORIZONTAL_ALIGNMENT_LEFT, -1, 16, ArtTheme.ui("header"))
	# Round pacing: countdown to the next round (= the next shop refresh) and
	# the mm:ss match clock — replaces the old raw `tick %d` readout.
	var next_secs := ceili(float(view.ticks_to_next_round()) / float(TICK_HZ))
	draw_string(font, Vector2(212, 63), tr("next in %ds") % next_secs, HORIZONTAL_ALIGNMENT_LEFT, -1, 12, ArtTheme.ui("accent_dim"))
	draw_string(font, Vector2(212, 81), _mmss(view.tick()), HORIZONTAL_ALIGNMENT_LEFT, -1, 13, ArtTheme.ui("text_dim"))
	_draw_boss_strip(font, head, vp)
	_draw_arsenal(font, head, vp)

# The layered tank HP bar: dark track, white lagging damage chunk, live HP fill.
func _draw_hp_bar(r: Rect2) -> void:
	var max_hp := float(maxi(view.tank_max_hp(), 1))
	var hp := float(maxi(view.tank_hp(), 0))
	draw_rect(r, ArtTheme.ui("panel_border").darkened(0.5))
	var chunk_w := r.size.x * clampf(_hp_chunk / max_hp, 0.0, 1.0)
	if chunk_w > 0.0:
		draw_rect(Rect2(r.position, Vector2(chunk_w, r.size.y)), Color(1, 1, 1, 0.85))
	var hp_w := r.size.x * clampf(hp / max_hp, 0.0, 1.0)
	if hp_w > 0.0:
		draw_rect(Rect2(r.position, Vector2(hp_w, r.size.y)), ArtTheme.ui("hp"))

# Top-center boss strip: a "BOSS IN mm:ss" warning during the final approach,
# then the boss HP bar (name + smooth-lerped fill) while the boss is alive.
# Boss enemy names come from the Rust catalog and are translated at this draw
# boundary like every other content name (the CSV keys ARE the English names).
func _draw_boss_strip(font: Font, head: Font, vp: Vector2) -> void:
	var bi: Dictionary = view.boss_info()
	if bi.is_empty():
		# Pre-boss countdown, only inside the warning window.
		var spawn := view.boss_spawn_tick()
		var left := spawn - view.tick()
		if spawn <= 0 or left <= 0 or left > BOSS_WARN_TICKS:
			return
		var warn := tr("BOSS IN %s") % _mmss(left)
		var ww := head.get_string_size(warn, HORIZONTAL_ALIGNMENT_LEFT, -1, 17).x
		draw_string(head, Vector2((vp.x - ww) * 0.5, 32), warn, HORIZONTAL_ALIGNMENT_LEFT, -1, 17, ArtTheme.ui("danger"))
		return
	# Boss alive: prominent top-center bar. Sized/positioned to clear both the
	# top-left panel (ends x≈342) and the arsenal panel (starts x≈vp.x−246).
	var bw := 440.0
	var bx := (vp.x - bw) * 0.5
	var nm: String = tr(_boss_name(int(bi["kind"])))
	var nw := head.get_string_size(nm, HORIZONTAL_ALIGNMENT_LEFT, -1, 15).x
	draw_string(head, Vector2((vp.x - nw) * 0.5, 26), nm, HORIZONTAL_ALIGNMENT_LEFT, -1, 15, ArtTheme.ui("danger"))
	var bar := Rect2(Vector2(bx, 32), Vector2(bw, 14))
	draw_rect(bar, ArtTheme.ui("panel_bg"))
	var fill_w := bar.size.x * clampf(_boss_disp / 1000.0, 0.0, 1.0)
	if fill_w > 0.0:
		draw_rect(Rect2(bar.position, Vector2(fill_w, bar.size.y)), ArtTheme.ui("danger").darkened(0.15))
	draw_rect(bar, ArtTheme.ui("panel_border"), false, 1.0)
	# Live permille as a % readout inside the bar (right edge) for exact reads.
	var pct := "%d%%" % int(clampf(float(bi["hp_permille"]) / 10.0, 0.0, 100.0))
	var pw := font.get_string_size(pct, HORIZONTAL_ALIGNMENT_LEFT, -1, 11).x
	draw_string(font, Vector2(bar.position.x + bar.size.x - pw - 6, bar.position.y + 11), pct,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 11, ArtTheme.ui("text"))

# Boss display name by enemy catalog kind. Kind 2 is the STABLE boss index in
# `sim-core content::ENEMIES` (see its doc comment); the name doubles as the
# translation key, matching how the arena's enemy names reach the CSV.
func _boss_name(kind: int) -> String:
	match kind:
		2: return "The Hippocrate"
		_: return "The Hippocrate"   # single boss today; future bosses map here

# A9 — Arsenal panel (build-identity surface): the C3 footprint (234 px wide,
# top-right, py 12) now grouped by ATTACK CLASS with rarity-colored counts and
# a SYNERGIES block for the owned per-weapon-count self-scalers, so a build
# reads as a build instead of a flat name list.
#
# Layout budget @1280×720: the panel must clear the bottom shop bar (top =
# vp.y − 162 = 558) and the top-center boss strip (ends x≈860; panel starts
# x=1034). Worst case = 4 class headers + 4×(3 stacks + 1 "+N more") +
# 1 "+N more" classes line + 1 SYNERGIES header + 3 synergy lines + 1 "+N
# more" = 26 rows → height 26 + 8 + 26×19 + 6 = 534, bottom edge 546 < 558. ✓
#
# Overflow rule (documented): grouped mode shows the top M = 4 classes by
# owned-weapon count (ties: lower class id first); within a class the top 3
# stacks by count (ties: name order), then a dim "+N more"; if > 4 classes
# exist, one final "+N more" line counts the hidden classes. Synergies show
# the top 3 active self-scalers by current bonus, then "+N more".
#
# Early game: with fewer than 2 distinct attack classes (0–2 weapons) grouping
# is noise, so the panel falls back to the flat "Name ×N" list (capped at 12
# stacks) — it never renders a one-class "group" as broken half-structure.
const ARS_MAX_CLASSES := 4     # M: classes shown, top by count
const ARS_MAX_PER_CLASS := 3   # stacks listed per class before "+N more"
const ARS_MAX_SYNERGY := 3     # synergy lines before "+N more"
const ARS_MAX_FLAT := 12       # simple-mode stacks before "+N more"
const ARS_GROUP_MIN_CLASSES := 2

# Class-id → header word (index = the sim's attack_scope_id; tr()'d at build).
const ARS_CLASS_KEYS: Array[String] = ["SINGLE", "SPLASH", "BARRAGE", "AREA", "WAVE", "BOUNCE"]
# Damage-type id → word (tr()'d at build; "Normal" reuses the existing key).
const ARS_DMG_KEYS: Array[String] = ["Normal", "Piercing", "Magic", "Siege", "Chaos"]

# Row styles for the cached display rows.
const ARS_ROW_ENTRY := 0     # weapon stack: name left, "×N" right (rarity color)
const ARS_ROW_CLASS := 1     # class header: "SPLASH" left, "×7" right
const ARS_ROW_MORE := 2      # dim "+N more" overflow line
const ARS_ROW_SYN := 3       # synergy line (accent)
const ARS_ROW_SYNHEAD := 4   # "SYNERGIES" subheader

# Cached display rows [{t, r, s, rar}] — rebuilt ONLY when the arsenal
# revision moves (a purchase; see SimView.arsenal_rev) or the locale changes.
# Drawing iterates prebuilt strings: zero per-frame string allocation.
var _ars_rows: Array = []
var _ars_total := 0            # total weapons owned (header badge)
var _ars_rev := -9223372036854775807
var _ars_locale := ""

# Rarity → count color (mirrors shop.gd's _rarity_color).
func _rarity_color(r: int) -> Color:
	match r:
		1: return Color(0.40, 0.80, 0.45)   # uncommon
		2: return Color(0.35, 0.60, 1.00)   # rare
		3: return Color(0.78, 0.46, 0.96)   # epic
		_: return Color(0.60, 0.60, 0.66)   # common

# Rebuild the cached rows from the typed view dicts (purchase/locale edge only).
func _rebuild_arsenal_rows() -> void:
	var entries: Array = view.arsenal_entries()
	var syn: Array = view.arsenal_synergies()
	_ars_rows = []
	_ars_total = 0
	for e in entries:
		_ars_total += int(e["count"])
	# Bucket stacks per attack class, tallying each class's weapon count.
	var by_class := {}   # class_id -> {"n": int, "stacks": Array}
	for e in entries:
		var cid := int(e["class_id"])
		if not by_class.has(cid):
			by_class[cid] = {"n": 0, "stacks": []}
		by_class[cid]["n"] += int(e["count"])
		by_class[cid]["stacks"].append(e)
	if by_class.size() >= ARS_GROUP_MIN_CLASSES:
		var cids: Array = by_class.keys()
		cids.sort_custom(func(a, b) -> bool:
			var na: int = by_class[a]["n"]
			var nb: int = by_class[b]["n"]
			return na > nb if na != nb else a < b)
		for ci in mini(cids.size(), ARS_MAX_CLASSES):
			var cid: int = cids[ci]
			_ars_rows.append({
				"t": tr(ARS_CLASS_KEYS[clampi(cid, 0, 5)]),
				"r": "×%d" % int(by_class[cid]["n"]),
				"s": ARS_ROW_CLASS, "rar": 0,
			})
			var stacks: Array = by_class[cid]["stacks"]
			stacks.sort_custom(func(a, b) -> bool:
				var na: int = a["count"]
				var nb: int = b["count"]
				return na > nb if na != nb else String(a["name"]) < String(b["name"]))
			for si in mini(stacks.size(), ARS_MAX_PER_CLASS):
				_append_stack_row(stacks[si])
			if stacks.size() > ARS_MAX_PER_CLASS:
				_append_more_row(stacks.size() - ARS_MAX_PER_CLASS)
		if cids.size() > ARS_MAX_CLASSES:
			_append_more_row(cids.size() - ARS_MAX_CLASSES)
	else:
		# Simple mode: the flat name-sorted list (marshal order).
		for si in mini(entries.size(), ARS_MAX_FLAT):
			_append_stack_row(entries[si])
		if entries.size() > ARS_MAX_FLAT:
			_append_more_row(entries.size() - ARS_MAX_FLAT)
	# SYNERGIES block: only the ACTIVE self-scalers (source owned, bonus > 0).
	var active: Array = []
	for s in syn:
		if int(s["count"]) > 0 and int(s["bonus_milli"]) > 0:
			active.append(s)
	if active.is_empty():
		return
	active.sort_custom(func(a, b) -> bool:
		return int(a["bonus_milli"]) > int(b["bonus_milli"]))
	_ars_rows.append({"t": tr("SYNERGIES"), "r": "", "s": ARS_ROW_SYNHEAD, "rar": 0})
	for si in mini(active.size(), ARS_MAX_SYNERGY):
		var s: Dictionary = active[si]
		var dmg: String = tr(ARS_DMG_KEYS[clampi(int(s["damage_type"]), 0, 4)])
		_ars_rows.append({
			"t": tr("+%d%% %s — %s ×%d") % [
				int(round(float(s["bonus_milli"]) / 1000.0)), dmg,
				tr(String(s["name"])), int(s["count"])],
			"r": "", "s": ARS_ROW_SYN, "rar": 0,
		})
	if active.size() > ARS_MAX_SYNERGY:
		_append_more_row(active.size() - ARS_MAX_SYNERGY)

func _append_stack_row(e: Dictionary) -> void:
	_ars_rows.append({
		"t": tr(String(e["name"])),
		"r": "×%d" % int(e["count"]),
		"s": ARS_ROW_ENTRY, "rar": int(e["rarity"]),
	})

func _append_more_row(n: int) -> void:
	_ars_rows.append({"t": tr("+%d more") % n, "r": "", "s": ARS_ROW_MORE, "rar": 0})

func _draw_arsenal(font: Font, head: Font, vp: Vector2) -> void:
	# Cache edge: a purchase moved the arsenal revision, or the locale changed.
	var rev: int = view.arsenal_rev()
	var loc := TranslationServer.get_locale()
	if rev != _ars_rev or loc != _ars_locale:
		_ars_rev = rev
		_ars_locale = loc
		_rebuild_arsenal_rows()
	var pw := 234.0
	var px := vp.x - pw - 12.0
	var py := 12.0
	var row_h := 19.0
	var head_h := 26.0
	var ph := head_h + 8.0 + maxf(float(_ars_rows.size()), 1.0) * row_h + 6.0
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, ph)), ArtTheme.ui("panel_bg"))
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, ph)), ArtTheme.ui("panel_border"), false, 1.0)
	draw_rect(Rect2(Vector2(px, py), Vector2(pw, head_h)), ArtTheme.ui("panel_border").darkened(0.4))
	draw_string(head, Vector2(px + 10, py + 18), tr("ARSENAL"), HORIZONTAL_ALIGNMENT_LEFT, -1, 15, ArtTheme.ui("header"))
	var ct := "%d" % _ars_total
	var ctw := font.get_string_size(ct, HORIZONTAL_ALIGNMENT_LEFT, -1, 14).x
	draw_string(font, Vector2(px + pw - ctw - 10, py + 18), ct, HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("text_dim"))
	var ay := py + head_h + 8.0
	if _ars_rows.is_empty():
		draw_string(font, Vector2(px + 10, ay + 12), tr("— nothing yet —"), HORIZONTAL_ALIGNMENT_LEFT, pw - 20, 13, ArtTheme.ui("text_dim"))
		return
	for row in _ars_rows:
		var style: int = row["s"]
		var left: String = row["t"]
		var right: String = row["r"]
		match style:
			ARS_ROW_CLASS:
				draw_string(head, Vector2(px + 10, ay + 13), left, HORIZONTAL_ALIGNMENT_LEFT, pw - 56, 13, ArtTheme.ui("header"))
				if right != "":
					var hw := head.get_string_size(right, HORIZONTAL_ALIGNMENT_LEFT, -1, 13).x
					draw_string(head, Vector2(px + pw - hw - 10, ay + 13), right, HORIZONTAL_ALIGNMENT_LEFT, -1, 13, ArtTheme.ui("text_dim"))
			ARS_ROW_SYNHEAD:
				draw_string(head, Vector2(px + 10, ay + 13), left, HORIZONTAL_ALIGNMENT_LEFT, pw - 20, 13, ArtTheme.ui("header"))
			ARS_ROW_SYN:
				draw_string(font, Vector2(px + 10, ay + 13), left, HORIZONTAL_ALIGNMENT_LEFT, pw - 20, 12, ArtTheme.ui("accent"))
			ARS_ROW_MORE:
				draw_string(font, Vector2(px + 18, ay + 13), left, HORIZONTAL_ALIGNMENT_LEFT, pw - 28, 12, ArtTheme.ui("text_dim"))
			_:
				# Weapon stack: indented under its class header in grouped mode;
				# count right-aligned in the stack's rarity color.
				draw_string(font, Vector2(px + 18, ay + 13), left, HORIZONTAL_ALIGNMENT_LEFT, pw - 64, 14, ArtTheme.ui("text"))
				if right != "":
					var cw := font.get_string_size(right, HORIZONTAL_ALIGNMENT_LEFT, -1, 14).x
					draw_string(font, Vector2(px + pw - cw - 10, ay + 13), right, HORIZONTAL_ALIGNMENT_LEFT, -1, 14, _rarity_color(int(row["rar"])))
		ay += row_h
