local C = require("../src/shared/Content")
print("hash", C.CONTENT_HASH, "weapons", #C.WEAPONS, "enemies", #C.ENEMIES, "mods", #C.MODIFIERS, "waves", #C.WAVES, "escort", #C.BOSS_ESCORT, "ring", #C.SPAWN_RING)
print("FROST_MAX_STACKS", C.FROST_MAX_STACKS, "STARTING_WEAPON", C.STARTING_WEAPON, "BOSS", C.BOSS, "NUM_SCOPES", C.NUM_SCOPES)
print("BOSS_SPAWN_TICK", C.BOSS_SPAWN_TICK, "RAMP_INTERVAL", C.RAMP_INTERVAL, "ROUND_TICKS", C.ROUND_TICKS, "TICK_HZ", C.TICK_HZ)
local names = {}
for k, v in pairs(C) do if k == string.upper(k) and type(v) ~= "table" and type(v) ~= "function" then names[#names+1] = k end end
table.sort(names)
print("FLAT CONSTANTS (" .. #names .. "):")
for _, n in ipairs(names) do print("  " .. n .. " = " .. tostring(C[n])) end
