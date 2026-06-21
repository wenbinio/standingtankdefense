#!/usr/bin/env python3
"""Parse the extracted Tower Survivors stat blocks + upgrades into Rust catalog
literals and insert them into the sim's content.rs between GEN markers.

Maps onto the implemented schema; weapons with abilities the engine doesn't yet
model are imported as their base weapon (damage + attack type + status flavor),
with the exotic ability noted in a comment. Re-runnable.
"""
import json, re, os

HERE = os.path.dirname(os.path.abspath(__file__))
CATALOG = os.path.join(HERE, "parsed", "catalog.json")
CONTENT = os.path.join(HERE, "..", "..", "sim-core", "crates", "sim", "src", "content.rs")

DMG = {"Normal", "Piercing", "Magic", "Siege", "Chaos"}
FLAVORS = {"Poison", "Frost", "Fire", "Spikes"}


def parse_weapon(s):
    s = re.sub(r"\s+", " ", s).strip()
    m = re.match(r"Weapon - (.+?) Attack Type: (.+)", s)
    if not m:
        return None
    head, rest = m.group(1), m.group(2)
    types = [t.strip() for t in head.split("&")]
    base = next((t for t in types if t in DMG), None)
    if not base:
        return None
    flavor = next((t for t in types if t in FLAVORS), None)
    dmg = re.search(r"Damage: (\d+)", rest)
    cd = re.search(r"Attack Cooldown: ([\d.]+|N/A)", rest)
    rng = re.search(r"Range: (\d+)", rest)
    if not (dmg and cd and rng):
        return None
    dmg, rng = int(dmg.group(1)), int(rng.group(1))
    cd_ticks = 30 if cd.group(1) == "N/A" else max(1, round(float(cd.group(1)) * 30))

    prim = rest.split("Damage:")[0].split("&")[0].strip()
    attack = None
    if prim.startswith("Single Target"):
        attack = "Attack::SingleTarget"
    elif (mm := re.match(r"Splash \((\d+)\)", prim)):
        attack = "Attack::Splash(%s)" % mm.group(1)
    elif (mm := re.match(r"Bounce \((\d+)", prim)):
        attack = "Attack::Bounce(%s)" % mm.group(1)
    elif (mm := re.match(r"Barrage \((\d+)", prim)):
        attack = "Attack::Barrage(%s)" % mm.group(1)
    elif (mm := re.match(r"Wave \(\+(\d+)", prim)):
        attack = "Attack::Wave(%s)" % mm.group(1)
    elif (mm := re.match(r"Area \((\d+)\)", prim)):
        attack = "Attack::Area(%s)" % mm.group(1)
    elif prim.startswith("Area"):
        attack = "Attack::Area(%d)" % rng
    if not attack:
        return None
    proj = 0 if attack.startswith(("Attack::Area", "Attack::Wave")) else 45

    poison_dps = poison_ticks = frost = fire = stun = 0
    if flavor == "Poison" or "Poison (" in rest:
        pm = re.search(r"(\d+) damage over (\d+) second", rest)
        if pm:
            total, secs = int(pm.group(1)), int(pm.group(2))
            poison_ticks = secs * 30
            poison_dps = max(1, total // poison_ticks)
    if flavor == "Frost" or "Frost (" in rest:
        fm = re.search(r"Frost \((\d+) stack", rest)
        frost = int(fm.group(1)) if fm else 3
    if flavor == "Fire" or "Fire (" in rest:
        fm = re.search(r"Fire \((\d+) stack", rest)
        fire = int(fm.group(1)) if fm else 3
    sm = re.search(r"Stun \(([\d.]+) second", rest)
    if sm:
        stun = max(1, round(float(sm.group(1)) * 30))

    exotic = None
    for kw in ["Heal", "Summon", "Raises", "drain", "Drain", "Knockback", "Root", "reduce enemy",
               "damage taken", "permanent", "Skeletal", "Infernal", "explode", "burning",
               "liquid fire", "land mine", "Mana", "chance to hit", "bonus damage"]:
        if kw in rest:
            exotic = kw
            break
    return dict(base=base, attack=attack, dmg=dmg, cd=cd_ticks, rng=rng, proj=proj,
                poison_dps=poison_dps, poison_ticks=poison_ticks, frost=frost, fire=fire,
                stun=stun, exotic=exotic, flavor=flavor)


# signatures (base, attack, dmg, cd, rng) of hand-written weapons 0..9 to skip
HAND = {
    ("Piercing", "Attack::SingleTarget", 75, 30, 900),
    ("Siege", "Attack::Splash(300)", 300, 60, 1200),
    ("Magic", "Attack::SingleTarget", 125, 15, 900),
    ("Piercing", "Attack::SingleTarget", 60, 30, 900),
    ("Chaos", "Attack::Splash(150)", 100, 20, 600),
    ("Magic", "Attack::SingleTarget", 400, 45, 600),
    ("Siege", "Attack::Barrage(4)", 100, 30, 1200),
    ("Chaos", "Attack::Area(300)", 80, 30, 300),
    ("Normal", "Attack::Wave(300)", 500, 60, 300),
    ("Piercing", "Attack::Bounce(4)", 150, 30, 600),
}
ARCH = {"Attack::SingleTarget": "Shot", "Attack::Splash": "Blast", "Attack::Bounce": "Glaive",
        "Attack::Barrage": "Volley", "Attack::Wave": "Sweep", "Attack::Area": "Nova"}
COST = {0: 500, 1: 1500, 2: 3000, 3: 5000}


def rarity_of(p):
    special = p["poison_ticks"] or p["frost"] or p["fire"] or p["stun"] or p["exotic"]
    if p["dmg"] > 1000:
        return 3
    if p["dmg"] > 400:
        return 2
    if special or p["dmg"] > 150:
        return 1
    return 0


def emit_weapons(weapons):
    used, out, skipped = set(), [], 0
    for p in weapons:
        if (p["base"], p["attack"], p["dmg"], p["cd"], p["rng"]) in HAND:
            skipped += 1
            continue
        pre = p["flavor"] or p["base"]
        stem = "%s %s" % (pre, ARCH[p["attack"].split("(")[0]])
        name, i = stem, 2
        while name in used:
            name, i = "%s %d" % (stem, i), i + 1
        used.add(name)
        r = rarity_of(p)
        if p["poison_dps"] or p["frost"] or p["fire"] or p["stun"]:
            oh = ("StatusOnHit { poison_dps: %d, poison_ticks: %d, frost_stacks: %d, "
                  "fire_stacks: %d, stun_ticks: %d }" % (p["poison_dps"], p["poison_ticks"],
                                                          p["frost"], p["fire"], p["stun"]))
        else:
            oh = "StatusOnHit::NONE"
        note = ("  // exotic: %s (base only)" % p["exotic"]) if p["exotic"] else ""
        out.append(
            '    WeaponDef { name: "%s", rarity: %d, cost: %d, damage: %d, damage_type: DMG_%s, '
            'attack: %s, cooldown_ticks: %d, range: %d, proj_speed: %d, on_hit: %s },%s'
            % (name, r, COST[r], p["dmg"], p["base"].upper(), p["attack"], p["cd"],
               p["rng"], p["proj"], oh, note))
    return out, skipped


def emit_modifiers(upgrades):
    # Clean, effect-derived names so entries read well (the raw upgrade text often
    # bundles several effects; we import only the part the engine models).
    out, seen, skipped = [], set(), 0
    for u in upgrades:
        t = re.sub(r"\s+", " ", u).strip()
        eff = name = None
        if (m := re.match(r"\+(\d+)% (Normal|Piercing|Magic|Siege|Chaos) Damage$", t)):
            eff = "ModEffect::DamageTypePct(DMG_%s, %s, 100)" % (m.group(2).upper(), m.group(1))
            name = "+%s%% %s Damage" % (m.group(1), m.group(2))
        elif (m := re.match(r"\+(\d+)% Damage$", t)):
            eff = "ModEffect::DamageGlobalPct(%s, 100)" % m.group(1)
            name = "+%s%% Damage" % m.group(1)
        elif (m := re.match(r"\+(\d+)% Attack Speed", t)):
            eff = "ModEffect::AttackSpeedPct(%s, 100)" % m.group(1)
            name = "+%s%% Attack Speed" % m.group(1)
        elif (m := re.search(r"\+(\d+)% (?:Kill Bounty|Bounty Gold)", t)):
            eff = "ModEffect::BountyPct(%s, 100)" % m.group(1)
            name = "+%s%% Kill Bounty" % m.group(1)
        elif (m := re.match(r"\+(\d+) Gold Income", t)):
            eff = "ModEffect::IncomeFlat(%s)" % m.group(1)
            name = "+%s Gold Income" % m.group(1)
        elif (m := re.match(r"\+(\d+) Max HP", t)):
            eff = "ModEffect::MaxHp(%s)" % m.group(1)
            name = "+%s Max HP" % m.group(1)
        elif (m := re.match(r"\+(\d+) Armor", t)):
            eff = "ModEffect::Armor(%s)" % m.group(1)
            name = "+%s Armor" % m.group(1)
        elif (m := re.match(r"\+(\d+) Mana Shield", t)):
            n = int(m.group(1))
            eff = "ModEffect::ManaShield(%d, %d)" % (n, max(1, n // 200))
            name = "+%d Mana Shield" % n
        elif (m := re.match(r"\+(\d+) HP Regen", t)):
            eff = "ModEffect::HpRegen(%s)" % m.group(1)
            name = "+%s HP Regen" % m.group(1)
        elif (m := re.match(r"\+(\d+)% Dodge", t)):
            eff = "ModEffect::Dodge(%s)" % m.group(1)
            name = "+%s%% Dodge" % m.group(1)
        if eff is None or eff in seen:
            skipped += 1
            continue
        seen.add(eff)
        r = mod_rarity(name)
        out.append('    ModifierDef { name: "%s", rarity: %d, cost: %d, effect: %s, ramp: None },'
                   % (name, r, COST[r], eff))
    return out, skipped


def mod_rarity(name):
    num = int(re.search(r"(\d+)", name).group(1))
    if "Mana Shield" in name or "Max HP" in name:
        return 3 if num >= 4000 else 2 if num >= 2000 else 1 if num >= 1000 else 0
    if "Damage" in name or "Attack Speed" in name or "Dodge" in name or "Bounty" in name:
        return 2 if num >= 50 else 1 if num >= 25 else 0
    if "HP Regen" in name:
        return 1 if num >= 80 else 0
    if "Armor" in name:
        return 1 if num >= 20 else 0
    return 0


def splice(marker, lines):
    src = open(CONTENT).read()
    begin, end = "// GEN-%s-BEGIN" % marker, "// GEN-%s-END" % marker
    b = src.index(begin) + len(begin)
    e = src.index(end)
    body = " (generated by research/tower-survivors-map/gen_catalog.py)\n" + "\n".join(lines) + "\n    "
    open(CONTENT, "w").write(src[:b] + body + src[e:])


def main():
    # NOTE: this is a ONE-TIME bootstrap. The catalog in content.rs has since
    # been hand-curated (weapon NAMES were assigned by review; some on_hit/cost
    # values tuned). Re-running would regenerate generic names and overwrite that
    # work, so it requires an explicit --force and you should reconcile names
    # afterwards. Treat content.rs as canonical going forward.
    import sys
    if "--force" not in sys.argv:
        print("refusing to run: content.rs is now hand-curated; pass --force to "
              "regenerate (this OVERWRITES curated weapon names).")
        return
    d = json.load(open(CATALOG))
    weapons = [p for p in (parse_weapon(w) for w in d["weapons"]) if p]
    seen, uniq = set(), []
    for p in weapons:
        k = (p["base"], p["attack"], p["dmg"], p["cd"], p["rng"], p["poison_ticks"],
             p["frost"], p["fire"], p["stun"])
        if k not in seen:
            seen.add(k)
            uniq.append(p)
    wlines, wskip = emit_weapons(uniq)
    mlines, mskip = emit_modifiers(d["upgrades"])
    splice("WEAPONS", wlines)
    splice("MODIFIERS", mlines)
    print("weapons: parsed %d, unique %d, emitted %d (skipped %d hand-dupes)"
          % (len(weapons), len(uniq), len(wlines), wskip))
    print("modifiers: emitted %d, skipped %d (unsupported/duplicate)" % (len(mlines), mskip))


if __name__ == "__main__":
    main()
