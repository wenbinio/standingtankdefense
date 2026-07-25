--[[
	Fixed.luau — exact Luau emulation of `determinism::Fixed`
	(`sim-core/crates/determinism/src/lib.rs`). Contract: `roblox/CONTRACTS.md` §C2.

	WHY THIS FILE IS SHAPED LIKE THIS
	---------------------------------
	The Rust type is `Fixed(i64)`, Q48.16, with `i128` intermediates and a
	*saturating* narrowing back to i64. Luau numbers are IEEE doubles: integers are
	exact only up to 2^53. A raw Q48.16 value spans the full i64 range, and the
	intermediate product of two raws spans 128 bits, so a plain `a * b` silently
	drops low bits and corrupts the simulation invisibly.

	Therefore every value handled here is carried as FOUR 16-BIT LIMBS,
	little-endian, two's complement:

	    v = l[1] + l[2]*2^16 + l[3]*2^32 + l[4]*2^48      (mod 2^64, signed)

	16 bits is the widest limb for which a full limb*limb product (< 2^32) plus
	accumulation (< 2^35) stays exactly representable in a double, so every
	partial product in the 64x64 -> 128 multiply is exact. Nothing in this file
	ever forms a value larger than 2^36 in a Luau number.

	VALUE TYPE
	----------
	Fixed values (and bare i64 magnitudes, which share the representation) are
	OPAQUE. Callers must never index them; go through this module's functions.
	That is what keeps the documented escape hatch open: if throughput ever
	demands it, the internals can be swapped to plain doubles without touching a
	single call site.

	Constructors accept `number` (integral, |v| <= 2^53), an exact decimal
	`string` (full i64 range, e.g. "-9223372036854775808"), a `"0x..."` string
	(read as a 64-bit *bit pattern*, so "0xFFFFFFFFFFFFFFFF" == -1), or an
	existing value of this type.

	ENGINE INDEPENDENCE
	-------------------
	No Roblox global is referenced (`game`, `workspace`, `task`, `Instance`,
	`os`, `tick`). This module runs unchanged under a bare `luau` interpreter,
	which is how it is tested. Only `math`, `string`, `table`, `bit32` are used.
]]

local Fixed = {}

-- ============================ limb primitives ============================

local B = 65536 -- limb base, 2^16
local BM = 65535 -- limb mask
local HALF_TOP = 32768 -- 2^15: sign bit of the top limb
local SAFE = 9007199254740992 -- 2^53, the exact-integer ceiling for doubles

local mt = {}
mt.__index = mt

local function newValue(a, b, c, d)
	return setmetatable({ a, b, c, d }, mt)
end

local function isValue(v)
	return type(v) == "table" and getmetatable(v) == mt
end

-- Two's-complement negation of a 4-limb value (wrapping, like Rust's `-x` on
-- i64::MIN, which yields i64::MIN again).
local function negLimbs(v)
	local carry = 1
	local r1 = (BM - v[1]) + carry
	carry = 0
	if r1 >= B then
		r1 = r1 - B
		carry = 1
	end
	local r2 = (BM - v[2]) + carry
	carry = 0
	if r2 >= B then
		r2 = r2 - B
		carry = 1
	end
	local r3 = (BM - v[3]) + carry
	carry = 0
	if r3 >= B then
		r3 = r3 - B
		carry = 1
	end
	local r4 = (BM - v[4]) + carry
	if r4 >= B then
		r4 = r4 - B
	end
	return newValue(r1, r2, r3, r4)
end

local function isNeg(v)
	return v[4] >= HALF_TOP
end

local ZERO = newValue(0, 0, 0, 0)
local ONE = newValue(0, 1, 0, 0) -- 1 << 16 == 65536 -> limb[2] == 1
local I64_MAX = newValue(BM, BM, BM, 32767)
local I64_MIN = newValue(0, 0, 0, 32768)

-- ============================ decimal <-> limbs ============================

