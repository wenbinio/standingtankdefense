--!nonstrict
--[[
	checksum_test.luau — proves `src/shared/Checksum.luau` reproduces
	`determinism::Checksum` bit for bit.

	HOW TO RUN (no arguments needed — every expectation is embedded):

	    cd roblox/test && luau checksum_test.luau

	WHERE THE EXPECTED DIGESTS COME FROM
	------------------------------------
	They were produced BY RUNNING THE RUST, never by hand: a throwaway crate
	outside this repo took a path dependency on `sim-core/crates/determinism`,
	replayed each op sequence below through the real `determinism::Checksum`, and
	printed `finish()` as 16-digit lowercase hex. The op scripts and the digests
	in `CASES` are that program's verbatim output. Regenerating them is a matter
	of replaying the same sequences through the Rust; do NOT edit an `expect`
	value to make a failing test pass — a self-consistent-but-wrong checksum is
	the one outcome that makes the R3 oracle worthless.

	Every op is carried as a 16-digit two's-complement hex string (the C4 wire
	form), plus — when the value fits a double exactly — its decimal form. Ops
	that carry both are asserted through BOTH entry paths (plain Luau number and
	opaque `Fixed` value), which is what pins `writeI64`/`writeFixed`'s number
	conversion against the full-range one.
]]

local Fixed = require("../src/shared/Fixed")
local Checksum = require("../src/shared/Checksum")

local failures = 0
local checks = 0

local function fail(msg: string)
	failures += 1
	print("FAIL  " .. msg)
end

local function eq(got, want, msg: string)
	checks += 1
	if got ~= want then
		fail(msg .. ": got " .. tostring(got) .. ", want " .. tostring(want))
	end
end

