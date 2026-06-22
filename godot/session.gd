# Session hand-off (autoload `Session`). A tiny render-only bridge from the lobby
# to the match: the lobby plans a session via StLobby (host-authoritative seed +
# player set) and stows the result here so Match can launch with exactly those
# parameters. COSMETIC/PLUMBING ONLY — never feeds the deterministic sim's RNG
# beyond the seed the director itself owns.
#
# Defaults of 0 mean "unset": Match falls back to its standalone demo behavior
# when nothing came from a lobby. The lobby fills these in right before it calls
# change_scene_to_file("res://Match.tscn"); Match reads them once and clears them.
extends Node

# The authoritative plan seed handed back by StLobby.plan_seed() after try_start.
var lobby_seed: int = 0
# Number of PEERS in the planned session (NOT counting the host). Match adds the
# host back with `lobby_players + 1`. Derived from plan_player_count() - 1.
# 0 = no lobby launch pending; Match uses its own N.
var lobby_players: int = 0

# Clear the hand-off after Match consumes it, so a later direct (non-lobby) launch
# of Match.tscn doesn't accidentally reuse a stale plan.
func clear() -> void:
	lobby_seed = 0
	lobby_players = 0
