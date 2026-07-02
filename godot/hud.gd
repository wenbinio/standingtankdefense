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
