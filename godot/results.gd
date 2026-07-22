# Results — the centered run-summary panel shown on tank death (Main.tscn, UI
# layer; main.gd toggles visibility with the shop). main.gd hands it a death
# report ONCE at the death edge (set_report); every display string is composed
# there — never per frame — then _draw just paints the cached strings. Reads
# the (still-valid) sim through SimView only inside set_report (arsenal), plus
# Profile bests for the "best: N" companions. Offers Redeploy (Enter/Space/
# click) or Menu (Esc). Cosmetic only; it never touches the sim. Sets
# `redeploy_rect` for main.gd's click handler (via redeploy_hit()).
extends Node2D

var view: SimView = null      # wired by main.gd

# Results-panel "Redeploy" hit-target, recomputed by _draw each frame while
# dead and consulted by main.gd's click handler.
var redeploy_rect := Rect2()

# Enemy-kind display names by catalog kind index (mirrors theme.gd's
# ENEMY_MANIFEST order). English keys — tr()'d when the report is composed.
const ENEMY_NAMES := ["Squeakzilla", "Fanged Death", "The Hippocrate", "Doomduck",
	"Bacon", "Honk", "Bonk", "Nope Rope", "Croak", "Spicy", "Popsicle", "Dodo"]

# A11: one rotating new-player tip per death, cycled by total runs played and
# retired once the profile shows TIPS_UNTIL_RUNS runs. Translation-table keys.
const TIPS_UNTIL_RUNS := 20
const TIPS := [
	"Tip: enemy waves scale every round (and jump +20% at 10:00) — buy HP, armor and regen so the ramp doesn't outpace you.",
	"Tip: the shop closes for good at 15:00 when the boss arrives — spend your gold before the bell.",
	"Tip: Clear [Space] is the only thing that hurts the boss — keep it off cooldown once the boss is up.",
	"Tip: every shop slot rolls independently — rarity odds are 50/30/15/5, so an epic on the board is a rare treat.",
	"Tip: free rerolls bank up and never expire — spend them freely when the board is bad.",
	"Tip: damage types matter — piercing, siege and magic each punish different armor, and the typed +25% upgrades compound.",
	"Tip: economy compounds — income early snowballs into weapons late; pure greed and pure guns are both risky.",
	"Tip: Deep Freeze is an upgrade — without it 25 Frost stacks never freeze; with it, frozen enemies take +50% damage.",
]

# --- cached report strings (composed ONCE in set_report; no per-frame allocs) --
var _report := {}
var _stat_vals := PackedStringArray()    # round / damage / gold / weapons values
var _stat_bests := PackedStringArray()   # dim "best: N" companions ("" = none)
var _new_best := ""                      # gold headline under the subtitle
var _info_lines: Array = []              # [text: String, color: Color] pairs
var _tip := ""
var _goal_lines := PackedStringArray()   # "NEXT: ..." lines (A7)
var _unlock_names := PackedStringArray()
var _ars := ""
var _ph := 420.0                         # panel height, sized to the content

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

func clear_report() -> void:
	_report = {}