-- Exact decimal rendering of the 4-limb magnitude `t` (unsigned).
local function magToDecimal(l1, l2, l3, l4)
	local digits = {}
	local t1, t2, t3, t4 = l1, l2, l3, l4
	repeat
		local cur = t4
		t4 = (cur - cur % 10) / 10
		local rem = cur % 10
		cur = rem * B + t3
		t3 = (cur - cur % 10) / 10
		rem = cur % 10
		cur = rem * B + t2
		t2 = (cur - cur % 10) / 10
		rem = cur % 10
		cur = rem * B + t1
		t1 = (cur - cur % 10) / 10
		rem = cur % 10
		digits[#digits + 1] = string.char(48 + rem)
	until t1 == 0 and t2 == 0 and t3 == 0 and t4 == 0
	-- digits were produced least-significant first
	local n = #digits
	for i = 1, (n - n % 2) / 2 do
		digits[i], digits[n + 1 - i] = digits[n + 1 - i], digits[i]
	end
	return table.concat(digits)
end

--- Exact decimal string for a value (handles i64::MIN).
function Fixed.toDecimal(v)
	if not isValue(v) then
		v = Fixed.int64(v)
	end
	if isNeg(v) then
		local m = negLimbs(v)
		return "-" .. magToDecimal(m[1], m[2], m[3], m[4])
	end
	return magToDecimal(v[1], v[2], v[3], v[4])
end

mt.__tostring = function(v)
	return Fixed.toDecimal(v)
end

mt.__eq = function(a, b)
	return a[1] == b[1] and a[2] == b[2] and a[3] == b[3] and a[4] == b[4]
end

-- Parse an unsigned decimal digit string into limbs. Returns nil on overflow
-- past 2^64.
local function parseDecimalMag(s)
	local t1, t2, t3, t4 = 0, 0, 0, 0
	for i = 1, #s do
		local d = string.byte(s, i) - 48
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
			return nil
		end
	end
	return t1, t2, t3, t4
end

local function parseHex(s)
	local body = string.sub(s, 3)
	if #body == 0 or #body > 16 then
		return nil
	end
	local t = { 0, 0, 0, 0 }
	-- read right-to-left, 4 hex digits per limb
	local limb = 1
	local shift = 1
	for i = #body, 1, -1 do
		local c = string.byte(s, i + 2)
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
		t[limb] = t[limb] + d * shift
		shift = shift * 16
		if shift == B then
			shift = 1
			limb = limb + 1
		end
	end
	return t[1], t[2], t[3], t[4]
end

-- ============================ construction ============================

--- Coerce `v` (value | number | decimal/hex string) to the internal value type.
--- Errors rather than silently rounding: a `number` argument must be an exact
--- integer with |v| <= 2^53. Pass a decimal string for anything larger.
function Fixed.int64(v)
	if isValue(v) then
		return v
	end
	local tv = type(v)
	if tv == "number" then
		if v ~= v or v == math.huge or v == -math.huge then
			error("Fixed: not a finite number", 2)
		end
		if v % 1 ~= 0 then
			error("Fixed: non-integral number " .. tostring(v), 2)
		end
		if v > SAFE or v < -SAFE then
			error(
				"Fixed: |value| > 2^53 is not exactly representable as a Luau number; "
					.. "pass an exact decimal string instead (got " .. tostring(v) .. ")",
				2
			)
		end
		local neg = v < 0
		local m = neg and -v or v
		local l1 = m % B
		m = (m - l1) / B
		local l2 = m % B
		m = (m - l2) / B
		local l3 = m % B
		m = (m - l3) / B
		local l4 = m % B
		local out = newValue(l1, l2, l3, l4)
		if neg then
			return negLimbs(out)
		end
		return out
	elseif tv == "string" then
		local s = v
		local sign = 1
		if string.sub(s, 1, 1) == "-" then
			sign = -1
			s = string.sub(s, 2)
		elseif string.sub(s, 1, 1) == "+" then
			s = string.sub(s, 2)
		end
		local a, b, c, d
		local pfx = string.lower(string.sub(s, 1, 2))
		if pfx == "0x" then
			a, b, c, d = parseHex(string.lower(s))
			if a == nil then
				error("Fixed: bad hex integer '" .. v .. "'", 2)
			end
			local out = newValue(a, b, c, d)
			if sign < 0 then
				return negLimbs(out)
			end
			return out
		end
		if #s == 0 then
			error("Fixed: empty integer string", 2)
		end
		a, b, c, d = parseDecimalMag(s)
		if a == nil then
			error("Fixed: integer string out of i64 range: '" .. v .. "'", 2)
		end
		-- range check against the signed bounds
		if sign > 0 then
			if d > 32767 then
				error("Fixed: integer string out of i64 range: '" .. v .. "'", 2)
			end
			return newValue(a, b, c, d)
		else
			if d > 32768 or (d == 32768 and (a ~= 0 or b ~= 0 or c ~= 0)) then
				error("Fixed: integer string out of i64 range: '" .. v .. "'", 2)
			end
			return negLimbs(newValue(a, b, c, d))
		end
	end
	error("Fixed: cannot coerce " .. tv .. " to an i64", 2)
end

local int64 = Fixed.int64

--- Parse a BARE (or `0x`-prefixed) hex string as a 64-bit two's-complement bit
--- pattern: "ffffffffffffffff" -> -1, "8000000000000000" -> i64::MIN. This is
--- the wire form used by the parity vectors, and it is decoded straight into
--- limbs — never through a Luau number.
function Fixed.fromHex(s)
	if type(s) ~= "string" then
		error("Fixed.fromHex: expected a string", 2)
	end
	local body = s
	local pfx = string.lower(string.sub(body, 1, 2))
	if pfx == "0x" then
		body = string.sub(body, 3)
	end
	if #body == 0 or #body > 16 then
		error("Fixed.fromHex: expected 1..16 hex digits, got '" .. s .. "'", 2)
	end
	local a, b, c, d = parseHex("0x" .. string.lower(body))
	if a == nil then
		error("Fixed.fromHex: bad hex '" .. s .. "'", 2)
	end
	return newValue(a, b, c, d)
end

--- The 16-digit lowercase two's-complement hex form (the parity-vector wire
--- form). Exact for every i64.
function Fixed.toHex(v)
	if not isValue(v) then
		v = int64(v)
	end
	return string.format("%04x%04x%04x%04x", v[4], v[3], v[2], v[1])
