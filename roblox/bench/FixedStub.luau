--[[
	FixedStub.luau — *** THIS IS A STUB. IT IS NOT THE REAL `Fixed`. ***

	`roblox/src/shared/Fixed.luau` (CONTRACTS §C2) is owned by the numeric-core
	milestone (R0). This file exists only so the R2 throughput bench can run and
	produce a fixed-point-vs-double cost ratio *before* R0 lands. The bench
	prefers the real module and only falls back here — see `resolveFixed()` in
	`throughput.luau`, which prints loudly which one it used.

	It implements the §C2 API surface with the representation the contract
	implies: a Fixed value IS its raw Q48.16 integer, carried as a Luau double,
	with multiply/divide done through a split-limb path so the 106-bit product
	does not get truncated by the 53-bit mantissa.

	KNOWN DIVERGENCES FROM THE CONTRACT (why this must not be shipped):
	  * Exact only while |raw| < 2^53 and |a*b| < 2^69. The sim's real domain
	    (positions <= 1800<<16 ~ 2^27, multipliers ~2^16..2^24) sits far inside
	    that, but the saturation boundary cases §C2 calls out are NOT handled
	    exactly here — near i64::MAX a double cannot even name the operands.
	  * Saturation clamps to +/-2^63 (the double that i64::MAX rounds to), not to
	    i64::MAX/i64::MIN exactly.
	  * `sqrt` seeds from `math.sqrt` and corrects; the correction loop is
	    bounded, so inputs above ~2^53 may be off by a few ULP.
	  * §C4 records that `from_int` and `from_ratio` TRUNCATE on narrowing while
	    `mul`/`div`/`scale_i64` SATURATE. This stub saturates in all five.
	These are all correctness gaps, not speed gaps: the op *shape* (the count of
	floors, multiplies and divides on the hot path) is what the bench measures,
	and that is representative of any split-limb implementation.
]]

local FRAC_BITS = 16
local ONE_RAW = 65536 -- 2^16
local SPLIT = 16777216 -- 2^24, the limb radix
local SPLIT_HI = 4294967296 -- 2^32 == 2^48 >> 16
local SPLIT_MID = 256 -- 2^8  == 2^24 >> 16
local SAT_MAX = 9223372036854775808 -- 2^63 (i64::MAX rounded to a double)
local SAT_MIN = -9223372036854775808

local floor = math.floor

local Fixed = {}

Fixed.FRAC_BITS = FRAC_BITS
Fixed.ONE = ONE_RAW
Fixed.ZERO = 0

local function saturate(v: number): number
	if v > SAT_MAX then
		return SAT_MAX
	elseif v < SAT_MIN then
		return SAT_MIN
	end
	return v
end

-- floor(|a| * |b| / 2^16) plus the remainder-nonzero flag, for non-negative a, b.
-- a*b = ah*bh*2^48 + (ah*bl + al*bh)*2^24 + al*bl, and every term above the
-- low limb is divisible by 2^16, so the shift is exact term-by-term.
local function umulShift(a: number, b: number): (number, boolean)
	local ah = floor(a / SPLIT)
	local al = a - ah * SPLIT
	local bh = floor(b / SPLIT)
	local bl = b - bh * SPLIT

	local lo = al * bl
	local loQ = floor(lo / ONE_RAW)
	local rem = lo - loQ * ONE_RAW

	return ah * bh * SPLIT_HI + (ah * bl + al * bh) * SPLIT_MID + loQ, rem ~= 0
end

-- Rust's `(a as i128 * b as i128) >> 16` is an ARITHMETIC shift: it floors
-- toward negative infinity. floor(-x/m) = -(floor(x/m) + (x%m ~= 0 and 1 or 0)).
local function mulShift(a: number, b: number): number
	local neg = false
	if a < 0 then
		a = -a
		neg = not neg
	end
	if b < 0 then
		b = -b
		neg = not neg
	end
	local q, hasRem = umulShift(a, b)
	if neg then
		return -(if hasRem then q + 1 else q)
	end
	return q
end

function Fixed.fromInt(i: number): number
	return saturate(i * ONE_RAW)
end

function Fixed.fromRaw(raw: number): number
	return raw
end

function Fixed.raw(x: number): number
	return x
end

-- Rust: `(((num as i128) << 16) / den as i128) as i64` — integer division in
-- Rust TRUNCATES TOWARD ZERO (the doc comment on the Rust fn says otherwise;
-- the code is what ships, so this matches the code).
function Fixed.fromRatio(num: number, den: number): number
	local v = (num * ONE_RAW) / den
	return saturate(if v >= 0 then floor(v) else -floor(-v))
end

-- Arithmetic shift right: floors toward negative infinity, NOT toward zero.
function Fixed.floorToInt(x: number): number
	return floor(x / ONE_RAW)
end

function Fixed.add(a: number, b: number): number
	return saturate(a + b)
end

function Fixed.sub(a: number, b: number): number
	return saturate(a - b)
end

function Fixed.neg(a: number): number
	return saturate(-a)
end

function Fixed.mul(a: number, b: number): number
	return saturate(mulShift(a, b))
end

function Fixed.div(a: number, b: number): number
	local v = (a * ONE_RAW) / b
	return saturate(if v >= 0 then floor(v) else -floor(-v))
end

-- (v * raw) >> 16, saturating. Same shift semantics as `mul`.
function Fixed.scaleI64(x: number, v: number): number
	return saturate(mulShift(x, v))
end

-- floor(isqrt(raw << 16)); errors on negative, matching the Rust assert.
function Fixed.sqrt(x: number): number
	if x < 0 then
		error("Fixed.sqrt of negative", 2)
	end
	if x < 2 then
		return x
	end
	local n = x * ONE_RAW
	local r = floor(math.sqrt(n))
	-- Correct the double seed. Bounded so a pathological input cannot spin.
	for _ = 1, 4 do
		if r * r > n then
			r -= 1
		elseif (r + 1) * (r + 1) <= n then
			r += 1
		else
			break
		end
	end
	return r
end

-- --- Convenience surface the bench uses that §C2 does not name, but the real
-- --- module also provides. Mirrored here so the bench runs against either.
function Fixed.toInt(v: number): number
	return v
end

function Fixed.lo(v: number): number
	return v % 4294967296
end

function Fixed.hi(v: number): number
	return floor(v / 4294967296) % 4294967296
end

function Fixed.compare(a: number, b: number): number
	if a < b then
		return -1
	elseif a > b then
		return 1
	end
	return 0
end

function Fixed.lt(a: number, b: number): boolean
	return a < b
end

function Fixed.le(a: number, b: number): boolean
	return a <= b
end

Fixed.__IS_BENCH_STUB = true

return Fixed
