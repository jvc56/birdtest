# MAGPIE settings vs. the birdtest schema

This document checks every MAGPIE argument against
[backend/migrations/0001_initial.sql](backend/migrations/0001_initial.sql). It
answers three questions:

- Which arguments does birdtest store?
- Which ones are stored incorrectly?
- In what order should a job page show the settings?

**Source.** MAGPIE's `birdtest-contribute` branch, at the commit that
[docker/Dockerfile:6](docker/Dockerfile#L6) pins. Line numbers prefixed
`config.c:` refer to `src/impl/config.c` at that commit:

- `arg_token_t` is defined at `config.c:100–279`.
- Each token is registered with its CLI name at `config.c:12256–12438`.

**Count.** `arg_token_t` has exactly **175 tokens**, not counting
`NUMBER_OF_ARG_TOKENS`. 45 are registered as commands (`cmd(...)`) and 130 as
options (`arg(...)`). 160 of them predate endgame and pre-endgame play in
games jobs ([ENDGAME_PEG_PLAN.md](ENDGAME_PEG_PLAN.md)), which added 15: the
per-player `eplies1`/`eplies2`, `pegbag1`/`pegbag2`, `pegtopk1`/`pegtopk2`,
`pegstride1`/`pegstride2`, `pegpess1`/`pegpess2` and `pegnested1`/`pegnested2`,
and the nested-lookahead options `pegncaps`, `pegndepth` and `pegnstrides`.

## How a setting reaches MAGPIE

birdtest never sends MAGPIE a command line. A worker runs `magpie contribute`,
claims a task, and gets a JSON request built from the job's row and its player
configs' rows. For each task, `contribute` does three things:

1. It resets every per-player setting (`config.c:8324`) and every run-wide
   setting (`config.c:8856`) to a fixed value. That way nothing carries over
   from a contributor's `settings.txt` or from an earlier task.
2. It applies the values the request states.
3. It refuses any request that leaves out a required value
   (`config.c:8378`, `config.c:8414`, `config.c:8872`), or that states a
   pre-endgame setting a player does not use (`config.c:8400`).

So an argument counts as **covered** only when a column feeds it into every
request. An argument that `contribute` pins to a fixed value is not covered,
and that is deliberate.

Status values used in the table:

| Status | Meaning | Count |
|---|---|---:|
| Stored | A column holds it and every request states it. | 63 |
| Stored (per-player form) | MAGPIE's global option, which sets both players at once. birdtest states the per-player options (`l1`/`l2`, `pl1`/`pl2`, …) instead, so this needs no column of its own. | 17 |
| Run by a job type | A command that a job type runs. Its arguments come from that job type's config table. | 5 |
| Recorded | Not a setting, but birdtest records its value (a seed, a version, a builder). | 4 |
| Not stored | No column. The reason is given in the row. | 86 |

The 86 "Not stored" rows break down as follows:

| Reason | Count |
|---|---:|
| Interactive CLI command | 37 |
| Display, terminal or CLI-session option | 20 |
| A setting of the interactive `endgame` / `peg` commands, which a games player's solves do not read | 13 |
| Worker-local (data path, mmap, threads, transposition-table size) | 4 |
| Pinned by `contribute`, or read only by a feature it turns off (word info table ×3, PlayChooser ×2, overtime ×2, `mtmode`, small plays, heat map) | 10 |
| Other: challenge bonus, write buffer | 2 |

## Every argument

| # | Token | CLI name | Kind | Status | Where in `0001_initial.sql`, or why not |
|---:|---|---|---|---|---|
| 1 | `HELP` | `help` | command | Not stored | Interactive CLI command; no task runs it. |
| 2 | `SET` | `setoptions` | command | Not stored | Interactive CLI command. A task's settings are applied straight from its request JSON, not through `setoptions`. |
| 3 | `CGP` | `cgp` | command | Not stored | Loads a position. Opening-rack tasks always start on an empty board: `opening_rack_requests.previous_play` ([L990](backend/migrations/0001_initial.sql#L990)) is always NULL, and MAGPIE refuses a task where it isn't. |
| 4 | `MOVES` | `addmoves` | command | Not stored | Interactive CLI command; no task runs it. |
| 5 | `RACK` | `rack` | command | Run by a job type | Opening-rack job: each task's racks are unranked from `opening_rack_requests.rack_start` ([L988](backend/migrations/0001_initial.sql#L988)) + `rack_count` ([L989](backend/migrations/0001_initial.sql#L989)), over the space set by `job_opening_rack_config.rack_size` ([L685](backend/migrations/0001_initial.sql#L685)). |
| 6 | `RANDOM_RACK` | `rrack` | command | Not stored | Interactive CLI command; no task runs it. |
| 7 | `GEN` | `generate` | command | Run by a job type | Opening-rack job (`job_type` ([L375](backend/migrations/0001_initial.sql#L375)) `opening_rack`) generates moves for every rack. |
| 8 | `SIM` | `simulate` | command | Run by a job type | Opening-rack job with a simming player (`num_plies > 0`, [L573](backend/migrations/0001_initial.sql#L573)). |
| 9 | `SNOPRUNE` | `snoprune` | command | Not stored | Sim that protects named moves from pruning. Needs a hand-picked move list, which no job type has. |
| 10 | `GEN_AND_SIM` | `gsimulate` | command | Not stored | CLI shortcut. The opening-rack executor does gen + sim itself (see `generate`, `simulate`). |
| 11 | `RACK_AND_GEN` | `rg` | command | Not stored | CLI shortcut. The opening-rack executor does the same steps itself. |
| 12 | `RACK_AND_GEN_AND_SIM` | `rgsimulate` | command | Not stored | CLI shortcut. The opening-rack executor does the same steps itself. |
| 13 | `INFER` | `infer` | command | Not stored | Standalone inference. Inference inside a sim is covered by `use_inference` ([L584](backend/migrations/0001_initial.sql#L584)) and `inference_margin` ([L595](backend/migrations/0001_initial.sql#L595)). |
| 14 | `ENDGAME` | `endgame` | command | Not stored | Interactive command. A games or game-pairs player solves its endgames in autoplay instead, as `endgame_plies` asks (see `eplies1`/`eplies2`). |
| 15 | `PEG` | `peg` | command | Not stored | Interactive command. A games or game-pairs player solves its pre-endgames in autoplay instead, as `peg_max_bag` asks (see `pegbag1`/`pegbag2`). |
| 16 | `AUTOPLAY` | `autoplay` | command | Run by a job type | Games and game-pairs jobs (`job_type` ([L376](backend/migrations/0001_initial.sql#L376))). Its `<num_games>` is `job_game_config.games_per_batch` ([L697](backend/migrations/0001_initial.sql#L697)) / `job_game_pair_config.pairs_per_batch` ([L724](backend/migrations/0001_initial.sql#L724)) (sent as `game_requests.num_games` ([L1005](backend/migrations/0001_initial.sql#L1005))); its recorder list is `games` plus `positions` when `capture_positions` ([L715](backend/migrations/0001_initial.sql#L715)) is on. |
| 17 | `CONVERT` | `convert` | command | Recorded | Not a setting. The server and workers run it to build wordmaps and rack info tables, tracked in `derived_data` ([L307](backend/migrations/0001_initial.sql#L307)). |
| 18 | `CONTRIBUTE` | `contribute` | command | Not stored | The worker's entry point, not a setting. |
| 19 | `P1_NAME` | `p1` | command | Not stored | Cosmetic. birdtest's own label is `player_configs.name` ([L541](backend/migrations/0001_initial.sql#L541)), which MAGPIE never reads. |
| 20 | `P2_NAME` | `p2` | command | Not stored | Cosmetic. birdtest's own label is `player_configs.name` ([L541](backend/migrations/0001_initial.sql#L541)), which MAGPIE never reads. |
| 21 | `LEAVE_GEN` | `leavegen` | command | Run by a job type | Leave-generation job (`job_type` ([L378](backend/migrations/0001_initial.sql#L378))). Per-generation targets are `job_leave_config.target_rack_counts` ([L760](backend/migrations/0001_initial.sql#L760)). `contribute` passes one generation at a time with a target no rack can reach, and ends the task after `num_iterations` ([L752](backend/migrations/0001_initial.sql#L752)) games. `games_before_force_draw` has no column: the server's forced racks (`leave_requests.forced_racks` ([L1022](backend/migrations/0001_initial.sql#L1022))) replace it. |
| 22 | `CREATE_DATA` | `createdata` | command | Not stored | Interactive CLI command; no task runs it. |
| 23 | `DATA_PATH` | `path` | option | Not stored | Worker-local; doesn't change what a task computes. Each file is pinned by content instead (`input_data`). |
| 24 | `BINGO_BONUS` | `bb` | option | Stored | `jobs.bingo_bonus` ([L420](backend/migrations/0001_initial.sql#L420)) |
| 25 | `CHALLENGE_BONUS` | `cb` | option | Not stored | Only the `challenge` command reads it. Autoplay never plays a phony, so no task gets challenged. |
| 26 | `BOARD_LAYOUT` | `bdn` | option | Stored | `jobs.layout_id` ([L413](backend/migrations/0001_initial.sql#L413)), sent as `board_layout` ([L942](backend/migrations/0001_initial.sql#L942), [L956](backend/migrations/0001_initial.sql#L956), [L972](backend/migrations/0001_initial.sql#L972)) |
| 27 | `GAME_VARIANT` | `var` | option | Stored | `jobs.variant` ([L411](backend/migrations/0001_initial.sql#L411)) |
| 28 | `LETTER_DISTRIBUTION` | `ld` | option | Stored | `jobs.letterdist_id` ([L412](backend/migrations/0001_initial.sql#L412)), sent as `letter_distribution` |
| 29 | `LEXICON` | `lex` | option | Stored (per-player form) | Global form of `l1`/`l2`: `player_configs.kwg_id` ([L554](backend/migrations/0001_initial.sql#L554)) on each player config. |
| 30 | `USE_WMP` | `wmp` | option | Stored (per-player form) | Global form of `w1`/`w2`: `player_configs.use_wordmap` ([L589](backend/migrations/0001_initial.sql#L589)) on each player config. |
| 31 | `USE_RIT` | `rit` | option | Stored (per-player form) | Global form of `rit1`/`rit2`: `player_configs.use_rit` ([L590](backend/migrations/0001_initial.sql#L590)) on each player config. |
| 32 | `USE_MMAP_FOR_RIT` | `ritmmap` | option | Not stored | Worker-local; doesn't change what a task computes. (`contribute` mmaps tables unless the contributor turns it off.) |
| 33 | `USE_WIT` | `wit` | option | Not stored | `contribute` forces the word info table off for both players (MAGPIE `config.c:8226`). birdtest doesn't offer or pin a `.wit`, so no task could check one against a hash. |
| 34 | `LEAVES` | `leaves` | option | Stored (per-player form) | Global form of `k1`/`k2`: `player_configs.klv_id` ([L555](backend/migrations/0001_initial.sql#L555)) on each player config. |
| 35 | `P1_LEXICON` | `l1` | option | Stored | `player_configs.kwg_id` ([L554](backend/migrations/0001_initial.sql#L554)) on each player config |
| 36 | `P1_USE_WMP` | `w1` | option | Stored | `player_configs.use_wordmap` ([L589](backend/migrations/0001_initial.sql#L589)) on each player config |
| 37 | `P1_USE_RIT` | `rit1` | option | Stored | `player_configs.use_rit` ([L590](backend/migrations/0001_initial.sql#L590)) on each player config; the table's hash comes from `derived_data` ([L308](backend/migrations/0001_initial.sql#L308)) (role `rit`) |
| 38 | `P1_USE_WIT` | `wit1` | option | Not stored | Forced off by `contribute` (see `wit`). |
| 39 | `P1_LEAVES` | `k1` | option | Stored | `player_configs.klv_id` ([L555](backend/migrations/0001_initial.sql#L555)) on each player config |
| 40 | `P1_MOVE_SORT_TYPE` | `s1` | option | Stored | `player_configs.sort_strategy` ([L543](backend/migrations/0001_initial.sql#L543)) on each player config |
| 41 | `P1_MOVE_RECORD_TYPE` | `r1` | option | Stored | `player_configs.recorder_type` ([L542](backend/migrations/0001_initial.sql#L542)) on each player config |
| 42 | `P2_LEXICON` | `l2` | option | Stored | `player_configs.kwg_id` ([L554](backend/migrations/0001_initial.sql#L554)) on each player config |
| 43 | `P2_USE_WMP` | `w2` | option | Stored | `player_configs.use_wordmap` ([L589](backend/migrations/0001_initial.sql#L589)) on each player config |
| 44 | `P2_USE_RIT` | `rit2` | option | Stored | `player_configs.use_rit` ([L590](backend/migrations/0001_initial.sql#L590)) on each player config; the table's hash comes from `derived_data` ([L308](backend/migrations/0001_initial.sql#L308)) (role `rit`) |
| 45 | `P2_USE_WIT` | `wit2` | option | Not stored | Forced off by `contribute` (see `wit`). |
| 46 | `P2_LEAVES` | `k2` | option | Stored | `player_configs.klv_id` ([L555](backend/migrations/0001_initial.sql#L555)) on each player config |
| 47 | `P2_MOVE_SORT_TYPE` | `s2` | option | Stored | `player_configs.sort_strategy` ([L543](backend/migrations/0001_initial.sql#L543)) on each player config |
| 48 | `P2_MOVE_RECORD_TYPE` | `r2` | option | Stored | `player_configs.recorder_type` ([L542](backend/migrations/0001_initial.sql#L542)) on each player config |
| 49 | `WIN_PCT` | `winpct` | option | Stored | `player_configs.winpct_id` ([L556](backend/migrations/0001_initial.sql#L556)) on each player config. Run-wide in MAGPIE; job creation requires two simming players to agree on it. |
| 50 | `PLIES` | `plies` | option | Stored (per-player form) | Global form of `pl1`/`pl2`: `player_configs.num_plies` ([L573](backend/migrations/0001_initial.sql#L573)) on each player config. The opening-rack executor copies the player's value into it. |
| 51 | `SHPLIES` | `shplies` | option | Stored | `player_configs.num_plies_recorded` ([L574](backend/migrations/0001_initial.sql#L574)) on each player config. **Run-wide in MAGPIE but stored per player and never checked for agreement: see [Fix 1](#fix-1-plays-recorded-and-plies-recorded-are-run-wide-in-a-games-job).** |
| 52 | `SHOW_BU` | `showbu` | option | Not stored | Terminal display only; changes no result. |
| 53 | `ENDGAME_PLIES` | `eplies` | option | Not stored | A setting of the interactive `endgame` / `peg` commands. A games or game-pairs player's solves take theirs from the per-player options (`eplies1`, `pegbag1`, … below), which this one does not set. |
| 54 | `ENDGAME_TOP_K` | `etopk` | option | Not stored | A setting of the interactive `endgame` / `peg` commands. A games or game-pairs player's solves take theirs from the per-player options (`eplies1`, `pegbag1`, … below), which this one does not set. |
| 55 | `ENDGAME_TIME_LIMIT` | `etlim` | option | Not stored | A setting of the interactive `endgame` / `peg` commands. A games or game-pairs player's solves take theirs from the per-player options (`eplies1`, `pegbag1`, … below), which this one does not set. |
| 56 | `PEG_TOP_K` | `pegtopk` | option | Not stored | A setting of the interactive `endgame` / `peg` commands. A games or game-pairs player's solves take theirs from the per-player options (`eplies1`, `pegbag1`, … below), which this one does not set. |
| 57 | `PEG_TIME_LIMIT` | `pegtlim` | option | Not stored | A setting of the interactive `endgame` / `peg` commands. A games or game-pairs player's solves take theirs from the per-player options (`eplies1`, `pegbag1`, … below), which this one does not set. |
| 58 | `PEG_STRIDE` | `pegstride` | option | Not stored | A setting of the interactive `endgame` / `peg` commands. A games or game-pairs player's solves take theirs from the per-player options (`eplies1`, `pegbag1`, … below), which this one does not set. |
| 59 | `PEG_NOPRUNE` | `pnoprune` | option | Not stored | A setting of the interactive `endgame` / `peg` commands. A games or game-pairs player's solves take theirs from the per-player options (`eplies1`, `pegbag1`, … below), which this one does not set. |
| 60 | `PEG_PESSIMISTIC` | `pegpess` | option | Not stored | A setting of the interactive `endgame` / `peg` commands. A games or game-pairs player's solves take theirs from the per-player options (`eplies1`, `pegbag1`, … below), which this one does not set. |
| 61 | `PEG_NESTED` | `pegnested` | option | Not stored | A setting of the interactive `endgame` / `peg` commands. A games or game-pairs player's solves take theirs from the per-player options (`eplies1`, `pegbag1`, … below), which this one does not set. |
| 62 | `PEG_OUTCOMES` | `pegoutcomes` | option | Not stored | A setting of the interactive `endgame` / `peg` commands. A games or game-pairs player's solves take theirs from the per-player options (`eplies1`, `pegbag1`, … below), which this one does not set. Also display only. |
| 63 | `PEG_OUT_WIDTH` | `pegoutwidth` | option | Not stored | A setting of the interactive `endgame` / `peg` commands. A games or game-pairs player's solves take theirs from the per-player options (`eplies1`, `pegbag1`, … below), which this one does not set. Also display only. |
| 64 | `PEG_OUT_LINES` | `pegoutlines` | option | Not stored | A setting of the interactive `endgame` / `peg` commands. A games or game-pairs player's solves take theirs from the per-player options (`eplies1`, `pegbag1`, … below), which this one does not set. Also display only. |
| 65 | `NUMBER_OF_PLAYS` | `numplays` | option | Stored (per-player form) | Global form of `np1`/`np2`: `player_configs.num_plays` ([L577](backend/migrations/0001_initial.sql#L577)) on each player config. |
| 66 | `MAX_NUMBER_OF_DISPLAY_PLAYS` | `maxnumdplays` | option | Stored | `player_configs.num_plays_recorded` ([L580](backend/migrations/0001_initial.sql#L580)) on each player config. **Run-wide in MAGPIE but stored per player and never checked for agreement: see [Fix 1](#fix-1-plays-recorded-and-plies-recorded-are-run-wide-in-a-games-job).** |
| 67 | `NUMBER_OF_SMALL_PLAYS` | `numsmallplays` | option | Not stored | A setting of the interactive `endgame` / `peg` commands. A games or game-pairs player's solves take theirs from the per-player options (`eplies1`, `pegbag1`, … below), which this one does not set. |
| 68 | `MAX_ITERATIONS` | `iterations` | option | Stored (per-player form) | Global form of `i1`/`i2`: `player_configs.max_iterations` ([L582](backend/migrations/0001_initial.sql#L582)) on each player config. |
| 69 | `STOP_COND_PCT` | `scondition` | option | Stored (per-player form) | Global form of `sc1`/`sc2`: `player_configs.stopping_pct` ([L583](backend/migrations/0001_initial.sql#L583)) on each player config. MAGPIE's `none` is the same as `threshold = 'none'`, which birdtest does allow. |
| 70 | `INFERENCE_MARGIN` | `imargin` | option | Stored (per-player form) | Global form of `im1`/`im2`: `player_configs.inference_margin` ([L595](backend/migrations/0001_initial.sql#L595)) on each player config. |
| 71 | `MOVEGEN_MARGIN` | `mmargin` | option | Stored | `player_configs.movegen_margin` ([L603](backend/migrations/0001_initial.sql#L603)) on each player config. Run-wide in MAGPIE; job creation requires both players to agree on it. |
| 72 | `MIN_PLAY_ITERATIONS` | `minplayiterations` | option | Stored (per-player form) | Global form of `mi1`/`mi2`: `player_configs.min_play_iterations` ([L592](backend/migrations/0001_initial.sql#L592)) on each player config. |
| 73 | `USE_GAME_PAIRS` | `gp` | option | Stored | `job_type` ([L377](backend/migrations/0001_initial.sql#L377)) `game_pairs` (MAGPIE `config.c:9086`) |
| 74 | `USE_SMALL_PLAYS` | `sp` | option | Not stored | Endgame move-list format. `contribute` resets it to false (MAGPIE `config.c:8861`). |
| 75 | `SIM_WITH_INFERENCE` | `sinfer` | option | Stored (per-player form) | Global form of `si1`/`si2`: `player_configs.use_inference` ([L584](backend/migrations/0001_initial.sql#L584)) on each player config. **Opening-rack jobs force it off: see [Fix 2](#fix-2-opening-rack-jobs-ignore-inference).** |
| 76 | `USE_HEAT_MAP` | `useheatmap` | option | Not stored | Records heat-map data only. `contribute` resets it to false. |
| 77 | `WRITE_BUFFER_SIZE` | `wb` | option | Not stored | I/O buffer for autoplay's file recorders. `contribute` returns JSON, so this changes no result. |
| 78 | `HUMAN_READABLE` | `hr` | option | Not stored | Terminal display only; changes no result. `contribute` sets it to false. |
| 79 | `SHOW_MISTAKES` | `mistakes` | option | Not stored | Only `analyze` reads it; display only. |
| 80 | `RANDOM_SEED` | `seed` | option | Recorded | Per task, not a setting: `tasks.seed` ([L845](backend/migrations/0001_initial.sql#L845)), copied to `game_requests.seed` ([L1004](backend/migrations/0001_initial.sql#L1004)) and `leave_requests.seed` ([L1021](backend/migrations/0001_initial.sql#L1021)). An opening-rack task analyses rack *i* with seed + *i*. |
| 81 | `NUMBER_OF_THREADS` | `threads` | option | Not stored | Worker-local (the contributor's settings file). Results don't depend on it: `contribute` gives each game its own thread (per-game parallelism). |
| 82 | `PRINT_INTERVAL` | `pfrequency` | option | Not stored | Terminal display only; changes no result. `contribute` resets it to 0. |
| 83 | `EXEC_MODE` | `mode` | option | Not stored | CLI sync/async mode. |
| 84 | `TT_FRACTION_OF_MEM` | `ttfraction` | option | Not stored | Worker-local; doesn't change what a task computes. It also sizes the endgame transposition table a run's endgame and pre-endgame solves share: it changes how fast a solve runs, not what birdtest asks for. |
| 85 | `TIME_LIMIT` | `tlim` | option | Stored (per-player form) | Global form of `tl1`/`tl2`: `player_configs.time_limit_secs` ([L585](backend/migrations/0001_initial.sql#L585)) on each player config. |
| 86 | `SAMPLING_RULE` | `sr` | option | Stored (per-player form) | Global form of `sa1`/`sa2`: `player_configs.sampling_rule` ([L594](backend/migrations/0001_initial.sql#L594)) on each player config. |
| 87 | `THRESHOLD` | `threshold` | option | Stored (per-player form) | Global form of `th1`/`th2`: `player_configs.threshold` ([L593](backend/migrations/0001_initial.sql#L593)) on each player config. |
| 88 | `CUTOFF` | `cutoff` | option | Stored | `jobs.sim_cutoff` ([L421](backend/migrations/0001_initial.sql#L421)) |
| 89 | `UTILITY_W_WINPCT` | `uwin` | option | Stored (per-player form) | Global form of `uwin1`/`uwin2`: `player_configs.utility_w_winpct` ([L596](backend/migrations/0001_initial.sql#L596)) on each player config. |
| 90 | `UTILITY_W_SPREAD` | `uspread` | option | Stored (per-player form) | Global form of `uspread1`/`uspread2`: `player_configs.utility_w_spread` ([L597](backend/migrations/0001_initial.sql#L597)) on each player config. |
| 91 | `UTILITY_SPREAD_SCALE` | `uspreadscale` | option | Stored (per-player form) | Global form of `uspreadscale1`/`uspreadscale2`: `player_configs.utility_spread_scale` ([L598](backend/migrations/0001_initial.sql#L598)) on each player config. |
| 92 | `LOAD` | `load` | command | Not stored | Interactive CLI command; no task runs it. |
| 93 | `NEW_GAME` | `newgame` | command | Not stored | Interactive CLI command; no task runs it. |
| 94 | `EXPORT` | `export` | command | Not stored | Interactive CLI command; no task runs it. |
| 95 | `COMMIT` | `commit` | command | Not stored | Interactive CLI command; no task runs it. |
| 96 | `TOP_COMMIT` | `tcommit` | command | Not stored | Interactive CLI command; no task runs it. |
| 97 | `CHALLENGE` | `challenge` | command | Not stored | Interactive CLI command; no task runs it. |
| 98 | `UNCHALLENGE` | `unchallenge` | command | Not stored | Interactive CLI command; no task runs it. |
| 99 | `OVERTIME` | `overtimepenalty` | command | Not stored | Interactive CLI command; no task runs it. (Applies a penalty to a loaded game's history.) |
| 100 | `SWITCH_NAMES` | `switchnames` | command | Not stored | Interactive CLI command; no task runs it. |
| 101 | `SHOW_GAME` | `shgame` | command | Not stored | Interactive CLI command; no task runs it. |
| 102 | `SHOW_MOVES` | `shmoves` | command | Not stored | Interactive CLI command; no task runs it. |
| 103 | `SHOW_INFERENCE` | `shinference` | command | Not stored | Interactive CLI command; no task runs it. |
| 104 | `SHOW_ENDGAME` | `shendgame` | command | Not stored | Interactive CLI command; no task runs it. |
| 105 | `SHOW_PEG` | `shpeg` | command | Not stored | Interactive CLI command; no task runs it. |
| 106 | `SHOW_HEAT_MAP` | `heatmap` | command | Not stored | Interactive CLI command; no task runs it. |
| 107 | `NEXT` | `next` | command | Not stored | Interactive CLI command; no task runs it. |
| 108 | `PREVIOUS` | `previous` | command | Not stored | Interactive CLI command; no task runs it. |
| 109 | `GOTO` | `goto` | command | Not stored | Interactive CLI command; no task runs it. |
| 110 | `NOTE` | `note` | command | Not stored | Interactive CLI command; no task runs it. |
| 111 | `CNOTE` | `cnote` | command | Not stored | Interactive CLI command; no task runs it. |
| 112 | `PRINT_BOARDS` | `printboards` | option | Not stored | Terminal display only; changes no result. |
| 113 | `BOARD_COLOR` | `boardcolor` | option | Not stored | Terminal display only; changes no result. |
| 114 | `BOARD_TILE_GLYPHS` | `boardtiles` | option | Not stored | Terminal display only; changes no result. |
| 115 | `BOARD_BORDER` | `boardborder` | option | Not stored | Terminal display only; changes no result. |
| 116 | `BOARD_COLUMN_LABEL` | `boardcolumns` | option | Not stored | Terminal display only; changes no result. |
| 117 | `ON_TURN_MARKER` | `onturnmarker` | option | Not stored | Terminal display only; changes no result. |
| 118 | `ON_TURN_COLOR` | `onturncolor` | option | Not stored | Terminal display only; changes no result. |
| 119 | `ON_TURN_SCORE_STYLE` | `onturnscore` | option | Not stored | Terminal display only; changes no result. |
| 120 | `PRETTY` | `pretty` | option | Not stored | Terminal display only; changes no result. |
| 121 | `PRINT_ON_FINISH` | `printonfinish` | option | Not stored | Terminal display only; changes no result. `contribute` sets it to false. |
| 122 | `SHOW_PROMPT` | `shprompt` | option | Not stored | Terminal display only; changes no result. |
| 123 | `SAVE_SETTINGS` | `savesettings` | option | Not stored | CLI session behaviour; changes no result. |
| 124 | `AUTOSAVE_GCG` | `autosavegcg` | option | Not stored | CLI session behaviour; changes no result. |
| 125 | `FG_REQUIRED` | `fgrequired` | option | Not stored | Whether `newgame` needs a filename; CLI only. |
| 126 | `SHOW_GAME_WITH_MOVES` | `shwithmoves` | option | Not stored | Terminal display only; changes no result. |
| 127 | `P1_SIM_PLIES` | `pl1` | option | Stored | `player_configs.num_plies` ([L573](backend/migrations/0001_initial.sql#L573)) on each player config |
| 128 | `P2_SIM_PLIES` | `pl2` | option | Stored | `player_configs.num_plies` ([L573](backend/migrations/0001_initial.sql#L573)) on each player config |
| 129 | `P1_NUM_PLAYS` | `np1` | option | Stored | `player_configs.num_plays` ([L577](backend/migrations/0001_initial.sql#L577)) on each player config |
| 130 | `P2_NUM_PLAYS` | `np2` | option | Stored | `player_configs.num_plays` ([L577](backend/migrations/0001_initial.sql#L577)) on each player config |
| 131 | `P1_STOP_COND_PCT` | `sc1` | option | Stored | `player_configs.stopping_pct` ([L583](backend/migrations/0001_initial.sql#L583)) on each player config |
| 132 | `P2_STOP_COND_PCT` | `sc2` | option | Stored | `player_configs.stopping_pct` ([L583](backend/migrations/0001_initial.sql#L583)) on each player config |
| 133 | `P1_MAX_ITERATIONS` | `i1` | option | Stored | `player_configs.max_iterations` ([L582](backend/migrations/0001_initial.sql#L582)) on each player config |
| 134 | `P2_MAX_ITERATIONS` | `i2` | option | Stored | `player_configs.max_iterations` ([L582](backend/migrations/0001_initial.sql#L582)) on each player config |
| 135 | `P1_MIN_PLAY_ITERATIONS` | `mi1` | option | Stored | `player_configs.min_play_iterations` ([L592](backend/migrations/0001_initial.sql#L592)) on each player config |
| 136 | `P2_MIN_PLAY_ITERATIONS` | `mi2` | option | Stored | `player_configs.min_play_iterations` ([L592](backend/migrations/0001_initial.sql#L592)) on each player config |
| 137 | `P1_SIM_WITH_INFERENCE` | `si1` | option | Stored | `player_configs.use_inference` ([L584](backend/migrations/0001_initial.sql#L584)) on each player config |
| 138 | `P2_SIM_WITH_INFERENCE` | `si2` | option | Stored | `player_configs.use_inference` ([L584](backend/migrations/0001_initial.sql#L584)) on each player config |
| 139 | `P1_TIME_LIMIT` | `tl1` | option | Stored | `player_configs.time_limit_secs` ([L585](backend/migrations/0001_initial.sql#L585)) on each player config (must be 0 for a simmer; see [Notes](#lower-priority-notes)) |
| 140 | `P2_TIME_LIMIT` | `tl2` | option | Stored | `player_configs.time_limit_secs` ([L585](backend/migrations/0001_initial.sql#L585)) on each player config (must be 0 for a simmer; see [Notes](#lower-priority-notes)) |
| 141 | `P1_PLAY_CHOOSER_TIME` | `pc1` | option | Not stored | Turns on PlayChooser (timed play). `contribute` resets it to -1 (off) before every task (MAGPIE `config.c:8352`): timed games are not reproducible from the seed, so redundancy and game pairs would mean nothing. Deliberately omitted. |
| 142 | `P2_PLAY_CHOOSER_TIME` | `pc2` | option | Not stored | Turns on PlayChooser (timed play). `contribute` resets it to -1 (off) before every task (MAGPIE `config.c:8352`): timed games are not reproducible from the seed, so redundancy and game pairs would mean nothing. Deliberately omitted. |
| 143 | `OVERTIME_PENALTY_POINTS` | `otpenalty` | option | Not stored | Only read by a PlayChooser clock, which `contribute` always turns off (see `pc1`/`pc2`). |
| 144 | `OVERTIME_PERIOD` | `otperiod` | option | Not stored | Only read by a PlayChooser clock, which `contribute` always turns off (see `pc1`/`pc2`). |
| 145 | `P1_THRESHOLD` | `th1` | option | Stored | `player_configs.threshold` ([L593](backend/migrations/0001_initial.sql#L593)) on each player config |
| 146 | `P2_THRESHOLD` | `th2` | option | Stored | `player_configs.threshold` ([L593](backend/migrations/0001_initial.sql#L593)) on each player config |
| 147 | `P1_SAMPLING_RULE` | `sa1` | option | Stored | `player_configs.sampling_rule` ([L594](backend/migrations/0001_initial.sql#L594)) on each player config |
| 148 | `P2_SAMPLING_RULE` | `sa2` | option | Stored | `player_configs.sampling_rule` ([L594](backend/migrations/0001_initial.sql#L594)) on each player config |
| 149 | `P1_UTILITY_W_WINPCT` | `uwin1` | option | Stored | `player_configs.utility_w_winpct` ([L596](backend/migrations/0001_initial.sql#L596)) on each player config |
| 150 | `P2_UTILITY_W_WINPCT` | `uwin2` | option | Stored | `player_configs.utility_w_winpct` ([L596](backend/migrations/0001_initial.sql#L596)) on each player config |
| 151 | `P1_UTILITY_W_SPREAD` | `uspread1` | option | Stored | `player_configs.utility_w_spread` ([L597](backend/migrations/0001_initial.sql#L597)) on each player config |
| 152 | `P2_UTILITY_W_SPREAD` | `uspread2` | option | Stored | `player_configs.utility_w_spread` ([L597](backend/migrations/0001_initial.sql#L597)) on each player config |
| 153 | `P1_UTILITY_SPREAD_SCALE` | `uspreadscale1` | option | Stored | `player_configs.utility_spread_scale` ([L598](backend/migrations/0001_initial.sql#L598)) on each player config |
| 154 | `P2_UTILITY_SPREAD_SCALE` | `uspreadscale2` | option | Stored | `player_configs.utility_spread_scale` ([L598](backend/migrations/0001_initial.sql#L598)) on each player config |
| 155 | `P1_INFERENCE_MARGIN` | `im1` | option | Stored | `player_configs.inference_margin` ([L595](backend/migrations/0001_initial.sql#L595)) on each player config |
| 156 | `P2_INFERENCE_MARGIN` | `im2` | option | Stored | `player_configs.inference_margin` ([L595](backend/migrations/0001_initial.sql#L595)) on each player config |
| 157 | `P1_ENDGAME_PLIES` | `eplies1` | option | Stored | `player_configs.endgame_plies` ([L615](backend/migrations/0001_initial.sql#L615)) on each player config (0 solves nothing, the pre-endgame included) |
| 158 | `P2_ENDGAME_PLIES` | `eplies2` | option | Stored | `player_configs.endgame_plies` ([L615](backend/migrations/0001_initial.sql#L615)) on each player config (0 solves nothing, the pre-endgame included) |
| 159 | `P1_PEG_MAX_BAG` | `pegbag1` | option | Stored | `player_configs.peg_max_bag` ([L616](backend/migrations/0001_initial.sql#L616)) on each player config |
| 160 | `P2_PEG_MAX_BAG` | `pegbag2` | option | Stored | `player_configs.peg_max_bag` ([L616](backend/migrations/0001_initial.sql#L616)) on each player config |
| 161 | `P1_PEG_TOP_K` | `pegtopk1` | option | Stored | `player_configs.peg_stage_top_k` ([L618](backend/migrations/0001_initial.sql#L618)) on each player config |
| 162 | `P2_PEG_TOP_K` | `pegtopk2` | option | Stored | `player_configs.peg_stage_top_k` ([L618](backend/migrations/0001_initial.sql#L618)) on each player config |
| 163 | `P1_PEG_STRIDE` | `pegstride1` | option | Stored | `player_configs.peg_scenario_stride` ([L619](backend/migrations/0001_initial.sql#L619)) on each player config |
| 164 | `P2_PEG_STRIDE` | `pegstride2` | option | Stored | `player_configs.peg_scenario_stride` ([L619](backend/migrations/0001_initial.sql#L619)) on each player config |
| 165 | `P1_PEG_PESSIMISTIC` | `pegpess1` | option | Stored | `player_configs.peg_opp_model` ([L620](backend/migrations/0001_initial.sql#L620)) on each player config (`'pessimistic'` for true) |
| 166 | `P2_PEG_PESSIMISTIC` | `pegpess2` | option | Stored | `player_configs.peg_opp_model` ([L620](backend/migrations/0001_initial.sql#L620)) on each player config (`'pessimistic'` for true) |
| 167 | `P1_PEG_NESTED` | `pegnested1` | option | Stored | `player_configs.peg_nested` ([L621](backend/migrations/0001_initial.sql#L621)) on each player config |
| 168 | `P2_PEG_NESTED` | `pegnested2` | option | Stored | `player_configs.peg_nested` ([L621](backend/migrations/0001_initial.sql#L621)) on each player config |
| 169 | `PEG_NESTED_CAND_CAPS` | `pegncaps` | option | Stored | `player_configs.peg_nested_cand_caps` ([L623](backend/migrations/0001_initial.sql#L623)) on each player config. One option for both players on the CLI; birdtest states it per player. |
| 170 | `PEG_NESTED_DEPTH` | `pegndepth` | option | Stored | `player_configs.peg_nested_max_depth` ([L624](backend/migrations/0001_initial.sql#L624)) on each player config. One option for both players on the CLI; birdtest states it per player. |
| 171 | `PEG_NESTED_STRIDES` | `pegnstrides` | option | Stored | `player_configs.peg_nested_strides` ([L625](backend/migrations/0001_initial.sql#L625)) on each player config. One option for both players on the CLI; birdtest states it per player. |
| 172 | `MULTI_THREADING_MODE` | `mtmode` | option | Not stored | Changes results: `igp` gives a simmer every thread and so changes what it samples. `contribute` resets it to `pgp` before every task (MAGPIE `config.c:8860`). Deliberately omitted. |
| 173 | `ANALYZE` | `analyze` | command | Not stored | Interactive CLI command; no task runs it. |
| 174 | `VERSION` | `version` | command | Recorded | Not a setting. Each claim records the worker's version in `task_claims.magpie_version` ([L915](backend/migrations/0001_initial.sql#L915)); each job sets a floor in `jobs.min_magpie_*` ([L436](backend/migrations/0001_initial.sql#L436)). |
| 175 | `BUILDERS` | `builders` | command | Recorded | Not a setting. The server asks its own MAGPIE and records the answer in `derived_data.builder` ([L314](backend/migrations/0001_initial.sql#L314)). |

## Settings birdtest accounts for incorrectly

All of these are covered by a column. What's wrong is how the column is
checked or shown:

- **Fix 1** can make a job record different data from what its page says.
- **Fixes 2 and 3** affect display only.
- **Fix 4** is an optional addition, not a bug.

### Fix 1: Plays recorded and plies recorded are run-wide in a games job

**The problem.** `maxnumdplays` (`num_plays_recorded`) and `shplies`
(`num_plies_recorded`) are stored per player
([L574](backend/migrations/0001_initial.sql#L574),
[L580](backend/migrations/0001_initial.sql#L580)). In MAGPIE, however, each is
a single run-wide value:

- The games executor sets both from **player 1 only** (`config.c:9147–9156`).
- Both seats then read those values: `config.c:4074` for the capture cap, and
  `config.c:4164` and `config.c:4178` for plies.

The server handles them differently again
([jobs/mod.rs:733–745](backend/src/jobs/mod.rs#L733-L745)):

- It keeps player 1's `num_plays_recorded`.
- It keeps the larger of the two players' `num_plies_recorded`. That larger
  value never takes effect, because the worker never reports more plies than
  player 1's value.

Job creation compares the players' `movegen_margin` and win% model, but not
these two.

**The effect.** Take a games or game-pairs job with `capture_positions` on,
where player 2 states a different `num_plays_recorded` or a larger
`num_plies_recorded` than player 1:

- Player 2's positions are captured using player 1's numbers.
- The job page still shows player 2's own values as if they were applied.

With capture off, a games job never reads either value.

**The fix.** This needs no migration.

1. **Refuse the mismatch at job creation.** In
   [`validate_capture_play_cap`](backend/src/routes/admin.rs#L1796), which only
   runs when capture is on, refuse a job whose two configs differ on
   `num_plays_recorded` or `num_plies_recorded`. Use the field
   `capture_positions` and model the check on the `movegen_margin` check in
   [`validate_shared_player_options`](backend/src/routes/admin.rs#L1745).
   Add a test next to the existing capture tests.
2. **Simplify the server's ply cap.** In
   [jobs/mod.rs:740–745](backend/src/jobs/mod.rs#L740-L745), replace
   `player1.num_plies_recorded.max(player2.num_plies_recorded)` with player 1's
   value. Once step 1 is in place, the two values are always equal.
3. **Mark the values as unused when capture is off.**
   [`unusedPlayerSettings`](frontend/src/lib/jobSettings.ts#L339) should take
   the whole `JobConfig` instead of just the job type. For a games or pairs job
   with `capture_positions` off, it should mark `Plays recorded` and
   `Plies recorded` as unused. Update the caller in
   [JobSettings.svelte:50](frontend/src/lib/components/JobSettings.svelte#L50)
   and add cases to `jobSettings.test.ts`.
4. **Update the schema comment.** The comment above `movegen_margin`
   ([L599–602](backend/migrations/0001_initial.sql#L599-L602)) explains which
   per-player columns are really run-wide. Add these two, noting that for them
   it applies only to games and game-pairs jobs.
5. **Find existing jobs that are affected:**

   ```sql
   SELECT j.id, j.name, 'games' AS kind
   FROM jobs j
   JOIN job_game_config c ON c.job_id = j.id
   JOIN player_configs p1 ON p1.id = c.player1_config_id
   JOIN player_configs p2 ON p2.id = c.player2_config_id
   WHERE c.capture_positions
     AND (p1.num_plays_recorded <> p2.num_plays_recorded
          OR p1.num_plies_recorded <> p2.num_plies_recorded)
   UNION ALL
   SELECT j.id, j.name, 'game_pairs'
   FROM jobs j
   JOIN job_game_pair_config c ON c.job_id = j.id
   JOIN player_configs p1 ON p1.id = c.player1_config_id
   JOIN player_configs p2 ON p2.id = c.player2_config_id
   WHERE c.capture_positions
     AND (p1.num_plays_recorded <> p2.num_plays_recorded
          OR p1.num_plies_recorded <> p2.num_plies_recorded);
   ```

**Alternative.** MAGPIE could cap each seat by its own player's values. That
fixes the cause rather than refusing the setup, but it needs a MAGPIE release
and a higher version floor.

### Fix 2: Opening-rack jobs ignore inference

**The problem.** The opening-rack executor forces `sim_with_inference` off
(`config.c:8940–8942`), because an opening rack has no previous play to infer
from. A simming config still stores `use_inference` and `inference_margin`;
the CHECK at [L633](backend/migrations/0001_initial.sql#L633) requires them.
The job page then shows "Inference: yes", and the search summary
([`playerSummary`](frontend/src/lib/jobSettings.ts#L123)) says "inference".

**The fix.** This is UI only. The columns have to stay, because the same
config can also be used in a games job.

- In [`unusedPlayerSettings`](frontend/src/lib/jobSettings.ts#L339), add an
  opening-rack set that contains `Inference` and `Inference margin`.
- Have `playerSummary` drop "inference" when it describes an opening-rack
  job's player.

### Fix 3: Leave jobs store a sim cutoff they never send

**The problem.** [`jobs.sim_cutoff`](backend/migrations/0001_initial.sql#L421)
is `NOT NULL` for every job. However, the leave-generation request has no
`sim_cutoff`, and MAGPIE's leave path doesn't read one (`config.c:9526`,
`states_cutoff=false`). The job page still lists "Sim cutoff" under
"All settings" for a leave job.

**The fix.** Hide the row for leave jobs; the list below already does this.
You could also make the column nullable with
`CHECK ((job_type = 'leave_generation') = (sim_cutoff IS NULL))`, but that
adds a migration for little gain.

### Fix 4 (optional addition): bingo bonus and sim cutoff can't be set

**The situation.** Both columns exist, and every request states them. However,
[`CreateJobBody`](backend/src/routes/admin.rs#L1164) has no field for either,
and [`create_job`](backend/src/routes/admin.rs#L1399-L1400) always writes
MAGPIE's defaults (50 and 0.005). The schema covers them, but no admin can
change them.

**To add them:**

1. Add two optional fields to `CreateJobBody`.
2. Validate them the way MAGPIE does:
   - `sim_cutoff` must be finite and within 0–100.
   - `bingo_bonus` must fit in an `i32`; also refusing negative values is
     probably sensible.
3. Bind `unwrap_or(magpie_defaults::…)`.
4. Add both inputs to the job form under its advanced settings.
5. Leave out `sim_cutoff` for leave jobs (see Fix 3).

If bingo bonus becomes settable, promote it to a key row in the job list
below, next to Board.

### Lower-priority notes

These are limits rather than bugs. None of them needs a change today.

- **Integer width of `max_iterations` and `min_play_iterations`.** Both are
  `INT` ([L582](backend/migrations/0001_initial.sql#L582),
  [L592](backend/migrations/0001_initial.sql#L592)), but MAGPIE stores them
  as `uint64`. That caps them at 2,147,483,647. MAGPIE's own default of
  10¹² doesn't fit, but birdtest requires an explicit budget anyway. If a
  larger budget is ever needed, widen both to `BIGINT`.
- **`min_play_iterations` of 0.** The API requires at least 1; MAGPIE
  accepts 0.
- **`time_limit_secs`.** The column
  ([L585](backend/migrations/0001_initial.sql#L585)) is `INT`, while MAGPIE
  takes fractional seconds. The API requires 0 for a simmer, because a time
  limit makes results depend on hardware. A static player stores NULL, so the
  column is always 0 or NULL. Keep it anyway: MAGPIE requires the key for
  every simmer.
- **Margin precision.** MAGPIE stores `movegen_margin` and `inference_margin`
  in thousandths of a point (`double_to_equity`). A value finer than 0.001 is
  rounded on the worker, so two configs that differ only past the third
  decimal play identically. If you want them to look different only when
  they play differently, round to three decimals when creating the config.
- **Text columns without CHECK constraints.** `recorder_type`,
  `sort_strategy`, `threshold`, `sampling_rule` and `jobs.variant` are only
  validated in `admin.rs`. A CHECK constraint on each would stop a restored or
  hand-written row from holding a value that every worker refuses.

## Job settings, in display order

This is one ordered list for every job type. A job shows only the rows that
apply to it. **Rows 1–11 are the key rows** and are shown by default. Rows
12–20 appear after clicking **All settings**.

| # | Setting | Shown for | Column(s) | MAGPIE argument |
|---:|---|---|---|---|
| 1 | Type | all | `jobs.job_type` | `autoplay` / `gp` / `leavegen` / rack analysis |
| 2 | Variant | all | `jobs.variant` | `var` |
| 3 | Letter distribution | all | `jobs.letterdist_id` | `ld` |
| 4 | Board | all | `jobs.layout_id` | `bdn` |
| 5 | Games to play / Pairs to play (with SPRT: Cap) | games, pairs | `max_games` / `max_pairs` | `autoplay` total |
| 6 | SPRT ("none", or "Elo H0 → H1") | games, pairs | `sprt_enabled`, `elo_low`, `elo_high` | n/a |
| 7 | Records positions | games, pairs | `capture_positions` | `autoplay` recorder `positions` |
| 8 | Racks in all | opening racks | `total_racks` | `rack` (the space) |
| 9 | Rack size | opening racks | `rack_size` | `rack` |
| 10 | Generations | leave | `cardinality(target_rack_counts)` | `leavegen` |
| 11 | Target per rack | leave | `target_rack_counts` | `leavegen` targets |
| 12 | Bingo bonus | all | `jobs.bingo_bonus` | `bb` |
| 13 | Sim cutoff | games, pairs, opening racks | `jobs.sim_cutoff` | `cutoff` |
| 14 | Fewest games / pairs before the test acts | games, pairs with SPRT on | `min_games` / `min_pairs` | n/a |
| 15 | SPRT α | games, pairs with SPRT on | `sprt_alpha` | n/a |
| 16 | SPRT β | games, pairs with SPRT on | `sprt_beta` | n/a |
| 17 | Games per task / Pairs per task | games, pairs, leave | `games_per_batch` / `pairs_per_batch` / `num_iterations` | `autoplay <num_games>`; leave: game cap per task |
| 18 | Racks per task | opening racks, leave | `racks_per_batch` / `racks_per_task` | `rack` batch; `leave_requests.forced_racks` |
| 19 | Redundancy | all | `jobs.redundancy` | n/a |
| 20 | Oldest MAGPIE | all | `jobs.min_magpie_*` | `version` (compared at claim) |

With these key rows, a games job shows 7 rows by default, and an opening-rack
or leave job shows 6.

Compared with the current
[`jobGroups`](frontend/src/lib/jobSettings.ts#L167), this order makes four
changes:

- **Records positions becomes a key row.** It decides what data the job
  produces, and whether "Plays recorded" means anything (Fix 1).
- **Elo H0 and Elo H1 become one SPRT row.** One row says whether the test
  runs and between which bounds.
- **A leave job's "Games per task" moves to row 17.** It shares that row with
  the games batch size, which is the same kind of setting.
- **A leave job no longer shows Sim cutoff** (Fix 3).

Allocation and status are not on this list. They are the job's state rather
than its settings, and the page header already shows them.

## Player settings, in display order

This is one ordered list. The table has one value column per player: one for
an opening-rack or leave job, two side by side for a games or pairs job.
**Rows 1–13 are the key rows.** Rows 14–32 appear after clicking
**All settings**.

| # | Setting | Column | MAGPIE argument | Never read by |
|---:|---|---|---|---|
| 1 | Lexicon | `kwg_id` | `l1`/`l2` (`lex`) | n/a |
| 2 | Leaves | `klv_id` | `k1`/`k2` (`leaves`) | leave jobs |
| 3 | Plies (0 = static) | `num_plies` | `pl1`/`pl2` (`plies`) | n/a |
| 4 | Plays considered | `num_plays` | `np1`/`np2` (`numplays`) | n/a |
| 5 | Sort | `sort_strategy` | `s1`/`s2` | n/a |
| 6 | Win % model | `winpct_id` | `winpct` | static players |
| 7 | Iterations (most) | `max_iterations` | `i1`/`i2` (`iterations`) | static players |
| 8 | Stopping % | `stopping_pct` | `sc1`/`sc2` (`scondition`) | static players |
| 9 | Inference | `use_inference` | `si1`/`si2` (`sinfer`) | static players, opening-rack jobs (Fix 2) |
| 10 | Recorder | `recorder_type` | `r1`/`r2` | leave jobs |
| 11 | Plays recorded | `num_plays_recorded` | `maxnumdplays` | leave jobs; games/pairs jobs without capture (Fix 1) |
| 12 | Endgame ("6-ply endgame" or "off") | `endgame_plies` | `eplies1`/`eplies2` | opening-rack jobs; leave jobs (they refuse a player that solves) |
| 13 | Pre-endgame ("bag ≤ 2" or "off"; off without the endgame) | `peg_max_bag` | `pegbag1`/`pegbag2` | opening-rack jobs; leave jobs |
| 14 | Plies recorded | `num_plies_recorded` | `shplies` | leave jobs; games/pairs jobs without capture (Fix 1) |
| 15 | Iterations per play (fewest) | `min_play_iterations` | `mi1`/`mi2` (`minplayiterations`) | static players |
| 16 | Threshold | `threshold` | `th1`/`th2` (`threshold`) | static players |
| 17 | Sampling rule | `sampling_rule` | `sa1`/`sa2` (`sr`) | static players |
| 18 | Inference margin | `inference_margin` | `im1`/`im2` (`imargin`) | static players, opening-rack jobs (Fix 2) |
| 19 | Utility weight: win % | `utility_w_winpct` | `uwin1`/`uwin2` (`uwin`) | static players |
| 20 | Utility weight: spread | `utility_w_spread` | `uspread1`/`uspread2` (`uspread`) | static players |
| 21 | Utility spread scale | `utility_spread_scale` | `uspreadscale1`/`uspreadscale2` (`uspreadscale`) | static players |
| 22 | Time limit (s) | `time_limit_secs` | `tl1`/`tl2` (`tlim`) | static players (always 0 for a simmer) |
| 23 | Move-gen margin | `movegen_margin` | `mmargin` | leave jobs |
| 24 | PEG schedule | `peg_stage_top_k` | `pegtopk1`/`pegtopk2` | players without the pre-endgame; opening-rack and leave jobs |
| 25 | PEG stride | `peg_scenario_stride` | `pegstride1`/`pegstride2` | the same |
| 26 | PEG opponent | `peg_opp_model` | `pegpess1`/`pegpess2` | the same |
| 27 | Nested lookahead | `peg_nested` | `pegnested1`/`pegnested2` | the same |
| 28 | Nested caps | `peg_nested_cand_caps` | `pegncaps` | the same, and players without nested lookahead |
| 29 | Nested depth | `peg_nested_max_depth` | `pegndepth` | the same |
| 30 | Nested strides | `peg_nested_strides` | `pegnstrides` | the same |
| 31 | Wordmap | `use_wordmap` | `w1`/`w2` (`wmp`) | n/a |
| 32 | Rack info table | `use_rit` | `rit1`/`rit2` (`rit`) | n/a (leave jobs refuse it) |

How the "Never read by" column should affect the display:

- A row that a job never reads is shown muted, as `LEAVE_UNUSED` already
  does for leave jobs (and, for the endgame and pre-endgame rows,
  `OPENING_RACK_UNUSED` for opening-rack jobs).
- A simulation-only row (6–9 and 15–22) shows "—" for a static player, and a
  pre-endgame row (24–30) shows "—" for a player that does not run it.
- When every player in the table is static, hide the simulation rows instead
  of showing columns of dashes. The page already shows the Endgame and
  Pre-endgame key rows only when a player solves.

The current "Search" summary row repeats rows 3, 7–9, 12 and 13. It works
better as the one-line heading above the table (`playersLine`) than as a row
inside it. That keeps this list to actual settings.

Wordmap and rack info table come last. They only make move generation faster:
a wordmap is exact, and a rack info table is checked against the hash pinned
in `derived_data`. Neither changes which move is played.

## Coverage check

Every argument birdtest has to state appears in one of the two lists above.
That covers each row marked Stored, Stored (per-player form) or Run by a job
type:

| MAGPIE argument(s) | List row |
|---|---|
| `var` | Job 2 |
| `ld` | Job 3 |
| `bdn` | Job 4 |
| `bb` | Job 12 |
| `cutoff` | Job 13 |
| `gp` | Job 1 (job type `game_pairs`) |
| `autoplay` | Job 1, 5, 7, 17 |
| `leavegen` | Job 1, 10, 11, 17, 18 |
| `generate`, `simulate` | Job 1 (job type `opening_rack`); `simulate` also needs Player 3 > 0 |
| `rack` | Job 8, 9, 18 |
| `l1`, `l2`, `lex` | Player 1 |
| `k1`, `k2`, `leaves` | Player 2 |
| `pl1`, `pl2`, `plies` | Player 3 |
| `np1`, `np2`, `numplays` | Player 4 |
| `s1`, `s2` | Player 5 |
| `winpct` | Player 6 |
| `i1`, `i2`, `iterations` | Player 7 |
| `sc1`, `sc2`, `scondition` | Player 8 |
| `si1`, `si2`, `sinfer` | Player 9 |
| `r1`, `r2` | Player 10 |
| `maxnumdplays` | Player 11 |
| `eplies1`, `eplies2` | Player 12 |
| `pegbag1`, `pegbag2` | Player 13 |
| `shplies` | Player 14 |
| `mi1`, `mi2`, `minplayiterations` | Player 15 |
| `th1`, `th2`, `threshold` | Player 16 |
| `sa1`, `sa2`, `sr` | Player 17 |
| `im1`, `im2`, `imargin` | Player 18 |
| `uwin1`, `uwin2`, `uwin` | Player 19 |
| `uspread1`, `uspread2`, `uspread` | Player 20 |
| `uspreadscale1`, `uspreadscale2`, `uspreadscale` | Player 21 |
| `tl1`, `tl2`, `tlim` | Player 22 |
| `mmargin` | Player 23 |
| `pegtopk1`, `pegtopk2` | Player 24 |
| `pegstride1`, `pegstride2` | Player 25 |
| `pegpess1`, `pegpess2` | Player 26 |
| `pegnested1`, `pegnested2` | Player 27 |
| `pegncaps` | Player 28 |
| `pegndepth` | Player 29 |
| `pegnstrides` | Player 30 |
| `w1`, `w2`, `wmp` | Player 31 |
| `rit1`, `rit2`, `rit` | Player 32 |

The **Recorded** rows are not settings, so they appear in neither list:

- **`seed`** is required on every request. The scheduler assigns it per task:
  games start at 1 and advance by the batch size, an opening-rack task uses
  its rack index, and a leave task draws one at random. It belongs on a
  task's page.
- **`version`, `builders`, `convert`** are record-keeping, not settings.
