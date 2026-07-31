//! The `wardens` theme pack — the original grim dark fantasy (the Pale Wardens,
//! the Ninth Foundry, the Hexwrights, the fall of Ashmark). Default for the Steam
//! build (`docs/12 §12.4`).
//!
//! Text migrated VERBATIM out of `descriptions.rs`; `docs/12 §12.1` is explicit
//! that this work does not get bulldozed. `name` is left empty throughout: the
//! wardens display names are the catalog names, and the pack resolves them from
//! `content.rs` (see `themes` module docs).

use super::{Entry, ThemePack, NUM_MODIFIERS, NUM_WEAPONS};

pub static PACK: ThemePack = ThemePack {
    id: "wardens",
    label: "The Pale Wardens",
    rarity_tiers: ["Common", "Uncommon", "Rare", "Epic"],
    weapons: &WEAPONS,
    modifiers: &MODIFIERS,
};

static WEAPONS: [Entry; NUM_WEAPONS] = [
    Entry { name: "", flavor: "Warden issue, third pattern; the stock is notched once for every gunner the Pale Wardens lost holding this same dirt.", tip: "Single-target piercing. The first thing they hand you, and often the last." },  // 0
    Entry { name: "", flavor: "Ninth Foundry overrun, its sights still ground for a wall that no longer stands at Ashmark.", tip: "Lobbed siege splash. Wide blast, slow to reseat." },  // 1
    Entry { name: "", flavor: "The cold it throws is the same cold that took Ashmark in one night; the Hexwrights only learned to aim it afterward.", tip: "Fast single-target magic. Stacks frost toward the freeze." },  // 2
    Entry { name: "", flavor: "Greycoat field-rations went bad in the same crates that carried these tips, and the quartermaster shipped both anyway.", tip: "Single-target piercing. Light hit, heavy poison rot." },  // 3
    Entry { name: "", flavor: "Foundry-work turned outward; the men who pour the cannon learned long ago that fire spreads down a packed column.", tip: "Short-range chaos splash. Stacks fire that cooks, then bursts on death." },  // 4
    Entry { name: "", flavor: "The Hexwrights bound a storm into the head and never told the Wardens how to put it down.", tip: "Heavy single-target magic. Long stun on the target." },  // 5
    Entry { name: "", flavor: "Stamped at the Ninth Foundry, four-bolt pattern, built for a frontier that needed walls cleared faster than it had men.", tip: "Long-range siege barrage. Four bolts to four targets." },  // 6
    Entry { name: "", flavor: "Wardens who stood too long beside a failing reactor learned to wear the burn; this makes a weapon of it.", tip: "Chaos area pulse around the tank. Stacks fire on everything near." },  // 7
    Entry { name: "", flavor: "Salvaged off the Ashmark sappers, who swung once and let the ground carry the rest of the argument.", tip: "Point-blank normal wave. Big sweeping hit, nothing applied." },  // 8
    Entry { name: "", flavor: "A Hexwright trinket that does its own targeting; the Conclave swears it was never meant to leave the vault.", tip: "Single-target piercing chain. Bounces to four enemies." },  // 9
    Entry { name: "", flavor: "The Foundry built one and then could not stop building it; each copy makes the next cheaper to feed.", tip: "Single-target chaos. Damage compounds per Death Engine owned." },  // 10
    Entry { name: "", flavor: "First rite the Conclave teaches its apprentices, and the last thing a great many things ever saw.", tip: "Single-target magic. Plain bolt, no status." },  // 11
    Entry { name: "", flavor: "A rock the Greycoats no longer had to carry, thrown at a problem they no longer had to name.", tip: "Single-target siege. Heavy stone, no status." },  // 12
    Entry { name: "", flavor: "Conclave-cut and clean, with none of the frost or fire the Hexwrights add when they want to be remembered.", tip: "Single-target magic. Quick, plain hit." },  // 13
    Entry { name: "", flavor: "Whatever the Conclave sealed in here predates the war, and it leaves armor unconvinced.", tip: "Single-target chaos. Ignores armor type." },  // 14
    Entry { name: "", flavor: "Greycoat sidearms, three to a belt, meant for the work that happens after the line already broke.", tip: "Short-range piercing chain. Fast, bounces to three." },  // 15
    Entry { name: "", flavor: "The Conclave renders down the heads of its own dead and stuffs them with worse; they remember whose they were.", tip: "Chaos chain. Bounces between three bodies." },  // 16
    Entry { name: "", flavor: "Suckula sleeps all day and feasts all night; every bite it lands comes straight back to you as a little more life.", tip: "Heavy point-blank chaos wave. Slow, vampiric on the base item." },  // 17
    Entry { name: "", flavor: "Eight tubes off the Ashmark batteries, recovered from a position that fired this volley once and was overrun anyway.", tip: "Epic long-range piercing barrage. Eight bolts, each stuns." },  // 18
    Entry { name: "", flavor: "A Warden glaive cut to find its mark across the open ground command never reinforced.", tip: "Long-range piercing chain. Bounces to three." },  // 19
    Entry { name: "", flavor: "Ninth Foundry pressure-rig, the kind that scalds the loader as readily as the target.", tip: "Short-range siege splash. Brief stun on hit." },  // 20
    Entry { name: "", flavor: "The Conclave does not say where this eye came from, only that it chooses, and the choosing is final.", tip: "Heavy single-target chaos. Pure punch, no status." },  // 21
    Entry { name: "", flavor: "Warden spearwork, made for the press at the wire where there is no room to miss.", tip: "Fast single-target piercing. Short stun." },  // 22
    Entry { name: "", flavor: "A Hexwright cloud of small wrong things, loosed long over the Ashmark dead and quiet after.", tip: "Long-range chaos splash. Spreads on impact." },  // 23
    Entry { name: "", flavor: "Foundry stonework, short-ranged and unsubtle, built for a line that had run out of subtler answers.", tip: "Short-range siege single-target. Heavy stone." },  // 24
    Entry { name: "", flavor: "One enormous open-palm Slap, delivered with the full indignation of a creature that was having a perfectly nice day.", tip: "Epic single-target piercing. Enormous hit, no status." },  // 25
    Entry { name: "", flavor: "The Greycoats took to calling this one mercy and kept walking; whatever it touches stops being a leg.", tip: "Heavy single-target piercing. Lasting wound on the base item." },  // 26
    Entry { name: "", flavor: "A Warden exotic that takes from the wrong end of the wound and gives it back to the gunner.", tip: "Long-range normal single-target. Feeds you on the base item." },  // 27
    Entry { name: "", flavor: "Conclave-honed thought given an edge, cutting outbound and again on the return.", tip: "Magic chain. Carves through four." },  // 28
    Entry { name: "", flavor: "No Hexwright trick on this one, just Foundry steel that refuses to lie where it lands.", tip: "Normal chain. Bounces to three." },  // 29
    Entry { name: "", flavor: "Spikewheel off the Ashmark works, set rolling once and never minded the men behind it.", tip: "Long-range siege chain. Bounces through six." },  // 30
    Entry { name: "", flavor: "Whatever the Greycoats could not bury, they loaded; the ledger lists it as ammunition.", tip: "Normal splash. Heavy impact, brief stun." },  // 31
    Entry { name: "", flavor: "Conclave hardware that discharges clean and leaves the smell on the air longer than the target.", tip: "Single-target magic. Stuns on hit." },  // 32
    Entry { name: "", flavor: "Barbs the Greycoats scavenged off something the Conclave should have burned, each one a small lasting debt.", tip: "Single-target piercing. Light poison rot." },  // 33
    Entry { name: "", flavor: "A Hexwright thing that lands still living, briefly, and leaves the target the same way.", tip: "Single-target magic. Trickling poison." },  // 34
    Entry { name: "", flavor: "Foundry shell packed over a Greycoat sickness, so the crater goes on killing after the blast is done.", tip: "Siege splash. Poisons the whole crater." },  // 35
    Entry { name: "", flavor: "One bite, by a thing the Conclave bred and refuses to account for; the rest is only the waiting.", tip: "Single-target normal. Strong poison rot." },  // 36
    Entry { name: "", flavor: "They told it the safe load at the Ninth Foundry, and the catapult overruled them in stone.", tip: "Heavy siege splash. Bigger rock, bigger hole." },  // 37
    Entry { name: "", flavor: "A Conclave reach out of nowhere that closes on whatever the gunner could least spare.", tip: "Single-target chaos. Stuns on hit." },  // 38
    Entry { name: "", flavor: "Greycoat netting, weighted and thrown, because a thing on the floor counting threads is a thing not at the wire.", tip: "Fast long-range normal single-target. Tangling stun." },  // 39
    Entry { name: "", flavor: "The Hexwrights seeded this ground and walked away; what grew has its own opinion about trespass.", tip: "Wide piercing area pulse. Hits everything near." },  // 40
    Entry { name: "", flavor: "A Conclave spirit that died once already and took the lesson badly, loosed to chain through a column.", tip: "Epic magic chain. Wails through eight." },  // 41
    Entry { name: "", flavor: "A Hexwright reactor's exhale, a clean white nothing the Wardens learned to point downrange.", tip: "Point-blank magic wave. Sweeps wide, stuns." },  // 42
    Entry { name: "", flavor: "A full rack of Ninth Foundry warheads, every casing serial-stamped and none of them aimed at anyone in particular.", tip: "Long chaos barrage. Twelve warheads to scattered targets, no status." },  // 43
    Entry { name: "", flavor: "The Hexwrights bottled the night Ashmark froze and packed it behind a fuse.", tip: "Fast piercing splash. Stacks frost on the cluster." },  // 44
    Entry { name: "", flavor: "A Foundry reject that never machined round, so the Greycoats let it loose to crack heads instead.", tip: "Normal bounce. Four hops, each stuns." },  // 45
    Entry { name: "", flavor: "The Pale Wardens cut this from the field after the Relief failed to come, and it has been hungry since.", tip: "Epic normal bounce. Chains four; feeds the tank." },  // 46
    Entry { name: "", flavor: "Foundry-town surplus by the crateload, cheap enough that the quartermaster stopped counting them.", tip: "Fast normal splash. Rapid small bursts, no status." },  // 47
    Entry { name: "", flavor: "Warden arrows wrapped in Foundry pitch, lit one rank at a time the way the line always burns.", tip: "Piercing barrage. Four shots, each stacks fire." },  // 48
    Entry { name: "", flavor: "The Conclave strung this between the Ashmark dead and called the rot it leaves a side effect.", tip: "Chaos bounce. Six links; leaves poison." },  // 49
    Entry { name: "", flavor: "Hexwright work meant for close quarters, borrowing more power than the gunner can hold.", tip: "Short magic bounce. Chains four." },  // 50
    Entry { name: "", flavor: "Ladled straight from a Ninth Foundry crucible and slung before it cools.", tip: "Fast siege single-target. Stacks fire." },  // 51
    Entry { name: "", flavor: "Catapult ammunition the Greycoats stripped from Ashmark's broken walls and lobbed back.", tip: "Short normal splash. Crushing impact, no status." },  // 52
    Entry { name: "", flavor: "The Wardens worked these into the dirt of the last yard the night before they were overrun.", tip: "Epic normal wave. Heavy sweep; stuns." },  // 53
    Entry { name: "", flavor: "Shroom Doom seeds a whole patch of angry caps; they pop in a chain, and what survives the blast gets up as your Spores.", tip: "Epic chaos pulse. Massive fire stacks; stuns." },  // 54
    Entry { name: "", flavor: "The Ninth Foundry never meant for one furnace to feed a whole field, but the war stopped asking.", tip: "Epic magic pulse. Buries a pack in fire stacks." },  // 55
    Entry { name: "", flavor: "Standard Foundry issue for the front rank, where the line meets the dark and rarely holds.", tip: "Fast piercing splash. Stacks fire." },  // 56
    Entry { name: "", flavor: "The Foundry-towns tap the deep crucibles for this, and what it spits keeps burning long after Ashmark's example.", tip: "Epic long siege splash. Heavy fire stacks." },  // 57
    Entry { name: "", flavor: "A single sliver of the cold that took Ashmark, fired again and again until something stops.", tip: "Fast piercing single-target. Stacks frost." },  // 58
    Entry { name: "", flavor: "Conclave frost that does not wait to be aimed; it spreads the way the Ashmark winter spread.", tip: "Magic splash. Spreads frost on impact." },  // 59
    Entry { name: "", flavor: "The Hexwrights bound a piece of that killing winter to a frame and could never make it stop.", tip: "Epic magic pulse. Frosts a huge radius toward freeze." },  // 60
    Entry { name: "", flavor: "Ashmark-cold shards racked by the Greycoats, four to a volley, ledgered by the dozen.", tip: "Normal barrage. Four hits, each stacks frost." },  // 61
    Entry { name: "", flavor: "The first thing the quartermaster hands a green Greycoat, and the cheapest line in the ledger.", tip: "Fast piercing barrage. Three blades, no status." },  // 62
    Entry { name: "", flavor: "Blunt Foundry overrun, loud and serial-stamped, sold by weight to the Wardens.", tip: "Fast siege single-target. No status." },  // 63
    Entry { name: "", flavor: "A Greycoat deserter's rifle, recovered from Ashmark with the scope still sighted on a fleeing back.", tip: "Fast long normal single-target. No status." },  // 64
    Entry { name: "", flavor: "Ninth Foundry charges that the Greycoats lob underhand and never look back at.", tip: "Short siege splash. Brief stun." },  // 65
    Entry { name: "", flavor: "A single precise Sting from a very small, very offended thing — and suddenly the target is taking everyone's hits much harder.", tip: "Long chaos single-target. No status." },  // 66
    Entry { name: "", flavor: "Somebody's skull, packed by the Greycoats with worse and stamped for the line.", tip: "Chaos splash. Bursts and stuns." },  // 67
    Entry { name: "", flavor: "Foundry casings charged with Ashmark's leftover cold, breathed out across the crater.", tip: "Siege splash. Frosts the blast." },  // 68
    Entry { name: "", flavor: "The Conclave sends the cold out in a line now, the way the news of Ashmark traveled the front.", tip: "Magic wave. Sweeps and frosts." },  // 69
    Entry { name: "", flavor: "A wall of Foundry-fire walked forward across the dirt, the way the line is supposed to and no longer can.", tip: "Normal wave. Sweeps; heavy fire stacks." },  // 70
    Entry { name: "", flavor: "A Hexwright spark that flickers out and carries a sliver of the gunner back with each kill.", tip: "Fast chaos single-target. Heals the tank." },  // 71
    Entry { name: "", flavor: "The Conclave taught it to drink an enemy's borrowed power before it spends the enemy.", tip: "Fast long magic single-target. No status." },  // 72
    Entry { name: "", flavor: "Squirm keeps a wriggling brood in reserve; every kill it lands hatches another hungry Larva to join the march.", tip: "Epic long chaos single-target. No status." },  // 73
    Entry { name: "", flavor: "Warden field-rig that wraps the tank in Foundry heat and makes the burn everyone else's problem.", tip: "Rapid magic wave. Constant fire near the tank." },  // 74
    Entry { name: "", flavor: "Boom Bloom scatters a tidy little garden of pop-when-touched buds across the last yard, and the front rank finds every one.", tip: "Epic siege wave. Fast, brutal; stuns." },  // 75
    Entry { name: "", flavor: "A Warden quill-rig recovered off Ashmark, sprung once across a whole closing cluster.", tip: "Long piercing splash. Sprays the cluster, no status." },  // 76
    Entry { name: "", flavor: "Pure Conclave intention let off all at once, the kind of borrowing the Hexwrights regret in the morning.", tip: "Long magic splash. Detonates on impact." },  // 77
    Entry { name: "", flavor: "The sky pays the Ninth Foundry's debts all at once, then makes the line wait a long time for the next installment.", tip: "Epic siege barrage. Eight meteors; long reload." },  // 78
    Entry { name: "", flavor: "A keg the Greycoats meant for after the Relief came, lit and thrown instead.", tip: "Fast siege splash. Heals the tank." },  // 79
    Entry { name: "", flavor: "Quick mean Conclave work that goes through armor like it was never logged.", tip: "Fast long chaos single-target. Ignores armor types." },  // 80
    Entry { name: "", flavor: "Hexwright lightning bound to circle the tank, and standing near it is the gunner's own decision.", tip: "Wide magic pulse. Shocks everything around." },  // 81
    Entry { name: "", flavor: "The Conclave wired this to hum off the enemy's own borrowed mana, and things near it stop humming.", tip: "Long magic single-target. No status." },  // 82
    Entry { name: "", flavor: "Foundry-fire let off in a bloom, the way the front rank goes when the line finally breaks.", tip: "Chaos pulse. Erupts fire around the tank." },  // 83
    Entry { name: "", flavor: "A Ninth Foundry coil that taps fast enough to leave no gap between the taps.", tip: "Rapid long siege pulse. Near-constant stun-lock." },  // 84
    Entry { name: "", flavor: "Tangle throws out a snarl of sticky vine that pins a thing right where it stands and politely refuses to let go.", tip: "Fast epic normal single-target. Roots the target in place." },  // 85
];

