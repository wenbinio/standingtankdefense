--[[
	parity.luau — the R0 correctness oracle runner (`roblox/CONTRACTS.md` §C4).

	Loads the JSON parity vectors emitted from the Rust and asserts that
	`Fixed.luau` / `Rng.luau` reproduce every expected value bit for bit.
	Prints a pass/fail summary and exits non-zero if anything fails.

	HOW TO RUN
	----------
	The standalone `luau` interpreter has NO file I/O (no `io`, no `os.getenv`;
	`require` resolves `.luau` only), so the vector TEXT is handed to the runner
	on the command line. Every trailing argument after `-a` is concatenated in
	order and then parsed as a sequence of JSON documents — so a file may be
	passed whole, or split into chunks to stay under the OS per-argument limit
	(~128 KiB on Linux). From `roblox/test`:

	    tmp=$(mktemp -d); cat vectors/fixed_vectors.json vectors/rng_vectors.json > "$tmp/all"
	    split -b 100000 "$tmp/all" "$tmp/c."
	    args=(); for f in "$tmp"/c.*; do args+=("$(cat "$f")"); done
	    luau parity.luau -a "${args[@]}"

	(Splitting is safe: no JSON token can span a chunk boundary in a way that the
	loss of inter-chunk whitespace could merge.)

	Two other sources are accepted, for hosts that do have file access:
	  * a global `PARITY_VECTORS` — an array of JSON strings or decoded tables;
	  * `require("./vectors/<name>")` for any `.luau` shim returning either.

	VALUE ENCODING
	--------------
	When a document carries an `encoding` field mentioning "hex", every integer
	STRING in it is a two's-complement hex bit pattern (the authoritative form:
	16 lowercase digits). Those are decoded straight into split limbs via
	`Fixed.fromHex` / `Rng.hexParts` — never through a Luau `number`, which
	could not hold `i64::MAX` or `i64::MIN`. Plain JSON integers (u32 counts,
	`below` outputs, `player`/`purpose`/`round`) stay numbers. Without such an
	`encoding` field, integer strings are read as decimal (and `0x` prefixes are
	always honoured).

	WHAT IT ACCEPTS
	---------------
	The authoritative schema is op-keyed:

	    fixed_vectors.json
	      { "encoding":…, "frac_bits":16, "one":65536,
	        "from_int":[{i,out}], "from_int_wrapping":[{i,out}],
	        "from_ratio":[{num,den,out}], "from_ratio_wrapping":[…],
	        "floor_to_int":[{x,out}], "neg":[{a,out}], "sqrt":[{a,out}],
	        "add"|"sub"|"mul"|"div":[{a,b,out}], "scale_i64":[{x,v,out}] }

	    rng_vectors.json
	      { "seeds": [ { seed,
	                     next_u64:{out:[…], state_after},
	                     next_u32:{out:[…], state_after},
	                     below:  [ {n, out:[…], state_after}, … ],
	                     chance: [ {num, den, out:[…], state_after}, … ] } ],
	        "derive": [ {master, player, purpose, round, state, next_u64:[…]} ] }

	Every list restarts from a FRESH rng at that stream's seed, and
	`state_after` is checked once the list is consumed.

	Because C4 does not pin the key spelling, the reader also accepts the obvious
	variants (`cases`/`ops`/`streams` wrappers, `args:[…]` operands,
	`expect`/`expected`/`result`/`out`, `values` for `out`). It is deliberately
	LOUD: an unrecognized op or case shape is reported as a FAILURE, never
	skipped, so a schema mismatch can never masquerade as a pass.

	Cases may assert a Rust panic with "panics"|"error"|"throws": true (or an
	explicit null expectation). The authoritative vectors do not use this —
	`div` by zero and `sqrt` of a negative panic in Rust, and this port raises a
	Luau error for both — but the hook is here so those can be pinned later.
]]

local Fixed = require("../src/shared/Fixed")
local Rng = require("../src/shared/Rng")
local Json = require("../src/shared/Json")

-- ============================ plumbing ============================

local failures = {}
local passed = 0
local perOp = {}

