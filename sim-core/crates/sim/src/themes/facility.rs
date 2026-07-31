//! The `facility` theme pack — weapons and modifiers (`docs/12 §12.4`).
//!
//! Register: an anchored, malfunctioning containment apparatus holding a corridor
//! that should not exist. Weapons are catalogued anomalous objects; modifiers are
//! procedures, retrofits, clearance upgrades and site requisitions. The voice is
//! deadpan bureaucracy describing something awful, and the joke lives in the gap
//! between the two.
//!
//! **House style — this is load-bearing, do not smooth it out.** A catalog of 177
//! entries written by a person is uneven, and the unevenness is what makes any of
//! it land. So: entries run from three words to fifty; roughly a third of them are
//! a flat fact with no turn at the end; a few are funny, a few are plainly awful,
//! and a good number are just logistics with no joke in them at all. Kickers are
//! rationed. Rule-of-three lists appear twice in the whole pack, on purpose. The
//! em-dash parenthetical does not appear at all. If every entry ends on a twist
//! then no entry does, and an editor "improving" the dull ones back into punchlines
//! would undo the pack.
//!
//! **Originality (`docs/12 §12.5`).** Original fiction in the genre. No item
//! numbers, no wiki entity names, no borrowed Object Classes — the tier
//! vocabulary here (`CATALOGUED` / `RESTIVE` / `HOSTILE` / `UNGOVERNED`) is
//! invented for this pack — no wiki level or entity names, no real trademarks,
//! brands or people. The recurring furniture (the Corridor, the Register, the
//! nightly count, the sublevels, Recovery / Intake / Procurement / Section Nine)
//! is this game's, written to serve this game's mechanics.
//!
//! **Every tip is cross-checked against `content.rs` and the behavior modules.**
//! Flavor is allowed to lie; the tip is not. Where the catalog's own entry NAME
//! misstates its effect (`+10% Gold Income` is `IncomePct(20, 100)`), the tip
//! states the real number and the flavor makes the discrepancy the joke.

use super::{Entry, ThemePack, NUM_MODIFIERS, NUM_WEAPONS};

pub static PACK: ThemePack = ThemePack {
    id: "facility",
    label: "The Register",
    // `docs/12 §12.5`: invent the tier vocabulary, do not borrow one.
    rarity_tiers: ["CATALOGUED", "RESTIVE", "HOSTILE", "UNGOVERNED"],
    weapons: &WEAPONS,
    modifiers: &MODIFIERS,
};

