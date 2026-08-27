-- Replace per-game rows with the aggregates MAGPIE's autoplay actually reports.
--
-- `game_records` stored one row per game, which assumed autoplay emitted them.
-- It does not: it reports a summary per batch —
--
--   autoplay games <total> <p1_wins> <p1_losses> <p1_ties> <p1_firsts>
--                  <p1_score_mean> <p1_score_sd> <p2_score_mean> <p2_score_sd> ...
--
-- and in `-gp` mode a second such line covering only the *divergent* pairs:
-- those whose two games did not play identically. Pairs that played identically
-- are guaranteed ties carrying no information, so excluding them is the
-- variance reduction that pairing exists to provide.
--
-- Nothing downstream ever read the individual game rows — SPRT and the
-- dashboard both work off counts — and for game pairs the per-game rows could
-- not reconstruct pair outcomes anyway, so the derivation built on them was
-- wrong as well as unnecessary.

-- Dropping the table takes its index (game_records_task_idx) with it.
DROP TABLE IF EXISTS game_records;

CREATE TABLE game_results (
    task_claim_id     UUID PRIMARY KEY REFERENCES task_claims(id) ON DELETE CASCADE,
    task_id           UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,

    -- Every game this task played. For a game_pairs task this is two per pair.
    games             INT NOT NULL CHECK (games >= 0),
    wins              INT NOT NULL CHECK (wins >= 0),      -- player 1
    losses            INT NOT NULL CHECK (losses >= 0),
    ties              INT NOT NULL CHECK (ties >= 0),
    p1_score_mean     DOUBLE PRECISION NOT NULL,
    p1_score_sd       DOUBLE PRECISION NOT NULL,
    p2_score_mean     DOUBLE PRECISION NOT NULL,
    p2_score_sd       DOUBLE PRECISION NOT NULL,
    CONSTRAINT game_results_counts_sum CHECK (wins + losses + ties = games),

    -- The divergent subset. NULL for `games` jobs, which do not play pairs.
    divergent_games   INT CHECK (divergent_games >= 0),
    divergent_wins    INT CHECK (divergent_wins >= 0),
    divergent_losses  INT CHECK (divergent_losses >= 0),
    divergent_ties    INT CHECK (divergent_ties >= 0),
    CONSTRAINT game_results_divergent_all_or_nothing CHECK (
        (divergent_games IS NULL AND divergent_wins IS NULL
             AND divergent_losses IS NULL AND divergent_ties IS NULL)
        OR (divergent_games IS NOT NULL AND divergent_wins IS NOT NULL
             AND divergent_losses IS NOT NULL AND divergent_ties IS NOT NULL
             AND divergent_wins + divergent_losses + divergent_ties = divergent_games
             AND divergent_games <= games)
    ),

    submitted_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX game_results_task_idx ON game_results (task_id);
