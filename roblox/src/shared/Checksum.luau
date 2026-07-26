--!nonstrict
--[[
	Checksum.luau — exact Luau port of `determinism::Checksum`
	(`sim-core/crates/determinism/src/lib.rs`). Spec: `roblox/SIM-SPEC.md` §S2/§S6.

	FNV-1a 64-bit over the LITTLE-ENDIAN bytes of every value written. This is the
	whole R3 gate: `Arena.checksum` folds the authoritative state through this
	accumulator and the resulting u64 must equal the Rust's, tick for tick. A
	subtly wrong fold makes every trace comparison meaningless, so this file is a
	transliteration of the Rust, not a reimplementation of FNV.

	The Rust:

	    OFFSET = 0xcbf29ce484222325 ; PRIME = 0x00000100000001b3
	    write_u64(v): for b in v.to_le_bytes() { h ^= b as u64; h = h.wrapping_mul(PRIME) }
	    write_i64(v): write_u64(v as u64)        -- two's complement reinterpretation
	    write_u32(v): write_u64(v as u64)        -- ZERO-extended: four zero bytes still fold
	    write_fixed(v): write_i64(v.raw())
	    finish(): h

	REPRESENTATION
	--------------
	A Luau number is a double: exact only to 2^53, so the u64 accumulator is
	carried as two unsigned 32-bit halves `hi`, `lo`, exactly as `Rng.luau` carries
	its state. The 64x64 multiply is specialised on the constant:

	    PRIME = 0x0000_0100_0000_01b3 = 2^40 + 435

	so `h * PRIME (mod 2^64)` is `h * 435 + (h << 40)`, and `h << 40 (mod 2^64)`
	only survives through the low 24 bits of `lo`. Every intermediate formed here
	stays below 2^41, comfortably exact in a double.

	U64 VALUES
	----------
	`writeU64` takes the value as TWO numbers, `hi, lo` (high half first) — the
	same convention `Rng.nextU64` / `Rng.state` return, so

	    c:writeU64(Rng.state(s.rng_spawn))

	composes directly. `writeI64` / `writeFixed` take either a plain Luau number
	(integral, |v| <= 2^53 — S4: everything the Rust types `i64` sits far inside
	that) or an opaque `Fixed` value, which is read through `Fixed.hi`/`Fixed.lo`
	and therefore covers the full i64 range including `i64::MIN`.

	VERBOSE MODE (`SIM-SPEC` §S6)
	-----------------------------
	`Checksum.newVerbose()` returns an accumulator that records every write —
	label, value, and the running digest AFTER that write — in `c.log`. Feeding
	the same field sequence through it turns "tick 4173 diverges" into "the first
	differing field is `enemies[7].hp`". Every writer takes an optional trailing
	`label`; it is ignored (and costs one nil-check) on a normal accumulator.

	ENGINE INDEPENDENCE
	-------------------
	References no Roblox global. Requires only its sibling `Fixed.luau`, by
	relative path, so it runs unchanged under a bare `luau` interpreter.
]]

local Fixed = require("./Fixed")

local Checksum = {}

local B32 = 4294967296 -- 2^32
local B24 = 16777216 -- 2^24
local SAFE = 9007199254740992 -- 2^53, the exact-integer ceiling for doubles

-- FNV-1a 64. `PRIME == 2^40 + PRIME_LOW`; see the header.
local OFFSET_HI, OFFSET_LO = 0xcbf29ce4, 0x84222325
local PRIME_LOW = 435 -- 0x1b3

Checksum.OFFSET_HI = OFFSET_HI
Checksum.OFFSET_LO = OFFSET_LO

local mt = {}
mt.__index = Checksum

-- ============================ the fold ============================

--- One FNV-1a step: `h ^= b; h *= PRIME` (mod 2^64), with `0 <= b < 256`.
--- Only the low byte of `lo` can change on the xor, so the xor is a plain
--- `bit32.bxor` on the low half.
local function fold(h, l, b)
	l = bit32.bxor(l, b)
	-- h * PRIME == h * 435 + (h << 40).
	local p = l * PRIME_LOW -- < 2^32 * 435 < 2^41: exact
	local nl = p % B32
	local carry = (p - nl) / B32 -- < 435
	local nh = (h * PRIME_LOW + carry) % B32
	-- (h << 40) mod 2^64: `hi` shifts clean off the top, and `lo` contributes its
	-- low 24 bits, landing 8 bits up in the high half.
	nh = (nh + (l % B24) * 256) % B32
	return nh, nl
end

--- Fold the eight little-endian bytes of the u64 `(hi, lo)`.
local function foldU64(h, l, hi, lo)
	h, l = fold(h, l, bit32.band(lo, 255))
	h, l = fold(h, l, bit32.band(bit32.rshift(lo, 8), 255))
	h, l = fold(h, l, bit32.band(bit32.rshift(lo, 16), 255))
	h, l = fold(h, l, bit32.rshift(lo, 24))
	h, l = fold(h, l, bit32.band(hi, 255))
	h, l = fold(h, l, bit32.band(bit32.rshift(hi, 8), 255))
	h, l = fold(h, l, bit32.band(bit32.rshift(hi, 16), 255))
	h, l = fold(h, l, bit32.rshift(hi, 24))
	return h, l
end

-- ============================ conversions ============================

