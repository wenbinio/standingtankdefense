//! Steam-ready match results (`docs/07 §7.6`). Maps the director's authoritative
//! placement to the per-player rows a Steam build writes via `ISteamUserStats`
//! (leaderboards/achievements). Pure data — the actual Steam calls live behind
//! the `steam` adapter (`steam.rs`), not built in this sandbox.

use crate::transport::PeerId;

/// One player's end-of-match record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlayerStats {
    pub player: PeerId,
    /// Final placement (1 = last alive / best).
    pub place: u32,
    /// Finished in the surviving (top) half of the lobby (`docs/02 §2.7`).
    pub win: bool,
    /// Sole survivor — earns a "Last Stand".
    pub last_stand: bool,
}

/// The full set of rows to upload to Steam stats/leaderboards.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct MatchStats {
    pub rows: Vec<PlayerStats>,
}

impl MatchStats {
    /// Build from the director's final placements `(player, place)`. Win = top
    /// half; Last Stand = place 1 (`docs/02 §2.7`).
    pub fn from_placements(placements: &[(PeerId, u32)]) -> MatchStats {
        let n = placements.len() as u32;
        let win_cutoff = n.div_ceil(2); // top half (ceil for odd lobbies)
        let rows = placements
            .iter()
            .map(|&(player, place)| PlayerStats {
                player,
                place,
                win: place <= win_cutoff,
                last_stand: place == 1,
            })
            .collect();
        MatchStats { rows }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(n: u32) -> PeerId {
        PeerId(n)
    }

    #[test]
    fn eight_player_top_half_wins_and_sole_survivor_last_stands() {
        let placements: Vec<(PeerId, u32)> = (1..=8).map(|p| (pid(p), p)).collect();
        let stats = MatchStats::from_placements(&placements);
        // places 1..=4 win, 5..=8 lose; place 1 is the Last Stand.
        for r in &stats.rows {
            assert_eq!(r.win, r.place <= 4, "win cutoff wrong for place {}", r.place);
            assert_eq!(r.last_stand, r.place == 1);
        }
    }

    #[test]
    fn odd_lobby_uses_ceil_for_the_winning_half() {
        let placements: Vec<(PeerId, u32)> = (1..=7).map(|p| (pid(p), p)).collect();
        let stats = MatchStats::from_placements(&placements);
        // ceil(7/2) = 4 winners.
        assert_eq!(stats.rows.iter().filter(|r| r.win).count(), 4);
    }
}
