--[[
	Rng.luau — exact Luau emulation of `determinism::Rng`
	(`sim-core/crates/determinism/src/lib.rs`). Contract: `roblox/CONTRACTS.md` §C3.

	SplitMix64. The state is a u64, which a Luau double cannot hold, so it is
	carried as two unsigned 32-bit halves (`hi`, `lo`) and every 64-bit operation
	is done on 16-bit sub-limbs, where each partial product (< 2^32) and its
	accumulation (< 2^35) is exactly representable as a double.

	`u64`-valued results (`nextU64`, `state`) are returned as TWO numbers,
	`hi, lo` — high half first. That keeps them exact without allocating a
	wrapper per draw, which matters because these are the hottest calls in the
	sim. `Rng.toHex(hi, lo)` / `Rng.toDecimal(hi, lo)` render them.

	`below` reproduces the Rust rejection loop word-for-word, including HOW MANY
	u32 draws it consumes: an "equivalent" unbiased method that consumes a
	different number of words silently desynchronizes every downstream stream.

	This module is self-contained on purpose — it requires nothing, so it runs
	identically under Rojo/Roblox and under a bare `luau` interpreter, and it
	references no Roblox global.
]]

local Rng = {}

local B = 65536
local B32 = 4294967296
local SAFE = 9007199254740992 -- 2^53

-- SplitMix64 constants, split into 32-bit halves.
local GOLDEN_HI, GOLDEN_LO = 0x9E3779B9, 0x7F4A7C15
local MIX1_HI, MIX1_LO = 0xBF58476D, 0x1CE4E5B9
local MIX2_HI, MIX2_LO = 0x94D049BB, 0x133111EB

local mt = {}
mt.__index = mt

-- ============================ u64 primitives ============================

--- (ahi,alo) + (bhi,blo), wrapping at 2^64.
local function wadd(ahi, alo, bhi, blo)
	local lo = alo + blo
	local carry = 0
	if lo >= B32 then
		lo = lo - B32
		carry = 1
	end
	local hi = ahi + bhi + carry
	if hi >= B32 then
		hi = hi - B32
	end
	return hi, lo
end

--- Low 64 bits of (ahi,alo) * (bhi,blo). Schoolbook over 16-bit limbs; the
--- limbs at index >= 4 are exactly the bits that wrap away, so they are simply
--- never computed.
local function wmul(ahi, alo, bhi, blo)
	local a0 = alo % B
	local a1 = (alo - a0) / B
	local a2 = ahi % B
	local a3 = (ahi - a2) / B
	local b0 = blo % B
	local b1 = (blo - b0) / B
	local b2 = bhi % B
	local b3 = (bhi - b2) / B

	local c0 = a0 * b0
	local c1 = a0 * b1 + a1 * b0
	local c2 = a0 * b2 + a1 * b1 + a2 * b0
	local c3 = a0 * b3 + a1 * b2 + a2 * b1 + a3 * b0

	local r0 = c0 % B
	local carry = (c0 - r0) / B
	local t = c1 + carry
	local r1 = t % B
	carry = (t - r1) / B
	t = c2 + carry
	local r2 = t % B
	carry = (t - r2) / B
	t = c3 + carry
	local r3 = t % B

	return r3 * B + r2, r1 * B + r0
end

--- Logical right shift of a u64 by `s`, with 0 < s < 32.
local function shr(hi, lo, s)
	return bit32.rshift(hi, s), bit32.bor(bit32.rshift(lo, s), bit32.lshift(hi, 32 - s))
end

local function bxor64(ahi, alo, bhi, blo)
	return bit32.bxor(ahi, bhi), bit32.bxor(alo, blo)
end

--- Exact 32x32 -> 64 unsigned product, returned as (hi, lo).
local function mul32(x, y)
	local x0 = x % B
	local x1 = (x - x0) / B
	local y0 = y % B
	local y1 = (y - y0) / B
	local c0 = x0 * y0
	local c1 = x0 * y1 + x1 * y0
	local c2 = x1 * y1
	local r0 = c0 % B
	local carry = (c0 - r0) / B
	local t = c1 + carry
	local r1 = t % B
	carry = (t - r1) / B
	return c2 + carry, r1 * B + r0