/// Catalogued anomalous objects, by weapon index.
static WEAPONS: [Entry; NUM_WEAPONS] = [
    Entry {  // 0 — Bow: 75 piercing, single-target, 1.0 s, range 900
        name: "Bow, Site-Issued",
        flavor: "One per anchor, on signature. It is a bow.",
        tip: "Single-target piercing. The object you start with; it works forever and surprises nobody.",
    },
    Entry {  // 1 — Mortar Launcher: 340 siege, splash 300, 1.6 s, range 1200
        name: "Requisitioned Mortar",
        flavor: "Ordered by Facilities to clear a blockage on Sublevel 2. There was no blockage on Sublevel 2. There is now no Sublevel 2.",
        tip: "Long-range siege splash. Wide blast, slow to reload.",
    },
    Entry {  // 2 — Frost Bow: 130 magic, single-target, 0.5 s, 7 frost
        name: "Cold Locker Bow",
        flavor: "Eleven years at the back of the specimen freezer.",
        tip: "Fast single-target magic. 7 frost a hit — four hits reach the freeze at 25.",
    },
    Entry {  // 3 — Poison Bow: 60 piercing, poison 20/tick for 3 s
        name: "Dirty Arrows",
        flavor: "Medical cleared these for issue on the grounds that whatever coats the tips is already in the water supply.",
        tip: "Single-target piercing. Light hit, heavy 3-second poison.",
    },
    Entry {  // 4 — Flamecaster: 110 chaos, splash 200, range 600, 6 fire
        name: "Ignition Sprayer",
        flavor: "Recovered from a staff kitchen with no gas line and no power. The floor plan filed with the county does not give it a door.",
        tip: "Short-range chaos splash. 6 fire a hit; burning things take more damage and detonate when they die.",
    },
    Entry {  // 5 — Storm Hammer: 850 magic, 1.6 s, 1.5 s stun
        name: "Grounding Rod (Do Not Hold)",
        flavor: "Safe to operate from behind the yellow line. The line is repainted weekly by a contractor working from a drawing nobody has produced, and it has never twice been in the same place.",
        tip: "Heavy single-target magic. 1.5-second stun on the target.",
    },
    Entry {  // 6 — Ballista: 150 siege, barrage 4, range 1200
        name: "Four-Bolt Rig",
        flavor: "Shelving brackets, a winch motor and about nine metres of cable, assembled overnight into a firing frame by persons unknown, against a work order that no department at this site submitted and that Procurement has settled twice without querying either payment.",
        tip: "Long-range siege barrage. Four bolts, four separate targets.",
    },
    Entry {  // 7 — Immolation: 80 chaos, area 300, 1.0 s, 2 fire
        name: "Ambient Combustion Field",
        flavor: "The thermostat on this level will not go below forty-one degrees. Maintenance logged it as a comfort issue and closed the ticket.",
        tip: "Chaos pulse around the anchor. 2 fire to everything close, every second.",
    },
    Entry {  // 8 — Shockwave Axe: 900 normal, wave, 2.0 s, point-blank
        name: "Groundskeeper's Axe",
        flavor: "Worn to a grip that fits no hand on the roster. We have measured every hand on the roster.",
        tip: "Point-blank sweep. Huge hit on everything near; no status applied.",
    },
    Entry {  // 9 — Moon Glaive: 175 piercing, bounce 4, range 600
        name: "Returning Disc",
        flavor: "It comes back.",
        tip: "Piercing chain. Instantly strikes four nearby enemies.",
    },
    Entry {  // 10 — Death Engine: 260 chaos, single-target, range 900
        name: "Duplicating Apparatus",
        flavor: "Procurement approved more of them on the grounds that we already have several. That reasoning is what produced the several.",
        tip: "Single-target chaos. Unremarkable alone — it compounds only with the retrofit that pays +10% chaos per Apparatus owned.",
    },
    Entry {  // 11 — Magic Missile: 100 normal, single-target
        name: "Standard Emitter",
        flavor: "Emits. The finished report is one word long.",
        tip: "Single-target normal damage. Plain, cheap, dependable.",
    },
    Entry {  // 12 — Boulder: 110 siege, single-target
        name: "The Rock From Outside",
        flavor: "It was in the car park. It is now in the inventory. Nobody moved it.",
        tip: "Single-target siege. A rock, thrown. No status.",
    },
    Entry {  // 13 — Magic Bolt: 100 magic, single-target
        name: "Baseline Discharge Unit",
        flavor: "Trainee issue. One function, performed reliably, for nine years.",
        tip: "Single-target magic. Quick, plain hit.",
    },
    Entry {  // 14 — Chaos Orb: 95 chaos, single-target
        name: "Unlabelled Sphere",
        flavor: "Intake lost the tag. Writing a replacement would mean deciding what it is.",
        tip: "Single-target chaos. Chaos ignores armour class entirely.",
    },
    Entry {  // 15 — Throwing Axes: 55 piercing, bounce 3, range 300
        name: "Hatchet Set (Incomplete)",
        flavor: "Catalogued as a set of six. Six are present at every count. There have only ever been three.",
        tip: "Short-range piercing chain. Fast, hits three.",
    },
    Entry {  // 16 — Chaos Skulls: 50 chaos, bounce 3, range 600
        name: "Box Of Heads",
        flavor: "Filed under office supplies, because the box arrived filed under office supplies. Here we go by the box.",
        tip: "Chaos chain. Bounces between three targets.",
    },
    Entry {  // 17 — Suckula: 1100 chaos wave, 2.5 s, life-drain 40
        name: "The Thing In The Vending Machine",
        flavor: "Sublevel 3 breakroom, slot C4, since at least 2011. It takes exact change and it does not accept notes. Facilities have run a power lead to it out of the corridor supply, on the grounds that it was going to have one anyway. It gives back something warm.",
        tip: "Point-blank chaos sweep. Slow, but repairs the anchor 40 per enemy hit.",
    },
    Entry {  // 18 — Missile Barrage: 1400 piercing, barrage 8, 3 s stun
        name: "Sublevel 9 Ordnance Rack",
        flavor: "Sublevel 9 appears on no evacuation map and in no part of the building's legal description. The rack is against the north wall, four deep, restocked to a schedule, and the requisitions it draws against are settled out of a cost centre that closed in 2004.",
        tip: "Epic long-range barrage. Eight bolts, each stunning for 3 seconds.",
    },
    Entry {  // 19 — Seeker Axe: 60 piercing, bounce 3, range 900
        name: "Axe That Finds You",
        flavor: "Recovery named it. Recovery name things badly and then defend the name at meetings.",
        tip: "Long-range piercing chain. Bounces to three.",
    },
    Entry {  // 20 — Steam Cannon: 320 siege splash, range 300, 0.5 s stun
        name: "Boiler Tap",
        flavor: "There is no boiler. The tap is plumbed into something on the far side of the wall that has been paying our heating bill for years.",
        tip: "Short-range siege splash. Brief half-second stun.",
    },
    Entry {  // 21 — Demon Eye: 1050 chaos, +10 vulnerability stacks on hit
        name: "Observation Lens",
        flavor: "It shows you whatever you are already looking at, one second early.",
        tip: "Heavy single-target chaos. Each hit leaves +10% damage taken, stacking permanently.",
    },
    Entry {  // 22 — Impaler: 240 piercing, 0.5 s cadence, short stun
        name: "Pinning Spike",
        flavor: "Supplied with a laminated card explaining its purpose. The card has been laminated four times, each time over a different explanation.",
        tip: "Fast single-target piercing. Short stun on every hit.",
    },
    Entry {  // 23 — Chaos Swarm: 105 chaos splash, range 1200
        name: "The Jar",
        flavor: "The containment is a mason jar. It has held nine years.",
        tip: "Long-range chaos splash. Spreads on impact.",
    },
    Entry {  // 24 — Catapult: 380 siege single-target, range 600
        name: "Bay 2 Loading Arm",
        flavor: "Built to move pallets. A temp retrained it over an afternoon.",
        tip: "Short-range siege single-target. Heavy, no status.",
    },
    Entry {  // 25 — Slap: 2600 piercing, knockback 300
        name: "Open Hand, Enormous",
        flavor: "Manifests, delivers one slap, unmanifests. Psychology assess the humiliation as the primary hazard.",
        tip: "Epic single-target piercing. Enormous hit, and knocks the target back 300.",
    },
    Entry {  // 26 — Crippler: 1150 piercing, +8 vulnerability on hit
        name: "Load-Bearing Injury",
        flavor: "What it takes off does not grow back.",
        tip: "Heavy single-target piercing. Leaves +8% damage taken, stacking permanently.",
    },
    Entry {  // 27 — Lifeleecher: 850 normal, range 1200, life-drain 40
        name: "Transfusion Line",
        flavor: "Medical ask that nobody raise which direction it was originally built to run.",
        tip: "Long-range single-target. Repairs the anchor 40 per enemy hit.",
    },
    Entry {  // 28 — Spell Glaive: 260 magic, bounce 4
        name: "Idea With An Edge",
        flavor: "Reading the intake summary constitutes exposure. You have now read part of the intake summary.",
        tip: "Magic chain. Carves through four.",
    },
    Entry {  // 29 — Glaive Thrower: 70 normal, bounce 3
        name: "Blade Feeder",
        flavor: "Refuses to lie flat and refuses to stay in the drawer. No other trouble.",
        tip: "Normal chain. Bounces to three.",
    },
    Entry {  // 30 — Spikewheel Launcher: 230 siege, bounce 6, range 1200
        name: "Wheel, Still Rolling",
        flavor: "It has not stopped since intake. We built the storage rack around it.",
        tip: "Long-range siege chain. Rolls through six.",
    },
    Entry {  // 31 — Meatapult: 820 normal splash, 2 s stun
        name: "Organic Payload Sling",
        flavor: "The requisition form has a field for the payload. The field is blank.",
        tip: "Normal splash. Heavy impact, 2-second stun.",
    },
    Entry {  // 32 — Arcane Blaster: 360 magic, 1.5 s stun
        name: "Serviced Discharge Wand",
        flavor: "Serviced quarterly by a contractor whose company has no registration anywhere.",
        tip: "Single-target magic. 1.5-second stun on hit.",
    },
    Entry {  // 33 — Quills: 230 piercing, light 3 s poison
        name: "Shed Quill",
        flavor: "Found in the corridor carpet. Nothing on the manifest sheds.",
        tip: "Single-target piercing. Light poison for 3 seconds.",
    },
    Entry {  // 34 — Living Spittle: 230 magic, light 3 s poison
        name: "Expectorant Sample",
        flavor: "Held in a sealed tray in the cold room on Sublevel 3 and inspected four times daily under Form 22. It is still warm at every inspection, including the four o'clock inspection, which is on no rota and is always attended.",
        tip: "Single-target magic. Trickling 3-second poison.",
    },
    Entry {  // 35 — Poison Bomb: 320 siege splash, range 1200, poison
        name: "Canister, Vented",
        flavor: "Vented deliberately, at 03:40, under the standing authority of the duty officer, who logged the decision in one line and has not been asked about it since. The alternative was leaving it sealed, and the seal had begun making requests.",
        tip: "Long-range siege splash. Poisons the whole blast.",
    },
    Entry {  // 36 — Serpent: 700 normal, heavy 3 s poison
        name: "The Long Occupant",
        flavor: "Lives in the ductwork by preference. By every metric Facilities track, an exemplary tenant.",
        tip: "Single-target normal. One bite, then a strong 3-second poison.",
    },
    Entry {  // 37 — Overloaded Catapult: 780 siege splash, range 600
        name: "Overridden Loading Arm",
        flavor: "The safety limit was a number in a text file. Somebody changed the number.",
        tip: "Short-range siege splash. Bigger rock, bigger hole.",
    },
    Entry {  // 38 — Chaos Claw: 320 chaos, 1.5 s stun, range 900
        name: "Reaching Limb",
        flavor: "Extends from a wall cavity on request. Not always ours.",
        tip: "Single-target chaos. 1.5-second stun.",
    },
    Entry {  // 39 — Net Thrower: 230 normal, range 1200, short stun
        name: "Retrieval Net",
        flavor: "Standard Recovery kit, issued two to a team, rated for objects up to nine feet and four limbs. Personnel are reminded that the net is rated and they are not, and that recovery of the net takes precedence over recovery of the operator wherever the object remains inside it.",
        tip: "Fast long-range single-target. Tangling stun on hit.",
    },
    Entry {  // 40 — Thornburst: 60 piercing, area 400
        name: "Untreated Floor Growth",
        flavor: "Grounds decline to treat it. It reacts only to trespass.",
        tip: "Wide piercing pulse. Hits everything within 400 of the anchor.",
    },
    Entry {  // 41 — Chaotic Spirit: 1600 magic, bounce 8
        name: "Recording, Still Playing",
        flavor: "The tape ran out in 1998 and the machine has been unplugged since. It is a very long scream and it has structure.",
        tip: "Epic magic chain. Wails through eight targets.",
    },
    Entry {  // 42 — Energy Pulse (weapon): 900 magic wave, 2 s stun
        name: "Facility-Wide Discharge",
        flavor: "Every two and a half seconds the lights dim and one problem stops moving.",
        tip: "Point-blank magic sweep. Wide hit, 2-second stun.",
    },
    Entry {  // 43 — Cluster Rockets: 520 chaos, barrage 12
        name: "Twelve Small Problems",
        flavor: "Requisitioned as one problem. Delivered as twelve. Procurement consider this within tolerance.",
        tip: "Long-range chaos barrage. Twelve warheads at scattered targets.",
    },
    Entry {  // 44 — Frost Bomb: 360 piercing splash 150, 3 frost, 0.6 s
        name: "Chill Charge",
        flavor: "Detonation drops the local temperature by forty degrees.",
        tip: "Fast piercing splash. 3 frost across the cluster.",
    },
    Entry {  // 45 — Bouncy Cannonball: 620 normal, bounce 4, 3 s stun
        name: "Ball That Will Not Settle",
        flavor: "Left in the east stairwell in 2007 as a temporary measure, while the store below was reorganised. The store was reorganised. It has been going down the stairs ever since, and the stairwell has been closed to personnel for eleven of those years.",
        tip: "Normal chain. Four hops, each stunning for 3 seconds.",
    },
    Entry {  // 46 — Soulstealer: 2400 normal, bounce 4, life-drain 200
        name: "Withdrawal Instrument",
        flavor: "It takes something and it returns something.",
        tip: "Epic chain to four. Repairs the anchor 200 per enemy hit.",
    },
    Entry {  // 47 — Splasher: 65 normal splash, 0.5 s
        name: "The Bucket",
        flavor: "A bucket. Anomalous only in that it is never empty, which is why Procurement have stopped ordering buckets and started ordering explanations.",
        tip: "Fast normal splash. Rapid small bursts, no status.",
    },
    Entry {  // 48 — Fire Bow: 190 piercing, barrage 4, 4 fire
        name: "Kindling Rack",
        flavor: "The arrows are ordinary. The rack is where they learn.",
        tip: "Piercing barrage. Four shots, each adding 4 fire.",
    },
    Entry {  // 49 — Chaos Web: 180 chaos, bounce 6, light poison
        name: "Structural Webbing",
        flavor: "Engineering have confirmed it is load-bearing.",
        tip: "Chaos chain through six. Leaves a light poison.",
    },
    Entry {  // 50 — Magic Claw: 240 magic, bounce 4, range 300, mana-drain 20
        name: "Grasping Field, Close",
        flavor: "Effective within arm's reach, which is where most of the work here gets done.",
        tip: "Short-range magic chain to four. Restores 20 shield per enemy hit.",
    },
    Entry {  // 51 — Liquid Fire Hurler: 210 siege, 0.33 s, 4 fire, +3 vuln
        name: "The Long Ladle",
        flavor: "Sublevel 4 holds a crucible nobody built.",
        tip: "Very fast siege single-target. 4 fire and +3% damage taken per hit.",
    },
    Entry {  // 52 — Boulder Toss: 980 normal splash, range 300, +5 vuln
        name: "Loose Masonry",
        flavor: "The wall it came out of is intact. We hold photographs of both.",
        tip: "Point-blank normal splash. Leaves +5% damage taken, stacking.",
    },
    Entry {  // 53 — Bloody Spikes (weapon): 2200 normal wave, 1 s stun
        name: "The Floor Of Room 12",
        flavor: "Room 12 is a storage room. The floor of Room 12 is an event. We continue to store things in Room 12.",
        tip: "Epic point-blank sweep. Heavy hit, 1-second stun on everything caught.",
    },
    Entry {  // 54 — Shroom Doom: 3200 chaos area, 60 fire, 2 s stun, summons
        name: "Bloom Event, Contained",
        flavor: "The bloom is contained. What the bloom leaves standing is not contained, but it is on our side, which Legal insist is a different form entirely.",
        tip: "Epic chaos pulse. Massive hit, 60 fire, 2-second stun — and every kill raises a spore ally.",
    },
    Entry {  // 55 — Flame Generator: 1700 magic area, 200 fire, +4 vuln
        name: "Unlicensed Furnace",
        flavor: "Runs on nothing and vents into nothing. It heats six floors.",
        tip: "Epic magic pulse. Buries a pack in 200 fire stacks — the death-explosions do the rest.",
    },
    Entry {  // 56 — Firebreather: 180 piercing splash, 5 fire, +5 vuln
        name: "Exhaust Port",
        flavor: "It was venting outward the entire time. We have only recently established which side is outward.",
        tip: "Fast piercing splash. 5 fire and +5% damage taken per hit.",
    },
    Entry {  // 57 — Lavaspitter: 1800 siege splash, range 1200, 150 fire
        name: "Deep Tap",
        flavor: "Drilled to sixty metres for geothermal, on a survey that cost more than the drilling did. What came up is hotter than geothermal and has developed views about the drill.",
        tip: "Epic long-range siege splash. 150 fire stacks a shell.",
    },
    Entry {  // 58 — Frostbolt: 170 piercing, 0.5 s, 5 frost
        name: "Cold Sliver",
        flavor: "A shard off something larger. We have not located the larger thing.",
        tip: "Fast piercing single-target. 5 frost a hit, toward the freeze at 25.",
    },
    Entry {  // 59 — Living Ice: 300 magic splash 150, 2 frost
        name: "Ice That Spreads",
        flavor: "It does not melt, it relocates. It is heading for the server room.",
        tip: "Magic splash. Spreads 2 frost on impact.",
    },
    Entry {  // 60 — Ice Generator: 1000 magic area 375, 5 frost
        name: "Cold Plant",
        flavor: "Rated to chill one floor. Chills nine. The other eight are not in this building.",
        tip: "Epic magic pulse. 5 frost across a 375 radius — mass freezes.",
    },
    Entry {  // 61 — Ice Spears: 440 normal, barrage 4, 5 frost
        name: "Cold Store Spear Rack",
        flavor: "Counted by the dozen and issued by the four.",
        tip: "Normal barrage. Four hits, 5 frost each.",
    },
    Entry {  // 62 — Knives: 80 piercing, barrage 3, cheap
        name: "Canteen Cutlery",
        flavor: "Requisitioned for the canteen. Recategorised after the canteen incident, which was, in fairness to Procurement, a cutlery incident.",
        tip: "Fast piercing barrage. Three blades, no status.",
    },
    Entry {  // 63 — Blaster: 210 siege, 0.5 s, +5 vuln
        name: "Percussive Tool",
        flavor: "Maintenance's preferred instrument. Maintenance's only instrument.",
        tip: "Fast siege single-target. Leaves +5% damage taken.",
    },
    Entry {  // 64 — Bandit Sniper: 240 normal, range 900, +5 vuln
        name: "Fence Line Rifle",
        flavor: "The scope is sighted on something eleven hundred metres out. There is nothing eleven hundred metres out.",
        tip: "Fast long-range single-target. Marks the target for +5% damage taken.",
    },
    Entry {  // 65 — Bombs: 300 siege splash, range 300, 0.5 s stun
        name: "Hand Charges",
        flavor: "Carried by the crate, lobbed underhand, and not looked back at. That last part is step four of the procedure and is the only step anybody remembers.",
        tip: "Short-range siege splash. Brief half-second stun.",
    },
    Entry {  // 66 — Sting: 300 chaos, range 900, +10 vuln
        name: "Small Offended Thing",
        flavor: "Three centimetres long and deeply personal about it.",
        tip: "Long-range chaos single-target. +10% damage taken a hit, stacking permanently.",
    },
    Entry {  // 67 — Chaos Skull Bomb: 720 chaos splash, 1.5 s stun
        name: "Cranial Charge",
        flavor: "Somebody's, packed with somebody else's.",
        tip: "Chaos splash. Bursts and stuns for 1.5 seconds.",
    },
    Entry {  // 68 — Icebreather: 760 siege splash, 2 frost, +4 vuln
        name: "Cold Exhale",
        flavor: "Recorded once at intake as a sigh. Nothing at this site was breathing at the time of the recording. The recording is four minutes long, and the transcriber has asked to be moved to another department.",
        tip: "Siege splash. Frosts the blast and leaves +4% damage taken.",
    },
    Entry {  // 69 — Frostwave: 720 magic wave 150, 3 frost
        name: "Corridor Chill",
        flavor: "Crosses the length of a corridor in one pass. The corridor is longer on some nights and the pass takes exactly as long.",
        tip: "Point-blank magic sweep. Sweeps wide, 3 frost.",
    },
    Entry {  // 70 — Flamewave: 360 normal wave, 20 fire, +6 vuln
        name: "Rolling Burn",
        flavor: "Fire safety signed it off. It always travels away from the exits.",
        tip: "Point-blank sweep. 20 fire stacks and +6% damage taken.",
    },
    Entry {  // 71 — Chaotic Spirit Bolt: 190 chaos, 0.33 s, life-drain 40
        name: "Flicker",
        flavor: "Present in one frame of every security recording, always nearer than the last, always facing the camera.",
        tip: "Very fast chaos single-target. Repairs the anchor 40 per hit.",
    },
    Entry {  // 72 — Manabolt: 320 magic, range 1200, mana-drain 80
        name: "Siphon Bolt",
        flavor: "Takes whatever the target was running on. Personnel are advised that this category includes personnel.",
        tip: "Very fast long-range magic. Restores 80 shield per enemy hit.",
    },
    Entry {  // 73 — Squirm: 1300 chaos, range 1200, summons larvae
        name: "Brood Jar",
        flavor: "The jar holds a fixed number. The number is fixed at whatever you last counted. While you count, it counts back.",
        tip: "Epic long-range chaos. Every kill it lands hatches a larva ally.",
    },
    Entry {  // 74 — Immolation Aura: 75 magic wave 150, 0.4 s, 2 fire, +3 vuln
        name: "Warm Perimeter",
        flavor: "Personnel report that standing near the anchor is pleasant in February and a disciplinary matter by August.",
        tip: "Rapid magic sweep. Constant small fire and +3% damage taken.",
    },
    Entry {  // 75 — Boom Bloom: 2600 siege wave, 3 s stun, hazard field
        name: "Seed Bed",
        flavor: "Grounds planted it along the approach as ground cover, to specification, in the spring. It is excellent ground cover. It has needed no watering, no cutting and no attention of any kind, and nothing that has crossed it has been recovered from it.",
        tip: "Epic point-blank sweep. 3-second stun, and drops a mine field burning 1000 a tick for 3 seconds.",
    },
    Entry {  // 76 — Quill Burst: 340 piercing splash, range 1200, +10 vuln
        name: "Quill Discharge",
        flavor: "Fires once, across an entire approaching group. It has decided that we count.",
        tip: "Long-range piercing splash. Sprays the cluster for +10% damage taken.",
    },
    Entry {  // 77 — Arcane Burst: 360 magic splash, range 1200, +10 vuln
        name: "Intent, Released",
        flavor: "Held eleven years in a lead box on the theory that intent obeys lead. It did, until the annual review, which it attended.",
        tip: "Long-range magic splash. Detonates and leaves +10% damage taken.",
    },
    Entry {  // 78 — Meteor Barrage: 3400 siege, barrage 8, 4 s reload
        name: "Overhead Delivery",
        flavor: "The site holds no airspace clearance and no delivery contract. It holds a receiving log. Every page of the log is signed at the bottom by a name that is not on the roster, and the log is full.",
        tip: "Epic siege barrage. Eight impacts, then a very long four-second reload.",
    },
    Entry {  // 79 — Ale Launcher: 230 siege splash, life-drain 4
        name: "Keg, Breakroom",
        flavor: "Nobody stocked it. Nobody drinks from it. Everyone who walks past feels marginally better about the corridor.",
        tip: "Fast siege splash. Trickles 4 HP back per enemy hit.",
    },
    Entry {  // 80 — Chaos Bolt: 290 chaos, 0.33 s, range 1200
        name: "Quick Unpleasantness",
        flavor: "Assessed as anomalous, hazardous and rude. Only rude has a box on the form.",
        tip: "Very fast long-range chaos. Chaos ignores armour class.",
    },
    Entry {  // 81 — Rotating Orb of Lightning: 360 magic, area 600
        name: "Orbiting Fixture",
        flavor: "It circles the anchor at head height.",
        tip: "Wide magic pulse. Hits everything within 600, every second.",
    },
    Entry {  // 82 — Lightning Generator: 780 magic, range 1200, mana-drain 20
        name: "Draw-Down Coil",
        flavor: "Wired to hum off whatever the target is running on. Things near it stop humming.",
        tip: "Long-range magic single-target. Restores 20 shield per hit.",
    },
    Entry {  // 83 — Flame Nova: 220 chaos area, 20 fire, +6 vuln
        name: "Thermal Bloom",
        flavor: "Opens outward roughly every second. Fire safety attend and file the identical sentence each time.",
        tip: "Chaos pulse. 20 fire and +6% damage taken to everything close.",
    },
    Entry {  // 84 — Shocker: 130 siege area, 0.6 s, 2 s stun
        name: "Persistent Mains Fault",
        flavor: "Reported in 2009. Ticket still open. The ticket is the only thing keeping this level's population down.",
        tip: "Rapid siege pulse. 2-second stuns, fast enough to lock a crowd down.",
    },
    Entry {  // 85 — Tangle: 1300 normal, 0.4 s, root 1 s
        name: "Holdfast Vine",
        flavor: "It does not want to hurt anything. It wants everything to stay exactly where it is.",
        tip: "Fast epic single-target. Roots the target for 1 second — a root, so +% Stun Duration does not extend it.",
    },
];

