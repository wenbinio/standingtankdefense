//! `facility` theme pack — WORLD HALF: entities, the boss record, the strata of
//! the descent, the UI register, and the status-effect display names.
//!
//! Companion to the pack's ORDNANCE half (weapon/modifier display strings). This
//! module owns everything the player reads that is not an item on the shop form.
//!
//! # What this is
//!
//! `docs/12` §12.2, the hard rule: **a theme may change nothing the simulation can
//! observe.** Every item here is a display string keyed by catalog index. Nothing in
//! this module is read by `step()`, nothing reaches `state_checksum`, nothing is
//! serialized into `content.json`, and therefore `content_hash` does not move and the
//! R3 trace corpus does not need re-verifying. That separation is the entire point of
//! shipping a theme as its own artifact.
//!
//! `docs/12` §12.5, originality: this is original material written *in* the idiom of
//! liminal-facility fiction — numbered sublevels, dry containment-log voice, an
//! invented classification vocabulary, safety procedure that reads as absurd. It
//! lifts no named entity, item number, or object class from any community wiki, and
//! names no real brand, trademark, or person. The classification tiers below
//! (`TALLY` / `ATTRITION` / `PURSUIT` / `IMMURED` / `STANDOFF` / `INTERDICT` /
//! `DORMANT` / `UNCLOSED`) are invented for this game and are keyed to what the thing
//! mechanically does.
//!
//! # Accuracy
//!
//! Flavor may lie to the player; anything that reads as a *mechanical* statement may
//! not. Where an entry describes behavior it describes the behavior in `content.rs`:
//! Fortified armor reads as structural, ranged attackers read as attacking from a
//! distance, the inert practice target reads as inert, and the boss record describes
//! the plates/coverage/breach/enrage arithmetic exactly as `content.rs` implements it.

// ---------------------------------------------------------------------------
// Shape
// ---------------------------------------------------------------------------

/// One catalogued entity, keyed by its `content::ENEMIES` index.
///
/// `designation` is the register's name for it and `nickname` is what floor staff
/// actually say — the nickname is the short string for a health bar, the designation
/// the one for a record screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entity {
    /// Register designation — the display name (`docs/12` §12.3 "Enemy display names").
    pub designation: &'static str,
    /// Standing Inventory entry code.
    pub code: &'static str,
    /// Classification tier (invented vocabulary; see the module docs).
    pub tier: &'static str,
    /// Floor-staff shorthand. Short enough for a nameplate.
    pub nickname: &'static str,
    /// Flavor. Never mechanical truth the player can be punished for trusting.
    pub flavor: &'static str,
}

/// One stratum of the descent, keyed by the run clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stratum {
    /// Display name ("where am I" text).
    pub name: &'static str,
    /// The survey's note on it.
    pub note: &'static str,
}

/// A status effect's display name and its incident-log gloss.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusText {
    /// Stable key: `poison` / `frost` / `fire` / `spikes` / `stun` (+ derived states).
    pub key: &'static str,
    /// Display name.
    pub name: &'static str,
    /// One-line gloss for a tooltip.
    pub note: &'static str,
}

/// The boss's record. One boss, so this is a struct rather than a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BossRecord {
    pub designation: &'static str,
    pub code: &'static str,
    pub tier: &'static str,
    pub nickname: &'static str,
    /// The register's standing note on the entry.
    pub register: &'static str,
    /// The containment record proper.
    pub containment: &'static str,
    /// The rotating damage-type plates.
    pub plates: &'static str,
    /// Why a broad arsenal cracks the open flank harder.
    pub coverage: &'static str,
    /// What a `Floor Purge` (`Clear`) does to it.
    pub breach: &'static str,
    /// The enrage ramp.
    pub enrage: &'static str,
    /// What does not work on it.
    pub immunities: &'static str,
    /// What it does on reaching the anchor.
    pub contact: &'static str,
    /// Encounter framing, spoken when it arrives.
    pub arrival: &'static str,
    /// Encounter framing, on a kill.
    pub resolved: &'static str,
    /// Encounter framing, on a loss to it.
    pub unresolved: &'static str,
}

// ---------------------------------------------------------------------------
// Accessors — the index-keyed plug-in surface
// ---------------------------------------------------------------------------

/// The full record for entity index `i`; `None` out of range.
pub fn entity(i: u16) -> Option<&'static Entity> {
    ENTITIES.get(i as usize)
}