end

Fixed.FRAC_BITS = 16
Fixed.ONE = ONE
Fixed.ZERO = ZERO
Fixed.I64_MAX = I64_MAX
Fixed.I64_MIN = I64_MIN

--- `Fixed::from_raw` — reinterpret an i64 as a Fixed.
function Fixed.fromRaw(raw)
	return int64(raw)
end

--- `Fixed::raw` — the underlying i64.
--- Returns a Luau number, and ERRORS if the value cannot be represented exactly
--- (|raw| > 2^53). Use `Fixed.toDecimal` / `Fixed.hi`+`Fixed.lo` for those.
function Fixed.raw(x)
	return Fixed.toInt(x)
end

--- Value -> Luau number. Errors instead of losing bits.
function Fixed.toInt(v)
	if not isValue(v) then
		v = int64(v)
	end
	local neg = isNeg(v)
	local m = neg and negLimbs(v) or v
	-- The range check must happen on the LIMBS: composing first and testing the
	-- result would already have rounded (2^53+1 composes to 2^53 and would slip
	-- through). |value| <= 2^53 iff the top limb is < 32, or exactly 32 with the
	-- rest zero.
	local top = m[4]
	if top > 32 or (top == 32 and (m[1] ~= 0 or m[2] ~= 0 or m[3] ~= 0)) then
		error("Fixed.toInt: |value| > 2^53, not exactly representable (" .. Fixed.toDecimal(v) .. ")", 2)
	end
	local n = ((top * B + m[3]) * B + m[2]) * B + m[1]
	return neg and -n or n
end

--- The high / low unsigned 32-bit halves (for checksums and serialization).
function Fixed.hi(v)
	if not isValue(v) then
		v = int64(v)
	end
	return v[4] * B + v[3]
end

function Fixed.lo(v)
	if not isValue(v) then
		v = int64(v)
	end
	return v[2] * B + v[1]
end

function Fixed.isFixed(v)
	return isValue(v)
end

--- `Fixed::from_int` — `i << 16`. Rust's `<<` DISCARDS the bits shifted out
--- (shift-left is not overflow-checked in Rust, only the shift *amount* is), so
--- this wraps rather than saturates. Emulated exactly.
function Fixed.fromInt(i)
	local v = isValue(i) and i or int64(i)
	-- << 16 == shift limbs up by one, drop the old top limb
	return newValue(0, v[1], v[2], v[3])
end

--- `Fixed::floor_to_int` — arithmetic shift right by 16. Floors toward NEGATIVE
--- INFINITY, not toward zero. Result magnitude is < 2^48, so a plain Luau
--- number is exact here.
function Fixed.floorToInt(x)
	local v = isValue(x) and x or int64(x)
	local top = v[4]
	if top >= HALF_TOP then
		top = top - B
	end
	-- value = ((top*2^16 + l3)*2^16 + l2)*2^16 + l1 ; >> 16 drops l1 entirely.
	return (top * B + v[3]) * B + v[2]
