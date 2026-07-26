--[[
	Bot.luau — a transcription of `sim-core/crates/sim/src/bot.rs`.

	A deterministic auto-player so previews, the trace corpus and (later) lobby
	backfill can play themselves. It reads only the PUBLIC arena state (the same
	surface a real client/UI sees) and returns one `Input` per tick — proving the
	sim is drivable through its render-facing API. No RNG stream, no wall-clock:
	purely a function of observed state, so it never threatens determinism.

	The bot draws NOTHING from the sim's RNG streams. `Archetype.forSeed` hashes
	the public `master_seed` locally with a self-contained SplitMix64 finalizer —
	which is bit-for-bit the same mixing `Rng.nextU64` performs on a fresh stream
	seeded with that value, so it is implemented by that reuse rather than by a
	second copy of the constants.

	WEAPON SELECTION is a VALUE heuristic, not a price rule (see the "weapon
	valuation" section below). `bot.rs` replaced `cheapest_where(kind == Weapon)`
	with `Bot::best_weapon`: weapon-power estimation (damage-per-second × estimated
	simultaneous targets, adjusted for utility and reach), marginal value including
	the breadth SYNERGY a purchase grants the arsenal already owned, per-copy
	target SATURATION, a gold-dependent PRICE sensitivity, and a PATIENCE rule that
	declines a weak buy when something far better is on the board — every knob
	differing per archetype so the four build identities stay apart. All integer,
	no RNG, no wall-clock, stable slot iteration order.

	`Bot` is NOT part of `Arena.step`, and the R3 trace corpus carries a recorded
	input log rather than re-deriving decisions, so nothing here can move a
	checksum. It is graded by matching `bot.rs` decision-for-decision (which it
	does: verified against a Rust decision dump over 12 seeds × 9000 ticks and all
	nine challenge codes) and by the quality of lobby backfill in `Director`.

	No Roblox global is referenced; runs unchanged under a bare `luau`.
]]

local Rng = require("../Rng")
local Content = require("../Content")
local Input = require("./Input")

local Bot = {}

-- ============================ constants (bot.rs tail) ============================

--- Mask of all six attack-classes (JackOfAll completion target).
local ALL_CLASSES = 0b111111
--- The archetype weapon floor grows with the round but never exceeds this.
local WEAPON_FLOOR_CAP = 10
--- Hard cap on MODIFIER purchases per match for the default bot. A real build is
--- finite; without a cap the bot would buy a modifier every few ticks for the
--- whole match, compounding multiplicative damage / economy into a fixed-point
--- overflow. Challenge runs are exempt (they self-limit by playstyle).
local MODIFIER_BUY_CAP = 250
--- Hard cap on WEAPON instances for the default bot — a realistic arsenal.
local WEAPON_BUY_CAP = 40

-- ── weapon-valuation constants (`bot.rs` tail) ──────────────────────────────
--- Ceiling on the estimated simultaneous-target count (in tenths) any attack
--- class may claim. A `Barrage(12)` needs twelve enemies in range to be worth
--- twelve targets; crediting it in full would let the catalog's widest weapon
--- outrank everything on a board that rarely holds that many.
local MAX_TARGETS10 = 50
--- Mirror of `Modifiers::arsenal_synergy_add`: +10% per damage type beyond the
--- first, +8% per attack class beyond the first, +5% per distinct weapon def
--- beyond the first, each axis capped at +40% and the total at +120%. Copies are
--- neither rewarded nor taxed. Kept here as constants (rather than read from
--- `Modifiers.luau`) exactly as `bot.rs` keeps them out of `modifiers.rs`, so
--- the bot's MODEL of the incentive is explicit and reviewable; they must be
--- updated in lockstep if the shipped magnitudes move.
local SYN_PER_DAMAGE_TYPE_PCT = 10
local SYN_PER_ATTACK_CLASS_PCT = 8
local SYN_PER_DISTINCT_DEF_PCT = 5
local SYN_AXIS_CAP_PCT = 40
local SYN_TOTAL_CAP_PCT = 120
--- Upper bound on the gold-on-hand reference in the value/price trade-off. Four
--- times the priciest weapon (5000g), so once the bot holds ~80k gold price has
--- effectively stopped mattering — which is the intended behavior — while the
--- comparison stays in a small integer range as gold snowballs into the billions.
local PRICE_REF_CAP = 20000

local U32_MAX = 4294967295

-- ============================ integer arithmetic ============================

--- Rust's `/` on integers — truncates TOWARD ZERO, **not** toward negative
--- infinity (`SIM-SPEC.md` §S4 requires matching the Rust at every site, and
--- `range_pct` below divides a NEGATIVE numerator, where `math.floor` and Rust
--- disagree). The float quotient is corrected by an exact integer multiply, so a
--- 1-ulp division error on a large numerator cannot survive either.
local function idiv(a: number, b: number): number
	local neg = false
	if a < 0 then
		a, neg = -a, true
	end
	if b < 0 then
		b, neg = -b, not neg
	end
	local q = math.floor(a / b)
	if q * b > a then
		q -= 1
	elseif (q + 1) * b <= a then
		q += 1
	end
	return if neg then -q else q
end

--- `u32::count_ones` over the small masks the arsenal builds (bits 0..7).
local function countOnes(mask: number): number
	local n = 0
	while mask ~= 0 do
		mask = bit32.band(mask, mask - 1)
		n += 1
	end
	return n
end

-- ============================ catalog access ============================

-- Content ids are 0-based (§S3); Luau array storage is 1-indexed.
local function weaponDef(def: number)
	return Content.WEAPONS[def + 1]
end

local function modifierDef(def: number)
	return Content.MODIFIERS[def + 1]
end

--- The `(tag, a)` head of a payload's `words()` encoding (`CONTRACTS.md` §C1) —
--- the only operands any of the bot's category tests reads.
local function words(w)
	if w == nil then
		return 0, 0
	end
	local tag = w.tag
	if tag == nil then
		tag = w[1]
	end
	local a = w.a
	if a == nil then
		a = w[2]
	end
	return tag or 0, a or 0
end

--- `ModEffect` tags (`content.rs::ModEffect::words`). Grouped exactly as
--- `bot.rs` matches on the variants — by effect CATEGORY, never by item name, so
--- a future item of the same shape sorts correctly without touching this file.
local TAG_TRADE_MAXHP_FOR_GOLD = 31
local TAG_TRADE_REGEN_FOR_GOLD = 32

--- STATIC offense the bot can stack safely. The DYNAMIC stat-scaling damage
--- scalers (`DamagePerMaxHp` 40 / `DamagePerBountyPct` 41 /
--- `ShieldActiveDamagePct` 42 / `DamagePerWeapon` 30) are deliberately NOT here:
--- they multiply live tank/economy stats, so stacking them on a snowballing
--- build inflates damage past the Fixed range. Leaving them "Other" keeps the
--- bot from compounding them into an overflow.
local OFFENSE_TAGS = {
	[0] = true, -- DamageGlobalPct
	[1] = true, -- DamageTypePct
	[2] = true, -- DamageMulPct
	[3] = true, -- AttackSpeedPct
	[11] = true, -- DamageScopePct
	[20] = true, -- DamageVsStunnedPct
	[21] = true, -- DamageVsPoisonedPct
	[22] = true, -- PoisonDamagePct
	[23] = true, -- StunDurationPct
	[12] = true, -- SpikesFlat
	[13] = true, -- SpikesPct
	[14] = true, -- GrantVulnPulse
}

local DEFENSE_TAGS = {
	[6] = true, -- MaxHp
	[35] = true, -- MaxHpPct
	[7] = true, -- Armor
	[8] = true, -- ManaShield
	[9] = true, -- HpRegen
	[36] = true, -- HpRegenPct
	[37] = true, -- ManaRegenPct
	[10] = true, -- Dodge
	[15] = true, -- HealOnKill
	[16] = true, -- HealOnPoison
	[39] = true, -- HealOnDamaged
	[27] = true, -- HealingPct
	[28] = true, -- MissingHpHealPct
	[29] = true, -- GrantRevive
	[38] = true, -- ShieldActiveDrPct
	[34] = true, -- IncomeShieldPct
	[43] = true, -- ManaOnKill
}

--- `MaxHp` / `MaxHpPct` — the only defensive stat that scales the HP pool to
--- absorb the scaling contact damage.
local MAX_HP_TAGS = { [6] = true, [35] = true }

--- `ModifierDef::is_economy` — precomputed by `Content.luau` (true if ANY of the
--- def's effects is an economy effect).
local function modIsEconomy(def)
	return modifierDef(def).is_economy
end

-- ============================ offer classification ============================

--- `OfferKind::Weapon` test, tolerant of the enum's Luau spelling.
local function isWeaponOffer(off)
	local k = off.kind
	return k == 0 or k == "Weapon" or (type(k) == "table" and k.code == 0)
end

--- Whether buying `off` would strictly hurt *this* tank with no upside the bot's
--- policy can spend — i.e. a modifier that debuffs the owner's own survivability
--- (cuts Max HP, or pushes HP regen toward a per-tick drain) in exchange for a
--- gold windfall the greedy bot never converts back into survival. Detected from
--- the effect CATEGORY, not item names, so a future trap of the same shape is
--- declined too. Humans still see and may pick these as high-risk gambles — only
--- the BOT auto-declines them, since blindly buying them is the self-kill that
--- wrecked the survival-curve proxy.
local function isSelfHarmTrade(off)
	if isWeaponOffer(off) then
		return false
	end
	local fx = modifierDef(off.def).effects
	for i = 1, #fx do
		local tag, a = words(fx[i])
		-- Reduces Max HP for gold — a smaller HP pool the bot never offsets.
		-- Reduces HP regen for gold (may go negative → a drain) — self-damage.
		if (tag == TAG_TRADE_MAXHP_FOR_GOLD or tag == TAG_TRADE_REGEN_FOR_GOLD) and a > 0 then
			return true
		end
	end
	return false
end

--- Classify a modifier offer into the axis an archetype cares about. A modifier
--- counts as defensive if ANY effect raises survivability, offensive if ANY
--- raises damage, economic via `is_economy`.
local AXIS_OFFENSE = "Offense"
local AXIS_DEFENSE = "Defense"
local AXIS_ECONOMY = "Economy"
local AXIS_OTHER = "Other"

local function modAxis(def)
	if modIsEconomy(def) then
		return AXIS_ECONOMY
	end
	local fx = modifierDef(def).effects
	local offense, defense = false, false
	for i = 1, #fx do
		local tag = words(fx[i])
		if OFFENSE_TAGS[tag] then
			offense = true
		elseif DEFENSE_TAGS[tag] then
			defense = true
		end
	end
	if offense then
		return AXIS_OFFENSE
	elseif defense then
		return AXIS_DEFENSE
	end
	return AXIS_OTHER
end

--- Whether a modifier raises the tank's MAX HP (flat or %).
local function raisesMaxHp(def)
	local fx = modifierDef(def).effects
	for i = 1, #fx do
		local tag = words(fx[i])
		if MAX_HP_TAGS[tag] then
			return true
		end
	end
	return false
end

--- The opposite combat axis (offense ↔ defense); identity for non-combat axes.
local function otherAxis(a)
	if a == AXIS_OFFENSE then
		return AXIS_DEFENSE
	elseif a == AXIS_DEFENSE then
		return AXIS_OFFENSE
	end
	return a
end

--- `Bot::weapon_class` — a weapon's attack-class scope id.
local function weaponClass(def)
	return Content.attackScopeId(weaponDef(def).attack)
end

-- ============================ weapon valuation ============================
--
-- The bot used to choose weapons with `cheapest_where(kind == Weapon)`. Because
-- a weapon's `cost` is a pure function of its RARITY (500 / 1500 / 3000 / 5000),
-- "cheapest" meant "lowest-rarity offer, ties to the lowest slot index" — the
-- bot never looked at damage, cooldown, attack class or what it already owned.
-- That made it blind on the single most important decision in the game, so every
-- build-shape number measured through it described the PRICE TABLE rather than
-- the game. Everything below replaces that with a value estimate.
--
-- The model is deliberately a competent player's heuristic, not an optimizer:
--   value(offer) = own marginal power (with the synergy it would then enjoy)
--                + the synergy uplift it grants the arsenal ALREADY owned
-- ranked against price with a gold-dependent weight. All integer, no RNG, no
-- wall-clock, stable slot iteration order — it cannot perturb determinism.

--- `Attack` payload tags (C1: the tag IS `attack_scope_id`).
local ATTACK_SINGLE_TARGET = 0
local ATTACK_SPLASH = 1
local ATTACK_BARRAGE = 2
local ATTACK_AREA = 3
local ATTACK_WAVE = 4
local ATTACK_BOUNCE = 5

--- Estimated number of enemies one volley connects with, in TENTHS (10 == one
--- enemy). This is the single biggest thing price hides: a `Barrage(8)` and a
--- `SingleTarget` of equal damage-per-second are not equally good, and an
--- `Area`/`Wave` weapon that sweeps the pack camped on the tank is worth several
--- single-target weapons during the swarm phases the run actually dies to.
---
--- The numbers are a player's rule of thumb ("this one hits the whole pack"),
--- not a simulation: multi-target classes are credited generously but capped, so
--- a 12-projectile barrage cannot claim twelve simultaneous targets that rarely
--- all exist.
local function estTargets10(w): number
	local atk = w.attack
	local tag, a = atk.tag, atk.a
	if tag == ATTACK_SINGLE_TARGET then
		return 10
	elseif tag == ATTACK_SPLASH then
		-- Splash: the primary target plus whatever the radius catches.
		return 10 + math.clamp(idiv(a, 50), 0, 20)
	elseif tag == ATTACK_BARRAGE then
		-- Barrage/Bounce hit N DISTINCT enemies — the strongest honest multiplier
		-- in the catalog, capped because N enemies must actually be in range.
		return math.min(10 * a, MAX_TARGETS10)
	elseif tag == ATTACK_BOUNCE then
		return math.min(9 * a, MAX_TARGETS10)
	elseif tag == ATTACK_AREA then
		-- Instant pulses centred on the tank: they hit exactly the enemies that
		-- are about to deal contact damage, which is what kills the tank.
		return math.min(15 + idiv(a, 20), MAX_TARGETS10)
	end
	-- ATTACK_WAVE, `a` = extra reach beyond the listed range.
	return math.min(15 + idiv(w.range + a, 25), MAX_TARGETS10)
end

--- `WeaponAbility` payload tags.
local ABILITY_UTILITY_PCT = {
	[0] = 0, -- None
	-- Sustain: healing per enemy hit is survival, which is what the run is
	-- graded on.
	[1] = 25, -- LifeDrain
	[2] = 10, -- ManaDrain
	[3] = 12, -- Knockback
	[4] = 25, -- Root
	-- 5 VulnOnHit is `min(stacks * 2, 20)`, handled inline below: Vulnerability
	-- amplifies the WHOLE arsenal, not just this weapon.
	[6] = 20, -- Hazard
	[7] = 20, -- Summon
}
local ABILITY_VULN_ON_HIT = 5

--- Percentage uplift for everything a weapon does BESIDES its listed damage:
--- damage-over-time, crowd control, and its signature ability. Expressed as a
--- percent of the weapon's base power so it scales with the weapon rather than
--- swamping it. Bounded per-source so no single flag can dominate the ranking.
local function utilityPct(w): number
	local pct = 0
	local oh = w.on_hit
	-- Poison / fire keep dealing damage between shots.
	if oh.poison_dps > 0 then
		pct += math.min(oh.poison_dps * 2, 40)
	end
	if oh.fire_stacks > 0 then
		pct += math.min(idiv(oh.fire_stacks, 4), 30)
	end
	-- Frost slows the pack; stun removes attackers outright. Both convert into
	-- contact damage the tank never takes.
	if oh.frost_stacks > 0 then
		pct += math.min(oh.frost_stacks * 2, 15)
	end
	if oh.stun_ticks > 0 then
		pct += math.min(idiv(oh.stun_ticks, 3), 30)
	end
	local ab = w.ability
	if ab.tag == ABILITY_VULN_ON_HIT then
		pct += math.min(ab.a * 2, 20)
	else
		pct += ABILITY_UTILITY_PCT[ab.tag] or 0
	end
	return pct
end

--- Percentage adjustment for reach. A long-range projectile weapon starts hurting
--- the wave earlier and keeps firing while the tank is not yet swarmed; a
--- 300-range single-target weapon only fires once things are already on top of
--- you. Does not apply to `Area`/`Wave`, whose short listed range is inherent to
--- the class and already priced into `estTargets10`.
local function rangePct(w): number
	local tag = w.attack.tag
	if tag == ATTACK_AREA or tag == ATTACK_WAVE then
		return 0
	end
	return math.clamp(idiv(w.range - 600, 30), -15, 20)
end

--- Standalone power estimate for one instance of `def`, ignoring synergy — the
--- weapon's damage-per-second times how many enemies it lands on, adjusted for
--- utility and reach. `utilityWeightPct` lets an archetype care more or less
--- about the non-damage half (see `WeaponBias`).
local function weaponPower(def: number, utilityWeightPct: number): number
	local w = weaponDef(def)
	-- Tenths of damage-per-second: damage * TICK_HZ / cooldown, ×10.
	local dps10 = idiv(w.damage * 300, math.max(w.cooldown_ticks, 1))
	local base = idiv(dps10 * estTargets10(w), 10)
	local pct = math.max(100 + idiv(utilityPct(w) * utilityWeightPct, 100) + rangePct(w), 20)
	return math.max(idiv(base * pct, 100), 1)
end

--- The breadth incentive, mirrored from `Modifiers::arsenal_synergy_add`: +10%
--- per damage type beyond the first (cap +40%), +8% per attack class beyond the
--- first (cap +40%), +5% per distinct weapon def beyond the first (cap +40%),
--- total capped at +120%. Copies are neither rewarded nor taxed.
---
--- This is a MODEL of a rule the sim owns, kept here as constants so the bot
--- survives any revision of `Modifiers.luau` (which this module does not own).
--- If the shipped magnitudes change, the four constants must follow — the bot
--- would otherwise be optimizing a game it is no longer playing.
local function arsenalSynergyPct(damageTypes: number, attackClasses: number, distinctDefs: number): number
	local function axis(n: number, per: number): number
		-- `u32::saturating_sub(1)`.
		return math.min(math.max(n - 1, 0) * per, SYN_AXIS_CAP_PCT)
	end
	return math.min(
		axis(damageTypes, SYN_PER_DAMAGE_TYPE_PCT)
			+ axis(attackClasses, SYN_PER_ATTACK_CLASS_PCT)
			+ axis(distinctDefs, SYN_PER_DISTINCT_DEF_PCT),
		SYN_TOTAL_CAP_PCT
	)
end

--- What the `n`-th EXTRA copy of a weapon is actually worth, given `unit` is what
--- the first one is worth: `unit × SAT / (SAT + n)`.
---
--- This is target saturation, and it is the difference between a bot that can
--- weigh "another copy" against "something new" and one that cannot. Identical
--- weapons share a damage type, a range, a cooldown and a target pool, so the
--- tenth copy spends much of its output overkilling what the first nine already
--- killed, while a different weapon reaches targets, ranges and armor classes the
--- stack does not. A player estimates this instinctively ("I have enough of
--- those"); the bot needs it stated.
---
--- It is deliberately an ESTIMATE and not a claim about the rules: the sim does
--- not tax copies, and neither does the breadth synergy. Without it the
--- stack/spread choice is a cliff — copies score identically forever — and
--- whichever side of the cliff the bot lands on is an artifact of the shop it
--- happened to be rich in rather than of what the weapons are worth.
local function saturatedCopyPower(unit: number, alreadyOwned: number, saturation: number): number
	local sat = math.max(saturation, 1)
	return math.max(idiv(unit * sat, sat + math.max(alreadyOwned, 0)), 1)
end

-- ============================ Input construction ============================
-- `Input` is a packed integer code (`Input.luau`): 0 Noop, 2 Reroll, 3 Clear,
-- `1 + slot*256` BuyOffer. Nothing here builds one by hand.

local INPUT_NOOP = Input.NOOP
local INPUT_REROLL = Input.REROLL
local INPUT_CLEAR = Input.CLEAR
local buyOffer = Input.buyOffer

-- ============================ Challenge ============================

--- A self-imposed playstyle constraint the bot will honor while buying, so the
--- preview can earn the cosmetic challenge achievements on demand. Purely a
--- purchase filter over the public shop — it never bends the sim.
export type Challenge = { kind: string, class: number? }

Bot.Challenge = {
	None = { kind = "None" } :: Challenge,
	NoEconomy = { kind = "NoEconomy" } :: Challenge,
	JackOfAll = { kind = "JackOfAll" } :: Challenge,
	--- MEASUREMENT ONLY (not a shipped achievement): a PURE-ECONOMY player.
	EcoOnly = { kind = "EcoOnly" } :: Challenge,
}

--- Buy only weapons of this attack-class (scope id 0..6).
function Bot.purist(class): Challenge
	return { kind = "Purist", class = class }
end

--- Map the integer code to a challenge. 1..=6 → Purist of attack-class 0..5,
--- 7 → NoEconomy, 8 → JackOfAll, 9 → EcoOnly (measurement), else → None.
function Bot.challengeFromCode(code): Challenge
	if code >= 1 and code <= 6 then
		return Bot.purist(code - 1)
	elseif code == 7 then
		return Bot.Challenge.NoEconomy
	elseif code == 8 then
		return Bot.Challenge.JackOfAll
	elseif code == 9 then
		return Bot.Challenge.EcoOnly
	end
	return Bot.Challenge.None
end

--- Whether this challenge permits buying `off`.
function Bot.challengePermits(ch, off): boolean
	local weapon = isWeaponOffer(off)
	if ch.kind == "Purist" and weapon then
		return weaponClass(off.def) == ch.class
	elseif ch.kind == "NoEconomy" and not weapon then
		return not modIsEconomy(off.def)
	elseif ch.kind == "EcoOnly" then
		-- Pure-economy: forbid ALL weapons and every non-economy modifier; the
		-- only permitted purchase is an economy/gold modifier.
		if weapon then
			return false
		end
		return modIsEconomy(off.def)
	end
	return true
end

--- Authoritative apply-time guard: drop a `BuyOffer` that this challenge forbids
--- (resolved against the shop the buy actually lands on), so a slot-based
--- purchase can never violate the rule no matter how the shop shifted between
--- the bot's decision and the lead-delayed apply tick. A pure function of
--- `(input, state, challenge)` — applied identically on the director and the
--- client, it keeps their shadows in lockstep.
function Bot.challengeFilter(ch, inp, s)
	if Input.kind(inp) == Input.Kind.BuyOffer then
		-- `Input.slot` is 0-BASED; Luau storage is 1-indexed.
		local off = s.shop.offers[Input.slot(inp) + 1]
		if off ~= nil and not Bot.challengePermits(ch, off) then
			return INPUT_NOOP
		end
	end
	return inp
end

-- ============================ Archetype ============================

--- One of four build IDENTITIES the default bot adopts per match, chosen
--- DETERMINISTICALLY from the match seed. It is a PURCHASE BIAS only: it never
--- bends the sim, reads no wall-clock, and consumes no sim RNG stream.
local ARCH_GLASS_CANNON = "GlassCannon"
local ARCH_TANKY = "Tanky"
local ARCH_ECO_PIVOT = "EcoPivot"
local ARCH_BALANCED = "Balanced"

Bot.Archetype = {
	GlassCannon = ARCH_GLASS_CANNON,
	Tanky = ARCH_TANKY,
	EcoPivot = ARCH_ECO_PIVOT,
	Balanced = ARCH_BALANCED,
}

--- Deterministic per-match pick from the public match seed. The Rust applies a
--- SplitMix64 finalizer to `master_seed`; that is exactly `Rng::from_seed(seed)`
--- followed by one `next_u64()` (same GOLDEN addend and mix constants), so this
--- reuses the audited `Rng` rather than duplicating the constants. `z % 4`
--- depends only on the low two bits, i.e. on the low u32 half.
function Bot.archetypeForSeed(master_seed): string
	local r = Rng.fromSeed(master_seed)
	local _hi, lo = Rng.nextU64(r)
	local m = lo % 4
	if m == 0 then
		return ARCH_GLASS_CANNON
	elseif m == 1 then
		return ARCH_TANKY
	elseif m == 2 then
		return ARCH_ECO_PIVOT
	end
	return ARCH_BALANCED
end

--- The weapon-floor size this archetype holds before it diversifies into
--- modifiers — glass-cannon stacks many weapons; tanky keeps a lean floor.
local function weaponFloor(arch, round)
	local base
	if arch == ARCH_GLASS_CANNON then
		base = 6
	elseif arch == ARCH_TANKY then
		base = 2
	elseif arch == ARCH_ECO_PIVOT then
		base = 1
	else
		base = 3
	end
	return math.min(base + round, WEAPON_FLOOR_CAP)
end

--- Total weapons this archetype wants to own.
local function targetWeapons(arch)
	if arch == ARCH_GLASS_CANNON then
		return 20
	elseif arch == ARCH_TANKY then
		return 8
	elseif arch == ARCH_ECO_PIVOT then
		return 14
	end
	return 14
end

--- Last round in which this archetype still pours spare gold into ECONOMY
--- (income compounds only while rounds remain to pay it back).
local function econLastRound(arch)
	if arch == ARCH_GLASS_CANNON then
		return 2
	elseif arch == ARCH_TANKY then
		return 4
	elseif arch == ARCH_ECO_PIVOT then
		return 5
	end
	return 4
end

--- Per-axis buy WEIGHTS (offense, defense), out of their sum. Every archetype
--- keeps SOME defense (a tank with no HP/armor is one-shot by the scaling
--- contact damage), but the ratio is its identity.
local function axisWeights(arch)
	if arch == ARCH_GLASS_CANNON then
		return 7, 3
	elseif arch == ARCH_TANKY then
		return 3, 7
	end
	return 5, 5
end

--- How one archetype reads a weapon. These knobs are what keep the four build
--- identities apart now that all of them can see value: without them a
--- value-aware bot converges on one "best" line and the strategy-diversity
--- measurement dies.
---
---   `synergy_weight_pct`  How much of the breadth synergy this archetype
---     believes in. A glass-cannon discounts it (it wants the single biggest
---     hitter stacked); an eco-pivot, which buys late with a full wallet, leans
---     into it.
---   `utility_weight_pct`  How much the non-damage half (crowd control, sustain,
---     DoT) counts. Tanky builds pay up for stuns, slows and life-drain;
---     glass-cannons want numbers.
---   `price_divisor`  Price sensitivity: the gold-on-hand reference is
---     `gold / price_divisor`, so a LARGER divisor means a smaller reference,
---     which means price keeps mattering for longer (see `priceRef`).
---   `patience_pct`  Save-up discipline: decline to buy when the best AFFORDABLE
---     weapon is worth less than this percentage of the best weapon on the board
---     (see `bestWeapon`). Higher = more willing to sit on gold and wait.
---   `copy_saturation`  Copy tolerance: the saturation constant in
---     `saturatedCopyPower`. This is the axis that actually separates a STACKER
---     from a COVERAGE build, so it is per-archetype rather than global. Larger =
---     flatter = happier to buy the eleventh copy of its best weapon.
export type WeaponBias = {
	synergy_weight_pct: number,
	utility_weight_pct: number,
	price_divisor: number,
	patience_pct: number,
	copy_saturation: number,
}

-- Stack the biggest hitter; breadth is a distraction, utility is fluff, and it
-- will hold out (and overpay) for a top-end weapon.
local BIAS_GLASS_CANNON: WeaponBias = table.freeze({
	synergy_weight_pct = 60,
	utility_weight_pct = 50,
	price_divisor = 2,
	patience_pct = 55,
	copy_saturation = 30,
})
-- Buys crowd control and sustain: damage it never takes is damage it never has
-- to out-heal. Wants boards filled sooner, so less patient.
local BIAS_TANKY: WeaponBias = table.freeze({
	synergy_weight_pct = 100,
	utility_weight_pct = 175,
	price_divisor = 4,
	patience_pct = 30,
	copy_saturation = 5,
})
-- The value shopper: most price-sensitive, keenest on breadth (its whole plan is
-- to convert a big wallet into a broad board), and content to sit on gold while
-- the wallet does the work.
local BIAS_ECO_PIVOT: WeaponBias = table.freeze({
	synergy_weight_pct = 130,
	utility_weight_pct = 100,
	price_divisor = 8,
	patience_pct = 45,
	copy_saturation = 9,
})
local BIAS_BALANCED: WeaponBias = table.freeze({
	synergy_weight_pct = 100,
	utility_weight_pct = 100,
	price_divisor = 4,
	patience_pct = 40,
	copy_saturation = 8,
})

--- `Archetype::weapon_bias`.
local function archWeaponBias(arch): WeaponBias
	if arch == ARCH_GLASS_CANNON then
		return BIAS_GLASS_CANNON
	elseif arch == ARCH_TANKY then
		return BIAS_TANKY
	elseif arch == ARCH_ECO_PIVOT then
		return BIAS_ECO_PIVOT
	end
	return BIAS_BALANCED
end

--- `Archetype::weapon_bias` — exposed read-only (the Rust keeps it private) so
--- tests and tooling can inspect an archetype's identity without re-deriving the
--- table. The returned tables are frozen; nothing here reads back through it.
Bot.weaponBiasFor = archWeaponBias

-- ============================ Arsenal ============================

--- What the bot currently owns, summarized once per decision so each offer can be
--- priced against it in O(1). Built by scanning `s.weapons` in index order.
---
---   `type_mask`/`class_mask`  bit per damage type / attack class present.
---   `total_power`  sum of the SATURATED power of every owned instance — the base
---     that a synergy uplift would multiply.
---   `defs`  `{def, copies}` in first-seen order. Small (≤ the weapon cap), so a
---     linear scan beats any bitset over a catalog whose size is content-owned
---     and free to grow.
export type Arsenal = {
	type_mask: number,
	class_mask: number,
	damage_types: number,
	attack_classes: number,
	distinct_defs: number,
	total_power: number,
	defs: { { def: number, copies: number } },
}

--- `Arsenal::scan`.
local function scanArsenal(s, bias: WeaponBias): Arsenal
	local defs: { { def: number, copies: number } } = {}
	local typeMask, classMask = 0, 0
	local weapons = s.weapons
	for i = 1, #weapons do
		local w = weaponDef(weapons[i].def)
		typeMask = bit32.bor(typeMask, bit32.lshift(1, w.damage_type))
		classMask = bit32.bor(classMask, bit32.lshift(1, Content.attackScopeId(w.attack)))
		local found = false
		for k = 1, #defs do
			if defs[k].def == weapons[i].def then
				defs[k].copies += 1
				found = true
				break
			end
		end
		if not found then
			defs[#defs + 1] = { def = weapons[i].def, copies = 1 }
		end
	end
	local totalPower = 0
	for k = 1, #defs do
		local unit = weaponPower(defs[k].def, bias.utility_weight_pct)
		for i = 0, defs[k].copies - 1 do
			totalPower += saturatedCopyPower(unit, i, bias.copy_saturation)
		end
	end
	return {
		type_mask = typeMask,
		class_mask = classMask,
		damage_types = countOnes(typeMask),
		attack_classes = countOnes(classMask),
		distinct_defs = #defs,
		total_power = totalPower,
		defs = defs,
	}
end

--- `Arsenal::copies_of`.
local function copiesOf(ars: Arsenal, def: number): number
	for k = 1, #ars.defs do
		if ars.defs[k].def == def then
			return ars.defs[k].copies
		end
	end
	return 0
end

-- ============================ Bot ============================

--- Greedy survivor policy: clear when swarmed, otherwise keep buying.
export type BotState = {
	cooldown: number,
	target_weapons: number,
	challenge: Challenge,
	archetype: string?,
	mods_bought: number,
	offense_bought: number,
	defense_bought: number,
}

function Bot.new(): BotState
	return {
		--- Ticks to wait before acting again (keeps it from buying every frame).
		cooldown = 0,
		--- Stop *preferring* weapons over upgrades once we own this many.
		target_weapons = 12,
		challenge = Bot.Challenge.None,
		--- The build identity for this match, resolved on the first `decide`.
		archetype = nil,
		--- MODIFIER purchases this bot has issued (weapons excluded).
		mods_bought = 0,
		--- OFFENSE-axis modifier purchases (subset of `mods_bought`).
		offense_bought = 0,
		--- DEFENSE-axis modifier purchases (subset of `mods_bought`).
		defense_bought = 0,
	}
end

--- A bot that honors `c` while buying (otherwise identical to `new`).
function Bot.withChallenge(c): BotState
	local b = Bot.new()
	b.challenge = c
	return b
end

--- This match's archetype, resolved from the seed on first use and cached.
local function archetype(b, s)
	local a = b.archetype
	if a == nil then
		a = Bot.archetypeForSeed(s.master_seed)
		b.archetype = a
	end
	return a
end

--- Whether the bot may still buy MODIFIERS.
local function canBuyMod(b)
	return b.mods_bought < MODIFIER_BUY_CAP
end

--- Whether the bot may still buy WEAPONS.
local function canBuyWeapon(s)
	return #s.weapons < WEAPON_BUY_CAP
end

--- Commit a MODIFIER purchase at (1-based) `idx` of known `axis`: record it
--- against the cap and the per-axis counts, then set the post-buy think delay.
--- The single funnel for every modifier buy path.
local function buyMod(b, idx, axis)
	b.mods_bought += 1
	if axis == AXIS_OFFENSE then
		b.offense_bought += 1
	elseif axis == AXIS_DEFENSE then
		b.defense_bought += 1
	end
	b.cooldown = 6
	return buyOffer(idx - 1)
end

--- Which combat axis to buy next so the offense:defense purchase ratio tracks
--- the archetype's weights. Picks whichever axis is most "behind" its target
--- share (defense first on ties, so a fresh build always grabs survival early).
local function nextAxis(b, arch)
	local wo, wd = axisWeights(arch)
	-- Compare cross-multiplied shares: offense is behind iff off/wo <= def/wd.
	if b.offense_bought * wd <= b.defense_bought * wo then
		return AXIS_OFFENSE
	end
	return AXIS_DEFENSE
end

--- Whether the active challenge permits buying this offer *and* the offer is not
--- a strictly self-harmful trade the bot would die to. This is the single
--- chokepoint every buy path funnels through, so no selection route can
--- auto-pick a trap.
local function allowed(b, off)
	return Bot.challengePermits(b.challenge, off) and not isSelfHarmTrade(off)
end

--- Most EXPENSIVE affordable, ALLOWED offer matching `pred` (the bot snowballs
--- huge gold, so it should buy the most impactful item it can). Returns a
--- 1-based shop index, or nil.
local function priciestWhere(b, s, pred)
	local bestIdx, bestCost = nil, nil
	local offers = s.shop.offers
	for i = 1, #offers do
		local off = offers[i]
		if off.cost <= s.economy.gold and allowed(b, off) and pred(off) then
			if bestCost == nil or off.cost > bestCost then
				bestIdx, bestCost = i, off.cost
			end
		end
	end
	return bestIdx
end

--- Cheapest affordable slot whose offer matches `pred`, if any. Still the rule
--- for MODIFIERS in the generic fallback; weapons go through `bestWeapon`.
--- NOTE: unlike `priciestWhere`, the Rust does NOT apply `allowed` here — its one
--- caller folds `allowed` into `pred`. Transcribed as is.
local function cheapestWhere(s, pred)
	local bestIdx, bestCost = nil, nil
	local offers = s.shop.offers
	for i = 1, #offers do
		local off = offers[i]
		if off.cost <= s.economy.gold and pred(off) then
			if bestCost == nil or off.cost < bestCost then
				bestIdx, bestCost = i, off.cost
			end
		end
	end
	return bestIdx
end

--- The weapon bias for this match: the archetype's for a default run, the
--- balanced profile for a challenge run (which has no archetype identity — its
--- identity is the challenge).
local function weaponBias(b): WeaponBias
	if b.challenge.kind == "None" and b.archetype ~= nil then
		return archWeaponBias(b.archetype)
	end
	return BIAS_BALANCED
end

--- The gold-on-hand reference used to trade value off against price. Ranking is
--- by `value / (cost + ref)`:
---   * `ref ≈ 0` (broke) → the ranking is value-PER-GOLD, so a 500g weapon that
---     is half as good as a 5000g one wins, which is correct when the next
---     purchase is the only one you can afford;
---   * `ref` large (rich) → `cost` is noise beside it and the ranking becomes
---     value outright, which is also correct: when gold has stopped being the
---     constraint, buy the best thing on the board.
---
--- Capped so the comparison stays in a small integer range once the bot's economy
--- snowballs (gold reaches the billions late).
local function priceRef(s, bias: WeaponBias): number
	return math.clamp(idiv(s.economy.gold, math.max(bias.price_divisor, 1)), 0, PRICE_REF_CAP)
end

--- The valuation, for one offer, against an already-summarized arsenal:
---
---   value = own_power × (1 + synergy_after)          ← what it adds itself
---         + owned_power × Δsynergy                   ← what it adds to the rest
---
--- The second term is the whole point of the breadth incentive and the thing the
--- price-first policy could not see. Worked example with the shipped magnitudes:
--- holding 14 copies of one def, a new DISTINCT def moves the synergy by +5%,
--- worth `0.05 × 14p = 0.7p` on the existing stack alone — so a new weapon only
--- needs ~30% of a stacked weapon's raw power to be the better buy, and if it also
--- brings a new damage type or attack class (+10% / +8%) almost nothing beats it.
--- Once every axis is capped (+120%) Δ falls to zero and the bot correctly goes
--- back to stacking its strongest weapon.
local function weaponValue(ars: Arsenal, bias: WeaponBias, def: number): number
	local w = weaponDef(def)
	local copies = copiesOf(ars, def)
	-- What THIS copy adds, not what the first one was worth.
	local own = saturatedCopyPower(weaponPower(def, bias.utility_weight_pct), copies, bias.copy_saturation)

	local addsType = bit32.band(ars.type_mask, bit32.lshift(1, w.damage_type)) == 0
	local addsClass = bit32.band(ars.class_mask, bit32.lshift(1, Content.attackScopeId(w.attack))) == 0
	local addsDef = copies == 0

	local before = arsenalSynergyPct(ars.damage_types, ars.attack_classes, ars.distinct_defs)
	local after = arsenalSynergyPct(
		ars.damage_types + (if addsType then 1 else 0),
		ars.attack_classes + (if addsClass then 1 else 0),
		ars.distinct_defs + (if addsDef then 1 else 0)
	)
	-- The archetype's belief in breadth scales only the MARGIN, never the synergy
	-- the arsenal already has — that part is a fact, not an opinion.
	local delta = idiv((after - before) * bias.synergy_weight_pct, 100)
	return idiv(own * (100 + before + delta), 100) + idiv(ars.total_power * delta, 100)
end

--- Best affordable, allowed WEAPON offer by value-for-money, or nil. Returns a
--- 1-BASED shop index. `extra` narrows the candidate set (challenge runs use it to
--- require a class); pass `nil` for "any weapon".
---
--- `patiencePct` is the SAVE-UP rule. The bot declines to buy at all when the best
--- thing it can currently afford is worth less than `patiencePct`% of the best
--- weapon visible on the board — i.e. "there is something much better here, keep
--- the gold". Pass `0` to disable it (the weapon floor does, because board control
--- is not negotiable).
---
--- This is the other half of the fix, and it matters as much as the valuation.
--- Early on the bot earns ~500 gold a round and the shop shows one 500g Common
--- beside three 1500–3000g weapons worth five to nine times as much. Buying the
--- affordable one on repeat is how the arsenal used to become eleven copies of a
--- Common before the first shop ever refreshed — and no valuation can prevent
--- that, because when exactly one offer is affordable "cheapest" and "best" are
--- the same offer. Patience converts income into FEWER, BETTER weapons bought
--- LATER, which is both what a competent player does and what lets shop variety
--- reach the build at all.
---
--- Ties resolve to the LOWEST slot index (strict `>` improvement over an in-order
--- scan), so the choice is a pure, reproducible function of state.
local function bestWeapon(b, s, bias: WeaponBias, patiencePct: number, extra): number?
	local ars = scanArsenal(s, bias)
	local r = priceRef(s, bias)
	local bestIdx, bestValue, bestCost = nil, nil, nil
	local bestAffordableValue = 0
	local bestVisibleValue = 0
	local offers = s.shop.offers
	for i = 1, #offers do
		local off = offers[i]
		if isWeaponOffer(off) and allowed(b, off) and (extra == nil or extra(off)) then
			local v = weaponValue(ars, bias, off.def)
			-- The patience yardstick counts every weapon on the board, including
			-- the ones gold cannot reach yet — those are exactly what it saves for.
			if v > bestVisibleValue then
				bestVisibleValue = v
			end
			if off.cost <= s.economy.gold then
				local better
				if bestIdx == nil then
					better = true
				else
					-- value_i / (cost_i + r) > value_b / (cost_b + r), cross-multiplied.
					better = v * (bestCost + r) > bestValue * (off.cost + r)
				end
				if better then
					bestIdx, bestValue, bestCost = i, v, off.cost
					bestAffordableValue = v
				end
			end
		end
	end
	if patiencePct > 0 and bestAffordableValue * 100 < bestVisibleValue * patiencePct then
		return nil -- hold the gold; something much better is on the board.
	end
	return bestIdx
end

--- Best affordable slot holding a weapon this challenge wants, if any. For Purist
--- that's the target class; for JackOfAll an attack-class we don't own yet;
--- otherwise nil (generic buying handles the default bot). Within the wanted set
--- the pick is by VALUE-for-money, same as everywhere else — a challenge
--- constrains WHICH weapons are legal, it is not a reason to then take the worst
--- of them.
local function wantedWeapon(b, s)
	local challenge = b.challenge
	-- Patience 0: a challenge run's job is to ACQUIRE its class (that is what the
	-- achievement checks), so it takes what it can get rather than saving.
	return bestWeapon(b, s, weaponBias(b), 0, function(off)
		if challenge.kind == "Purist" then
			return weaponClass(off.def) == challenge.class
		elseif challenge.kind == "JackOfAll" then
			return bit32.band(s.bought_attack_mask, bit32.lshift(1, weaponClass(off.def))) == 0
		end
		return false
	end)
end

--- Can we reroll to fish for a wanted weapon (free, or affordable paid)?
local function canReroll(s)
	return s.economy.rerolls_remaining > 0 or s.economy.gold >= s.economy.reroll_cost
end

local function isModifierOffer(off)
	return not isWeaponOffer(off)
end

--- Choose this tick's action from the current state.
function Bot.decide(b, s)
	if s.dead then
		return INPUT_NOOP
	end
	if b.cooldown > 0 then
		b.cooldown -= 1
		return INPUT_NOOP
	end

	-- Emergency: swarmed and the Clear is off cooldown → wipe the board. A
	-- constrained run is weaker, so it leans on Clear sooner to survive long
	-- enough for its achievement to land.
	local clear_at = (b.challenge.kind == "None") and 14 or 8
	if #s.enemies >= clear_at and s.tick >= s.tank.clear_cooldown_end then
		b.cooldown = 30
		return INPUT_CLEAR
	end

	-- Purist / Jack-of-All: grab a wanted weapon; if none is offered, reroll to
	-- fish for one rather than spending the slot on something off-target.
	-- (Correctness no longer rides on this — the director/client buy-filter drops
	-- any disallowed purchase authoritatively — but fishing lets the bot acquire
	-- its class quickly so the achievement actually lands.)
	if b.challenge.kind == "Purist" or b.challenge.kind == "JackOfAll" then
		local idx = wantedWeapon(b, s)
		if idx ~= nil then
			b.cooldown = 4
			return buyOffer(idx - 1)
		end
		local jack_complete = b.challenge.kind == "JackOfAll" and s.bought_attack_mask == ALL_CLASSES
		if not jack_complete and canReroll(s) then
			b.cooldown = 8
			return INPUT_REROLL
		end
	end

	-- Default survivor: spend along this match's ARCHETYPE (chosen
	-- deterministically from the seed), so a seed sweep shows a real spread of
	-- builds. Order:
	--   (1) hold the archetype's weapon floor,
	--   (2) snowball ECONOMY while its window is open,
	--   (3) keep the arsenal topped up toward the archetype's weapon target,
	--   (4) buy the next COMBAT-AXIS modifier (offense/defense) that keeps the
	--       build near the archetype's offense:defense ratio — taking the most
	--       impactful (priciest) affordable item, since the bot snowballs gold.
	-- Falls through to the generic best-affordable scan only if nothing above
	-- fits (e.g. a shop with no matching axis offer).
	local arch_target_weapons = b.target_weapons
	if b.challenge.kind == "None" then
		local arch = archetype(b, s)
		local round = (s.round == U32_MAX) and 0 or s.round
		-- The generic fallback below honors the same PACE, so no path can dump the
		-- whole arsenal budget into a single early shop.
		arch_target_weapons = targetWeapons(arch)
		local floor = weaponFloor(arch, round)
		local bias = archWeaponBias(arch)
		-- (1) Weapon floor — the BEST weapon on the board for the money, not the
		-- cheapest one (see the valuation block above). Patience 0 here: below the
		-- floor the tank has no board control, and a weapon now beats a better
		-- weapon later.
		if #s.weapons < floor then
			local idx = bestWeapon(b, s, bias, 0, nil)
			if idx ~= nil then
				b.cooldown = 6
				return buyOffer(idx - 1)
			end
		end
		-- (2) Economy window: snowball income while rounds remain to pay it back.
		if canBuyMod(b) and round <= econLastRound(arch) then
			local idx = priciestWhere(b, s, function(o)
				return isModifierOffer(o) and modIsEconomy(o.def)
			end)
			if idx ~= nil then
				return buyMod(b, idx, AXIS_ECONOMY)
			end
		end
		-- (3) Keep the arsenal topped up toward the archetype's weapon target. How
		-- fast that happens is governed by PATIENCE, not by a quota: the bot buys
		-- when the board has something worth its gold (see `bestWeapon`), which
		-- spreads acquisition across shops and a growing wallet on its own.
		if canBuyWeapon(s) and #s.weapons < arch_target_weapons then
			local idx = bestWeapon(b, s, bias, bias.patience_pct, nil)
			if idx ~= nil then
				b.cooldown = 6
				return buyOffer(idx - 1)
			end
		end
		-- (4) Combat-axis modifier, holding the archetype's offense:defense ratio.
		if canBuyMod(b) then
			local want = nextAxis(b, arch)
			-- Survival floor: the scaling contact damage one-shots a flat-HP tank,
			-- so when buying DEFENSE the bot prefers a MAX-HP item (the only stat
			-- that scales the pool to absorb a leak) over flat armor/regen.
			if want == AXIS_DEFENSE then
				local idx = priciestWhere(b, s, function(o)
					return isModifierOffer(o) and raisesMaxHp(o.def)
				end)
				if idx ~= nil then
					return buyMod(b, idx, AXIS_DEFENSE)
				end
			end
			-- Try the wanted axis first (priciest = most impactful), then the other
			-- axis as a fallback so a shop missing the wanted axis still progresses.
			local axes = { want, otherAxis(want) }
			for k = 1, 2 do
				local axis = axes[k]
				local idx = priciestWhere(b, s, function(o)
					return isModifierOffer(o) and modAxis(o.def) == axis
				end)
				if idx ~= nil then
					return buyMod(b, idx, axis)
				end
			end
		end
	end

	-- Generic fallback: buy the best affordable, challenge-allowed offer. Prefer a
	-- weapon while under the target arsenal size. For the default bot, weapons stop
	-- at the ARCHETYPE target (not the hard cap), so a tanky build keeps a lean
	-- arsenal and a glass-cannon a fat one — i.e. the archetype's weapon count is
	-- preserved instead of every build draining its leftover gold into the same
	-- 40-weapon ceiling. (`WEAPON_BUY_CAP` remains the absolute safety ceiling for
	-- challenge runs / edge cases.)
	--
	-- Weapons are ranked by VALUE-for-money here too, so no path — floor, top-up,
	-- or this fallback — can quietly reintroduce the price-first pick. Modifiers
	-- keep the cheapest-first rule (their axis preference is handled above, and
	-- modifier valuation is out of scope).
	local want_weapon = #s.weapons < arch_target_weapons
	local mods_ok = canBuyMod(b) or b.challenge.kind ~= "None"
	local weapons_ok
	if b.challenge.kind == "None" then
		weapons_ok = #s.weapons < arch_target_weapons
	else
		weapons_ok = canBuyWeapon(s)
	end
	local bias = weaponBias(b)
	local weaponPick = if weapons_ok then bestWeapon(b, s, bias, bias.patience_pct, nil) else nil
	local modPick = if mods_ok
		then cheapestWhere(s, function(o)
			return isModifierOffer(o) and allowed(b, o)
		end)
		else nil
	-- Kind preference wins outright; the other kind is the fallback.
	local ordered = if want_weapon then { weaponPick, modPick } else { modPick, weaponPick }
	local offers = s.shop.offers
	for k = 1, 2 do
		local idx = ordered[k]
		if idx ~= nil then
			if isWeaponOffer(offers[idx]) then
				b.cooldown = 6
				return buyOffer(idx - 1)
			end
			return buyMod(b, idx, modAxis(offers[idx].def))
		end
	end

	-- Nothing affordable/allowed — spend a free reroll to fish for options.
	if s.economy.rerolls_remaining > 0 then
		b.cooldown = 20
		return INPUT_REROLL
	end
	return INPUT_NOOP
end

return Bot
