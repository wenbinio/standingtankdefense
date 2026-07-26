--!nonstrict
--[[
	Content.luau — the M0 catalog (`CONTRACTS.md` C1), exposing the tables and
	accessors of `sim-core/crates/sim/src/content.rs`. Spec: `roblox/SIM-SPEC.md`
	§S2/§S3.

	The Rust tables remain the single source of truth; `content.json` is the build
	artifact generated from them and this module is its reader. Nothing here
	invents a number — every value comes out of the JSON, and every derived value
	(`damageMultiplier`, `enemyHpMult`, the scope ids, `is_meta`/`is_economy`) is a
	transliteration of the corresponding `content.rs` function.

	WHY THE JSON IS EMBEDDED
	------------------------
	Standalone Luau has no file I/O, and this layer may not touch a Roblox global
	(`CONTRACTS.md` C5), so there is no way to *read* `src/shared/content.json`
	from here. Every sim module does a bare `require("../Content")` and reads
	constants at module scope, so the catalog must be present the moment this file
	returns. It therefore ships as a verbatim copy of `content.json` in `RAW_JSON`
	and self-loads at require time.

	THE COPY IS CHECKED, NOT TRUSTED: `test/content_test.luau` asserts `RAW_JSON`
	is byte-identical to `src/shared/content.json`, so a regenerated artifact
	cannot silently leave this module stale. `Content.load(source)` re-loads from
	any source (the JSON text, or the table Rojo's `.json` ModuleScript returns),
	for a host that would rather point the sim at the synced instance.

	INDEXING
	--------
	Content ids are 0-BASED (array position IS identity, C1 / §S3); Luau arrays are
	1-INDEXED. This module stores plain 1-indexed arrays and callers do the offset,
	matching the convention already established across the sim modules:

	    Content.WEAPONS[inst.def + 1]      -- Rust: WEAPONS[w.def as usize]
	    Content.SPAWN_RING[ringIdx + 1]    -- Rust: SPAWN_RING[ring_idx]

	Every entry also carries its own 0-based `id`, so a def can be recovered from a
	definition without tracking the offset back.

	NAMES
	-----
	Tables mirror the Rust statics in UPPER_SNAKE (`WEAPONS`, `ENEMIES`,
	`MODIFIERS`, `WAVES`/`WAVE_M0`, `BOSS_ESCORT`, `SPAWN_RING`, `DAMAGE_MATRIX`,
	`TIMELINE`); functions are camelCase per §S3. lowerCamel aliases (`weapons`,
	`enemies`, `modifiers`, `waves`, `bossEscort`, `spawnRing`, `timeline`) point
	at the SAME objects, because both spellings are in use across the sim modules.

	Scalar constants are available BOTH as the raw `content.json` passthrough
	(`Content.constants.frost_max_stacks`) and flattened to the top level in
	UPPER_SNAKE mirroring `content.rs`'s own identifiers
	(`Content.FROST_MAX_STACKS`). The flat names are DERIVED from `constants` and
	the Steam timeline at load, never hand-listed, so they cannot drift apart.

	PAYLOAD ENUMS
	-------------
	`Attack`, `WeaponAbility`, `EnemyAbility` and `ModEffect` ship as the C1
	`{tag, a, b, c}` WORDS form. They are decoded into tables that KEEP the words
	and add the Rust variant name plus named operands:

	    { variant = "Hazard", tag = 6, a = 40, b = 300, c = 90,
	      dmg = 40, radius = 300, ticks = 90 }

	`variant` is the exact Rust variant name — branch on `variant` or on `tag`.
	`tag/a/b/c` are kept because `lib.rs::checksum` hashes `effect.words()` /
	`ability.words()` verbatim: a decoded value must reproduce its own encoding.

	TIMELINES
	---------
	BOTH ship — `Content.TIMELINE.steam` and `.roblox` (`docs/10` F1). Which one is
	active is the caller's decision; only the STEAM values are flattened to bare
	names, because those are the constants the Rust itself compiles in and the
	oracle runs. The fork's rescaled numbers are reachable only via
	`Content.TIMELINE.roblox.*`, so nothing picks them up by accident.

	ENGINE INDEPENDENCE
	-------------------
	References no Roblox global. Requires only its siblings `Fixed.luau` and
	`Json.luau`, by relative path.
]]

local Fixed = require("./Fixed")
local Json = require("./Json")

-- A verbatim copy of `src/shared/content.json` (see the header). Asserted
-- byte-identical to that file by `test/content_test.luau`.
local RAW_JSON = [==[
{
  "schema_version": 1,
  "content_hash": "457de647f39181a4",
  "damage_matrix": [
    [65536,65536,32768],
    [131072,65536,22937],
    [65536,131072,32768],
    [65536,32768,98304],
    [65536,65536,65536]
  ],
  "weapons": [
    {"name":"Bow","rarity":0,"cost":500,"damage":75,"damage_type":1,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":30,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Mortar Launcher","rarity":0,"cost":500,"damage":340,"damage_type":3,"attack":{"tag":1,"a":300,"b":0,"c":0},"cooldown_ticks":48,"range":1200,"proj_speed":30,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Frost Bow","rarity":1,"cost":1500,"damage":130,"damage_type":2,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":15,"range":900,"proj_speed":50,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":7,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Poison Bow","rarity":0,"cost":500,"damage":60,"damage_type":1,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":30,"range":900,"proj_speed":45,"on_hit":{"poison_dps":20,"poison_ticks":90,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Flamecaster","rarity":1,"cost":1500,"damage":110,"damage_type":4,"attack":{"tag":1,"a":200,"b":0,"c":0},"cooldown_ticks":20,"range":600,"proj_speed":40,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":6,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Storm Hammer","rarity":2,"cost":3000,"damage":850,"damage_type":2,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":48,"range":600,"proj_speed":40,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":45},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Ballista","rarity":1,"cost":1500,"damage":150,"damage_type":3,"attack":{"tag":2,"a":4,"b":0,"c":0},"cooldown_ticks":27,"range":1200,"proj_speed":50,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Immolation","rarity":1,"cost":1500,"damage":80,"damage_type":4,"attack":{"tag":3,"a":300,"b":0,"c":0},"cooldown_ticks":30,"range":300,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":2,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Shockwave Axe","rarity":2,"cost":3000,"damage":900,"damage_type":0,"attack":{"tag":4,"a":300,"b":0,"c":0},"cooldown_ticks":60,"range":300,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Moon Glaive","rarity":1,"cost":1500,"damage":175,"damage_type":1,"attack":{"tag":5,"a":4,"b":0,"c":0},"cooldown_ticks":30,"range":600,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Death Engine","rarity":2,"cost":3000,"damage":260,"damage_type":4,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":27,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Magic Missile","rarity":0,"cost":500,"damage":100,"damage_type":0,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":27,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Boulder","rarity":0,"cost":500,"damage":110,"damage_type":3,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":27,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Magic Bolt","rarity":0,"cost":500,"damage":100,"damage_type":2,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":27,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Chaos Orb","rarity":0,"cost":500,"damage":95,"damage_type":4,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":27,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Throwing Axes","rarity":0,"cost":500,"damage":55,"damage_type":1,"attack":{"tag":5,"a":3,"b":0,"c":0},"cooldown_ticks":21,"range":300,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Chaos Skulls","rarity":0,"cost":500,"damage":50,"damage_type":4,"attack":{"tag":5,"a":3,"b":0,"c":0},"cooldown_ticks":21,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Suckula","rarity":2,"cost":3000,"damage":1100,"damage_type":4,"attack":{"tag":4,"a":300,"b":0,"c":0},"cooldown_ticks":75,"range":300,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":1,"a":40,"b":0,"c":0}},
    {"name":"Missile Barrage","rarity":3,"cost":5000,"damage":1400,"damage_type":1,"attack":{"tag":2,"a":8,"b":0,"c":0},"cooldown_ticks":75,"range":1200,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":90},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Seeker Axe","rarity":0,"cost":500,"damage":60,"damage_type":1,"attack":{"tag":5,"a":3,"b":0,"c":0},"cooldown_ticks":27,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Steam Cannon","rarity":1,"cost":1500,"damage":320,"damage_type":3,"attack":{"tag":1,"a":300,"b":0,"c":0},"cooldown_ticks":30,"range":300,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":15},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Demon Eye","rarity":2,"cost":3000,"damage":1050,"damage_type":4,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":24,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":5,"a":10,"b":0,"c":0}},
    {"name":"Impaler","rarity":1,"cost":1500,"damage":240,"damage_type":1,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":15,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":22},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Chaos Swarm","rarity":0,"cost":500,"damage":105,"damage_type":4,"attack":{"tag":1,"a":300,"b":0,"c":0},"cooldown_ticks":30,"range":1200,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Catapult","rarity":1,"cost":1500,"damage":380,"damage_type":3,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":24,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Slap","rarity":3,"cost":5000,"damage":2600,"damage_type":1,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":21,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":3,"a":300,"b":0,"c":0}},
    {"name":"Crippler","rarity":2,"cost":3000,"damage":1150,"damage_type":1,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":24,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":5,"a":8,"b":0,"c":0}},
    {"name":"Lifeleecher","rarity":2,"cost":3000,"damage":850,"damage_type":0,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":45,"range":1200,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":1,"a":40,"b":0,"c":0}},
    {"name":"Spell Glaive","rarity":1,"cost":1500,"damage":260,"damage_type":2,"attack":{"tag":5,"a":4,"b":0,"c":0},"cooldown_ticks":36,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Glaive Thrower","rarity":0,"cost":500,"damage":70,"damage_type":0,"attack":{"tag":5,"a":3,"b":0,"c":0},"cooldown_ticks":27,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Spikewheel Launcher","rarity":1,"cost":1500,"damage":230,"damage_type":3,"attack":{"tag":5,"a":6,"b":0,"c":0},"cooldown_ticks":42,"range":1200,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Meatapult","rarity":2,"cost":3000,"damage":820,"damage_type":0,"attack":{"tag":1,"a":300,"b":0,"c":0},"cooldown_ticks":45,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":60},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Arcane Blaster","rarity":1,"cost":1500,"damage":360,"damage_type":2,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":30,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":45},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Quills","rarity":1,"cost":1500,"damage":230,"damage_type":1,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":24,"range":900,"proj_speed":45,"on_hit":{"poison_dps":8,"poison_ticks":90,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Living Spittle","rarity":1,"cost":1500,"damage":230,"damage_type":2,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":24,"range":900,"proj_speed":45,"on_hit":{"poison_dps":8,"poison_ticks":90,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Poison Bomb","rarity":1,"cost":1500,"damage":320,"damage_type":3,"attack":{"tag":1,"a":300,"b":0,"c":0},"cooldown_ticks":36,"range":1200,"proj_speed":45,"on_hit":{"poison_dps":8,"poison_ticks":90,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Serpent","rarity":2,"cost":3000,"damage":700,"damage_type":0,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":24,"range":600,"proj_speed":45,"on_hit":{"poison_dps":22,"poison_ticks":90,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Overloaded Catapult","rarity":2,"cost":3000,"damage":780,"damage_type":3,"attack":{"tag":1,"a":300,"b":0,"c":0},"cooldown_ticks":36,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Chaos Claw","rarity":1,"cost":1500,"damage":320,"damage_type":4,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":24,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":45},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Net Thrower","rarity":1,"cost":1500,"damage":230,"damage_type":0,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":15,"range":1200,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":22},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Thornburst","rarity":0,"cost":500,"damage":60,"damage_type":1,"attack":{"tag":3,"a":400,"b":0,"c":0},"cooldown_ticks":45,"range":400,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Chaotic Spirit","rarity":3,"cost":5000,"damage":1600,"damage_type":2,"attack":{"tag":5,"a":8,"b":0,"c":0},"cooldown_ticks":75,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Energy Pulse","rarity":2,"cost":3000,"damage":900,"damage_type":2,"attack":{"tag":4,"a":300,"b":0,"c":0},"cooldown_ticks":75,"range":300,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":60},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Cluster Rockets","rarity":2,"cost":3000,"damage":520,"damage_type":4,"attack":{"tag":2,"a":12,"b":0,"c":0},"cooldown_ticks":75,"range":1200,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Frost Bomb","rarity":2,"cost":3000,"damage":360,"damage_type":1,"attack":{"tag":1,"a":150,"b":0,"c":0},"cooldown_ticks":18,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":3,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Bouncy Cannonball","rarity":2,"cost":3000,"damage":620,"damage_type":0,"attack":{"tag":5,"a":4,"b":0,"c":0},"cooldown_ticks":45,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":90},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Soulstealer","rarity":3,"cost":5000,"damage":2400,"damage_type":0,"attack":{"tag":5,"a":4,"b":0,"c":0},"cooldown_ticks":36,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":1,"a":200,"b":0,"c":0}},
    {"name":"Splasher","rarity":0,"cost":500,"damage":65,"damage_type":0,"attack":{"tag":1,"a":300,"b":0,"c":0},"cooldown_ticks":15,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Fire Bow","rarity":1,"cost":1500,"damage":190,"damage_type":1,"attack":{"tag":2,"a":4,"b":0,"c":0},"cooldown_ticks":30,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":4,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Chaos Web","rarity":1,"cost":1500,"damage":180,"damage_type":4,"attack":{"tag":5,"a":6,"b":0,"c":0},"cooldown_ticks":36,"range":600,"proj_speed":45,"on_hit":{"poison_dps":6,"poison_ticks":90,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Magic Claw","rarity":1,"cost":1500,"damage":240,"damage_type":2,"attack":{"tag":5,"a":4,"b":0,"c":0},"cooldown_ticks":30,"range":300,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":2,"a":20,"b":0,"c":0}},
    {"name":"Liquid Fire Hurler","rarity":1,"cost":1500,"damage":210,"damage_type":3,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":10,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":4,"stun_ticks":0},"ability":{"tag":5,"a":3,"b":0,"c":0}},
    {"name":"Boulder Toss","rarity":2,"cost":3000,"damage":980,"damage_type":0,"attack":{"tag":1,"a":300,"b":0,"c":0},"cooldown_ticks":45,"range":300,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":5,"a":5,"b":0,"c":0}},
    {"name":"Bloody Spikes","rarity":3,"cost":5000,"damage":2200,"damage_type":0,"attack":{"tag":4,"a":300,"b":0,"c":0},"cooldown_ticks":36,"range":300,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":30},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Shroom Doom","rarity":3,"cost":5000,"damage":3200,"damage_type":4,"attack":{"tag":3,"a":300,"b":0,"c":0},"cooldown_ticks":90,"range":600,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":60,"stun_ticks":60},"ability":{"tag":7,"a":1,"b":1500,"c":600}},
    {"name":"Flame Generator","rarity":3,"cost":5000,"damage":1700,"damage_type":2,"attack":{"tag":3,"a":300,"b":0,"c":0},"cooldown_ticks":60,"range":600,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":200,"stun_ticks":0},"ability":{"tag":5,"a":4,"b":0,"c":0}},
    {"name":"Firebreather","rarity":1,"cost":1500,"damage":180,"damage_type":1,"attack":{"tag":1,"a":150,"b":0,"c":0},"cooldown_ticks":15,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":5,"stun_ticks":0},"ability":{"tag":5,"a":5,"b":0,"c":0}},
    {"name":"Lavaspitter","rarity":3,"cost":5000,"damage":1800,"damage_type":3,"attack":{"tag":1,"a":300,"b":0,"c":0},"cooldown_ticks":45,"range":1200,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":150,"stun_ticks":0},"ability":{"tag":5,"a":5,"b":0,"c":0}},
    {"name":"Frostbolt","rarity":1,"cost":1500,"damage":170,"damage_type":1,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":15,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":5,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Living Ice","rarity":1,"cost":1500,"damage":300,"damage_type":2,"attack":{"tag":1,"a":150,"b":0,"c":0},"cooldown_ticks":30,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":2,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Ice Generator","rarity":3,"cost":5000,"damage":1000,"damage_type":2,"attack":{"tag":3,"a":375,"b":0,"c":0},"cooldown_ticks":45,"range":900,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":5,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Ice Spears","rarity":2,"cost":3000,"damage":440,"damage_type":0,"attack":{"tag":2,"a":4,"b":0,"c":0},"cooldown_ticks":36,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":5,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Knives","rarity":0,"cost":500,"damage":80,"damage_type":1,"attack":{"tag":2,"a":3,"b":0,"c":0},"cooldown_ticks":18,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Blaster","rarity":1,"cost":1500,"damage":210,"damage_type":3,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":15,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":5,"a":5,"b":0,"c":0}},
    {"name":"Bandit Sniper","rarity":1,"cost":1500,"damage":240,"damage_type":0,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":18,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":5,"a":5,"b":0,"c":0}},
    {"name":"Bombs","rarity":1,"cost":1500,"damage":300,"damage_type":3,"attack":{"tag":1,"a":300,"b":0,"c":0},"cooldown_ticks":30,"range":300,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":15},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Sting","rarity":1,"cost":1500,"damage":300,"damage_type":4,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":24,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":5,"a":10,"b":0,"c":0}},
    {"name":"Chaos Skull Bomb","rarity":2,"cost":3000,"damage":720,"damage_type":4,"attack":{"tag":1,"a":300,"b":0,"c":0},"cooldown_ticks":45,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":45},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Icebreather","rarity":2,"cost":3000,"damage":760,"damage_type":3,"attack":{"tag":1,"a":300,"b":0,"c":0},"cooldown_ticks":45,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":2,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":5,"a":4,"b":0,"c":0}},
    {"name":"Frostwave","rarity":2,"cost":3000,"damage":720,"damage_type":2,"attack":{"tag":4,"a":150,"b":0,"c":0},"cooldown_ticks":45,"range":300,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":3,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Flamewave","rarity":1,"cost":1500,"damage":360,"damage_type":0,"attack":{"tag":4,"a":200,"b":0,"c":0},"cooldown_ticks":45,"range":300,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":20,"stun_ticks":0},"ability":{"tag":5,"a":6,"b":0,"c":0}},
    {"name":"Chaotic Spirit Bolt","rarity":1,"cost":1500,"damage":190,"damage_type":4,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":10,"range":600,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":1,"a":40,"b":0,"c":0}},
    {"name":"Manabolt","rarity":1,"cost":1500,"damage":320,"damage_type":2,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":10,"range":1200,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":2,"a":80,"b":0,"c":0}},
    {"name":"Squirm","rarity":3,"cost":5000,"damage":1300,"damage_type":4,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":18,"range":1200,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":7,"a":0,"b":500,"c":250}},
    {"name":"Immolation Aura","rarity":0,"cost":500,"damage":75,"damage_type":2,"attack":{"tag":4,"a":150,"b":0,"c":0},"cooldown_ticks":12,"range":300,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":2,"stun_ticks":0},"ability":{"tag":5,"a":3,"b":0,"c":0}},
    {"name":"Boom Bloom","rarity":3,"cost":5000,"damage":2600,"damage_type":3,"attack":{"tag":4,"a":200,"b":0,"c":0},"cooldown_ticks":45,"range":300,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":90},"ability":{"tag":6,"a":1000,"b":200,"c":90}},
    {"name":"Quill Burst","rarity":1,"cost":1500,"damage":340,"damage_type":1,"attack":{"tag":1,"a":300,"b":0,"c":0},"cooldown_ticks":45,"range":1200,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":5,"a":10,"b":0,"c":0}},
    {"name":"Arcane Burst","rarity":1,"cost":1500,"damage":360,"damage_type":2,"attack":{"tag":1,"a":300,"b":0,"c":0},"cooldown_ticks":45,"range":1200,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":5,"a":10,"b":0,"c":0}},
    {"name":"Meteor Barrage","rarity":3,"cost":5000,"damage":3400,"damage_type":3,"attack":{"tag":2,"a":8,"b":0,"c":0},"cooldown_ticks":120,"range":1200,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Ale Launcher","rarity":1,"cost":1500,"damage":230,"damage_type":3,"attack":{"tag":1,"a":200,"b":0,"c":0},"cooldown_ticks":15,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":1,"a":4,"b":0,"c":0}},
    {"name":"Chaos Bolt","rarity":1,"cost":1500,"damage":290,"damage_type":4,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":10,"range":1200,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Rotating Orb of Lightning","rarity":2,"cost":3000,"damage":360,"damage_type":2,"attack":{"tag":3,"a":600,"b":0,"c":0},"cooldown_ticks":30,"range":600,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Lightning Generator","rarity":2,"cost":3000,"damage":780,"damage_type":2,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":30,"range":1200,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":2,"a":20,"b":0,"c":0}},
    {"name":"Flame Nova","rarity":1,"cost":1500,"damage":220,"damage_type":4,"attack":{"tag":3,"a":300,"b":0,"c":0},"cooldown_ticks":36,"range":300,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":20,"stun_ticks":0},"ability":{"tag":5,"a":6,"b":0,"c":0}},
    {"name":"Shocker","rarity":1,"cost":1500,"damage":130,"damage_type":3,"attack":{"tag":3,"a":300,"b":0,"c":0},"cooldown_ticks":18,"range":600,"proj_speed":0,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":60},"ability":{"tag":0,"a":0,"b":0,"c":0}},
    {"name":"Tangle","rarity":3,"cost":5000,"damage":1300,"damage_type":0,"attack":{"tag":0,"a":0,"b":0,"c":0},"cooldown_ticks":12,"range":900,"proj_speed":45,"on_hit":{"poison_dps":0,"poison_ticks":0,"frost_stacks":0,"fire_stacks":0,"stun_ticks":0},"ability":{"tag":4,"a":30,"b":0,"c":0}}
  ],
  "enemies": [
    {"name":"Squeakzilla","base_hp":200,"move_speed":8,"contact_damage":500,"bounty":10,"armor_class":0,"archetype":0,"ability":{"tag":0,"a":0,"b":0,"c":0},"boss":false},
    {"name":"Fanged Death","base_hp":1200,"move_speed":4,"contact_damage":1500,"bounty":40,"armor_class":1,"archetype":1,"ability":{"tag":0,"a":0,"b":0,"c":0},"boss":false},
    {"name":"The Hippocrate","base_hp":33000000,"move_speed":3,"contact_damage":45000,"bounty":0,"armor_class":0,"archetype":5,"ability":{"tag":0,"a":0,"b":0,"c":0},"boss":true},
    {"name":"Doomduck","base_hp":120,"move_speed":8,"contact_damage":350,"bounty":6,"armor_class":0,"archetype":0,"ability":{"tag":0,"a":0,"b":0,"c":0},"boss":false},
    {"name":"Bacon","base_hp":260,"move_speed":16,"contact_damage":700,"bounty":16,"armor_class":0,"archetype":2,"ability":{"tag":0,"a":0,"b":0,"c":0},"boss":false},
    {"name":"Honk","base_hp":180,"move_speed":22,"contact_damage":600,"bounty":18,"armor_class":0,"archetype":2,"ability":{"tag":0,"a":0,"b":0,"c":0},"boss":false},
    {"name":"Bonk","base_hp":4000,"move_speed":3,"contact_damage":2200,"bounty":80,"armor_class":2,"archetype":1,"ability":{"tag":0,"a":0,"b":0,"c":0},"boss":false},
    {"name":"Nope Rope","base_hp":320,"move_speed":6,"contact_damage":200,"bounty":30,"armor_class":0,"archetype":3,"ability":{"tag":1,"a":900,"b":8589934637,"c":250},"boss":false},
    {"name":"Croak","base_hp":300,"move_speed":6,"contact_damage":200,"bounty":22,"armor_class":0,"archetype":4,"ability":{"tag":1,"a":700,"b":4294967326,"c":180},"boss":false},
    {"name":"Spicy","base_hp":600,"move_speed":5,"contact_damage":300,"bounty":34,"armor_class":0,"archetype":4,"ability":{"tag":1,"a":600,"b":17179869220,"c":420},"boss":false},
    {"name":"Popsicle","base_hp":900,"move_speed":4,"contact_damage":350,"bounty":40,"armor_class":1,"archetype":4,"ability":{"tag":1,"a":650,"b":12884901936,"c":500},"boss":false},
    {"name":"Dodo","base_hp":800,"move_speed":0,"contact_damage":0,"bounty":12,"armor_class":0,"archetype":6,"ability":{"tag":0,"a":0,"b":0,"c":0},"boss":false}
  ],
  "modifiers": [
    {"name":"+10% Damage","rarity":0,"cost":500,"effects":[{"tag":0,"a":1,"b":10,"c":0}],"ramp":null},
    {"name":"Improved Piercing Attacks","rarity":0,"cost":500,"effects":[{"tag":1,"a":1,"b":1,"c":10}],"ramp":null},
    {"name":"Improved Siege Attacks","rarity":0,"cost":500,"effects":[{"tag":1,"a":3,"b":1,"c":10}],"ramp":null},
    {"name":"Improved Magic Attacks","rarity":0,"cost":500,"effects":[{"tag":1,"a":2,"b":1,"c":10}],"ramp":null},
    {"name":"+25% Damage (Epic)","rarity":3,"cost":5000,"effects":[{"tag":2,"a":2,"b":5,"c":0}],"ramp":null},
    {"name":"Rapidfire","rarity":0,"cost":500,"effects":[{"tag":3,"a":1,"b":10,"c":0}],"ramp":null},
    {"name":"Bounty Hunter","rarity":1,"cost":1500,"effects":[{"tag":4,"a":1,"b":2,"c":0}],"ramp":null},
    {"name":"Entangled Gold Mine","rarity":0,"cost":500,"effects":[{"tag":5,"a":20,"b":0,"c":0},{"tag":18,"a":25,"b":100,"c":0}],"ramp":null},
    {"name":"+10% Gold Income","rarity":1,"cost":1000,"effects":[{"tag":17,"a":20,"b":100,"c":0}],"ramp":null},
    {"name":"+25% Gold Income","rarity":2,"cost":2000,"effects":[{"tag":17,"a":50,"b":100,"c":0}],"ramp":null},
    {"name":"Transmute","rarity":2,"cost":3000,"effects":[{"tag":4,"a":100,"b":100,"c":0},{"tag":19,"a":5,"b":200,"c":0}],"ramp":null},
    {"name":"Imbued Masonry","rarity":1,"cost":1500,"effects":[{"tag":6,"a":2000,"b":0,"c":0},{"tag":35,"a":25,"b":100,"c":0}],"ramp":null},
    {"name":"+10 Armor","rarity":0,"cost":500,"effects":[{"tag":7,"a":10,"b":0,"c":0}],"ramp":null},
    {"name":"Moonwell","rarity":1,"cost":1500,"effects":[{"tag":8,"a":2000,"b":10,"c":0}],"ramp":null},
    {"name":"+50 HP Regen","rarity":0,"cost":500,"effects":[{"tag":9,"a":50,"b":0,"c":0}],"ramp":null},
    {"name":"Evasion","rarity":1,"cost":1500,"effects":[{"tag":10,"a":10,"b":0,"c":0}],"ramp":null},
    {"name":"Power Generator","rarity":2,"cost":3000,"effects":[{"tag":0,"a":2,"b":100,"c":0}],"ramp":{"effect":{"tag":0,"a":1,"b":100,"c":0},"interval_ticks":900}},
    {"name":"Scroll of Chaos","rarity":2,"cost":3000,"effects":[{"tag":1,"a":4,"b":20,"c":100}],"ramp":{"effect":{"tag":1,"a":4,"b":3,"c":100},"interval_ticks":900}},
    {"name":"Compounding Greed (+10 Income, +5/round)","rarity":1,"cost":1500,"effects":[{"tag":5,"a":10,"b":0,"c":0}],"ramp":{"effect":{"tag":5,"a":5,"b":0,"c":0},"interval_ticks":900}},
    {"name":"Blessed Armor","rarity":1,"cost":1500,"effects":[{"tag":7,"a":10,"b":0,"c":0}],"ramp":{"effect":{"tag":7,"a":5,"b":0,"c":0},"interval_ticks":900}},
    {"name":"Focusfire","rarity":1,"cost":1500,"effects":[{"tag":11,"a":0,"b":25,"c":100}],"ramp":null},
    {"name":"+25% Splash Damage","rarity":1,"cost":1500,"effects":[{"tag":11,"a":1,"b":25,"c":100}],"ramp":null},
    {"name":"+25% Barrage Damage","rarity":1,"cost":1500,"effects":[{"tag":11,"a":2,"b":25,"c":100}],"ramp":null},
    {"name":"+25% Area Damage","rarity":1,"cost":1500,"effects":[{"tag":11,"a":3,"b":25,"c":100}],"ramp":null},
    {"name":"Wavefire","rarity":1,"cost":1500,"effects":[{"tag":11,"a":4,"b":25,"c":100}],"ramp":null},
    {"name":"+25% Bounce Damage","rarity":1,"cost":1500,"effects":[{"tag":11,"a":5,"b":25,"c":100}],"ramp":null},
    {"name":"Command Aura","rarity":1,"cost":1500,"effects":[{"tag":11,"a":6,"b":25,"c":100}],"ramp":null},
    {"name":"Trueshot Aura","rarity":1,"cost":1500,"effects":[{"tag":11,"a":7,"b":25,"c":100}],"ramp":null},
    {"name":"Engineering Upgrade","rarity":1,"cost":1500,"effects":[{"tag":11,"a":8,"b":100,"c":100}],"ramp":null},
    {"name":"Bash","rarity":1,"cost":1500,"effects":[{"tag":20,"a":20,"b":100,"c":0}],"ramp":null},
    {"name":"Corrosive Poison","rarity":1,"cost":1500,"effects":[{"tag":21,"a":25,"b":100,"c":0}],"ramp":null},
    {"name":"Potent Poison","rarity":0,"cost":500,"effects":[{"tag":22,"a":10,"b":100,"c":0}],"ramp":null},
    {"name":"Dazing Stuns","rarity":1,"cost":1500,"effects":[{"tag":23,"a":50,"b":100,"c":0}],"ramp":null},
    {"name":"Dreadlord Fang","rarity":0,"cost":500,"effects":[{"tag":12,"a":80,"b":0,"c":0},{"tag":39,"a":8,"b":0,"c":0}],"ramp":null},
    {"name":"+300 Spikes Damage","rarity":1,"cost":1500,"effects":[{"tag":12,"a":300,"b":0,"c":0}],"ramp":null},
    {"name":"+50% Spikes Damage","rarity":1,"cost":1500,"effects":[{"tag":13,"a":50,"b":100,"c":0}],"ramp":null},
    {"name":"Growing Spikes","rarity":2,"cost":3000,"effects":[{"tag":12,"a":80,"b":0,"c":0}],"ramp":{"effect":{"tag":12,"a":10,"b":0,"c":0},"interval_ticks":900}},
    {"name":"Vulnerability Totem","rarity":2,"cost":3000,"effects":[{"tag":14,"a":5,"b":1200,"c":30}],"ramp":null},
    {"name":"Mask of Death","rarity":1,"cost":1500,"effects":[{"tag":6,"a":1000,"b":0,"c":0},{"tag":15,"a":15,"b":0,"c":0}],"ramp":null},
    {"name":"+60 Heal on Kill","rarity":2,"cost":3000,"effects":[{"tag":15,"a":60,"b":0,"c":0}],"ramp":null},
    {"name":"Reanimating Poison","rarity":2,"cost":3000,"effects":[{"tag":16,"a":5,"b":0,"c":0}],"ramp":null},
    {"name":"Multiplication Gems","rarity":1,"cost":1500,"effects":[{"tag":24,"a":0,"b":3,"c":0}],"ramp":null},
    {"name":"Duplicator","rarity":2,"cost":3000,"effects":[{"tag":24,"a":2,"b":1,"c":0}],"ramp":null},
    {"name":"Black Market","rarity":1,"cost":1500,"effects":[{"tag":25,"a":1,"b":0,"c":0}],"ramp":null},
    {"name":"Magic Treasure","rarity":1,"cost":1000,"effects":[{"tag":26,"a":250,"b":0,"c":0}],"ramp":{"effect":{"tag":5,"a":5,"b":0,"c":0},"interval_ticks":900}},
    {"name":"Ankh of Reconstruction","rarity":3,"cost":5000,"effects":[{"tag":29,"a":2000,"b":0,"c":0}],"ramp":null},
    {"name":"Healing Hand","rarity":1,"cost":1500,"effects":[{"tag":27,"a":25,"b":100,"c":0}],"ramp":null},
    {"name":"Living Wood","rarity":2,"cost":3000,"effects":[{"tag":6,"a":2000,"b":0,"c":0},{"tag":28,"a":15,"b":1000,"c":0}],"ramp":null},
    {"name":"Enchanted Moon Arrow","rarity":2,"cost":3000,"effects":[{"tag":1,"a":1,"b":100,"c":100},{"tag":30,"a":0,"b":1,"c":1}],"ramp":null},
    {"name":"Refined Explosives","rarity":2,"cost":3000,"effects":[{"tag":1,"a":3,"b":100,"c":100},{"tag":30,"a":1,"b":3,"c":1}],"ramp":null},
    {"name":"Overclocked Death Engine (+10% Chaos per Death Engine)","rarity":2,"cost":3000,"effects":[{"tag":30,"a":10,"b":4,"c":10}],"ramp":null},
    {"name":"Philosopher's Stone","rarity":1,"cost":0,"effects":[{"tag":31,"a":1000,"b":2000,"c":0}],"ramp":null},
    {"name":"Cursed Treasure","rarity":2,"cost":0,"effects":[{"tag":32,"a":100,"b":5000,"c":0}],"ramp":null},
    {"name":"Bloodmoney (+1 Gold per 100 Damage)","rarity":1,"cost":1500,"effects":[{"tag":33,"a":1,"b":60,"c":0}],"ramp":null},
    {"name":"Bloodmoney II (+1 Gold per 20 Damage)","rarity":2,"cost":3000,"effects":[{"tag":33,"a":1,"b":12,"c":0}],"ramp":null},
    {"name":"Wartithe (25% of Income as Mana Shield)","rarity":2,"cost":3000,"effects":[{"tag":34,"a":25,"b":100,"c":0}],"ramp":null},
    {"name":"Aegis Protocol (+2500 Shield, +400/round)","rarity":3,"cost":5000,"effects":[{"tag":8,"a":2500,"b":20,"c":0}],"ramp":{"effect":{"tag":8,"a":400,"b":4,"c":0},"interval_ticks":900}},
    {"name":"Improved Masonry","rarity":0,"cost":500,"effects":[{"tag":6,"a":500,"b":0,"c":0}],"ramp":null},
    {"name":"Greater Piercing Attacks","rarity":0,"cost":500,"effects":[{"tag":1,"a":1,"b":10,"c":100}],"ramp":null},
    {"name":"Improved Normal Attacks","rarity":0,"cost":500,"effects":[{"tag":1,"a":0,"b":10,"c":100}],"ramp":null},
    {"name":"Improved Siege Attacks","rarity":0,"cost":500,"effects":[{"tag":1,"a":3,"b":10,"c":100}],"ramp":null},
    {"name":"Improved Chaos Attacks","rarity":0,"cost":500,"effects":[{"tag":1,"a":4,"b":10,"c":100}],"ramp":null},
    {"name":"+1000 Max HP","rarity":1,"cost":1500,"effects":[{"tag":6,"a":1000,"b":0,"c":0}],"ramp":null},
    {"name":"+10 Armor","rarity":0,"cost":500,"effects":[{"tag":7,"a":10,"b":0,"c":0}],"ramp":null},
    {"name":"+2000 Max HP","rarity":2,"cost":3000,"effects":[{"tag":6,"a":2000,"b":0,"c":0}],"ramp":null},
    {"name":"+50% Kill Bounty","rarity":2,"cost":3000,"effects":[{"tag":4,"a":50,"b":100,"c":0}],"ramp":null},
    {"name":"Recharge","rarity":2,"cost":3000,"effects":[{"tag":8,"a":2000,"b":10,"c":0},{"tag":37,"a":25,"b":100,"c":0}],"ramp":null},
    {"name":"+20 Gold Income","rarity":0,"cost":500,"effects":[{"tag":5,"a":20,"b":0,"c":0}],"ramp":null},
    {"name":"Tower Armor","rarity":0,"cost":500,"effects":[{"tag":7,"a":5,"b":0,"c":0}],"ramp":null},
    {"name":"+10% Attack Speed","rarity":0,"cost":500,"effects":[{"tag":3,"a":10,"b":100,"c":0}],"ramp":null},
    {"name":"Renew","rarity":1,"cost":1500,"effects":[{"tag":9,"a":80,"b":0,"c":0},{"tag":36,"a":25,"b":100,"c":0}],"ramp":null},
    {"name":"Repair Crew","rarity":0,"cost":500,"effects":[{"tag":9,"a":20,"b":0,"c":0}],"ramp":null},
    {"name":"Magic Coin","rarity":0,"cost":500,"effects":[{"tag":5,"a":5,"b":0,"c":0}],"ramp":null},
    {"name":"Gold Mine","rarity":0,"cost":500,"effects":[{"tag":5,"a":10,"b":0,"c":0},{"tag":17,"a":10,"b":100,"c":0}],"ramp":null},
    {"name":"+100% Kill Bounty","rarity":2,"cost":3000,"effects":[{"tag":4,"a":100,"b":100,"c":0}],"ramp":null},
    {"name":"Improved Magic Attacks","rarity":0,"cost":500,"effects":[{"tag":1,"a":2,"b":10,"c":100}],"ramp":null},
    {"name":"+40 HP Regen","rarity":0,"cost":500,"effects":[{"tag":9,"a":40,"b":0,"c":0}],"ramp":null},
    {"name":"Energy Shield","rarity":3,"cost":5000,"effects":[{"tag":8,"a":10000,"b":50,"c":0},{"tag":38,"a":30,"b":100,"c":0}],"ramp":null},
    {"name":"Evasion","rarity":0,"cost":500,"effects":[{"tag":10,"a":10,"b":0,"c":0}],"ramp":null},
    {"name":"Escalating Plunder (+100% Bounty, +15%/round)","rarity":2,"cost":3000,"effects":[{"tag":4,"a":100,"b":100,"c":0}],"ramp":{"effect":{"tag":4,"a":15,"b":100,"c":0},"interval_ticks":900}},
    {"name":"Living Fortress (+2500 Max HP, +500/round)","rarity":3,"cost":5000,"effects":[{"tag":6,"a":2500,"b":0,"c":0}],"ramp":{"effect":{"tag":6,"a":500,"b":0,"c":0},"interval_ticks":900}},
    {"name":"Mana Shield","rarity":1,"cost":1500,"effects":[{"tag":8,"a":1000,"b":5,"c":0}],"ramp":null},
    {"name":"Mending Engine (+120 Regen, +30/round)","rarity":1,"cost":1500,"effects":[{"tag":9,"a":120,"b":0,"c":0}],"ramp":{"effect":{"tag":9,"a":30,"b":0,"c":0},"interval_ticks":900}},
    {"name":"Mastercrafted Masonry","rarity":2,"cost":3000,"effects":[{"tag":6,"a":5000,"b":0,"c":0},{"tag":40,"a":1,"b":100,"c":0}],"ramp":null},
    {"name":"Golden Ring","rarity":3,"cost":5000,"effects":[{"tag":4,"a":200,"b":100,"c":0},{"tag":41,"a":1,"b":100,"c":0}],"ramp":null},
    {"name":"Arcane Mark","rarity":3,"cost":5000,"effects":[{"tag":8,"a":4000,"b":20,"c":0},{"tag":42,"a":20,"b":100,"c":0}],"ramp":null},
    {"name":"Maw of Death","rarity":2,"cost":3000,"effects":[{"tag":8,"a":2000,"b":10,"c":0},{"tag":43,"a":15,"b":0,"c":0}],"ramp":null},
    {"name":"Energy Pulse","rarity":2,"cost":3000,"effects":[{"tag":8,"a":2000,"b":10,"c":0},{"tag":44,"a":1200,"b":15,"c":0}],"ramp":null},
    {"name":"Poison Armor","rarity":1,"cost":1500,"effects":[{"tag":7,"a":10,"b":0,"c":0},{"tag":12,"a":40,"b":0,"c":0},{"tag":45,"a":2,"b":90,"c":0}],"ramp":null},
    {"name":"Bloody Spikes","rarity":2,"cost":3000,"effects":[{"tag":12,"a":80,"b":0,"c":0},{"tag":46,"a":20,"b":25,"c":0}],"ramp":null},
    {"name":"Blight Aura","rarity":3,"cost":5000,"effects":[{"tag":9,"a":200,"b":0,"c":0},{"tag":47,"a":600,"b":30,"c":200}],"ramp":null}
  ],
  "waves": [
    {"enemy":0,"cadence_ticks":18,"start_tick":0},
    {"enemy":1,"cadence_ticks":95,"start_tick":0},
    {"enemy":3,"cadence_ticks":45,"start_tick":360},
    {"enemy":11,"cadence_ticks":600,"start_tick":900},
    {"enemy":4,"cadence_ticks":95,"start_tick":750},
    {"enemy":3,"cadence_ticks":70,"start_tick":1800},
    {"enemy":5,"cadence_ticks":130,"start_tick":1350},
    {"enemy":8,"cadence_ticks":120,"start_tick":2700},
    {"enemy":7,"cadence_ticks":180,"start_tick":7200},
    {"enemy":9,"cadence_ticks":200,"start_tick":7200},
    {"enemy":6,"cadence_ticks":300,"start_tick":10800},
    {"enemy":10,"cadence_ticks":240,"start_tick":10800},
    {"enemy":6,"cadence_ticks":150,"start_tick":36000},
    {"enemy":5,"cadence_ticks":60,"start_tick":36000},
    {"enemy":9,"cadence_ticks":90,"start_tick":45000},
    {"enemy":10,"cadence_ticks":100,"start_tick":45000},
    {"enemy":0,"cadence_ticks":8,"start_tick":45000}
  ],
  "boss_escort": [
    {"enemy":0,"cadence_ticks":4,"start_tick":54000},
    {"enemy":4,"cadence_ticks":12,"start_tick":54000},
    {"enemy":5,"cadence_ticks":15,"start_tick":54000},
    {"enemy":1,"cadence_ticks":45,"start_tick":54000}
  ],
  "spawn_ring": [
    {"x":78643200,"y":0},
    {"x":83427328,"y":83427328},
    {"x":0,"y":98304000},
    {"x":-55640064,"y":55640064},
    {"x":117964800,"y":0},
    {"x":-69533696,"y":-69533696},
    {"x":0,"y":-78643200},
    {"x":83427328,"y":-83427328}
  ],
  "timeline": {
    "steam": {"tick_hz":30,"round_ticks":900,"boss_spawn_tick":54000,"ramp_interval":5400,"ramp_intervals_to_boss":10,"gentle_ticks":4500,"warn_ticks":900,"ramp_per_round":900,"scale_step_1_tick":18000,"scale_step_2_tick":27000,"cliff_20_tick":36000,"cliff_25_tick":45000},
    "roblox": {"tick_hz":30,"round_ticks":600,"boss_spawn_tick":9000,"ramp_interval":900,"ramp_intervals_to_boss":10,"gentle_ticks":750,"warn_ticks":150,"ramp_per_round":600,"scale_step_1_tick":3000,"scale_step_2_tick":4500,"cliff_20_tick":6000,"cliff_25_tick":7500}
  },
  "constants": {
    "tick_hz": 30,
    "frost_max_stacks": 25,
    "ramp_per_round": 900,
    "boss_contact_cadence": 16,
    "starting_weapon": 0,
    "death_engine": 10,
    "boss": 2,
    "num_weapons": 86,
    "num_enemies": 12,
    "num_modifiers": 91,
    "num_damage_types": 5,
    "num_armor_classes": 3,
    "num_scopes": 12,
    "dmg_normal": 0,
    "dmg_piercing": 1,
    "dmg_magic": 2,
    "dmg_siege": 3,
    "dmg_chaos": 4,
    "armor_light": 0,
    "armor_medium": 1,
    "armor_fortified": 2,
    "range_scope_short": 6,
    "range_scope_long": 7,
    "range_scope_short_max": 600,
    "rarity_scope_base": 8,
    "archetype_swarm": 0,
    "archetype_tank": 1,
    "archetype_fast": 2,
    "archetype_caster": 3,
    "archetype_ranged": 4,
    "archetype_boss": 5,
    "archetype_inert": 6,
    "enemy_ranged_dtype_shift": 32,
    "ramp_gentle_num": 41,
    "ramp_gentle_den": 40,
    "ramp_warn_num": 127,
    "ramp_warn_den": 125,
    "ramp_jump_num": 57,
    "ramp_jump_den": 50,
    "early_grunt_cadence": 18,
    "early_peon_cadence": 45,
    "early_raider_cadence": 95,
    "early_bandit_cadence": 130,
    "fixed_frac_bits": 16,
    "fixed_one": 65536
  }
}
]==]

local Content = {}

--- The `content.json` text this module was built from (see the header). Exposed
--- so a consumer — or `test/content_test.luau` — can prove it is byte-identical
--- to `src/shared/content.json`.
Content.RAW_JSON = RAW_JSON

-- ============================ payload enums ============================

-- `{ tag = { variantName, operandName... } }`, mirroring each `words()` in
-- `content.rs`. Operand names are listed in `a, b, c` order.

--- `content::Attack`. C1: the tag IS `attack_scope_id`, so a port carries one
--- mapping, not two.
local ATTACK = {
	[0] = { "SingleTarget" },
	[1] = { "Splash", "radius" },
	[2] = { "Barrage", "n" },
	[3] = { "Area", "radius" },
	[4] = { "Wave", "extra" },
	[5] = { "Bounce", "n" },
}

--- `content::WeaponAbility`.
local WEAPON_ABILITY = {
	[0] = { "None" },
	[1] = { "LifeDrain", "per_hit" },
	[2] = { "ManaDrain", "per_hit" },
	[3] = { "Knockback", "dist" },
	[4] = { "Root", "ticks" },
	[5] = { "VulnOnHit", "stacks" },
	[6] = { "Hazard", "dmg", "radius", "ticks" },
	[7] = { "Summon", "kind", "hp", "damage" },
}

--- `content::EnemyAbility`. `RangedAttack` carries FOUR fields in three operand
--- slots, so `b` is unpacked separately below (C1 addendum).
local ENEMY_ABILITY = {
	[0] = { "None" },
	[1] = { "RangedAttack", "range" },
}

--- `content::ModEffect`. Tags are the `ModEffect::words` discriminants.
local MOD_EFFECT = {
	[0] = { "DamageGlobalPct", "num", "den" },
	[1] = { "DamageTypePct", "damage_type", "num", "den" },
	[2] = { "DamageMulPct", "num", "den" },
	[3] = { "AttackSpeedPct", "num", "den" },
	[4] = { "BountyPct", "num", "den" },
	[5] = { "IncomeFlat", "flat" },
	[6] = { "MaxHp", "flat" },
	[7] = { "Armor", "flat" },
	[8] = { "ManaShield", "pool", "regen" },
	[9] = { "HpRegen", "flat" },
	[10] = { "Dodge", "num" },
	[11] = { "DamageScopePct", "scope", "num", "den" },
	[12] = { "SpikesFlat", "flat" },
	[13] = { "SpikesPct", "num", "den" },
	[14] = { "GrantVulnPulse", "magnitude", "range", "interval_ticks" },
	[15] = { "HealOnKill", "flat" },
	[16] = { "HealOnPoison", "flat" },
	[17] = { "IncomePct", "num", "den" },
	[18] = { "IncomeRegenPct", "num", "den" },
	[19] = { "BountyProc", "chance_pct", "bonus_pct" },
	[20] = { "DamageVsStunnedPct", "num", "den" },
	[21] = { "DamageVsPoisonedPct", "num", "den" },
	[22] = { "PoisonDamagePct", "num", "den" },
	[23] = { "StunDurationPct", "num", "den" },
	[24] = { "GrantDuplicator", "rarity", "copies" },
	[25] = { "GrantVoucher", "rarity" },
	[26] = { "GrantGold", "gold" },
	[27] = { "HealingPct", "num", "den" },
	[28] = { "MissingHpHealPct", "num", "den" },
	[29] = { "GrantRevive", "bonus" },
	[30] = { "DamagePerWeapon", "weapon_def", "damage_type", "num_pct" },
	[31] = { "TradeMaxHpForGold", "hp_cost", "gold_gain" },
	[32] = { "TradeRegenForGold", "regen_cost", "gold_gain" },
	[33] = { "GoldPerDamagePct", "num", "den" },
	[34] = { "IncomeShieldPct", "num", "den" },
	[35] = { "MaxHpPct", "num", "den" },
	[36] = { "HpRegenPct", "num", "den" },
	[37] = { "ManaRegenPct", "num", "den" },
	[38] = { "ShieldActiveDrPct", "num", "den" },
	[39] = { "HealOnDamaged", "flat" },
	[40] = { "DamagePerMaxHp", "num", "den" },
	[41] = { "DamagePerBountyPct", "num", "den" },
	[42] = { "ShieldActiveDamagePct", "num", "den" },
	[43] = { "ManaOnKill", "flat" },
	[44] = { "ShieldBreakStun", "range", "stun_ticks" },
	[45] = { "SpikesPoison", "dps", "ticks" },
	[46] = { "StackingSpikes", "per_stack", "max_stacks" },
	[47] = { "DamageAura", "range", "cadence_ticks", "damage" },
}

--- `ModEffect::is_meta` — META items modify the shop/purchase flow rather than
--- the tank's stats. Keyed by `words()` tag.
local META_TAGS = { [24] = true, [25] = true, [26] = true, [31] = true, [32] = true }
--- `ModEffect::is_economy` — income / gold generation.
local ECONOMY_TAGS = { [5] = true, [17] = true, [26] = true }

Content.ATTACK_VARIANTS = table.freeze(ATTACK)
Content.WEAPON_ABILITY_VARIANTS = table.freeze(WEAPON_ABILITY)
Content.ENEMY_ABILITY_VARIANTS = table.freeze(ENEMY_ABILITY)
Content.MOD_EFFECT_VARIANTS = table.freeze(MOD_EFFECT)

-- ============================ decoding ============================

local function num(v, what)
	if type(v) == "string" then
		-- `Json.decode` returns the literal TEXT for |v| >= 2^53. C1 promises the
		-- catalog never carries one, so reaching here means a corrupt artifact.
		error(what .. ": value " .. v .. " is outside the exact-integer range", 0)
	end
	if type(v) ~= "number" or v % 1 ~= 0 then
		error(what .. ": expected an integer, got " .. tostring(v), 0)
	end
	return v
end

--- Decode one `{tag, a, b, c}` WORDS payload. The result KEEPS `tag/a/b/c` —
--- `lib.rs::checksum` hashes `effect.words()` / `ability.words()` verbatim, so a
--- decoded value must be able to reproduce its own encoding — and adds
--- `variant` (the exact Rust variant name) plus the named operands.
local function decodeWords(variants, w, what)
	if type(w) ~= "table" then
		error(what .. ": expected a {tag,a,b,c} object", 0)
	end
	local tag = num(w.tag, what .. ".tag")
	local spec = variants[tag]
	if spec == nil then
		error(what .. ": unknown tag " .. tag, 0)
	end
	local a = num(w.a, what .. ".a")
	local b = num(w.b, what .. ".b")
	local c = num(w.c, what .. ".c")
	local d = { variant = spec[1], tag = tag, a = a, b = b, c = c }
	local ops = { a, b, c }
	for i = 2, #spec do
		d[spec[i]] = ops[i - 1]
	end
	return d
end

local B32 = 4294967296 -- 2^32 == 1 << ENEMY_RANGED_DTYPE_SHIFT

--- `EnemyAbility::from_words`. `RangedAttack` packs four fields into three
--- operands: `b = cooldown_ticks | (damage_type << 32)` (C1 addendum), unpacked
--- with `b // 2^32` and `b % 2^32` exactly as the contract prescribes.
local function decodeEnemyAbility(w, what)
	local d = decodeWords(ENEMY_ABILITY, w, what)
	if d.variant == "RangedAttack" then
		local b = d.b
		local cooldown = b % B32
		d.cooldown_ticks = cooldown
		d.damage_type = (b - cooldown) / B32
		d.damage = d.c
	end
	return table.freeze(d)
end

local function decodeModEffect(w, what)
	local d = decodeWords(MOD_EFFECT, w, what)
	d.is_meta = META_TAGS[d.tag] == true
	d.is_economy = ECONOMY_TAGS[d.tag] == true
	return table.freeze(d)
end

-- ============================ derived accessors ============================

--- `content::attack_scope_id` — the scope id of a weapon's attack class. C1
--- pins this to the `Attack` payload tag (test-asserted on the Rust side).
function Content.attackScopeId(attack)
	return attack.tag
end

--- `content::range_scope_id` — 6 = short (<= 600), 7 = long.
function Content.rangeScopeId(range)
	if range <= Content.RANGE_SCOPE_SHORT_MAX then
		return Content.RANGE_SCOPE_SHORT
	end
	return Content.RANGE_SCOPE_LONG
end

--- `content::rarity_scope_id` — 8 = Common .. 11 = Epic (`8 + rarity.min(3)`).
function Content.rarityScopeId(rarity)
	if rarity > 3 then
		rarity = 3
	end
	return Content.RARITY_SCOPE_BASE + rarity
end

--- `ModEffect::is_meta`.
function Content.modEffectIsMeta(effect)
	return effect.is_meta
end

--- `ModEffect::is_economy`.
function Content.modEffectIsEconomy(effect)
	return effect.is_economy
end

--- `content::damage_multiplier` — the armor matrix as a `Fixed` multiplier.
--- Mirrors the Rust's index normalisation exactly: `damage_type % 5` and
--- `min(armor_class, NUM_ARMOR_CLASSES - 1)`. Both arguments are 0-based ids.
function Content.damageMultiplier(damageType, armorClass)
	local dt = damageType % Content.NUM_DAMAGE_TYPES
	local ac = armorClass
	if ac > Content.NUM_ARMOR_CLASSES - 1 then
		ac = Content.NUM_ARMOR_CLASSES - 1
	end
	return Content.DAMAGE_MATRIX[dt + 1][ac + 1]
end

-- `enemy_hp_mult`'s three per-interval factors. Pure `(num, den)` ratios out of
-- `constants` and identical on BOTH timelines — only the tick gates rescale
-- (`docs/10` F1), which is what keeps the difficulty endpoint untouched.
local G, W, J
-- `base(k)`, the post-cliff value at the start of interval `k`, memoised. The
-- Rust recomputes it per call by compounding from `Fixed::ONE`; this is a cache
-- of a pure function, so every value is bit-identical to the Rust's.
local baseCache

local function baseAt(k)
	local have = 0
	while baseCache[have + 1] ~= nil do
		have += 1
	end
	for i = have + 1, k do
		baseCache[i] = Fixed.mul(Fixed.mul(Fixed.mul(baseCache[i - 1], G), W), J)
	end
	return baseCache[k]
end

--- `content::enemy_hp_mult` — enemy HP/damage scaling at `tick`, as a `Fixed`.
--- A stepped ramp: gentle climb -> steeper "warning" window -> an instantaneous
--- step AT each interval boundary; from the boss tick on it holds the endpoint
--- tier (`base(ramp_intervals_to_boss)`), which IS that last cliff's post value.
---
--- `timeline` is OPTIONAL and defaults to `Content.TIMELINE.steam` — the
--- constants the Rust itself compiles in, so the default IS the oracle. Pass
--- `Content.TIMELINE.roblox` for the F1 rescale. Both ship; neither is baked in.
function Content.enemyHpMult(tick, timeline)
	local tl = timeline or Content.TIMELINE.steam
	if tl.ramp_interval == nil then
		error("Content.enemyHpMult: not a timeline (want Content.TIMELINE.steam / .roblox)", 2)
	end
	if tick >= tl.boss_spawn_tick then
		return baseAt(tl.ramp_intervals_to_boss)
	end

	local interval = tl.ramp_interval
	local k = (tick - tick % interval) / interval
	local lo = k * interval
	local warnStart = lo + tl.gentle_ticks
	local hi = lo + interval

	local bk = baseAt(k)
	local gentleEnd = Fixed.mul(bk, G) -- value at the end of the gentle climb
	local warnEnd = Fixed.mul(gentleEnd, W) -- pre-cliff peak (just before the step)

	local a, b, from, to
	if tick < warnStart then
		-- Gentle climb: bk -> bk*G across the first `gentle_ticks`.
		a, b, from, to = lo, warnStart, bk, gentleEnd
	else
		-- Warning sub-ramp: bk*G -> bk*G*W across the final ticks (steeper). The
		-- dramatic step (warnEnd -> base(k+1)) lands AT `hi`.
		a, b, from, to = warnStart, hi, gentleEnd, warnEnd
	end
	-- `from + (to - from) * ((tick - a) / (b - a))`, all Fixed.
	return Fixed.add(from, Fixed.mul(Fixed.sub(to, from), Fixed.fromRatio(tick - a, b - a)))
end

-- ============================ load ============================

local function buildWeapon(id, w)
	local onHit = w.on_hit
	local what = "weapon[" .. id .. "]"
	return table.freeze({
		id = id,
		name = w.name,
		rarity = num(w.rarity, what .. ".rarity"),
		cost = num(w.cost, what .. ".cost"),
		damage = num(w.damage, what .. ".damage"),
		damage_type = num(w.damage_type, what .. ".damage_type"),
		attack = table.freeze(decodeWords(ATTACK, w.attack, what .. ".attack")),
		cooldown_ticks = num(w.cooldown_ticks, what .. ".cooldown_ticks"),
		range = num(w.range, what .. ".range"),
		proj_speed = num(w.proj_speed, what .. ".proj_speed"),
		on_hit = table.freeze({
			poison_dps = num(onHit.poison_dps, what .. ".on_hit.poison_dps"),
			poison_ticks = num(onHit.poison_ticks, what .. ".on_hit.poison_ticks"),
			frost_stacks = num(onHit.frost_stacks, what .. ".on_hit.frost_stacks"),
			fire_stacks = num(onHit.fire_stacks, what .. ".on_hit.fire_stacks"),
			stun_ticks = num(onHit.stun_ticks, what .. ".on_hit.stun_ticks"),
		}),
		ability = table.freeze(decodeWords(WEAPON_ABILITY, w.ability, what .. ".ability")),
	})
end

local function buildEnemy(id, e)
	local what = "enemy[" .. id .. "]"
	if type(e.boss) ~= "boolean" then
		error(what .. ".boss: expected a JSON boolean", 0)
	end
	return table.freeze({
		id = id,
		name = e.name,
		base_hp = num(e.base_hp, what .. ".base_hp"),
		move_speed = num(e.move_speed, what .. ".move_speed"),
		contact_damage = num(e.contact_damage, what .. ".contact_damage"),
		bounty = num(e.bounty, what .. ".bounty"),
		armor_class = num(e.armor_class, what .. ".armor_class"),
		archetype = num(e.archetype, what .. ".archetype"),
		ability = decodeEnemyAbility(e.ability, what .. ".ability"),
		boss = e.boss,
	})
end

local function buildModifier(id, m)
	local what = "modifier[" .. id .. "]"
	local effects = {}
	for i, w in ipairs(m.effects) do
		effects[i] = decodeModEffect(w, what .. ".effects[" .. i .. "]")
	end
	-- `ModifierDef::is_meta` / `is_economy`: true if ANY effect is.
	local isMeta, isEconomy = false, false
	for _, e in ipairs(effects) do
		isMeta = isMeta or e.is_meta
		isEconomy = isEconomy or e.is_economy
	end
	local ramp = nil
	if m.ramp ~= nil and m.ramp ~= Json.null then
		ramp = table.freeze({
			effect = decodeModEffect(m.ramp.effect, what .. ".ramp.effect"),
			interval_ticks = num(m.ramp.interval_ticks, what .. ".ramp.interval_ticks"),
		})
	end
	return table.freeze({
		id = id,
		name = m.name,
		rarity = num(m.rarity, what .. ".rarity"),
		cost = num(m.cost, what .. ".cost"),
		effects = table.freeze(effects),
		ramp = ramp,
		is_meta = isMeta,
		is_economy = isEconomy,
	})
end

local function buildWaveTable(list, what)
	local out = {}
	for i, w in ipairs(list) do
		out[i] = table.freeze({
			enemy = num(w.enemy, what .. ".enemy"),
			cadence_ticks = num(w.cadence_ticks, what .. ".cadence_ticks"),
			start_tick = num(w.start_tick, what .. ".start_tick"),
		})
	end
	return table.freeze(out)
end

--- Flatten a `content.json` scalar table onto `Content` in UPPER_SNAKE_CASE,
--- mirroring `content.rs`'s own identifiers. DERIVED, never hand-listed, so the
--- flat names and `Content.constants` cannot drift apart. A key present twice
--- (`tick_hz` and `ramp_per_round` appear in both `constants` and the Steam
--- timeline) must agree — a disagreement is a corrupt artifact, not a choice.
local function flatten(t, sourceName)
	for key, value in pairs(t) do
		if type(key) == "string" and type(value) ~= "table" then
			local name = string.upper(key)
			local prev = rawget(Content, name)
			if prev ~= nil and prev ~= value then
				error(
					"Content: " .. sourceName .. "." .. key .. " (" .. tostring(value) ..
						") disagrees with the already-flattened " .. name .. " (" .. tostring(prev) .. ")",
					0
				)
			end
			Content[name] = value
		end
	end
end

--- (Re)load the C1 catalog. `source` is either the `content.json` TEXT or an
--- already-decoded table (what Rojo's `.json` ModuleScript returns). Populates
--- this module in place and returns it.
---
--- The module self-loads from its embedded copy at require time, so callers do
--- NOT have to call this. It exists so a Roblox host can point the sim at the
--- Rojo-synced `content.json` instance, and so tests can load a variant.
function Content.load(source)
	local doc
	if type(source) == "string" then
		doc = Json.decode(source)
	elseif type(source) == "table" then
		doc = source
	else
		error("Content.load: expected the content.json text or a decoded table", 2)
	end

	if doc.schema_version ~= 1 then
		error("Content.load: unsupported schema_version " .. tostring(doc.schema_version), 2)
	end
	local k = doc.constants
	if type(k) ~= "table" then
		error("Content.load: missing `constants`", 2)
	end

	Content.SCHEMA_VERSION = doc.schema_version
	Content.CONTENT_HASH = doc.content_hash
	Content.constants = table.freeze(k)
	Content.CONSTANTS = Content.constants

	-- Timelines. BOTH ship; which one is active is the caller's decision.
	Content.TIMELINE = table.freeze({
		steam = table.freeze(doc.timeline.steam),
		roblox = table.freeze(doc.timeline.roblox),
	})
	Content.timeline = Content.TIMELINE

	-- Flat UPPER_SNAKE constants (see `flatten`). `constants` first, then the
	-- STEAM timeline: the Rust's own module constants (`BOSS_SPAWN_TICK`,
	-- `RAMP_INTERVAL`, `GENTLE_TICKS`, `ROUND_TICKS`, the cliff gates) ARE the
	-- Steam values, and the Steam timeline is what the oracle runs. The fork's
	-- rescaled values stay reachable as `Content.TIMELINE.roblox.*` only — they
	-- are deliberately NOT flattened, so nothing silently picks up the fork's
	-- numbers by reading a bare name.
	flatten(k, "constants")
	flatten(Content.TIMELINE.steam, "timeline.steam")

	if Content.ENEMY_RANGED_DTYPE_SHIFT ~= 32 then
		-- The unpack above is written against 2^32; C1 ships the constant
		-- precisely so a port can check rather than assume.
		error("Content.load: enemy_ranged_dtype_shift is not 32", 2)
	end
	if Content.FIXED_FRAC_BITS ~= Fixed.FRAC_BITS then
		error("Content.load: content.json fixed_frac_bits disagrees with Fixed.luau", 2)
	end

	-- `content::damage_multiplier`'s matrix, `[damage_type][armor_class]`, as
	-- `Fixed` values built from the exported raw i64s. Stored 1-INDEXED; go
	-- through `Content.damageMultiplier(dt, ac)` with 0-based ids.
	local rows = {}
	for i, row in ipairs(doc.damage_matrix) do
		local cols = {}
		for j, raw in ipairs(row) do
			cols[j] = Fixed.fromRaw(num(raw, "damage_matrix[" .. i .. "][" .. j .. "]"))
		end
		rows[i] = table.freeze(cols)
	end
	Content.DAMAGE_MATRIX = table.freeze(rows)
	Content.damageMatrix = Content.DAMAGE_MATRIX

	local weapons = {}
	for i, w in ipairs(doc.weapons) do
		weapons[i] = buildWeapon(i - 1, w)
	end
	Content.WEAPONS = table.freeze(weapons)

	local enemies = {}
	for i, e in ipairs(doc.enemies) do
		enemies[i] = buildEnemy(i - 1, e)
	end
	Content.ENEMIES = table.freeze(enemies)

	local modifiers = {}
	for i, m in ipairs(doc.modifiers) do
		modifiers[i] = buildModifier(i - 1, m)
	end
	Content.MODIFIERS = table.freeze(modifiers)

	Content.WAVES = buildWaveTable(doc.waves, "waves")
	Content.WAVE_M0 = Content.WAVES -- the Rust static's name
	Content.BOSS_ESCORT = buildWaveTable(doc.boss_escort, "boss_escort")

	local ring = {}
	for i, p in ipairs(doc.spawn_ring) do
		ring[i] = table.freeze({
			x = Fixed.fromRaw(num(p.x, "spawn_ring[" .. i .. "].x")),
			y = Fixed.fromRaw(num(p.y, "spawn_ring[" .. i .. "].y")),
		})
	end
	Content.SPAWN_RING = table.freeze(ring)

	-- lowercase aliases (same objects) — both spellings are in use across the
	-- concurrently-landed sim modules; neither group of callers is wrong.
	Content.weapons = Content.WEAPONS
	Content.enemies = Content.ENEMIES
	Content.modifiers = Content.MODIFIERS
	Content.waves = Content.WAVES
	Content.bossEscort = Content.BOSS_ESCORT
	Content.spawnRing = Content.SPAWN_RING

	-- C1 ships the table sizes precisely so a port can assert its own load:
	-- index-is-identity is worthless if a row went missing on the way in.
	local function checkCount(got, want, what)
		if got ~= want then
			error("Content.load: " .. what .. " count " .. got .. " ~= constants.num_" .. what .. " " .. want, 2)
		end
	end
	checkCount(#Content.WEAPONS, k.num_weapons, "weapons")
	checkCount(#Content.ENEMIES, k.num_enemies, "enemies")
	checkCount(#Content.MODIFIERS, k.num_modifiers, "modifiers")
	if #Content.DAMAGE_MATRIX ~= Content.NUM_DAMAGE_TYPES then
		error("Content.load: damage_matrix has " .. #Content.DAMAGE_MATRIX .. " rows", 2)
	end

	-- `enemy_hp_mult` curve parameters (identical on both timelines).
	G = Fixed.fromRatio(k.ramp_gentle_num, k.ramp_gentle_den)
	W = Fixed.fromRatio(k.ramp_warn_num, k.ramp_warn_den)
	J = Fixed.fromRatio(k.ramp_jump_num, k.ramp_jump_den)
	baseCache = { [0] = Fixed.ONE }

	Content.loaded = true
	return Content
end

Content.load(RAW_JSON)

return Content
