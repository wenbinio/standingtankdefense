--[[
	trace_test.luau — the R3 gate runner (`roblox/SIM-SPEC.md` §S6).

	R3 is done when the Luau per-tick checksum trace is IDENTICAL to the Rust's,
	for every seed in the trace corpus, for the full run length. Not "looks
	right", not "plays fine" — byte-identical `u64` per tick.

	This runner loads a trace document, constructs a fresh state for its seed,
	replays the scripted inputs through `Arena.step`, and asserts the Luau
	checksum after EVERY tick equals the trace's. On a mismatch it reports THE
	FIRST DIVERGENT TICK and stops — that number is the only useful debugging
	output, so it is printed loudly and everything after it is noise.

	HOW TO RUN
	----------
	The standalone `luau` interpreter has NO file I/O, so the trace TEXT is handed
	to the runner on the command line. Every trailing argument after `-a` that is
	not a `--flag` is concatenated in order and parsed as a sequence of JSON
	documents — so a file may be passed whole, or split into chunks to stay under
	the OS per-argument limit (~128 KiB on Linux). From `roblox/test`:

	    luau trace_test.luau -a "$(cat traces/seed_12345.json)"

	    # whole corpus + content.json, chunked:
	    tmp=$(mktemp -d); cat ../src/shared/content.json traces/*.json > "$tmp/all"
	    split -b 100000 "$tmp/all" "$tmp/c."
	    args=(); for f in "$tmp"/c.*; do args+=("$(cat "$f")"); done
	    luau trace_test.luau -a "${args[@]}"

	FLAGS (passed among the argv chunks; they start with `--`, JSON never does)
	--------------------------------------------------------------------------
	    --verbose        On divergence, dump EVERY checksummed field, in checksum
	                     order, for the divergent tick — so the first differing
	                     field is one `diff` away from the Rust's dump.
	    --tick=N         With --verbose, also dump the fields after tick N of
	                     every trace, pass or fail.
	    --max-ticks=N    Stop each trace after N ticks (a fast smoke run).

	DOCUMENT SCHEMA (§S6)
	---------------------
	    { "schema_version": 1, "seed": 12345, "content_hash": "<hex>",
	      "inputs": [ {"tick": 0, "code": 0}, ... ],
	      "checksums": [ "<16-hex u64>", ... ] }

	  * `checksums[i]` is the checksum AFTER tick `i` completes (0-based tick,
	    1-based Luau array — `checksums[1]` is the state after tick 0).
	  * `inputs` is the scripted input log; ticks not listed are `Noop`.
	  * `content_hash` must match `content.json`'s, so a stale trace fails loudly
	    instead of silently. Supply `content.json` as one of the argv documents if
	    `Content.luau` does not expose its own hash.
	  * `u64` values are 16-digit lowercase hex (`CONTRACTS.md` §C4).

	INPUT CODES
	-----------
	A trace's `code` IS an `Input.luau` value: the packed integer of
	`net::wire::InputCode` (0 Noop, 2 Reroll, 3 Clear, `1 + slot*256` BuyOffer).
	It is validated through `Input.kind` and passed to `Arena.step` unchanged. An
	unrecognized code is a LOUD failure, never a silent Noop.
]]

local Json = require("../src/shared/Json")
local Content = require("../src/shared/Content")
local Arena = require("../src/shared/sim/Arena")
local Input = require("../src/shared/sim/Input")

-- ============================ argv ============================

local rawArgs = { ... }
local verbose = false
local dumpTick = nil
local maxTicks = nil
local chunks = {}

for _, a in ipairs(rawArgs) do
	if string.sub(a, 1, 2) == "--" then
		if a == "--verbose" then
			verbose = true
		elseif string.sub(a, 1, 7) == "--tick=" then
			dumpTick = tonumber(string.sub(a, 8))
		elseif string.sub(a, 1, 12) == "--max-ticks=" then
			maxTicks = tonumber(string.sub(a, 13))
		else
			error("trace_test: unknown flag " .. a, 0)
		end
	else
		chunks[#chunks + 1] = a
	end
end

local docs = {}
if #chunks > 0 then
	local ok, parsed = pcall(Json.decodeMany, table.concat(chunks, ""))
	if not ok then
		error("trace_test: JSON parse error: " .. tostring(parsed), 0)
	end
	for _, d in ipairs(parsed) do
		docs[#docs + 1] = d
	end
end

local injected = rawget(_G, "TRACE_DOCS")
if type(injected) == "table" then
	for _, v in ipairs(injected) do
		if type(v) == "string" then
			for _, d in ipairs(Json.decodeMany(v)) do
				docs[#docs + 1] = d
			end
		elseif type(v) == "table" then
			docs[#docs + 1] = v
		end
	end
end

-- ============================ document classification ============================

local traces = {}
local contentDoc = nil

for _, d in ipairs(docs) do
	if type(d) == "table" then
		if type(d.checksums) == "table" then
			traces[#traces + 1] = d
		elseif type(d.weapons) == "table" and type(d.damage_matrix) == "table" then
			contentDoc = d
		end
	end
end

-- ============================ content hash ============================

local function normHash(h)
	if type(h) ~= "string" then
		return nil
	end
	local s = string.lower(h)
	if string.sub(s, 1, 2) == "0x" then
		s = string.sub(s, 3)
	end
	return s
end

--- The hash of the content this build is actually running, taken from
--- `content.json` itself when it was supplied on argv, otherwise from whatever
--- `Content.luau` publishes.
local function resolveContentHash()
	if contentDoc ~= nil then
		return normHash(contentDoc.content_hash), "content.json (argv)"
	end
	for _, k in ipairs({ "CONTENT_HASH", "contentHash", "content_hash" }) do
		local v = Content[k]
		if type(v) == "function" then
			local ok, r = pcall(v)
			if ok then
				v = r
			end
		end
		local n = normHash(v)
		if n ~= nil then
			return n, "Content." .. k
		end
	end
	for _, k in ipairs({ "doc", "document", "json", "raw" }) do
		local d = Content[k]
		if type(d) == "table" then
			local n = normHash(d.content_hash)
			if n ~= nil then
				return n, "Content." .. k .. ".content_hash"
			end
		end
	end
	return nil, nil
end

-- If Content wants the catalog handed to it explicitly, do so before anything
-- constructs state.
if contentDoc ~= nil and type(Content.load) == "function" then
	Content.load(contentDoc)
end

local expectedHash, hashSource = resolveContentHash()

-- ============================ input decoding ============================

local function decodeInput(entry, label)
	local code = entry.code
	if type(code) ~= "number" then
		error("trace_test: " .. label .. ": input entry has no numeric `code`", 0)
	end
	-- Validate through the module that owns the encoding rather than re-deriving
	-- it here: an unknown tag must be a loud failure, never a silent Noop.
	local ok, kind = pcall(Input.kind, code)
	if not ok or kind == nil then
		error("trace_test: " .. label .. ": unrecognized input code " .. tostring(code), 0)
	end
	return code
end

-- ============================ field dump ============================

--- Every checksummed write for this state, in checksum order: the label, the
--- exact bits hashed, and the RUNNING digest after that write. Diff two of these
--- (Luau vs Rust) and the first differing line is the field that diverged.
local function dumpFields(s, tick)
	local rows = Arena.checksumFields(s)
	print(string.format("---- checksummed fields after tick %d (%d writes) ----", tick, #rows))
	for i, r in ipairs(rows) do
		print(string.format("%5d  %-44s %-6s %-16s %s", i, tostring(r.label), r.kind, r.value, r.digest))
	end
	print("---- end field dump ----")
end

-- ============================ the gate ============================

local failures = {}

local function runTrace(doc, label)
	local seed = doc.seed
	if seed == nil then
		failures[#failures + 1] = label .. ": document has no `seed`"
		return
	end
	if doc.schema_version ~= nil and doc.schema_version ~= 1 then
		failures[#failures + 1] =
			string.format("%s: schema_version %s (runner speaks 1)", label, tostring(doc.schema_version))
		return
	end

	-- STALE-TRACE GATE. A trace recorded against different content is worthless:
	-- it would diverge for a reason that has nothing to do with the transcription.
	local traceHash = normHash(doc.content_hash)
	if traceHash == nil then
		failures[#failures + 1] = label .. ": document has no `content_hash`"
		return
	end
	if expectedHash == nil then
		failures[#failures + 1] = label
			.. ": cannot verify content_hash — neither content.json nor Content.luau"
			.. " supplied one. Pass content.json as an argv document."
		return
	end
	if traceHash ~= expectedHash then
		failures[#failures + 1] = string.format(
			"%s: STALE TRACE — content_hash %s, but this build's content is %s (from %s)."
				.. " Regenerate the corpus against the current content.json.",
			label,
			traceHash,
			expectedHash,
			tostring(hashSource)
		)
		return
	end

	-- Scripted inputs, keyed by the 0-based tick they apply at.
	local byTick = {}
	local inputs = doc.inputs
	if inputs ~= nil then
		if type(inputs) ~= "table" then
			failures[#failures + 1] = label .. ": `inputs` is not an array"
			return
		end
		for i, e in ipairs(inputs) do
			if type(e) ~= "table" or type(e.tick) ~= "number" then
				failures[#failures + 1] = label .. ": inputs[" .. i .. "] has no numeric `tick`"
				return
			end
			byTick[e.tick] = decodeInput(e, label .. " inputs[" .. i .. "]")
		end
	end

	local checksums = doc.checksums
	local n = #checksums
	if maxTicks ~= nil and maxTicks < n then
		n = maxTicks
	end

	local s = Arena.newState(seed, doc.player_id or 0)

	for i = 1, n do
		local tick = i - 1 -- `checksums[i]` is the state AFTER tick `i-1`
		Arena.step(s, byTick[tick] or Input.NOOP)
		local got = Arena.checksum(s)
		local want = normHash(checksums[i])
		if want == nil then
			failures[#failures + 1] =
				string.format("%s: checksums[%d] is not a hex string (%s)", label, i, tostring(checksums[i]))
			return
		end
		if got ~= want then
			print("")
			print("========================================================")
			print(string.format("  %s: FIRST DIVERGENT TICK = %d", label, tick))
			print("========================================================")
			print(string.format("    expected (rust) : %s", want))
			print(string.format("    got      (luau) : %s", got))
			if tick > 0 then
				print(string.format("    ticks verified  : %d (0 .. %d)", tick, tick - 1))
			else
				print("    ticks verified  : 0 — the very first tick already diverges")
			end
			if verbose then
				dumpFields(s, tick)
			else
				print("    re-run with --verbose to dump every checksummed field at this tick.")
			end
			failures[#failures + 1] = string.format("%s: diverged at tick %d", label, tick)
			return -- stop at the FIRST divergence; everything after it is noise
		end
		if verbose and dumpTick == tick then
			dumpFields(s, tick)
		end
	end

	print(string.format("  %-28s OK  %d tick(s), seed %s", label, n, tostring(seed)))
end

-- ============================ main ============================

if #traces == 0 then
	print("trace_test: NO TRACES SUPPLIED.")
	print("  Pass the trace JSON on the command line after -a (see the header of this file):")
	print('    luau trace_test.luau -a "$(cat traces/seed_12345.json)"')
	error("trace_test: no traces", 0)
end

print(string.format("trace_test: %d trace(s); content_hash %s (%s)", #traces, tostring(expectedHash), tostring(hashSource)))

for i, doc in ipairs(traces) do
	local label = doc.name or ("trace#" .. i .. " seed=" .. tostring(doc.seed))
	runTrace(doc, label)
end

print("")
if #failures > 0 then
	print(string.format("trace_test: %d FAILURE(S)", #failures))
	for _, f in ipairs(failures) do
		print("  " .. f)
	end
	error("trace_test: FAILED", 0)
end
print(string.format("trace_test: PASS — %d trace(s); every per-tick checksum matched the recorded trace.", #traces))
