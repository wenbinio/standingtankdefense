--[[
	Json.luau — a small, dependency-free JSON decoder.

	Exists for one reason: the parity vectors are JSON emitted by the Rust, and
	they carry raw i64 / u64 values that a Luau double CANNOT hold exactly.

	THE BIG-INTEGER RULE
	--------------------
	An integer literal whose magnitude is < 2^53 decodes to a `number`.
	An integer literal whose magnitude is >= 2^53 decodes to the LITERAL TEXT as
	a `string` (e.g. "9223372036854775807"), because turning it into a double
	would silently change its value. `Fixed.int64` / `Rng.seedParts` accept both
	forms, so consumers do not have to care — but nothing is ever rounded behind
	their back.

	References no Roblox global; runs under a bare `luau` interpreter.
]]

local Json = {}

local SAFE = 9007199254740992 -- 2^53

--- Sentinel for JSON `null` (storing nil would delete the key).
Json.null = setmetatable({}, { __tostring = function()
	return "null"
end })

local ESCAPES = {
	[34] = '"',
	[92] = "\\",
	[47] = "/",
	[98] = "\b",
	[102] = "\f",
	[110] = "\n",
	[114] = "\r",
	[116] = "\t",
}

local function utf8Encode(cp)
	if cp < 0x80 then
		return string.char(cp)
	elseif cp < 0x800 then
		return string.char(0xC0 + (cp - cp % 64) / 64, 0x80 + cp % 64)
	elseif cp < 0x10000 then
		local a = (cp - cp % 4096) / 4096
		local b = ((cp % 4096) - cp % 64) / 64
		return string.char(0xE0 + a, 0x80 + b, 0x80 + cp % 64)
	else
		local a = (cp - cp % 262144) / 262144
		local b = ((cp % 262144) - cp % 4096) / 4096
		local c = ((cp % 4096) - cp % 64) / 64
		return string.char(0xF0 + a, 0x80 + b, 0x80 + c, 0x80 + cp % 64)
	end
end

local parseValue

local function fail(s: string, i: number, msg: string): never
	local line = 1
	for _ in string.gmatch(string.sub(s, 1, i), "\n") do
		line = line + 1
	end
	error("Json: " .. msg .. " at byte " .. i .. " (line " .. line .. ")", 0)
end

local function skipWs(s, i)
	local _, j = string.find(s, "^[ \t\r\n]*", i)
	return (j or i - 1) + 1
end

local function parseString(s, i)
	-- s[i] == '"'
	i = i + 1
	local out = {}
	while true do
		local c = string.byte(s, i)
		if c == nil then
			fail(s, i, "unterminated string")
		elseif c == 34 then
			return table.concat(out), i + 1
		elseif c == 92 then
			local e = string.byte(s, i + 1)
			local simple = ESCAPES[e]
			if simple then
				out[#out + 1] = simple
				i = i + 2
			elseif e == 117 then -- \u
				local hex = string.sub(s, i + 2, i + 5)
				local cp = tonumber(hex, 16)
				if cp == nil then
					fail(s, i, "bad \\u escape")
				end
				i = i + 6
				if cp >= 0xD800 and cp <= 0xDBFF and string.sub(s, i, i + 1) == "\\u" then
					local lo = tonumber(string.sub(s, i + 2, i + 5), 16)
					if lo and lo >= 0xDC00 and lo <= 0xDFFF then
						cp = 0x10000 + (cp - 0xD800) * 0x400 + (lo - 0xDC00)
						i = i + 6
					end
				end
				out[#out + 1] = utf8Encode(cp)
			else
				fail(s, i, "bad escape")
			end
		else
			local j = string.find(s, '["\\]', i)
			if j == nil then
				fail(s, i, "unterminated string")
			end
			out[#out + 1] = string.sub(s, i, j - 1)
			i = j
		end
	end
end

local function parseNumber(s, i)
	local first, last = string.find(s, "^-?%d+%.?%d*[eE]?[-+]?%d*", i)
	if first == nil then
		fail(s, i, "bad number")
	end
	local text = string.sub(s, first, last)
	local n = tonumber(text)
	if n == nil then
		fail(s, i, "bad number '" .. text .. "'")
	end
	-- Integer literals too large for an exact double are kept as text.
	if not string.find(text, "[%.eE]") then
		if n >= SAFE or n <= -SAFE then
			return text, last + 1
		end
	end
	return n, last + 1
end

parseValue = function(s, i)
	i = skipWs(s, i)
	local c = string.byte(s, i)
	if c == nil then
		return fail(s, i, "unexpected end of input")
	elseif c == 123 then -- {
		local obj = {}
		i = skipWs(s, i + 1)
		if string.byte(s, i) == 125 then
			return obj, i + 1
		end
		while true do
			i = skipWs(s, i)
			if string.byte(s, i) ~= 34 then
				fail(s, i, "expected object key")
			end
			local k
			k, i = parseString(s, i)
			i = skipWs(s, i)
			if string.byte(s, i) ~= 58 then
				fail(s, i, "expected ':'")
			end
			local v
			v, i = parseValue(s, i + 1)
			obj[k] = v
			i = skipWs(s, i)
			local d = string.byte(s, i)
			if d == 44 then
				i = i + 1
			elseif d == 125 then
				return obj, i + 1
			else
				fail(s, i, "expected ',' or '}'")
			end
		end
	elseif c == 91 then -- [
		local arr = {}
		local n = 0
		i = skipWs(s, i + 1)
		if string.byte(s, i) == 93 then
			return arr, i + 1
		end
		while true do
			local v
			v, i = parseValue(s, i)
			n = n + 1
			arr[n] = v
			i = skipWs(s, i)
			local d = string.byte(s, i)
			if d == 44 then
				i = i + 1
			elseif d == 93 then
				return arr, i + 1
			else
				fail(s, i, "expected ',' or ']'")
			end
		end
	elseif c == 34 then -- "
		return parseString(s, i)
	elseif string.sub(s, i, i + 3) == "true" then
		return true, i + 4
	elseif string.sub(s, i, i + 4) == "false" then
		return false, i + 5
	elseif string.sub(s, i, i + 3) == "null" then
		return Json.null, i + 4
	else
		return parseNumber(s, i)
	end
end

--- Decode one JSON document. Trailing whitespace is allowed.
function Json.decode(text)
	local v, i = parseValue(text, 1)
	i = skipWs(text, i)
	if i <= #text then
		fail(text, i, "trailing content")
	end
	return v
end

--- Decode a whitespace-separated SEQUENCE of JSON documents into an array.
--- (The parity runner receives its vectors as concatenated argv chunks, which
--- may hold several documents.)
function Json.decodeMany(text)
	local out = {}
	local i = skipWs(text, 1)
	while i <= #text do
		local v
		v, i = parseValue(text, i)
		out[#out + 1] = v
		i = skipWs(text, i)
	end
	return out
end

return Json
