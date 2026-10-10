#!/usr/bin/env python3
"""Synthetic birdtest worker — speaks the worker API without running MAGPIE.

**Test tooling only.** The one production client is MAGPIE itself (`magpie
contribute`). This script submits *invented* results, and the server cannot
tell them from real ones: pointed at a production server, every result it
submits is recorded as a genuine contribution, feeds match-test verdicts and rating
fits, and has to be found and deleted by hand. Never run it against anything
but a disposable test stack.

It exists so server-side behaviour can be tested at speed and on purpose.
Real games take real time and produce results nobody chose; almost every
interesting server property is about something else:

  * scheduling — deficit-based allocation
  * the claim lifecycle — heartbeat timeouts, stale tokens, reclamation
  * match tests and ratings — which need a *chosen* win rate to reach a known verdict
  * submission validation and the plausibility checks — which need a client
    that misbehaves deliberately

Every mode is deterministic under `--seed`, so a failing CI run reproduces.
"""

import argparse
import json
import logging
import random
import sys
import threading
import time
import uuid
from dataclasses import dataclass, field
from typing import List, Optional, Tuple

logger = logging.getLogger("fake-worker")

# The version every claim reports: past any floor a job sets, so the fake is
# offered every job, and the floor filter is tested in the Rust tiers.
MAGPIE_VERSION = "99.0.0"


@dataclass
class Stats:
    claimed: int = 0
    shutdown: int = 0
    submitted: int = 0
    accepted: int = 0
    rejected: int = 0
    no_work: int = 0
    rate_limited: int = 0
    unavailable: int = 0
    declined: int = 0
    errors: int = 0
    lock: threading.Lock = field(default_factory=threading.Lock)

    def bump(self, name: str) -> None:
        with self.lock:
            setattr(self, name, getattr(self, name) + 1)

    def summary(self) -> str:
        return (
            f"claimed={self.claimed} "
            f"shutdown={self.shutdown} submitted={self.submitted} "
            f"accepted={self.accepted} rejected={self.rejected} "
            f"no_work={self.no_work} rate_limited={self.rate_limited} "
            f"unavailable={self.unavailable} declined={self.declined} errors={self.errors}"
        )


# ---------------------------------------------------------------------------
# Synthetic results
# ---------------------------------------------------------------------------


def _game_outcome(rng: random.Random, p1_win_probability: float) -> int:
    """One game's result for player 1, in half-points: 2 win, 1 tie, 0 loss."""
    roll = rng.random()
    if roll < p1_win_probability:
        return 2
    if roll < p1_win_probability + 0.02:
        return 1  # draws are rare but must be exercised
    return 0


def _aggregate(rng: random.Random, games: int, p1_win_probability: float) -> dict:
    """One synthetic `autoplay` summary — the shape MAGPIE actually reports.

    Draws each game's outcome so the counts have realistic sampling noise
    rather than being the exact expectation, which is what makes match-test runs
    interesting.
    """
    wins = losses = ties = 0
    for _ in range(games):
        outcome = _game_outcome(rng, p1_win_probability)
        wins += outcome == 2
        ties += outcome == 1
        losses += outcome == 0

    return {
        "games": games,
        "wins": wins,
        "losses": losses,
        "ties": ties,
        "p1_score_mean": round(rng.uniform(400, 460), 6),
        "p1_score_sd": round(rng.uniform(45, 70), 6),
        "p2_score_mean": round(rng.uniform(400, 460), 6),
        "p2_score_sd": round(rng.uniform(45, 70), 6),
    }


# The letter distributions a synthetic game knows, by the name a task request
# gives (`letter_distribution`): each letter, its count and its score, in the
# file's order, which is MAGPIE's machine-letter order. The blank is `?`. A
# name not listed plays with English; the fixture's is the e2e suite's own
# (fixtures/versions/*/letterdistributions/english_fixture.csv), so its boards
# show only letters that job can score.
DISTRIBUTIONS = {
    "english": [
        ("?", 2, 0), ("A", 9, 1), ("B", 2, 3), ("C", 2, 3), ("D", 4, 2), ("E", 12, 1),
        ("F", 2, 4), ("G", 3, 2), ("H", 2, 4), ("I", 9, 1), ("J", 1, 8), ("K", 1, 5),
        ("L", 4, 1), ("M", 2, 3), ("N", 6, 1), ("O", 8, 1), ("P", 2, 3), ("Q", 1, 10),
        ("R", 6, 1), ("S", 4, 1), ("T", 6, 1), ("U", 4, 1), ("V", 2, 4), ("W", 2, 4),
        ("X", 1, 8), ("Y", 2, 4), ("Z", 1, 10),
    ],
    "english_fixture": [
        ("?", 1, 0), ("A", 3, 1), ("B", 2, 3), ("C", 2, 3), ("D", 2, 2), ("E", 3, 1),
    ],
}
BOARD_DIM = 15
RACK_SIZE = 7