--- Integral Luau number -> two's-complement u64 halves. Errors rather than
--- silently truncating: a checksum that quietly hashes the wrong bits is the one
--- failure mode this whole file exists to prevent.
local function numParts(v, who)
	if type(v) ~= "number" or v % 1 ~= 0 then
		error(who .. ": expected an integral number, got " .. tostring(v), 3)
	end
	if v > SAFE or v < -SAFE then
		error(who .. ": |value| > 2^53, not exactly representable — pass a Fixed value instead", 3)
	end
	if v >= 0 then
		local lo = v % B32
		return (v - lo) / B32, lo
	end
	local m = -v
	local mlo = m % B32
	local mhi = (m - mlo) / B32
	if mlo == 0 then
		return (B32 - mhi) % B32, 0
	end
	return (B32 - mhi - 1) % B32, B32 - mlo
end

--- 16-digit lowercase hex of the u64 `(hi, lo)` — the wire form for traces
--- (`CONTRACTS.md` C4 / `SIM-SPEC` §S6). No `0x` prefix.
function Checksum.toHex(hi, lo)
	return string.format("%08x%08x", hi, lo)
end

--- Inverse of `toHex`: a bare (or `0x`-prefixed) 1..16-digit hex string ->
--- `hi, lo`. Never passes through a Luau number wide enough to lose bits.
function Checksum.fromHex(s)
	if type(s) ~= "string" then
		error("Checksum.fromHex: expected a string", 2)
	end
	local body = s
	if string.lower(string.sub(body, 1, 2)) == "0x" then
		body = string.sub(body, 3)
	end
	if #body == 0 or #body > 16 then
		error("Checksum.fromHex: expected 1..16 hex digits, got '" .. s .. "'", 2)
	end
	body = string.rep("0", 16 - #body) .. string.lower(body)
	local hi = tonumber(string.sub(body, 1, 8), 16)
	local lo = tonumber(string.sub(body, 9, 16), 16)
	if hi == nil or lo == nil then
		error("Checksum.fromHex: bad hex '" .. s .. "'", 2)
	end
	return hi, lo
end

-- ============================ accumulator ============================

--- `Checksum::new` — a fresh accumulator seeded with the FNV offset basis.
function Checksum.new()
	return setmetatable({ hi = OFFSET_HI, lo = OFFSET_LO, log = nil }, mt)
end

--- As `new`, but records every write into `c.log` (see the header). Used by the
--- trace runner's `--verbose` mode; never by the hot path.
function Checksum.newVerbose()
	return setmetatable({ hi = OFFSET_HI, lo = OFFSET_LO, log = {} }, mt)
end

local function record(c, kind, hi, lo, label)
	local log = c.log
	log[#log + 1] = {
		index = #log + 1,
		kind = kind,
		label = label,
		value = Checksum.toHex(hi, lo),
		digest = Checksum.toHex(c.hi, c.lo),
	}
end

--- `Checksum::write_u64` — `hi, lo` are the unsigned 32-bit halves, high first
--- (the `Rng.nextU64` / `Rng.state` convention).
function Checksum.writeU64(c, hi, lo, label)
	c.hi, c.lo = foldU64(c.hi, c.lo, hi, lo)
	if c.log then
		record(c, "u64", hi, lo, label)
	end
end

--- `Checksum::write_i64` — reinterprets the value as a u64 (two's complement).
--- Accepts an integral Luau number (|v| <= 2^53) or an opaque `Fixed` value.
function Checksum.writeI64(c, v, label)
	local hi, lo
	if type(v) == "table" then
		hi, lo = Fixed.hi(v), Fixed.lo(v)
	else
		hi, lo = numParts(v, "Checksum.writeI64")
	end
	c.hi, c.lo = foldU64(c.hi, c.lo, hi, lo)
	if c.log then
		record(c, "i64", hi, lo, label)
	end
end

--- `Checksum::write_u32` — ZERO-extended to u64, so the four high bytes are
--- folded as zeroes. Dropping them would not be FNV of the same input.
function Checksum.writeU32(c, v, label)
	if type(v) ~= "number" or v % 1 ~= 0 or v < 0 or v >= B32 then
		error("Checksum.writeU32: expected an integer in [0, 2^32), got " .. tostring(v), 2)
	end
	c.hi, c.lo = foldU64(c.hi, c.lo, 0, v)
	if c.log then
		record(c, "u32", 0, v, label)
	end
end

--- `Checksum::write_fixed` — `write_i64(v.raw())`. Takes an opaque `Fixed`
--- value (the normal case) or a plain number holding a raw.
function Checksum.writeFixed(c, v, label)
	local hi, lo
	if type(v) == "table" then
		hi, lo = Fixed.hi(v), Fixed.lo(v)
	else
		hi, lo = numParts(v, "Checksum.writeFixed")
	end
	c.hi, c.lo = foldU64(c.hi, c.lo, hi, lo)
	if c.log then
		record(c, "fixed", hi, lo, label)
	end
end

--- `Checksum::finish` — the digest as `hi, lo` (high half first).
function Checksum.finish(c)
	return c.hi, c.lo
end

--- `finish`, rendered as the 16-digit lowercase hex a trace stores.
function Checksum.finishHex(c)
	return Checksum.toHex(c.hi, c.lo)
end

mt.__tostring = function(c)
	return "Checksum(" .. Checksum.toHex(c.hi, c.lo) .. ")"
end

return Checksum