end

-- ============================ seed coercion ============================

local function parseU64String(s)
	local str = s
	local hex = string.lower(string.sub(str, 1, 2)) == "0x"
	local hi, lo = 0, 0
	if hex then
		local body = string.sub(str, 3)
		if #body == 0 or #body > 16 then
			return nil
		end
		for i = 1, #body do
			local c = string.byte(body, i)
			local d
			if c >= 48 and c <= 57 then
				d = c - 48
			elseif c >= 97 and c <= 102 then
				d = c - 87
			elseif c >= 65 and c <= 70 then
				d = c - 55
			else
				return nil
			end
			-- shift the 64-bit accumulator left by 4 and or-in the digit
			local carry = bit32.rshift(bit32.band(lo, 0xF0000000), 28)
			lo = bit32.bor(bit32.lshift(lo, 4), d)
			hi = bit32.bor(bit32.lshift(hi, 4), carry)
		end
		return hi % B32, lo % B32
	end
	-- decimal, exact, via 16-bit limbs
	local t1, t2, t3, t4 = 0, 0, 0, 0
	for i = 1, #str do
		local d = string.byte(str, i) - 48
		if d < 0 or d > 9 then
			return nil
		end
		local cur = t1 * 10 + d
		t1 = cur % B
		local carry = (cur - t1) / B
		cur = t2 * 10 + carry
		t2 = cur % B
		carry = (cur - t2) / B
		cur = t3 * 10 + carry
		t3 = cur % B
		carry = (cur - t3) / B
		cur = t4 * 10 + carry
		t4 = cur % B
		carry = (cur - t4) / B
		if carry ~= 0 then
			return nil -- >= 2^64
		end
	end
	return t4 * B + t3, t2 * B + t1
end

--- Coerce a seed (number | decimal string | "0x..." string) to (hi, lo).
--- Errors rather than rounding: a plain number seed must be an exact
--- non-negative integer below 2^53.
function Rng.seedParts(seed)
	local ts = type(seed)
	if ts == "number" then
		if seed % 1 ~= 0 or seed ~= seed then
			error("Rng: non-integral seed " .. tostring(seed), 2)
		end
		if seed < 0 then
			error("Rng: negative seed; pass the u64 as a decimal or 0x string", 2)
		end
		if seed >= SAFE then
			error("Rng: seed >= 2^53 is not exact as a Luau number; pass a decimal or 0x string", 2)
		end
		local lo = seed % B32
		return (seed - lo) / B32, lo
	elseif ts == "string" then
		local hi, lo = parseU64String(seed)
		if hi == nil then
			error("Rng: bad u64 seed string '" .. seed .. "'", 2)
		end
		return hi, lo
	elseif ts == "table" and seed.hi ~= nil and seed.lo ~= nil then
		return seed.hi, seed.lo
	end
	error("Rng: cannot coerce " .. ts .. " to a u64 seed", 2)
end

--- Parse a BARE (or `0x`-prefixed) hex u64 into (hi, lo). This is the wire form
--- used by the parity vectors; it never passes through a Luau number.
function Rng.hexParts(s)
	if type(s) ~= "string" then
		error("Rng.hexParts: expected a string", 2)
	end
	local body = s
	if string.lower(string.sub(body, 1, 2)) == "0x" then
		body = string.sub(body, 3)
	end
	if #body == 0 or #body > 16 then
		error("Rng.hexParts: expected 1..16 hex digits, got '" .. s .. "'", 2)
	end
	local hi, lo = parseU64String("0x" .. body)
	if hi == nil then
		error("Rng.hexParts: bad hex '" .. s .. "'", 2)
	end
	return hi, lo
end

-- ============================ rendering ============================

function Rng.toHex(hi, lo)
	return string.format("0x%08x%08x", hi, lo)
end