static MODIFIERS: [Entry; NUM_MODIFIERS] = [
    Entry { name: "", flavor: "Foundry stamps this kit \"general issue,\" the catch-all crate the Wardens hand out when nothing specialized survived the haul from Ashmark.", tip: "+10% to all weapon damage." },  // 0
    Entry { name: "", flavor: "Hexwrights ground these heads to a finer point than the Foundry would risk, and the Wardens never asked how.", tip: "Improved Piercing Attacks: +10% piercing damage." },  // 1
    Entry { name: "", flavor: "Recovered from the Ashmark siege-works, calibrated for walls that fell the night the line broke.", tip: "Improved Siege Attacks: +10% siege damage." },  // 2
    Entry { name: "", flavor: "The Conclave bottles a little more of what it borrows, and signs none of it.", tip: "Improved Magic Attacks: +10% magic damage." },  // 3
    Entry { name: "", flavor: "Foundry overrun stamped EPIC and locked in the deep vault; the Wardens were told it would never leave Ashmark.", tip: "+25% damage, multiplicative on top." },  // 4
    Entry { name: "", flavor: "A Hexwright governor wired into the firing chain, shaving the pause the Foundry built in for safety.", tip: "Rapidfire: +10% attack speed." },  // 5
    Entry { name: "", flavor: "The quartermaster's ledger pays a premium per body, settled out of whatever the Greycoats strip off the field.", tip: "Bounty Hunter: +50% gold per kill." },  // 6
    Entry { name: "", flavor: "A Hexwright-tapped seam that pays the gunner in coin and in mending both, the ledger and the surgeon settled from one vein.", tip: "Entangled Gold Mine: +20 income, 25% of it healed." },  // 7
    Entry { name: "", flavor: "The quartermaster skims a margin off every coin the Greycoats turn in, and the ledger never forgets.", tip: "+10% to passive income." },  // 8
    Entry { name: "", flavor: "A heavier cut, written into the ledger by a quartermaster who stopped pretending the conscripts would be paid.", tip: "+25% to passive income." },  // 9
    Entry { name: "", flavor: "The quartermaster keeps a crooked ledger; now and then a body settles for far more than its line was worth, and pays the premium too.", tip: "Transmute: +100% bounty; 5% of kills pay triple." },  // 10
    Entry { name: "", flavor: "Warden masons doubled the wall on the wall they already had, the way a siege teaches you to never stop pouring stone.", tip: "Imbued Masonry: +2000 max HP, then +25%." },  // 11
    Entry { name: "", flavor: "Foundry hull-shave, rolled too thin in the rush but thick enough to turn what the Relief left you to face.", tip: "+10 flat armor. Shaves every hit." },  // 12
    Entry { name: "", flavor: "A Hexwright spring fed into the hull, the borrowed skin topping itself off the way a well does.", tip: "Moonwell: +2000 mana shield, regenerating." },  // 13
    Entry { name: "", flavor: "The Relief that never came included field-surgeons; the Wardens improvised this drip in their absence.", tip: "+50 HP regen per tick." },  // 14
    Entry { name: "", flavor: "A Warden's last trick when the line is held and the Relief is a rumor: be where the blow is not.", tip: "Evasion: +10% chance to dodge a hit." },  // 15
    Entry { name: "", flavor: "The longer the war drags on, the heavier the Foundry's old governor leans into the firing chain.", tip: "Power Generator: +2% damage now, +1% every round." },  // 16
    Entry { name: "", flavor: "The Conclave warned that borrowed chaos compounds with every round it sits unrepaid, and The Hippocrate is still on the horizon.", tip: "Scroll of Chaos: +20% chaos now, +3% every round." },  // 17
    Entry { name: "", flavor: "The quartermaster's ledger breeds on itself; each round the Greycoats turn in more, and the line grows fatter.", tip: "+10 income now, +5 every round." },  // 18
    Entry { name: "", flavor: "Scar over scar, the way a Warden's plating thickens every round the Relief fails to arrive.", tip: "Blessed Armor: +10 armor now, +5 every round." },  // 19
    Entry { name: "", flavor: "Warden marksmanship doctrine, the kind drilled into gunners told to make every single shot count.", tip: "Focusfire: +25% to single-target weapons." },  // 20
    Entry { name: "", flavor: "Foundry overpressure tuning meant for the crater-makers, ground for walls and the crowds behind them.", tip: "+25% to splash weapons." },  // 21
    Entry { name: "", flavor: "Greycoat volley discipline, salvaged off a unit that fired in ranks until the ranks ran out.", tip: "+25% to barrage weapons." },  // 22
    Entry { name: "", flavor: "A Hexwright field-binding that thickens whatever the tank pulses out into the ring of dirt around it.", tip: "+25% to area-pulse weapons." },  // 23
    Entry { name: "", flavor: "Calibration recovered from the Ashmark breakwalls, meant to shove a whole front rank into the next world.", tip: "Wavefire: +25% to wave weapons." },  // 24
    Entry { name: "", flavor: "Foundry ricochet-work, each hop tuned to land angrier than the last off scrap nobody else would salvage.", tip: "+25% to bounce weapons." },  // 25
    Entry { name: "", flavor: "Warden close-line doctrine, where the work is done at knife-reach and the gunner smells what he kills.", tip: "Command Aura: +25% to short-range (<=600)." },  // 26
    Entry { name: "", flavor: "A long-glass sighting kit off an Ashmark sniper-nest, for killing them before they have a face.", tip: "Trueshot Aura: +25% to long-range (>=900)." },  // 27
    Entry { name: "", flavor: "The Foundry's cheapest overruns, the junk the Greycoats are handed first, made to earn twice its serial.", tip: "Engineering Upgrade: doubles common-weapon damage." },  // 28
    Entry { name: "", flavor: "Warden execution drill: a thing on the ground and stunned is a thing the line stops counting.", tip: "Bash: +20% damage to stunned enemies." },  // 29
    Entry { name: "", flavor: "Finish what the field-rations and the rot already started in them.", tip: "Corrosive Poison: +25% damage to poisoned." },  // 30
    Entry { name: "", flavor: "Greycoat sickness spread through Ashmark in the bad winters; the Conclave learned to concentrate it.", tip: "Potent Poison: +10% applied poison damage." },  // 31
    Entry { name: "", flavor: "A Hexwright lock that holds the stunned a breath longer, long enough for the Wardens to work.", tip: "Dazing Stuns: +50% stun duration." },  // 32
    Entry { name: "", flavor: "Field-improvised cruelty: Ashmark scrap-iron driven point-out into the tank's hull, set so the swing that lands draws a little back.", tip: "Dreadlord Fang: +80 retaliation when hit." },  // 33
    Entry { name: "", flavor: "A denser hide of welded scrap, the kind a Warden bolts on when reaching the tank should cost a limb.", tip: "+300 retaliation damage when hit." },  // 34
    Entry { name: "", flavor: "The same improvised barbs, ground keener by a gunner with nothing left to do but sharpen the punishment.", tip: "+50% to all spikes damage." },  // 35
    Entry { name: "", flavor: "Every round the war drags on, the Wardens hammer fresh scrap into the hull and the welcome gets crueler.", tip: "Growing Spikes: +80 now, +10 every round." },  // 36
    Entry { name: "", flavor: "A Hexwright marking-rite that opens the flesh of everything near; the marked do not heal what is opened.", tip: "Vulnerability Totem: nearby enemies take +5%/sec." },  // 37
    Entry { name: "", flavor: "The Relief never came, so the Wardens took their mending off the dying and bolted fresh plate while they were at it.", tip: "Mask of Death: +1000 max HP, +15 HP per kill." },  // 38
    Entry { name: "", flavor: "A deeper draught of the same grim arithmetic, each corpse on the line paying back the Relief's debt.", tip: "+60 HP each time an enemy dies." },  // 39
    Entry { name: "", flavor: "Conclave rot-spores feed two mouths at once, the dying enemy's and the gunner the Relief abandoned.", tip: "Reanimating Poison: +5 HP every poison tick." },  // 40
    Entry { name: "", flavor: "Three flips of a Greycoat token, and the quartermaster's ledger pays out in triplicate before anyone checks the column twice.", tip: "Multiplication Gems: next common yields 3 copies." },  // 41
    Entry { name: "", flavor: "The Conclave keeps a second of everything worth having, off the ledger and out of the quartermaster's count.", tip: "Duplicator: next rare bought yields 1 free copy." },  // 42
    Entry { name: "", flavor: "A name in the right margin of the ledger, and the Greycoats look the other way once.", tip: "Black Market: next uncommon purchase is free." },  // 43
    Entry { name: "", flavor: "Coin the Greycoats buried at Ashmark before the line broke, dug up and still drawing interest.", tip: "Magic Treasure: +250 gold now, +5 income/round." },  // 44
    Entry { name: "", flavor: "The Wardens buried this rite with their last chaplain; it answers once, and resents being asked.", tip: "Ankh of Reconstruction: revive once, +2000 max HP." },  // 45
    Entry { name: "", flavor: "What field-dressing the Wardens have left, doled out by a surgeon who stopped counting the dead at Ashmark.", tip: "Healing Hand: +25% to all healing received." },  // 46
    Entry { name: "", flavor: "Hexwright greenwood grown through the hull, more wall and slower mending the closer to death the gunner runs.", tip: "Living Wood: +2000 max HP, 1.5% missing HP/sec." },  // 47
    Entry { name: "", flavor: "Every Warden bow racked beside this one lends its draw to the next; the whole rack of them sights truer for the hoard.", tip: "Enchanted Moon Arrow: +100% piercing, +1% per Bow." },  // 48
    Entry { name: "", flavor: "Ranged together, Foundry mortars find the old Ashmark firing tables faster, each barrel correcting the last.", tip: "Refined Explosives: +100% siege, +1% per Mortar." },  // 49
    Entry { name: "", flavor: "Feed the Foundry's worst machine more of its own kind and the chaos in it deepens with the count.", tip: "+10% chaos per Death Engine owned." },  // 50
    Entry { name: "", flavor: "The Greycoats will buy the meat off a living Warden, and the ledger never asks why the line went thin.", tip: "Philosopher's Stone: -1000 max HP, +2000 gold." },  // 51
    Entry { name: "", flavor: "Stop the surgeon's work, sell the bandages, and pay the quartermaster in the healing you'll never get.", tip: "Cursed Treasure: -100 HP regen, +5000 gold." },  // 52
    Entry { name: "", flavor: "The Greycoats price the dead by the wound; every hundred you open is a coin in the ledger.", tip: "+1 gold per 100 damage dealt." },  // 53
    Entry { name: "", flavor: "A richer contract from the same bloody ledger, paying out every twenty points of ruin you deal.", tip: "+1 gold per 20 damage dealt." },  // 54
    Entry { name: "", flavor: "The Hexwrights tithe a quarter of your takings into borrowed math, keeping the skin paid up.", tip: "Income tops up your mana shield, 25%." },  // 55
    Entry { name: "", flavor: "Conclave shieldwork that studies the siege as it stands, thickening a little more each round it endures.", tip: "+2500 shield, +400 more each round." },  // 56
    Entry { name: "", flavor: "A patched plate off an Ashmark casualty, just enough give before the seam parts.", tip: "Improved Masonry: +500 max HP." },  // 57
    Entry { name: "", flavor: "A spare Warden quiver, the heads honed on the same stone the last gunner used.", tip: "Improved Piercing Attacks: +10% piercing damage." },  // 58
    Entry { name: "", flavor: "Plain Foundry shot, no markings, no cleverness, stamped out by the crate.", tip: "Improved Normal Attacks: +10% normal damage." },  // 59
    Entry { name: "", flavor: "A Foundry overrun of siege charges, serialed for a wall at Ashmark that no longer stands.", tip: "Improved Siege Attacks: +10% siege damage." },  // 60
    Entry { name: "", flavor: "Conclave ordnance the Hexwrights signed for but never logged, full of something that resents the barrel.", tip: "Improved Chaos Attacks: +10% chaos damage." },  // 61
    Entry { name: "", flavor: "Hull plating cut from a dead Ashmark engine, bolted over the old wounds.", tip: "+1000 max HP." },  // 62
    Entry { name: "", flavor: "A Foundry armor ration, thin but stamped and accounted for in the ledger.", tip: "+10 flat armor." },  // 63
    Entry { name: "", flavor: "Heavy Warden plate pulled off a tank that held this line three reliefs ago.", tip: "+2000 max HP." },  // 64
    Entry { name: "", flavor: "The Greycoats run the corpse-tally generous when the bodies pile this high.", tip: "+50% gold per kill." },  // 65
    Entry { name: "", flavor: "A Hexwright recharging-rite for the borrowed skin, drawing the pool back up a quarter faster than it bleeds.", tip: "Recharge: +2000 shield, +25% shield regen." },  // 66
    Entry { name: "", flavor: "A standing line in the quartermaster's ledger, paid out of Ashmark salvage each round.", tip: "+20 gold income per round." },  // 67
    Entry { name: "", flavor: "A scrap of Foundry plate, half a ration, glancing off what it can.", tip: "Tower Armor: +5 flat armor." },  // 68
    Entry { name: "", flavor: "A Foundry gear-kit that shaves the pause between firings, cut from a faster machine.", tip: "+10% attack speed." },  // 69
    Entry { name: "", flavor: "The Wardens' field-surgeon at full pace, stitching faster than the line falls apart, and faster still as the wounds mount.", tip: "Renew: +80 HP regen, then +25%." },  // 70
    Entry { name: "", flavor: "A slow, stubborn mending, all the relief the Wardens could spare this post.", tip: "Repair Crew: +20 HP regen per tick." },  // 71
    Entry { name: "", flavor: "A thin trickle on the Greycoat ledger, the kind command forgets to honor.", tip: "Magic Coin: +5 gold income per round." },  // 72
    Entry { name: "", flavor: "A modest seam in the quartermaster's books, paid round on round and skimmed a margin besides.", tip: "Gold Mine: +10 income, +10% income." },  // 73
    Entry { name: "", flavor: "The Greycoats pay twice over for the same fallen, the ledger long past honesty.", tip: "+100% gold per kill." },  // 74
    Entry { name: "", flavor: "Conclave shot the Hexwrights leaned on too hard, biting back through the barrel each time.", tip: "Improved Magic Attacks: +10% magic damage." },  // 75
    Entry { name: "", flavor: "A respectable clip of mending, more than the Relief ever delivered to this line.", tip: "+40 HP regen per tick." },  // 76
    Entry { name: "", flavor: "An obscene coat of Hexwright life-work, more borrowed skin than any Warden was meant to wear, and it turns the blows it eats.", tip: "Energy Shield: +10000 mana shield." },  // 77
    Entry { name: "", flavor: "A Greycoat's bad habits, learned at Ashmark: one swing in ten finds only air.", tip: "Evasion: +10% chance to dodge a hit." },  // 78
    Entry { name: "", flavor: "The Greycoat ledger compounds as the siege drags on, the dead worth more every round The Hippocrate nears.", tip: "+100% kill gold, +15% more each round." },  // 79
    Entry { name: "", flavor: "Warden salvage welded round Warden salvage, the wall thickening every round the line holds.", tip: "+2500 max HP, +500 more each round." },  // 80
    Entry { name: "", flavor: "A thin Hexwright coat of borrowed math, enough to spend before your own skin.", tip: "Mana Shield: +1000 mana shield." },  // 81
    Entry { name: "", flavor: "The Wardens' last surgeon works faster the longer the war grinds, mending quickening as the siege wears on.", tip: "+120 HP/tick regen, +30 more each round." },  // 82
    Entry { name: "", flavor: "Foundry masons fit the old wall to a finer tolerance than the war deserves, and the thicker it stands the harder its guns hit.", tip: "Mastercrafted Masonry: +5000 max HP, +1% damage per 2000 max HP." },  // 83
    Entry { name: "", flavor: "A Greycoat quartermaster's signet, worn smooth on the corpse-ledger; the richer the bounty it counts, the meaner the line shoots.", tip: "Golden Ring: +200% kill bounty, +1% damage per 50% bounty." },  // 84
    Entry { name: "", flavor: "A Hexwright sigil burned into the borrowed skin; while the shield holds, every shot carries a little of that stolen fire.", tip: "Arcane Mark: +4000 mana shield, +20% damage while it holds." },  // 85
    Entry { name: "", flavor: "The Conclave fed this thing on the dying and it learned to drink; each corpse on the line tops its borrowed skin back up.", tip: "Maw of Death: +2000 mana shield, +15 shield per enemy killed." },  // 86
    Entry { name: "", flavor: "Hexwright shielding wound so tight that when it finally fails it fails outward, a white concussion that lays the whole front rank flat.", tip: "Energy Pulse: +2000 mana shield; when it breaks, stun all enemies in 1200 for 0.5s." },  // 87
    Entry { name: "", flavor: "Greycoat sickness ground into the welds, so the swing that draws the tank's blood draws its own rot back.", tip: "Poison Armor: +10 armor, +40 spikes; retaliation also poisons." },  // 88
    Entry { name: "", flavor: "Each blow against the hull breaks off another barb into it; the longer the line presses, the crueler the welcome grows, until the round resets the count.", tip: "Bloody Spikes: +80 spikes; retaliation stacks more damage each hit (resets each round)." },  // 89
    Entry { name: "", flavor: "A Hexwright rot-totem bolted to the tank, breathing a slow green ruin into the ground around it that the close-pressed never leave clean.", tip: "Blight Aura: +200 regen; pulse 200 poison damage in 600 every second." },  // 90
];
