# FxOverlay — screen-space FX canvas (Main.tscn, first child of the UI layer).
# Draws the pooled Fx bus (impact sparks, kill bursts, shockwaves, combat text)
# and the full-screen flash pulse ABOVE the world but BENEATH the HUD, exactly
# where main.gd used to draw them. On the UI CanvasLayer so the camera's shake
# offset does not move it (Fx emitter positions are already screen-space).
extends Node2D

var fx: Fx = null             # shared juice bus (wired by main.gd)
var _font: Font = null

func _ready() -> void:
	_font = ArtTheme.ui_font(false)
	if _font == null:
		_font = ThemeDB.fallback_font

func _process(_delta: float) -> void:
	queue_redraw()

func _draw() -> void:
	if fx == null:
		return
	fx.draw(self, _font)
	# Full-screen flash pulse on big hits / Clear.
	var fc := fx.flash_color()
	if fc.a > 0.001:
		draw_rect(Rect2(Vector2.ZERO, get_viewport_rect().size), fc)