-- Each op is { kind, valueHex, decimal? }.
local CASES = {
	{ name = "empty", expect = "cbf29ce484222325", ops = {  } },
	{ name = "u64_0", expect = "a8c7f832281a39c5", ops = { { "u64", "0000000000000000", 0 } } },
	{ name = "u64_1", expect = "89cd31291d2aefa4", ops = { { "u64", "0000000000000001", 1 } } },
	{ name = "u64_1_2", expect = "7717980363c8e066", ops = { { "u64", "0000000000000001", 1 }, { "u64", "0000000000000002", 2 } } },
	{ name = "u64_2_1", expect = "072184407c3a4ac6", ops = { { "u64", "0000000000000002", 2 }, { "u64", "0000000000000001", 1 } } },
	{ name = "u64_max", expect = "8cf51a8bfca3883d", ops = { { "u64", "ffffffffffffffff" } } },
	{ name = "u64_golden", expect = "6d19e938bfe4af54", ops = { { "u64", "9e3779b97f4a7c15" } } },
	{ name = "u64_deadbeef", expect = "4a24c862f1a075ae", ops = { { "u64", "deadbeefcafef00d" } } },
	{ name = "u64_hi_only", expect = "3cd02a862a574321", ops = { { "u64", "ffffffff00000000" } } },
	{ name = "u64_lo_only", expect = "4ebaad864bc6f2e1", ops = { { "u64", "00000000ffffffff", 4294967295 } } },
	{ name = "u64_run", expect = "da93818ecbae67d5", ops = { { "u64", "0102030405060708" }, { "u64", "020406080a0c0e10" }, { "u64", "0306090c0f121518" }, { "u64", "04080c1014181c20" }, { "u64", "050a0f14191e2328" }, { "u64", "060c12181e242a30" }, { "u64", "070e151c232a3138" }, { "u64", "0810182028303840" } } },
	{ name = "u32_0", expect = "a8c7f832281a39c5", ops = { { "u32", "0000000000000000", 0 } } },
	{ name = "u32_1", expect = "89cd31291d2aefa4", ops = { { "u32", "0000000000000001", 1 } } },
	{ name = "u32_max", expect = "4ebaad864bc6f2e1", ops = { { "u32", "00000000ffffffff", 4294967295 } } },
	{ name = "u32_seq", expect = "2a4af55120cddd0a", ops = { { "u32", "0000000000000000", 0 }, { "u32", "0000000000000001", 1 }, { "u32", "0000000000000002", 2 }, { "u32", "000000000000001e", 30 }, { "u32", "000000000000d2f0", 54000 }, { "u32", "00000000ffffffff", 4294967295 } } },
	{ name = "u32_vs_u64_same", expect = "d52608e5c8b3b625", ops = { { "u32", "0000000000003039", 12345 }, { "u64", "0000000000003039", 12345 } } },
	{ name = "i64_0", expect = "a8c7f832281a39c5", ops = { { "i64", "0000000000000000", 0 } } },
	{ name = "i64_neg1", expect = "8cf51a8bfca3883d", ops = { { "i64", "ffffffffffffffff", -1 } } },
	{ name = "i64_neg2", expect = "fc1a35225397861c", ops = { { "i64", "fffffffffffffffe", -2 } } },
	{ name = "i64_pow32_neg", expect = "3cd02a862a574321", ops = { { "i64", "ffffffff00000000", -4294967296 } } },
	{ name = "i64_pow32_neg1", expect = "f92ed8f4ce1c3300", ops = { { "i64", "ffffffff00000001", -4294967295 } } },
	{ name = "i64_min", expect = "a8c7783228196045", ops = { { "i64", "8000000000000000" } } },
	{ name = "i64_max", expect = "8cf59a8bfca461bd", ops = { { "i64", "7fffffffffffffff" } } },
	{ name = "i64_safe_pos", expect = "a85b383227bdd4a5", ops = { { "i64", "0020000000000000", 9007199254740992 } } },
	{ name = "i64_safe_neg", expect = "aae7a93229e819e8", ops = { { "i64", "ffe0000000000000", -9007199254740992 } } },
	{ name = "i64_mixed", expect = "fbc6aa98ba4f9404", ops = { { "i64", "00000000000003e8", 1000 }, { "i64", "fffffffffffffc18", -1000 }, { "i64", "0000000000010000", 65536 }, { "i64", "ffffffffffff0000", -65536 }, { "i64", "0000000000000001", 1 }, { "i64", "ffffffffffffffff", -1 } } },
	{ name = "fixed_zero", expect = "a8c7f832281a39c5", ops = { { "fixed", "0000000000000000", 0 } } },
	{ name = "fixed_one", expect = "cc3193ff3a28738c", ops = { { "fixed", "0000000000010000", 65536 } } },
	{ name = "fixed_neg_one", expect = "947d457e83b076f7", ops = { { "fixed", "ffffffffffff0000", -65536 } } },
	{ name = "fixed_frac", expect = "401d524ad51e77ce", ops = { { "fixed", "0000000000000001", 1 }, { "fixed", "ffffffffffffffff", -1 }, { "fixed", "0000000000008000", 32768 }, { "fixed", "ffffffffffff8000", -32768 } } },
	{ name = "fixed_ring0", expect = "788240e64d71ef49", ops = { { "fixed", "0000000004b00000", 78643200 }, { "fixed", "0000000000000000", 0 } } },
	{ name = "fixed_min_max", expect = "75b074d98b3a39dd", ops = { { "fixed", "8000000000000000" }, { "fixed", "7fffffffffffffff" } } },
	{ name = "mixed_state_prologue", expect = "59160c8a4be7db69", ops = { { "u32", "0000000000000000", 0 }, { "u32", "0000000000000000", 0 }, { "u64", "deadbeefcafef00d" }, { "u32", "0000000000000003", 3 }, { "i64", "0000000000001388", 5000 }, { "i64", "0000000000001388", 5000 }, { "fixed", "0000000000000000", 0 }, { "fixed", "0000000000000000", 0 }, { "u32", "0000000000000000", 0 }, { "i64", "0000000000000000", 0 }, { "u32", "0000000000000000", 0 }, { "u32", "0000000000000064", 100 }, { "i64", "0000000000000000", 0 }, { "i64", "0000000000000000", 0 }, { "i64", "0000000000000000", 0 }, { "i64", "0000000000000000", 0 }, { "i64", "0000000000000000", 0 }, { "fixed", "0000000000010000", 65536 }, { "i64", "0000000000000000", 0 }, { "i64", "0000000000000000", 0 }, { "fixed", "0000000000010000", 65536 }, { "fixed", "0000000000000000", 0 }, { "u32", "0000000000000000", 0 }, { "i64", "0000000000000000", 0 } } },
	{ name = "long_random_256", expect = "51dcc8df70889a67", ops = { { "u64", "d347bc89f523c9c2" }, { "i64", "e63433d6558a6173" }, { "u32", "00000000ea21696e", 3928058222 }, { "fixed", "b21d98e9af7c0d66" }, { "u64", "680b3b9a1a59b797" }, { "i64", "e146518f1fd48003" }, { "u32", "00000000d96fb0a9", 3647975593 }, { "fixed", "295a9ab88b4e18eb" }, { "u64", "892f3ab431821ec2" }, { "i64", "3ed4726a5a985871" }, { "u32", "00000000ee146f29", 3994316585 }, { "fixed", "51b056ee0f4ca553" }, { "u64", "b0daf6259d202701" }, { "i64", "73aaea276f0a66bd" }, { "u32", "00000000e6dee96e", 3873368430 }, { "fixed", "b3c84f298787aa53" }, { "u64", "16a96ab972ec54fa" }, { "i64", "588a7add41579339" }, { "u32", "00000000494b0e0d", 1229655565 }, { "fixed", "bfe773561b778b29" }, { "u64", "276f1d0e6fd2c59c" }, { "i64", "932812d3c87153f2" }, { "u32", "00000000c74fa8b8", 3343886520 }, { "fixed", "d8d77be67af7d3ec" }, { "u64", "3d915ba9aea18697" }, { "i64", "8b3967e637971b48" }, { "u32", "000000008707fc0b", 2265447435 }, { "fixed", "23e948fc8df402ad" }, { "u64", "6eda4d3d9e322db7" }, { "i64", "f62d7b7c3d9a6cd4" }, { "u32", "000000006b1f644e", 1797219406 }, { "fixed", "6484e708b36eb29c" }, { "u64", "89526611f5a3516f" }, { "i64", "3e7d626edbc3c331" }, { "u32", "000000006205a337", 1644536631 }, { "fixed", "e5767354a57b3dec" }, { "u64", "feca99d49d32a543" }, { "i64", "7dece6f9611f321b" }, { "u32", "00000000df36956f", 3744896367 }, { "fixed", "a8a6fa6e4925fd2a" }, { "u64", "5013b13994b72746" }, { "i64", "cfef396b13f7852b" }, { "u32", "000000006091bd80", 1620163968 }, { "fixed", "3301e2bf4b471b2d" }, { "u64", "1ebe4224afc9d0b2" }, { "i64", "64d4ff15099c647e" }, { "u32", "000000005a1af64f", 1511716431 }, { "fixed", "862aae4ede888e0d" }, { "u64", "6d38aa06cc9f5418" }, { "i64", "220641795dffb4cf" }, { "u32", "00000000bb3a1b1e", 3141147422 }, { "fixed", "f3fdf1c034b9faa8" }, { "u64", "1ff394207459dfb7" }, { "i64", "c5a2dde684aea673" }, { "u32", "00000000c5b7e541", 3317163329 }, { "fixed", "e033754accff80a9" }, { "u64", "90fcf11ef1009978" }, { "i64", "1c8c7bd947bf1f28" }, { "u32", "0000000028c61fdc", 684072924 }, { "fixed", "55aa15d3387238d1" }, { "u64", "5bd1bf8e908111af" }, { "i64", "51807b05d51ff235" }, { "u32", "00000000b1a37511", 2980279569 }, { "fixed", "57321c0941bed249" }, { "u64", "22ba5c8ac304eb93" }, { "i64", "68713dc741848005" }, { "u32", "00000000403bd8d4", 1077663956 }, { "fixed", "ec979f905dc1fff2" }, { "u64", "9e99cfe6c0971dd0" }, { "i64", "c51b6308917feaf8" }, { "u32", "000000009ec39d9b", 2663619995 }, { "fixed", "17890d69b30cf695" }, { "u64", "f42fe6e734fe9f15" }, { "i64", "ed1306f7c1339b65" }, { "u32", "00000000d06c89a6", 3496774054 }, { "fixed", "e002997e4a8fa8bd" }, { "u64", "99fac695e5e1c6d3" }, { "i64", "5b89af1b19e2165d" }, { "u32", "00000000f6d2fc28", 4141022248 }, { "fixed", "f8a29937b2b47d3b" }, { "u64", "d1e0fe94573f0596" }, { "i64", "3fd18dcb6c44bc05" }, { "u32", "0000000085a8cd21", 2242432289 }, { "fixed", "d9f97495c0f94755" }, { "u64", "2a81167195d00d91" }, { "i64", "16259427b18b0a8b" }, { "u32", "00000000f8686bf4", 4167592948 }, { "fixed", "91601e8a7189ba03" }, { "u64", "4f90f00f3263ef3a" }, { "i64", "75e163ffb95f7254" }, { "u32", "00000000052224af", 86123695 }, { "fixed", "9ba4b3e1a1126458" }, { "u64", "277ec9435c440c22" }, { "i64", "849fe7a41b8db037" }, { "u32", "000000004f06def8", 1325850360 }, { "fixed", "b43a72bd0a7681a3" }, { "u64", "63dec919798335cb" }, { "i64", "63b02a2965c5557e" }, { "u32", "00000000d5cad25f", 3586839135 }, { "fixed", "6adb8cc8d46787c1" }, { "u64", "e0985fcfebdab14c" }, { "i64", "856247ba484e7063" }, { "u32", "0000000024ec616f", 619471215 }, { "fixed", "b946d3fafdab1682" }, { "u64", "390d34792f279002" }, { "i64", "6a3fc891366416c3" }, { "u32", "0000000005c5de99", 96853657 }, { "fixed", "d0fc686fd18bf52b" }, { "u64", "1f689380da1bca63" }, { "i64", "a17be4b6cf4024bc" }, { "u32", "000000001dd472d2", 500462290 }, { "fixed", "002a89634785f290" }, { "u64", "5355d21cfbf90ad7" }, { "i64", "cf67232774a729f4" }, { "u32", "000000006b87817a", 1804042618 }, { "fixed", "76c42c5086083049" }, { "u64", "f939890c5cd8446e" }, { "i64", "c47732191518d88f" }, { "u32", "0000000002574cfb", 39275771 }, { "fixed", "7fd05e505ec10792" }, { "u64", "cae2badf756128c4" }, { "i64", "c35a220cb1008ab2" }, { "u32", "000000001a1ebaa4", 438221476 }, { "fixed", "86ae1385481cf287" }, { "u64", "69d0e6cd36854a6a" }, { "i64", "3dea25e73a9a8cd4" }, { "u32", "000000007b39a958", 2067376472 }, { "fixed", "a59ed0fcd47acb67" }, { "u64", "037da1d33c649058" }, { "i64", "476a18c5646026c2" }, { "u32", "000000003fb5fa37", 1068890679 }, { "fixed", "bf26e3d0e52d2cbb" }, { "u64", "8f3054bcbdb10e1f" }, { "i64", "7c2fb257fe69621f" }, { "u32", "0000000081ef6ba6", 2179951526 }, { "fixed", "468852ed520b5466" }, { "u64", "863f2d2da7c7f2ae" }, { "i64", "aef298774f99ade4" }, { "u32", "000000002a7b0bcb", 712707019 }, { "fixed", "854f27fda5eeb5f1" }, { "u64", "d24777752c247b45" }, { "i64", "8a91dd8f83c450be" }, { "u32", "0000000091a3cb2a", 2443430698 }, { "fixed", "e706b70325d8514d" }, { "u64", "beb52e8ccda550ff" }, { "i64", "bbd129d7317e9c8d" }, { "u32", "000000002ffaede7", 804974055 }, { "fixed", "df905ddd9fa69309" }, { "u64", "34e2cae2b7a42893" }, { "i64", "df7f764dd6f71809" }, { "u32", "00000000fcaa09b3", 4239002035 }, { "fixed", "81be9e358b329d6e" }, { "u64", "a31407677f09f621" }, { "i64", "8f0cb0f7baf87a4f" }, { "u32", "000000004d8c492b", 1301039403 }, { "fixed", "8f6c64d61b373e87" }, { "u64", "a4e595d7404cd470" }, { "i64", "6030cfaf0fd87585" }, { "u32", "00000000a9f252f1", 2851230449 }, { "fixed", "dd45d6f064a6672a" }, { "u64", "2fb395427f736ae5" }, { "i64", "b446c65605b738d4" }, { "u32", "0000000088ff3489", 2298426505 }, { "fixed", "1f0a46619f94e3b6" }, { "u64", "bd2ec9cd90569cf6" }, { "i64", "ef9c631fdcee6855" }, { "u32", "000000002fd14268", 802243176 }, { "fixed", "ecaa517f21fcb673" }, { "u64", "0b8cbf5a53a3c46f" }, { "i64", "5e2c255f457aa40b" }, { "u32", "00000000bb900bb3", 3146779571 }, { "fixed", "b67de7f44472e255" }, { "u64", "9d22833ba18c864a" }, { "i64", "45ac423d75670013" }, { "u32", "000000003e28a820", 1042851872 }, { "fixed", "620a10da62d8550d" }, { "u64", "081e67ed26177a20" }, { "i64", "86a1a02a9af3717b" }, { "u32", "00000000a51f3e83", 2770288259 }, { "fixed", "5babb65a196952d2" }, { "u64", "74936f93718dd32f" }, { "i64", "5b119cd7f46eb9d1" }, { "u32", "000000008453c960", 2220083552 }, { "fixed", "f37845d99f674bc1" }, { "u64", "a386dd03ffc4b471" }, { "i64", "4c963aca32c7905a" }, { "u32", "0000000099220bc5", 2569145285 }, { "fixed", "bb2eb999ffd12ed5" }, { "u64", "3b126a7e09520795" }, { "i64", "60f30b5b1c5a3384" }, { "u32", "00000000a4aa1f6a", 2762612586 }, { "fixed", "2525adbe65ddc815" }, { "u64", "b081e8ac5f571ee5" }, { "i64", "cbde290b62ca86be" }, { "u32", "0000000015259221", 354783777 }, { "fixed", "9753ca4233394dac" }, { "u64", "cdde6a0bb88740a0" }, { "i64", "4ff967a96d27963f" }, { "u32", "000000004cbeb940", 1287567680 }, { "fixed", "357e9bc19940fc31" }, { "u64", "7609d150c346738d" }, { "i64", "3822aafc0c0bfcd1" }, { "u32", "000000004c793125", 1283010853 }, { "fixed", "a8f3e64273679fb9" }, { "u64", "fe253fcd6d8c0486" }, { "i64", "b10d4c1abbc73823" }, { "u32", "0000000059935e3f", 1502830143 }, { "fixed", "f0dee2deab8163f1" }, { "u64", "83569eba28938b64" }, { "i64", "454ef314b8178163" }, { "u32", "0000000010768bd7", 276204503 }, { "fixed", "7ca630930827d9a9" }, { "u64", "1f0afa443970cf85" }, { "i64", "47ffd2fdf4f01bff" }, { "u32", "00000000f7030971", 4144171377 }, { "fixed", "8e22217b6b77ccbb" }, { "u64", "f254bc1fbca98d82" }, { "i64", "6e2062b7b84f170b" }, { "u32", "00000000dc4670fd", 3695603965 }, { "fixed", "d025c10f3a079f90" }, { "u64", "b8059887d06f31b3" }, { "i64", "8b41557024156069" }, { "u32", "000000005a55593c", 1515542844 }, { "fixed", "e73b57198d641fe3" }, { "u64", "343dfdb619b3a2e2" }, { "i64", "289c63aecdd4b3a5" }, { "u32", "0000000092fe041b", 2466120731 }, { "fixed", "940fa8904ae03037" }, { "u64", "6495c9d17a6b9cee" }, { "i64", "6a6b2603b0d4fc64" }, { "u32", "00000000f186d188", 4052144520 }, { "fixed", "eead9645cbb4ec3a" }, { "u64", "582596587d31f759" }, { "i64", "db45af241a912d89" }, { "u32", "0000000014f09414", 351310868 }, { "fixed", "d27b9c22d4a3261c" }, { "u64", "88dd2b8bb42103a4" }, { "i64", "6078b91332af26f8" }, { "u32", "0000000059ed05ea", 1508705770 }, { "fixed", "df70ee8fd092ac64" }, { "u64", "5550805b4c01edad" }, { "i64", "e128cd96b9c52cfe" }, { "u32", "000000005411fc9a", 1410464922 }, { "fixed", "7991e934348bbd62" }, { "u64", "367d7592329fef9b" }, { "i64", "0c71177328307050" }, { "u32", "00000000ab181929", 2870483241 }, { "fixed", "316e9c737a4ff0d9" }, { "u64", "3294e6cd496eed94" }, { "i64", "6e5a469f697104a9" }, { "u32", "00000000752dc2fd", 1965933309 }, { "fixed", "56523ff582139d76" }, { "u64", "54b8380db75ce405" }, { "i64", "d90c6eab980d5dd3" }, { "u32", "00000000149bbb77", 345750391 }, { "fixed", "c703cbc7c4a5506a" } } },}

