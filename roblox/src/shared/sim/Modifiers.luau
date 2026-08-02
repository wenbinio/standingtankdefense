--!nonstrict
--[[
	Modifiers.luau — transcription of `sim-core/crates/sim/src/modifiers.rs`
	(`roblox/SIM-SPEC.md` §S1: this is a transcription, not a redesign).

	The modifier / stacking engine (`docs/05 §5.3`). Folds purchased modifiers
	into a small aggregate and answers the live queries damage resolution and
	weapon firing ask each tick. Deterministic: pure fixed-point, no RNG, no
	allocation beyond the aggregate itself. Struct fields mirror `state.rs` (so
	snapshots/checksums can serialize them); the behavior lives here.

	Stacking rule (from the source map): **additive within a kind, multiplicative
	across distinct multiplicative sources**. So all `+%` of the same additive
	flavor sum, then the result is multiplied by each independent `×` source.

	TRANSCRIPTION NOTES (Luau-specific, no behavior of their own)
	------------------------------------------------------------
	* `ModEffect` arrives from `content.json` in the `words()` encoding
	  (`CONTRACTS.md` C1): `{ tag, a, b, c }`. The dispatch table below is keyed
	  by that tag and each entry is named after the Rust variant it ports.
	* Rust `Fixed` values stay `Fixed.luau` values and are NEVER indexed (§S4).
	  Rust `i64`/`u32` stay plain Luau numbers.
	* Content ids are 0-based; Luau arrays are 1-indexed (§S3). Every such
	  boundary is spelled out with an explicit `+ 1`.
	* No Roblox global is referenced.
	* The soft caps' `SOFT^3 / mag^3` intermediates are i128 in the Rust and run
	  to ~2.5e26, far past 2^53. They are carried here as base-2^16 limb
	  magnitudes (see "exact wide integers" below) — never as raw Luau numbers.
]]

local Fixed = require("../Fixed")
local Content = require("../Content")

local Modifiers = {}

--[[
	Hard i64 backstop for the compounding percentage riders (`MaxHpPct` /
	`HpRegenPct` / `ManaRegenPct`). Each such rider multiplies its stat by ~1.25
	per purchase, so before the soft caps below a pathological repeat-buy stack
	could run a stat to i64 overflow and poison every downstream `hp + x`. Since
	the soft caps landed this is belt-and-braces: the measured max Max HP over 80
	seeds is now ~434k, seven orders of magnitude under this ceiling, so the clamp
	never trips. Kept anyway — it is free, bit-stable across platforms, and guards
	any future effect that writes these stats without going through
	`addSoftCapped`.
]]
local STAT_CEIL = 1000000000000

-- u32::MAX, for the `saturating_mul` in the Dodge cap.
local U32_MAX = 4294967295

local ZERO = Fixed.ZERO
local ONE = Fixed.ONE
local add = Fixed.add
local sub = Fixed.sub
local mul = Fixed.mul
local fromRatio = Fixed.fromRatio
local fromInt = Fixed.fromInt

-- `Fixed::scale_i64` returning a plain Luau number. `scaleI64` yields an opaque
-- i64 value (so the saturation boundary stays exact); every consumer here feeds
-- it straight into integer arithmetic, exactly as the Rust does. `toInt` ERRORS
-- above 2^53 rather than rounding — per §S4 every quantity that reaches here
-- sits far inside that, and a loud failure beats a silently wrong stat.
local function scaleToInt(f, v: number): number
	return Fixed.toInt(Fixed.scaleI64(f, v))
end

local function isZero(f): boolean
	return Fixed.eq(f, ZERO)
end

--[[
	==================== exact wide integers (soft-cap intermediates) ====================

	The soft caps below form `raw_inc × SOFT^3` over `|stat|^3`. In the Rust both
	live in an `i128`: the numerator peaks near 2.5e26 (~2^88, per the comment on
	`soft_capped_inc`) and the denominator, with `|stat|` allowed all the way up to
	`STAT_CEIL`, reaches 1e36 (~2^120). Both are far past 2^53, so NOTHING in that
	computation may touch a raw Luau number — a plain `a * b` would silently drop
	low bits and the truncating divide would then land on the wrong integer.

	`Fixed` cannot carry them either: it is an i64 with *saturating* narrowing, and
	these intermediates need 128 bits of headroom before the divide brings them
	back into i64 range. So magnitudes are carried here as little-endian base-2^16
	limb arrays — the same representation, and the same reasoning, as `Fixed.luau`
	itself: a limb×limb product is < 2^32 and a whole accumulation column stays
	under 2^34, so every partial product is an exact double. Nothing below ever
	forms a Luau number larger than 2^34.

	These are magnitudes (unsigned). Sign is handled by the callers, which mirror
	Rust's TRUNCATE-TOWARD-ZERO integer division by dividing magnitudes and
	reapplying the sign afterwards.
]]

local BIG_BASE = 65536

-- Drop leading zero limbs. Magnitudes are kept normalized so `magCmp` may compare
-- lengths first; the empty array is zero.
local function magTrim(t)
	local n = #t
	while n > 0 and t[n] == 0 do
		t[n] = nil
		n -= 1
	end
	return t
end

--- A non-negative integral Luau number (|v| <= 2^53) as limbs.
local function magFromInt(v: number)
	local t = {}
	local n = 0
	while v > 0 do
		local l = v % BIG_BASE
		n += 1
		t[n] = l
		v = (v - l) / BIG_BASE
	end
	return t
end

--- Schoolbook multiply. Each `ai * b[j]` is < 2^32 and each accumulator holds one
--- such product plus a limb plus a carry, so every `t` here is < 2^34 — exact.
local function magMul(a, b)
	local la, lb = #a, #b
	if la == 0 or lb == 0 then
		return {}
	end
	local p = table.create(la + lb, 0)
	for i = 1, la do
		local ai = a[i]
		if ai ~= 0 then
			local carry = 0
			for j = 1, lb do
				local k = i + j - 1
				local t = p[k] + ai * b[j] + carry
				local r = t % BIG_BASE
				p[k] = r
				carry = (t - r) / BIG_BASE
			end
			-- The product of `la + lb` limbs never needs a `la + lb + 1`st, so this
			-- propagation always terminates inside `p`.
			local k = i + lb
			while carry ~= 0 do
				local t = p[k] + carry
				local r = t % BIG_BASE
				p[k] = r
				carry = (t - r) / BIG_BASE
				k += 1
			end
		end
	end
	return magTrim(p)
end