# Compose EVERY display string for the death panel from main.gd's report (see
# main._build_death_report). Called exactly once per death, so the per-frame
# _draw below never formats or translates anything.
func set_report(report: Dictionary) -> void:
	_report = report
	var rec: Dictionary = report.get("rec", {})
	var rnd := int(rec.get("round", 0))
	var dmg := int(rec.get("damage", 0))
	var gold := int(rec.get("gold", 0))
	var runs := int(report.get("total_runs", 0))
	var had_history := runs > 1   # a PREVIOUS run exists to compare against

	# Stat values + "best: N" companions (A1). No garbage comparisons on the
	# very first run — companions only appear once a previous best exists.
	_stat_vals = PackedStringArray([
		"%d" % rnd, "%d" % dmg, "%d" % gold, "%d" % int(rec.get("weapons_bought", 0)),
	])
	_stat_bests = PackedStringArray(["", "", "", ""])
	if had_history:
		_stat_bests[0] = tr("best: %s") % str(Profile.best_round)
		_stat_bests[1] = tr("best: %s") % Profile.fmt_num(Profile.best_damage)
		_stat_bests[2] = tr("best: %s") % Profile.fmt_num(Profile.best_gold)

	# Gold NEW BEST headline (A1) — round outranks damage outranks gold.
	var bests: Dictionary = report.get("bests", {})
	_new_best = ""
	if bool(bests.get("round", false)):
		_new_best = tr("NEW BEST ROUND!")
	elif bool(bests.get("damage", false)):
		_new_best = tr("NEW BEST DAMAGE!")
	elif bool(bests.get("gold", false)):
		_new_best = tr("NEW BEST GOLD!")

	# Near-miss framing (A2): survived clock, boss distance, best delta.
	_info_lines = []
	var tick := int(report.get("tick", 0))
	_info_lines.append([tr("Survived %s") % _mmss(tick), ArtTheme.ui("text")])
	var boss_tick := int(report.get("boss_spawn_tick", 0))
	var boss_permille := int(report.get("boss_hp_permille", -1))
	if boss_permille >= 0:
		@warning_ignore("integer_division")
		_info_lines.append([tr("Boss had %d%% left") % (boss_permille / 10),
			ArtTheme.ui("danger")])
	elif boss_tick > 0 and tick < boss_tick:
		_info_lines.append([tr("The boss was %s away") % _mmss(boss_tick - tick),
			ArtTheme.ui("danger").lightened(0.2)])
	var prev_best := int(report.get("prev_best_round", 0))
	if prev_best > 0:
		if rnd > prev_best:
			_info_lines.append([tr("+%d rounds vs your best") % (rnd - prev_best),
				ArtTheme.ui("coin")])
		elif rnd < prev_best:
			_info_lines.append([tr("-%d rounds short of your best") % (prev_best - rnd),
				ArtTheme.ui("text_dim")])

	# Death explanation (A11): dominant on-screen kind + trailing-10 s damage.
	var top_kind := int(report.get("top_kind", -1))
	if top_kind >= 0 and top_kind < ENEMY_NAMES.size():
		_info_lines.append([tr("Overwhelmed by %s ×%d — took %s damage in the final 10 s") % [
			tr(ENEMY_NAMES[top_kind]), int(report.get("top_kind_count", 0)),
			_fmt_k(int(report.get("recent_damage", 0)))], ArtTheme.ui("danger").darkened(0.1)])

	# One rotating tip while the profile is young (A11).
	_tip = tr(TIPS[(runs - 1) % TIPS.size()]) if runs > 0 and runs < TIPS_UNTIL_RUNS else ""

	# Next goals (A7): up to 2 "NEXT: name — cur / target" lines.
	_goal_lines = PackedStringArray()
	for g in report.get("goals", []):
		_goal_lines.append(tr("NEXT: %s — %s / %s") % [tr(String(g["name"])),
			Profile.fmt_num(int(g["current"])), Profile.fmt_num(int(g["target"]))])

	# Arsenal + unlock names (translated once).
	var owned: PackedStringArray = view.arsenal_lines() if view != null else PackedStringArray()
	_ars = "  ·  ".join(_tr_arsenal(owned)) if owned.size() > 0 else tr("— nothing acquired —")
	_unlock_names = PackedStringArray()
	for id in report.get("unlocks", []):
		_unlock_names.append("★ " + tr(String(Profile.ach_def(id).get("name", id))))

	# Panel height from the content (same constants _draw uses).
	var h := 110.0                                # title + subtitle block
	if _new_best != "":
		h += 24.0
	h += 4.0 * 30.0 + 4.0                         # stat rows
	h += _info_lines.size() * 20.0
	if _tip != "":
		h += 20.0
	h += 10.0                                     # gap
	h += 20.0 + 30.0                              # ARSENAL header + line
	h += 20.0                                     # ACHIEVEMENTS header
	h += maxi(_unlock_names.size(), 1) * 22.0
	h += _goal_lines.size() * 20.0
	h += 84.0                                     # button + prompt
	_ph = h