end

-- ============================ add / sub / neg ============================

--- `Add for Fixed` — saturating.
function Fixed.add(a, b)
	local x = isValue(a) and a or int64(a)
	local y = isValue(b) and b or int64(b)
	local s1 = x[1] + y[1]
	local c = 0
	if s1 >= B then
		s1 = s1 - B
		c = 1
	end
	local s2 = x[2] + y[2] + c
	c = 0
	if s2 >= B then
		s2 = s2 - B
		c = 1
	end
	local s3 = x[3] + y[3] + c
	c = 0
	if s3 >= B then
		s3 = s3 - B
		c = 1
	end
	local s4 = x[4] + y[4] + c
	if s4 >= B then
		s4 = s4 - B
	end
	-- signed overflow iff the operands share a sign that the result does not
	local sx = x[4] >= HALF_TOP
	local sy = y[4] >= HALF_TOP
	if sx == sy then
		local sr = s4 >= HALF_TOP
		if sr ~= sx then
			return sx and I64_MIN or I64_MAX
		end
	end
	return newValue(s1, s2, s3, s4)
end

--- `Neg for Fixed` — saturating (`-i64::MIN == i64::MAX`).
function Fixed.neg(a)
	local x = isValue(a) and a or int64(a)
	if x[4] == 32768 and x[1] == 0 and x[2] == 0 and x[3] == 0 then
		return I64_MAX -- saturating_neg of i64::MIN
	end
	return negLimbs(x)
end

--- `Sub for Fixed` — saturating.
function Fixed.sub(a, b)
	local x = isValue(a) and a or int64(a)
	local y = isValue(b) and b or int64(b)
	-- two's-complement subtract, then detect signed overflow directly (doing
	-- add(x, neg(y)) would mis-saturate for y == i64::MIN).
	local d1 = x[1] - y[1]
	local br = 0
	if d1 < 0 then
		d1 = d1 + B
		br = 1
	end
	local d2 = x[2] - y[2] - br
	br = 0
	if d2 < 0 then
		d2 = d2 + B
		br = 1
	end
	local d3 = x[3] - y[3] - br
	br = 0
	if d3 < 0 then
		d3 = d3 + B
		br = 1
	end
	local d4 = x[4] - y[4] - br
	if d4 < 0 then
		d4 = d4 + B
	end
	local sx = x[4] >= HALF_TOP
	local sy = y[4] >= HALF_TOP
	if sx ~= sy then
		local sr = d4 >= HALF_TOP
		if sr ~= sx then
			return sx and I64_MIN or I64_MAX
		end
	end
	return newValue(d1, d2, d3, d4)
end

-- ============================ 128-bit core ============================

-- Magnitude of a value, as four limbs, plus its sign. |i64::MIN| == 2^63 still
-- fits in the four unsigned limbs, which is why magnitudes are used throughout.
local function magOf(v)
	if v[4] >= HALF_TOP then
		local m = negLimbs(v)
		return m, true
	end
	return v, false
end

-- Full 64x64 -> 128 unsigned product, as eight 16-bit limbs (little-endian).
-- Every partial product a[i]*b[j] is < 2^32 and at most four of them plus a
-- carry (< 2^19) land in one accumulator, so the largest Luau number formed is
-- < 2^35 — exact.
local function mulMag(a, b)
	local p = { 0, 0, 0, 0, 0, 0, 0, 0 }
	for i = 1, 4 do
		local ai = a[i]
		if ai ~= 0 then
			local k = i
			p[k] = p[k] + ai * b[1]
			p[k + 1] = p[k + 1] + ai * b[2]
			p[k + 2] = p[k + 2] + ai * b[3]
			p[k + 3] = p[k + 3] + ai * b[4]
		end
	end
	local carry = 0
	for k = 1, 8 do
		local t = p[k] + carry
		local r = t % B
		p[k] = r
		carry = (t - r) / B
	end
	return p
end