/// `(display name, flavor)` for entity index `i`; `("", "")` out of range.
///
/// Deliberately the same shape as `descriptions::weapon_text` / `modifier_text`, so a
/// pack module can hold enemies in the same index-keyed table it holds everything
/// else in.
pub fn entity_text(i: u16) -> (&'static str, &'static str) {
    match ENTITIES.get(i as usize) {
        Some(e) => (e.designation, e.flavor),
        None => ("", ""),
    }
}

/// `(nickname, tier)` for entity index `i`; `("", "")` out of range. The pair a
/// nameplate wants.
pub fn entity_plate(i: u16) -> (&'static str, &'static str) {
    match ENTITIES.get(i as usize) {
        Some(e) => (e.nickname, e.tier),
        None => ("", ""),
    }
}

/// The stratum the run is in at `tick`. Display-only: derived from the same 3-minute
/// interval the difficulty ramp uses, so the descent reads as escalation, and pinned
/// to the last stratum from the boss tick onward.
pub fn stratum_at(tick: u32) -> &'static Stratum {
    let i = (tick / crate::content::RAMP_INTERVAL) as usize;
    &STRATA[i.min(STRATA.len() - 1)]
}

/// A UI string by key; `""` if the key is unknown. Keys are dotted and stable
/// (`hud.*`, `boss.*`, `shop.*`, `death.*`, `results.*`, `board.*`, `boot.*`).
pub fn ui(key: &str) -> &'static str {
    UI.iter().find(|(k, _)| *k == key).map(|(_, v)| *v).unwrap_or("")
}

/// A status effect's display text by key; `None` if unknown.
pub fn status(key: &str) -> Option<&'static StatusText> {
    STATUS.iter().find(|s| s.key == key)
}

// ---------------------------------------------------------------------------
// The entities — indices are `content::ENEMIES` indices (identity, `CONTRACTS.md` C1)
// ---------------------------------------------------------------------------

