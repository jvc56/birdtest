# Capturing Position Analyses From Games

**Status: proposed, not implemented.** Nothing in the code stores in-game
position analyses today; `game_results` holds only the per-batch aggregate.

## The idea

A worker playing a game already analyzes a position on every turn: it generates
candidate moves, ranks them, and picks one. Those analyses are discarded. A
games or game-pairs job should be able to keep them, so a job run to settle an
Elo question also produces a corpus of analyzed positions.

## What is actually capturable, and what it costs

This is the constraint that shapes everything else, and it splits by player type.

**A simming player's analysis is free.** `autoplay_worker->move_lists[player]` is
sized by that player's `num_plays` (`-np1` / `-np2`) and, at the moment a move is
chosen, holds exactly that many candidates ranked by simulation. The work is
already done and thrown away; capturing it costs only serialization.

**A static player's analysis does not exist yet.** Static play calls
`get_top_move_for_player_on_turn`, which forces `MOVE_RECORD_BEST`
([gameplay.c](https://github.com/jvc56/MAGPIE/blob/main/src/impl/gameplay.c)), so
the move list ends up holding one entry. Capturing a *ranked list* from a static
player means relaxing that override, which makes every turn of every game record
and sort moves it currently discards. That is a real slowdown on the job's
primary purpose.

So the honest framing is:

| Player | Capture cost | What you get |
|---|---|---|
| Simming | Serialization only | The simulated ranking the player actually used |
| Static | Slower move generation on every turn | A ranked list the player did not need |

The feature is most defensible for simming players. For static players it should
be possible but clearly marked as slowing the job down.

## Volume is the dominant constraint

A game runs about **22.5 turns** (measured). Two games per pair. So:

| Job | Positions | Move rows at 10 kept each |
|---|---|---|
| `max_pairs = 40,000` | 1,800,000 | 18,000,000 |
| `max_games = 400,000` | 9,000,000 | 90,000,000 |

Capturing every position of every game is not viable as a default. **Capture is
off unless configured, and sampled when on.**

### Redundancy makes it worse, and the fix is cheap

Games are seeded and deterministic, so with `redundancy > 1` every worker on a
task plays *identical* games and would capture *identical* positions. That is
pure duplication -- X copies of the same analysis.

Keying captured positions on `(task_id, game_index, turn_number)` rather than on
the claim, with `ON CONFLICT DO NOTHING`, makes the first accepted claim the one
that lands and the rest no-ops. Redundancy keeps doing its job for the *result*
(agreement between workers is still checked) without multiplying the corpus.

## Schema

The existing `position_analysis_records` / `position_analysis_moves` /
`position_analysis_plies` tables are the right home. This is exactly the
distinction already drawn for opening racks: the *request* is job-type-specific,
but what comes back **is a position analysis** regardless of what produced it.

Two changes are needed:

1. **A surrogate key.** The record is currently keyed `(task_claim_id, rack)`,
   which cannot address an in-game position -- the same rack recurs across turns
   and games. Replace it with `id BIGSERIAL PRIMARY KEY`, and have
   `position_analysis_moves` reference that one column instead of the pair.
2. **Provenance columns**, null for opening racks:

```sql
CREATE TABLE position_analysis_records (
    id             BIGSERIAL PRIMARY KEY,
    task_claim_id  UUID NOT NULL REFERENCES task_claims(id) ON DELETE CASCADE,
    task_id        UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    rack           TEXT NOT NULL,
    -- CGP of the position analyzed. NULL for an opening rack, where the board
    -- is empty by definition and the rack is the whole position.
    position       TEXT,
    -- In-game positions only: which game of the batch, and which turn.
    game_index     SMALLINT,
    turn_number    SMALLINT,
    num_moves      INT NOT NULL,
    submitted_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One captured analysis per position per task, so redundant claims replaying
-- identical games do not multiply the corpus.
CREATE UNIQUE INDEX position_analysis_records_in_game_idx
    ON position_analysis_records (task_id, game_index, turn_number)
    WHERE game_index IS NOT NULL;

-- Unchanged for opening racks: one analysis per rack per claim.
CREATE UNIQUE INDEX position_analysis_records_rack_idx
    ON position_analysis_records (task_claim_id, rack)
    WHERE game_index IS NULL;
```

## Configuration

On both `job_game_config` and `job_game_pair_config`:

| Column | Default | Meaning |
|---|---|---|
| `capture_positions` | `false` | Off unless asked for |
| `capture_sample_rate` | `0.01` | Fraction of positions kept |
| `capture_top_moves` | `10` | Ranked moves stored per captured position -- the same distinction as `top_moves_stored`, separate from how many the player generated |
| `capture_max_turn` | `NULL` | Only turns at or below this, so a job can target openings |
| `capture_max_per_task` | `1000` | Safety valve against a misconfigured rate |

**Sampling must be deterministic.** Derive the decision from
`hash(seed, game_index, turn_number) < rate` rather than from a random number
generator, so re-running a task captures the same positions. Without that,
redundant claims sample *different* positions and the `ON CONFLICT` dedup above
stops working.

A later refinement worth more than a blanket rate: capture only positions where
the top two candidates are within some equity margin. Those are the decisions
that discriminate between strategies, and they are a small fraction of turns.
It needs more than one candidate to exist, so it only applies to simming players.

## MAGPIE changes

A new autoplay recorder, `positions`, selectable through the existing options
string (`autoplay games,positions`) the way `games`, `winpct` and `leaves`
already are.

- It needs a per-turn hook. `game_data_add_game` fires once per game; this fires
  once per turn, after the move is chosen and while `move_lists[player]` still
  holds the candidates.
- It records the CGP, the rack, the turn number, and the top `capture_top_moves`
  entries of the move list.
- It must be bounded in memory: a batch of 20 pairs at 22.5 turns is 900 turns,
  and an unbounded accumulator over a large batch is a leak in all but name.
  Cap it at `capture_max_per_task` and stop recording past that.
- The sampling predicate belongs on the MAGPIE side, so unsampled positions cost
  nothing rather than being serialized and discarded by the server.

For static players, capture additionally requires an option that relaxes the
`MOVE_RECORD_BEST` override in `get_top_move_for_player_on_turn` -- and that
option should be what makes the slowdown visible, rather than it being an
invisible consequence of a birdtest setting.

## Wire format

`GameResultsResponse` gains an optional array alongside the aggregates it
already carries:

```json
{
  "all_games": { "...": "..." },
  "divergent_games": { "...": "..." },
  "positions": [
    { "game_index": 0, "turn_number": 3,
      "rack": "AEINRST",
      "position": "15/15/... AEINRST/ 0/0 0",
      "num_moves": 412,
      "moves": [ { "move": "8D RETAINS", "score": 74, "equity": 81.2 } ] }
  ]
}
```

Absent when capture is off, which keeps every existing client valid.

Server-side validation should reject positions outside the task's own games
(`game_index` beyond the batch) and cap the array length at
`capture_max_per_task`, since a submission carrying millions of positions is
otherwise an easy way to exhaust the database.

## Phasing

1. **Schema** -- surrogate key, provenance columns, the two partial unique
   indexes. Independent of MAGPIE and worth doing first, since it also
   simplifies `position_analysis_moves`'s foreign key.
2. **Config and validation** -- the five columns, the wire field, the server-side
   caps. Testable end to end with the fake worker before MAGPIE can produce
   anything.
3. **MAGPIE `positions` recorder** for simming players, where the analysis is
   already computed.
4. **Static-player capture**, behind an option that names its cost.
5. **Equity-margin filtering**, if the blanket sample rate proves too blunt.

## Open questions

1. **Is the corpus worth the storage?** A job producing 18M move rows as a side
   effect of settling an Elo question is a significant commitment. It may be that
   what is actually wanted is a much smaller, targeted capture -- opening turns
   only, or close decisions only -- rather than a sampled cross-section.
2. **Should captured positions feed the opening rack tables at all?** An opening
   rack job analyzes turn 1 exhaustively; a games job would capture turn 1
   positions incidentally, with a different player config. Sharing a table means
   queries must always filter by job, or by `position IS NULL`.
3. **Does anything read this yet?** No dashboard surface is proposed here. Without
   a consumer, the feature is a write-only corpus, and the sample rate should
   probably start much lower than 1%.