func _draw() -> void:
	if view == null or _report.is_empty():
		return
	var vp: Vector2 = get_viewport_rect().size
	var font: Font = _font
	var head: Font = _font_head

	# Dim the arena behind the panel so the summary reads cleanly.
	draw_rect(Rect2(Vector2.ZERO, vp), Color(0.02, 0.02, 0.04, 0.55))

	# Panel geometry, centered.
	var pw := 560.0
	var ph := _ph
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
	var ry := py + 94.0
	if _new_best != "":
		# A1: gold new-best headline, pulsing on the render clock.
		var glow := 0.5 + 0.5 * sin(Time.get_ticks_msec() * 0.005)
		var bw0 := head.get_string_size(_new_best, HORIZONTAL_ALIGNMENT_LEFT, -1, 18).x
		draw_string(head, Vector2(cx - bw0 * 0.5, ry), _new_best,
			HORIZONTAL_ALIGNMENT_LEFT, -1, 18, ArtTheme.ui("coin") * (1.1 + 0.5 * glow))
		ry += 24.0

	# Stat rows (label left, value right, dim best companion beside the value).
	var lx := px + 36.0
	var rx := px + pw - 36.0
	ry += 16.0
	var rstep := 30.0
	_draw_stat_row(font, head, lx, rx, ry, tr("Round reached"), _stat_vals[0], ArtTheme.ui("header"), _stat_bests[0])
	ry += rstep
	_draw_stat_row(font, head, lx, rx, ry, tr("Damage dealt"), _stat_vals[1], ArtTheme.ui("accent"), _stat_bests[1])
	ry += rstep
	_draw_stat_row(font, head, lx, rx, ry, tr("Gold earned"), _stat_vals[2], ArtTheme.ui("coin"), _stat_bests[2])
	ry += rstep
	_draw_stat_row(font, head, lx, rx, ry, tr("Weapons bought"), _stat_vals[3], ArtTheme.ui("text"), _stat_bests[3])
	ry += rstep + 4.0

	# Near-miss framing + death explanation (A2/A11), pre-composed lines.
	for line in _info_lines:
		draw_string(font, Vector2(lx, ry), line[0], HORIZONTAL_ALIGNMENT_LEFT, pw - 72.0, 13, line[1])
		ry += 20.0
	if _tip != "":
		draw_string(font, Vector2(lx, ry), _tip, HORIZONTAL_ALIGNMENT_LEFT, pw - 72.0, 12, ArtTheme.ui("text_dim"))
		ry += 20.0
	ry += 10.0

	# Owned arsenal, condensed onto one wrapped line (pre-translated).
	draw_string(head, Vector2(lx, ry), tr("ARSENAL"), HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("header"))
	ry += 20.0
	draw_string(font, Vector2(lx, ry), _ars, HORIZONTAL_ALIGNMENT_LEFT, pw - 72.0, 13, ArtTheme.ui("text"))
	ry += 30.0

	# Achievements unlocked this run — pulsing celebration rows (A6).
	draw_string(head, Vector2(lx, ry), tr("ACHIEVEMENTS UNLOCKED"), HORIZONTAL_ALIGNMENT_LEFT, -1, 14, ArtTheme.ui("accent"))
	ry += 20.0
	if _unlock_names.is_empty():
		draw_string(font, Vector2(lx, ry), tr("— none this run —"), HORIZONTAL_ALIGNMENT_LEFT, pw - 72.0, 13, ArtTheme.ui("text_dim"))
		ry += 22.0
	else:
		for i in _unlock_names.size():
			# Render-clock pulse: brightness + a gentle scale breath per row.
			var t := Time.get_ticks_msec() * 0.004 + i * 1.3
			var k := 0.5 + 0.5 * sin(t)
			var sc := 1.0 + 0.05 * k
			var col := ArtTheme.ui("accent").lerp(Color(1.6, 1.4, 0.8), 0.7 * k)
			draw_set_transform(Vector2(lx, ry), 0.0, Vector2(sc, sc))
			draw_string(font, Vector2.ZERO, _unlock_names[i], HORIZONTAL_ALIGNMENT_LEFT, pw - 72.0, 14, col)
			draw_set_transform(Vector2.ZERO, 0.0, Vector2.ONE)
			ry += 22.0

	# Next goals (A7).
	for gl in _goal_lines:
		draw_string(font, Vector2(lx, ry), gl, HORIZONTAL_ALIGNMENT_LEFT, pw - 72.0, 13, ArtTheme.ui("header"))
		ry += 20.0

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

# One label/value row for the results panel; `best` (may be "") is the dim
# personal-best companion drawn just left of the value.
func _draw_stat_row(font: Font, head: Font, lx: float, rx: float, y: float, label: String, value: String, vcol: Color, best: String = "") -> void:
	draw_string(font, Vector2(lx, y), label, HORIZONTAL_ALIGNMENT_LEFT, -1, 16, ArtTheme.ui("text_dim"))
	var vw := head.get_string_size(value, HORIZONTAL_ALIGNMENT_LEFT, -1, 18).x
	draw_string(head, Vector2(rx - vw, y + 1), value, HORIZONTAL_ALIGNMENT_LEFT, -1, 18, vcol)
	if best != "":
		var bw := font.get_string_size(best, HORIZONTAL_ALIGNMENT_LEFT, -1, 12).x
		draw_string(font, Vector2(rx - vw - 14.0 - bw, y), best, HORIZONTAL_ALIGNMENT_LEFT, -1, 12, ArtTheme.ui("text_dim").darkened(0.1))

# "mm:ss" for a tick count (30 Hz sim ticks; render-side arithmetic only).
func _mmss(ticks: int) -> String:
	@warning_ignore("integer_division")
	var secs := maxi(ticks, 0) / 30
	@warning_ignore("integer_division")
	return "%02d:%02d" % [secs / 60, secs % 60]

# Compact damage figure for the death-explanation line ("47k" / "812").
func _fmt_k(n: int) -> String:
	if n >= 1000:
		return "%dk" % roundi(float(n) / 1000.0)
	return str(n)

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
