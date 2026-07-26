# 12 — Theme Packs: presentation is not content

## 12.1 Why a pack and not a rename

The catalog already ships one fully-written theme: grim dark fantasy — the Pale Wardens, the Ninth Foundry, the Hexwrights, the fall of Ashmark — with flavor and a mechanical tip for all 86 weapons and 91 modifiers in `descriptions.rs`. That is real work and it is good; it does not get bulldozed.

It is also the wrong register for the Roblox fork. `[09] §2.2` found that Roblox's earners are meme-native — *Toilet Tower Defense* has passed 4 billion visits — and liminal-horror-plus-meme is the dominant idiom of that audience. A **Backrooms / containment-facility / meme** skin is therefore not a whim, it is the market-aligned presentation for `[10]`'s fork.

So: **themes are a swappable presentation layer, and the mechanical catalog is untouched.**

## 12.2 The hard rule

**A theme may change nothing that the simulation can observe.** Names and flavor text are display strings; they never reach `state_checksum`, never alter a draw, never change a cost, a stat, or an id.

Concretely:

- A theme maps **catalog index → display strings**. Indices are identity (`CONTRACTS.md` C1) and are the same in every theme.
- Themes live **outside `content.json`**, in their own artifact, so swapping or adding one does **not** move `content_hash` and does **not** invalidate the R3 trace corpus. This is the whole reason for the separation — a reskin must never cost a re-verification of the sim.
- `descriptions.rs` is already exactly this shape (`(flavor, tip)` keyed by index). A theme pack generalizes it: it adds a **display name** alongside, and covers enemies and arenas too.
- No theme may add, remove or reorder a catalog entry. If a theme wants a mechanic that doesn't exist, that is a content change and belongs in `content.rs` under the balance process (`[11] §11.4`), reviewed separately.

If any of this is violated, the theme has become content and must go through the balance pipeline instead.

## 12.3 What a pack contains

| element | scope |
| --- | --- |
| Weapon display names + flavor + tip | all 86, by index |
| Modifier display names + flavor + tip | all 91, by index |
| Enemy display names + flavor | all 12, by index |
| Boss name + encounter framing | the one boss |
| Arena / level names | the run's "where am I" text |
| Status-effect display names | poison / frost / fire / spikes / stun |
| UI register | how the shop, results and HUD address the player |

## 12.4 The packs

1. **`wardens`** — the existing dark fantasy. Stays as the default for the Steam build. Sourced from `descriptions.rs` unchanged.
2. **`facility`** — the new one, for the Roblox fork: liminal-space dread plus containment-log deadpan plus genuine internet humor. The tank is an anchored, malfunctioning containment apparatus; the run descends through numbered levels; weapons are catalogued anomalous objects; enemies are entities with classifications; the boss is whatever is at the bottom.

## 12.5 Originality and licensing — read before writing

The SCP Foundation wiki and the Backrooms wikis are **community works under CC BY-SA**. That licence permits reuse *with attribution and share-alike*, and share-alike is genuinely awkward for a shipped game — it can pull obligations onto derived material. Specific named entities (SCP-173, SCP-096, individual Backrooms levels and entities) are somebody's authored work, and some names additionally carry trademark exposure.

**So: write original material in the idiom, and do not lift named entities.** The idiom — liminal architecture, numbered levels, dry containment-log voice, classification tiers, redacted text, safety procedures that read as absurd — is a *genre*, and genre is not ownable. That is both the legally clean path and the better creative one, because it lets the writing serve *this* game's mechanics instead of gesturing at somebody else's canon.

Rules for the `facility` pack:
- **No** SCP item numbers, no SCP-wiki entity names, no Object Classes lifted verbatim (invent the tier vocabulary), no Backrooms-wiki level or entity names.
- **No** real trademarks, real people, or real brands.
- **Yes** to original liminal-facility fiction, original classification vocabulary, and memes that are genuinely part of the commons — the *format* of internet humor rather than a specific creator's character.
- Keep it funny **and** unsettling. The joke should not undercut the dread; the best of this genre is deadpan bureaucracy describing something awful.
- Every mechanical **tip** must stay accurate. Flavor may lie to the player; the tip may not.

Attribution for any genre inspiration goes in `CREDITS.md`.
