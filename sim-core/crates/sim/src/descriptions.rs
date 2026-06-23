//! Flavor + mechanical tips for every weapon and modifier, keyed by catalog
//! index. Lore-written (the Pale Wardens, the Ninth Foundry, the Conclave, the
//! fall of Ashmark). Surfaced as shop tooltips; never touches the sim.

/// `(flavor, tip)` for weapon index `i`; `("", "")` out of range.
pub fn weapon_text(i: u16) -> (&'static str, &'static str) {
    WEAPON_TEXT.get(i as usize).copied().unwrap_or(("", ""))
}

/// `(flavor, tip)` for modifier index `i`; `("", "")` out of range.
pub fn modifier_text(i: u16) -> (&'static str, &'static str) {
    MODIFIER_TEXT.get(i as usize).copied().unwrap_or(("", ""))
}

const WEAPON_TEXT: [(&str, &str); 86] = [
    ("Warden issue, third pattern; the stock is notched once for every gunner the Pale Wardens lost holding this same dirt.", "Single-target piercing. The first thing they hand you, and often the last."),  // 0
    ("Ninth Foundry overrun, its sights still ground for a wall that no longer stands at Ashmark.", "Lobbed siege splash. Wide blast, slow to reseat."),  // 1
    ("The cold it throws is the same cold that took Ashmark in one night; the Hexwrights only learned to aim it afterward.", "Fast single-target magic. Stacks frost toward the freeze."),  // 2
    ("Greycoat field-rations went bad in the same crates that carried these tips, and the quartermaster shipped both anyway.", "Single-target piercing. Light hit, heavy poison rot."),  // 3
    ("Foundry-work turned outward; the men who pour the cannon learned long ago that fire spreads down a packed column.", "Short-range chaos splash. Stacks fire that cooks, then bursts on death."),  // 4
    ("The Hexwrights bound a storm into the head and never told the Wardens how to put it down.", "Heavy single-target magic. Long stun on the target."),  // 5
    ("Stamped at the Ninth Foundry, four-bolt pattern, built for a frontier that needed walls cleared faster than it had men.", "Long-range siege barrage. Four bolts to four targets."),  // 6
    ("Wardens who stood too long beside a failing reactor learned to wear the burn; this makes a weapon of it.", "Chaos area pulse around the tank. Stacks fire on everything near."),  // 7
    ("Salvaged off the Ashmark sappers, who swung once and let the ground carry the rest of the argument.", "Point-blank normal wave. Big sweeping hit, nothing applied."),  // 8
    ("A Hexwright trinket that does its own targeting; the Conclave swears it was never meant to leave the vault.", "Single-target piercing chain. Bounces to four enemies."),  // 9
    ("The Foundry built one and then could not stop building it; each copy makes the next cheaper to feed.", "Single-target chaos. Damage compounds per Death Engine owned."),  // 10
    ("First rite the Conclave teaches its apprentices, and the last thing a great many things ever saw.", "Single-target magic. Plain bolt, no status."),  // 11
    ("A rock the Greycoats no longer had to carry, thrown at a problem they no longer had to name.", "Single-target siege. Heavy stone, no status."),  // 12
    ("Conclave-cut and clean, with none of the frost or fire the Hexwrights add when they want to be remembered.", "Single-target magic. Quick, plain hit."),  // 13
    ("Whatever the Conclave sealed in here predates the war, and it leaves armor unconvinced.", "Single-target chaos. Ignores armor type."),  // 14
    ("Greycoat sidearms, three to a belt, meant for the work that happens after the line already broke.", "Short-range piercing chain. Fast, bounces to three."),  // 15
    ("The Conclave renders down the heads of its own dead and stuffs them with worse; they remember whose they were.", "Chaos chain. Bounces between three bodies."),  // 16
    ("Suckula sleeps all day and feasts all night; every bite it lands comes straight back to you as a little more life.", "Heavy point-blank chaos wave. Slow, vampiric on the base item."),  // 17
    ("Eight tubes off the Ashmark batteries, recovered from a position that fired this volley once and was overrun anyway.", "Epic long-range piercing barrage. Eight bolts, each stuns."),  // 18
    ("A Warden glaive cut to find its mark across the open ground command never reinforced.", "Long-range piercing chain. Bounces to three."),  // 19
    ("Ninth Foundry pressure-rig, the kind that scalds the loader as readily as the target.", "Short-range siege splash. Brief stun on hit."),  // 20
    ("The Conclave does not say where this eye came from, only that it chooses, and the choosing is final.", "Heavy single-target chaos. Pure punch, no status."),  // 21
    ("Warden spearwork, made for the press at the wire where there is no room to miss.", "Fast single-target piercing. Short stun."),  // 22
    ("A Hexwright cloud of small wrong things, loosed long over the Ashmark dead and quiet after.", "Long-range chaos splash. Spreads on impact."),  // 23
    ("Foundry stonework, short-ranged and unsubtle, built for a line that had run out of subtler answers.", "Short-range siege single-target. Heavy stone."),  // 24
    ("One enormous open-palm Slap, delivered with the full indignation of a creature that was having a perfectly nice day.", "Epic single-target piercing. Enormous hit, no status."),  // 25
    ("The Greycoats took to calling this one mercy and kept walking; whatever it touches stops being a leg.", "Heavy single-target piercing. Lasting wound on the base item."),  // 26
    ("A Warden exotic that takes from the wrong end of the wound and gives it back to the gunner.", "Long-range normal single-target. Feeds you on the base item."),  // 27
    ("Conclave-honed thought given an edge, cutting outbound and again on the return.", "Magic chain. Carves through four."),  // 28
    ("No Hexwright trick on this one, just Foundry steel that refuses to lie where it lands.", "Normal chain. Bounces to three."),  // 29
    ("Spikewheel off the Ashmark works, set rolling once and never minded the men behind it.", "Long-range siege chain. Bounces through six."),  // 30
    ("Whatever the Greycoats could not bury, they loaded; the ledger lists it as ammunition.", "Normal splash. Heavy impact, brief stun."),  // 31
    ("Conclave hardware that discharges clean and leaves the smell on the air longer than the target.", "Single-target magic. Stuns on hit."),  // 32
    ("Barbs the Greycoats scavenged off something the Conclave should have burned, each one a small lasting debt.", "Single-target piercing. Light poison rot."),  // 33
    ("A Hexwright thing that lands still living, briefly, and leaves the target the same way.", "Single-target magic. Trickling poison."),  // 34
    ("Foundry shell packed over a Greycoat sickness, so the crater goes on killing after the blast is done.", "Siege splash. Poisons the whole crater."),  // 35
    ("One bite, by a thing the Conclave bred and refuses to account for; the rest is only the waiting.", "Single-target normal. Strong poison rot."),  // 36
    ("They told it the safe load at the Ninth Foundry, and the catapult overruled them in stone.", "Heavy siege splash. Bigger rock, bigger hole."),  // 37
    ("A Conclave reach out of nowhere that closes on whatever the gunner could least spare.", "Single-target chaos. Stuns on hit."),  // 38
    ("Greycoat netting, weighted and thrown, because a thing on the floor counting threads is a thing not at the wire.", "Fast long-range normal single-target. Tangling stun."),  // 39
    ("The Hexwrights seeded this ground and walked away; what grew has its own opinion about trespass.", "Wide piercing area pulse. Hits everything near."),  // 40
    ("A Conclave spirit that died once already and took the lesson badly, loosed to chain through a column.", "Epic magic chain. Wails through eight."),  // 41
    ("A Hexwright reactor's exhale, a clean white nothing the Wardens learned to point downrange.", "Point-blank magic wave. Sweeps wide, stuns."),  // 42
    ("A full rack of Ninth Foundry warheads, every casing serial-stamped and none of them aimed at anyone in particular.", "Long chaos barrage. Twelve warheads to scattered targets, no status."),  // 43
    ("The Hexwrights bottled the night Ashmark froze and packed it behind a fuse.", "Fast piercing splash. Stacks frost on the cluster."),  // 44
    ("A Foundry reject that never machined round, so the Greycoats let it loose to crack heads instead.", "Normal bounce. Four hops, each stuns."),  // 45
    ("The Pale Wardens cut this from the field after the Relief failed to come, and it has been hungry since.", "Epic normal bounce. Chains four; feeds the tank."),  // 46
    ("Foundry-town surplus by the crateload, cheap enough that the quartermaster stopped counting them.", "Fast normal splash. Rapid small bursts, no status."),  // 47
    ("Warden arrows wrapped in Foundry pitch, lit one rank at a time the way the line always burns.", "Piercing barrage. Four shots, each stacks fire."),  // 48
    ("The Conclave strung this between the Ashmark dead and called the rot it leaves a side effect.", "Chaos bounce. Six links; leaves poison."),  // 49
    ("Hexwright work meant for close quarters, borrowing more power than the gunner can hold.", "Short magic bounce. Chains four."),  // 50
    ("Ladled straight from a Ninth Foundry crucible and slung before it cools.", "Fast siege single-target. Stacks fire."),  // 51
    ("Catapult ammunition the Greycoats stripped from Ashmark's broken walls and lobbed back.", "Short normal splash. Crushing impact, no status."),  // 52
    ("The Wardens worked these into the dirt of the last yard the night before they were overrun.", "Epic normal wave. Heavy sweep; stuns."),  // 53
    ("Shroom Doom seeds a whole patch of angry caps; they pop in a chain, and what survives the blast gets up as your Spores.", "Epic chaos pulse. Massive fire stacks; stuns."),  // 54
    ("The Ninth Foundry never meant for one furnace to feed a whole field, but the war stopped asking.", "Epic magic pulse. Buries a pack in fire stacks."),  // 55
    ("Standard Foundry issue for the front rank, where the line meets the dark and rarely holds.", "Fast piercing splash. Stacks fire."),  // 56
    ("The Foundry-towns tap the deep crucibles for this, and what it spits keeps burning long after Ashmark's example.", "Epic long siege splash. Heavy fire stacks."),  // 57
    ("A single sliver of the cold that took Ashmark, fired again and again until something stops.", "Fast piercing single-target. Stacks frost."),  // 58
    ("Conclave frost that does not wait to be aimed; it spreads the way the Ashmark winter spread.", "Magic splash. Spreads frost on impact."),  // 59
    ("The Hexwrights bound a piece of that killing winter to a frame and could never make it stop.", "Epic magic pulse. Frosts a huge radius toward freeze."),  // 60
    ("Ashmark-cold shards racked by the Greycoats, four to a volley, ledgered by the dozen.", "Normal barrage. Four hits, each stacks frost."),  // 61
    ("The first thing the quartermaster hands a green Greycoat, and the cheapest line in the ledger.", "Fast piercing barrage. Three blades, no status."),  // 62
    ("Blunt Foundry overrun, loud and serial-stamped, sold by weight to the Wardens.", "Fast siege single-target. No status."),  // 63
    ("A Greycoat deserter's rifle, recovered from Ashmark with the scope still sighted on a fleeing back.", "Fast long normal single-target. No status."),  // 64
    ("Ninth Foundry charges that the Greycoats lob underhand and never look back at.", "Short siege splash. Brief stun."),  // 65
    ("A single precise Sting from a very small, very offended thing — and suddenly the target is taking everyone's hits much harder.", "Long chaos single-target. No status."),  // 66
    ("Somebody's skull, packed by the Greycoats with worse and stamped for the line.", "Chaos splash. Bursts and stuns."),  // 67
    ("Foundry casings charged with Ashmark's leftover cold, breathed out across the crater.", "Siege splash. Frosts the blast."),  // 68
    ("The Conclave sends the cold out in a line now, the way the news of Ashmark traveled the front.", "Magic wave. Sweeps and frosts."),  // 69
    ("A wall of Foundry-fire walked forward across the dirt, the way the line is supposed to and no longer can.", "Normal wave. Sweeps; heavy fire stacks."),  // 70
    ("A Hexwright spark that flickers out and carries a sliver of the gunner back with each kill.", "Fast chaos single-target. Heals the tank."),  // 71
    ("The Conclave taught it to drink an enemy's borrowed power before it spends the enemy.", "Fast long magic single-target. No status."),  // 72
    ("Squirm keeps a wriggling brood in reserve; every kill it lands hatches another hungry Larva to join the march.", "Epic long chaos single-target. No status."),  // 73
    ("Warden field-rig that wraps the tank in Foundry heat and makes the burn everyone else's problem.", "Rapid magic wave. Constant fire near the tank."),  // 74
    ("Boom Bloom scatters a tidy little garden of pop-when-touched buds across the last yard, and the front rank finds every one.", "Epic siege wave. Fast, brutal; stuns."),  // 75
    ("A Warden quill-rig recovered off Ashmark, sprung once across a whole closing cluster.", "Long piercing splash. Sprays the cluster, no status."),  // 76
    ("Pure Conclave intention let off all at once, the kind of borrowing the Hexwrights regret in the morning.", "Long magic splash. Detonates on impact."),  // 77
    ("The sky pays the Ninth Foundry's debts all at once, then makes the line wait a long time for the next installment.", "Epic siege barrage. Eight meteors; long reload."),  // 78
    ("A keg the Greycoats meant for after the Relief came, lit and thrown instead.", "Fast siege splash. Heals the tank."),  // 79
    ("Quick mean Conclave work that goes through armor like it was never logged.", "Fast long chaos single-target. Ignores armor types."),  // 80
    ("Hexwright lightning bound to circle the tank, and standing near it is the gunner's own decision.", "Wide magic pulse. Shocks everything around."),  // 81
    ("The Conclave wired this to hum off the enemy's own borrowed mana, and things near it stop humming.", "Long magic single-target. No status."),  // 82
    ("Foundry-fire let off in a bloom, the way the front rank goes when the line finally breaks.", "Chaos pulse. Erupts fire around the tank."),  // 83
    ("A Ninth Foundry coil that taps fast enough to leave no gap between the taps.", "Rapid long siege pulse. Near-constant stun-lock."),  // 84
    ("Tangle throws out a snarl of sticky vine that pins a thing right where it stands and politely refuses to let go.", "Fast epic normal single-target. Roots the target in place."),  // 85
];

