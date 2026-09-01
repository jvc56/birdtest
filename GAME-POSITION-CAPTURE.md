# Capturing Position Analyses From Games

**Status: implemented.** Verified end to end: MAGPIE captured 98 positions
across 4 real games, with the CGP evolving turn by turn, and the redundancy
deduplication held under `redundancy = 2`.

The asymmetry this plan is built around is directly visible in the stored data.
A `games` job pairing a simming player against a static one, the player config's `num_plays_recorded` of 6:

| Turn | Player | Ranked | Stored |
|---|---|---|---|
| 0 | simming | 6 | 6 |
| 1 | static | 1 | 1 |
| 2 | simming | 6 | 6 |
| 3 | static | 1 | 1 |

Players alternate, and so does what is capturable. Capturing a real ranked list
from a static player still needs the `MOVE_RECORD_BEST` override relaxed
(phase 5, not done).

**One semantic worth knowing:** for a simming player the `rank` is the
simulation's ordering while the stored `equity` is the *static* equity, so the
two disagree -- a captured position can show rank 1 at equity 1.11 and rank 6 at
14.89. That is correct but easy to misread; exposing the simulated evaluation
would need `SimResults` access from the recorder.

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

## Volume

Every position of every game is captured -- there is no sampling. A game runs
about **22.5 turns** (measured), and a pair is two games, so these are the actual
row counts a job produces, not a worst case:

| Job | Positions | Move rows at 10 kept each |
|---|---|---|
| `max_pairs = 40,000` | 1,800,000 | 18,000,000 |
| `max_games = 400,000` | 9,000,000 | 90,000,000 |

That is the price of the feature and it is worth stating plainly: turning
capture on roughly doubles the storage a job produces per unit of Elo
information, and does so in the largest table in the schema.

**`games_per_batch` becomes the memory and payload control.** With no per-task
cap, the size of a submission is a direct function of the batch size: a batch of
20 games is about 450 positions and a few hundred KB of JSON, while a batch of
1,000 games is 22,500 positions and on the order of 15 MB. That is a real
consideration for a job that wants both capture and large batches, and it is the
existing knob rather than a new one.

### Redundancy would multiply the corpus, and the fix is cheap

Games are seeded and deterministic, so with `redundancy > 1` every worker on a
task plays *identical* games and captures *identical* positions. That is pure
duplication -- X copies of the same analysis.

Keying captured positions on `(task_id, game_index, turn_number)` rather than on
the claim, with `ON CONFLICT DO NOTHING`, makes the first accepted claim the one
that lands and the rest no-ops. Redundancy keeps doing its job for the *result*
-- agreement between workers is still checked -- without multiplying the corpus.

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

Two columns on both `job_game_config` and `job_game_pair_config`:

| Column | Default | Meaning |
|---|---|---|
| `capture_positions` | `false` | Off unless asked for |