local function bump(op, ok)
	local e = perOp[op]
	if not e then
		e = { pass = 0, fail = 0 }
		perOp[op] = e
	end
	if ok then
		e.pass += 1
		passed += 1
	else
		e.fail += 1
	end
end

local function fail(op, what, detail)
	bump(op, false)
	failures[#failures + 1] = string.format("[%s] %s\n        %s", op, what, detail)
end

local function pass(op)
	bump(op, true)
end

local function norm(s)
	return (string.gsub(string.lower(tostring(s)), "[_%-%s]", ""))
end

local function isArray(t)
	return type(t) == "table" and t[1] ~= nil
end

-- ============================ value decoding ============================

-- Set per document from its `encoding` field.
local hexMode = false

local function isHexish(s: string)
	return #s <= 16 and string.match(s, "^%x+$") ~= nil
end

--- JSON scalar -> exact i64 value (never via a Luau number when it is a string).
local function decodeI64(v: any)
	if type(v) == "string" then
		local p2 = string.lower(string.sub(v, 1, 2))
		if p2 == "0x" or string.lower(string.sub(v, 1, 3)) == "-0x" then
			return Fixed.int64(v)
		end
		if hexMode and isHexish(v) then
			return Fixed.fromHex(v)
		end
		return Fixed.int64(v)
	end
	return Fixed.int64(v)
end

--- JSON scalar -> u64 as (hi, lo).
local function decodeU64(v: any)
	if type(v) == "string" then
		if string.lower(string.sub(v, 1, 2)) == "0x" or (hexMode and isHexish(v)) then
			return Rng.hexParts(v)
		end
		return Rng.seedParts(v)
	end
	return Rng.seedParts(v)
end

--- JSON scalar -> u32 number.
local function decodeU32(v: any)
	if type(v) == "number" then
		return v
	end
	if type(v) == "string" then
		if string.lower(string.sub(v, 1, 2)) == "0x" then
			return tonumber(string.sub(v, 3), 16)
		end
		if hexMode and isHexish(v) then
			return tonumber(v, 16)
		end
		return tonumber(v)
	end
	return nil
end

local EXPECT_KEYS = { "out", "expect", "expected", "result", "want", "r", "expect_raw" }

local function expectedOf(case)
	for _, k in ipairs(EXPECT_KEYS) do
		local v = rawget(case, k)
		if v ~= nil and v ~= Json.null then
			return v
		end
	end
	return nil
end

local function expectsPanic(case)
	if case.panics == true or case.error == true or case.throws == true or case.panic == true then
		return true
	end
	for _, k in ipairs(EXPECT_KEYS) do
		if rawget(case, k) == Json.null then
			return true
		end
	end
	return false
end

-- ============================ fixed vectors ============================

local FIXED_OPS = {
	from_int = { n = 1, args = { { "i", "a", "x", "value", "n" } }, run = function(a)
		return Fixed.fromInt(a)
	end },
	from_raw = { n = 1, args = { { "raw", "a", "x", "i" } }, run = function(a)
		return Fixed.fromRaw(a)
	end },
	raw = { n = 1, args = { { "raw", "a", "x", "i" } }, run = function(a)
		local v = Fixed.fromRaw(a)
		local ok, num = pcall(Fixed.raw, v)
		if ok then
			return Fixed.int64(num) -- exercise the real accessor when it is in range
		end
		return v
	end },
	from_ratio = { n = 2, args = { { "num", "a", "x", "n" }, { "den", "b", "y", "d" } }, run = function(a, b)
		return Fixed.fromRatio(a, b)
	end },
	floor_to_int = { n = 1, args = { { "x", "a", "raw", "value" } }, run = function(a)
		return Fixed.int64(Fixed.floorToInt(a))
	end },
	neg = { n = 1, args = { { "a", "x", "value" } }, run = function(a)
		return Fixed.neg(a)
	end },
	sqrt = { n = 1, args = { { "a", "x", "value" } }, run = function(a)
		return Fixed.sqrt(a)
	end },
	add = { n = 2, args = { { "a", "x", "lhs" }, { "b", "y", "rhs" } }, run = function(a, b)
		return Fixed.add(a, b)
	end },
	sub = { n = 2, args = { { "a", "x", "lhs" }, { "b", "y", "rhs" } }, run = function(a, b)
		return Fixed.sub(a, b)
	end },
	mul = { n = 2, args = { { "a", "x", "lhs" }, { "b", "y", "rhs" } }, run = function(a, b)
		return Fixed.mul(a, b)
	end },
	div = { n = 2, args = { { "a", "x", "lhs" }, { "b", "y", "rhs" } }, run = function(a, b)
		return Fixed.div(a, b)
	end },
	scale_i64 = { n = 2, args = { { "x", "a", "fixed", "self", "m" }, { "v", "b", "value", "i" } }, run = function(a, b)
		return Fixed.scaleI64(a, b)
	end },
}

-- Look-up table keyed by the normalized op name, including the aliases the
-- emitter actually uses. `*_wrapping` are the same functions: `from_int` and
-- `from_ratio` narrow by TRUNCATION in the Rust (only mul/div/scale_i64
-- saturate), and this port wraps identically — the separate lists just make
-- that asymmetry visible in the report.
local OP_BY_NAME = {}
for k, spec in pairs(FIXED_OPS) do
	OP_BY_NAME[norm(k)] = spec
end
OP_BY_NAME.fromintwrapping = FIXED_OPS.from_int
OP_BY_NAME.fromratiowrapping = FIXED_OPS.from_ratio
OP_BY_NAME.scalei64wrapping = FIXED_OPS.scale_i64
OP_BY_NAME.ratio = FIXED_OPS.from_ratio
OP_BY_NAME.floortoi64 = FIXED_OPS.floor_to_int

local function pick(case, names)
	for _, k in ipairs(names) do
		local v = rawget(case, k)
		if v ~= nil then
			return v
		end
	end
	return nil
end

local function caseLabel(opName, case)
	local parts = {}
	if isArray(case.args) then
		for _, v in ipairs(case.args) do
			parts[#parts + 1] = tostring(v)
		end
	else
		for _, k in ipairs({ "i", "a", "b", "x", "y", "v", "num", "den", "raw", "lhs", "rhs" }) do
			if rawget(case, k) ~= nil then
				parts[#parts + 1] = k .. "=" .. tostring(rawget(case, k))
			end
		end
	end
	return opName .. "(" .. table.concat(parts, ", ") .. ")"
end

local function runFixedCase(case, inheritedOp)
	if type(case) ~= "table" then
		fail("fixed", "case is not an object", tostring(case))
		return
	end
	local rawOp = case.op or case.name or case.fn or case.kind or inheritedOp
	if rawOp == nil then
		fail("fixed", "case has no op", caseLabel("?", case))
		return
	end
	local opName = norm(rawOp)
	local spec = OP_BY_NAME[opName]
	if spec == nil then
		fail(tostring(rawOp), "unsupported op (not in C2)", caseLabel(tostring(rawOp), case))
		return
	end

	local ra, rb
	if isArray(case.args) then
		ra, rb = case.args[1], case.args[2]
	else
		ra = pick(case, spec.args[1])
		if spec.n == 2 then
			rb = pick(case, spec.args[2])
		end
	end
	if ra == nil or (spec.n == 2 and rb == nil) then
		fail(opName, "could not read operands", caseLabel(opName, case))
		return
	end

	local label = caseLabel(opName, case)
	local okD, a, b = pcall(function()
		if spec.n == 2 then
			return decodeI64(ra), decodeI64(rb)
		end
		return decodeI64(ra), nil
	end)
	if not okD then
		fail(opName, "could not decode operands", label .. ": " .. tostring(a))
		return
	end

	local ok, got = pcall(spec.run, a, b)

	if expectsPanic(case) then
		if ok then
			fail(opName, "expected a panic, got a value", label .. " -> " .. Fixed.toDecimal(got))
		else
			pass(opName)
		end
		return
	end

	local want = expectedOf(case)
	if want == nil then
		fail(opName, "case has no expected value", label)
		return
	end
	if not ok then
		fail(opName, "errored", label .. " -> " .. tostring(got) .. " (expected " .. tostring(want) .. ")")
		return
	end

	local okW, wantV = pcall(decodeI64, want)
	if not okW then
		fail(opName, "unreadable expected value", label .. " expect=" .. tostring(want) .. " (" .. tostring(wantV) .. ")")
		return
	end
	local okG, gotV = pcall(Fixed.int64, got)
	if not okG then
		fail(opName, "unreadable result", label)
		return
	end
	if Fixed.compare(gotV, wantV) ~= 0 then
		fail(
			opName,
			"MISMATCH",
			label
				.. "\n        got      "
				.. Fixed.toHex(gotV)
				.. "  ("
				.. Fixed.toDecimal(gotV)
				.. ")\n        expected "
				.. Fixed.toHex(wantV)
				.. "  ("
				.. Fixed.toDecimal(wantV)
				.. ")"
		)
	else
		pass(opName)
	end
end

local function runFixedDoc(doc, label)
	local count = 0

	-- header sanity, when the document declares it
	if doc.frac_bits ~= nil then
		if tonumber(doc.frac_bits) == Fixed.FRAC_BITS then
			pass("header")
		else
			fail("header", "FRAC_BITS mismatch", label .. ": vectors say " .. tostring(doc.frac_bits))
		end
		count += 1
	end
	if doc.one ~= nil then
		local okO, oneV = pcall(decodeI64, doc.one)
		if okO and Fixed.compare(oneV, Fixed.ONE) == 0 then
			pass("header")
		else
			fail("header", "ONE mismatch", label .. ": vectors say " .. tostring(doc.one))
		end
		count += 1
	end

	local list = doc.cases or doc.fixed or doc.vectors or doc.tests or doc.fixed_vectors
	if list == nil and isArray(doc) then
		list = doc
	end
	if isArray(list) then
		for _, case in ipairs(list) do
			runFixedCase(case, nil)
			count += 1
		end
	end

	local groups = doc.ops or doc.groups or doc.by_op
	local function runGroups(tbl)
		for k, v in pairs(tbl) do
			if isArray(v) and OP_BY_NAME[norm(k)] then
				for _, case in ipairs(v) do
					runFixedCase(case, k)
					count += 1
				end
			end
		end
	end
	if type(groups) == "table" then
		runGroups(groups)
	end
	if list == nil and groups == nil then
		runGroups(doc)
	end

	if count == 0 then
		fail("fixed", "no cases found in document", label)
	end
	return count
end

-- ============================ rng vectors ============================

local function u64Eq(hi, lo, want)
	local ok, whi, wlo = pcall(decodeU64, want)
	if not ok then
		return false, "?"
	end
	return hi == whi and lo == wlo, Rng.toHex(whi, wlo)
end

local function checkU64(tag, op, hi, lo, want)
	local same, wtxt = u64Eq(hi, lo, want)
	if same then
		pass(op)
	else
		fail(op, "MISMATCH", tag .. "\n        got      " .. Rng.toHex(hi, lo) .. "\n        expected " .. wtxt .. " (" .. tostring(want) .. ")")
	end
end

--- `group` is either a plain array of expected values, or an object with
--- `out`/`values`/`expect` plus an optional `state_after`.
local function listAndTail(group)
	if isArray(group) then
		return group, nil
	end
	if type(group) ~= "table" then
		return nil, nil
	end
	local values = group.out or group.values or group.expect or group.expected or group.results
	return values, group.state_after or group.state or group.after
end

local function runU64List(mkRng, group, label)
	local values, after = listAndTail(group)
	if not isArray(values) then
		fail("nextU64", "missing output list", label)
		return
	end
	local r = mkRng()
	for i, want in ipairs(values) do
		local hi, lo = Rng.nextU64(r)
		checkU64(label .. " next_u64 #" .. i, "nextU64", hi, lo, want)
	end
	if after ~= nil then
		local hi, lo = Rng.state(r)
		checkU64(label .. " next_u64 state_after", "state", hi, lo, after)
	end
end

local function runU32List(mkRng, group, label)
	local values, after = listAndTail(group)
	if not isArray(values) then
		fail("nextU32", "missing output list", label)
		return
	end
	local r = mkRng()
	for i, want in ipairs(values) do
		local v = Rng.nextU32(r)
		local w = decodeU32(want)
		if v == w then
			pass("nextU32")
		else
			fail("nextU32", "MISMATCH", label .. " next_u32 #" .. i .. "\n        got      " .. tostring(v) .. "\n        expected " .. tostring(want))
		end
	end
	if after ~= nil then
		local hi, lo = Rng.state(r)
		checkU64(label .. " next_u32 state_after", "state", hi, lo, after)
	end
end

local function runBelowGroup(mkRng, group, label)
	local values, after = listAndTail(group)
	local n = decodeU32(group.n or group.bound or group.max)
	if values == nil or n == nil then
		fail("below", "below group missing n or outputs", label)
		return
	end
	local r = mkRng()
	for i, want in ipairs(values) do
		local v = Rng.below(r, n)
		local w = decodeU32(want)
		if v == w then
			pass("below")
		else
			fail("below", "MISMATCH", label .. " below(n=" .. n .. ") #" .. i .. "\n        got      " .. tostring(v) .. "\n        expected " .. tostring(want))
		end
	end
	if after ~= nil then
		local hi, lo = Rng.state(r)
		checkU64(label .. " below(n=" .. n .. ") state_after", "state", hi, lo, after)
	end
end

local function runChanceGroup(mkRng, group, label)
	local values, after = listAndTail(group)
	local num = decodeU32(group.num or group.n)
	local den = decodeU32(group.den or group.d)
	if values == nil or num == nil or den == nil then
		fail("chance", "chance group missing num/den or outputs", label)
		return
	end
	local r = mkRng()
	for i, want in ipairs(values) do
		local v = Rng.chance(r, num, den)
		local w = want
		if type(w) == "string" then
			w = (w == "true")
		end
		if v == w then
			pass("chance")
		else
			fail("chance", "MISMATCH", label .. " chance(" .. num .. "/" .. den .. ") #" .. i .. "\n        got      " .. tostring(v) .. "\n        expected " .. tostring(want))
		end
	end
	if after ~= nil then
		local hi, lo = Rng.state(r)
		checkU64(label .. " chance(" .. num .. "/" .. den .. ") state_after", "state", hi, lo, after)
	end
end

-- a group key may hold a single group object or an array of them
local function eachGroup(group, fn)
	if isArray(group) and type(group[1]) == "table" and not isArray(group[1]) then
		for _, g in ipairs(group) do
			fn(g)
		end
	else
		fn(group)
	end
end

local function runSequence(mkRng, seq, label)
	local r = mkRng()
	for i, step in ipairs(seq) do
		local op = norm(step.op or step.name or "")
		local want = expectedOf(step)
		local tag = label .. " sequence[" .. i .. "] " .. op
		if op == "nextu64" then
			local hi, lo = Rng.nextU64(r)
			checkU64(tag, "nextU64", hi, lo, want)
		elseif op == "nextu32" then
			local v = Rng.nextU32(r)
			if v == decodeU32(want) then
				pass("nextU32")
			else
				fail("nextU32", "MISMATCH", tag .. "\n        got      " .. tostring(v) .. "\n        expected " .. tostring(want))
			end
		elseif op == "below" then
			local n = decodeU32(step.n or step.bound or 0)
			local v = Rng.below(r, n)
			if v == decodeU32(want) then
				pass("below")
			else
				fail("below", "MISMATCH", tag .. "(n=" .. n .. ")\n        got      " .. tostring(v) .. "\n        expected " .. tostring(want))
			end
		elseif op == "chance" then
			local num = decodeU32(step.num or 0)
			local den = decodeU32(step.den or 1)
			local v = Rng.chance(r, num, den)
			if v == want then
				pass("chance")
			else
				fail("chance", "MISMATCH", tag .. "(" .. num .. "/" .. den .. ")\n        got      " .. tostring(v) .. "\n        expected " .. tostring(want))
			end
		elseif op == "state" then
			local hi, lo = Rng.state(r)
			checkU64(tag, "state", hi, lo, want)
		else
			fail(tostring(step.op), "unsupported rng op in sequence", tag)
		end
	end
end

--- Build the "fresh rng" factory for a stream entry, from either a seed or a
--- derive tuple.
local function rngFactory(entry, label)
	if entry.seed ~= nil then
		local hi, lo = decodeU64(entry.seed)
		return function()
			return Rng.fromSeedParts(hi, lo)
		end
	end
	local d = entry.derive or entry
	if d.master ~= nil then
		local mhi, mlo = decodeU64(d.master)
		local player = decodeU32(d.player or 0)
		local purpose = decodeU32(d.purpose or d.stream or 0)
		local round = decodeU32(d.round or 0)
		return function()
			return Rng.derive({ hi = mhi, lo = mlo }, player, purpose, round)
		end
	end
	error("stream '" .. label .. "' has neither a seed nor a derive tuple", 0)
end

local function runStream(entry, index)
	if type(entry) ~= "table" then
		fail("rng", "stream is not an object", tostring(entry))
		return
	end
	local label = tostring(
		entry.name
			or entry.label
			or (entry.seed and ("seed " .. tostring(entry.seed)))
			or (entry.master and ("derive " .. tostring(entry.master) .. "/" .. tostring(entry.player) .. "/" .. tostring(entry.purpose) .. "/" .. tostring(entry.round)))
			or ("stream#" .. index)
	)
	local okF, mkRng = pcall(rngFactory, entry, label)
	if not okF then
		fail("rng", "bad stream", tostring(mkRng))
		return
	end

	local recognized = 0
	for key, group in pairs(entry) do
		local k = norm(key)
		if (k == "state" or k == "expectedstate" or k == "derivedstate") and type(group) ~= "table" then
			local hi, lo = Rng.state(mkRng())
			checkU64(label .. " state", "state", hi, lo, group)
			recognized += 1
		elseif k == "nextu64" then
			runU64List(mkRng, group, label)
			recognized += 1
		elseif k == "nextu32" then
			runU32List(mkRng, group, label)
			recognized += 1
		elseif string.sub(k, 1, 5) == "below" and type(group) == "table" then
			eachGroup(group, function(g)
				runBelowGroup(mkRng, g, label)
			end)
			recognized += 1
		elseif string.sub(k, 1, 6) == "chance" and type(group) == "table" then
			eachGroup(group, function(g)
				runChanceGroup(mkRng, g, label)
			end)
			recognized += 1
		elseif (k == "sequence" or k == "ops" or k == "draws" or k == "steps") and isArray(group) then
			runSequence(mkRng, group, label)
			recognized += 1
		end
	end

	if recognized == 0 then
		fail("rng", "no recognized checks in stream", label)
	end
end

local function runRngDoc(doc, label)
	local total = 0
	local lists = {}
	if isArray(doc.seeds) then
		lists[#lists + 1] = doc.seeds
	end
	if isArray(doc.derive) then
		lists[#lists + 1] = doc.derive
	end
	if #lists == 0 then
		local list = doc.streams or doc.rng or doc.cases or doc.vectors or doc.tests
		if list == nil and isArray(doc) then
			list = doc
		end
		if isArray(list) then
			lists[#lists + 1] = list
		end
	end
	if #lists == 0 then
		fail("rng", "no streams found in document", label)
		return 0
	end
	for _, list in ipairs(lists) do
		for i, s in ipairs(list) do
			runStream(s, i)
			total += 1
		end
	end
	return total
end

-- ============================ document dispatch ============================

local function classify(doc)
	if type(doc) ~= "table" then
		return "unknown"
	end
	if doc.seeds ~= nil or doc.derive ~= nil or doc.streams ~= nil or doc.rng ~= nil or doc.rng_vectors ~= nil then
		return "rng"
	end
	if doc.fixed ~= nil or doc.fixed_vectors ~= nil then
		return "fixed"
	end
	local list = doc.cases or doc.vectors or doc.tests
	if list == nil and isArray(doc) then
		list = doc
	end
	if isArray(list) and type(list[1]) == "table" then
		local c = list[1]
		if c.seed ~= nil or c.derive ~= nil or c.master ~= nil or c.next_u64 ~= nil or c.sequence ~= nil then
			return "rng"
		end
		return "fixed"
	end
	if type(doc.ops) == "table" then
		return "fixed"
	end
	for k, v in pairs(doc) do
		if isArray(v) and OP_BY_NAME[norm(k)] then
			return "fixed"
		end
	end
	return "unknown"
end

-- ============================ input ============================

local args = { ... }
local docs = {}
local sources = {}

local function ingest(text, label)
	local ok, parsed = pcall(Json.decodeMany, text)
	if not ok then
		fail("input", "JSON parse error", label .. ": " .. tostring(parsed))
		return
	end
	for _, d in ipairs(parsed) do
		docs[#docs + 1] = d
		sources[#docs] = label
	end
end

if #args > 0 then
	ingest(table.concat(args, ""), "argv (" .. #args .. " chunk(s))")
end

local injected = rawget(_G, "PARITY_VECTORS")
if type(injected) == "table" then
	for i, v in ipairs(injected) do
		if type(v) == "string" then
			ingest(v, "PARITY_VECTORS[" .. i .. "]")
		elseif type(v) == "table" then
			docs[#docs + 1] = v
			sources[#docs] = "PARITY_VECTORS[" .. i .. "]"
		end
	end
end

if #docs == 0 then
	for _, name in ipairs({ "fixed_vectors", "rng_vectors" }) do
		local ok, mod = pcall(require, "./vectors/" .. name)
		if ok then
			if type(mod) == "string" then
				ingest(mod, name)
			elseif type(mod) == "table" then
				docs[#docs + 1] = mod
				sources[#docs] = name
			end
		end
	end
end

if #docs == 0 then
	for _, f in ipairs(failures) do
		print("  " .. f)
	end
	print("parity: NO VECTORS SUPPLIED.")
	print("  Pass the JSON on the command line after -a (see the header of this file):")
	print('    luau parity.luau -a "$(cat vectors/fixed_vectors.json)" "$(cat vectors/rng_vectors.json)"')
	error("parity: no vectors", 0)
end

-- ============================ run ============================

print(string.format("parity: %d document(s)", #docs))
for i, doc in ipairs(docs) do
	local kind = classify(doc)
	local label = sources[i] .. " doc#" .. i
	hexMode = type(doc.encoding) == "string" and string.find(string.lower(doc.encoding), "hex") ~= nil
	local enc = hexMode and "hex" or "decimal"
	if kind == "fixed" then
		local n = runFixedDoc(doc, label)
		print(string.format("  doc#%d  fixed  %-7s %d case(s)", i, enc, n))
	elseif kind == "rng" then
		local n = runRngDoc(doc, label)
		print(string.format("  doc#%d  rng    %-7s %d stream(s)", i, enc, n))
	else
		fail("input", "unrecognized document shape (neither fixed nor rng vectors)", label)
	end
end

-- ============================ report ============================

local opNames = {}
for k in pairs(perOp) do
	opNames[#opNames + 1] = k
end
table.sort(opNames)

print("")
print(string.format("%-20s %10s %10s", "op", "pass", "fail"))
for _, k in ipairs(opNames) do
	local e = perOp[k]
	print(string.format("%-20s %10d %10d", k, e.pass, e.fail))
end

print("")
if #failures > 0 then
	local show = math.min(#failures, 25)
	print(string.format("FAILURES (%d, showing %d):", #failures, show))
	for i = 1, show do
		print("  " .. failures[i])
	end
	print("")
	print(string.format("parity: FAIL — %d passed, %d failed", passed, #failures))
	error("parity: " .. #failures .. " failing assertion(s)", 0)
end

print(string.format("parity: PASS — %d assertions, 0 failures", passed))