const MODIFIER_TEXT: [(&str, &str); 91] = [
    ("Foundry stamps this kit \"general issue,\" the catch-all crate the Wardens hand out when nothing specialized survived the haul from Ashmark.", "+10% to all weapon damage."),  // 0
    ("Hexwrights ground these heads to a finer point than the Foundry would risk, and the Wardens never asked how.", "Improved Piercing Attacks: +10% piercing damage."),  // 1
    ("Recovered from the Ashmark siege-works, calibrated for walls that fell the night the line broke.", "Improved Siege Attacks: +10% siege damage."),  // 2
    ("The Conclave bottles a little more of what it borrows, and signs none of it.", "Improved Magic Attacks: +10% magic damage."),  // 3
    ("Foundry overrun stamped EPIC and locked in the deep vault; the Wardens were told it would never leave Ashmark.", "+25% damage, multiplicative on top."),  // 4
    ("A Hexwright governor wired into the firing chain, shaving the pause the Foundry built in for safety.", "Rapidfire: +10% attack speed."),  // 5
    ("The quartermaster's ledger pays a premium per body, settled out of whatever the Greycoats strip off the field.", "Bounty Hunter: +50% gold per kill."),  // 6
    ("A Hexwright-tapped seam that pays the gunner in coin and in mending both, the ledger and the surgeon settled from one vein.", "Entangled Gold Mine: +20 income, 25% of it healed."),  // 7
    ("The quartermaster skims a margin off every coin the Greycoats turn in, and the ledger never forgets.", "+10% to passive income."),  // 8
    ("A heavier cut, written into the ledger by a quartermaster who stopped pretending the conscripts would be paid.", "+25% to passive income."),  // 9
    ("The quartermaster keeps a crooked ledger; now and then a body settles for far more than its line was worth, and pays the premium too.", "Transmute: +100% bounty; 5% of kills pay triple."),  // 10
    ("Warden masons doubled the wall on the wall they already had, the way a siege teaches you to never stop pouring stone.", "Imbued Masonry: +2000 max HP, then +25%."),  // 11
    ("Foundry hull-shave, rolled too thin in the rush but thick enough to turn what the Relief left you to face.", "+10 flat armor. Shaves every hit."),  // 12
    ("A Hexwright spring fed into the hull, the borrowed skin topping itself off the way a well does.", "Moonwell: +2000 mana shield, regenerating."),  // 13
    ("The Relief that never came included field-surgeons; the Wardens improvised this drip in their absence.", "+50 HP regen per tick."),  // 14
    ("A Warden's last trick when the line is held and the Relief is a rumor: be where the blow is not.", "Evasion: +10% chance to dodge a hit."),  // 15
    ("The longer the war drags on, the heavier the Foundry's old governor leans into the firing chain.", "Power Generator: +2% damage now, +1% every round."),  // 16
    ("The Conclave warned that borrowed chaos compounds with every round it sits unrepaid, and The Hippocrate is still on the horizon.", "Scroll of Chaos: +20% chaos now, +3% every round."),  // 17
    ("The quartermaster's ledger breeds on itself; each round the Greycoats turn in more, and the line grows fatter.", "+10 income now, +5 every round."),  // 18
    ("Scar over scar, the way a Warden's plating thickens every round the Relief fails to arrive.", "Blessed Armor: +10 armor now, +5 every round."),  // 19
    ("Warden marksmanship doctrine, the kind drilled into gunners told to make every single shot count.", "Focusfire: +25% to single-target weapons."),  // 20
    ("Foundry overpressure tuning meant for the crater-makers, ground for walls and the crowds behind them.", "+25% to splash weapons."),  // 21
    ("Greycoat volley discipline, salvaged off a unit that fired in ranks until the ranks ran out.", "+25% to barrage weapons."),  // 22
    ("A Hexwright field-binding that thickens whatever the tank pulses out into the ring of dirt around it.", "+25% to area-pulse weapons."),  // 23
    ("Calibration recovered from the Ashmark breakwalls, meant to shove a whole front rank into the next world.", "Wavefire: +25% to wave weapons."),  // 24
    ("Foundry ricochet-work, each hop tuned to land angrier than the last off scrap nobody else would salvage.", "+25% to bounce weapons."),  // 25
    ("Warden close-line doctrine, where the work is done at knife-reach and the gunner smells what he kills.", "Command Aura: +25% to short-range (<=600)."),  // 26
    ("A long-glass sighting kit off an Ashmark sniper-nest, for killing them before they have a face.", "Trueshot Aura: +25% to long-range (>=900)."),  // 27
    ("The Foundry's cheapest overruns, the junk the Greycoats are handed first, made to earn twice its serial.", "Engineering Upgrade: doubles common-weapon damage."),  // 28
    ("Warden execution drill: a thing on the ground and stunned is a thing the line stops counting.", "Bash: +20% damage to stunned enemies."),  // 29
    ("Finish what the field-rations and the rot already started in them.", "Corrosive Poison: +25% damage to poisoned."),  // 30
    ("Greycoat sickness spread through Ashmark in the bad winters; the Conclave learned to concentrate it.", "Potent Poison: +10% applied poison damage."),  // 31
    ("A Hexwright lock that holds the stunned a breath longer, long enough for the Wardens to work.", "Dazing Stuns: +50% stun duration."),  // 32
    ("Field-improvised cruelty: Ashmark scrap-iron driven point-out into the tank's hull, set so the swing that lands draws a little back.", "Dreadlord Fang: +80 retaliation when hit."),  // 33
    ("A denser hide of welded scrap, the kind a Warden bolts on when reaching the tank should cost a limb.", "+300 retaliation damage when hit."),  // 34
    ("The same improvised barbs, ground keener by a gunner with nothing left to do but sharpen the punishment.", "+50% to all spikes damage."),  // 35
    ("Every round the war drags on, the Wardens hammer fresh scrap into the hull and the welcome gets crueler.", "Growing Spikes: +80 now, +10 every round."),  // 36
    ("A Hexwright marking-rite that opens the flesh of everything near; the marked do not heal what is opened.", "Vulnerability Totem: nearby enemies take +5%/sec."),  // 37
    ("The Relief never came, so the Wardens took their mending off the dying and bolted fresh plate while they were at it.", "Mask of Death: +1000 max HP, +15 HP per kill."),  // 38
    ("A deeper draught of the same grim arithmetic, each corpse on the line paying back the Relief's debt.", "+60 HP each time an enemy dies."),  // 39
    ("Conclave rot-spores feed two mouths at once, the dying enemy's and the gunner the Relief abandoned.", "Reanimating Poison: +5 HP every poison tick."),  // 40
    ("Three flips of a Greycoat token, and the quartermaster's ledger pays out in triplicate before anyone checks the column twice.", "Multiplication Gems: next common yields 3 copies."),  // 41
    ("The Conclave keeps a second of everything worth having, off the ledger and out of the quartermaster's count.", "Duplicator: next rare bought yields 1 free copy."),  // 42
    ("A name in the right margin of the ledger, and the Greycoats look the other way once.", "Black Market: next uncommon purchase is free."),  // 43
    ("Coin the Greycoats buried at Ashmark before the line broke, dug up and still drawing interest.", "Magic Treasure: +250 gold now, +5 income/round."),  // 44
    ("The Wardens buried this rite with their last chaplain; it answers once, and resents being asked.", "Ankh of Reconstruction: revive once, +2000 max HP."),  // 45
    ("What field-dressing the Wardens have left, doled out by a surgeon who stopped counting the dead at Ashmark.", "Healing Hand: +25% to all healing received."),  // 46
    ("Hexwright greenwood grown through the hull, more wall and slower mending the closer to death the gunner runs.", "Living Wood: +2000 max HP, 1.5% missing HP/sec."),  // 47
    ("Every Warden bow racked beside this one lends its draw to the next; the whole rack of them sights truer for the hoard.", "Enchanted Moon Arrow: +100% piercing, +1% per Bow."),  // 48
    ("Ranged together, Foundry mortars find the old Ashmark firing tables faster, each barrel correcting the last.", "Refined Explosives: +100% siege, +1% per Mortar."),  // 49
    ("Feed the Foundry's worst machine more of its own kind and the chaos in it deepens with the count.", "+10% chaos per Death Engine owned."),  // 50
    ("The Greycoats will buy the meat off a living Warden, and the ledger never asks why the line went thin.", "Philosopher's Stone: -1000 max HP, +2000 gold."),  // 51
    ("Stop the surgeon's work, sell the bandages, and pay the quartermaster in the healing you'll never get.", "Cursed Treasure: -100 HP regen, +5000 gold."),  // 52
    ("The Greycoats price the dead by the wound; every hundred you open is a coin in the ledger.", "+1 gold per 100 damage dealt."),  // 53
    ("A richer contract from the same bloody ledger, paying out every twenty points of ruin you deal.", "+1 gold per 20 damage dealt."),  // 54
    ("The Hexwrights tithe a quarter of your takings into borrowed math, keeping the skin paid up.", "Income tops up your mana shield, 25%."),  // 55
    ("Conclave shieldwork that studies the siege as it stands, thickening a little more each round it endures.", "+2500 shield, +400 more each round."),  // 56
    ("A patched plate off an Ashmark casualty, just enough give before the seam parts.", "Improved Masonry: +500 max HP."),  // 57
    ("A spare Warden quiver, the heads honed on the same stone the last gunner used.", "Improved Piercing Attacks: +10% piercing damage."),  // 58
    ("Plain Foundry shot, no markings, no cleverness, stamped out by the crate.", "Improved Normal Attacks: +10% normal damage."),  // 59
    ("A Foundry overrun of siege charges, serialed for a wall at Ashmark that no longer stands.", "Improved Siege Attacks: +10% siege damage."),  // 60
    ("Conclave ordnance the Hexwrights signed for but never logged, full of something that resents the barrel.", "Improved Chaos Attacks: +10% chaos damage."),  // 61
    ("Hull plating cut from a dead Ashmark engine, bolted over the old wounds.", "+1000 max HP."),  // 62
    ("A Foundry armor ration, thin but stamped and accounted for in the ledger.", "+10 flat armor."),  // 63
    ("Heavy Warden plate pulled off a tank that held this line three reliefs ago.", "+2000 max HP."),  // 64
    ("The Greycoats run the corpse-tally generous when the bodies pile this high.", "+50% gold per kill."),  // 65
    ("A Hexwright recharging-rite for the borrowed skin, drawing the pool back up a quarter faster than it bleeds.", "Recharge: +2000 shield, +25% shield regen."),  // 66
    ("A standing line in the quartermaster's ledger, paid out of Ashmark salvage each round.", "+20 gold income per round."),  // 67
    ("A scrap of Foundry plate, half a ration, glancing off what it can.", "Tower Armor: +5 flat armor."),  // 68
    ("A Foundry gear-kit that shaves the pause between firings, cut from a faster machine.", "+10% attack speed."),  // 69
    ("The Wardens' field-surgeon at full pace, stitching faster than the line falls apart, and faster still as the wounds mount.", "Renew: +80 HP regen, then +25%."),  // 70
    ("A slow, stubborn mending, all the relief the Wardens could spare this post.", "Repair Crew: +20 HP regen per tick."),  // 71
    ("A thin trickle on the Greycoat ledger, the kind command forgets to honor.", "Magic Coin: +5 gold income per round."),  // 72
    ("A modest seam in the quartermaster's books, paid round on round and skimmed a margin besides.", "Gold Mine: +10 income, +10% income."),  // 73
    ("The Greycoats pay twice over for the same fallen, the ledger long past honesty.", "+100% gold per kill."),  // 74
    ("Conclave shot the Hexwrights leaned on too hard, biting back through the barrel each time.", "Improved Magic Attacks: +10% magic damage."),  // 75
    ("A respectable clip of mending, more than the Relief ever delivered to this line.", "+40 HP regen per tick."),  // 76
    ("An obscene coat of Hexwright life-work, more borrowed skin than any Warden was meant to wear, and it turns the blows it eats.", "Energy Shield: +10000 mana shield."),  // 77
    ("A Greycoat's bad habits, learned at Ashmark: one swing in ten finds only air.", "Evasion: +10% chance to dodge a hit."),  // 78
    ("The Greycoat ledger compounds as the siege drags on, the dead worth more every round The Hippocrate nears.", "+100% kill gold, +15% more each round."),  // 79
    ("Warden salvage welded round Warden salvage, the wall thickening every round the line holds.", "+2500 max HP, +500 more each round."),  // 80
    ("A thin Hexwright coat of borrowed math, enough to spend before your own skin.", "Mana Shield: +1000 mana shield."),  // 81
    ("The Wardens' last surgeon works faster the longer the war grinds, mending quickening as the siege wears on.", "+120 HP/tick regen, +30 more each round."),  // 82
    ("Foundry masons fit the old wall to a finer tolerance than the war deserves, and the thicker it stands the harder its guns hit.", "Mastercrafted Masonry: +5000 max HP, +1% damage per 2000 max HP."),  // 83
    ("A Greycoat quartermaster's signet, worn smooth on the corpse-ledger; the richer the bounty it counts, the meaner the line shoots.", "Golden Ring: +200% kill bounty, +1% damage per 50% bounty."),  // 84
    ("A Hexwright sigil burned into the borrowed skin; while the shield holds, every shot carries a little of that stolen fire.", "Arcane Mark: +4000 mana shield, +20% damage while it holds."),  // 85
    ("The Conclave fed this thing on the dying and it learned to drink; each corpse on the line tops its borrowed skin back up.", "Maw of Death: +2000 mana shield, +15 shield per enemy killed."),  // 86
    ("Hexwright shielding wound so tight that when it finally fails it fails outward, a white concussion that lays the whole front rank flat.", "Energy Pulse: +2000 mana shield; when it breaks, stun all enemies in 1200 for 0.5s."),  // 87
    ("Greycoat sickness ground into the welds, so the swing that draws the tank's blood draws its own rot back.", "Poison Armor: +10 armor, +40 spikes; retaliation also poisons."),  // 88
    ("Each blow against the hull breaks off another barb into it; the longer the line presses, the crueler the welcome grows, until the round resets the count.", "Bloody Spikes: +80 spikes; retaliation stacks more damage each hit (resets each round)."),  // 89
    ("A Hexwright rot-totem bolted to the tank, breathing a slow green ruin into the ground around it that the close-pressed never leave clean.", "Blight Aura: +200 regen; pulse 200 poison damage in 600 every second."),  // 90
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content;

    #[test]
    fn arrays_cover_the_catalog() {
        assert_eq!(WEAPON_TEXT.len(), content::WEAPONS.len());
        assert_eq!(MODIFIER_TEXT.len(), content::MODIFIERS.len());
    }

    #[test]
    fn out_of_range_is_empty() {
        assert_eq!(weapon_text(9999), ("", ""));
        assert_eq!(modifier_text(9999), ("", ""));
    }
}
