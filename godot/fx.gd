# Reusable, allocation-light render-only juice helper for Standing Tank Defense.
#
# This is the "juice bus": a pooled custom-`_draw` particle emitter plus
# screen-shake, hit-stop and flash accumulators, and floating combat text.
# It is PURELY cosmetic — it reads nothing from the sim and feeds nothing back.
# All animation runs on wall-clock / frame counters; never on sim ticks.
#
# Public seam (used by main.gd; safe to reuse from match.gd):
#   var fx := Fx.new()                     # construct one per arena view
#   fx.update(dt)                          # advance every frame (pass real delta)
#   fx.draw(canvas, to_screen, font)       # draw all FX (call inside _draw)
#       to_screen: Callable(Vector2 world) -> Vector2 screen
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
# A particle is a flat Dictionary; pools are plain Arrays we filter in place.
class_name Fx
extends RefCounted

# --- tunables -------------------------------------------------------------
const SHAKE_DECAY := 1.8           # trauma units per second bled off
const SHAKE_MAX_PIXELS := 12.0
const GRAVITY := 520.0             # px/s^2 for spark fall (screen space)
const DRAG := 2.6                  # velocity damping per second

# --- pools ----------------------------------------------------------------
var _sparks: Array = []            # {pos,vel,life,ttl,col,r}
var _waves: Array = []             # {pos,life,ttl,col,maxr}
var _texts: Array = []             # {pos,life,ttl,col,str,size,rise}

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

# --- emitters -------------------------------------------------------------

# A radial spray of small glowing shards. `color` should be bright (>1 channels
# bloom under glow). Cheap: count defaults small.
func burst_sparks(p: Vector2, color: Color, count := 8, speed := 240.0, life := 0.32) -> void:
	for i in count:
		var ang := _rng.randf() * TAU
		var sp := speed * (0.45 + _rng.randf() * 0.85)
		_sparks.append({
			"pos": p,
			"vel": Vector2(cos(ang), sin(ang)) * sp,
			"life": life * (0.7 + _rng.randf() * 0.6),
			"ttl": life * (0.7 + _rng.randf() * 0.6),
			"col": color,
			"r": 2.0 + _rng.randf() * 2.5,
		})

# Enemy death: a fuller, slightly slower burst + a shockwave + shake.
func kill_burst(p: Vector2, color: Color) -> void:
	burst_sparks(p, color, 14, 300.0, 0.45)
	shockwave(p, Color(color.r, color.g, color.b, 0.9), 64.0, 0.35)
	add_shake(0.12)

# Expanding ring. `maxr` is the outer radius in pixels at end of life.
func shockwave(p: Vector2, color: Color, max_radius := 80.0, life := 0.4) -> void:
	_waves.append({"pos": p, "life": life, "ttl": life, "col": color, "maxr": max_radius})

# Floating combat text that rises and fades.
func combat_text(p: Vector2, text: String, color := Color(1, 1, 1), size := 18, rise := 46.0) -> void:
	_texts.append({
		"pos": p, "life": 0.85, "ttl": 0.85, "col": color,
		"str": text, "size": size, "rise": rise,
	})

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

	var damp := maxf(0.0, 1.0 - DRAG * dt)
	for s in _sparks:
		s["ttl"] -= dt
		s["vel"] = s["vel"] * damp + Vector2(0, GRAVITY * dt)
		s["pos"] += s["vel"] * dt
	_sparks = _sparks.filter(func(s): return s["ttl"] > 0.0)

	for w in _waves:
		w["ttl"] -= dt
	_waves = _waves.filter(func(w): return w["ttl"] > 0.0)

	for tx in _texts:
		tx["ttl"] -= dt
	_texts = _texts.filter(func(tx): return tx["ttl"] > 0.0)

# --- draw -----------------------------------------------------------------
# All emitter positions are already screen-space, so `to_screen` is only needed
# by callers; here everything is drawn directly on the canvas.
func draw(canvas: CanvasItem, font := ThemeDB.fallback_font) -> void:
	# shockwave rings (under sparks)
	for w in _waves:
		var k: float = 1.0 - w["ttl"] / w["life"]      # 0..1
		var rad: float = lerpf(6.0, w["maxr"], k)
		var a: float = (1.0 - k) * w["col"].a
		var col: Color = Color(w["col"].r, w["col"].g, w["col"].b, a)
		canvas.draw_arc(w["pos"], rad, 0.0, TAU, 28, col, maxf(2.0, 6.0 * (1.0 - k)), true)

	# sparks (bright -> bloom under glow)
	for s in _sparks:
		var k2: float = clampf(s["ttl"] / s["life"], 0.0, 1.0)
		var col2: Color = s["col"]
		# keep it bright until near the end so glow pops, then fade alpha
		var a2: float = clampf(k2 * 1.6, 0.0, 1.0)
		var r: float = s["r"] * (0.4 + 0.6 * k2)
		canvas.draw_circle(s["pos"], r, Color(col2.r, col2.g, col2.b, a2))

	# floating combat text
	for tx in _texts:
		var k3: float = 1.0 - tx["ttl"] / tx["life"]
		var p: Vector2 = tx["pos"] - Vector2(0, tx["rise"] * k3)
		var a3: float = clampf((1.0 - k3) * 1.6, 0.0, 1.0)
		var c: Color = tx["col"]
		var sz: int = tx["size"]
		# cheap drop-shadow for legibility
		canvas.draw_string(font, p + Vector2(1.5, 1.5), tx["str"],
			HORIZONTAL_ALIGNMENT_CENTER, -1, sz, Color(0, 0, 0, a3 * 0.7))
		canvas.draw_string(font, p, tx["str"],
			HORIZONTAL_ALIGNMENT_CENTER, -1, sz, Color(c.r, c.g, c.b, a3))
