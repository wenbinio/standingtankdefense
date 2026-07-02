// Standing Tank Defense — screen-space vignette + escalating danger color-grade.
//
// Render-only post pass. Lives on a full-rect ColorRect ABOVE the arena draw
// but BELOW the HUD, using BackBufferCopy via SCREEN_TEXTURE so it grades the
// already-rendered arena. `danger` (0..1) is driven by sim.round() from the
// front-end — a cosmetic read, never fed back into the sim.
shader_type canvas_item;
render_mode unshaded;

uniform sampler2D screen_tex : hint_screen_texture, filter_linear_mipmap;

// 0 = calm, 1 = max danger. Pushes saturation + a red bias and tightens the
// vignette as the rounds climb.
uniform float danger : hint_range(0.0, 1.0) = 0.0;
uniform float vignette_strength : hint_range(0.0, 2.0) = 0.9;
uniform float vignette_softness : hint_range(0.01, 1.5) = 0.55;
uniform vec3 danger_tint : source_color = vec3(1.0, 0.62, 0.5);
// P3 tank feedback (both 0..1, driven per-frame by arena_renderer._process):
//  - hurt: spikes to 1 on TankHit, decays over ~0.4 s — a hard red vignette
//    clamp-down the instant the tank takes damage.
//  - heartbeat: low-HP pulse (below 33% HP), a slow red thump whose depth and
//    rate scale as HP drops. COMPOSES with the round-danger grade above; it
//    never replaces it.
uniform float hurt : hint_range(0.0, 1.0) = 0.0;
uniform float heartbeat : hint_range(0.0, 1.0) = 0.0;

vec3 adjust_saturation(vec3 c, float sat) {
	float l = dot(c, vec3(0.2126, 0.7152, 0.0722));
	return mix(vec3(l), c, sat);
}

void fragment() {
	vec3 col = texture(screen_tex, SCREEN_UV).rgb;

	// --- escalating color grade -------------------------------------------
	// Saturation creeps up and a red/orange bias pushes in with danger.
	float sat = 1.0 + danger * 0.45;
	col = adjust_saturation(col, sat);
	col = mix(col, col * danger_tint, danger * 0.35);
	// slight overall contrast lift under danger for a harsher look
	col = mix(col, smoothstep(vec3(0.0), vec3(1.0), col), danger * 0.18);

	// --- screen-space vignette --------------------------------------------
	vec2 d = SCREEN_UV - vec2(0.5);
	// aspect-correct so the falloff is round, not stretched
	float dist = length(d * vec2(1.0, 1.0)) * 1.41421356;
	float vig = smoothstep(1.0, 1.0 - vignette_softness, dist);
	// danger tightens & darkens the vignette; the hurt spike and the low-HP
	// heartbeat tighten it further (composed, not replacing danger).
	float red_pulse = clamp(hurt + heartbeat, 0.0, 1.0);
	float strength = vignette_strength * (1.0 + danger * 0.6 + red_pulse * 0.9);
	vig = mix(1.0, vig, clamp(strength, 0.0, 1.0));
	col *= vig;

	// faint danger-colored darkening in the corners (menace creeping in)
	float corner = (1.0 - vig);
	col = mix(col, col * danger_tint * 0.4, corner * danger * 0.5);

	// blood-red edge wash for tank damage / low HP (screen-edge flash)
	col = mix(col, vec3(0.62, 0.05, 0.04), corner * red_pulse * 0.65);

	COLOR = vec4(col, 1.0);
}