function Rng.toDecimal(hi, lo)
	local t = { lo % B, (lo - lo % B) / B, hi % B, (hi - hi % B) / B }
	local digits = {}
	repeat
		local rem = 0
		for i = 4, 1, -1 do
			local cur = rem * B + t[i]
			local q = (cur - cur % 10) / 10
			rem = cur % 10
			t[i] = q
		end
		digits[#digits + 1] = string.char(48 + rem)
	until t[1] == 0 and t[2] == 0 and t[3] == 0 and t[4] == 0
	local n = #digits
	for i = 1, (n - n % 2) / 2 do
		digits[i], digits[n + 1 - i] = digits[n + 1 - i], digits[i]
	end
	return table.concat(digits)
end

-- ============================ Rng ============================

--- `Rng::from_seed`.
function Rng.fromSeed(seed)
	local hi, lo = Rng.seedParts(seed)
	return setmetatable({ hi = hi, lo = lo }, mt)
end

--- Construct directly from two u32 halves.
function Rng.fromSeedParts(hi, lo)
	return setmetatable({ hi = hi, lo = lo }, mt)
end

--- `Rng::state` — returns the u64 as (hi, lo).
function Rng.state(r)
	return r.hi, r.lo
end

function Rng.clone(r)
	return setmetatable({ hi = r.hi, lo = r.lo }, mt)
end

--- `Rng::next_u64` — advances the state. Returns the u64 as (hi, lo).
function Rng.nextU64(r)
	local hi, lo = wadd(r.hi, r.lo, GOLDEN_HI, GOLDEN_LO)
	r.hi = hi
	r.lo = lo
	local zh, zl = hi, lo
	local sh, sl = shr(zh, zl, 30)
	zh, zl = bxor64(zh, zl, sh, sl)
	zh, zl = wmul(zh, zl, MIX1_HI, MIX1_LO)
	sh, sl = shr(zh, zl, 27)
	zh, zl = bxor64(zh, zl, sh, sl)
	zh, zl = wmul(zh, zl, MIX2_HI, MIX2_LO)
	sh, sl = shr(zh, zl, 31)
	return bit32.bxor(zh, sh), bit32.bxor(zl, sl)
end

--- `Rng::next_u32` — the HIGH half of the next u64. Exact as a Luau number.
function Rng.nextU32(r)
	local hi = Rng.nextU64(r)
	return hi
end

--- `Rng::derive` — `stream = splitmix(hash(master, player, purpose, round))`.
function Rng.derive(master, player, purpose, round)
	local shi, slo = Rng.seedParts(master)
	local coords = { player, purpose, round }
	local tmp = setmetatable({ hi = 0, lo = 0 }, mt)
	for i = 1, 3 do
		local v = coords[i]
		if v == nil or v % 1 ~= 0 or v < 0 or v >= B32 then
			error("Rng.derive: player/purpose/round must be u32 integers", 2)
		end
		-- s ^= (v as u64).wrapping_mul(GOLDEN)
		local mh, ml = wmul(0, v, GOLDEN_HI, GOLDEN_LO)
		shi, slo = bxor64(shi, slo, mh, ml)
		tmp.hi = shi
		tmp.lo = slo
		shi, slo = Rng.nextU64(tmp)
	end
	return setmetatable({ hi = shi, lo = slo }, mt)
end

--- `Rng::below` — unbiased integer in [0, n) (Lemire). `below(0) == 0`.
--- The draw count matches the Rust exactly, rejection loop included.
function Rng.below(r, n)
	if n == 0 then
		return 0
	end
	local x = Rng.nextU32(r)
	local mhi, mlo = mul32(x, n)
	local l = mlo
	if l < n then
		local t = (B32 - n) % n -- n.wrapping_neg() % n
		while l < t do
			x = Rng.nextU32(r)
			mhi, mlo = mul32(x, n)
			l = mlo
		end
	end
	return mhi
end

--- `Rng::chance` — true with probability num/den.
function Rng.chance(r, num, den)
	return Rng.below(r, den) < num
end

mt.__tostring = function(r)
	return "Rng(" .. Rng.toHex(r.hi, r.lo) .. ")"
end

return Rng