-- ============================ replay ============================

-- `useNumbers`: when true, ops that carry a decimal are fed to writeI64 /
-- writeFixed as plain Luau numbers; when false, always as opaque Fixed values
-- decoded straight from the hex bit pattern.
local function replay(ops, useNumbers: boolean): string
	local c = Checksum.new()
	for _, op in ipairs(ops) do
		local kind, hex, dec = op[1], op[2], op[3]
		if kind == "u64" then
			c:writeU64(Checksum.fromHex(hex))
		elseif kind == "u32" then
			c:writeU32(tonumber(string.sub(hex, 9, 16), 16))
		elseif kind == "i64" then
			if useNumbers and dec ~= nil then
				c:writeI64(dec)
			else
				c:writeI64(Fixed.fromHex(hex))
			end
		elseif kind == "fixed" then
			if useNumbers and dec ~= nil then
				c:writeFixed(dec)
			else
				c:writeFixed(Fixed.fromHex(hex))
			end
		else
			error("unknown op kind " .. tostring(kind))
		end
	end
	return c:finishHex()
end

for _, case in ipairs(CASES) do
	eq(replay(case.ops, false), case.expect, "case " .. case.name .. " (Fixed path)")
	eq(replay(case.ops, true), case.expect, "case " .. case.name .. " (number path)")