--- `m ^ k` for a small non-negative `k`. Mirrors the Rust's `for _ in
--- 0..SOFT_FALLOFF_POW { num *= s; den *= mag }` — same product, same exponent
--- constant, so raising the exponent needs no change here.
local function magPow(m, k: number)
	local r = { 1 }
	for _ = 1, k do
		r = magMul(r, m)
	end
	return r
end

--- Unsigned compare: -1 / 0 / 1. Operands must be normalized.
local function magCmp(a, b)
	local la, lb = #a, #b
	if la ~= lb then
		return la < lb and -1 or 1
	end
	for i = la, 1, -1 do
		if a[i] ~= b[i] then
			return a[i] < b[i] and -1 or 1
		end
	end
	return 0
end

--- `floor(num / den)` for magnitudes; `den` must be non-zero.
---
--- Restoring binary long division: the remainder takes one bit of the numerator at
--- a time and is compared against (and conditionally reduced by) the divisor. The
--- only numbers this forms are single limbs doubled plus a carry bit (< 2^17), so
--- it is exact at ANY operand width — which is the point, since `den` here can be
--- 120 bits wide. Knuth-D would be faster; this runs only when a stat is past its
--- knee, i.e. on a purchase, never in a per-tick loop, so clarity wins.
local function magDiv(num, den)
	if magCmp(num, den) < 0 then
		return {}
	end
	local n = #num
	local dn = #den
	local q = table.create(n, 0)
	local r = {}
	local rn = 0
	for i = n, 1, -1 do
		local limb = num[i]
		local qd = 0
		for b = 15, 0, -1 do
			-- r = r * 2 + bit b of limb
			local carry = bit32.band(bit32.rshift(limb, b), 1)
			for k = 1, rn do
				local t = r[k] * 2 + carry
				if t >= BIG_BASE then
					r[k] = t - BIG_BASE
					carry = 1
				else
					r[k] = t
					carry = 0
				end
			end
			if carry ~= 0 then
				rn += 1
				r[rn] = carry
			end
			qd *= 2
			if rn > dn or (rn == dn and magCmp(r, den) >= 0) then
				-- r -= den
				local borrow = 0
				for k = 1, rn do
					local t = r[k] - (den[k] or 0) - borrow
					if t < 0 then
						r[k] = t + BIG_BASE
						borrow = 1
					else
						r[k] = t
						borrow = 0
					end
				end
				while rn > 0 and r[rn] == 0 do
					r[rn] = nil
					rn -= 1
				end
				qd += 1
			end
		end
		q[i] = qd
	end
	return magTrim(q)
end

--- A magnitude back to a plain Luau number, ERRORING above 2^53 rather than
--- rounding — the `Fixed.toInt` discipline (§S4): a loud failure beats a silently
--- wrong stat. The range check runs on the LIMBS, because composing first would
--- already have rounded. Every quotient that reaches here is bounded in magnitude
--- by its own `raw_inc`, which arrived as an exact Luau number, so this is
--- belt-and-braces.
local function magToInt(m): number
	local n = #m
	if n > 4 or (n == 4 and (m[4] > 32 or (m[4] == 32 and (m[1] ~= 0 or m[2] ~= 0 or m[3] ~= 0)))) then
		error("Modifiers: soft-cap quotient exceeds 2^53 and is not exactly representable", 2)
	end
	local v = 0
	for i = n, 1, -1 do
		v = v * BIG_BASE + m[i]
	end
	return v
end

--- Exact decimal rendering of a magnitude, for handing a wide result back to
--- `Fixed.fromRaw` (which parses the full i64 range from a string). Short division
--- by 10: the remainder is < 10, so `rem * 2^16 + limb < 10 * 2^16` and each
--- quotient limb stays < 2^16 — every intermediate exact.
local function magToDecimal(m): string
	local n = #m
	if n == 0 then
		return "0"
	end
	local t = table.create(n, 0)
	for i = 1, n do
		t[i] = m[i]
	end
	local digits = {}
	repeat
		local rem = 0
		for i = #t, 1, -1 do
			local cur = rem * BIG_BASE + t[i]
			rem = cur % 10
			t[i] = (cur - rem) / 10
		end
		digits[#digits + 1] = string.char(48 + rem)
		magTrim(t)
	until #t == 0
	-- digits came out least-significant first
	local out = table.create(#digits, "")
	local n2 = #digits
	for i = 1, n2 do
		out[i] = digits[n2 + 1 - i]
	end
	return table.concat(out)
end

--- The limbs of a `Fixed`'s raw i64 MAGNITUDE, plus its sign. Goes through the
--- sanctioned `Fixed.hi`/`Fixed.lo` accessors, so the opaque value is never
--- indexed (§S4) and no intermediate exceeds 2^32. `i64::MIN` is special-cased
--- because `Fixed.neg` saturates there (it would report 2^63 - 1, one short) —
--- unreachable for any real factor, but the Rust's `as i128` widening is exact and
--- this keeps the transcription exact too.
local function fixedRawMag(f)
	if Fixed.eq(f, Fixed.I64_MIN) then
		return { 0, 0, 0, 32768 }, true -- 2^63
	end
	local neg = Fixed.lt(f, ZERO)
	local v = neg and Fixed.neg(f) or f
	local hi = Fixed.hi(v)
	local lo = Fixed.lo(v)
	local l1 = lo % BIG_BASE
	local l3 = hi % BIG_BASE
	return magTrim({ l1, (lo - l1) / BIG_BASE, l3, (hi - l3) / BIG_BASE }), neg
end

--[[
	========================= Power-curve bounding (docs/11 §11.2) =========================

	Multiplicative stacking is the intended engine (`[02] §2.1`) — the problem was
	never that it is strong, it is that it was UNBOUNDED, and specifically unbounded
	in the TAIL. Measured over 80 seeds (35-min horizon) the distribution of peak
	Max HP was p50 24,000 (the STARTING value — over half of all runs never buy Max
	HP), p90 117,000, p99 3.72e9. So this is a tail problem, not a central-tendency
	problem: a flat nerf or a low hard cap would flatten a fantasy the median run is
	not even having. What is wanted is something invisible at p50–p90 and brutal at
	p99. Hence a SOFT CAP with a CUBIC tail, every knee placed just above the
	measured p90:

	    effective_increment = raw_increment                     while stat <= SOFT
	    effective_increment = raw_increment × (SOFT / stat)^3    while stat >  SOFT

	Properties that matter:
	  * CONTINUOUS at the knee (the factor is exactly 1 when `stat == SOFT`), so the
	    curve flattens rather than cliffs — no purchase ever becomes worthless, and
	    more is always strictly more.
	  * Below the knee NOTHING changes, bit for bit. Nine runs in ten are unaffected.
	  * Above the knee the stat grows like `SOFT × (1 + ¾n)^⅓` in the number of
	    purchases instead of `×1.25^n`. Sixty compounding buys move Max HP from 130k
	    to ~470k rather than to 3.7 billion.

	Pure integer math on the limb magnitudes above — no floats, bit-stable, and it
	feeds `state_checksum`.
]]

--- Falloff exponent above a knee. 3 = cubic. Raising it bites the tail harder
--- without moving anything below the knee.
local SOFT_FALLOFF_POW = 3

-- The four defensive knees below were re-placed by the `docs/11 §11.8` pass. The
-- RULE is unchanged — a knee sits just clear of the measured p90 — but the
-- distributions underneath moved, because Max HP and the Mana-Shield pool stopped
-- being inert buckets and became the sustain stats (see `grantPoolRegen` /
-- `grantPoolRecharge`).

--- Soft-cap knee for **Max HP**, flat and `%` sources alike. LOWERED from 130,000
--- because the per-item Max-HP grants roughly doubled in the same pass.
local MAX_HP_SOFT = 90000
--- Soft-cap knee for **HP regen**, in HP per tick. RAISED from 16,000: regen is
--- now the stat that Max HP feeds, and it has to answer a median 3,387 HP/tick
--- deficit (p99 43,030).
local HP_REGEN_SOFT = 120000
--- Soft-cap knee for **Mana-Shield regen**, in shield per tick. RAISED from 2,000:
--- at that knee a 158,000 pool took 79 ticks to refill, so the pool was a one-shot
--- buffer and never a sustain path.
local MANA_REGEN_SOFT = 60000
--- Soft-cap knee for the **Mana-Shield pool** — a second HP bar, so it gets the
--- same treatment as Max HP. RAISED from 100,000; the shield absorbs before HP and
--- carries no `docs/11 §11.2` magnitude target, so it has headroom Max HP does not.
local MANA_SHIELD_SOFT = 300000
--- Soft-cap knee for **Armor**. Armor is flat subtraction, so an unbounded value
--- is literal invulnerability. Measured p90 660, p99 17,240.
local ARMOR_SOFT = 1000
--- Soft-cap knee for the multiplicative damage product `mul_global`, as a raw
--- `Fixed` (×8). Measured p90 is ×1.0 — nine runs in ten never buy a single
--- multiplicative damage source — while p99 reached ×4.18e8. Nine `+25% Damage
--- (Epic)` purchases land at the knee at full value; past it each further
--- multiplicative source contributes a shrinking factor.
local DAMAGE_MUL_SOFT = Fixed.fromRaw(8 * 65536) -- Rust: `Fixed::from_raw(8 << 16)`
--- Soft-cap knee for **income-as-HP-regen**, as a raw `Fixed` (×0.25 = 25% of the
--- per-tick gold award healed). `Entangled Gold Mine` grants exactly +25% per copy,
--- so this knee is "one copy": copies one and two land at full value and every copy
--- after them decays cubically. Much tighter than the other knees, deliberately —
--- see `EFFECTS[18]`.
local INCOME_REGEN_SOFT = Fixed.fromRaw(16384) -- Rust: `Fixed::from_raw(1 << 14)`

--- `raw_inc × (soft / |stat|)^SOFT_FALLOFF_POW`, returned unchanged below the
--- knee. `soft` must be > 0. The result is always <= `raw_inc` in magnitude, so
--- the Rust's narrowing back to i64 cannot overflow and neither can this.
---
--- Rust's `i128` `/` TRUNCATES TOWARD ZERO, so the division runs on magnitudes and
--- the sign is reapplied afterwards (`//` would floor toward negative infinity and
--- be off by one for every inexact reduction — §S4).
local function softCappedInc(rawInc: number, stat: number, soft: number): number
	local mag = stat < 0 and -stat or stat
	if mag <= soft then
		return rawInc
	end
	if rawInc == 0 then
		return 0
	end
	local neg = rawInc < 0
	local ar = neg and -rawInc or rawInc
	local num = magMul(magFromInt(ar), magPow(magFromInt(soft), SOFT_FALLOFF_POW))
	local den = magPow(magFromInt(mag), SOFT_FALLOFF_POW)
	local q = magToInt(magDiv(num, den))
	return neg and -q or q