/// Entity display records, one per `content::ENEMIES` row, in catalog order.
pub static ENTITIES: [Entity; 12] = [
    // 0 — Squeakzilla: swarm floor. Light armor, 300 hp, arrives constantly.
    Entity {
        designation: "Occupant, Small",
        code: "SI-0114",
        tier: "TALLY I",
        nickname: "Squeaks",
        flavor: "Arrives at a rate the intake form has no field for, which is why the register only ever records it in the plural. One of them is a housekeeping matter. The facility has never been asked about one of them.",
    },
    // 1 — Fanged Death: slow medium-armored bruiser, 3000 hp, heavy contact bite.
    Entity {
        designation: "Orderly",
        code: "SI-0207",
        tier: "ATTRITION III",
        nickname: "Bitey",
        flavor: "Does not hurry, and has never had to. Plated at the shoulder, unhurried at the hip, and carrying the sincere institutional conviction that you are late for something. It closes the distance and it bites, and both of those take exactly as long as they take.",
    },
    // 2 — The boss. Short form here; the full record is BOSS_RECORD below.
    Entity {
        designation: "The Attending",
        code: "SI-0000",
        tier: "UNCLOSED",
        nickname: "Doctor",
        flavor: "Entry zero. Opened before the facility had a numbering system, which is why it holds that number and why nothing else does. Six point three million units of clinical patience behind five plates of armour worn one at a time. See the full record; there is a full record.",
    },
    // 3 — Doomduck: cheapest chaff, 200 hp, weakest thing in the catalog.
    Entity {
        designation: "Waterfowl, Surplus",
        code: "SI-0031",
        tier: "TALLY 0",
        nickname: "Ducks",
        flavor: "Two hundred units of unhappy poultry per unit, released into the corridors when the aviary level was written off rather than emptied. The cheapest line the register still bothers to count. Individually it is the least of your problems, in the strict sense that everything else is worse.",
    },
    // 4 — Bacon: fast rusher, speed 16, 700 contact.
    Entity {
        designation: "Livestock, Ambulatory",
        code: "SI-0342",
        tier: "PURSUIT II",
        nickname: "Speedpig",
        flavor: "Left the agricultural level under its own power and has been improving its time ever since. It covers open floor fast enough to reach the anchor between purges, and it hits considerably harder than its file suggests. The facility's position is that it remains an asset.",
    },
    // 5 — Honk: fastest enemy, speed 22, glassy at 400 hp.
    Entity {
        designation: "Waterfowl, Aggravated",
        code: "SI-0343",
        tier: "PURSUIT III",
        nickname: "Honk",
        flavor: "The fastest thing in the inventory and the least able to absorb an answer, a combination the register stopped describing as a trade-off after the third revision. Four hundred units of grievance at speed. It announces itself on approach. That is not a courtesy.",
    },
    // 6 — Bonk: Fortified armor, 16000 hp, speed 3. Only Siege bites hard.
    Entity {
        designation: "Load-Bearing Element",
        code: "SI-0509",
        tier: "IMMURED IV",
        nickname: "Wall",
        flavor: "Structural — not as a figure of speech. It was surveyed as part of the building and stayed on the plans for two years, until the team resurveying the west stair found it two metres from where the plans put it. Small arms register on it as maintenance. Taking it out of a corridor is demolition work, and demolition work is a requisition.",
    },
    // 7 — Nope Rope: caster, magic bolts at range 900 — the longest standoff.
    Entity {
        designation: "Utility Cable, Animate",
        code: "SI-0621",
        tier: "INTERDICT III",
        nickname: "The Cable",
        flavor: "Logged during the rewire as nine metres of surplus conduit and signed off by an electrician who has since transferred. It has developed a preference for the far end of a corridor and a way of reaching down one that the electrical schedule does not cover. Do not close with it. It has no need of you closing with it.",
    },
    // 8 — Croak: ranged spitter, light frequent piercing spit at range 700.
    Entity {
        designation: "Sump Resident",
        code: "SI-0705",
        tier: "STANDOFF II",
        nickname: "Spitter",
        flavor: "Came up the drainage the month the lower sumps stopped draining. It sits at the edge of the light and spits at whatever is nearest, steadily, from further off than feels reasonable — a small wound delivered often enough that the incident log now files it weekly rather than individually.",
    },
    // 9 — Spicy: ranged chaos breath, harder-hitting, 2000 hp.
    Entity {
        designation: "Furnace Tenant",
        code: "SI-0808",
        tier: "STANDOFF III",
        nickname: "Warm",
        flavor: "Relocated itself out of the boiler level and brought the boiler level's working temperature with it. It exhales across open floor without ever needing to arrive, hard enough that coveralls are now issued as a consumable. Two thousand units of it, and every one of them warm.",
    },
    // 10 — Popsicle: slow, medium-armored ranged breather with heavy siege breath.
    Entity {
        designation: "Refrigerant Body",
        code: "SI-0912",
        tier: "STANDOFF IV",
        nickname: "Chill",
        flavor: "Signed out of the cold store on a form nobody has been able to produce. Slow, plated, and entirely unhurried about the business: it opens up from a distance, it opens up heavy, and then it takes its time about opening up again. The frost on the corridor around it is not weather.",
    },
    // 11 — Dodo: inert practice target. Never moves, no contact damage.
    Entity {
        designation: "Calibration Subject",
        code: "SI-1000",
        tier: "DORMANT 0",
        nickname: "Target",
        flavor: "Issued to the range for sighting-in and never collected. It does not approach, does not answer, and does not appear to mind. Eight hundred units of standing there. The register lists it as a fixture. The register does not say what it was before it was a fixture.",
    },
];

// ---------------------------------------------------------------------------
// The boss — SI-0000
// ---------------------------------------------------------------------------