-- Saturating narrow of a magnitude+sign to an i64 value. `n` is the number of
-- significant limbs in `q`. Mirrors `sat_i128_to_i64`.
local function satFromMag(q, n, neg)
	local over = false
	for i = 5, n do
		if q[i] ~= 0 then
			over = true
			break
		end
	end
	if not over then
		local top = q[4]
		if neg then
			if top > HALF_TOP or (top == HALF_TOP and (q[1] ~= 0 or q[2] ~= 0 or q[3] ~= 0)) then
				over = true
			end
		else
			if top >= HALF_TOP then
				over = true
			end
		end
	end
	if over then
		return neg and I64_MIN or I64_MAX
	end
	local out = newValue(q[1], q[2], q[3], q[4])
	if neg then
		return negLimbs(out)
	end
	return out
end

-- `(a * b) >> 16` with an exact 128-bit intermediate and an arithmetic (floor)
-- shift, then saturating narrowing. Shared by `mul` and `scaleI64` because the
-- Rust bodies are the same expression.
--
-- This is THE hot path of the whole simulation, so it is fully unrolled over
-- scalar locals: no intermediate tables are built, and the only allocation is
-- the single result value. Every quantity below is < 2^36, hence exact.
local function mulShiftSat(x, y)
	-- magnitudes, in scalars (two's-complement negate, inlined)
	local a1, a2, a3, a4 = x[1], x[2], x[3], x[4]
	local neg = false
	if a4 >= HALF_TOP then
		neg = true
		local c = 1
		a1 = BM - a1 + c
		if a1 >= B then
			a1 -= B
		else
			c = 0
		end
		a2 = BM - a2 + c
		if a2 >= B then
			a2 -= B
		else
			c = 0
		end
		a3 = BM - a3 + c
		if a3 >= B then
			a3 -= B
		else
			c = 0
		end
		a4 = BM - a4 + c
		if a4 >= B then
			a4 -= B
		end
	end
	local b1, b2, b3, b4 = y[1], y[2], y[3], y[4]
	if b4 >= HALF_TOP then
		neg = not neg
		local c = 1
		b1 = BM - b1 + c
		if b1 >= B then
			b1 -= B
		else
			c = 0
		end
		b2 = BM - b2 + c
		if b2 >= B then
			b2 -= B
		else
			c = 0
		end
		b3 = BM - b3 + c
		if b3 >= B then
			b3 -= B
		else
			c = 0
		end
		b4 = BM - b4 + c
		if b4 >= B then
			b4 -= B
		end
	end

	-- 64x64 -> 128 schoolbook; each column sums at most four products < 2^32
	local t = a1 * b1
	local p1 = t % B
	local carry = (t - p1) / B

	t = a1 * b2 + a2 * b1 + carry
	local q1 = t % B
	carry = (t - q1) / B

	t = a1 * b3 + a2 * b2 + a3 * b1 + carry
	local q2 = t % B
	carry = (t - q2) / B

	t = a1 * b4 + a2 * b3 + a3 * b2 + a4 * b1 + carry
	local q3 = t % B
	carry = (t - q3) / B

	t = a2 * b4 + a3 * b3 + a4 * b2 + carry
	local q4 = t % B
	carry = (t - q4) / B

	t = a3 * b4 + a4 * b3 + carry
	local q5 = t % B
	carry = (t - q5) / B

	t = a4 * b4 + carry
	local q6 = t % B
	local q7 = (t - q6) / B
	-- q1..q7 are already the product >> 16 (dropping limb p1 IS the shift)

	if neg and p1 ~= 0 then
		-- floor(-m / 2^16) == -ceil(m / 2^16): the dropped remainder rounds the
		-- magnitude UP before the sign is reapplied.
		q1 += 1
		if q1 >= B then
			q1 = 0
			q2 += 1
			if q2 >= B then
				q2 = 0
				q3 += 1
				if q3 >= B then
					q3 = 0
					q4 += 1
					if q4 >= B then
						q4 = 0
						q5 += 1
						if q5 >= B then
							q5 = 0
							q6 += 1
							if q6 >= B then
								q6 = 0
								q7 += 1
							end
						end
					end
				end
			end
		end
	end

	-- saturating narrow (mirrors `sat_i128_to_i64`)
	if q5 ~= 0 or q6 ~= 0 or q7 ~= 0 then
		return neg and I64_MIN or I64_MAX
	end
	if neg then
		if q4 > HALF_TOP or (q4 == HALF_TOP and (q1 ~= 0 or q2 ~= 0 or q3 ~= 0)) then
			return I64_MIN
		end
		-- negate in place; a zero magnitude negates to zero, so an unsigned zero
		-- product is unaffected by a stale sign
		local c = 1
		q1 = BM - q1 + c
		if q1 >= B then
			q1 -= B
		else
			c = 0
		end
		q2 = BM - q2 + c
		if q2 >= B then
			q2 -= B
		else
			c = 0
		end
		q3 = BM - q3 + c
		if q3 >= B then
			q3 -= B
		else
			c = 0
		end
		q4 = BM - q4 + c
		if q4 >= B then
			q4 -= B
		end
		return newValue(q1, q2, q3, q4)
	end
	if q4 >= HALF_TOP then
		return I64_MAX
	end
	return newValue(q1, q2, q3, q4)
end

--- `Fixed::mul` — `(a*b) >> 16` via an exact i128 intermediate, then saturate.
function Fixed.mul(a, b)
	local x = isValue(a) and a or int64(a)
	local y = isValue(b) and b or int64(b)
	return mulShiftSat(x, y)
end

--- `Fixed::scale_i64` — `(v * self) >> 16`, saturating. Returns an i64 VALUE
--- (not a Luau number) so the saturation boundary stays exact; convert with
--- `Fixed.toInt` when the magnitude is known to be small.
function Fixed.scaleI64(x, v)
	local a = isValue(x) and x or int64(x)
	local b = isValue(v) and v or int64(v)
	return mulShiftSat(b, a)
end

-- --------------------------- division ---------------------------

local function cmp5(a, b)
	for i = 5, 1, -1 do
		local ai, bi = a[i], b[i]
		if ai ~= bi then
			return ai < bi and -1 or 1
		end
	end
	return 0
end

local function sub5(a, b)
	local br = 0
	for i = 1, 5 do
		local t = a[i] - b[i] - br
		if t < 0 then
			a[i] = t + B
			br = 1
		else
			a[i] = t
			br = 0
		end
	end
end

-- Unsigned magnitude division: floor(num / den), where `num` is `nlen` limbs
-- (nlen <= 5, i.e. <= 80 bits) and `den` is 4 limbs (padded to 5). Plain binary
-- restoring division: the running remainder is always < den <= 2^64, so
-- `r*2 + bit < 2^65` fits in the five limbs and every step is exact.
local function divMag(num, nlen, den)
	local q = {}
	for i = 1, nlen do
		q[i] = 0
	end
	local r = { 0, 0, 0, 0, 0 }
	for i = nlen, 1, -1 do
		local limb = num[i]
		local qi = 0
		for bpos = 15, 0, -1 do
			local carry = bit32.extract(limb, bpos, 1)
			for k = 1, 5 do
				local t = r[k] * 2 + carry
				if t >= B then
					r[k] = t - B
					carry = 1
				else
					r[k] = t
					carry = 0
				end
			end
			if cmp5(r, den) >= 0 then
				sub5(r, den)
				qi = qi + bit32.lshift(1, bpos)
			end
		end
		q[i] = qi
	end
	return q
end

local ZERO_DIV = "Fixed: attempt to divide by zero"

-- Shared body of `div` and `from_ratio`: floor(|a| << 16 / |b|) with the sign
-- applied afterwards (Rust integer division truncates toward zero, so the
-- magnitude quotient is exactly right).
local function divShifted(x, y)
	if y[1] == 0 and y[2] == 0 and y[3] == 0 and y[4] == 0 then
		error(ZERO_DIV, 3)
	end
	local ma, na = magOf(x)
	local mb, nb = magOf(y)

	-- FAST PATH, and still exact. When |a| < 2^36 the shifted numerator is
	-- < 2^52, and when |b| < 2^52 the denominator is too; both are then exact
	-- doubles. IEEE division is correctly rounded, so math.floor lands on the
	-- true quotient or one either side of it; the remainder correction below
	-- pins it down, and every product/difference it forms stays <= 2^53, hence
	-- exact. (This covers every magnitude the simulation actually uses; the
	-- general path underneath is what makes the i64 extremes right.)
	if false then
		local num = ((ma[3] * B + ma[2]) * B + ma[1]) * B
		local den = ((mb[4] * B + mb[3]) * B + mb[2]) * B + mb[1]
		local q = math.floor(num / den)
		local r = num - q * den
		if r < 0 then
			q -= 1
			r += den
		end
		if r >= den then
			q += 1
		end
		local l1 = q % B
		q = (q - l1) / B
		local l2 = q % B
		q = (q - l2) / B
		local l3 = q % B
		q = (q - l3) / B
		return { l1, l2, l3, q, 0 }, na ~= nb
	end

	local num = { 0, ma[1], ma[2], ma[3], ma[4] } -- << 16
	local den = { mb[1], mb[2], mb[3], mb[4], 0 }
	local q = divMag(num, 5, den)
	return q, na ~= nb
end

--- `Fixed::div` — `(a << 16) / b` via an exact i128 intermediate, then saturate.
function Fixed.div(a, b)
	local x = isValue(a) and a or int64(a)
	local y = isValue(b) and b or int64(b)
	local q, neg = divShifted(x, y)
	return satFromMag(q, 5, neg)
end

--- `Fixed::from_ratio` — `((num << 16) / den) as i64`. NOTE: the Rust `as i64`
--- here is a WRAPPING cast, not the saturating one used by mul/div; emulated as
--- such (only reachable for absurd ratios).
function Fixed.fromRatio(num, den)
	local x = isValue(num) and num or int64(num)
	local y = isValue(den) and den or int64(den)
	local q, neg = divShifted(x, y)
	local out = newValue(q[1], q[2], q[3], q[4]) -- truncate to 64 bits
	if neg then
		return negLimbs(out)
	end
	return out
end

-- ============================ sqrt ============================

-- floor(sqrt(n)) where n is a magnitude of up to 80 bits, given as 5 limbs.
-- A double gives a starting guess good to well under 1 ulp of the answer
-- (the result is < 2^40); the exact 128-bit comparison then pins it down.
local function isqrtMag(n)
	local approx = (((n[5] * B + n[4]) * B + n[3]) * B + n[2]) * B + n[1]
	local r = math.floor(math.sqrt(approx))
	if r < 0 then
		r = 0
	end
	local function sqrCmp(v)
		-- compare v*v against n
		local l1 = v % B
		local m = (v - l1) / B
		local l2 = m % B
		m = (m - l2) / B
		local l3 = m % B
		m = (m - l3) / B
		local l4 = m % B
		local p = mulMag({ l1, l2, l3, l4 }, { l1, l2, l3, l4 })
		for i = 8, 6, -1 do
			if p[i] ~= 0 then
				return 1
			end
		end
		for i = 5, 1, -1 do
			if p[i] ~= n[i] then
				return p[i] < n[i] and -1 or 1
			end
		end
		return 0
	end
	local guard = 0
	while r > 0 and sqrCmp(r) > 0 do
		r = r - 1
		guard = guard + 1
		if guard > 64 then
			error("Fixed.sqrt: isqrt failed to converge")
		end
	end
	while sqrCmp(r + 1) <= 0 do
		r = r + 1
		guard = guard + 1
		if guard > 64 then
			error("Fixed.sqrt: isqrt failed to converge")
		end
	end
	return r
end

--- `Fixed::sqrt` — floor integer sqrt of `raw << 16`, in raw units. Errors on
--- a negative input (the Rust asserts).
function Fixed.sqrt(x)
	local v = isValue(x) and x or int64(x)
	if isNeg(v) then
		error("Fixed::sqrt of negative", 2)
	end
	local n = { 0, v[1], v[2], v[3], v[4] } -- (raw as u128) << 16
	local r = isqrtMag(n)
	-- r < 2^40, so the plain-number path is exact
	local l1 = r % B
	local m = (r - l1) / B
	local l2 = m % B
	m = (m - l2) / B
	local l3 = m % B
	m = (m - l3) / B
	return newValue(l1, l2, l3, m % B)
end

-- ============================ comparison ============================

--- Signed comparison: -1 / 0 / 1.
function Fixed.compare(a, b)
	local x = isValue(a) and a or int64(a)
	local y = isValue(b) and b or int64(b)
	local sx = x[4] >= HALF_TOP
	local sy = y[4] >= HALF_TOP
	if sx ~= sy then
		return sx and -1 or 1
	end
	for i = 4, 1, -1 do
		if x[i] ~= y[i] then
			return x[i] < y[i] and -1 or 1
		end
	end
	return 0
end

function Fixed.eq(a, b)
	return Fixed.compare(a, b) == 0
end

function Fixed.lt(a, b)
	return Fixed.compare(a, b) < 0
end

function Fixed.le(a, b)
	return Fixed.compare(a, b) <= 0
end

mt.__lt = function(a, b)
	return Fixed.compare(a, b) < 0
end
mt.__le = function(a, b)
	return Fixed.compare(a, b) <= 0
end

return Fixed
