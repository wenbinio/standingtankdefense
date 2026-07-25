--[[
	differential.luau — the coverage the parity vectors CANNOT give us.

	`parity.luau` proves `Fixed.luau` matches the Rust on every case the Rust
	emitted. It does not, and cannot, probe the guard boundaries of the fast
	paths inside `Fixed.luau` — those are an implementation detail the Rust
	knows nothing about. Widening the `div` fast-path guard from |a| < 2^36 to
	|a| < 2^48 still passes every parity vector, and is still wrong.

	This test closes that gap by differential comparison against `FixedRef.luau`
	— a deliberately slow, obviously-correct reference with NO fast paths
	(schoolbook multiply, bit-by-bit restoring division). Both must agree
	bit-for-bit on operands drawn across every width and sign, with extra
	density around each guard boundary.

	Keep both. Parity anchors us to the Rust; this anchors the optimizations to
	the reference. Neither alone is sufficient.

	RUN
	    cd roblox/test
	    luau -O2 differential.luau              # default scale, ~1.8M comparisons
	    luau -O2 differential.luau -a 0.1       # 10% scale, for a quick check

	Exits non-zero on any divergence.
]]

local New = require("../src/shared/Fixed") -- fast paths + Knuth Algorithm D
local Ref = require("./FixedRef") -- no fast paths, binary restoring division
local Rng = require("../src/shared/Rng")

local scale = tonumber(({ ... })[1]) or 1.0
local function n_of(base: number): number
	return math.max(1, math.floor(base * scale))
end

local total, bad = 0, 0
local function fail(kind: string, ...)
	bad += 1
	if bad <= 4 then
		print("DIVERGENCE", kind, ...)
	end
end

-- Draw an operand of exactly `bits` significant bits, random sign, as a
-- matched (New, Ref) pair built from the same hex bit pattern.
local function operand(r, bits: number)
	local hi, lo = Rng.nextU64(r)
	local hx = string.format("%08x%08x", hi, lo)
	local keep = math.ceil(bits / 4)
	local m = string.rep("0", 16 - keep) .. string.sub(hx, 17 - keep)
	local a, b = New.fromHex(m), Ref.fromHex(m)
	if Rng.below(r, 2) == 1 then
		return New.neg(a), Ref.neg(b), m
	end
	return a, b, m
end

local function check(kind: string, x, y, ctx1: string?, ctx2: string?)
	total += 1
	if New.toHex(x) ~= Ref.toHex(y) then
		fail(kind, ctx1 or "", ctx2 or "", New.toHex(x), Ref.toHex(y))
	end
end

-- 1. Full-width mul / div / fromRatio / scaleI64 across every operand width.
--    `abits <= 36 and bbits <= 52` is the div fast-path guard; drawing widths
--    uniformly over 1..64 straddles it in both directions.
do
	local r = Rng.fromSeed(20250725)
	local fastHits = 0
	for _ = 1, n_of(150000) do
		local abits, bbits = 1 + Rng.below(r, 64), 1 + Rng.below(r, 64)
		local na, ra = operand(r, abits)
		local nb, rb = operand(r, bbits)
		if Ref.compare(rb, Ref.ZERO) ~= 0 then
			if abits <= 36 and bbits <= 52 then
				fastHits += 1
			end
			check("DIV", New.div(na, nb), Ref.div(ra, rb))
			check("RATIO", New.fromRatio(na, nb), Ref.fromRatio(ra, rb))
			check("MUL", New.mul(na, nb), Ref.mul(ra, rb))
			check("SCALE", New.scaleI64(na, nb), Ref.scaleI64(ra, rb))
		end
	end
	-- Divisors of 1..4 significant limbs, exercising every Algorithm D shape
	-- (including the normalization and add-back branches).
	for shape = 1, 4 do
		for _ = 1, n_of(20000) do
			local na, ra = operand(r, 1 + Rng.below(r, 64))
			local nb, rb = operand(r, 1 + Rng.below(r, 16 * shape))
			if Ref.compare(rb, Ref.ZERO) ~= 0 then
				check("DIV-SHAPE" .. shape, New.div(na, nb), Ref.div(ra, rb))
			end
		end
	end
	print(string.format("  full-width:  %d fast-path div hits", fastHits))
end

-- 2. The narrow-mul fast path (both raws < 2^31). Widths 1..33 straddle the
--    2^31 guard from both sides; the edge table pins the exact boundaries,
--    zero, and both saturation limits against each other.
do
	local r = Rng.fromSeed(777)
	for _ = 1, n_of(400000) do
		local na, ra = operand(r, 1 + Rng.below(r, 33))
		local nb, rb = operand(r, 1 + Rng.below(r, 33))
		check("MUL-NARROW", New.mul(na, nb), Ref.mul(ra, rb))
		check("SCALE-NARROW", New.scaleI64(na, nb), Ref.scaleI64(ra, rb))
	end
	local edges = {
		"0000000000000000", "0000000000000001", "ffffffffffffffff",
		"000000007fffffff", "0000000080000000", "ffffffff80000000",
		"ffffffff7fffffff", "000000000000ffff", "0000000000010000",
		"7fffffffffffffff", "8000000000000000", "0000000100000000",
	}
	for _, x in ipairs(edges) do
		for _, y in ipairs(edges) do
			local nx, ny = New.fromHex(x), New.fromHex(y)
			local rx, ry = Ref.fromHex(x), Ref.fromHex(y)
			check("EDGE-MUL", New.mul(nx, ny), Ref.mul(rx, ry), x, y)
			check("EDGE-SCALE", New.scaleI64(nx, ny), Ref.scaleI64(rx, ry), x, y)
		end
	end
end

-- 3. sqrt, whose fast path guards on raw < 2^36.
do
	local r = Rng.fromSeed(31337)
	for _ = 1, n_of(300000) do
		local bits = 1 + Rng.below(r, 63)
		local hi, lo = Rng.nextU64(r)
		local hx = string.format("%08x%08x", hi, lo)
		local keep = math.ceil(bits / 4)
		local m = string.rep("0", 16 - keep) .. string.sub(hx, 17 - keep)
		local a, b = New.fromHex(m), Ref.fromHex(m)
		if New.compare(a, New.ZERO) >= 0 then
			check("SQRT", New.sqrt(a), Ref.sqrt(b), m)
		end
	end
	for _, k in
		ipairs({ 0, 1, 2, 3, 4, 8, 9, 15, 16, 17, 65535, 65536, 65537, 4294967295, 4294967296, 68719476735, 68719476736 })
	do
		check("SQRT-EDGE", New.sqrt(New.int64(k)), Ref.sqrt(Ref.int64(k)), tostring(k))
	end
end

print(string.format("differential: %d comparisons, %d differences", total, bad))
if bad > 0 then
	error("DIVERGENCE — Fixed.luau fast paths disagree with FixedRef.luau", 0)
end
print("differential: PASS")