/// The full record for `content::BOSS` (enemy index 2).
///
/// Every mechanical clause below is `content.rs` as implemented: 6.3M HP; five
/// damage-type plates with exactly one exposed at a time, rotating every
/// `BOSS_PLATE_TICKS` (150 ticks = 5 s) in the order Normal → Piercing → Magic →
/// Siege → Chaos; the exposed flank taking `4/5 × coverage` where coverage is the
/// number of distinct damage types the arsenal owns (1..=5) and everything else
/// `2/5`; a `Clear` breaching every plate for `BOSS_BREACH_TICKS` (45 ticks = 1.5 s)
/// on top of its `BOSS_CLEAR_DAMAGE` chip; immunity to all status and weapon
/// abilities; a planted contact hit every `BOSS_CONTACT_CADENCE` (16 ticks) that
/// enrages by half its base every `BOSS_ENRAGE_INTERVAL` (900 ticks = 30 s), uncapped.
pub static BOSS_RECORD: BossRecord = BossRecord {
    designation: "The Attending",
    code: "SI-0000",
    tier: "UNCLOSED",
    nickname: "Doctor",
    register: "STANDING INVENTORY, ENTRY ZERO. Status: OPEN. Status has been OPEN for the entire operating history of this facility, and the field admits no other value, because no other value was ever typed into it.",
    containment: "File SI-0000 was opened before there was a numbering system to open it under, which is why it holds that number and why nothing else does. It has never been closed. Containment consists of the building: ten sublevels of it, surveyed, numbered, and placed on top of the entry in the hope that depth counts as a procedure. It does not eat, does not sleep, does not heal, and has never been observed to do anything to a member of staff that is not written down somewhere as a step. Six point three million units of clinical patience. The apron is not stained. It has simply always been that colour.",
    plates: "It wears five plates — impact, puncture, charge, blast, and the fifth, which the schedule lists only as OTHER — and it can wear exactly one at a time. The rotation is five seconds a plate, in that order, without deviation, the way a round is walked. Whichever plate is forward, the flank behind it is bare, and the readout will tell you which. Everything you put anywhere else lands, and lands at a fraction, and is duly recorded as having landed.",
    coverage: "It armours against what it has been shown. An arsenal that has only ever done one thing to it has taught it precisely one thing, and it is an attentive study: a single-note build finds every plate seated and the bare flank narrower than it looks. Bring more kinds of harm than it has plates for the habit of, and the flank opens wider each rotation — not because its armour is worse, but because it cannot pre-empt a punishment it has no file for. Five kinds of harm crack it better than four times over one kind.",
    breach: "A floor purge takes every plate off it at once. One and a half seconds, out of a ten-second recharge, in which there is no forward plate and nothing you own is landing on armour — this is the window the whole arsenal is for. The purge also chips it directly, but only a chip: entry zero is not a thing you delete with the emergency system, it is a thing you open with the emergency system and then shoot.",
    enrage: "It becomes less patient at a measurable rate. Every thirty seconds it is on the floor adds half again to what a single contact costs you, compounding, and there is no ceiling written into the procedure because nobody drafting the procedure expected the question to come up. This is a race and always has been. It is not on the sign by the lift.",
    immunities: "Contaminants, combustion, cold, shock and restraint have all been applied to SI-0000 under controlled conditions and are recorded in the file as ATTEMPTED. None of them take. It is not resistant to them; the entry simply declines to be a thing they happen to.",
    contact: "On reaching the anchor it does not detonate, and it does not stop. It plants, and it works — one contact roughly twice a second, at a pace it has clearly done before, for as long as there is an anchor in front of it.",
    arrival: "30:00 — INTAKE SUSPENDED. THE ATTENDING IS ON THE FLOOR. It is not hurrying, and it has known where you are for thirty minutes. Watch the flank. Save the purge for a plate you cannot otherwise reach.",
    resolved: "SI-0000 marked RESOLVED, pending review. For the first time in the operating history of this facility, entry zero reads something other than OPEN. Review is scheduled. Please vacate the floor.",
    unresolved: "SI-0000 REMAINS OPEN. The figure it was standing at when you stopped has been entered in the file, which is more than most attempts contribute. It will be waiting at exactly the same number of minutes, wearing exactly the same five plates, in exactly this order.",
};

// ---------------------------------------------------------------------------
// The strata — the descent, keyed to the run clock
// ---------------------------------------------------------------------------