end

-- ============================ direct assertions ============================

-- The FNV-1a 64 offset basis, straight from the Rust.
eq(Checksum.new():finishHex(), "cbf29ce484222325", "empty digest is the offset basis")
do
	local hi, lo = Checksum.new():finish()
	eq(hi, Checksum.OFFSET_HI, "finish() high half")
	eq(lo, Checksum.OFFSET_LO, "finish() low half")
end

-- `write_u32(v)` IS `write_u64(v as u64)` — zero-extended, four zero bytes and all.
for _, v in ipairs({ 0, 1, 2, 255, 256, 65535, 65536, 12345, 54000, 2147483647, 4294967295 }) do
	local a = Checksum.new()
	a:writeU32(v)
	local b = Checksum.new()
	b:writeU64(0, v)
	eq(a:finishHex(), b:finishHex(), "writeU32(" .. v .. ") == writeU64(0, " .. v .. ")")
end

-- `write_fixed(x)` IS `write_i64(x.raw())`.
for _, raw in ipairs({ 0, 1, -1, 65536, -65536, 78643200, -78643200, 2147483648, -2147483648 }) do
	local a = Checksum.new()
	a:writeFixed(Fixed.fromRaw(raw))
	local b = Checksum.new()
	b:writeI64(raw)
	eq(a:finishHex(), b:finishHex(), "writeFixed(" .. raw .. ") == writeI64(" .. raw .. ")")
