//! Flavor + tip text for every weapon and modifier, keyed by catalog index.
//!
//! Pure data, render/UI-facing only — never feeds the checksum. Mirrors the
//! `&'static str` idiom used for names in [`crate::content`]. The arrays are
//! sized 1:1 with `content::WEAPONS` / `content::MODIFIERS`; a length test below
//! fails loudly if a catalog entry is added without text.

/// `(flavor, tip)` for weapon index `i`. Out of range → `("", "")`.
pub fn weapon_text(i: u16) -> (&'static str, &'static str) {
    WEAPON_TEXT.get(i as usize).copied().unwrap_or(("", ""))
}

/// `(flavor, tip)` for modifier index `i`. Out of range → `("", "")`.
pub fn modifier_text(i: u16) -> (&'static str, &'static str) {
    MODIFIER_TEXT.get(i as usize).copied().unwrap_or(("", ""))
}

/// Indexed 1:1 with [`content::WEAPONS`].
const WEAPON_TEXT: [(&str, &str); 86] = [
    // 0 — Bow (piercing, single target, starter)
    ("Wood, gut, and spite. It outlived the man who carved it, and it'll outlive you.",
     "Single-target piercing. Cheap, honest, ruthless."),
    // 1 — Mortar Launcher (siege splash)
    ("It doesn't hate the crowd. It just doesn't see the difference between them.",
     "Lobbed siege splash. Wide blast, slow reload."),
    // 2 — Frost Bow (magic, frost stacks)
    ("Cold doesn't kill. It just holds them still while everything else does.",
     "Fast magic shots. Stacks frost; slows."),
    // 3 — Poison Bow (piercing, poison DoT)
    ("The arrow's an afterthought. The rot is the whole point.",
     "Weak hit, heavy poison bleed-out."),
    // 4 — Flamecaster (chaos splash, fire stacks)
    ("Light one, light the row. They cook on the way to dying.",
     "Chaos splash. Stacks fire; cooks then bursts."),
    // 5 — Storm Hammer (magic, stun)
    ("One swing. The thunder gets there before the corpse hits the dirt.",
     "Heavy magic hit. Stuns the target."),
    // 6 — Ballista (siege barrage 4)
    ("Aim is for people with fewer bolts.",
     "Siege barrage. Four bolts, four bodies."),
    // 7 — Immolation (chaos area, fire stacks)
    ("Stand close. Let the heat come off you like a confession.",
     "Chaos pulse around the tank. Stacks fire."),
    // 8 — Shockwave Axe (normal wave, heavy)
    ("The ground remembers the swing long after the screaming stops.",
     "Sweeping wave. Big normal hit, everything near."),
    // 9 — Moon Glaive (piercing bounce 4)
    ("Throw it once. It does the rest of the math itself.",
     "Piercing bounce. Chains to four enemies."),
    // 10 — Death Engine (chaos single, self-scaling generator)
    ("It doesn't aim. It doesn't tire. It doesn't ask whose side you're on.",
     "Chaos single-target. Scales with each one owned."),
    // 11 — Magic Missile (normal single)
    ("The first spell anyone learns, and the last thing plenty of things see.",
     "Plain magical bolt. One target, normal damage."),
    // 12 — Boulder (siege single)
    ("Gravity, with extra steps.",
     "Single-target siege. A rock, thrown angry."),
    // 13 — Magic Bolt (magic single)
    ("No fire, no frost. Just a clean hole where the magic went in.",
     "Single-target magic. Plain and quick."),
    // 14 — Chaos Orb (chaos single)
    ("Whatever it's made of, it wasn't meant to leave the vault.",
     "Single-target chaos. Ignores armor types."),
    // 15 — Throwing Axes (piercing bounce 4, short, fast)
    ("Up close, all the time. They land before you finish exhaling.",
     "Short-range piercing bounce. Fast, four hops."),
    // 16 — Chaos Skulls (chaos bounce 4)
    ("They remember whose head they were. They don't care anymore.",
     "Chaos bounce. Skips between four bodies."),
    // 17 — Chaos Heart (chaos wave, heal exotic)
    ("It beats for nobody, and it beats anyway.",
     "Heavy chaos wave. Slow, vampiric upkeep."),
    // 18 — Missile Barrage (piercing barrage 8, epic, stun)
    ("Eight at once. The sky decides this isn't your day.",
     "Epic piercing barrage. Eight hits, all stun."),
    // 19 — Seeker Axe (piercing bounce 4, long)
    ("It finds them. It always finds them. That's the whole feature.",
     "Long-range piercing bounce. Chains four."),
    // 20 — Steam Cannon (siege splash, short, stun)
    ("Pressure builds. Pressure is released. Someone is in the way of the release.",
     "Short siege splash. Brief stun on hit."),
    // 21 — Demon Eye (chaos single, heavy)
    ("It watches. It picks. The picking is fatal.",
     "Heavy chaos single-target. Pure punch."),
    // 22 — Impaler (piercing single, fast, stun)
    ("In, through, and out the back. They're upright out of habit.",
     "Fast piercing single-target. Short stun."),
    // 23 — Chaos Swarm (chaos splash, long)
    ("A cloud of small wrong things, and then quiet.",
     "Long-range chaos splash. Spreads on impact."),
    // 24 — Catapult (siege single, short)
    ("It's a rock, and you're a problem. The math is simple.",
     "Short-range siege single-target. Heavy stone."),
    // 25 — Wind Spear (piercing single, epic, knockback)
    ("The air sharpens, picks a throat, and is gone.",
     "Epic piercing single-target. Enormous hit."),
    // 26 — Crippler (piercing single, rare, permanent)
    ("It doesn't kill the leg. It just ends the leg's career.",
     "Heavy piercing single-target. Lasting wound."),
    // 27 — Lifeleecher (normal single, long, heal)
    ("Every drop it takes, it takes from the wrong end.",
     "Long-range normal single-target. Feeds you."),
    // 28 — Spell Glaive (magic bounce 4)
    ("Sharpened thought. It cuts on the way out and on the way back.",
     "Magic bounce. Carves through four."),
    // 29 — Glaive Thrower (normal bounce 4)
    ("No magic, no excuses. Just steel that won't sit still.",
     "Normal bounce. Four ricochets, no frills."),
    // 30 — Spikewheel Launcher (siege bounce 8, long)
    ("It rolls. It does not stop rolling. Ask the eight behind you.",
     "Long siege bounce. Mows through eight."),
    // 31 — Meatapult (normal splash, stun)
    ("Whatever it was, it's ammunition now. Show some respect, or don't.",
     "Normal splash. Heavy impact, brief stun."),
    // 32 — Arcane Blaster (magic single, stun)
    ("Point. Discharge. The smell lingers; the target doesn't.",
     "Magic single-target. Stuns on hit."),
    // 33 — Quills (piercing single, light poison)
    ("Each one's a little gift that keeps on taking.",
     "Piercing single-target. Light poison bleed."),
    // 34 — Living Spittle (magic single, light poison)
    ("It's still alive when it lands. Briefly. So is the target.",
     "Magic single-target. Trickling poison."),
    // 35 — Poison Bomb (siege splash, poison)
    ("The blast is the polite part.",
     "Siege splash. Poisons the whole crater."),
    // 36 — Serpent (normal single, heavy poison)
    ("It bites once. Once is the appointment; the rest is the wait.",
     "Normal single-target. Strong poison bleed."),
    // 37 — Overloaded Catapult (siege splash, rare)
    ("They told it the safe load. It laughed in stone.",
     "Heavy siege splash. Bigger rock, bigger hole."),
    // 38 — Chaos Claw (chaos single, stun)
    ("Reaches out of nowhere, takes something that mattered.",
     "Chaos single-target. Stuns on hit."),
    // 39 — Net Thrower (normal single, fast, stun)
    ("Can't run if you're already on the floor counting threads.",
     "Fast normal single-target. Tangling stun."),
    // 40 — Thornburst (piercing area, big radius)
    ("The garden's gone feral. Mind the garden.",
     "Wide piercing area pulse. Hits all near."),
    // 41 — Chaotic Spirit (magic bounce 8, epic)
    ("It died once already and took the lesson badly.",
     "Epic magic bounce. Wails through eight."),
    // 42 — Energy Pulse (magic wave, stun)
    ("A clean white nothing, and then a list of names.",
     "Magic wave. Sweeps wide, stuns."),
    // 43 — Cluster Rockets (chaos barrage 12, long)
    ("Twelve apologies, none of them sincere.",
     "Long chaos barrage. Twelve warheads out."),
    // 44 — Frost Bomb (piercing splash, fast, frost)
    ("Shatters warm. Lands cold. Leaves them slow.",
     "Fast piercing splash. Frosts the cluster."),
    // 45 — Bouncy Cannonball (normal bounce 4, stun)
    ("It's having more fun than anyone it hits.",
     "Normal bounce. Four hops, each stuns."),
    // 46 — Soulstealer (normal bounce 4, epic, heal)
    ("It collects. You'd call it greedy if it answered to you.",
     "Epic normal bounce. Reaps and feeds you."),
    // 47 — Splasher (normal splash, very fast)
    ("Cheap, quick, and everywhere at once. Like a rumor.",
     "Rapid normal splash. Constant small bursts."),
    // 48 — Fire Bow (piercing barrage 4, fire)
    ("Four arrows, four small fires, one bad afternoon.",
     "Piercing barrage. Each shot stacks fire."),
    // 49 — Chaos Web (chaos bounce 8, poison)
    ("Strung between corpses. It works either way.",
     "Chaos bounce. Eight links, leaves poison."),
    // 50 — Magic Claw (magic bounce 4, short, mana)
    ("Up close and personal, in the worst dialect of magic.",
     "Short magic bounce. Chains four."),
    // 51 — Liquid Fire Hurler (siege single, very fast, fire)
    ("It pours. It clings. It does not negotiate.",
     "Fast siege single-target. Stacks fire."),
    // 52 — Boulder Toss (normal splash, short, heavy)
    ("Old-fashioned. Devastating. Honest about it.",
     "Short normal splash. Crushing impact."),
    // 53 — Bloody Spikes (normal wave, epic, fast, stun)
    ("The floor grew teeth and an opinion.",
     "Epic normal wave. Fast, ruinous, stuns."),
    // 54 — Inferno Stone (chaos area, epic, huge fire + stun)
    ("Drop it once and the world handles the rest.",
     "Epic chaos pulse. Massive fire, stuns."),
    // 55 — Flame Generator (magic area, epic, huge fire)
    ("It makes fire the way other things make excuses: endlessly.",
     "Epic magic pulse. Buries them in fire stacks."),
    // 56 — Firebreather (piercing splash, fast, fire)
    ("Exhales. The front rank stops attending.",
     "Fast piercing splash. Lays down fire."),
    // 57 — Lavaspitter (siege splash, epic, long, fire)
    ("Spits the earth's bad temper at anything that moves.",
     "Epic long siege splash. Heavy fire stacks."),
    // 58 — Frostbolt (piercing single, fast, frost)
    ("A small cruelty, delivered cold and often.",
     "Fast piercing single-target. Frosts target."),
    // 59 — Living Ice (magic splash, frost)
    ("It crawls before it cracks. Then it's everywhere and it's freezing.",
     "Magic splash. Spreads frost on impact."),
    // 60 — Ice Generator (magic area, epic, long, frost)
    ("It just keeps making winter. Nobody asked it to stop, exactly.",
     "Epic magic pulse. Frosts a huge radius."),
    // 61 — Ice Spears (normal barrage 4, frost)
    ("Four shards. Four shudders. Then the slow part.",
     "Normal barrage. Four frosting hits."),
    // 62 — Knives (piercing barrage 4, fast, cheap)
    ("Cheap, plural, and unsentimental about it.",
     "Fast piercing barrage. Four cheap blades."),
    // 63 — Blaster (siege single, fast)
    ("Loud, blunt, and rarely wrong.",
     "Fast siege single-target. No tricks."),
    // 64 — Bandit Sniper (normal single, fast, long)
    ("Doesn't fight fair. Never claimed to.",
     "Fast long normal single-target. Picks off."),
    // 65 — Bombs (siege splash, short, stun)
    ("Light, lob, look away. Manners.",
     "Short siege splash. Concussive stun."),
    // 66 — Death Coil (chaos single, long)
    ("A black ribbon that finds the soft middle of things.",
     "Long chaos single-target. Withers a target."),
    // 67 — Chaos Skull Bomb (chaos splash, stun)
    ("Somebody's skull, packed with worse.",
     "Chaos splash. Bursts and stuns."),
    // 68 — Icebreather (siege splash, frost)
    ("Breathes out the long cold and the slow death after.",
     "Siege splash. Frosts the blast."),
    // 69 — Frostwave (magic wave, frost)
    ("The cold arrives in a line, like bad news.",
     "Magic wave. Sweeps and frosts."),
    // 70 — Flamewave (normal wave, fire)
    ("A wall of heat that walks toward you on purpose.",
     "Normal wave. Sweeps, stacks heavy fire."),
    // 71 — Chaotic Spirit Bolt (chaos single, very fast, heal)
    ("It flickers out and takes a sliver of someone with it.",
     "Fast chaos single-target. Sips life back."),
    // 72 — Manabolt (magic single, very fast, long, drain)
    ("Spends their magic against them, then spends them.",
     "Fast long magic single-target. Drains."),
    // 73 — Death Generator (chaos single, epic, long, raises)
    ("It makes corpses, then makes them work.",
     "Epic long chaos single-target. Relentless."),
    // 74 — Immolation Aura (magic wave, very fast, fire)
    ("Stand inside the burn. Let it be everyone else's problem.",
     "Rapid magic wave. Constant fire near tank."),
    // 75 — Goblin Land Mines (siege wave, epic, fast, stun)
    ("The goblin who set them is also gone. Worth it, apparently.",
     "Epic siege wave. Fast, brutal, stuns."),
    // 76 — Quill Burst (piercing splash, long)
    ("One bristling instant, then a quiet field of pincushions.",
     "Long piercing splash. Sprays the cluster."),
    // 77 — Arcane Burst (magic splash, long)
    ("A clean detonation of pure intention.",
     "Long magic splash. Blasts on impact."),
    // 78 — Meteor Barrage (siege barrage 8, epic, very slow)
    ("The sky pays its debts all at once.",
     "Epic siege barrage. Eight meteors, long wait."),
    // 79 — Ale Launcher (siege splash, fast, heal)
    ("A round on the house. The house burns down.",
     "Fast siege splash. Heals you on the side."),
    // 80 — Chaos Bolt (chaos single, very fast, long)
    ("Fast, mean, and unbothered by your armor.",
     "Fast long chaos single-target. Ignores armor."),
    // 81 — Rotating Orb of Lightning (magic area, huge radius)
    ("It circles. Standing near it is a personal decision.",
     "Huge magic pulse. Shocks everything around."),
    // 82 — Lightning Generator (magic single, long, mana)
    ("It hums. Things near it stop humming.",
     "Long magic single-target. Steady arc."),
    // 83 — Flame Nova (chaos area, fire)
    ("Blooms outward, all heat and no mercy.",
     "Chaos pulse. Erupts fire around the tank."),
    // 84 — Shocker (siege area, very fast, long, stun)
    ("Tap, tap, tap. Nobody gets to move between the taps.",
     "Rapid long siege pulse. Chronic stun-lock."),
    // 85 — Entangler (normal single, epic, very fast, root)
    ("Roots them where they stand and keeps them company.",
     "Fast epic normal single-target. Pins down."),
];