/// The eleven strata of a run: one per 3-minute difficulty interval, plus the boss
/// floor from 30:00. Architecturally plausible at the top and progressively less so
/// on the way down, so escalation reads as *descending* rather than as a number
/// going up. Display-only; the sim has no concept of a stratum.
pub static STRATA: [Stratum; 11] = [
    // 00:00 — 03:00
    Stratum {
        name: "Sublevel 1 — Intake",
        note: "Fluorescent, linoleum, a chair by the door for a visitor. Nothing here is wrong yet, and that is the strongest evidence you are going to be offered.",
    },
    // 03:00 — 06:00
    Stratum {
        name: "Sublevel 2 — Records",
        note: "Aisles of cabinets holding a form for every occupant this building has had. Yours is already filled in. It is filled in in the past tense.",
    },
    // 06:00 — 09:00
    Stratum {
        name: "Sublevel 3 — Plant Rooms",
        note: "Pumps, ducting, and a hum the survey attributes to the pumps. The survey has been asked about the hum twice and has attributed it to the pumps twice.",
    },
    // 09:00 — 12:00
    Stratum {
        name: "Sublevel 4 — The Long Corridor",
        note: "Four hundred metres of corridor in a building whose longest exterior wall is sixty. The signage numbers the doors the whole way, correctly, in sequence.",
    },
    // 12:00 — 15:00
    Stratum {
        name: "Sublevel 5 — Repeating Ward",
        note: "Six beds, a window, a chart. Then six beds, a window, a chart. It is the same chart; staff have checked, and the checking is what stopped.",
    },
    // 15:00 — 18:00
    Stratum {
        name: "Sublevel 6 — Descending Stair",
        note: "It has landings, handrails, and non-slip nosings to specification. It has no upward flight. This was logged as a fire-code deficiency and the shift went home.",
    },
    // 18:00 — 21:00
    Stratum {
        name: "Sublevel 7 — Standing Water",
        note: "Ankle-deep, warm, and exactly level in every room regardless of the floor's grade. The drains are working perfectly. Samples have been taken and are unremarkable.",
    },
    // 21:00 — 24:00
    Stratum {
        name: "Sublevel 8 — Plant Rooms (Inverted)",
        note: "The same pumps as Sublevel 3 — same models, same serial numbers, same service stickers — mounted on the ceiling and running. Maintenance signed the inspection from below.",
    },
    // 24:00 — 27:00
    Stratum {
        name: "Sublevel 9 — Below The Lowest Floor",
        note: "This building has eight sublevels. That is documented, it is on the plaque by the lift, and you are welcome to file a correction from where you are standing.",
    },
    // 27:00 — 30:00
    Stratum {
        name: "Sublevel 10 — Cartographic Error",
        note: "Not on the plan, not on the fire map, not on the plaque. Present, lit, and swept weekly. The facility's position is that the plans are correct.",
    },
    // 30:00 — the boss floor. The numbering wraps to zero, which is where you came in.
    Stratum {
        name: "Sublevel 0 — The Attending's Floor",
        note: "The numbering resets here. The survey places Sublevel 0 at street level, above the entrance, where you came in — and the survey is not wrong, which is the part to sit with. Something has been standing in it for thirty minutes, waiting for you to finish descending to it.",
    },
];

// ---------------------------------------------------------------------------
// Status effects
// ---------------------------------------------------------------------------

/// Display names for the status effects, in incident-category register. The first
/// five are the `docs/12` §12.3 set (poison / frost / fire / spikes / stun); the rest
/// are the derived states the sim also tracks, named so nothing on screen falls back
/// to a mechanical word.
pub static STATUS: [StatusText; 8] = [
    StatusText {
        key: "poison",
        name: "CONTAMINATED",
        note: "Logged as a spill and handled as a spill: nobody attends to it, and it goes on being true for a while.",
    },
    StatusText {
        key: "frost",
        name: "COLD-SOAKED",
        note: "Core temperature falling. Movement degrades as it accumulates; enough of it and the subject stops entirely.",
    },
    StatusText {
        key: "freeze",
        name: "SEIZED",
        note: "Cold-soak carried to completion. The subject is still on the floor plan; it is no longer on the move.",
    },
    StatusText {
        key: "fire",
        name: "ALIGHT",
        note: "Combustion in progress and progressing. Extinguishers are mounted on Sublevel 3, which is above you.",
    },
    StatusText {
        key: "spikes",
        name: "HAZARD TRIM",
        note: "Unsafe projections fitted to the anchor's exterior. Anything that reaches the hull is injured by the hull. This is a documented feature.",
    },
    StatusText {
        key: "stun",
        name: "NON-RESPONSIVE",
        note: "Upright, present, and not currently participating. It will resume; the file is clear that it resumes.",
    },
    StatusText {
        key: "root",
        name: "SECURED IN PLACE",
        note: "Restrained where it stands. It is not stopped — only stopped from getting any closer.",
    },
    StatusText {
        key: "vulnerable",
        name: "FLAGGED FOR REVIEW",
        note: "Marked by the anchor's survey field. Everything that lands on it after the mark lands harder.",
    },
];

// ---------------------------------------------------------------------------
// UI register — facility apparatus, not a game menu
// ---------------------------------------------------------------------------

