# Tower Survivors (WC3) — Map Extraction

Finalized extraction of the inspiration map so **future work doesn't have to repeat it**.
Source: `TowerSurvivors v1.58.w3x` (MoPaQ/MPQ archive), author **Nethalythic**, players **2–8**.

## What's here

```
research/tower-survivors-map/
├── README.md           ← this file
├── extract.py          ← re-runnable extractor + parser (offline-capable)
├── mpyq.py             ← vendored single-file MPQ reader (BSD), used only to open a .w3x
├── raw/                ← files pulled out of the .w3x
│   ├── war3map.wts         (487 KB) trigger strings — THE source of all tooltips/numbers
│   ├── war3map.w3i         map info (name, players, loading screen refs)
│   ├── war3mapMisc.txt     gameplay constants incl. the armor/damage-type matrix
│   ├── war3map.w3u         unit object data (binary)
│   └── war3map.w3a         ability object data (binary)
└── parsed/
    └── catalog.json    ← structured: 118 units, 79 weapons, 87 upgrades, 8 long notes
```

The human-readable writeup of this data is [`docs/appendix-A-map-extraction.md`](../../docs/appendix-A-map-extraction.md); the interpretation is [`docs/01-source-analysis.md`](../../docs/01-source-analysis.md).

## Reproduce

```bash
# Offline — re-parse the committed raw/war3map.wts into parsed/catalog.json:
python3 extract.py

# From scratch — given the original archive:
python3 extract.py /path/to/TowerSurvivors.w3x
```

## What could NOT be extracted

- The compiled **script** (`war3map.j` / `war3map.lua`) is **protected/encrypted** in this build
  (the author distributes an anti-cheat version via Discord). `extract.py` reports it as
  `ENCRYPTED (skipped)`. The script holds wave-spawn timing, the economy tick, and the
  last-man-standing arbitration. Those flow details are reconstructed in the docs from the
  changelog/help strings and labeled *(inferred)* where not directly confirmed.
- The encrypted `(listfile)`; we read files by their well-known WC3 names instead.

To go further, decrypt the script block with a fuller MPQ tool (StormLib / `MPQEditor` /
`CascView`) and a WC3 map deprotector, then read the JASS/Lua. None of the design
conclusions in `docs/` depend on that step.

## Key facts captured (see catalog.json for the full data)

- **2–8 players**, ~**15 min** match + **Samwise** boss (fixed stats; killable only by the tower's `Clear`).
- Damage matrix: **Normal / Piercing / Magic / Siege / Chaos** (+ status flavors **Poison / Frost / Fire / Spikes**); cross-source damage bonuses are **multiplicative**.
- Attack types: **Single Target, Splash(R), Bounce(N), Barrage(N), Wave(+R), Area(R)**.
- Rarity ladder: **Common (500g) → Uncommon → Rare → Epic (~5000g)**.
- Roguelike loop: per-round shop, **5 starting rerolls** (escalating cost), targeted shop
  items (**Black Market, Magic Treasure, Multiplication Gems**).
- Multiplayer-relevant: ships **"Codeless Save and Load (Multiplayer)" + replay detection** —
  evidence the original needed to smuggle per-player state through WC3 sync natives for lack
  of a server. See `docs/01-source-analysis.md` §1.6.