end

--- Apply `raw_inc` to `t[key]` through the soft cap, clamp to the `STAT_CEIL`
--- backstop, and return the increment that was ACTUALLY applied (so a caller can
--- keep a paired value — current HP behind max HP, current shield behind its pool
--- — exactly in step). Reductions (`raw_inc < 0`) are attenuated the same way,
--- which keeps the function monotone and its inverse well behaved.
---
--- The Rust's `saturating_add` is unreachable here and so is not emulated: `stat`
--- is bounded by `STAT_CEIL` (1e12) and `|inc| <= |raw_inc|`, which arrived as an
--- exact Luau number, so the sum is exact and the clamp is what actually binds.
local function addSoftCapped(t, key: string, rawInc: number, soft: number): number
	local stat = t[key]
	local inc = softCappedInc(rawInc, stat, soft)
	local nextStat = stat + inc
	if nextStat > STAT_CEIL then
		nextStat = STAT_CEIL
	elseif nextStat < -STAT_CEIL then
		nextStat = -STAT_CEIL
	end
	t[key] = nextStat
	return nextStat - stat
end

--- Denominator of the Max-HP → HP-regen grant, in `max_hp²` units per HP/tick.
local POOL_REGEN_SCALE = 500000
--- Denominator of the Mana-Shield pool → shield-regen grant.
local POOL_RECHARGE_SCALE = 500000

--- Shared body of the two pool-fed sustain grants (`docs/11 §11.8`): Max HP and
--- the Mana-Shield pool each buy their OWN faucet, so a pool is the sustain stat
--- rather than a bucket that merely delays a loss.
---
--- The grant is `inc × pool / scale`, i.e. QUADRATIC in the pool. That shape is
--- forced: the opening leak is ~12–20 HP/tick so two cheap Masonry buys must be
--- worth tens of HP/tick, while a 170k end-game fortress needs thousands — a
--- ~1,000× span demanded across a ~7× span in pool size, which no constant divisor
--- covers. Squaring turns the 7× into ~50×.
---
--- Rust computes `(i128 × i128 / i128) as i64` and then tests `> 0`, so a
--- non-positive `inc` or a non-positive pool grants nothing.
---
--- EXACTNESS. This cannot call `addSoftCapped`, because the raw grant is the one
--- place in this file where an INTERMEDIATE legitimately leaves the exact-double
--- range: `inc × pool` is quadratic, and a pathological Max-HP stack pushes it past
--- 2^53 while the soft-capped RESULT stays tiny. Rust's `i128` carries that
--- intermediate for free; here it stays a magnitude, and the knee's cubic
--- attenuation is applied to the magnitude — so `magToInt` is only ever reached on
--- a value the cap has already brought back inside range. The destination stat is a
--- regen rate and is never negative, so only the positive side of `addSoftCapped`'s
--- clamp is reproduced; above `STAT_CEIL` the answer is `STAT_CEIL` either way and
--- the wide magnitude never has to be narrowed at all.
local function grantPoolFed(t, key: string, poolValue: number, inc: number, scale: number, soft: number)
	if inc <= 0 or poolValue <= 0 then
		return
	end
	local g = magDiv(magMul(magFromInt(inc), magFromInt(poolValue)), magFromInt(scale))
	if #g == 0 then
		return -- the truncating division floored to zero: Rust's `g > 0` fails
	end
	local stat = t[key]
	local mag = stat < 0 and -stat or stat
	if mag > soft then
		g = magDiv(magMul(g, magPow(magFromInt(soft), SOFT_FALLOFF_POW)), magPow(magFromInt(mag), SOFT_FALLOFF_POW))
	end
	local nextStat
	if magCmp(g, magFromInt(STAT_CEIL)) >= 0 then
		nextStat = STAT_CEIL
	else
		nextStat = stat + magToInt(g)
		if nextStat > STAT_CEIL then
			nextStat = STAT_CEIL
		end
	end
	t[key] = nextStat