class _SyntheticGame:
    """A game played by placing random tiles, not words, on a 15x15 board.

    Nothing about it is Scrabble but its bookkeeping, which is what a captured
    position has to get right for the site to draw it: tiles are dealt from a
    bag and drawn back to seven, a play is a straight line through a tile
    already down (the first covers the centre), a blank is played as a lower
    case letter, the scores add up, and each position is written exactly as
    MAGPIE writes one -- the board as CGP, the racks in machine-letter order
    with blanks last, a play as `8G HUH` or `E9 (E)RUVIM`.
    """

    def __init__(self, rng: random.Random, distribution: str, first_seat: int):
        self.rng = rng
        rows = DISTRIBUTIONS.get(distribution, DISTRIBUTIONS["english"])
        self.order = {letter: i for i, (letter, _, _) in enumerate(rows)}
        self.scores = {letter: score for letter, _, score in rows}
        self.letters = [letter for letter, _, _ in rows if letter != "?"]
        # A small distribution is repeated to about a full bag, so a game
        # still runs its twenty-odd turns.
        bag = [letter for letter, count, _ in rows for _ in range(count)]
        self.bag = bag * max(1, -(-100 // len(bag)))
        rng.shuffle(self.bag)
        self.board: List[List[Optional[str]]] = [[None] * BOARD_DIM for _ in range(BOARD_DIM)]
        self.racks: List[List[str]] = [[], []]
        for seat in (0, 1):
            self._draw(seat)
        self.game_scores = [0, 0]
        self.zeros = 0
        self.on_turn = first_seat

    def _draw(self, seat: int) -> None:
        while len(self.racks[seat]) < RACK_SIZE and self.bag:
            self.racks[seat].append(self.bag.pop())

    def rack_string(self, seat: int) -> str:
        return "".join(sorted(self.racks[seat], key=lambda l: (l == "?", self.order[l])))

    def cgp(self) -> str:
        rows = []
        for row in self.board:
            text, empty = "", 0
            for square in row:
                if square is None:
                    empty += 1
                    continue
                if empty:
                    text += str(empty)
                    empty = 0
                text += square
            rows.append(text + (str(empty) if empty else ""))
        return (f"{'/'.join(rows)} {self.rack_string(0)}/{self.rack_string(1)} "
                f"{self.game_scores[0]}/{self.game_scores[1]} {self.zeros}")

    def over(self) -> bool:
        return self.zeros >= 6 or (not self.bag and not all(self.racks))

    def _placement(self, rack: List[str]) -> Optional[dict]:
        """One random play from `rack`, or None if this draw found no room."""
        rng = self.rng
        vertical = rng.random() < 0.5
        # Mostly a few tiles, as real plays are, and now and then the rack.
        count = len(rack) if rng.random() < 0.05 else min(len(rack), rng.choice((1, 2, 2, 3, 3, 3, 4, 4, 5)))
        down = [(r, c) for r in range(BOARD_DIM) for c in range(BOARD_DIM) if self.board[r][c]]
        if down:
            row, col = rng.choice(down)
            back = rng.randint(0, count)
        else:
            # The opening play covers the centre square.
            row = col = BOARD_DIM // 2
            count = max(count, 2)
            back = rng.randint(0, count - 1)
        dr, dc = (1, 0) if vertical else (0, 1)
        row, col = row - dr * back, col - dc * back
        if not (0 <= row < BOARD_DIM and 0 <= col < BOARD_DIM):
            return None
        before = (row - dr, col - dc)
        if 0 <= before[0] < BOARD_DIM and 0 <= before[1] < BOARD_DIM and self.board[before[0]][before[1]]:
            return None
        tiles = rng.sample(rack, count)
        squares = []  # (row, col, letter, placed)
        r, c = row, col
        while tiles or (0 <= r < BOARD_DIM and 0 <= c < BOARD_DIM and self.board[r][c]):
            if not (0 <= r < BOARD_DIM and 0 <= c < BOARD_DIM):
                return None
            if self.board[r][c]:
                squares.append((r, c, self.board[r][c], False))
            else:
                # Nothing alongside a new tile, so the board reads as a
                # crossword rather than a heap.
                for nr, nc in ((r + dc, c + dr), (r - dc, c - dr)):
                    if 0 <= nr < BOARD_DIM and 0 <= nc < BOARD_DIM and self.board[nr][nc]:
                        return None
                tile = tiles.pop()
                letter = rng.choice(self.letters).lower() if tile == "?" else tile
                squares.append((r, c, letter, True))
            r, c = r + dr, c + dc
        if down and all(placed for *_, placed in squares):
            return None
        if len(squares) < 2:
            return None
        word = ""
        for i, (_, _, letter, placed) in enumerate(squares):
            opens = not placed and (i == 0 or squares[i - 1][3])
            closes = not placed and (i == len(squares) - 1 or squares[i + 1][3])
            word += ("(" if opens else "") + letter + (")" if closes else "")
        start = (f"{chr(65 + col)}{row + 1}" if vertical else f"{row + 1}{chr(65 + col)}")
        score = sum(0 if letter.islower() else self.scores[letter] for _, _, letter, _ in squares)
        used = [sq for sq in squares if sq[3]]
        if len(used) == RACK_SIZE:
            score += 50
        return {
            "move": f"{start} {word}",
            "score": score,
            "squares": used,
            "tiles": ["?" if letter.islower() else letter for _, _, letter, _ in used],
        }

    def ranked_moves(self, wanted: int) -> List[dict]:
        """Up to `wanted` distinct plays for the player on turn, best first."""
        rack = self.racks[self.on_turn]
        plays = {}
        for _ in range(wanted * 20):
            if len(plays) >= wanted:
                break
            play = self._placement(rack)
            if play and play["move"] not in plays:
                play["equity"] = round(play["score"] + self.rng.uniform(-6, 6), 3)
                plays[play["move"]] = play
        ranked = sorted(plays.values(), key=lambda p: -p["equity"])
        if len(ranked) < wanted and rack and self.bag:
            swapped = self.rng.sample(rack, self.rng.randint(1, min(len(rack), len(self.bag))))
            ranked.append({
                "move": f"(exch {''.join(sorted(swapped, key=lambda l: (l == '?', self.order[l])))})",
                "score": 0,
                "equity": round(self.rng.uniform(-10, 5), 3),
                "tiles": swapped,
                "squares": [],
            })
        if not ranked:
            ranked.append({"move": "pass", "score": 0, "equity": 0.0, "tiles": [], "squares": []})
        return ranked

    def play(self, move: dict) -> None:
        seat = self.on_turn
        for r, c, letter, _ in move["squares"]:
            self.board[r][c] = letter
        for tile in move["tiles"]:
            self.racks[seat].remove(tile)
        if move["move"].startswith("(exch"):
            self._draw(seat)
            self.bag.extend(move["tiles"])
            self.rng.shuffle(self.bag)
        else:
            self._draw(seat)
        self.game_scores[seat] += move["score"]
        self.zeros = 0 if move["score"] else self.zeros + 1
        self.on_turn = 1 - seat


def _simulates(player: dict) -> bool:
    """Whether MAGPIE simulates a turn of this player's: it does when the
    player has plies to simulate, and ranks statically when it has none."""
    return (player.get("num_plies") or 0) > 0


def _ranked_move(rng: random.Random, move: dict, player: dict, plies_recorded: int) -> dict:
    """One ranked move as MAGPIE writes it for `player`.

    A simulated move carries how often the simulation played it out, its win
    percentage, its blended utility and per-ply statistics, as many plies as
    the player simulates and are recorded; a static player's carries 0
    iterations and nothing more. The server refuses either shape from the
    other player (`plausibility::check_analysis`,
    `check_analyses_against_players`).
    """
    entry = {"move": move["move"], "score": move["score"], "equity": move["equity"], "iterations": 0}
    if not _simulates(player):
        return entry
    # Most for the leaders, in a real simulation.
    entry["iterations"] = rng.randint(20, 400)
    entry["win_percentage"] = round(rng.uniform(20, 80), 3)
    # The win%+spread blend, sometimes used to rank moves instead.
    entry["blended_utility"] = round(rng.uniform(0, 1), 3)
    plies = min(player["num_plies"], plies_recorded)
    if plies:
        entry["plies"] = [
            {
                "ply": p,
                "bingo_percentage": round(rng.uniform(0, 25), 3),
                "average_score": round(rng.uniform(25, 45), 3),
            }
            for p in range(plies)
        ]
    return entry


def _players(request: dict) -> List[dict]:
    """A games or pairs request's players, by seat: player 1 sits in seat 0,
    as MAGPIE seats them, whichever seat moves first."""
    return [request.get("player1") or {}, request.get("player2") or {}]


def _recorded(request: dict) -> Tuple[int, int]:
    """How many plays and plies a captured position keeps: player 1's, which
    MAGPIE reads for the whole run (and job creation holds player 2 to)."""
    player1 = request.get("player1") or {}
    return player1.get("num_plays_recorded") or 10, player1.get("num_plies_recorded") or 0


def _synthetic_position(game: "_SyntheticGame", rng: random.Random, game_index: int,
                        turn: int, ranked: List[dict], previous: Optional[dict],
                        player: dict, plies_recorded: int) -> dict:
    """One captured position of `game` as it stands, its moves `ranked`, as
    `player` -- the one on turn -- analysed it: simulated, or statically."""
    simulates = _simulates(player)
    position = {
        "game_index": game_index,
        "turn_number": turn,
        "rack": game.rack_string(game.on_turn),
        "position": game.cgp(),
        "num_moves": rng.randint(len(ranked), 400),
        "analysis": "sim" if simulates else "static",
        "moves": [_ranked_move(rng, move, player, plies_recorded) for move in ranked],
    }
    # Absent on the first turn of a game: nothing preceded it.
    if previous is not None:
        position["previous_move"] = previous["move"]
        position["previous_move_score"] = previous["score"]
        # What a simmer that infers inferred the opponent kept, from that
        # move: MAGPIE infers only before a simulation, only with
        # `use_inference`, and only from a move, so never on a game's first
        # turn or after a pass.
        if simulates and player.get("use_inference") and (
            previous["score"] or "exch" in previous["move"]
        ):
            position["inference"] = _synthetic_inference(rng)
    # The move played from here: the top of the ranking, which is what the
    # synthetic game plays.
    position["played_move"] = ranked[0]["move"]
    position["played_move_score"] = ranked[0]["score"]
    return position


def _synthetic_inference(rng: random.Random) -> dict:
    """An inference as MAGPIE reports it: how many distinct leaves it found, how
    many it drew, their mean equity, and the most drawn (at most ten), most
    drawn first."""
    found = rng.randint(1, 300)
    total = rng.randint(found, found * 40)
    leaves = []
    draws = total
    for _ in range(min(10, found)):
        draws = rng.randint(1, max(1, min(draws, total // 3 or 1)))
        tiles = rng.randint(0, 6)
        leaves.append({
            "leave": "".join(sorted(rng.choice("AEINORSTLD?") for _ in range(tiles))),
            "draws": draws,
            "equity": round(rng.uniform(-10, 30), 3),
        })
    return {"num_leaves": found, "total_draws": total,
            "average_equity": round(rng.uniform(0, 20), 3), "leaves": leaves}


def _add_first_divergences(result: dict, request: dict, rng: random.Random,
                           divergent_pairs: List[int]) -> None:
    """A pairs task keeping first divergences: from each pair that diverged,
    both games' positions at the turn they first disagree.

    Before that turn a pair's two games are one game with the seats swapped, so
    the two positions share the board and the mover's tiles; the second game's
    CGP has the racks and scores the other way round, its mover in the other
    seat. Its player ranks the moves differently, so the pair diverges here.
    """
    players = _players(request)
    top_moves, top_plies = _recorded(request)
    distribution = request.get("letter_distribution", "english")
    positions = []
    for pair in divergent_pairs:
        game = _SyntheticGame(rng, distribution, first_seat=pair % 2)
        previous = None
        turn = 0
        # The turns both games played alike, before they diverge.
        alike = rng.randint(0, 14)
        while turn < alike and not game.over():
            previous = game.ranked_moves(1)[0]
            game.play(previous)
            turn += 1
        # The first game's mover is the player in the seat on turn; in the
        # second, which started from the other seat, the other player.
        mover, other = players[game.on_turn], players[1 - game.on_turn]
        ranked = game.ranked_moves(top_moves)
        first = _synthetic_position(game, rng, pair * 2, turn, ranked, previous, mover, top_plies)
        # The other game: the same position from the other seat, and another
        # player's ranking, which puts a different move first.
        second_ranked = ranked[1:] + ranked[:1] if len(ranked) > 1 else ranked
        second = _synthetic_position(game, rng, pair * 2 + 1, turn, second_ranked, previous,
                                     other, top_plies)
        board, racks, scores, *rest = second["position"].split(" ")
        swapped_racks = "/".join(reversed(racks.split("/")))
        swapped_scores = "/".join(reversed(scores.split("/")))
        second["position"] = " ".join([board, swapped_racks, swapped_scores, *rest])
        positions += [first, second]
    result["positions"] = positions


def _add_captured_positions(result: dict, request: dict, rng: random.Random,
                            games: int) -> None:
    """Synthesize the per-turn analyses a real worker would capture.

    Only when the job asked for them, so the default path stays the shape every
    existing client produces. Each game is a `_SyntheticGame`, so every
    position is one the site can draw: the board, both racks and scores
    agreeing with the plays before it, the play before it highlighted where it
    was put down.
    """
    if not request.get("capture_positions"):
        return
    players = _players(request)
    top_moves, top_plies = _recorded(request)
    distribution = request.get("letter_distribution", "english")
    positions = []
    for game_index in range(games):
        # MAGPIE alternates the seat that starts; player 1's rack is first in
        # the CGP either way.
        game = _SyntheticGame(rng, distribution, first_seat=game_index % 2)
        previous = None
        # Real games run about 22 turns; varying it exercises the turn bound.
        for turn in range(rng.randint(18, 26)):
            if game.over():
                break
            ranked = game.ranked_moves(top_moves)
            positions.append(_synthetic_position(
                game, rng, game_index, turn, ranked, previous, players[game.on_turn], top_plies
            ))
            # The player plays their top move, as a static player does.
            previous = ranked[0]
            game.play(previous)
    result["positions"] = positions


def _result_for(request: dict, rng: random.Random, p1_win_probability: float) -> dict:
    job_type = request["job_type"]

    if job_type == "games":
        result = {"all_games": _aggregate(rng, request["num_games"], p1_win_probability)}
        _add_captured_positions(result, request, rng, request["num_games"])
        return result

    if job_type == "game_pairs":
        # Built one pair at a time, because the pair is the unit the server
        # evaluates: it scores the pentanomial, five counts indexed by player
        # 1's half-point total across the pair.
        #
        # A pair whose two games played identically is a guaranteed 1-1 split
        # (the same game from both seats), so it lands in bucket 2 and stays in
        # the sample. Those pairs are what make paired play worth doing — they
        # pull the variance down — and dropping them, as the divergent-only
        # view does, would make a hairline difference look enormous.
        pairs = request["num_games"]
        divergence_rate = rng.uniform(0.2, 0.9)
        pentanomial = [0, 0, 0, 0, 0]
        wins = losses = ties = 0
        divergent_games = divergent_wins = divergent_losses = divergent_ties = 0

        divergent_pairs = []
        for pair in range(pairs):
            if rng.random() < divergence_rate:
                divergent_pairs.append(pair)
                # Divergent: the two games are played out independently.
                outcomes = [_game_outcome(rng, p1_win_probability) for _ in range(2)]
                divergent_games += 2
                for outcome in outcomes:
                    divergent_wins += outcome == 2
                    divergent_ties += outcome == 1
                    divergent_losses += outcome == 0
            else:
                # Identical: player 1 takes one seat's win and the other's loss.
                outcomes = [2, 0]

            for outcome in outcomes:
                wins += outcome == 2
                ties += outcome == 1
                losses += outcome == 0
            pentanomial[sum(outcomes)] += 1

        all_games = {
            "games": pairs * 2,
            "wins": wins,
            "losses": losses,
            "ties": ties,
            "p1_score_mean": round(rng.uniform(400, 460), 6),
            "p1_score_sd": round(rng.uniform(45, 70), 6),
            "p2_score_mean": round(rng.uniform(400, 460), 6),
            "p2_score_sd": round(rng.uniform(45, 70), 6),
        }
        result = {
            "all_games": all_games,
            "pentanomial": pentanomial,
            # Still reported, still stored, purely as a diagnostic: it says how
            # often the two configs actually differ. Nothing is tested on it.
            "divergent_games": {
                "games": divergent_games,
                "wins": divergent_wins,
                "losses": divergent_losses,
                "ties": divergent_ties,
                "p1_score_mean": all_games["p1_score_mean"],
                "p1_score_sd": all_games["p1_score_sd"],
                "p2_score_mean": all_games["p2_score_mean"],
                "p2_score_sd": all_games["p2_score_sd"],
            },
        }
        if request.get("capture_positions") and request.get("capture_first_divergence"):
            _add_first_divergences(result, request, rng, divergent_pairs)
        else:
            _add_captured_positions(result, request, rng, pairs * 2)
        return result

    if job_type == "opening_rack":
        # One analysis per rack in the batch, keyed by the rack itself: the
        # request carries `racks` (the server batches them, since the rack space
        # runs to millions) and the response is matched up rack by rack.
        # The job's one player analyses every rack: simulated or static as
        # it is, which is what the server stores the analysis as.
        player = request.get("player") or {}
        analyses = []
        for rack in request["racks"]:
            count = min(rng.randint(2, 6), player.get("num_plays_recorded") or 10)
            moves = []
            # Ranked best-first, so equity descends down the list.
            equity = rng.uniform(20.0, 45.0)
            for i in range(count):
                equity -= rng.uniform(0.5, 4.0)
                # A play uses at least one tile; a one-tile rack is legal, so
                # the lower bound cannot assume two.
                tiles = rng.randint(1, len(rack))
                play = {
                    "move": f"8{chr(ord('D') + i)} {rack[:tiles]}",
                    "score": rng.randint(12, 90),
                    "equity": round(equity, 3),
                }
                moves.append(_ranked_move(rng, play, player, player.get("num_plies_recorded") or 0))
            # How many were ranked before truncation to the job's
            # num_plays_recorded, which is generally far more than is reported.
            # Required: MAGPIE always sends it.
            analyses.append(
                {"rack": rack, "moves": moves, "num_moves": count + rng.randint(0, 200)}
            )
        return {"racks": analyses}

    if job_type == "leave_generation":
        return {
            "racks": [
                {
                    "rack": rack,
                    "count": rng.randint(1, 12),
                    "mean": round(rng.uniform(-8.0, 32.0), 3),
                }
                for rack in request["forced_racks"]
            ]
        }

    raise ValueError(f"unknown job type {job_type!r}")


# Every way `malformed` mode breaks a submission. Each violates a different rule,
# so a run with enough tasks exercises all of them.
CORRUPTIONS = [
    "wrong_type", "missing_field", "inconsistent_counts", "odd_pair_count", "empty",
]


def _corrupt(result: dict, rng: random.Random, choice: Optional[str] = None) -> dict:
    """Produce a submission the server should reject with 400.

    `choice` names one of `CORRUPTIONS`; left out, one is drawn from `rng`,
    which is what `malformed` mode does. `--emit-fixture` names each in turn.
    """
    if choice is None:
        choice = rng.choice(CORRUPTIONS)

    if choice == "empty":
        return {}
    if choice == "wrong_type":
        return {"all_games": "not-an-object", "moves": "not-a-list", "racks": "not-a-list"}
    if choice == "missing_field" and result.get("all_games"):
        stripped = dict(result["all_games"])
        stripped.pop("wins", None)
        return {**result, "all_games": stripped}
    if choice == "inconsistent_counts" and result.get("all_games"):
        # wins + losses + ties must equal games.
        broken = {**result["all_games"], "wins": result["all_games"]["wins"] + 7}
        return {**result, "all_games": broken}
    if choice == "odd_pair_count" and result.get("all_games"):
        # A game_pairs task must report an even number of games, two per pair.
        broken = {**result["all_games"]}
        broken["games"] += 1
        broken["wins"] += 1
        return {**result, "all_games": broken}
    return {"unexpected": True}


def _submission(assignment: dict, mode: str, rng: random.Random,
                p1_win_probability: float) -> Optional[Tuple[str, dict]]:
    """What a worker in `mode` submits for `assignment`: a claim token and a
    result, or None when the mode submits nothing at all (`abandon`).

    The one place a submission is built, shared by the running worker and by
    `--emit-fixture`, so a captured fixture is exactly what a run would send.
    """
    if mode in ("abandon", "time_limit"):
        # Claim and never submit, so the heartbeat timeout has to reclaim the
        # task. Pair with a short HEARTBEAT_TIMEOUT_SECONDS (180, the least the
        # server accepts). A `time_limit` worker declines instead (`run`).
        return None
    token = assignment["claim_token"]
    result = _result_for(assignment["task_request"], rng, p1_win_probability)
    if mode == "malformed":
        result = _corrupt(result, rng)
    if mode == "stale":
        # A token that was never issued must be ignored, not accepted. Drawn
        # from `rng` rather than uuid4() so the mode stays deterministic.
        token = str(uuid.UUID(int=rng.getrandbits(128), version=4))
    return token, result


def _movegens(result: dict) -> int:
    """The `movegens` a submission of `result` reports beside it.

    MAGPIE counts every call to its move generator; the fake generates no
    moves, so it reports a stand-in in proportion to the work its result
    describes -- thirty per game played, fifty per rack -- which is always
    positive and far under what the server refuses as implausible.
    """
    games = result.get("all_games", {}).get("games", 0)
    racks = len(result.get("racks", []))
    return max(1, 30 * games + 50 * racks)


def emit_fixture(args: argparse.Namespace) -> None:
    """Print, without contacting a server, what one worker in `--mode` would
    submit for the assignment in the file `--emit-fixture` names.

    This is how the server's captured-submission fixtures in
    `backend/src/jobs/testdata/` are made -- see the README there. The
    assignment is a claim response (`contract-fixtures/assignment-*.json`);
    each `--override KEY=JSON` replaces one field of its `task_request` first,
    which is how a `game_pairs` or position-capturing request is derived from
    the games assignment. A dotted key reaches into an object:
    `player1.use_inference=true`. The random stream is worker 0's under `--seed`, so
    the output is the first submission a `--workers 1` run would make.

    What is printed depends on the mode:
      normal     the result, as posted under `result`
      stale      the whole body, `{"claim_token": ..., "movegens": ..., "result": ...}`
      malformed  `{variant: result}` for every entry of CORRUPTIONS, rather
                 than the one a run draws at random
      abandon    null: the mode submits nothing
    """
    with open(args.emit_fixture) as f:
        assignment = json.load(f)
    for override in args.override:
        key, _, value = override.partition("=")
        *path, field = key.split(".")
        target = assignment["task_request"]
        for step in path:
            target = target[step]
        target[field] = json.loads(value)
    rng = random.Random(f"{args.seed}:0")

    if args.mode == "malformed":
        result = _result_for(assignment["task_request"], rng, args.p1_win_rate)
        output = {choice: _corrupt(result, rng, choice) for choice in CORRUPTIONS}
    elif args.mode in ("normal", "stale", "abandon"):
        submission = _submission(assignment, args.mode, rng, args.p1_win_rate)
        if submission is None:
            output = None
        elif args.mode == "stale":
            output = {
                "claim_token": submission[0],
                "movegens": _movegens(submission[1]),
                "result": submission[1],
            }
        else:
            output = submission[1]
    else:
        raise SystemExit(f"--emit-fixture has nothing to capture for --mode {args.mode}")
    print(json.dumps(output))


# ---------------------------------------------------------------------------
# One simulated worker
# ---------------------------------------------------------------------------


class FakeWorker:
    # A task costs two requests and the worker endpoints are rate limited per
    # identity, so throttling is expected under load.
    RETRIES = 5

    def __init__(self, args: argparse.Namespace, index: int, stats: Stats):
        self.args = args
        self.stats = stats
        # Jobs this worker has found it cannot run, resent with every claim.
        # In memory only, exactly like MAGPIE's: a client that remembered its
        # limitations across restarts would refuse work it can now do.
        self.unsupported: List[str] = []
        # Set when the server tells the worker it will never be useful again
        # until something on its end changes.
        self.shutdown: Optional[dict] = None
        # Left unset until the server issues one. A client-invented UUID is
        # rejected: birdtest only accepts identities it handed out.
        self.worker_uuid: Optional[str] = None
        # Imported here, so `--emit-fixture` runs on a bare Python: it is what
        # CI re-emits the committed fixtures with.
        import requests

        self.session = requests.Session()
        # Each simulated worker gets its own stream so concurrent runs stay
        # reproducible regardless of thread interleaving.
        self.rng = random.Random(f"{args.seed}:{index}")

    @property
    def headers(self) -> dict:
        if self.worker_uuid:
            return {"X-Worker-UUID": self.worker_uuid}
        # No header at all on the first claim, which is how a worker asks to be
        # issued an identity.
        return {}

    def _url(self, path: str) -> str:
        return f"{self.args.server_url.rstrip('/')}{path}"

    def claim(self) -> Optional[dict]:
        # The body is required: the version drives the per-job floor filter, so
        # a server that had to assume one would be guessing. The board and rack
        # are a default MAGPIE build's, the only one the server dispatches to.
        body = {
            "magpie_version": MAGPIE_VERSION,
            "board_dim": BOARD_DIM,
            "rack_size": RACK_SIZE,
            "unsupported_jobs": self.unsupported,
        }
        response = self.session.post(
            self._url("/api/worker/task"), headers=self.headers, json=body, timeout=30
        )
        if response.status_code == 204:
            # "Nothing right now" -- sleep and ask again. Not the same as a
            # shutdown directive, which says the worker will never be useful
            # until its data or its MAGPIE changes.
            self.stats.bump("no_work")
            return None
        if response.status_code == 429:
            # A throttle, not a lack of work: wait as told, and the run loop
            # asks again.
            self.stats.bump("rate_limited")
            time.sleep(float(response.headers.get("Retry-After", "1")))
            return None
        response.raise_for_status()
        assignment = response.json()
        if "shutdown" in assignment:
            self.shutdown = assignment["shutdown"]
            self.stats.bump("shutdown")
            logger.info(
                "shutdown (%s): %s",
                self.shutdown.get("reason"),
                self.shutdown.get("message"),
            )
            return None
        self.stats.bump("claimed")
        # What MAGPIE prints a task as, and the limit it stops it at. The fake
        # finishes every task at once, so the limit binds only in `time_limit`
        # mode, which hits it every time.
        logger.debug(
            "claimed a task of %s (limit %ss)",
            assignment.get("job_name"),
            assignment.get("max_task_seconds"),
        )
        # Adopt the identity the server assigned and use it from here on.
        if not self.worker_uuid:
            self.worker_uuid = assignment.get("worker_uuid")
        return assignment

    def decline(self, claim_token: str, reason: str) -> None:
        """Hands a claim back, as MAGPIE does a task it stopped at the
        assignment's `max_task_seconds` (`time_limit`)."""
        response = self.session.post(
            self._url("/api/worker/decline"),
            headers=self.headers,
            json={"claim_token": claim_token, "reason": reason},
            timeout=30,
        )
        if response.status_code == 429:
            self.stats.bump("rate_limited")
            return
        response.raise_for_status()
        self.stats.bump("declined")

    def submit(self, claim_token: str, result: dict) -> None:
        # Back off and retry rather than counting a throttle or an outage as a
        # rejection — "the server could not take this now" and "the server
        # refused my data" are the distinction this whole harness exists to
        # make. As MAGPIE does: a 429 or a 5xx (the server's 503 while a job is
        # purged, or while it stores other large results) is retried after its
        # `Retry-After`, and only another 4xx is a refusal.
        for attempt in range(self.RETRIES):
            response = self.session.post(
                self._url("/api/worker/result"),
                headers=self.headers,
                json={"claim_token": claim_token, "movegens": _movegens(result), "result": result},
                timeout=60,
            )
            if response.status_code == 429:
                self.stats.bump("rate_limited")
            elif response.status_code >= 500:
                self.stats.bump("unavailable")
            else:
                break
            if attempt == self.RETRIES - 1:
                self.stats.bump("errors")
                return
            time.sleep(float(response.headers.get("Retry-After", "1")))

        self.stats.bump("submitted")

        if response.status_code >= 400:
            # Expected in `malformed` mode; a finding anywhere else.
            self.stats.bump("rejected")
            logger.info("submission rejected: %s %s", response.status_code, response.text[:200])
            return

        # A stale claim token is silently ignored by design, and reported as
        # accepted=false rather than as an error.
        if response.json().get("accepted"):
            self.stats.bump("accepted")
        else:
            self.stats.bump("rejected")

    def run(self) -> None:
        completed = 0
        while self.args.tasks == 0 or completed < self.args.tasks:
            try:
                assignment = self.claim()
            except Exception:
                self.stats.bump("errors")
                logger.warning("claim failed", exc_info=True)
                time.sleep(self.args.idle_wait)
                continue

            if self.shutdown is not None:
                # Exit cleanly rather than spinning: nothing here is doable
                # until the contributor updates something.
                return

            if assignment is None:
                time.sleep(self.args.idle_wait)
                continue

            try:
                submission = _submission(
                    assignment, self.args.mode, self.rng, self.args.p1_win_rate
                )
            except Exception:
                self.stats.bump("errors")
                logger.exception("could not build a synthetic result")
                continue

            if submission is None:
                if self.args.mode == "time_limit":
                    try:
                        self.decline(assignment["claim_token"], "time_limit")
                    except Exception:
                        self.stats.bump("errors")
                        logger.warning("decline failed", exc_info=True)
                completed += 1
                continue
            token, result = submission

            # Submitted at once, so a claim is never held near the heartbeat
            # timeout and the fake sends no heartbeats (MAGPIE does, every 30
            # s, through a solve that can take minutes).
            try:
                self.submit(token, result)
            except Exception:
                self.stats.bump("errors")
                logger.warning("submit failed", exc_info=True)

            completed += 1


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Synthetic birdtest worker for testing the server without MAGPIE."
    )
    # No default: the only server it may be pointed at is a disposable test
    # stack's (the e2e suite's passes its own), and the development stack's
    # address, the old default, is one it must never be.
    parser.add_argument("--server-url", help="the test stack's backend; required unless --emit-fixture")
    parser.add_argument(
        "--workers", type=int, default=1,
        help="simulated workers running concurrently, for exercising claim races",
    )
    parser.add_argument(
        "--tasks", type=int, default=1,
        help="tasks each worker completes; 0 runs until interrupted",
    )
    parser.add_argument(
        "--mode",
        choices=["normal", "malformed", "stale", "abandon", "time_limit"],
        default="normal",
        help=(
            "normal: plausible results. malformed: submissions the server should "
            "reject. stale: submit under a claim token that was never issued. "
            "abandon: claim and never submit, so the heartbeat timeout reclaims. "
            "time_limit: decline every task as one stopped at its time limit, so "
            "three in a row set the job aside."
        ),
    )
    parser.add_argument(
        "--p1-win-rate", type=float, default=0.5,
        help="bias player 1's results, to drive the match test to a chosen verdict",
    )
    parser.add_argument("--seed", default="birdtest", help="makes a run reproducible")
    parser.add_argument("--idle-wait", type=float, default=1.0)
    parser.add_argument(
        "--emit-fixture", metavar="ASSIGNMENT_JSON",
        help=(
            "print what one worker in --mode would submit for this assignment "
            "and exit, without contacting a server; see emit_fixture()"
        ),
    )
    parser.add_argument(
        "--override", action="append", default=[], metavar="KEY=JSON",
        help="with --emit-fixture: replace one task_request field first",
    )
    args = parser.parse_args()

    if args.emit_fixture:
        emit_fixture(args)
        return
    if not args.server_url:
        parser.error("--server-url is required")

    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")

    stats = Stats()
    workers = [FakeWorker(args, i, stats) for i in range(args.workers)]
    threads = [threading.Thread(target=w.run, daemon=True) for w in workers]

    started = time.monotonic()
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()

    elapsed = time.monotonic() - started
    logger.info("%s in %.1fs", stats.summary(), elapsed)
    print(json.dumps({
        "claimed": stats.claimed,
        "submitted": stats.submitted,
        "accepted": stats.accepted,
        "rejected": stats.rejected,
        "no_work": stats.no_work,
        "rate_limited": stats.rate_limited,
        "unavailable": stats.unavailable,
        "declined": stats.declined,
        "errors": stats.errors,
        "elapsed_seconds": round(elapsed, 3),
    }))

    # Non-zero on transport failures only. A rejection is a *result* here, not
    # an error: `malformed` and `stale` runs expect them.
    sys.exit(1 if stats.errors else 0)


if __name__ == "__main__":
    main()
