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

local U32_MAX = 4294967295

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

--- `ModEffect::is_economy` — IncomeFlat | IncomePct | GrantGold.
local ECONOMY_TAGS = { [5] = true, [17] = true, [26] = true }

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

--- `ModifierDef::is_economy` — any effect is an economy effect.
local function modIsEconomy(def)
	local m = modifierDef(def)
	local fx = m.effects
	for i = 1, #fx do
		local tag = words(fx[i])
		if ECONOMY_TAGS[tag] then
			return true
		end
	end
	return false
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

--- Cheapest affordable slot whose offer matches `pred`, if any. NOTE: unlike
--- `priciestWhere`, the Rust does NOT apply `allowed` here — transcribed as is.
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

--- Cheapest affordable slot holding a weapon this challenge wants, if any. For
--- Purist that's the target class; for JackOfAll an attack-class we don't own
--- yet; otherwise nil (generic buying handles the default bot).
local function wantedWeapon(b, s)
	local pickIdx, pickCost = nil, nil
	local offers = s.shop.offers
	for i = 1, #offers do
		local off = offers[i]
		if off.cost <= s.economy.gold and isWeaponOffer(off) then
			local want = false
			if b.challenge.kind == "Purist" then
				want = weaponClass(off.def) == b.challenge.class
			elseif b.challenge.kind == "JackOfAll" then
				want = bit32.band(s.bought_attack_mask, bit32.lshift(1, weaponClass(off.def))) == 0
			end
			if want and (pickCost == nil or off.cost < pickCost) then
				pickIdx, pickCost = i, off.cost
			end
		end
	end
	return pickIdx
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
		arch_target_weapons = targetWeapons(arch)
		local round = (s.round == U32_MAX) and 0 or s.round
		local floor = weaponFloor(arch, round)
		-- (1) Weapon floor.
		if #s.weapons < floor then
			local idx = cheapestWhere(s, isWeaponOffer)
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
		-- (3) Keep the arsenal topped up toward the archetype's weapon target.
		if canBuyWeapon(s) and #s.weapons < arch_target_weapons then
			local idx = cheapestWhere(s, isWeaponOffer)
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
	-- weapon while under the target arsenal size; among same-kind, cheapest. For
	-- the default bot, weapons stop at the ARCHETYPE target (not the hard cap), so
	-- a tanky build keeps a lean arsenal and a glass-cannon a fat one.
	-- (`WEAPON_BUY_CAP` remains the absolute safety ceiling for challenge runs.)
	local want_weapon = #s.weapons < arch_target_weapons
	local mods_ok = canBuyMod(b) or b.challenge.kind ~= "None"
	local weapons_ok
	if b.challenge.kind == "None" then
		weapons_ok = #s.weapons < arch_target_weapons
	else
		weapons_ok = canBuyWeapon(s)
	end
	local bestIdx, bestCost, bestWeapon = nil, nil, nil
	local offers = s.shop.offers
	for i = 1, #offers do
		local off = offers[i]
		local is_weapon = isWeaponOffer(off)
		local kind_ok = is_weapon and weapons_ok or (not is_weapon and mods_ok)
		if off.cost <= s.economy.gold and allowed(b, off) and kind_ok then
			local better
			if bestIdx == nil then
				better = true
			elseif is_weapon ~= bestWeapon then
				-- Kind preference wins outright.
				better = (is_weapon == want_weapon)
			else
				better = off.cost < bestCost
			end
			if better then
				bestIdx, bestCost, bestWeapon = i, off.cost, is_weapon
			end
		end
	end
	if bestIdx ~= nil then
		if bestWeapon then
			b.cooldown = 6
			return buyOffer(bestIdx - 1)
		end
		return buyMod(b, bestIdx, modAxis(offers[bestIdx].def))
	end

	-- Nothing affordable/allowed — spend a free reroll to fish for options.
	if s.economy.rerolls_remaining > 0 then
		b.cooldown = 20
		return INPUT_REROLL
	end
	return INPUT_NOOP
end

return Bot