/// Indexed 1:1 with [`content::MODIFIERS`].
const MODIFIER_TEXT: [(&str, &str); 84] = [
    // 0 — +10% Damage (global)
    ("Hit harder. There's no second lesson.",
     "+10% to all weapon damage."),
    // 1 — +10% Piercing Damage
    ("Sharper is just meaner with better posture.",
     "+10% piercing damage."),
    // 2 — +10% Siege Damage
    ("Walls were a suggestion. So were bodies.",
     "+10% siege damage."),
    // 3 — +10% Magic Damage
    ("Borrow more from the things that should stay asleep.",
     "+10% magic damage."),
    // 4 — +25% Damage (Epic, multiplicative)
    ("Why nudge the numbers when you can break them?",
     "+25% damage, multiplicative on top."),
    // 5 — +10% Attack Speed
    ("Reload is a state of mind. Lose it.",
     "+10% attack speed. Shorter cooldowns."),
    // 6 — +50% Kill Bounty
    ("Corpses pay better when you ask nicely with volume.",
     "+50% gold per kill."),
    // 7 — +20 Gold Income
    ("Greed is the only god that pays out.",
     "+20 gold income per round."),
    // 8 — +10% Gold Income (pct)
    ("Skim a little off every coin. It adds up. It always adds up.",
     "+10% to passive income."),
    // 9 — +25% Gold Income (pct)
    ("More than a skim. A confident bite.",
     "+25% to passive income."),
    // 10 — Golden Vitality (25% income as HP regen)
    ("Money can't buy health. This is the loophole.",
     "Heals 25% of income each tick."),
    // 11 — Lucky Strikes (5% chance: +200% bounty)
    ("Most kills pay scale. Some kills pay rent.",
     "5% of kills pay triple bounty."),
    // 12 — +2000 Max HP
    ("More wall between you and the inevitable.",
     "+2000 max HP."),
    // 13 — +10 Armor
    ("Let it bounce off something for once.",
     "+10 flat armor. Shaves every hit."),
    // 14 — +2000 Mana Shield
    ("A second skin made of borrowed math.",
     "+2000 mana shield, slow regen."),
    // 15 — +50 HP Regen
    ("Stitch faster than they cut.",
     "+50 HP regen per tick."),
    // 16 — +10% Dodge
    ("Not there is the best armor there is.",
     "+10% chance to dodge a hit."),
    // 17 — Building Power (+2% dmg, +1%/round, ramp)
    ("Patience is a weapon. So is everything you build on it.",
     "+2% damage now, +1% every round."),
    // 18 — Escalating Chaos (+20% chaos, +3%/round, ramp)
    ("It feeds on the clock. Don't make it wait long.",
     "+20% chaos now, +3% every round."),
    // 19 — Compounding Greed (+10 income, +5/round, ramp)
    ("Today's coin breeds tomorrow's pile.",
     "+10 income now, +5 every round."),
    // 20 — Hardening (+10 armor, +5/round, ramp)
    ("Scar tissue, by appointment.",
     "+10 armor now, +5 every round."),
    // 21 — +25% Single-Target Damage
    ("Pick one. Mean it.",
     "+25% to single-target weapons."),
    // 22 — +25% Splash Damage
    ("Why ruin one when the crater's already dug?",
     "+25% to splash weapons."),
    // 23 — +25% Barrage Damage
    ("More of everything, harder.",
     "+25% to barrage weapons."),
    // 24 — +25% Area Damage
    ("The whole room signed up. The whole room pays.",
     "+25% to area-pulse weapons."),
    // 25 — +25% Wave Damage
    ("Push the front rank into the next world louder.",
     "+25% to wave weapons."),
    // 26 — +25% Bounce Damage
    ("Each hop hits a little angrier.",
     "+25% to bounce weapons."),
    // 27 — +25% Short-Range Damage
    ("Up close, where the work is honest.",
     "+25% to short-range weapons (<=600)."),
    // 28 — +25% Long-Range Damage
    ("Kill them before they smell you.",
     "+25% to long-range weapons (>=900)."),
    // 29 — +100% Common Weapon Damage
    ("The cheap junk earns its keep, twice over.",
     "Doubles common-rarity weapon damage."),
    // 30 — +20% Damage to Stunned
    ("Kick them while they're down. That's what down is for.",
     "+20% damage to stunned enemies."),
    // 31 — +25% Damage to Poisoned
    ("Finish what the rot started.",
     "+25% damage to poisoned enemies."),
    // 32 — +10% Poison Damage
    ("Make the slow death less slow.",
     "+10% applied poison damage."),
    // 33 — +50% Stun Duration
    ("Keep them on the floor a little longer.",
     "+50% stun duration."),
    // 34 — +80 Spikes Damage
    ("Touch the tank, lose a hand.",
     "+80 retaliation damage when hit."),
    // 35 — +300 Spikes Damage
    ("Touch the tank, lose the argument.",
     "+300 retaliation damage when hit."),
    // 36 — +50% Spikes Damage
    ("Sharpen the punishment.",
     "+50% to all spikes damage."),
    // 37 — Bloody Spikes (+80, +10/round, ramp)
    ("Every round, the welcome gets less welcoming.",
     "+80 spikes now, +10 every round."),
    // 38 — Vulnerability Pulse (aura)
    ("Mark them all. The flesh remembers being marked.",
     "Nearby enemies take +5% damage/sec."),
    // 39 — +15 Heal on Kill
    ("Their last breath, your next.",
     "+15 HP each time an enemy dies."),
    // 40 — +60 Heal on Kill
    ("A feast, one corpse at a time.",
     "+60 HP each time an enemy dies."),
    // 41 — Vampiric Spores (+5 HP/poison tick)
    ("The rot feeds two mouths now.",
     "+5 HP every poison tick you deal."),
    // 42 — Magic Coin (+3 copies of next Common)
    ("Flip it. Wish small. It pays small, three times.",
     "Next common bought yields 3 free copies."),
    // 43 — Duplicator (+1 copy of next Rare)
    ("Why own one good thing?",
     "Next rare bought yields 1 free copy."),
    // 44 — Black Market (next Uncommon free)
    ("Everything's for sale. Some things twice, one free.",
     "Next uncommon purchase is free."),
    // 45 — Magic Treasure (+250 gold, +5 income/round, ramp)
    ("Buried money, dug up and put to work.",
     "+250 gold now, +5 income every round."),
    // 46 — Ankh of Reincarnation (revive, +2000 max HP)
    ("Death filed the paperwork early. It got rejected. Once.",
     "Revive once on lethal hit, +2000 max HP."),
    // 47 — +25% Healing
    ("Bleed slower, mend faster.",
     "+25% to all healing received."),
    // 48 — Regeneration (1.5% missing HP/sec)
    ("The closer to dead, the harder it claws back.",
     "Heals 1.5% of missing HP each second."),
    // 49 — +1% Piercing Damage per Bow
    ("Every bow you hoard sharpens the rest.",
     "+1% piercing per Bow owned."),
    // 50 — +1% Siege Damage per Mortar
    ("A choir of mortars sings louder together.",
     "+1% siege per Mortar Launcher owned."),
    // 51 — Overclocked Death Engine (+10% Chaos per Death Engine)
    ("Feed the machine more of itself. It likes that.",
     "+10% chaos per Death Engine owned."),
    // 52 — Blood Pact (-1000 max HP, +2000 gold)
    ("Sell the meat. Buy the means.",
     "-1000 max HP, +2000 gold. No going back."),
    // 53 — Last Rites (-100 HP regen, +5000 gold)
    ("Stop healing. Start hoarding. Pray it's enough.",
     "-100 HP regen, +5000 gold."),
    // 54 — Bloodmoney (+1 gold per 100 damage)
    ("Every wound you open is a coin you keep.",
     "+1 gold per 100 damage dealt."),
    // 55 — Bloodmoney II (+1 gold per 20 damage)
    ("The slaughter pays five times better now.",
     "+1 gold per 20 damage dealt."),
    // 56 — Wartithe (25% of income as mana shield)
    ("Tithe to the shield. The shield keeps you solvent.",
     "Income tops up your mana shield, 25%."),
    // 57 — Aegis Protocol (scaling mana shield)
    ("Stolen math, and it learns more every round.",
     "+2500 shield, +400 more each round."),
    // 58 — +500 Max HP
    ("A little more give before you break.",
     "+500 max HP."),
    // 59 — +10% Piercing Damage (gen dup)
    ("Hone the point that little bit further.",
     "+10% piercing damage."),
    // 60 — +10% Normal Damage
    ("Hit plain. Hit hard.",
     "+10% normal damage."),
    // 61 — +10% Siege Damage (gen dup)
    ("Make the rubble finer.",
     "+10% siege damage."),
    // 62 — +10% Chaos Damage
    ("Pour more of the wrong stuff in.",
     "+10% chaos damage."),
    // 63 — +1000 Max HP
    ("More buffer between you and the dirt.",
     "+1000 max HP."),
    // 64 — +10 Armor (gen dup)
    ("Let more of it glance off.",
     "+10 flat armor."),
    // 65 — +2000 Max HP (rare)
    ("A serious wall, for serious trouble.",
     "+2000 max HP."),
    // 66 — +50% Kill Bounty (rare)
    ("The dead are generous when there are this many.",
     "+50% gold per kill."),
    // 67 — +2000 Mana Shield (rare)
    ("More borrowed skin to throw away first.",
     "+2000 mana shield, slow regen."),
    // 68 — +20 Gold Income (gen dup)
    ("Steady coin for steady greed.",
     "+20 gold income per round."),
    // 69 — +5 Armor
    ("A thin shave off every blow.",
     "+5 flat armor."),
    // 70 — +10% Attack Speed (gen dup)
    ("Shave the pause between killings.",
     "+10% attack speed."),
    // 71 — +80 HP Regen
    ("Knit it back faster than they tear it.",
     "+80 HP regen per tick."),
    // 72 — +20 HP Regen
    ("A slow, stubborn mending.",
     "+20 HP regen per tick."),
    // 73 — +5 Gold Income
    ("A trickle. Trickles fill buckets.",
     "+5 gold income per round."),
    // 74 — +10 Gold Income
    ("A little more in the pile every round.",
     "+10 gold income per round."),
    // 75 — +100% Kill Bounty
    ("Twice paid for the same dead men.",
     "+100% gold per kill."),
    // 76 — +10% Magic Damage (gen dup)
    ("Lean harder on the things that bite back.",
     "+10% magic damage."),
    // 77 — +40 HP Regen
    ("Mend at a respectable clip.",
     "+40 HP regen per tick."),
    // 78 — +10000 Mana Shield (epic)
    ("An obscene coat of borrowed life.",
     "+10000 mana shield, fast regen."),
    // 79 — +10% Dodge (gen dup)
    ("One in ten swings hits the air and stays mad.",
     "+10% chance to dodge a hit."),
    // 80 — Escalating Plunder (scaling bounty)
    ("Greed with interest. The bodies pay more each round.",
     "+100% kill gold, +15% more each round."),
    // 81 — Living Fortress (scaling max HP)
    ("Meat becomes wall becomes mountain. Keep chewing.",
     "+2500 max HP, +500 more each round."),
    // 82 — +1000 Mana Shield
    ("A thin coat of borrowed math.",
     "+1000 mana shield, slow regen."),
    // 83 — Mending Engine (scaling regen)
    ("It stitches faster the longer the war drags on.",
     "+120 HP/tick regen, +30 more each round."),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content;

    #[test]
    fn descriptions_cover_every_catalog_entry() {
        assert_eq!(
            WEAPON_TEXT.len(),
            content::WEAPONS.len(),
            "WEAPON_TEXT length must match content::WEAPONS"
        );
        assert_eq!(
            MODIFIER_TEXT.len(),
            content::MODIFIERS.len(),
            "MODIFIER_TEXT length must match content::MODIFIERS"
        );
    }

    #[test]
    fn descriptions_out_of_range_is_empty() {
        assert_eq!(weapon_text(u16::MAX), ("", ""));
        assert_eq!(modifier_text(u16::MAX), ("", ""));
    }
}