end

-- Order sensitivity (`determinism`'s own `checksum_order_sensitive` test).
do
	local a = Checksum.new()
	a:writeU64(0, 1)
	a:writeU64(0, 2)
	local b = Checksum.new()
	b:writeU64(0, 2)
	b:writeU64(0, 1)
	checks += 1
	if a:finishHex() == b:finishHex() then
		fail("checksum must be order sensitive")
	end
end

-- hex round-trip.
for _, s in ipairs({
	"0000000000000000",
	"ffffffffffffffff",
	"8000000000000000",
	"7fffffffffffffff",
	"cbf29ce484222325",
	"deadbeefcafef00d",
}) do
	local hi, lo = Checksum.fromHex(s)
	eq(Checksum.toHex(hi, lo), s, "hex round-trip " .. s)
end
eq(Checksum.toHex(Checksum.fromHex("1b3")), "00000000000001b3", "fromHex zero-pads short input")

-- The dot form and the method form are the same function.
do
	local a = Checksum.new()
	Checksum.writeU32(a, 7)
	Checksum.writeI64(a, -7)
	local b = Checksum.new()
	b:writeU32(7)
	b:writeI64(-7)
	eq(a:finishHex(), b:finishHex(), "dot form == method form")
end

-- Verbose mode records one entry per write, each carrying the digest AFTER it.
do
	local v = Checksum.newVerbose()
	v:writeU32(1, "tick")
	v:writeI64(-1, "hp")
	v:writeFixed(Fixed.ONE, "pos.x")
	v:writeU64(0, 2, "rng")
	eq(#v.log, 4, "verbose log length")
	eq(v.log[1].label, "tick", "verbose label")
	eq(v.log[2].kind, "i64", "verbose kind")
	eq(v.log[2].value, "ffffffffffffffff", "verbose value hex")
	eq(v.log[4].digest, v:finishHex(), "verbose last digest == finish")
	local plain = Checksum.new()
	plain:writeU32(1)
	plain:writeI64(-1)
	plain:writeFixed(Fixed.ONE)
	plain:writeU64(0, 2)
	eq(v:finishHex(), plain:finishHex(), "verbose digest == plain digest")
end

-- Guard rails: silently hashing the wrong bits is the failure mode this file
-- exists to prevent, so out-of-domain arguments must raise, not truncate.
local function mustError(f, msg: string)
	checks += 1
	if pcall(f) then
		fail(msg .. ": expected an error")
	end
end
mustError(function()
	Checksum.new():writeU32(-1)
end, "writeU32(-1)")
mustError(function()
	Checksum.new():writeU32(4294967296)
end, "writeU32(2^32)")
mustError(function()
	Checksum.new():writeU32(1.5)
end, "writeU32(1.5)")
mustError(function()
	Checksum.new():writeI64(1.5)
end, "writeI64(1.5)")
mustError(function()
	Checksum.new():writeI64(9007199254740994)
end, "writeI64(2^53 + 2)")

-- ============================ summary ============================

print(string.format("checksum_test: %d checks, %d failures", checks, failures))
if failures > 0 then
	error(string.format("%d checksum check(s) FAILED", failures), 0)
end
