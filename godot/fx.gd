# Reusable, allocation-free render-only juice helper for Standing Tank Defense.
#
# This is the "juice bus": a pooled custom-`_draw` particle emitter plus
# screen-shake, hit-stop and flash accumulators, and floating combat text.
# It is PURELY cosmetic — it reads nothing from the sim and feeds nothing back.
# All animation runs on wall-clock / frame counters; never on sim ticks.
#
# Public seam (used by main.gd; safe to reuse from match.gd):
#   var fx := Fx.new()                     # construct one per arena view
#   fx.update(dt)                          # advance every frame (pass real delta)
#   fx.draw(canvas, font)                  # draw all FX (call inside _draw);
#       emitter positions are screen-space already, so no world->screen mapper
#   fx.shake_offset() -> Vector2           # add to your camera/world origin
#   fx.hitstop_active() -> bool            # true while a freeze frame holds
#   # emitters (screen-space positions):
#   fx.burst_sparks(screen_pos, color, count, speed, life)
#   fx.kill_burst(screen_pos, color)
#   fx.shockwave(screen_pos, color, max_radius, life)
#   fx.combat_text(screen_pos, text, color, size, rise)
#   fx.add_shake(amount)                   # accumulate trauma (0..1-ish)
#   fx.add_hitstop(seconds)
#   fx.add_flash(color, life)              # full-screen tint pulse
#   fx.flash_color() -> Color              # current screen flash (a==0 if none)
#
# POOLING (P1.6): every particle type lives in preallocated, fixed-capacity
# structure-of-arrays pools kept DENSE by swap-remove — expired slot i is
# overwritten by the last live slot and the live count shrinks. No Dictionary
# per particle, no Array.filter() rebuild, no per-frame heap allocation;
# emitters over cap simply drop (the caps are generous for the on-screen
# budget). The only allocation after _init is the String stored by
# combat_text(), which happens at emit time, not in the frame loop.
class_name Fx
extends RefCounted

# --- tunables -------------------------------------------------------------
const SHAKE_DECAY := 1.8           # trauma units per second bled off
const SHAKE_MAX_PIXELS := 12.0
const GRAVITY := 520.0             # px/s^2 for spark fall (screen space)
const DRAG := 2.6                  # velocity damping per second

# Per-type pool capacities (hard caps; emits beyond a cap are dropped).
const SPARK_CAP := 512
const WAVE_CAP := 48
const TEXT_CAP := 64

# --- pools (dense structure-of-arrays; live entries are 0.._*_n-1) ---------
var _spark_pos := PackedVector2Array()
var _spark_vel := PackedVector2Array()
var _spark_life := PackedFloat32Array()
var _spark_ttl := PackedFloat32Array()
var _spark_r := PackedFloat32Array()
var _spark_col := PackedColorArray()
var _spark_n := 0

var _wave_pos := PackedVector2Array()
var _wave_life := PackedFloat32Array()
var _wave_ttl := PackedFloat32Array()
var _wave_maxr := PackedFloat32Array()
var _wave_col := PackedColorArray()
var _wave_n := 0

var _text_pos := PackedVector2Array()
var _text_life := PackedFloat32Array()
var _text_ttl := PackedFloat32Array()
var _text_rise := PackedFloat32Array()
var _text_size := PackedInt32Array()
var _text_col := PackedColorArray()
var _text_str := PackedStringArray()
var _text_n := 0

# --- juice accumulators ---------------------------------------------------
var _trauma := 0.0
var _hitstop := 0.0
var _flash_col := Color(0, 0, 0, 0)
var _flash_ttl := 0.0
var _flash_life := 0.0
var _rng := RandomNumberGenerator.new()
var _seed := 0.0                   # phase seed for deterministic-looking shake

func _init() -> void:
	_rng.randomize()
	# Preallocate every pool once; nothing below ever resizes them.
	_spark_pos.resize(SPARK_CAP)
	_spark_vel.resize(SPARK_CAP)
	_spark_life.resize(SPARK_CAP)
	_spark_ttl.resize(SPARK_CAP)
	_spark_r.resize(SPARK_CAP)
	_spark_col.resize(SPARK_CAP)
	_wave_pos.resize(WAVE_CAP)
	_wave_life.resize(WAVE_CAP)
	_wave_ttl.resize(WAVE_CAP)
	_wave_maxr.resize(WAVE_CAP)
	_wave_col.resize(WAVE_CAP)
	_text_pos.resize(TEXT_CAP)
	_text_life.resize(TEXT_CAP)
	_text_ttl.resize(TEXT_CAP)
	_text_rise.resize(TEXT_CAP)
	_text_size.resize(TEXT_CAP)
	_text_col.resize(TEXT_CAP)
	_text_str.resize(TEXT_CAP)