/// Procedures, retrofits, clearances and requisitions, by modifier index.
static MODIFIERS: [Entry; NUM_MODIFIERS] = [
    Entry {  // 0 — +10% to all weapon damage
        name: "Blanket Efficacy Memo",
        flavor: "A one-page memo instructing every catalogued object to try ten per cent harder. It is not clear why this works.",
        tip: "+10% to all weapon damage.",
    },
    Entry {  // 1 — +10% piercing
        name: "Sharpening Rota",
        flavor: "Every edged object in the register goes through the wheel on Tuesdays. Two of them go through it unaccompanied.",
        tip: "+10% piercing damage.",
    },
    Entry {  // 2 — +10% siege
        name: "Demolition Clearance",
        flavor: "Standing permission to remove load-bearing walls.",
        tip: "+10% siege damage.",
    },
    Entry {  // 3 — +10% magic
        name: "Field Recalibration",
        flavor: "Ten per cent more of whatever the meters measure. Nobody currently at this site can tell you what the meters measure.",
        tip: "+10% magic damage.",
    },
    Entry {  // 4 — ×1.25 multiplicative
        name: "Sanction: Unrestricted",
        flavor: "One sheet, countersigned by an office that does not answer its telephone.",
        tip: "×1.25 damage — multiplicative, on top of everything else.",
    },
    Entry {  // 5 — Rapidfire, +10% attack speed
        name: "Duty Cycle Override",
        flavor: "The pause between firings was a safety feature. It was documented as a safety feature. It has been shortened.",
        tip: "+10% attack speed.",
    },
    Entry {  // 6 — Bounty Hunter, +50% gold per kill
        name: "Salvage Rights, Extended",
        flavor: "Whatever comes down the corridor becomes yours, contractually, the moment it stops moving.",
        tip: "+50% gold per kill.",
    },
    Entry {  // 7 — Entangled Gold Mine: +20 income/tick, 25% of it healed
        name: "Payroll Splice",
        flavor: "A quarter of the money goes to the anchor's repairs and the repairs go somewhere else. Accounting have drawn a diagram to explain the arrangement to the auditors and to themselves. The diagram loops.",
        tip: "+20 passive gold per tick; 25% of every income payout also repairs you.",
    },
    Entry {  // 8 — IncomePct(20,100): +20% income (catalog NAME says 10%)
        name: "Requisition Cycle: Shortened",
        flavor: "Filed as a ten per cent improvement. It is twenty. The clerk responsible has been promoted twice.",
        tip: "+20% to passive gold income.",
    },
    Entry {  // 9 — IncomePct(50,100): +50% income (catalog NAME says 25%)
        name: "Requisition Cycle: Aggressive",
        flavor: "Same clerk. Same discrepancy. Larger.",
        tip: "+50% to passive gold income.",
    },
    Entry {  // 10 — Transmute: +100% bounty, 5% proc for +200% base
        name: "Assay Bench",
        flavor: "Weighs the remains and prices them.",
        tip: "+100% kill bounty; 5% of kills pay a further +200% of base bounty.",
    },
    Entry {  // 11 — Imbued Masonry: +2000 max HP then +25%
        name: "Poured Reinforcement",
        flavor: "Concrete over concrete over the original concrete, poured to a schedule that has not been reviewed since the schedule was written. The original is still in there and is still the part that holds.",
        tip: "+2000 max HP, then +25% of max HP on top.",
    },
    Entry {  // 12 — +10 flat armour
        name: "Plate Retrofit",
        flavor: "Bolted on during a night shift with no work order and no sign-off.",
        tip: "+10 flat armour — shaved off every incoming hit.",
    },
    Entry {  // 13 — Moonwell: +2000 shield, 10/tick regen
        name: "Reservoir Tap",
        flavor: "Something beneath the site refills faster than we can draw from it.",
        tip: "+2000 shield pool, refilling 10 a tick.",
    },
    Entry {  // 14 — +50 HP regen per tick
        name: "Standing Repair Order",
        flavor: "Maintenance are now permanently assigned to the anchor. They have moved a cot in.",
        tip: "+50 HP repaired per tick.",
    },
    Entry {  // 15 — Evasion: +10 dodge out of 100, capped at 70
        name: "Displacement Tolerance",
        flavor: "The anchor is anchored. It is nonetheless, occasionally, measurably not where the hit lands.",
        tip: "+10% chance to avoid a hit entirely (dodge caps at 70%).",
    },
    Entry {  // 16 — Power Generator: +2% damage, +1% per 30 s
        name: "Generator, Warming Up",
        flavor: "Output climbs the longer it runs. The manual gives no ceiling.",
        tip: "+2% damage now, +1% more every 30 seconds.",
    },
    Entry {  // 17 — Scroll of Chaos: +20% chaos, +3% per 30 s
        name: "Standing Wave Notice",
        flavor: "Chaos compounds while it goes unpaid. That sentence is on the notice, in bold, above a telephone number.",
        tip: "+20% chaos damage now, +3% more every 30 seconds.",
    },
    Entry {  // 18 — Compounding Greed: +10 income, +5 per 30 s
        name: "Interest Accrual Clause",
        flavor: "Buried in the site's lease at clause 14, under a heading about grounds maintenance. Nobody has identified the lessor, but they are being paid and we are being paid more.",
        tip: "+10 passive gold per tick, +5 more every 30 seconds.",
    },
    Entry {  // 19 — Blessed Armor: +10 armour, +5 per 30 s
        name: "Scar Tissue Protocol",
        flavor: "The plating thickens where it has been hit before. The anchor is slowly becoming a map of its worst nights.",
        tip: "+10 armour now, +5 more every 30 seconds.",
    },
    Entry {  // 20 — Focusfire: +25% single-target weapons
        name: "Single-Subject Doctrine",
        flavor: "Section Nine hold that a facility with one problem at a time is a facility with a future.",
        tip: "+25% damage for single-target weapons.",
    },
    Entry {  // 21 — +25% splash weapons
        name: "Overpressure Waiver",
        flavor: "Signed on the understanding that the blast will exceed the room, and that the room was optional.",
        tip: "+25% damage for splash weapons.",
    },
    Entry {  // 22 — +25% barrage weapons
        name: "Volley Discipline Course",
        flavor: "Two days, catered, off site, with a workbook. The catering is the part personnel remember and the part the incident report concerns itself with.",
        tip: "+25% damage for barrage weapons.",
    },
    Entry {  // 23 — +25% area-pulse weapons
        name: "Perimeter Saturation Order",
        flavor: "Everything within arm's reach of the anchor is redesignated an active surface. Signage is pending.",
        tip: "+25% damage for area-pulse weapons.",
    },
    Entry {  // 24 — Wavefire: +25% wave weapons
        name: "Corridor Sweep Authorisation",
        flavor: "Authorises one clean pass down the corridor, end to end, without stopping to identify what is in it. The authorisation renews itself monthly and requires no signature. This was raised at review in 2016 and again in 2019, and the position has not changed.",
        tip: "+25% damage for wave weapons.",
    },
    Entry {  // 25 — +25% bounce weapons
        name: "Applied Ricochet Study",
        flavor: "Three years of research into why things here come back angrier, distilled into a checkbox on a form.",
        tip: "+25% damage for bounce weapons.",
    },
    Entry {  // 26 — Command Aura: +25% for range <= 600
        name: "Close-Quarters Certification",
        flavor: "Certifies personnel to work within six hundred units of an active problem. The certification is one afternoon long.",
        tip: "+25% damage for weapons with range 600 or less.",
    },
    Entry {  // 27 — Trueshot Aura: +25% for range > 600
        name: "Standoff Doctrine",
        flavor: "Anything worth looking at is worth looking at from further away.",
        tip: "+25% damage for weapons with range over 600.",
    },
    Entry {  // 28 — Engineering Upgrade: +100% Common-weapon damage
        name: "Cheap Stock Reappraisal",
        flavor: "The bottom shelf was re-surveyed after somebody noticed it had been doing most of the work.",
        tip: "Doubles the damage of every Common weapon.",
    },
    Entry {  // 29 — Bash: +20% vs stunned
        name: "Downed-Subject Procedure",
        flavor: "Step one: confirm it is down. Step two: continue. There is no step three.",
        tip: "+20% damage to stunned enemies.",
    },
    Entry {  // 30 — Corrosive Poison: +25% vs poisoned
        name: "Follow-Up Exposure Policy",
        flavor: "Finishes what the water supply began. Medical have stopped attending the review meetings.",
        tip: "+25% damage to poisoned enemies.",
    },
    Entry {  // 31 — Potent Poison: +10% applied poison DoT
        name: "Undiluted Concentrate",
        flavor: "The dilution step existed for the handler's benefit, not the subject's. The handler has been reassigned.",
        tip: "+10% to the poison damage your weapons apply.",
    },
    Entry {  // 32 — Dazing Stuns: +50% stun duration
        name: "Extended Hold Order",
        flavor: "Authorises keeping a subject still for half again as long.",
        tip: "+50% stun duration.",
    },
    Entry {  // 33 — Dreadlord Fang: +80 spikes, +8 heal when hit
        name: "Barbed Cladding",
        flavor: "Anything that reaches the anchor leaves a little of itself behind.",
        tip: "+80 retaliation damage when hit, and +8 HP repaired each time a hit lands on you.",
    },
    Entry {  // 34 — +300 spikes
        name: "Hostile Surface Treatment",
        flavor: "Applied over two nights by contractors in full suits who declined to explain the suits and left four sealed tins behind the plant room door.",
        tip: "+300 retaliation damage when hit.",
    },
    Entry {  // 35 — +50% spikes
        name: "Barb Sharpening Schedule",
        flavor: "Weekly. Carried out by an operator with nothing else to do and a growing amount of feeling about it.",
        tip: "+50% to all retaliation damage.",
    },
    Entry {  // 36 — Growing Spikes: +80 now, +10 per 30 s
        name: "Accretion Notice",
        flavor: "The cladding is not maintained. It accumulates. Facilities have reclassified it from equipment to geology.",
        tip: "+80 retaliation now, +10 more every 30 seconds.",
    },
    Entry {  // 37 — Vulnerability Totem: +5 vuln stacks in 1200 every second
        name: "Wide Field Marker",
        flavor: "Everything inside the marker is logged as already injured. The logging appears to be doing the work.",
        tip: "Enemies within 1200 take +5% more damage each second, stacking without limit.",
    },
    Entry {  // 38 — Mask of Death: +1000 max HP, +15 heal on kill
        name: "Attrition Ledger",
        flavor: "Each death out in the corridor is entered as a credit against the anchor's condition.",
        tip: "+1000 max HP, and +15 HP repaired per enemy killed.",
    },
    Entry {  // 39 — +60 heal on kill
        name: "Deep Attrition Clause",
        flavor: "The same arithmetic, four times as generous.",
        tip: "+60 HP repaired each time an enemy dies.",
    },
    Entry {  // 40 — Reanimating Poison: +5 HP per poison tick
        name: "Reclamation Culture",
        flavor: "The culture feeds on whatever it is dissolving and posts the difference back to us, warm, at intervals.",
        tip: "+5 HP repaired every tick an enemy is taking poison damage.",
    },
    Entry {  // 41 — Multiplication Gems: next Common yields 3 extra copies
        name: "Favourable Clerical Error",
        flavor: "The next common requisition will be filled four times. The clerk responsible has been commended and cannot be located.",
        tip: "Your next Common purchase arrives with 3 extra free copies.",
    },
    Entry {  // 42 — Duplicator: next Rare yields 1 extra copy
        name: "Duplicate Filing",
        flavor: "Everything worth having is filed twice here. One copy for the register, one for whatever reads the register at night.",
        tip: "Your next Rare purchase arrives with 1 extra free copy.",
    },
    Entry {  // 43 — Black Market: next Uncommon is free
        name: "Off-Ledger Requisition",
        flavor: "A name written in the margin, and one item leaves the receiving bay.",
        tip: "Your next Uncommon purchase costs nothing.",
    },
    Entry {  // 44 — Magic Treasure: +250 gold, +5 income per 30 s
        name: "Petty Cash Tin (Sealed)",
        flavor: "Two hundred and fifty on opening. It keeps quietly getting heavier on the shelf, which is why it was sealed.",
        tip: "+250 gold immediately, and +5 passive income every 30 seconds.",
    },
    Entry {  // 45 — Ankh: one revive, +2000 max HP
        name: "Continuity Provision",
        flavor: "Guarantees the site remains staffed. It does not specify by whom.",
        tip: "One revive: on a fatal hit, fully repair and gain +2000 max HP instead of dying.",
    },
    Entry {  // 46 — Healing Hand: +25% healing received
        name: "Medical Escalation",
        flavor: "Everything the anchor is given now goes further. Medical are reluctant to explain why they had been holding back.",
        tip: "+25% to all healing you receive.",
    },
    Entry {  // 47 — Living Wood: +2000 max HP, 1.5% missing HP per second
        name: "Structural Growth",
        flavor: "Something is growing through the anchor's plating. It closes the worst wounds fastest.",
        tip: "+2000 max HP, and heals 1.5% of missing HP every second.",
    },
    Entry {  // 48 — Enchanted Moon Arrow: +100% piercing, +1% per Bow
        name: "Revised Fletching Standard",
        flavor: "Every bow in the register now shares one specification, and specifications here have a way of noticing one another.",
        tip: "+100% piercing damage, and +1% more for every Bow, Site-Issued you own.",
    },
    Entry {  // 49 — Refined Explosives: +100% siege, +1% per Mortar Launcher
        name: "Consolidated Firing Tables",
        flavor: "One table for every mortar on site. Each barrel corrects the last.",
        tip: "+100% siege damage, and +1% more for every Requisitioned Mortar you own.",
    },
    Entry {  // 50 — +10% chaos per Death Engine owned
        name: "Duplication Consent Form",
        flavor: "Signing it permits the apparatus to acknowledge its own copies. They have been aware of one another for some time.",
        tip: "+10% chaos damage for every Duplicating Apparatus you own.",
    },
    Entry {  // 51 — Philosopher's Stone: -1000 max HP, +2000 gold, free
        name: "Asset Stripping Order",
        flavor: "Sells a thousand points of the anchor's structure to a buyer who never visits and always pays on time. Payment clears the same afternoon, out of an account the bank will confirm exists and will not discuss.",
        tip: "Free. -1000 max HP, +2000 gold.",
    },
    Entry {  // 52 — Cursed Treasure: -100 regen, +5000 gold, free
        name: "Terminated Maintenance Contract",
        flavor: "Five thousand up front. The repair crew are escorted out through the corridor.",
        tip: "Free. -100 HP regen per tick (this can go negative and drain you), +5000 gold.",
    },
    Entry {  // 53 — GoldPerDamagePct(1,60): 1 gold per 60 damage
        name: "Damage Invoicing",
        flavor: "Every point of harm done in this building is billed to somebody.",
        tip: "Earn 1 gold per 60 damage you deal.",
    },
    Entry {  // 54 — GoldPerDamagePct(1,12): 1 gold per 12 damage
        name: "Itemised Damage Invoicing",
        flavor: "The same arrangement, line by line. The payer has never once queried an item.",
        tip: "Earn 1 gold per 12 damage you deal.",
    },
    Entry {  // 55 — Wartithe: 25% of income into the shield
        name: "Protective Withholding",
        flavor: "A quarter of every payment is diverted into the anchor's field before it reaches the account. Nobody set this up.",
        tip: "25% of every income payout also tops up your shield.",
    },
    Entry {  // 56 — Aegis Protocol: +2500 shield, +400 per 30 s
        name: "Escalating Containment Field",
        flavor: "The field studies each siege as it happens and comes back thicker.",
        tip: "+2500 shield (20 a tick), growing +400 more every 30 seconds.",
    },
    Entry {  // 57 — Improved Masonry: +500 max HP
        name: "Patch Plate",
        flavor: "Cut from a plate that was itself a patch. There is a seam under the seam.",
        tip: "+500 max HP.",
    },
    Entry {  // 58 — Greater Piercing Attacks: +10% piercing
        name: "Bonded Spare Quiver",
        flavor: "Stored beside the first one and honed on the same stone.",
        tip: "+10% piercing damage.",
    },
    Entry {  // 59 — Improved Normal Attacks: +10% normal
        name: "Unremarkable Ordnance Uplift",
        flavor: "No markings and no file. Simply more of it, hitting harder.",
        tip: "+10% normal damage.",
    },
    Entry {  // 60 — Improved Siege Attacks: +10% siege
        name: "Charge Weight Increase",
        flavor: "Approved by raising the number on a form until nobody with authority was still reading the form.",
        tip: "+10% siege damage.",
    },
    Entry {  // 61 — Improved Chaos Attacks: +10% chaos
        name: "Unclassified Ordnance Release",
        flavor: "Signed for with a single initial that corresponds to nobody on the roster. The crates went out.",
        tip: "+10% chaos damage.",
    },
    Entry {  // 62 — +1000 max HP
        name: "Salvaged Hull Section",
        flavor: "Cut off an anchor that stopped answering on Sublevel 6 and welded over the anchor that still does.",
        tip: "+1000 max HP.",
    },
    Entry {  // 63 — +10 armour
        name: "Armour Ration",
        flavor: "One issue per anchor per quarter, drawn against Form 6 and countersigned by the duty supervisor, who is required to inspect the plate for delamination and prior use before it leaves the store, and who has never found any.",
        tip: "+10 flat armour.",
    },
    Entry {  // 64 — +2000 max HP
        name: "Heavy Plate Allocation",
        flavor: "Pulled off an anchor that held this corridor through three incidents.",
        tip: "+2000 max HP.",
    },
    Entry {  // 65 — +50% kill bounty
        name: "Body Count Premium",
        flavor: "Accounting run the tally generous when the tally gets this long. The alternative is a per-item form, in triplicate, for every body, and Accounting have costed the paper.",
        tip: "+50% gold per kill.",
    },
    Entry {  // 66 — Recharge: +2000 shield, then +25% shield regen
        name: "Field Recharge Cycle",
        flavor: "Draws the pool back up a quarter faster than it drains.",
        tip: "+2000 shield (10 a tick), then +25% to shield regeneration.",
    },
    Entry {  // 67 — +20 gold income per tick
        name: "Standing Salvage Line",
        flavor: "A line item that pays out every tick, funded from a budget code that appears in no budget.",
        tip: "+20 passive gold per tick.",
    },
    Entry {  // 68 — Tower Armor: +5 armour
        name: "Scrap Facing",
        flavor: "Half a ration of plate, applied wherever the last incident report said the hits were landing.",
        tip: "+5 flat armour.",
    },
    Entry {  // 69 — +10% attack speed
        name: "Salvaged Gear Kit",
        flavor: "Taken out of something faster and fitted to something that had not asked to be faster.",
        tip: "+10% attack speed.",
    },
    Entry {  // 70 — Renew: +80 regen, then +25% regen
        name: "Repair Crew, Doubled",
        flavor: "Two shifts stitching at once. They work quicker as the damage mounts.",
        tip: "+80 HP regen per tick, then +25% on top of your regen.",
    },
    Entry {  // 71 — Repair Crew: +20 regen
        name: "Repair Crew, One",
        flavor: "One man, one trolley, the entire corridor. He is very calm about it and has been here longer than the corridor.",
        tip: "+20 HP repaired per tick.",
    },
    Entry {  // 72 — Magic Coin: +5 income
        name: "Coin, Returned",
        flavor: "It spends, and it is back in the tin by morning. Finance have written it off eleven times.",
        tip: "+5 passive gold per tick.",
    },
    Entry {  // 73 — Gold Mine: +10 income and +10% income
        name: "Modest Seam",
        flavor: "A small opening in the floor of the receiving bay that pays a little and takes a percentage of interest in you.",
        tip: "+10 passive gold per tick, and +10% to passive income.",
    },
    Entry {  // 74 — +100% kill bounty
        name: "Double Indemnity Schedule",
        flavor: "Every body pays twice. The schedule does not explain the second payment.",
        tip: "+100% gold per kill.",
    },
    Entry {  // 75 — Improved Magic Attacks: +10% magic
        name: "Emitter Tuning Pass",
        flavor: "Ten per cent more output, at the cost of a hum that personnel describe, in the anonymous survey, as expectant.",
        tip: "+10% magic damage.",
    },
    Entry {  // 76 — +40 HP regen
        name: "Supplementary Repair Allocation",
        flavor: "More than this corridor has ever been allocated. It arrived without a covering note or a requesting department.",
        tip: "+40 HP repaired per tick.",
    },
    Entry {  // 77 — Energy Shield: +10000 shield, -30% damage while it holds
        name: "Full Envelope",
        flavor: "The anchor disappears inside its own field. Personnel report that it is quieter in there. Two of them have asked to go back in, and there is no form for that yet.",
        tip: "+10000 shield (50 a tick), and -30% to ALL damage taken while the shield holds.",
    },
    Entry {  // 78 — Evasion: +10% dodge
        name: "Institutional Bad Habits",
        flavor: "The anchor has learned when not to quite be there. Nobody taught it.",
        tip: "+10% chance to dodge a hit (dodge caps at 70%).",
    },
    Entry {  // 79 — Escalating Plunder: +100% bounty, +15% per 30 s
        name: "Indexed Bounty Schedule",
        flavor: "Payments rise with the length of the incident. The incident has no defined end.",
        tip: "+100% kill bounty now, +15% more every 30 seconds.",
    },
    Entry {  // 80 — Living Fortress: +2500 max HP, +500 per 30 s
        name: "Continuous Pour",
        flavor: "The concrete never stops arriving. Nobody drives the trucks, and the trucks are punctual.",
        tip: "+2500 max HP, +500 more every 30 seconds.",
    },
    Entry {  // 81 — Mana Shield: +1000 shield
        name: "Thin Envelope",
        flavor: "Enough field to spend before the plating has to. Medical call it a buffer. Personnel call it first.",
        tip: "+1000 shield (5 a tick).",
    },
    Entry {  // 82 — Mending Engine: +120 regen, +30 per 30 s
        name: "Self-Improving Repair Loop",
        flavor: "It gets better at the job the longer the job goes on. It is the only thing on site that does.",
        tip: "+120 HP regen per tick, +30 more every 30 seconds.",
    },
    Entry {  // 83 — Mastercrafted Masonry: +5000 max HP, +1% damage per 2000 max HP
        name: "Load-Bearing Doctrine",
        flavor: "The site engineer determined that a wall, made thick enough, becomes a weapon. He was correct, and he has been reassigned.",
        tip: "+5000 max HP, and +1% damage for every 2000 max HP you have — recalculated live.",
    },
    Entry {  // 84 — Golden Ring: +200% bounty, +1% damage per 50% bounty
        name: "Procurement Signet",
        flavor: "Worn smooth on the body ledger. The richer the count it keeps, the meaner the register becomes about keeping it.",
        tip: "+200% kill bounty, and +1% damage for every 50% bounty above base.",
    },
    Entry {  // 85 — Arcane Mark: +4000 shield, +20% damage while it holds
        name: "Inward-Facing Sigil",
        flavor: "Cut into the inner face of the field housing, pointing at the anchor.",
        tip: "+4000 shield (20 a tick), and +20% damage while the shield is up.",
    },
    Entry {  // 86 — Maw of Death: +2000 shield, +15 shield per kill
        name: "Reversed Intake",
        flavor: "The field was built to keep things out. At some point it began accepting deliveries.",
        tip: "+2000 shield (10 a tick), and +15 shield restored per enemy killed.",
    },
    Entry {  // 87 — Energy Pulse: +2000 shield; break stuns 1200 for 0.5 s
        name: "Documented Failure Mode",
        flavor: "When the field fails, it fails outward. This was discovered rather than designed and written up afterwards as a feature.",
        tip: "+2000 shield (10 a tick); when it breaks, stun everything within 1200 for half a second.",
    },
    Entry {  // 88 — Poison Armor: +10 armour, +40 spikes, spikes poison
        name: "Septic Cladding",
        flavor: "The welds were never cleaned. Everything that opens the anchor takes a little of the corridor home with it.",
        tip: "+10 armour, +40 retaliation; retaliation also poisons for 3 seconds.",
    },
    Entry {  // 89 — Bloody Spikes: +80 spikes, +20 per hit to +500, resets
        name: "Embedded Debris",
        flavor: "Each blow leaves more of the previous blow in the surface. It clears at the shift change.",
        tip: "+80 retaliation; every hit you take adds +20 more, up to +500. Resets each round.",
    },
    Entry {  // 90 — Blight Aura: +200 regen, 200 damage + poison in 600 per second
        name: "Adjusted Atmosphere",
        flavor: "The air around the anchor is now within tolerance for the anchor and well outside it for everything else. Respirators are stocked at the lift and checked monthly against a list that includes eleven people who no longer work here.",
        tip: "+200 HP regen per tick; every second, 200 damage plus poison to everything within 600.",
    },
];
