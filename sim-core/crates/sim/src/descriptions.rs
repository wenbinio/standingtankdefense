//! Flavor + mechanical tips for every weapon and modifier, keyed by catalog
//! index. **The text itself now lives in [`crate::themes`]** — this module is the
//! stable, pack-agnostic entry point the render layer has always called, bound to
//! the default pack (`wardens`, the Steam build's theme; `docs/12 §12.4`).
//!
//! Prefer `themes::pack("facility").unwrap().weapon_text(i)` when a specific pack
//! is wanted; these two functions exist so `godot/rust/src/lib.rs` and anything
//! else that only ever wants the default keeps working unchanged.

use crate::themes;

/// `(flavor, tip)` for weapon index `i` in the default pack; `("", "")` out of
/// range.
pub fn weapon_text(i: u16) -> (&'static str, &'static str) {
    themes::default_pack().weapon_text(i)
}

/// `(flavor, tip)` for modifier index `i` in the default pack; `("", "")` out of
/// range.
pub fn modifier_text(i: u16) -> (&'static str, &'static str) {
    themes::default_pack().modifier_text(i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content;

    #[test]
    fn arrays_cover_the_catalog() {
        for i in 0..content::WEAPONS.len() as u16 {
            assert!(!weapon_text(i).1.is_empty(), "weapon {i} tip");
        }
        for i in 0..content::MODIFIERS.len() as u16 {
            assert!(!modifier_text(i).1.is_empty(), "modifier {i} tip");
        }
    }

    #[test]
    fn out_of_range_is_empty() {
        assert_eq!(weapon_text(9999), ("", ""));
        assert_eq!(modifier_text(9999), ("", ""));
    }

    /// The migration must be lossless: index 0 and the last index still read
    /// exactly as they did before the text moved into `themes::wardens`.
    #[test]
    fn wardens_text_survived_the_move_verbatim() {
        assert_eq!(
            weapon_text(0),
            (
                "Warden issue, third pattern; the stock is notched once for every gunner the Pale Wardens lost holding this same dirt.",
                "Single-target piercing. The first thing they hand you, and often the last."
            )
        );
        assert_eq!(
            modifier_text(90),
            (
                "A Hexwright rot-totem bolted to the tank, breathing a slow green ruin into the ground around it that the close-pressed never leave clean.",
                "Blight Aura: +200 regen; pulse 200 poison damage in 600 every second."
            )
        );
    }
}