# --- emitters -------------------------------------------------------------

# A radial spray of small glowing shards. `color` should be bright (>1 channels
# bloom under glow). Cheap: count defaults small.
func burst_sparks(p: Vector2, color: Color, count := 8, speed := 240.0, life := 0.32) -> void:
	for i in count:
		if _spark_n >= SPARK_CAP:
			return
		var ang := _rng.randf() * TAU
		var sp := speed * (0.45 + _rng.randf() * 0.85)
		var lf := life * (0.7 + _rng.randf() * 0.6)
		_spark_pos[_spark_n] = p
		_spark_vel[_spark_n] = Vector2(cos(ang), sin(ang)) * sp
		_spark_life[_spark_n] = lf
		_spark_ttl[_spark_n] = lf
		_spark_col[_spark_n] = color
		_spark_r[_spark_n] = 2.0 + _rng.randf() * 2.5
		_spark_n += 1

# Enemy death: a fuller, slightly slower burst + a shockwave + shake.
func kill_burst(p: Vector2, color: Color) -> void:
	burst_sparks(p, color, 14, 300.0, 0.45)
	shockwave(p, Color(color.r, color.g, color.b, 0.9), 64.0, 0.35)
	add_shake(0.12)

# Expanding ring. `maxr` is the outer radius in pixels at end of life.
func shockwave(p: Vector2, color: Color, max_radius := 80.0, life := 0.4) -> void:
	if _wave_n >= WAVE_CAP:
		return
	_wave_pos[_wave_n] = p
	_wave_life[_wave_n] = life
	_wave_ttl[_wave_n] = life
	_wave_maxr[_wave_n] = max_radius
	_wave_col[_wave_n] = color
	_wave_n += 1

# Floating combat text that rises and fades.
func combat_text(p: Vector2, text: String, color := Color(1, 1, 1), size := 18, rise := 46.0) -> void:
	if _text_n >= TEXT_CAP:
		return
	_text_pos[_text_n] = p
	_text_life[_text_n] = 0.85
	_text_ttl[_text_n] = 0.85
	_text_rise[_text_n] = rise
	_text_size[_text_n] = size
	_text_col[_text_n] = color
	_text_str[_text_n] = text
	_text_n += 1

# --- juice bus ------------------------------------------------------------

func add_shake(amount: float) -> void:
	_trauma = clampf(_trauma + amount, 0.0, 1.2)

func add_hitstop(seconds: float) -> void:
	_hitstop = maxf(_hitstop, seconds)

func add_flash(color: Color, life := 0.18) -> void:
	_flash_col = color
	_flash_ttl = life
	_flash_life = life

func hitstop_active() -> bool:
	return _hitstop > 0.0

# Camera shake offset to add to the world origin this frame. Trauma^2 feels
# better than linear; two desynced sines + noise give an organic jitter.
func shake_offset() -> Vector2:
	if _trauma <= 0.0:
		return Vector2.ZERO
	var amp := _trauma * _trauma * SHAKE_MAX_PIXELS
	return Vector2(
		sin(_seed * 53.7) * amp,
		sin(_seed * 71.3 + 1.7) * amp
	)

# Current full-screen flash tint; alpha 0 means "no flash".
func flash_color() -> Color:
	if _flash_ttl <= 0.0:
		return Color(0, 0, 0, 0)
	var k := _flash_ttl / maxf(_flash_life, 0.0001)
	return Color(_flash_col.r, _flash_col.g, _flash_col.b, _flash_col.a * k)

# --- per-frame advance ----------------------------------------------------