end

--- Max HP buys SUSTAIN, not just a bigger bucket. See `grantPoolFed`.
local function grantPoolRegen(tank, maxHpInc: number)
	grantPoolFed(tank, "hp_regen_per_tick", tank.max_hp, maxHpInc, POOL_REGEN_SCALE, HP_REGEN_SOFT)
end

--- The Mana Shield's twin: a bigger pool recharges faster. See `grantPoolFed`.
local function grantPoolRecharge(tank, poolInc: number)
	grantPoolFed(tank, "mana_regen_per_tick", tank.mana_shield_max, poolInc, POOL_RECHARGE_SCALE, MANA_REGEN_SOFT)
end

--- `Fixed` twin of `softCappedInc`, for the multiplicative damage product.
--- Attenuates the *added* factor (the `+25%` of a `×1.25` source), never the
--- product already banked, so the curve is monotone and continuous at the knee.
---
--- The quotient is handed back through an exact decimal string rather than a Luau
--- number, because a `Fixed` raw spans the full i64 range. Rust's `as i64` here is
--- a WRAPPING cast; it cannot fire (the quotient's magnitude is bounded by
--- `factor`'s own raw), and `Fixed.fromRaw` would error rather than wrap if it
--- somehow did.
local function softCappedFactor(factor, current, soft)
	if Fixed.le(current, soft) then
		return factor
	end
	local fMag, fNeg = fixedRawMag(factor)
	if #fMag == 0 then
		return factor
	end
	local num = magMul(fMag, magPow((fixedRawMag(soft)), SOFT_FALLOFF_POW))
	local den = magPow((fixedRawMag(current)), SOFT_FALLOFF_POW)
	local dec = magToDecimal(magDiv(num, den))
	return Fixed.fromRaw(fNeg and ("-" .. dec) or dec)
end

--[[
	======================= Arsenal breadth synergy (docs/11 §11.2) =======================

	Measured failure: a sampled run's arsenal was `Magic Bolt ×14` — one weapon
	bought fourteen times. Copies stacked cleanly and NOTHING rewarded breadth, so
	an 86-weapon catalog collapsed to "find the best, buy it repeatedly".

	Note the shape of the old incentive: an extra copy and an extra DISTINCT weapon
	were exactly equivalent — both just add one more firing unit. Breadth was not
	worse, it was *neutral*, and a tie is decided by whichever weapon happens to be
	cheapest or strongest. So the cheapest possible thumb on the scale flips it.

	This is deliberately a BONUS, not a nerf on copies: your fourteenth Magic Bolt
	is worth exactly what it always was. A build that covers more damage types /
	attack classes / distinct weapons simply gets a global additive damage bonus on
	top. The bonus is derived LIVE from the owned arsenal (never baked at purchase),
	so selling into or growing out of breadth tracks immediately.

	  synergy% = 10% × (distinct damage types  − 1)   capped at 4 extra (+40%)
	           +  8% × (distinct attack classes − 1)   capped at 5 extra (+40%)
	           +  5% × (distinct weapon defs    − 1)   capped at 8 extra (+40%)

	Maximum +120%, reached only by a genuinely wide build (all 5 damage types, all 6
	attack classes, 9+ distinct weapons). `Magic Bolt ×14` scores exactly +0%. It is
	GLOBAL ADDITIVE and folds in through `selfScalingAdd` — i.e. it lands in the
	same additive pool as `add_global`/`add_self`/`add_dyn` and is then multiplied by
	`mul_global`, exactly like every other additive source (`[02] §2.1`). The
	composition ORDER is unchanged.
]]

--- Additive % per distinct damage type beyond the first, over 100.
local SYNERGY_PER_TYPE = 10
--- Max extra damage types counted (5 types => 4 beyond the first).
local SYNERGY_TYPE_CAP = 4
--- Additive % per distinct attack class beyond the first, over 100.
local SYNERGY_PER_CLASS = 8
--- Max extra attack classes counted (6 classes => 5 beyond the first).
local SYNERGY_CLASS_CAP = 5
--- Additive % per distinct weapon def beyond the first, over 100.
local SYNERGY_PER_DEF = 5
--- Max extra distinct weapon defs counted.
local SYNERGY_DEF_CAP = 8
--- Bitset width for the distinct-weapon-def count: Rust `DEF_BITSET_WORDS * 64`
--- (4 × u64). A def id at or beyond it is not counted, which can only ever
--- UNDER-count — never error — exactly as the Rust's guard does.
local DEF_BITSET_BITS = 256
--- Same set, as 32-bit `bit32` words.
local DEF_BITSET_WORDS32 = DEF_BITSET_BITS // 32

-- `Fixed::from_ratio(per × extra, 100)` for every reachable `extra`, memoised at
-- module scope. This is a cache of a pure function of two constants, so every
-- entry is bit-identical to the Rust's — and it keeps the fire-time path free of
-- `Fixed` division. NOTE the Rust multiplies the NUMERATOR by `extra` before
-- forming the ratio (`from_ratio(10 * extra, 100)`); `extra × from_ratio(10, 100)`
-- is a DIFFERENT number once the ratio is inexact, so the memo is built the same
-- way round.
local function synergyTable(per: number, cap: number)
	local t = table.create(cap + 1, ZERO)
	for extra = 0, cap do
		t[extra + 1] = fromRatio(per * extra, 100)
	end
	return t
end

local SYNERGY_TYPE_ADD = synergyTable(SYNERGY_PER_TYPE, SYNERGY_TYPE_CAP)
local SYNERGY_CLASS_ADD = synergyTable(SYNERGY_PER_CLASS, SYNERGY_CLASS_CAP)
local SYNERGY_DEF_ADD = synergyTable(SYNERGY_PER_DEF, SYNERGY_DEF_CAP)

-- Scratch for the distinct-weapon-def bitset, cleared on entry to
-- `arsenalSynergyAdd`. Module scope so the fire-time path allocates nothing;
-- `arsenalSynergyAdd` runs straight through without calling back into itself, so
-- there is no re-entrancy hazard.
local defWords = table.create(DEF_BITSET_WORDS32, 0)

--- Rust's `(n - 1).clamp(0, cap)`.
local function synergyExtra(n: number, cap: number): number
	local e = n - 1
	if e < 0 then
		return 0
	elseif e > cap then
		return cap
	end
	return e
end

-- ============================ construction ============================

--- `Modifiers::new`.
function Modifiers.new()
	local addByType = table.create(5, ZERO)
	local addByScope = table.create(Content.NUM_SCOPES, ZERO)
	return {
		add_global = ZERO,
		add_by_type = addByType,
		add_by_scope = addByScope,
		mul_global = ONE,
		attack_speed = ZERO,
		vs_stunned = ZERO,
		vs_poisoned = ZERO,
		poison_dmg_mult = ONE,
		stun_dur_mult = ONE,
		weapon_count_scaling = {},
		dmg_per_maxhp_rate = ZERO,
		dmg_per_bounty_rate = ZERO,
		shield_active_dmg = ZERO,
	}
end

-- ============================ ramps ============================

--- Phase: apply each active time-scaling ramp whose interval has elapsed
--- (`docs/06`). Deterministic — fixed ticks, fixed order (ramps are append-only,
--- never reordered).
function Modifiers.applyRamps(s)
	local ramps = s.ramps
	if #ramps == 0 then
		return
	end
	for i = 1, #ramps do
		local r = ramps[i]
		while s.tick >= r.next_apply do
			Modifiers.applyEffect(s.modifiers, r.effect, s.economy, s.tank)
			r.next_apply += r.interval_ticks
		end
	end
end

-- ============================ live queries ============================

--[[
	Live GLOBAL additive damage bonus from the three DYNAMIC scalers, evaluated
	against the current `tank`/`economy` (NOT baked at purchase) — the offensive
	analogue of `selfScalingAdd`. Folded into the per-weapon multiplier at fire
	time exactly like `selfScalingAdd` (additive, then `×mul_global`):
	  * Mastercrafted Masonry: `rate × (max_hp / 2000)`.
	  * Golden Ring: `rate × ((bounty_mult − 1) / 0.5)`  ( = rate × 2 × bonus ).
	  * Arcane Mark: `shield_active_dmg` while `mana_shield > 0`, else nothing.
	Integer/Fixed only (feeds the checksum). A `bounty_mult` below the 1.0 base
	(never happens in normal play) is clamped so the term can't go negative.
]]
function Modifiers.dynamicGlobalAdd(m, tank, economy)
	local acc = ZERO
	if not isZero(m.dmg_per_maxhp_rate) and tank.max_hp > 0 then
		-- units = max_hp / 2000 (Fixed); bonus = rate × units.
		local units = fromRatio(tank.max_hp, 2000)
		acc = add(acc, mul(m.dmg_per_maxhp_rate, units))
	end
	if not isZero(m.dmg_per_bounty_rate) then
		-- bounty bonus above the 1.0 base, in units of 50% (×2 of the fraction).
		local bonus = sub(economy.bounty_mult, ONE)
		if Fixed.lt(ZERO, bonus) then
			local units = mul(bonus, fromInt(2))
			acc = add(acc, mul(m.dmg_per_bounty_rate, units))
		end
	end
	if not isZero(m.shield_active_dmg) and tank.mana_shield > 0 then
		acc = add(acc, m.shield_active_dmg)
	end
	return acc
end

--- Full damage multiplier for a specific weapon: global + its damage type +
--- each scope it matches (attack class, range bucket, rarity), times the
--- multiplicative product. Consulted at fire time (`docs/05 §5.3`).
---
--- The four additions run in the Rust's order: with saturating `Fixed` adds and
--- a multiplicative tail, reordering is observable.
function Modifiers.weaponDamageMult(m, w)
	local acc = add(m.add_global, m.add_by_type[w.damage_type % 5 + 1])
	acc = add(acc, m.add_by_scope[Content.attackScopeId(w.attack) + 1])
	acc = add(acc, m.add_by_scope[Content.rangeScopeId(w.range) + 1])
	acc = add(acc, m.add_by_scope[Content.rarityScopeId(w.rarity) + 1])
	return mul(add(ONE, acc), m.mul_global)
end

--[[
	GLOBAL additive damage % earned by covering multiple damage types / attack
	classes / distinct weapons — the breadth incentive (`docs/11 §11.2`). A pure
	function of the owned arsenal: no state, no RNG, resolved live at fire time
	exactly like `selfScalingAdd`.

	    synergy = 0.10 × min(distinct_damage_types  − 1, 4)
	            + 0.08 × min(distinct_attack_classes − 1, 5)
	            + 0.05 × min(distinct_weapon_defs    − 1, 8)

	An empty arsenal scores `ZERO` (each `distinct − 1` term is clamped at 0).
	Deliberately a bonus and never a penalty: stacking copies is worth exactly what
	it always was; breadth is simply worth more.

	TRANSCRIPTION NOTE: the Rust builds three bitsets and takes `count_ones()` at
	the end. This counts DISTINCT-ON-INSERT instead — identical by construction
	(the popcount of a union IS the number of distinct bits inserted) and it skips a
	256-bit popcount on a per-fire path. Iteration order is irrelevant either way,
	which is what makes the value stable.
]]
function Modifiers.arsenalSynergyAdd(weapons)
	local n = #weapons
	if n == 0 then
		return ZERO
	end
	for i = 1, DEF_BITSET_WORDS32 do
		defWords[i] = 0
	end
	local typeMask, classMask = 0, 0
	local types, classes, defs = 0, 0, 0
	local WEAPONS = Content.WEAPONS
	for i = 1, n do
		local def = weapons[i].def
		local wd = WEAPONS[def + 1] -- Rust: WEAPONS[w.def as usize]
		local tb = bit32.lshift(1, wd.damage_type % 5)
		if bit32.band(typeMask, tb) == 0 then
			typeMask = bit32.bor(typeMask, tb)
			types += 1
		end
		local cb = bit32.lshift(1, Content.attackScopeId(wd.attack) % 6)
		if bit32.band(classMask, cb) == 0 then
			classMask = bit32.bor(classMask, cb)
			classes += 1
		end
		if def < DEF_BITSET_BITS then
			local w = def // 32 + 1
			local db = bit32.lshift(1, def % 32)
			local word = defWords[w]
			if bit32.band(word, db) == 0 then
				defWords[w] = bit32.bor(word, db)
				defs += 1
			end
		end
	end
	-- The three terms are summed in the Rust's order: `Fixed` adds saturate, so
	-- reordering is observable.
	local acc = add(ZERO, SYNERGY_TYPE_ADD[synergyExtra(types, SYNERGY_TYPE_CAP) + 1])
	acc = add(acc, SYNERGY_CLASS_ADD[synergyExtra(classes, SYNERGY_CLASS_CAP) + 1])
	acc = add(acc, SYNERGY_DEF_ADD[synergyExtra(defs, SYNERGY_DEF_CAP) + 1])
	return acc
end

--- Additive self-scaling % for a weapon of `dmgType`, given the owned weapons:
--- `Σ rule.per × (count of rule.weapon_def)` over rules matching the type, PLUS
--- the arsenal-breadth synergy (global, type-independent — see
--- `Modifiers.arsenalSynergyAdd`). Resolved live at fire time (it depends on the
--- current arsenal).
---
--- Both terms are ADDITIVE and land in the caller's single additive pool before
--- the `× mul_global` step, so the composition order is exactly as before:
--- `(1 + add_static + add_self + add_dyn) × mul_global`. The synergy rides here
--- (rather than in `weaponDamageMult`) because this is the only fire-time hook
--- that is handed the owned arsenal. It is the FIRST term of the accumulator, as
--- in the Rust.
function Modifiers.selfScalingAdd(m, dmgType: number, weapons)
	local acc = Modifiers.arsenalSynergyAdd(weapons)
	local rules = m.weapon_count_scaling
	for i = 1, #rules do
		local rule = rules[i]
		if rule.dmg_type == dmgType then
			local count = 0
			for j = 1, #weapons do
				if weapons[j].def == rule.weapon_def then
					count += 1
				end
			end
			if count > 0 then
				acc = add(acc, mul(rule.per, fromInt(count)))
			end
		end
	end
	return acc
end

--- Scale a weapon's on-hit status by the player's flavor modifiers
--- (Poison-damage % and Stun-duration %). Depends only on the modifiers, so
--- it bakes into the hit at fire time (like base damage). Frost/Fire stacks
--- are unaffected here.
---
--- The Rust takes `StatusOnHit` by value (it is `Copy`); the Luau input is a
--- shared table owned by `Content`, so a COPY is returned and the content table
--- is never mutated.
function Modifiers.scaleOnHit(m, onHit)
	local out = {
		poison_dps = onHit.poison_dps,
		poison_ticks = onHit.poison_ticks,
		frost_stacks = onHit.frost_stacks,
		fire_stacks = onHit.fire_stacks,
		stun_ticks = onHit.stun_ticks,
	}
	if out.poison_dps > 0 and not Fixed.eq(m.poison_dmg_mult, ONE) then
		out.poison_dps = scaleToInt(m.poison_dmg_mult, out.poison_dps)
	end
	if out.stun_ticks > 0 and not Fixed.eq(m.stun_dur_mult, ONE) then
		-- SOURCE FIDELITY: the Rust is `scale_i64(..) as u32`, a WRAPPING cast.
		-- It is unreachable in practice (`stun_dur_mult` starts at ONE and every
		-- catalog rider is positive, so the product is never negative), and a
		-- silent wrap here would be worse than a visible number, so the plain
		-- integer is kept.
		out.stun_ticks = scaleToInt(m.stun_dur_mult, out.stun_ticks)
	end
	return out
end

--- Total damage multiplier for a hit of `damageType`: `(1 + Σ matching
--- additive%) × Π multiplicative factors`.
function Modifiers.damageMult(m, damageType: number)
	local additive = add(add(ONE, m.add_global), m.add_by_type[damageType % 5 + 1])
	return mul(additive, m.mul_global)
end

--- Attack-speed multiplier (`1 + Σ attack-speed%`). Effective cooldown is the
--- base cooldown divided by this.
function Modifiers.attackSpeedMult(m)
	return add(ONE, m.attack_speed)
end

-- ============================ effect application ============================

-- One handler per `ModEffect` variant, keyed by its `words()` tag
-- (`content.rs::ModEffect::words`). The operand names match the Rust variant's
-- field names so each body reads as the Rust match arm it ports.
local EFFECTS: { [number]: (any, number, number, number, any, any) -> () } = {}

-- 0: DamageGlobalPct(n, d) — +% additive to ALL weapon damage.
EFFECTS[0] = function(m, n, d, _c, _economy, _tank)
	m.add_global = add(m.add_global, fromRatio(n, d))
end

-- 1: DamageTypePct(t, n, d) — +% additive to one damage type's damage.
EFFECTS[1] = function(m, t, n, d, _economy, _tank)
	local i = t % 5 + 1
	m.add_by_type[i] = add(m.add_by_type[i], fromRatio(n, d))
end

-- 2: DamageMulPct(n, d) — a multiplicative damage factor: ×(1 + n/d).
-- THIS is the "multiplicative across sources" half of the stacking rule: each
-- purchase compounds into `mul_global` rather than summing with the additives.
--
-- SOFT-CAPPED multiplicative source. The added factor (the `+0.25` of a `×1.25`
-- source) is attenuated by `(DAMAGE_MUL_SOFT / mul_global)^3` once the banked
-- product passes ×8; below that it is untouched. Continuous at the knee, monotone,
-- and pure integer (see `softCappedFactor`).
EFFECTS[2] = function(m, n, d, _c, _economy, _tank)
	local f = softCappedFactor(fromRatio(n, d), m.mul_global, DAMAGE_MUL_SOFT)
	m.mul_global = mul(m.mul_global, add(ONE, f))
end

-- 3: AttackSpeedPct(n, d)
EFFECTS[3] = function(m, n, d, _c, _economy, _tank)
	m.attack_speed = add(m.attack_speed, fromRatio(n, d))
end

-- 4: BountyPct(n, d)
EFFECTS[4] = function(_m, n, d, _c, economy, _tank)
	economy.bounty_mult = add(economy.bounty_mult, fromRatio(n, d))
end

-- 5: IncomeFlat(f)
EFFECTS[5] = function(_m, f, _b, _c, economy, _tank)
	economy.income_per_tick += f
end

-- 6: MaxHp(f) — +flat max HP (and current HP).
-- Flat Max HP goes through the SAME soft cap as the % rider, so the knee is a
-- property of the STAT, not of one effect. Below it a `+2000 Max HP` is exactly
-- +2000; above it the round-ramping `+500/round` sources stop compounding into the
-- millions. Current HP tracks the APPLIED increment, not the raw one.
EFFECTS[6] = function(_m, f, _b, _c, _economy, tank)
	local inc = addSoftCapped(tank, "max_hp", f, MAX_HP_SOFT)
	tank.hp += inc
	grantPoolRegen(tank, inc)
end

-- 7: Armor(a)
EFFECTS[7] = function(_m, a, _b, _c, _economy, tank)
	addSoftCapped(tank, "armor", a, ARMOR_SOFT)
end

-- 8: ManaShield(pool, regen)
EFFECTS[8] = function(_m, pool, regen, _c, _economy, tank)
	local inc = addSoftCapped(tank, "mana_shield_max", pool, MANA_SHIELD_SOFT)
	tank.mana_shield += inc
	addSoftCapped(tank, "mana_regen_per_tick", regen, MANA_REGEN_SOFT)
	grantPoolRecharge(tank, inc)
end

-- 9: HpRegen(r)
EFFECTS[9] = function(_m, r, _b, _c, _economy, tank)
	addSoftCapped(tank, "hp_regen_per_tick", r, HP_REGEN_SOFT)
end

--[[
	10: Dodge(n)
	Additive, but HARD-CAPPED at 70% of `dodge_den` (integer math:
	`dodge_den * 7 / 10`). This is a deliberate nerf: at the old
	99% ceiling dodge was an invincibility button — once a build hit
	it, NO enemy lever (HP / contact / volume / boss) could pressure
	it and ~88% of runs coasted to the 60-min cap purely on dodge.
	Capping the effective avoid-rate at 70% leaves dodge strong but
	mortal: ~30% of hits still land, so the HP staircase and the boss
	can finally threaten a snowball, and mitigation (armor / shield /
	regen / spikes) becomes a real competing axis instead of a
	strictly-worse alternative. Integer/Fixed only — feeds the
	checksum, so no floats.
]]
EFFECTS[10] = function(_m, n, _b, _c, _economy, tank)
	local cap = math.min(tank.dodge_den * 7, U32_MAX) // 10
	tank.dodge_num = math.min(tank.dodge_num + n, cap)
end

-- 11: DamageScopePct(sid, n, d)
EFFECTS[11] = function(m, sid, n, d, _economy, _tank)
	local i = sid % Content.NUM_SCOPES + 1
	m.add_by_scope[i] = add(m.add_by_scope[i], fromRatio(n, d))
end

-- 12: SpikesFlat(n)
EFFECTS[12] = function(_m, n, _b, _c, _economy, tank)
	tank.spikes_damage += n
end

-- 13: SpikesPct(n, d)
EFFECTS[13] = function(_m, n, d, _c, _economy, tank)
	tank.spikes_mult = add(tank.spikes_mult, fromRatio(n, d))
end

-- 14: GrantVulnPulse(..) — registered as per-arena trigger state in
-- `buy_modifier`; no aggregate contribution here.
EFFECTS[14] = function() end

-- 15: HealOnKill(n)
EFFECTS[15] = function(_m, n, _b, _c, _economy, tank)
	tank.heal_on_kill += n
end

-- 16: HealOnPoison(n)
EFFECTS[16] = function(_m, n, _b, _c, _economy, tank)
	tank.heal_on_poison += n
end

-- 17: IncomePct(n, d)
EFFECTS[17] = function(_m, n, d, _c, economy, _tank)
	economy.income_mult = add(economy.income_mult, fromRatio(n, d))
end

-- 18: IncomeRegenPct(n, d)
-- SOFT-CAPPED, and this is the load-bearing one (`docs/11 §11.8`).
-- `income_regen_pct` was the ONLY sustain stat in the game with no bound of any
-- kind on its stack, and that ONE fact decided 97% of losses. The knee is placed at
-- ONE copy, so a build that treats this as one sustain option among several is
-- bit-for-bit unaffected; past it the ratio grows ~n^¼ instead of ~n. Note this
-- bounds the RATIO, where `Economy.incomeRegenTickCap` bounds the per-tick RESULT:
-- independent, and both needed.
EFFECTS[18] = function(_m, n, d, _c, economy, _tank)
	local f = softCappedFactor(fromRatio(n, d), economy.income_regen_pct, INCOME_REGEN_SOFT)
	economy.income_regen_pct = add(economy.income_regen_pct, f)
end

-- 19: BountyProc(c, b) — chance is a plain percent; the bonus is a FRACTION of
-- the base bounty, so `b` is a percentage over 100.
EFFECTS[19] = function(_m, c, b, _c3, economy, _tank)
	economy.bounty_proc_chance_pct += c
	economy.bounty_proc_bonus = add(economy.bounty_proc_bonus, fromRatio(b, 100))
end

-- 20: DamageVsStunnedPct(n, d)
EFFECTS[20] = function(m, n, d, _c, _economy, _tank)
	m.vs_stunned = add(m.vs_stunned, fromRatio(n, d))
end

-- 21: DamageVsPoisonedPct(n, d)
EFFECTS[21] = function(m, n, d, _c, _economy, _tank)
	m.vs_poisoned = add(m.vs_poisoned, fromRatio(n, d))
end

-- 22: PoisonDamagePct(n, d) — note this ADDS into a multiplier that starts at
-- ONE (so a single +50% yields ×1.5), matching the Rust.
EFFECTS[22] = function(m, n, d, _c, _economy, _tank)
	m.poison_dmg_mult = add(m.poison_dmg_mult, fromRatio(n, d))
end

-- 23: StunDurationPct(n, d) — same shape as PoisonDamagePct.
EFFECTS[23] = function(m, n, d, _c, _economy, _tank)
	m.stun_dur_mult = add(m.stun_dur_mult, fromRatio(n, d))
end

-- 24/25/26: GrantDuplicator / GrantVoucher / GrantGold — META purchase-flow
-- effects, intercepted in `buy_modifier`; no aggregate contribution here.
EFFECTS[24] = function() end
EFFECTS[25] = function() end
EFFECTS[26] = function() end

-- 27: HealingPct(n, d)
EFFECTS[27] = function(_m, n, d, _c, _economy, tank)
	tank.healing_mult = add(tank.healing_mult, fromRatio(n, d))
end

-- 28: MissingHpHealPct(n, d)
EFFECTS[28] = function(_m, n, d, _c, _economy, tank)
	tank.missing_hp_heal_pct = add(tank.missing_hp_heal_pct, fromRatio(n, d))
end

-- 29: GrantRevive(bonus)
EFFECTS[29] = function(_m, bonus, _b, _c, _economy, tank)
	tank.revives += 1
	tank.revive_bonus_hp = math.max(tank.revive_bonus_hp, bonus)
end

-- 30: DamagePerWeapon(def, ty, num) — append a self-scaling rule; `per` is a
-- percentage over 100.
EFFECTS[30] = function(m, def, ty, num, _economy, _tank)
	table.insert(m.weapon_count_scaling, {
		weapon_def = def,
		dmg_type = ty,
		per = fromRatio(num, 100),
	})
end

-- 31/32: TradeMaxHpForGold / TradeRegenForGold — the HP/regen→gold trades need
-- the full ArenaState (gold scoreboard) and are intercepted in `buy_modifier`.
EFFECTS[31] = function() end
EFFECTS[32] = function() end

-- 33: GoldPerDamagePct(n, d)
EFFECTS[33] = function(_m, n, d, _c, economy, _tank)
	economy.gold_per_damage = add(economy.gold_per_damage, fromRatio(n, d))
end

-- 34: IncomeShieldPct(n, d)
EFFECTS[34] = function(_m, n, d, _c, economy, _tank)
	economy.income_shield_pct = add(economy.income_shield_pct, fromRatio(n, d))
end

--[[
	35/36/37 — Percentage riders evaluated against the CURRENT stat at apply-time
	(so in a bundle they compound on the flat effect listed before them).
	Integer/Fixed only — these feed the checksum. The compounding is
	CLAMPED to `STAT_CEIL`: repeatedly buying a `[flat, pct]` bundle
	multiplies the stat by ~1.25 each time, which would otherwise overflow
	i64 (and poison every downstream `hp + x`) in a pathological purchase
	stack. `STAT_CEIL` (1e12) sits far above any reachable real build yet
	leaves ample headroom for downstream adds, so the clamp is bit-stable
	across platforms and never trips in normal play. Same spirit as the
	Dodge hard-cap. The inc itself is also derived from the clamped stat,
	so it stays bounded.

	RETROACTIVITY: the increment is taken from the stat as it stands when this
	effect runs — NOT from a base snapshot — which is exactly why effect order
	inside a bundle is load-bearing and why `Modifiers.apply` walks
	`def.effects` in slice order.

	Each of the three is additionally SOFT-CAPPED (see `softCappedInc`): below the
	knee the rider applies at full strength (so no ordinary build changes at all),
	above it the increment decays as `(SOFT/stat)^3` and the stat grows ~n^⅓ instead
	of exponentially. `STAT_CEIL` stays as the hard i64 backstop but is now
	unreachable in practice.

	ORDER: the raw increment is computed from the CURRENT stat FIRST, and only then
	fed through the cap — `raw = (n/d) × stat`, then `applied = cap(raw)`. And it is
	the APPLIED amount, never the raw one, that `tank.hp` follows.
]]
-- 35: MaxHpPct(n, d)
EFFECTS[35] = function(_m, n, d, _c, _economy, tank)
	local raw = scaleToInt(fromRatio(n, d), tank.max_hp)
	local inc = addSoftCapped(tank, "max_hp", raw, MAX_HP_SOFT)
	tank.hp += inc
	grantPoolRegen(tank, inc)
end

-- 36: HpRegenPct(n, d)
EFFECTS[36] = function(_m, n, d, _c, _economy, tank)
	local raw = scaleToInt(fromRatio(n, d), tank.hp_regen_per_tick)
	addSoftCapped(tank, "hp_regen_per_tick", raw, HP_REGEN_SOFT)
end

-- 37: ManaRegenPct(n, d)
EFFECTS[37] = function(_m, n, d, _c, _economy, tank)
	local raw = scaleToInt(fromRatio(n, d), tank.mana_regen_per_tick)
	addSoftCapped(tank, "mana_regen_per_tick", raw, MANA_REGEN_SOFT)
end

-- 38: ShieldActiveDrPct(n, d)
EFFECTS[38] = function(_m, n, d, _c, _economy, tank)
	tank.shield_active_dr = add(tank.shield_active_dr, fromRatio(n, d))
end

-- 39: HealOnDamaged(n)
EFFECTS[39] = function(_m, n, _b, _c, _economy, tank)
	tank.heal_on_damaged += n
end

-- 40/41/42 — DYNAMIC global-damage scalers (resolved live at fire time via
-- `dynamicGlobalAdd`, mirroring `DamagePerWeapon`/`selfScalingAdd`).
-- Accumulate only the per-unit RATE / flat bonus here.
EFFECTS[40] = function(m, n, d, _c, _economy, _tank)
	m.dmg_per_maxhp_rate = add(m.dmg_per_maxhp_rate, fromRatio(n, d))
end
EFFECTS[41] = function(m, n, d, _c, _economy, _tank)
	m.dmg_per_bounty_rate = add(m.dmg_per_bounty_rate, fromRatio(n, d))
end
EFFECTS[42] = function(m, n, d, _c, _economy, _tank)
	m.shield_active_dmg = add(m.shield_active_dmg, fromRatio(n, d))
end

-- 43: ManaOnKill(n)
EFFECTS[43] = function(_m, n, _b, _c, _economy, tank)
	tank.mana_on_kill += n
end

-- EXPANSION E2 — plain tank-field setters (no aggregate; integer only).
-- 44: ShieldBreakStun(r, t)
-- Shield-break stun (Energy Pulse): arm the pulse range/duration. Keep the
-- STRONGER of any stacked copies so two Energy Pulses don't shrink the window.
EFFECTS[44] = function(_m, r, t, _c, _economy, tank)
	tank.shieldbreak_stun_range = math.max(tank.shieldbreak_stun_range, r)
	tank.shieldbreak_stun_ticks = math.max(tank.shieldbreak_stun_ticks, t)
end

-- 45: SpikesPoison(dps, t)
-- Spikes poison (Poison Armor): keep whichever DoT deals more total
-- remaining damage (mirrors the poison-application rule in `status`).
EFFECTS[45] = function(_m, dps, t, _c, _economy, tank)
	local existing = tank.spikes_poison_dps * tank.spikes_poison_ticks
	local incoming = dps * t
	if incoming >= existing then
		tank.spikes_poison_dps = dps
		tank.spikes_poison_ticks = t
	end
end

-- 46: StackingSpikes(per, max)
-- Stacking spikes (Bloody Spikes): set the per-stack bonus and the cap.
-- Additive per-stack, max cap takes the larger of stacked copies.
EFFECTS[46] = function(_m, per, max, _c, _economy, tank)
	tank.spikes_stack_per += per
	tank.spikes_stacks_max = math.max(tank.spikes_stacks_max, max)
end

-- 47: DamageAura(r, c, d)
-- Damage/poison aura (Blight Aura): arm the emitter. Keep the wider range,
-- shorter (more frequent) cadence, and larger damage of any stacked
-- copies; the poison rider mirrors Poison Armor's DoT (2/tick × 90).
EFFECTS[47] = function(_m, r, c, d, _economy, tank)
	tank.aura_range = math.max(tank.aura_range, r)
	tank.aura_damage += d
	if tank.aura_cadence == 0 then
		tank.aura_cadence = c
	else
		tank.aura_cadence = math.min(tank.aura_cadence, c)
	end
	tank.aura_poison_dps = math.max(tank.aura_poison_dps, 2)
	tank.aura_poison_ticks = math.max(tank.aura_poison_ticks, 90)
end

--- Apply a single `ModEffect` (used both for a modifier's base effect and
--- for each tick of a time-scaling ramp). Damage/attack-speed effects
--- accumulate in this aggregate; economy/defensive effects apply immediately.
function Modifiers.applyEffect(m, effect, economy, tank)
	local handler = EFFECTS[effect.tag]
	if handler == nil then
		error("Modifiers.applyEffect: unknown ModEffect tag " .. tostring(effect.tag), 2)
	end
	handler(m, effect.a, effect.b, effect.c, economy, tank)
end

--- Fold a purchased modifier in (applies each of its base `effects`, in order).
--- Order is load-bearing: the percentage riders (35/36/37) read the CURRENT
--- stat, so a `[flat, pct]` bundle compounds.
function Modifiers.apply(m, def, economy, tank)
	local effects = def.effects
	for i = 1, #effects do
		Modifiers.applyEffect(m, effects[i], economy, tank)
	end
end

return Modifiers