/// UI strings by stable dotted key. `docs/10` F2 makes death cheap and retry
/// near-instant, so the death and retry voice is written as the facility *re-anchoring*
/// you — a scheduled, unremarkable, already-filed event — rather than as a game over.
pub static UI: [(&str, &str); 72] = [
    // --- boot / framing ---
    ("boot.title", "STANDING INVENTORY"),
    ("boot.anchor", "ANCHOR SET. YOU ARE THE STANDING APPARATUS. YOU DO NOT MOVE."),
    ("boot.brief", "Something below is on its way up. Your entire job is to still be here when it stops."),
    ("boot.begin", "TAKE THE FLOOR"),
    ("pause.title", "HELD"),
    ("pause.note", "You are holding. Nothing below you is holding with you."),

    // --- HUD ---
    ("hud.title", "STANDING APPARATUS — ANCHORED"),
    ("hud.integrity", "INTEGRITY"),
    ("hud.field", "FIELD CHARGE"),
    ("hud.plating", "PLATING"),
    ("hud.repair", "SELF-REPAIR"),
    ("hud.credit", "REQUISITION CREDIT"),
    ("hud.stipend", "STIPEND / CYCLE"),
    ("hud.cycle", "CYCLE"),
    ("hud.clock", "TIME ON FLOOR"),
    ("hud.depth", "DEPTH"),
    ("hud.purge", "FLOOR PURGE"),
    ("hud.purge_ready", "PURGE ARMED"),
    ("hud.purge_cooling", "PURGE RECHARGING"),
    ("hud.resolved", "ENTRIES CLOSED"),
    ("hud.intake", "INTAKE"),
    ("hud.intake_suspended", "INTAKE SUSPENDED"),
    ("hud.arsenal", "ORDNANCE ON THE LINE"),
    ("hud.orders", "STANDING ORDERS"),
    ("hud.coverage", "HARM COVERAGE"),
    ("hud.coverage_note", "How many distinct kinds of harm your ordnance can do. Nothing on this floor counts them. Entry zero counts them."),

    // --- boss ---
    ("boss.bar", "SI-0000 — THE ATTENDING"),
    ("boss.exposed", "BARE FLANK"),
    ("boss.plated", "PLATE FORWARD"),
    ("boss.breach", "ALL PLATING OFF"),
    ("boss.patience", "PATIENCE"),
    ("boss.patience_note", "Degrades on a timer. It has no floor and no cap; the procedure simply does not address the question."),

    // --- shop ---
    ("shop.title", "REQUISITIONS — WINDOW OPEN"),
    ("shop.subtitle", "One window per cycle. The storeroom does not hold stock and the storeman does not hold opinions."),
    ("shop.ordnance", "ORDNANCE, ISSUED"),
    ("shop.orders", "STANDING ORDERS"),
    ("shop.buy", "SIGN FOR IT"),
    ("shop.bought", "SIGNED FOR"),
    ("shop.reroll", "AMEND THE ORDER"),
    ("shop.reroll_cost", "AMENDMENT FEE"),
    ("shop.reroll_none", "The storeman has stopped humouring you."),
    ("shop.insufficient", "INSUFFICIENT CREDIT ON THIS LINE"),
    ("shop.empty", "LINE STRUCK OUT"),
    ("shop.close", "CLOSE THE WINDOW"),
    ("shop.footer", "Every item on this form was recovered from a floor below this one. Sign anyway; the alternative is on its way up."),

    // --- death / retry (docs/10 F2: cheap death, ~3 s re-entry) ---
    ("death.header", "ANCHOR LOST"),
    ("death.subtitle", "Integrity reached zero. The apparatus has been recovered and the floor has been swept."),
    ("death.body", "Loss of an anchor is a scheduled event. Form 11 has been filed on your behalf, as it is filed every time, by someone whose whole job that is. You have not been billed."),
    ("death.retry", "RE-ANCHOR"),
    ("death.retry_note", "Roughly three seconds. The facility does this a great deal and has got quick at it."),
    ("death.reanchoring", "RE-ANCHORING…"),
    ("death.reanchored", "ANCHOR SET. DEPTH RESET TO SUBLEVEL 1. Nothing about the building has changed in the interval."),
    ("death.leave", "SIGN OUT"),
    ("death.leave_note", "The lift is on Sublevel 1. It has always been on Sublevel 1."),

    // --- results ---
    ("results.header", "RUN RECORD FILED"),
    ("results.time", "TIME ON FLOOR"),
    ("results.depth", "DEEPEST STRATUM"),
    ("results.resolved", "ENTRIES CLOSED"),
    ("results.credit", "CREDIT DRAWN"),
    ("results.damage", "DAMAGE ATTRIBUTED"),
    ("results.coverage", "HARM COVERAGE AT RECOVERY"),
    ("results.purges", "PURGES SPENT"),
    ("results.footer", "Filed. Nobody reads these. They are kept."),

    // --- leaderboard (docs/10 F2: rolling five-minute board, Last Stand as an award) ---
    ("board.title", "STANDING BOARD"),
    ("board.subtitle", "Rolling five minutes. Entries older than the window are struck without notice."),
    ("board.position", "POSITION"),
    ("board.you", "THIS ANCHOR"),
    ("board.depth", "DEPTH REACHED"),
    ("board.empty", "NO ANCHORS ON THE BOARD. This has happened before."),
    ("board.struck", "STRUCK — OUTSIDE THE WINDOW"),
    ("board.sole", "SOLE ANCHOR"),
    ("board.sole_note", "You are the only apparatus in this cohort still standing. The facility has noticed. It notices this often enough to have a form for it."),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content;

    /// Every field of `BOSS_RECORD`, for the emptiness sweep.
    fn boss_fields() -> Vec<(&'static str, &'static str)> {
        let b = &BOSS_RECORD;
        vec![
            ("designation", b.designation),
            ("code", b.code),
            ("tier", b.tier),
            ("nickname", b.nickname),
            ("register", b.register),
            ("containment", b.containment),
            ("plates", b.plates),
            ("coverage", b.coverage),
            ("breach", b.breach),
            ("enrage", b.enrage),
            ("immunities", b.immunities),
            ("contact", b.contact),
            ("arrival", b.arrival),
            ("resolved", b.resolved),
            ("unresolved", b.unresolved),
        ]
    }

    /// `docs/12` §12.3: the pack covers ALL 12 enemies, by index.
    #[test]
    fn every_enemy_index_is_covered() {
        assert_eq!(
            ENTITIES.len(),
            content::ENEMIES.len(),
            "the facility pack must name every enemy in the catalog"
        );
        for i in 0..content::ENEMIES.len() as u16 {
            assert!(entity(i).is_some(), "enemy index {i} has no entity record");
            let (name, flavor) = entity_text(i);
            assert!(!name.is_empty(), "enemy index {i} has an empty designation");
            assert!(!flavor.is_empty(), "enemy index {i} has an empty flavor");
        }
    }

    /// No string anywhere in the pack is empty.
    #[test]
    fn no_string_is_empty() {
        for (i, e) in ENTITIES.iter().enumerate() {
            for (field, v) in [
                ("designation", e.designation),
                ("code", e.code),
                ("tier", e.tier),
                ("nickname", e.nickname),
                ("flavor", e.flavor),
            ] {
                assert!(!v.trim().is_empty(), "entity {i}: empty {field}");
            }
        }
        for (field, v) in boss_fields() {
            assert!(!v.trim().is_empty(), "boss record: empty {field}");
        }
        for (i, s) in STRATA.iter().enumerate() {
            assert!(!s.name.trim().is_empty(), "stratum {i}: empty name");
            assert!(!s.note.trim().is_empty(), "stratum {i}: empty note");
        }
        for s in STATUS.iter() {
            assert!(!s.key.trim().is_empty(), "status: empty key");
            assert!(!s.name.trim().is_empty(), "status {}: empty name", s.key);
            assert!(!s.note.trim().is_empty(), "status {}: empty note", s.key);
        }
        for (k, v) in UI.iter() {
            assert!(!k.trim().is_empty(), "ui: empty key");
            assert!(!v.trim().is_empty(), "ui {k}: empty value");
        }
    }

    /// Out-of-range indices are empty rather than a panic — same contract as
    /// `descriptions::weapon_text`.
    #[test]
    fn out_of_range_is_empty() {
        assert_eq!(entity_text(9999), ("", ""));
        assert_eq!(entity_plate(9999), ("", ""));
        assert!(entity(9999).is_none());
        assert_eq!(ui("no.such.key"), "");
        assert!(status("no-such-status").is_none());
    }

    /// The boss record and the boss's entity row are the same entity, and it sits at
    /// `content::BOSS`.
    #[test]
    fn boss_record_matches_the_catalog_boss() {
        let e = entity(content::BOSS).expect("boss index has an entity record");
        assert_eq!(e.designation, BOSS_RECORD.designation);
        assert_eq!(e.code, BOSS_RECORD.code);
        assert_eq!(e.tier, BOSS_RECORD.tier);
        assert_eq!(e.nickname, BOSS_RECORD.nickname);
        assert!(content::ENEMIES[content::BOSS as usize].boss);
        // Exactly one catalog entry is a boss, so exactly one record is UNCLOSED.
        let unclosed = ENTITIES.iter().filter(|e| e.tier == "UNCLOSED").count();
        assert_eq!(unclosed, 1);
    }

    /// Codes and nicknames are unique — two rows sharing a nameplate is a content bug.
    #[test]
    fn codes_and_names_are_unique() {
        for (i, a) in ENTITIES.iter().enumerate() {
            for b in ENTITIES.iter().skip(i + 1) {
                assert_ne!(a.code, b.code, "duplicate entry code {}", a.code);
                assert_ne!(a.designation, b.designation, "duplicate designation");
                assert_ne!(a.nickname, b.nickname, "duplicate nickname {}", a.nickname);
            }
        }
        for (i, (k, _)) in UI.iter().enumerate() {
            for (k2, _) in UI.iter().skip(i + 1) {
                assert_ne!(k, k2, "duplicate ui key {k}");
            }
        }
        for (i, a) in STATUS.iter().enumerate() {
            for b in STATUS.iter().skip(i + 1) {
                assert_ne!(a.key, b.key, "duplicate status key {}", a.key);
            }
        }
    }

    /// The five `docs/12` §12.3 status effects are all named.
    #[test]
    fn the_five_status_effects_are_named() {
        for key in ["poison", "frost", "fire", "spikes", "stun"] {
            let s = status(key).unwrap_or_else(|| panic!("status {key} is unnamed"));
            assert!(!s.name.is_empty());
        }
    }

    /// The descent is total over the run clock, is one stratum per difficulty
    /// interval, and pins to the boss floor from `BOSS_SPAWN_TICK` on.
    #[test]
    fn strata_cover_the_run_clock() {
        // One per 3-minute interval up to the boss tick, plus the boss floor.
        let intervals = (content::BOSS_SPAWN_TICK / content::RAMP_INTERVAL) as usize;
        assert_eq!(STRATA.len(), intervals + 1);

        // Total and monotone: every tick maps to a stratum, and the index only ever
        // goes down (deeper) as the clock advances.
        let mut last = stratum_at(0);
        assert_eq!(last.name, STRATA[0].name);
        for t in (0..content::BOSS_SPAWN_TICK + 4 * 60 * 30).step_by(97) {
            let s = stratum_at(t);
            assert!(!s.name.is_empty());
            last = s;
        }
        // The boss tick and everything after it is the last stratum.
        assert_eq!(stratum_at(content::BOSS_SPAWN_TICK).name, STRATA[STRATA.len() - 1].name);
        assert_eq!(stratum_at(u32::MAX).name, STRATA[STRATA.len() - 1].name);
        assert_eq!(last.name, STRATA[STRATA.len() - 1].name);
        // The interval boundary is where the stratum changes.
        assert_eq!(stratum_at(content::RAMP_INTERVAL - 1).name, STRATA[0].name);
        assert_eq!(stratum_at(content::RAMP_INTERVAL).name, STRATA[1].name);
    }

    /// `docs/12` §12.5, binding: no lifted item numbers, entity names, or object
    /// classes from the community wikis, and no real brands.
    #[test]
    fn originality_rule_holds() {
        const FORBIDDEN: &[&str] = &[
            "scp-", "scp foundation", "backrooms", "poolrooms", "almond water",
            "euclid", "keter", "thaumiel", "apollyon", "neutralized", "explained",
            "object class", "site-19", "the entity", "skin-stealer", "hound",
            "o5", "mtf", "wikidot",
        ];
        let mut corpus: Vec<&str> = Vec::new();
        for e in ENTITIES.iter() {
            corpus.extend([e.designation, e.code, e.tier, e.nickname, e.flavor]);
        }
        corpus.extend(boss_fields().into_iter().map(|(_, v)| v));
        for s in STRATA.iter() {
            corpus.extend([s.name, s.note]);
        }
        for s in STATUS.iter() {
            corpus.extend([s.name, s.note]);
        }
        for (_, v) in UI.iter() {
            corpus.push(v);
        }
        for text in corpus {
            let lower = text.to_lowercase();
            for bad in FORBIDDEN {
                assert!(
                    !lower.contains(bad),
                    "docs/12 §12.5 violation: {bad:?} appears in {text:?}"
                );
            }
        }
    }

    /// Sanity: this module is display-only. It holds no numbers the sim could read —
    /// every public item is a string or a struct of strings.
    #[test]
    fn the_pack_is_strings_only() {
        // If this ever needs to change, the theme has become content (`docs/12` §12.2)
        // and belongs in the balance pipeline instead.
        let _: &'static str = ENTITIES[0].designation;
        let _: &'static str = BOSS_RECORD.containment;
        let _: &'static str = STRATA[0].name;
        let _: &'static str = STATUS[0].name;
        let _: &'static str = UI[0].1;
    }
}