func update(dt: float) -> void:
	_seed += dt * 60.0
	# Hit-stop eats real time but we still bleed it; callers may choose to also
	# slow their own animation while hitstop_active().
	if _hitstop > 0.0:
		_hitstop = maxf(0.0, _hitstop - dt)
	if _trauma > 0.0:
		_trauma = maxf(0.0, _trauma - SHAKE_DECAY * dt)
	if _flash_ttl > 0.0:
		_flash_ttl = maxf(0.0, _flash_ttl - dt)

	# Advance + expire in place: a dead slot i is overwritten by the last live
	# slot (swap-remove keeps the pool dense; order is irrelevant for FX).
	var damp := maxf(0.0, 1.0 - DRAG * dt)
	var i := 0
	while i < _spark_n:
		var ttl := _spark_ttl[i] - dt
		if ttl <= 0.0:
			_spark_n -= 1
			_spark_pos[i] = _spark_pos[_spark_n]
			_spark_vel[i] = _spark_vel[_spark_n]
			_spark_life[i] = _spark_life[_spark_n]
			_spark_ttl[i] = _spark_ttl[_spark_n]
			_spark_r[i] = _spark_r[_spark_n]
			_spark_col[i] = _spark_col[_spark_n]
			continue   # re-examine the swapped-in slot
		_spark_ttl[i] = ttl
		var vel := _spark_vel[i] * damp + Vector2(0, GRAVITY * dt)
		_spark_vel[i] = vel
		_spark_pos[i] += vel * dt
		i += 1

	i = 0
	while i < _wave_n:
		var wttl := _wave_ttl[i] - dt
		if wttl <= 0.0:
			_wave_n -= 1
			_wave_pos[i] = _wave_pos[_wave_n]
			_wave_life[i] = _wave_life[_wave_n]
			_wave_ttl[i] = _wave_ttl[_wave_n]
			_wave_maxr[i] = _wave_maxr[_wave_n]
			_wave_col[i] = _wave_col[_wave_n]
			continue
		_wave_ttl[i] = wttl
		i += 1

	i = 0
	while i < _text_n:
		var tttl := _text_ttl[i] - dt
		if tttl <= 0.0:
			_text_n -= 1
			_text_pos[i] = _text_pos[_text_n]
			_text_life[i] = _text_life[_text_n]
			_text_ttl[i] = _text_ttl[_text_n]
			_text_rise[i] = _text_rise[_text_n]
			_text_size[i] = _text_size[_text_n]
			_text_col[i] = _text_col[_text_n]
			_text_str[i] = _text_str[_text_n]
			continue
		_text_ttl[i] = tttl
		i += 1

# --- draw -----------------------------------------------------------------
# All emitter positions are already screen-space, so `to_screen` is only needed
# by callers; here everything is drawn directly on the canvas.
func draw(canvas: CanvasItem, font := ThemeDB.fallback_font) -> void:
	# shockwave rings (under sparks)
	for i in _wave_n:
		var k: float = 1.0 - _wave_ttl[i] / _wave_life[i]      # 0..1
		var rad := lerpf(6.0, _wave_maxr[i], k)
		var wc := _wave_col[i]
		var a: float = (1.0 - k) * wc.a
		canvas.draw_arc(_wave_pos[i], rad, 0.0, TAU, 28,
			Color(wc.r, wc.g, wc.b, a), maxf(2.0, 6.0 * (1.0 - k)), true)

	# sparks (bright -> bloom under glow)
	for i in _spark_n:
		var k2 := clampf(_spark_ttl[i] / _spark_life[i], 0.0, 1.0)
		var col2 := _spark_col[i]
		# keep it bright until near the end so glow pops, then fade alpha
		var a2 := clampf(k2 * 1.6, 0.0, 1.0)
		var r := _spark_r[i] * (0.4 + 0.6 * k2)
		canvas.draw_circle(_spark_pos[i], r, Color(col2.r, col2.g, col2.b, a2))

	# floating combat text
	for i in _text_n:
		var k3: float = 1.0 - _text_ttl[i] / _text_life[i]
		var p := _text_pos[i] - Vector2(0, _text_rise[i] * k3)
		var a3 := clampf((1.0 - k3) * 1.6, 0.0, 1.0)
		var c := _text_col[i]
		var sz := _text_size[i]
		# cheap drop-shadow for legibility
		canvas.draw_string(font, p + Vector2(1.5, 1.5), _text_str[i],
			HORIZONTAL_ALIGNMENT_CENTER, -1, sz, Color(0, 0, 0, a3 * 0.7))
		canvas.draw_string(font, p, _text_str[i],
			HORIZONTAL_ALIGNMENT_CENTER, -1, sz, Color(c.r, c.g, c.b, a3))