How many ranked plays come back per position is the player config's
`num_plays_recorded` (MAGPIE's `maxnumdplays`), not a job setting -- it pairs
with `num_plays` (`-np`), which is how many the player simulates. Per-ply
statistics pair the same way: `num_plies_recorded` (`shplies`) against `plies`
(`-pl`).

There is deliberately no sample rate, no turn limit and no per-task cap. Capture
is all-or-nothing per job, which removes the need for the sampling to be
deterministic and removes the failure mode where redundant claims sample
different positions and defeat the deduplication above.

## MAGPIE changes

Good news: **the hook already exists.** `autoplay_results_add_move()` is called
once per turn from `autoplay.c`, immediately after the move is chosen and before
it is played:

```c
const Move *move = game_runner_get_best_move(autoplay_worker, game_runner);
...
autoplay_results_add_move(autoplay_worker->autoplay_results,
                          game_runner->game, move, &rare_rack_or_move_leave);
```

`Recorder` already carries an `add_move_func`, and three recorders already use
it (`leaves_data_add_move`, `fj_data_add_move`, `win_pct_data_add_move`). A
positions recorder is a fourth instance of an established pattern rather than
new machinery.

### 1. `RecorderArgs` needs three more fields

This is the one real gap. The struct today is:

```c
typedef struct RecorderArgs {
  const Game *game;
  const Move *move;        // the move chosen, not the ones considered
  const Rack *leave;
  int number_of_turns;
  uint64_t seed;
  bool divergent;
  bool human_readable;
  const AutoplayGameTiming *timing;
} RecorderArgs;
```

and `autoplay_results_add_move` populates only `game`, `move` and `leave` --
everything else is zeroed. So a recorder can see *which move was played* but not
*what else was considered*, nor which game or turn it belongs to. Add:

```c
  const MoveList *move_list;  // the ranked candidates this turn
  int game_number;            // which game of the batch
  int pair_game_number;       // 0 or 1 within a pair; 0 when not pairing
  int turn_number;            // turn within the game
```

All four are available at the call site: the candidates are
`autoplay_worker->move_lists[player_on_turn_index]`, and `game_runner` already
tracks `game_number`, `pair_game_number` and `turn_number`. Widening
`autoplay_results_add_move`'s signature to take them is the bulk of the change,
and it touches the three existing `add_move` recorders only insofar as they
ignore the new fields.

### 2. A `positions` recorder

`AUTOPLAY_RECORDER_TYPE_POSITION` alongside the existing enum values, registered
through `autoplay_results_set_recorder()` with the same seven function pointers
the others use, and selectable through the options string:

```c
} else if (has_iprefix(option_str, "positions")) {
  options |= autoplay_results_build_option(AUTOPLAY_RECORDER_TYPE_POSITION);
}
```

so `autoplay games,positions` works from the command line too, which makes the
recorder testable without birdtest in the loop.

Per-turn, `positions_data_add_move` records:

- **The position**, via `game_get_cgp(game, false)` from `src/impl/cgp.h`.
- **The rack**, from the player on turn.
- **`game_number`, `pair_game_number`, `turn_number`** for provenance.
- **The top `num_plays_recorded` entries of `move_list`**, formatted with
  `string_builder_add_move()` exactly as the opening-rack executor already does,
  with `equity_is_convertible()` guarding the pass sentinel.

Two details that will otherwise bite:

- **The move list is reused across turns.** `autoplay_worker->move_lists[]` is
  allocated once per worker and refilled every turn, so the recorder must copy
  what it needs rather than retaining the pointer.
- **`MOVE_RECORD_BEST` leaves one entry.** For a static player the list holds
  only the chosen move (see the cost table above), so the recorder will capture
  a one-move "ranking" unless the override is relaxed. That is correct
  behaviour, not a bug, but it means capture on a static-player job produces
  much less than it looks like it should.

### 3. Threading and consolidation

Autoplay runs one `AutoplayWorker` per thread, each with its own recorder
instance, merged at the end through `consolidate_func`. A positions recorder
accumulates a list per thread and concatenates on consolidate, following
`leaves_data_consolidate`.

Because positions are recorded per thread and threads interleave games, the
merged list is **not** in game or turn order. Either sort on consolidate, or
accept unordered output and let the server key on
`(game_number, pair_game_number, turn_number)` -- which it must do anyway. The
latter is simpler and is what the schema above assumes.

### 4. Emitting it

The existing `str_func` produces the `-hr false` summary lines that the
contribution client parses for game results. Positions are far too large for
that shape, and the client reads results in-process anyway, so the recorder
should expose a typed accessor rather than a string:

```c
typedef struct AutoplayPosition {
  char *cgp;
  char *rack;
  int game_number;
  int pair_game_number;
  int turn_number;
  int num_moves;              // how many were ranked, before truncation
  AutoplayPositionMove *moves; // num_plays_recorded of them
  int num_stored_moves;
} AutoplayPosition;

// Borrowed, owned by the recorder; valid until the next autoplay run.
const AutoplayPosition *autoplay_results_get_positions(
    const AutoplayResults *results, int *count);
```

This mirrors `autoplay_results_get_game_summary()`, which was added for exactly
this reason -- so the client reads results out of the structs instead of parsing
formatted output. `config_contribute_games` then serializes the array into the
`positions` field of its submission.

### 5. Turning it on

The contribution client sets the recorder option when the task request asks for
it, so `capture_positions` on the birdtest job becomes `autoplay games,positions`
on the MAGPIE invocation, with `num_plays_recorded` bounding how many entries per
turn are serialized.

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
      "previous_move": "8D DOG", "previous_move_score": 10,
      "num_moves": 412,
      "moves": [ { "move": "8D RETAINS", "score": 74, "equity": 81.2,
                   "win_percentage": 62.1, "blended_utility": 0.64 } ] }
  ]
}
```

`previous_move`/`previous_move_score` are absent on turn 0 of a game, where
nothing preceded it. `blended_utility` -- the win%+spread blend, sometimes
used to rank moves instead of equity or raw win percentage -- has the same
nullability as `win_percentage`: present only for a simming player.

Absent when capture is off, which keeps every existing client valid.

Server-side validation should reject positions outside the task's own games --
a `game_index` beyond the batch, or a `turn_number` beyond any plausible game --
since the submission is otherwise unbounded input written straight into the
largest table in the schema. The natural bound is the batch's own size: at most
`games_per_batch` games, and a generous per-game turn ceiling.

## Phasing

1. **Schema** -- surrogate key, provenance columns, the two partial unique
   indexes. Independent of MAGPIE and worth doing first, since it also
   simplifies `position_analysis_moves`'s foreign key.
2. **Config and validation** -- the two columns, the wire field, the server-side
   bounds. Testable end to end with the fake worker before MAGPIE can produce
   anything.
3. **`RecorderArgs` widening** -- the move list, game and turn fields. Mechanical,
   touches the three existing `add_move` recorders only as a signature change,
   and is worth landing on its own so the recorder itself reviews cleanly.
4. **MAGPIE `positions` recorder** for simming players, where the analysis is
   already computed. Testable from the command line via
   `autoplay games,positions`.
5. **Static-player capture**, behind an option that relaxes the
   `MOVE_RECORD_BEST` override and names its cost.

## Open questions

1. **Should captured positions share the opening rack tables?** An opening rack
   job analyzes turn 1 exhaustively; a games job captures turn 1 positions
   incidentally, under a different player config. Sharing a table means queries
   must always filter by job, or on `position IS NULL`. The alternative is a
   separate `game_position_analyses` table, which duplicates the moves table.
2. **What bounds a submission?** With no per-task cap, `games_per_batch` is the
   only control on payload size, and a job configured with both capture and a
   large batch will produce very large submissions. Worth deciding whether the
   server should reject over some size rather than discovering the limit in
   production.

There is deliberately no consumer yet: this is a corpus being built for later
use. That is a legitimate reason to capture everything rather than sample, but
it does mean the first real query against it may want an index that does not
exist yet.
