# birdtest journeys

Every journey a visitor, a signed-in user and an admin can take on the
birdtest website, written as a checklist for a tester working on a **local
instance started with `scripts/dev.py`**. Working through it should leave you
confident the site can go live: each journey says what to do and what you
should see, including what the site must refuse.

[README.md](README.md) is how to run the site, [PLAN.md](PLAN.md) its design,
and [TESTING.md](TESTING.md) the automated tests, whose end-to-end suite covers
a handful of these journeys (`E-1`..`E-18`). This document is the whole
surface, by hand.

## How to use this document

- Journeys are numbered (`V-3`, `U-7`, `A-12`) so a result can be reported
  against one. Tick each checkbox that behaved as described, and for anything
  that did not, note the journey, the URL, the account and what you saw.
- **Do** lines are actions; **Expect** lines are what must happen. Text in
  quotes is what the page should say, word for word or close to it.
- Do one pass at phone width too (about 390 px; your browser's device toolbar
  will do): every page should be usable without scrolling sideways.

## Setting up

You need a built MAGPIE and its data (README, "Running locally"), Docker, and
two browsers — or one normal window and one private one — so you can be signed
in and signed out at once.

### Start the site

```bash
./scripts/dev.py --reset-db --worker-windows \
    --leavegen-job_ab --opening-rack-job_ab --games-job --pairs-job
```

That drops any earlier local database, starts the stack, seeds it with the
four jobs the job flags ask for (dev.py starts with none otherwise), starts four
MAGPIE workers — each in its own terminal window, where Ctrl-C stops it and
Enter starts it again — and opens **http://localhost:5173** (or the
`WEB_PORT` in your `.env`) signed in as the admin. Ctrl-C in dev.py stops the
workers and the stack; the database is kept, and
`./scripts/dev.py --worker-windows` (without `--reset-db`) picks it up again.

Journey [A-1](#a-1-become-the-first-admin), the first admin on an empty site,
needs a separate run with `--fresh`. Do it last: it empties the database.

### What the seed gives you

| | |
|---|---|
| **Admin** | `dev`, password `devpassword123!` |
| **Contributor accounts** | `dev-contributor-1` and `dev-contributor-2`, same password; workers 3 and 4 run under their API keys |
| **Anonymous workers** | Workers 1 and 2 |
| **Data** | The MAGPIE-DATA version your MAGPIE checkout installed, and MAGPIE's two-letter test data (version `20000101`: distribution `english_ab`, lexicon `CSW21_ab`, eight possible full racks) |
| **Player configs** | `static-equity` and `static-score` on CSW24; `ab-static-equity-no-rit` (the leave job's player) and `ab-sim-2ply-rack` (the opening-rack job's, a 2-ply simmer seeking 80% agreement) on CSW21_ab |
| **Jobs**, each active at 25% | "dev games (positions saved)" and "dev game pairs (first divergences saved)", `static-equity` against `static-score` on CSW24 (a games or pairs job between two configs is named for its pairing, so these read "…: static-equity vs static-score"; below they are called by the name alone); "dev opening racks (english_ab)"; "dev leave generation (english_ab)", six generations |

The games and pairs jobs run for hours: their cap is 100,000, and neither
test is acted on before 50,000 games or pairs. The two english_ab jobs finish
within minutes -- there are only eight racks -- and stay listed as completed.

### Useful locally

- **Mail** is not sent. Every message the site would send — confirmation
  links, password resets, notices — is printed in the backend's log instead:
  `docker compose logs -f backend`, and look for `---- email`. Any address
  will do when registering.
- **Switching accounts** without passwords: open
  `http://localhost:5173/api/dev/login?username=<name>&next=/` to be signed in
  as any account. This exists only locally; on the real site it must answer
  `404`.
- **A small data set** for leave generation: while dev.py runs, MAGPIE-DATA
  version `20000101` on branch `two-letter` is MAGPIE's two-letter test data
  (distribution `english_ab`, lexicon `CSW21_ab`, eight possible racks). See
  [A-3](#a-3-import-data) and
  [A-12](#a-12-a-leave-generation-job-start-to-finish).
- **Wordmaps, rack info tables and word info tables** are built by dev.py itself, within about
  fifteen seconds of a job needing one; on the real site a scheduled task does
  it every five minutes.

Job types, as the site names them:

| Type | What it computes | Progress is counted in |
|---|---|---|
| Games | Two players play each other; a significance test can decide which is better | games |
| Game Pairs | The same, in pairs of games with the first move swapped; also feeds ratings | pairs |
| Opening Rack Analysis | The ranked moves for every opening rack | racks |
| Leave Generation | Leave values, built generation by generation from self-play | generations |

---

## Part 1: Visitors and users

Use a signed-out window for the V journeys unless one says otherwise.

### V-1 Find your way around

- [ ] **Expect** a header with **birdtest**, **Jobs**, **Ratings**, **Players**,
  **Contributions** and **Users**, and on the right **Sign in** and **Register**;
  a footer "birdtest — crowdsourced crossword game research".
- [ ] **Do** click each nav link. **Expect** the current one highlighted, and
  no page erroring.
- [ ] **Do** narrow the window to phone width. **Expect** the header in two
  rows -- **birdtest** with **Sign in** and **Register** on its right, then
  the links -- and no page to scroll sideways.
- [ ] **Expect** no **Admin** link unless signed in as an admin.

### V-2 The home page

- [ ] **Expect** "Crowdsourced Crossword Game Research" and the buttons **Browse
  jobs** and **Create an account**.
- [ ] **Expect** next "Active Jobs" listing the seeded jobs still running as
  cards: name, status badge, "25% allocation". Clicking one opens its job page.
- [ ] **Expect** after it a "Contribute" section (not a card, like "Active
  Jobs") of four numbered steps: **Install MAGPIE** (linking MAGPIE's
  getting-started section), **(Optional) Create an account** (to contribute
  under a username: make an API key and copy it for the next step; else
  anonymously), **Create a contribute.txt file** (a table of its settings and
  each one's default -- `server` https://birdtest.org, `apikey` none "(step
  2)", `threads` every core but one, `maxtasks` 0, `idlewait` 5 -- and a
  sample. Here, off birdtest.org, the step is not optional and the sample is a
  `server` line with this site's own address; on birdtest.org the heading
  starts "(Optional)" and the sample sets `threads`) and **Run the contribute
  command** (`./bin/magpie contribute`).
- [ ] At phone width, **expect** the settings as a stacked list instead of a
  table -- each name and its default on one line, what it is under them --
  with nothing cut off at the screen's edge.
- [ ] (With every job deactivated, A-8) **expect** the Active Jobs section gone
  rather than an error.

### V-3 Browse the jobs

- [ ] **Do** open **Jobs**. **Expect** a table, newest first, with Name, Type,
  Status, Allocation ("25%") and Progress ("value / max
  unit": games, pairs, racks or generations).
- [ ] **Expect** the progress to have moved when you reload a minute later.
- [ ] A job that workers keep declining, and none has completed in 24 hours,
  shows a red **stalled** pill with a tooltip saying so. (Hard to stage
  locally; note it if you see it.)
- [ ] Signed out, **expect** no **New job** button; as an admin, **expect** one
  beside the title, opening the job form.

### V-4 A job's page: what every job shows

Open "dev game pairs (first divergences saved)".

- [ ] **Expect** the job's name as the title with "Game Pairs" beside it, then
  "CSW24 · classic · static, by equity vs static, by score".
- [ ] **Expect** a full-width Status card: the badge ("active") and nothing
  beside it -- the badge says it plainly, and its allocation has its own card.
  (Inactive: "Paused: no worker is offered its tasks until it is given an
  allocation." and "Its significance test is paused while the job is inactive (see the
  Significance Test card)." Set aside by the server, its reason instead, A-17;
  and under it, once any of its tasks has hit the time limit, "N tasks hit the
  time limit — lower the batch size.")
- [ ] **Expect** under it four cards, in a row on a wide screen and two by two
  on a narrower one: Allocation, Tasks completed, Throughput (the job's pace
  over the last hour in its own unit, "N pairs/hour"; "—" once nothing has
  finished in the hour) and Estimated time left.
- [ ] **Expect** a Progress card: a bar of "pairs completed", then **Tasks**
  with Waiting to be reissued / In progress / Done counts and no explanation
  under them; then "Created by dev, <date>" and "requires MAGPIE ≥ …".
- [ ] **Expect** a Contributors table: `dev-contributor-1`,
  `dev-contributor-2` and "Anonymous · <16 characters>" rows, with compute time
  (on a wide screen) and tasks completed, and under it "N movegens for this
  job".
- [ ] **Expect** last a **Job settings** card showing every row, with no **All
  settings** button, each label in Title Case: Type "Game Pairs", Variant,
  Letter Distribution, Board, Bingo Bonus 50, Maximum Pairs 100,000,
  Significance Test "yes (95%)" (one row, no separate Confidence %), Position
  Recorder "yes (first divergences)", Sim Cutoff, Threading "Intra-game
  parallelism (all threads on one game)", Minimum Pairs (50,000), Pairs Per Task and Oldest MAGPIE,
  and no download link.
- [ ] **Expect** after it a **Player settings** card: the players side by side,
  headed "Player 1" and "Player 2", each with its colour dot (green, purple:
  the colours their moves are drawn in under Saved positions) and its name
  linking to its config's page. It lists only the settings they differ in, as
  ordinary rows (Sorted By), with no block heading, tint or fold. (Two players
  alike say "These players' settings are identical."; one player lists its key
  rows.)
- [ ] **Do** press **All settings** on the Player settings card. **Expect**
  every setting, the key rows first -- Lexicon, Leaves, Sorted By, Move
  Recorder, Moves Generated, Plies, Uses Inference "—", Uses Preendgame "no"
  and Uses Endgame "no" -- then the rest (no simulation rows: neither
  simulates), Moves Recorded, Plies Recorded "—" (both are static) and Movegen
  Margin "—" (neither recorder keeps moves by equity). The button now reads
  **Different settings only** and puts the card back. On a job that records no
  positions ("tester no test", A-7), Moves Recorded is in grey: it never reads
  it.
- [ ] At phone width, **expect** the tables to wrap (or scroll inside the
  card), never the page.
- [ ] **Do** open `/jobs/00000000-0000-4000-8000-000000000000`. **Expect** "no
  such job".

### V-5 A job's page updates live

- [ ] **Do** keep the job's page open and do nothing. **Expect** its counts,
  progress bar and win/loss/draw figures to change without reloading.
- [ ] **Do** `docker compose restart backend` and wait. **Expect** the page to
  recover by itself within a minute.

### V-6 Games and game-pairs jobs: the score and the test

On the same job:

- [ ] **Expect**, after the settings, a **Match score** card with two tables,
  **All games** and **Games that diverged**, each a column per player
  (`static-equity`, `static-score`) and rows Wins, Losses (each player's are
  the other's wins), Draws, Average score and Average spread (signed, "+3.2"
  and "-3.2"); in each row the better value green and the worse red -- the
  fewer losses green -- equal values (Draws) neither; no chart. Under the
  first "Over N games, both games of every pair.", under the second just
  "Over M games: both games of the K pairs whose games did not play
  identically." The figures count games, not pairs.
- [ ] **Expect** after it a **Significance Test** card with a "running" badge and
  one sentence: "static-equity scores x% per game (95% interval a% to b%)." --
  no Elo or rating figure anywhere on the card. Then "The test is not acted on until 50,000 pairs
  are complete." and a bar: the interval shaded on a scale of player 1's
  score, a dashed line at 50% and a mark at the score, labelled "even at
  50%, static-equity's score x%". No W/L/D line of its own.
- [ ] **Do** open **What does the interval mean?**. **Expect** the question
  -- is one player better, at 95% confidence -- each pair one observation,
  scored by player 1's result across its two games, that the interval stays
  valid however often it is checked, and that the chance of naming a winner
  between two equal players is at most about 5%.
- [ ] **Expect** the pair-outcome table in the Significance Test card, a column per player:
  "Won both", "Won one, drew one", "Even", each cell "count (share%)", the
  higher of a row green and the lower red, "Even" the same for both; and a
  line on how many pairs diverged.
- [ ] On "dev games (positions saved)", **expect** one match-score table (no
  divergent one), the explanation per game (0, ½ or 1), no "both games of
  every pair" note, and no pair-outcome table.
- [ ] On a job an admin has deactivated (A-8), **expect** "paused while the job
  is inactive: no pairs are being played, so the test is not moving".
- [ ] On "tester no test" (A-7), **expect** the Match score card and no
  Significance Test card, and settings rows "Pairs To Play 20" and
  "Significance Test no" in place of the test's settings.
- [ ] On a games or pairs job's **Manage** page, **expect** the same Settings,
  Match score and (with a test) Significance Test cards after Progress, and no
  one-line summary of the test in Progress.

### V-7 A finished job says why it finished

A finished job's Status card says, beside its badge, "Finished <date>:" and why.
Make the jobs in [A-7](#a-7-create-jobs), [A-9](#a-9-finish-purge-and-delete)
and [A-12](#a-12-a-leave-generation-job-start-to-finish), then check:

- [ ] a job stopped at its cap: "it reached its cap of 20 pairs before its
  significance test decided: player 1 scored a% to b% per game, at 95%
  confidence";
- [ ] a job whose test decided: "its significance test found player 1 better
  at 95% confidence after N pairs: player 1 scored a% to b% per game" (or player 2);
- [ ] a job an admin forced: "an admin force-completed it before its test
  decided";
- [ ] a job without a test: "it played the 20 pairs it was set to";
- [ ] a leave-generation job: "its last generation was built".

### V-8 Opening-rack jobs: look up a rack

Open "dev opening racks (english_ab)".

- [ ] **Expect** a "racks settled" bar, and once it has finished (within
  minutes) "Racks analyzed 8 / 8", Racks settled 8 and Settled without a
  consensus figures, and "Each rack is analysed 2 to 5, until 80% agree on the
  best move; …" (its player, `ab-sim-2ply-rack`, simulates; its Job settings
  say Minimum Analyses Per Rack 2, Maximum Analyses Per Rack 5, Consensus %
  80%, and no Racks in all or Rack size rows), a **Look up a rack** box, and
  under it "Analysed racks to try:" with its racks as buttons, each of A and B
  only and none twice, in random order, then **Shuffle**. **Do** press
  **Shuffle**. **Expect** the racks drawn again (on a job this small, the
  same eight in another order; on a large one, other racks from all over the
  alphabet rather than a run of neighbours).
- [ ] **Do** click one. **Expect** it fills the box, says which move is best
  in how many of its analyses, and shows each analysis's ranked moves
  (Analysis, #, Move, Score, Equity, Win %, and P1-S, P1-BP, P2-S and P2-BP:
  the reply's and the next turn's average score and bingo percentage, each
  header explained on hover), every cell on one line.
- [ ] **Do** type that rack in lower case and in another order. **Expect** the
  same moves.
- [ ] **Do** search `QQQQQQQ`. **Expect** "No analysis stored for that rack
  yet."

### V-9 Leave-generation jobs

Open "dev leave generation (english_ab)" as soon as dev.py has started: it
finishes within minutes.

- [ ] While it runs, **expect** "Generation g of 6 — target … occurrences per
  rack", "N tasks and M games played this generation — live.", a bar of racks
  at target out of 8, and "Fewest occurrences so far: …" with when the rack
  figures were last merged (or "not computed yet"), and a note that totals are
  merged in batches. **Expect** a table of the six generations -- Generation,
  Target per rack (100, 200, 500, 1,000, 1,000, 1,000), State ("closed",
  "playing now" in bold, "to come"). Once done, "Finished …: its last
  generation was built", and every generation "closed".
- [ ] **Expect** a Settings card: Type "Leave Generation", Generations 6,
  Target Per Rack "100 → 200 → 500 → 1,000 → 1,000 → 1,000", Games Per Task
  1,000 and Racks Per Task, with no lexicon or wordmap row of
  its own; then one column headed "Player", `ab-static-equity-no-rit` linking to its
  config: search "static, by equity", Lexicon CSW21_ab -- and
  its leaves, recorder, Uses Preendgame and Uses Endgame in grey, under
  "Settings in grey are the player config's, and this job does not use
  them." **Do** **All settings** on the player card. **Expect** Wordmap "yes"
  and Rack Info Table "no", and Moves Recorded, Plies Recorded and Movegen
  Margin grey too.

### V-10 Saved positions

Open "dev game pairs (first divergences saved)".

- [ ] On a job just activated that has saved nothing yet, **expect** "No
  positions saved yet: one appears here as soon as the job has saved one.",
  and a position to appear by itself, without a reload, once one is saved.
- [ ] Signed out -- no account is needed to see them -- **expect** one turn of a game pair: "Turn T of a game pair,
  where the players first chose differently", then "Showing:" and a button
  per player ("static-equity's move", "static-score's move", each with its
  colour dot), the first pressed. Under them one game: "Game 1 of the pair ·
  PLAYER to move", "rack … · after MOVE (score) · played PLAY (score) · N
  moves ranked by static equity", the PLAY underlined in its player's colour;
  its board -- premium squares (TW, DW, TL, DL and a star on the centre),
  tiles each with a letter and a score (a blank a red lower-case letter with
  none), the tiles MOVE placed outlined, the tiles PLAY puts down drawn
  dashed in its player's colour, both racks and scores with **to move** on the
  player whose rack it is, and no CGP text -- and its ranked moves, the one
  played tinted and marked **played**. **Do** press the other button.
  **Expect** "Game 2 of the pair · OTHER PLAYER to move", the same board and
  rack, the other player's move drawn instead, and its own ranked moves, the
  top differing from the first's. (A pairs job saving every turn toggles the
  same way; a turn one game never reached alone has no buttons and says the
  other game has no position at that turn; a simmer's turns add **Win %**,
  **P1-S**, **P1-BP**, **P2-S** and **P2-BP** columns, the first two plies'
  average score and bingo percentage, P1 the reply; a solved turn's depth is
  **Solved Plies**.)
- [ ] On a wide screen (1920 × 1080), **expect** each game's moves beside its
  board with no gap between them, every cell on one line (a simmer's list
  with an **Iters** column after Win %), and nothing scrolling sideways.
  **Do** press **Random position** a few times. **Expect** the board the same
  size every time. Narrower than 1280 pixels, **expect** the moves under the
  board.
- [ ] On "dev sim games (positions saved)" (`--sim-games-job`), whose simmers
  infer, **expect** on a position past the first turn whose previous move was
  not a pass, under its moves, "Inferred from MOVE: N possible leaves, average
  equity E." and a table of up to ten leaves the opponent may have kept:
  Leave, Draws ("count (share%)") and Equity.
- [ ] On "dev games (positions saved)", **expect** one game's position: its
  board with the previous move outlined and the move played drawn dashed in
  green, both racks and scores under it, "played PLAY (score)" in the summary,
  and the played move marked in the list.
- [ ] **Do** **Random position** a few times. **Expect** another position
  each time (now and then the same one again), the board, racks and scores agreeing with each other; on a turn 1
  position an empty board and nothing outlined, and after an exchange
  (`(exch …)`) or a pass nothing outlined either.
- [ ] **Do** search the rack on the board, typed in lower case and another
  order. **Expect** "Position 1 with the rack …, newest first" and a pair
  whose first game (or, if only the second holds it, whose second) has that
  rack -- a pair whose two games both hold it once, not twice; where there are more, **Next** and **Previous** step through
  them.
- [ ] **Do** search `QQQQQQQ`. **Expect** "No saved position has the rack
  QQQQQQQ."
- [ ] **Expect** under the board each player's "Player N", score and (for
  one) **to move** on a line, its name on the next (a long one cut short,
  whole on hover), and the two racks' tiles level with each other.
- [ ] Narrow the window to phone width. **Expect** the board to shrink with
  it, nothing scrolling sideways; the rack box and **Search** on a line under
  **Random position**; and the players' buttons as the two halves of one
  control, each just a colour dot and the name.
- [ ] On "tester no test" (A-7), which saves no positions, **expect** no such section.

### V-11 Ratings

- [ ] Before any pool exists, **expect** "No rating pools yet" and an
  explanation.
- [ ] After [A-13](#a-13-rating-pools), **do** open **Ratings** → the pool.
  **Expect** "classic · english · standard15 — anchored at 1500", "Last fit
  <time> (membership) over N pairs from 1 job, K iterations", and a warning
  if the fit did not converge.
- [ ] **Expect** a dot plot with ±1 standard-error bars and the anchor in
  amber.
- [ ] **Expect** straight after it a "Cross table": every rated config against
  every other, best first, each cell the row's win % "±" its standard error
  over its average spread ("+6.8"), a blank diagonal, "·" where two never
  played, and the rating in the last column. Each pair of cells mirrors across
  the diagonal (100 − the win %, the same error, the spread negated). A cell
  over 50.0% is tinted green, under it red, and one showing 50.0% not at all;
  a score the ratings predict badly is amber text on that tint. Under the
  table a line: a gap predicts what it does between WESPA players, and the
  absolute level is only where the anchor was pinned. **Do** hover a cell.
  **Expect** "<row> against <column>: … The ratings predict …% (residual …
  percentage points)."
- [ ] As an admin, **expect** after it an "All configs" table (Rating (WESPA
  scale), ± SE, pairs; the anchor tagged), then the Anchor card. Signed out,
  **expect** neither.
- [ ] Narrow the window to phone width. **Expect** the cross table to scroll
  sideways inside its card with the config names staying put, and nothing
  else on the page scrolling sideways.
- [ ] **Expect** the ratings to shift on a reload a few minutes later: the pool
  is refitted every two minutes while its jobs play.
- [ ] Signed out or as a contributor, **expect** no membership controls and no
  **New rating pool** button.

### V-12 Player configs

- [ ] **Do** open **Players**. **Expect** the four seeded configs, newest first:
  name, how it searches ("static, by equity", "static, by score", "2-ply sim,
  200 iterations"), lexicon (CSW24, or CSW21_ab for the `ab-` ones), leaves,
  created.
- [ ] **Do** open `static-equity`. **Expect** "static, by equity" beside its
  name, and a table of Lexicon, Leaves, Sorted By, Move Recorder, Moves
  Generated, Plies, Uses Inference "—", Uses Preendgame "no" and Uses Endgame
  "no", and a JSON download; a simmer's (A-4) has the same rows, Uses
  Inference "yes" or "no". **Do** press **All settings**. **Expect** every
  setting in the same table, Lexicon still first; a simmer's with Win %
  Model, Maximum Total Iterations and Stopping % among them.
- [ ] **Do** open `/player-configs/00000000-0000-4000-8000-000000000000`.
  **Expect** "no such player config".
- [ ] Signed out, **expect** no **New player config** button on **Players**; as
  an admin, **expect** one beside the title, opening the player-config form.

### V-13 Users and contributors

- [ ] **Do** open **Users**. **Expect** "Registered users": `dev` with an
  **admin** tag and the two contributor accounts, with tasks completed and when
  they joined — and no email addresses anywhere.
- [ ] **Do** open **Contributions** (the page is still `/workers`). **Expect**
  the heading "Contributions" and all four workers ranked by
  movegens, most first, the column marked ↓: the two accounts by name, the
  two anonymous ones as "Anonymous · <16 characters>", never as their UUID;
  each with its movegens to the last digit ("1,234,567"), its compute time
  to the second in every unit it has ("2m 13s", "5h 20m 13s", "3d 4h 5m 6s";
  the hours on hover), tasks and last result.
- [ ] **Expect** above the list a "Site totals" card: three figures --
  Movegens to the last digit, Compute time and Tasks -- each the list's column
  summed; then a table of the same three by job type, a row each for Opening
  Rack Analysis, Games, Game Pairs and Leave Generation, 0 for a type nothing
  has run for.
- [ ] **Do** click a contributor's name. **Expect** a ▸ that turns ▾ and,
  under the row, a small table with the same headers: a row per job type with
  their movegens, compute time and tasks, adding up to the row's own. **Do**
  click it again. **Expect** it folded away.
- [ ] **Do** leave the page open, a contributor's row unfolded, while the
  workers run. **Expect** the counts -- the job-type totals, the rows and the
  unfolded breakdown -- to rise by themselves every 30 seconds, on the page and
  in the order chosen, and the row to stay unfolded.
- [ ] **Do** click **Compute time**, then **Tasks**. **Expect** the list
  re-ranked by each, most first, the arrow moving with it; still all four.
- [ ] At phone width, **expect** the site's three totals one to a line, its
  table wrapping inside its card, a "Rank by"
  row of the three columns above the list and only the ranked column beside
  the name. **Do** choose **Tasks**, and unfold a contributor. **Expect** the
  tasks column in its place, the breakdown under the row, and no sideways
  scroll.

### U-1 Register

Use a signed-out window.

- [ ] **Do** open **Register** with a 2-character username. **Expect** "must be
  between 3 and 32 characters".
- [ ] **Do** try `alice@`, `a@b` and `a..b@example.org` as the email. **Expect**
  "must be a valid email address".
- [ ] **Do** submit the form empty. **Expect** each error in red under its field
  ("must not be empty" under Password) — and never the browser's own popup.
- [ ] **Do** try the password `password123`. **Expect** "too weak — choose a
  longer, less predictable password". (The strength hint under the field is
  only a guide.)
- [ ] **Do** register `alice`, `alice@example.org`, a strong password.
  **Expect** "Check your email", with a hint that the link is in the backend's
  log.
- [ ] **Do** register `ALICE` again. **Expect** "that username is taken".
- [ ] **Do** register `alice2` with `alice@example.org`. **Expect** the same
  "Check your email" page — the site does not reveal the address is in use —
  and, in the backend's log, a notice to alice that an account already exists.

### U-2 Confirm the email address

- [ ] **Do** sign in as alice before confirming. **Expect** "confirm your email
  address before signing in — check your inbox".
- [ ] **Do** open the confirmation link from the backend's log. **Expect**
  "Email confirmed", then the sign-in page.
- [ ] **Do** open the same link again. **Expect** it still says confirmed.
- [ ] **Do** open `/confirm-email`, and `/confirm-email?code=nonsense`.
  **Expect** "That link is missing its confirmation code." and "that
  confirmation link is invalid or has expired".

### U-3 Sign in and out

- [ ] **Do** sign in with a wrong password, and as `nobody`. **Expect** the same
  "incorrect username or password" for both.
- [ ] **Do** sign in as `Alice` (any case). **Expect** to land on **Account**,
  alice's name in the header.
- [ ] **Do**, signed out, open `/account`. **Expect** the sign-in page, and to
  be sent back to `/account` after signing in.
- [ ] **Do** sign in from `/login?next=//evil.example`. **Expect** to land on
  **Account**, not another site.
- [ ] **Do** fail sign-in eleven times within a minute. **Expect** "too many
  requests".
- [ ] **Do** **Sign in** with both fields empty. **Expect** "must not be empty"
  in red under each, not the browser's popup.
- [ ] **Do** **Sign out**. **Expect** the home page, with **Sign in** back in
  the header.

### U-4 Reset a forgotten password

- [ ] **Do** **Sign in** → **Forgot password?** with `alice@example.org`.
  **Expect** "If that address has a confirmed account, a reset link is on its
  way. The link expires in 30 minutes." and a mail in the backend's log naming
  alice.
- [ ] **Do** the same with `nobody@example.org`. **Expect** the same message, and
  no mail.
- [ ] **Do** the same with `a@b`. **Expect** "must be a valid email address" in
  red under the field, and nothing sent.
- [ ] **Do** open the link and set `password123`. **Expect** "too weak — …" and
  the link still usable. **Then** set a strong password. **Expect** the sign-in
  page; the old password fails and the new one works.
- [ ] **Expect** alice signed out in the other browser too.
- [ ] **Do** reuse the link. **Expect** "that reset link is invalid or has
  expired".

### U-5 Your account

- [ ] **Do** click your username. **Expect** username, email, role
  ("contributor", or "admin" for `dev`) and tasks completed.

### U-6 API keys

As alice:

- [ ] **Do** **Generate key** with the label `laptop`. **Expect** "Copy this key
  now — it is not shown again.", the key and its line for `contribute.txt`
  (`apikey …`), and a row: laptop, created, last used "—", "active".
- [ ] **Do** reload. **Expect** the key itself gone and the row kept.
- [ ] **Do** **Deactivate** it, then **Activate** it. **Expect** the status to
  follow.
- [ ] **Do** **Revoke** it and confirm. **Expect** the row gone.
- [ ] Make a fresh key for U-8.

### U-7 Sign out everywhere

- [ ] **Do** sign in as alice in both browsers. In one, **Account** → **Sign out
  everywhere** → confirm. **Expect** that one on the sign-in page, and the
  other signed out when it next loads a page.

### U-8 Contribute with MAGPIE

The four worker windows are already contributing.

- [ ] **Do** Ctrl-C in worker 1's window. **Expect** "MAGPIE exited" and a
  prompt. **Do** Enter. **Expect** it contributing again under the same
  "Anonymous · …" name on **Contributions**.
- [ ] **Do** give worker 2 alice's key: stop it, add `apikey <key>` to
  `.dev-workers/worker-02/contribute.txt`, and start it. **Expect** its results
  credited to alice — her tasks completed rising on **Account** and **Users**,
  her name among job pages' contributors — and the key's "Last used" filled
  in.
- [ ] **Do** deactivate that key while the worker runs. **Expect** the worker
  refused. Reactivate the key and restart the worker.

### U-9 What a signed-in user may not do

As alice:

- [ ] **Do** open `/admin/jobs/new`. **Expect** to be sent to the home page.
- [ ] **Expect** no **Admin** link, no **Manage** button on job pages, no **New
  job** button on **Jobs**, no **New player config** button on **Players**, no
  **New rating pool** button, and no membership controls on a pool.

---

## Part 2: Admins

Sign in as `dev` (dev.py opens the site that way).

### A-1 Become the first admin

Do this last: it empties the database.

- [ ] **Do** stop dev.py and run `./scripts/dev.py --fresh`. **Expect** the site
  signed out, with no jobs, no player configs, no data and no accounts, and
  dev.py printing how to become the first admin.
- [ ] **Do** follow the three steps dev.py prints last: register in the
  browser; then, in a new terminal in the birdtest folder (dev.py keeps the
  first one), print the confirmation link from the backend's log and open it,
  and run the `psql` command with your username. **Expect** `UPDATE 1`, and on
  a reload an **Admin** link and "admin" on **Account**. There is no page or
  endpoint for this, by design; dev.py's steps end with how the live site
  differs.
- [ ] **Do** import the data as in A-3, from nothing: **expect** every file
  "new".

### A-2 The admin area

- [ ] **Do** click **Admin**. **Expect** the tabs New job, Allocation, Player
  configs, New rating pool, Input data, Fleet, Users, Bans, Derived data,
  Backups, Settings and Audit log. `/admin` itself goes to the jobs list.
- [ ] Signed out, **expect** any `/admin` page to send you to sign in and back.

### A-3 Import data

- [ ] **Do** open **Input data**. **Expect** the seeded files — lexica, leaves,
  letter distributions, layouts and win% models — each with its version,
  digest, size and "Pinned by" count.
- [ ] **Do** version `19990101`, **Fetch and diff**. **Expect** "Import failed:
  no data-19990101.tgz …".
- [ ] **Do** version `2026`. **Expect** "tarball_date must be YYYYMMDD". **Do**
  branch `../x`. **Expect** "that is not a git ref name …".
- [ ] **Expect** the two-letter files the english_ab jobs were seeded on --
  `CSW21_ab.kwg`, `CSW21_ab.klv2` and `english_ab.csv`, version `20000101`.
  (Started without an `_ab` job flag, importing
  `20000101` / `two-letter` gives "3 new, 0 changed, 0 already known." and
  **Insert 3 rows** "Confirmed. 3 rows inserted.")
- [ ] **Do** import `20000101` / `two-letter` again. **Expect** "0 new, 0
  changed, 3 already known." and "No new data to insert.", with no **Insert**
  button, and `input_data.import_nothing_new` in the audit log.
- [ ] **Do** re-import the seeded version (its date is in the file list) on
  branch `main`, and leave the page while it downloads; come back. **Expect**
  the import picked up where it was, and every file "already known".
- [ ] **Do** click **Insert** in two tabs on one staged import. **Expect** the
  second refused: "this import is confirmed, not staged".
- [ ] **Expect** **Delete** disabled on every pinned file. Keep the two-letter
  files for A-12.

### A-4 Create player configs

- [ ] **Do** **Player configs** → **New**: `tester-static`, **Move Recorder
  (-r)** best, the CSW24 lexicon and leaves, **Sorted By (-s)** equity.
  **Expect** it listed, and on the public **Players** page. **Expect** every
  field named as the settings tables name it, in Title Case, with its MAGPIE
  argument beside it, and no spinner arrows on any number box (the arrow keys
  still step it).
- [ ] **Do** `tester-static-all`: the same, with **Move Recorder (-r)** all
  (A-7's opening-rack job uses it).
- [ ] **Do** a simmer: tick **Simming Player**, choose the win% model, **Plies
  (-pl)** 1, **Maximum Total Iterations (-i)** 100. **Expect** it saved and described "1-ply sim, 100
  iterations".
- [ ] **Do** tick **Simming Player** without a win% model. **Expect** the
  browser to insist on one.
- [ ] **Do** set Stopping % to 100, then 0. **Expect** the browser to hold the
  form: "Stopping % must be above 0 and below 100."
- [ ] **Do** **Show advanced options**. **Expect** **Wordmap (-w)** and **Rack
  Info Table (-rit)** ticked, with a note that a table costs a contributor
  about 1.9 GB, and **Word Info Table (-wit)** ticked too, with a note that
  the server builds it once per lexicon; unticking it hides the note.
- [ ] **Do** pair the CSW24 lexicon with the `FRA20` leaves. **Expect** "leaves
  … are not compatible with lexicon …".
- [ ] **Do** look at **Endgame and Preendgame**. **Expect** **Uses
  Preendgame** greyed out until **Uses Endgame** is ticked, and unticked
  again when the endgame is.
- [ ] **Do** `tester-solver`: static, tick **Uses Endgame** (**Endgame Plies
  (-eplies1)** 6) and **Uses Preendgame** (**Preendgame Maximum Bag
  (-pegbag1)** 2). **Expect** it saved and described "static, by equity ·
  6-ply endgame · PEG ≤2"; on its page Uses Endgame "yes (6 plies)", Uses
  Preendgame "yes (bag ≤ 2)", and with **All settings** the PEG Schedule
  MAGPIE defaults to (32, 16, 8, 4, 2; Nested Caps 8, 4, 2; Nested Strides 1,
  1, 5, 7).
- [ ] **Do** **Show pre-endgame schedule** and type `4, x` as the **PEG
  Schedule (-pegtopk1)**. **Expect** "PEG Schedule: "x" is not a whole
  number." and nothing saved.
- [ ] **Do** make a leave job with `tester-solver` as its player. **Expect** the
  form to say it "solves endgames" before the submit.
- [ ] **Expect** no way to edit a config: they never change once made.

### A-5 Delete a player config

- [ ] **Do** delete `tester-static` and confirm. **Expect** it gone.
- [ ] **Do** delete `static-equity`. **Expect** "a job, a rating pool, a rating
  history or a clone references this player config".

### A-6 Wordmaps, rack info tables and word info tables

- [ ] **Do** open **Derived data**. **Expect** the CSW24 wordmap and rack info
  table the seeded jobs use, "built", with size and SHA-256.
- [ ] After making a job on a new lexicon (A-12), **expect** its wordmap
  "pending", then "built" within about fifteen seconds, and the job handing
  out work only then.
- [ ] **Do** keep **Derived data** open, with nothing building, while you make
  that job in another tab. **Expect** its new rows to appear within about ten
  seconds, without a reload.
- [ ] **Do** make a player config, leaving **Word Info Table (-wit)** ticked, and a
  games job with it. **Expect** a "Word info table" row named for the lexicon
  on **Derived data** and on the job's page, built within seconds, and the
  config's page to say "Word Info Table: yes" under **All settings**.

### A-7 Create jobs

**New job**, then **Create job**; each is created **inactive** at 0%. An
opening-rack or leave job lands on its admin page; a games or game-pairs job
lands on **Allocation**, the new job marked "new" and listed first. A games or
pairs job's players are a checklist, seated in the order ticked: tick the
first-named player first, so `static-equity` vs `static-score` is equity's
player-1 side, and each such job is named "{name}: A vs B" (so "tester no test"
is "tester no test: static-equity vs static-score"; below it is called by its
name alone). The fields are named as the settings tables
name them (**Job Name**, **Job Type**, **Letter Distribution**, **Board**,
**Pairs Per Task**, …), and the job types read "Opening Rack Analysis",
"Games", "Game Pairs" and "Leave Generation". The letter distribution and
board start on "Choose…": pick `english` and `standard15` each time.

- [ ] **Game Pairs** "tester no test": `static-equity` vs `static-score`, 10
  pairs per task. **Expect** **Significance Test** unticked, no Confidence % or
  Minimum Pairs fields, and the target labelled "Pairs To Play": set it to 20.
  It finishes once it has played them (V-7).
- [ ] **Game Pairs** "tester cap": the same, with **Significance Test** ticked:
  **expect** Confidence % (95) and Minimum Pairs to appear and the target to
  read "Maximum Pairs". Minimum Pairs 20, Maximum Pairs 20. It finishes at
  its cap (V-7).
- [ ] **Game Pairs** "tester decided": `static-equity` vs `static-score`, 10 per
  task, **Significance Test** ticked, Confidence % 95, Minimum Pairs 100, Maximum
  Pairs 5,000. Equity against score is a large difference, so its test should
  decide for `static-equity` once it runs (V-7); if it has not after an hour,
  note where its interval got to.
- [ ] **Games** "tester positions": the same two players, 2 per task, 20 Games
  To Play, **Position Recorder** ticked. **Expect** the batch's maximum to
  drop to 1,000, and, once it runs, a saved-positions section on its page
  (V-10), and no **Only Where Each Pair First Diverges** option: a games job
  has no pairs.
- [ ] **Game Pairs** "tester divergences": `static-equity` vs `static-score`, 5
  pairs per task, 50 Pairs To Play, **Position Recorder** and then **Only
  Where Each Pair First Diverges** ticked (it appears only once recording is).
  **Expect** its settings to say "Position Recorder: yes (first divergences)", and
  on its page each saved position to be a pair's two games at one turn, the
  same board and rack, each player to move in one, their ranked moves
  differing at the top; **Random position** never shows a lone game.
- [ ] **Games** "tester round robin" with four configs ticked (`static-equity`,
  `static-score`, `tester-static-all` and one more). **Expect** the preview to
  say "4 configs → 6 jobs" and list "tester round robin: static-equity vs
  static-score" and the five other pairings, each once, the first ticked on the
  left; **Create 6 jobs**. **Expect** **Allocation** with "The 6 new jobs are
  marked new …", all six inactive at 0%. **Do** tick one config alone.
  **Expect** "1 config → 1 self-play job". **Do** tick 13. **Expect** "At most
  12 player configs: 13 are ticked." and the submit refused. Two simmers on
  different win% models in a round robin: **expect** the error to name their
  pairing, and nothing created.
- [ ] **Games**: **expect** the batch field to step by 2, and the browser to
  refuse an odd number.
- [ ] **Games** or **Game Pairs**: **expect** a **Threading** select on
  "Intra-game parallelism (all threads on one game)", saying intra-game parallelism gives all
  threads to one game's simulation and makes iteration-bounded simulations
  reproducible, per-game parallelism plays games in parallel, and it only
  matters when a player simulates. **Do** create "tester no test" with
  "Per-game parallelism (one game per thread)". **Expect** its Job settings to say
  Threading "Per-game parallelism (one game per thread)", and the others "Intra-game parallelism (all threads on one game)". No other job type has the field.
- [ ] **Opening Rack Analysis** with `static-equity` (recorder best, 10 plays
  kept). **Expect** a warning that only one play per rack would be stored, and
  creation refused; with `tester-static-all` (A-4) it is accepted.
  **Expect** under **Analyses Per Rack** "One: tester-static-all is static, …"
  and no number boxes; on its admin page, a Consensus card saying its player
  is static, with no consensus to change.
- [ ] **Opening Rack Analysis** "tester consensus" on english_ab with the
  seeded simmer `ab-sim-2ply-rack`: **Minimum Analyses Per Rack** 2,
  **Maximum Analyses Per Rack** 3, **Consensus %** 100. **Expect** the note
  under them to say each rack is analysed at least 2 times until 100% agree,
  or 3 times. **Do** set the maximum to 1 first. **Expect** the Consensus %
  box disabled and "One analysis per rack." Created and activated, **expect**
  its Job settings to say Minimum Analyses Per Rack 2, Maximum Analyses Per
  Rack 3 and Consensus % 100%, Racks analyzed / Racks settled / Settled
  without a consensus figures, and, once it finishes, every one of its 8
  racks settled. **Do** look up one of its racks. **Expect** "MOVE is the best
  move in k of n analyses (…%)" and an Analysis column numbering each
  analysis's ranked moves, each with its Win % and P1-S…P2-BP.
- [ ] **Do** change "tester consensus" once it has completed, on its
  **Manage** page's **Consensus** card: the same three boxes, filled with its
  settings, **Save** disabled until one changes. Set the minimum above the
  maximum. **Expect** "The most analyses must be at least the fewest, and at
  most 100." Set Minimum 3 and Maximum 4. **Expect** "Saved: N racks
  unsettled, so the job reopened, inactive at 0% — give it an allocation on the
  Allocation page to run it." -- inactive at 0%, any final export of it now a
  snapshot, and its Job settings saying 3 and 4. The audit log has
  `job.consensus_changed` ("min 2 -> 3, max 3 -> 4; N racks unsettled") and
  `job.deactivated` (from completed). **Do** give it 25% on **Allocation**.
  Once it has completed again, set Consensus % to 60. **Expect** "Saved: the job is
  completed, with 0 racks unsettled." Lowering the settings on an active job
  until no rack is unsettled completes it.
- [ ] **Do** leave the letter distribution on "Choose…" and submit. **Expect**
  the browser to hold the form on that field.
- [ ] **Do** clear a number box and submit. **Expect** "Fill in every setting: …
  is empty."
- [ ] **Do** tick **Significance Test** and set Confidence % to 100. **Expect** the
  browser to hold the form (99.99 is the most it takes); sent anyway, the
  server refuses it: "must be above 50 and below 100".

### A-8 Activate, share out and deactivate

The allocation is the only switch: above 0% a job is active, at 0% inactive,
and both are set on **Allocation** (admin tabs). The seeded games and pairs
jobs hold 50% between them once the two english_ab jobs have finished (100%
while those run).

- [ ] **Do** open a job's **Manage** page. **Expect** "Allocation: N% of
  claims" read-only with a link to the Allocation page, and no Activate or
  Deactivate button.
- [ ] **Do** open **Allocation**. **Expect** every active and inactive job,
  none completed, each with its status, its share now and a box set to it (0
  for an inactive job), the total "N% of 100% allocated", and **No changes**
  disabled.
- [ ] **Do** set "tester cap" to 60 and **Save**. **Expect** the total red,
  "lower some jobs before saving", and the save disabled.
- [ ] **Do** set "dev games (positions saved)" to 0 and **Save**. **Expect**
  "Saved: 1 job changed.", the games job inactive at 0%, and on its **Manage**
  page **inactive** in the Status card with "Paused: no worker is offered its
  tasks …", the same cards as the public page's; its public page no longer
  says "live".
- [ ] **Do** set "tester cap" to 25 and **Save**. **Expect** it active at 25%
  and the workers taking its tasks.
- [ ] **Do** type 150, then 2.5, in a box. **Expect** "Every allocation must be
  a whole number from 0 to 100." and the save disabled.
- [ ] **Do** change an active job's share and **Save** again. **Expect** the
  new share taken.
- [ ] **Do** set every job but one to 0, the one at 25%. **Expect** that job to
  get all the work: shares weigh only against the other active jobs.
- [ ] **Do** set the seeded games job to 0 and "tester cap" to its share plus
  the games job's. **Expect** the total unchanged, the two rows in bold and
  **Save 2 changes**. **Save**. **Expect** "Saved: 2 jobs changed.", the games
  job inactive at 0% and "tester cap" at its new share, and in the audit log
  `job.deactivated` ("25% -> 0%") and `job.allocation_changed` ("25% -> 50%").
- [ ] **Do** **Share equally**. **Expect** 100% split among the jobs above 0% and
  the ones marked new.
- [ ] **Do** create a game-pairs job with **Oldest MAGPIE** `9.9.9`, and
  make it the only active one. **Expect** every worker told its MAGPIE is too
  old and stopping. Set it to 0% and restart the workers.

### A-9 Finish, purge and delete

- [ ] **Do** let "tester cap" run. **Expect** it completed within a few
  minutes, with the cap note (V-7), its allocation 0% ("completed jobs hold
  0%"), Force complete disabled, and the job gone from **Allocation**.
- [ ] **Do** give "tester decided" an allocation. **Expect** a decided test (V-7).
- [ ] **Do** **Force complete** a running job and confirm. **Expect** "Job
  force-completed.", its allocation 0%, and "an admin force-completed it
  before its test decided".
- [ ] **Do** **Purge results** on a completed job and confirm. **Expect**
  "Results purged. The job is inactive at 0%: give it an allocation on the
  Allocation page to start over from its first task." and its counts at zero.
  On an active job the same: a purge leaves every job inactive at 0%.
- [ ] **Do** open a job's **Manage** page in two tabs and **Delete job** in one.
  **Expect** that tab on the jobs list, without the job, and the other saying
  "This job no longer exists." with every action disabled.

### A-10 Export a job

- [ ] **Do** **Export results** on "tester cap". **Expect** "Building…", then
  "Final results · N rows · N MB · download · SHA-256 of the .gz …", with links
  valid for an hour.
- [ ] **Do** download. **Expect** a gzipped file of one JSON object per line.
- [ ] **Do** **Export a snapshot** on an active job. **Expect** "Snapshot as of
  <time> — job still running" in amber, with a download. **Do** it again a
  minute later. **Expect** a later time and more rows.
- [ ] **Do** export "tester positions" while it runs, then again once it has
  completed. **Expect** the first a snapshot with its positions as a second
  download, and the button on the completed job to read **Build the final
  export** until the final one is built, then "Final results".
- [ ] **Do** **Export a snapshot** on an active leave-generation job. **Expect**
  the note that rack totals are as of the last merge.

### A-11 Data gaps

- [ ] **Do** make a worker whose data lacks a two-letter file: in a new
  directory, `cp -rs ~/MAGPIE/data data`, delete
  `data/lexica/CSW21_ab.kwg`, write a `contribute.txt` naming the site
  (README, "Contributing with MAGPIE"), and run `~/MAGPIE/bin/magpie
  contribute` there while the two-letter job (A-12) is active. **Expect** it to
  decline that job, and the job's **Manage** page to list `kwg CSW21_ab` under
  data gaps with its workers and declines. Other jobs show "No worker has
  declined this job."

### A-12 A leave-generation job, start to finish

On the two-letter data imported in A-3: eight possible racks, so a generation
closes in seconds rather than never.

- [ ] **Do** **Player configs** → **New**: `tester-ab`, **Move Recorder (-r)**
  best, the `CSW21_ab` lexicon and leaves, **Sorted By (-s)** equity, static,
  and **Rack Info Table (-rit)** unticked under **Show advanced options**.
- [ ] **Do** **New job**: Leave Generation, "tester leaves", letter distribution
  `english_ab`, board `standard15`, player config `ab-sim-2ply-rack`.
  **Expect** a red "ab-sim-2ply-rack simulates 2 plies and asks for a rack info table; leave
  generation plays statically on equity, without a rack info table or endgame
  solving." (Submitted anyway, the server refuses it the same way.) **Do** pick `tester-ab`, games per task 1,000, racks per task 50,
  occurrences per rack per generation "100, 200, 300". **Expect** "3
  generations: 100 → 200 → 300." under the targets. (**Do** type "100, 0" there first. **Expect** a red
  "Every target must be between 1 and 1,000,000, not 0.")
- [ ] **Do** make room (A-8) and give it 25% on **Allocation**. **Expect** its wordmap built
  on **Derived data** within about fifteen seconds (A-6).
- [ ] **Expect**, on its public page, the generations closing one after another
  — "Generation 2 of 3 — target 200 occurrences per rack", "Targets by
  generation: 100, 200, 300.", racks at target "8 / 8" — and the job completed
  within about a minute: "its last generation was built".
- [ ] **Do** **Merge progress now** on its **Manage** page. **Expect** "Merged N
  staged results into M racks." (N may be 0 once it has finished.)
- [ ] **Do** **Check artifacts**. **Expect** "Checked 3 generations: 0
  rewritten, 0 differing from the recorded hash." and a row a generation.
- [ ] **Do** **Force rebuild** and confirm. **Expect** every generation
  rewritten. (On an active job it is refused: "deactivate the job (0% on
  the allocation page) before forcing a rebuild …".)
- [ ] **Do** export it ([A-10](#a-10-export-a-job)). **Expect** a download.

### A-13 Rating pools

- [ ] **Do** **New rating pool**. **Expect** the letter distribution and board
  layout on "Choose…", not filled in.
- [ ] **Do** **New rating pool**: "tester pool", classic, the `english`
  distribution and `standard15` the seeded jobs use, anchor `static-equity` at
  1500, with `static-score` ticked. **Expect** the pool's page, fitted, both
  rated from the seeded pairs job.
- [ ] **Do** a second pool with nothing ticked. **Expect** "Never computed."
  until a member is added.
- [ ] **Do** **Remove** `static-score`. **Expect** it gone and the anchor left
  alone, with nothing to be rated against. **Add** it back. **Expect** it rated
  again. **Expect** no Remove on the anchor.
- [ ] **Do** **Recompute**. **Expect** "Last fit … (manual)".
- [ ] **Do** under **Anchor**, pick `static-score` and 1600, **Save**. **Expect**
  "anchored at 1600", "Last fit … (anchor)", `static-score` marked anchor at
  1600.0 and every other rating moved by the same amount. **Expect**
  `static-equity` now has a **Remove** button.
- [ ] **Do** pick a config that is not a member as the anchor (from "Other
  player configs") and **Save**. **Expect** it listed as a member and the
  anchor.
- [ ] **Do** **Delete pool** on the second pool, and confirm. **Expect** the
  ratings list, without it.
- [ ] **Expect** no Anchor card or Delete button when signed out or as a
  non-admin.

### A-14 Delete a user

- [ ] **Do** **Users** → **Delete** on `alice2` (U-1), and confirm. **Expect** it
  gone from the list and unable to sign in.
- [ ] **Do** delete `dev-contributor-2`. **Expect** worker 4, which runs under
  its key, refused, and its results still counted — shown as `deleted-<id>` on
  **Contributions** and job pages.
- [ ] **Do** delete `dev`. **Expect** "you cannot delete your own account".

### A-15 Ban a worker

- [ ] **Do** **Bans** → **Select** an anonymous worker under "Known workers",
  give a reason, **Ban worker**. **Expect** "Banned …" and the ban listed.
- [ ] **Expect** that worker's window to show its claims refused: "this worker
  identity is banned".
- [ ] **Do** ban `dev-contributor-1` by user id. **Expect** worker 3 refused,
  while `dev-contributor-1` can still sign in to the website.
- [ ] **Do** ban the same identity twice. **Expect** "that already exists".
- [ ] **Do** **Unban** both and confirm. **Expect** "Lifted the ban on …";
  restart the two workers and they contribute again.

### A-16 Fleet, backups and the audit log

- [ ] **Do** **Fleet**. **Expect** your MAGPIE's version, the workers that ran
  it, and their claims over the last seven days.
- [ ] **Do** **Backups**. **Expect** "No successful backup has ever been
  recorded." in red: nothing backs up a local stack.
- [ ] **Do** **Audit log**. **Expect** everything done above, newest first:
  `job.created`, `job.activated`, `job.deactivated`, `job.completed`,
  `job.consensus_changed`, `job.time_limit_changed`, `job.purged`, `job.deleted`, `job.export_started`,
  `input_data.import_staged`, `input_data.import_confirmed`,
  `input_data.import_nothing_new`,
  `rating_pool.created`, `rating_pool.member_added`, `user.deleted`,
  `worker.banned`, `worker.unbanned`, `job.set_aside` and
  more.
- [ ] **Do** filter by action `job.activated`, then by target type `job`.
  **Expect** only those, and paging to keep the filter.

### A-17 A job's task time limit

Needs a MAGPIE that stops a task at the assignment's `max_task_seconds` and
declines it `time_limit` (from the pin after this change). An older one runs
on, and the server takes the claim back a minute past its deadline instead,
which -- the worker still heartbeating -- counts the same, at the job's next
claim. A claim whose worker had stopped heartbeating counts toward nothing.

- [ ] **Expect** no **Settings** in the admin menu: there is no site-wide
  limit, and the `/admin/settings` page is gone.
- [ ] **Do** open **Create job**. **Expect** Task Time Limit (Seconds) 3600
  beside Oldest MAGPIE, with "1h." under it. **Do** type 599. **Expect** "A
  whole number of seconds from 600 to 86,400 (ten minutes to a day)." and the
  form refusing to submit; sent anyway, the server refuses it on the field.
- [ ] **Do** create a **Games** job of A-4's simmer against itself, with a
  batch big enough that a task takes longer than ten minutes, and a limit of
  600. **Expect** its Job settings to say Task Time Limit "10m (600
  seconds)".
- [ ] **Do** open its Manage page. **Expect** Task Time Limit (Seconds) 600 in
  the Controls card, **Save** disabled until the value changes. **Do** set
  700 and **Save**. **Expect** "Saved. Claims made from now on are given the
  new limit; those already made keep their deadlines.", its Job settings
  saying "11m 40s (700 seconds)", and `job.time_limit_changed` in the audit
  log ("max_task_seconds 600 -> 700"). **Do** set it back to 600.
- [ ] **Do** give it an allocation.
  **Expect** each task MAGPIE claims to stop at ten minutes, and the job's
  page to say "1 task hit this job's 10m time limit — lower the batch size, or
  raise the limit." and then more. **Expect**, after the third in a row, the job inactive at 0%, its
  Status card reading "Set aside by the server: 3 tasks in a row hit the
  10-minute time limit with none completed between: …", and `job.set_aside`
  in the audit log ("N% -> 0%: …").
- [ ] **Do** give it an allocation again. **Expect** it active, the reason gone
  and the count of tasks that hit the limit still shown; three more in a row
  set it aside again.
- [ ] **Do** raise its limit to 3600 on its Manage page. **Expect** the
  claims made from then on given an hour (the assignment's
  `max_task_seconds`), and those already running keeping their ten minutes.

---

## Part 3: From the command line

The site is a single-page app over a JSON API, and everything the pages do can
be done with `curl`. These are a handful of examples — not the whole API,
which is in [PLAN.md](PLAN.md)'s API tables — each with what to expect, so a
run doubles as a check.

Set up once:

```bash
SITE=http://localhost:5173     # or your WEB_PORT
JAR=$(mktemp)                  # cookie jar for a signed-in session
JOB=...                        # a job id, from its page's URL
```

### Anyone, signed out

**The active jobs.** Expect `{"items":[...],"total":N,...}`.

```bash
curl -s "$SITE/api/jobs?status=active&per_page=20" | jq '.items[] | {id, name, job_type, status}'
```

**A job's statistics and full configuration.** An unknown id is `404` with
`"no such job"`.

```bash
curl -s "$SITE/api/jobs/$JOB"        | jq '{status: .job.status, games, opening_racks}'
curl -s "$SITE/api/jobs/$JOB/config" | jq '.players[] | {name, lexicon, num_plies}'
```

**One rack of an opening-rack job**, in any order and case. Expect its ranked
moves, or an empty list for a rack not analysed yet.

```bash
curl -s "$SITE/api/jobs/$JOB/results?rack=aeinrst" | jq '.items[:3]'
```

**A job, live.** Server-sent events: a `stats` event with the statistics, then
another as results arrive. Ctrl-C to stop.

```bash
curl -sN "$SITE/api/jobs/$JOB/stream"
```

**A job's saved positions** (the seeded "(positions saved)" job): one at
random, or those with one rack, up to 20 a page, newest first;
`next_cursor`, when present, fetches the next page (`&cursor=…`). The board
they are drawn on is public too.

```bash
curl -s "$SITE/api/jobs/$JOB/positions/random" \
  | jq '{game_index, turn_number, rack, position, previous_move, best: .moves[0].move}'
curl -s "$SITE/api/jobs/$JOB/positions?rack=aeinrst&per_page=5" \
  | jq '.items[] | {game_index, turn_number, rack, best: .moves[0].move}'
curl -s "$SITE/api/jobs/$JOB/board" | jq '{start, first_row: .squares[0], letters: .letters[:3]}'
```

**Player configs, rating pools, and what a contributor needs.**

```bash
curl -s "$SITE/api/player-configs" | jq '.[] | {id, name, lexicon, num_plies}'
curl -s "$SITE/api/rating-pools"   | jq '.[] | {id, name, members}'
curl -s "$SITE/api/worker/client-version"
```

**What must be refused.**

```bash
curl -s -o /dev/null -w '%{http_code}\n' "$SITE/api/me"                     # 401: not signed in
curl -s -o /dev/null -w '%{http_code}\n' "$SITE/api/admin/player-configs"   # 401
```

### A signed-in user

Sign in once; the session and a CSRF token land in the cookie jar. A request
that changes something (`POST`, `PATCH`, `DELETE`) must send the CSRF cookie
back in an `X-CSRF-Token` header — without it, expect `403` "missing CSRF
header".

```bash
curl -s -c "$JAR" -H 'Content-Type: application/json' \
  -d '{"username":"dev-contributor-1","password":"devpassword123!"}' "$SITE/api/auth/login"
CSRF=$(awk '$6 == "birdtest_csrf" {print $7}' "$JAR")
```

Expect `{"username":"dev-contributor-1","is_admin":false}`. A wrong password is
`401`, and more than ten attempts a minute from one address `429`.

**Who am I.**

```bash
curl -s -b "$JAR" "$SITE/api/me"
```

**Make an API key.** The key is in this answer and never again; the list
shows only its label and when it was last used. A key is a worker credential
for `contribute.txt` (`apikey <key>`): the site's own routes do not accept it.

```bash
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"label":"laptop"}' "$SITE/api/me/api-keys"
curl -s -b "$JAR" "$SITE/api/me/api-keys"
```

**An admin route, as a user**, is `403`:

```bash
curl -s -o /dev/null -w '%{http_code}\n' -b "$JAR" "$SITE/api/admin/fleet"
```

**Sign out.** `/api/me` is `401` again afterwards.

```bash
curl -s -b "$JAR" -c "$JAR" -H "X-CSRF-Token: $CSRF" -X POST "$SITE/api/auth/logout"
```

### An admin

Sign in the same way as `dev` (`"is_admin":true` in the answer).

**Create a player config.** File ids come from the input data. Expect `201`
and the stored config, every default filled in.

```bash
curl -s -b "$JAR" "$SITE/api/admin/input-data" | jq '.[] | {id, role, name, tarball_date}'
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"name":"cli-static-equity","recorder_type":"best","sort_strategy":"equity",
       "kwg_id":"<kwg id>","klv_id":"<klv id>","num_plays_recorded":1}' \
  "$SITE/api/admin/player-configs"
```

**Create a job, activate it, deactivate it.** Created inactive at 0% (`201`);
an allocation above 0% activates it (`200`), 0% deactivates it, and a share
that would take the active jobs past 100% is `409` — with the seed's games and
pairs jobs active, 50% is the most left. The allocations endpoint is the only
switch; it answers `{ "jobs": [...] }`, each named job as it now stands.
Creation answers `{ "jobs": [...] }` too: the two configs here make one job,
"cli test: A vs B", and three or more would make a job per pairing. Set `JOB`
to its id. This one runs no significance test and plays its 20 pairs;
`"test_enabled": true` turns the test on, with `min_pairs` then required (at
least 1, at most `max_pairs`) and `confidence_pct` optional (95 by default,
strictly between 50 and 100), and `min_pairs` or `confidence_pct` sent without
it is a `400` naming each.

```bash
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"name":"cli test","job_type":"game_pairs","variant":"classic",
       "letterdist_id":"<letterdist id>","layout_id":"<layout id>",
       "player_config_ids":["<config id>","<other config id>"],
       "pairs_per_batch":10,"max_pairs":20}' \
  "$SITE/api/admin/jobs" | jq '.jobs[] | {id, name}'
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' -X PUT \
  -d '{"allocations":[{"job_id":"'"$JOB"'","allocation":4}]}' "$SITE/api/admin/jobs/allocations"
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' -X PUT \
  -d '{"allocations":[{"job_id":"'"$JOB"'","allocation":0}]}' "$SITE/api/admin/jobs/allocations"
```

**Change an opening-rack job's consensus.** Only the fields sent change; the
answer is the job, its settings, how many racks they leave unsettled, and
whether a completed job reopened (inactive at 0%, until it is given an
allocation). A job of
another type, or settings creation would refuse, is a `400`.

```bash
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -X PATCH -d '{"min_results_per_rack":2,"max_results_per_rack":3,"consensus_pct":80}' \
  "$SITE/api/admin/jobs/<opening-rack job id>/consensus" | jq '{unsettled_racks, reopened, status: .job.status}'
```

**Operational health**: the fleet, the derived-file queue, the backups, and
the audit log filtered to one action.

```bash
curl -s -b "$JAR" "$SITE/api/admin/fleet"
curl -s -b "$JAR" "$SITE/api/admin/derived-data" | jq '.[] | {role, name, state}'
curl -s -b "$JAR" "$SITE/api/admin/backups"
curl -s -b "$JAR" "$SITE/api/admin/audit-log?action=job.activated&per_page=5"
```

**A job's task time limit.** Read it with the job's settings, and set it: 600
to 86,400 seconds, or a `400` on `max_task_seconds`. The job's claims made
after the change are given it.

```bash
curl -s "$SITE/api/jobs/$JOB/config" | jq '.job.max_task_seconds'
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' -X PATCH \
  -d '{"max_task_seconds":1800}' "$SITE/api/admin/jobs/$JOB/time-limit"
```

**Export a completed job.** Start it (`202`), poll until `state` is `ready`,
then fetch the presigned `download_url`. A job that is not completed is `409`.

```bash
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{}' "$SITE/api/admin/jobs/$JOB/export"
curl -s -b "$JAR" "$SITE/api/admin/jobs/$JOB/export" | jq '{state, download_url}'
curl -s "<download_url>" | gunzip | head -3
```

**Ban an anonymous worker and lift the ban.** The UUID is in the admin worker
list; the ban's id comes back from the ban.

```bash
curl -s -b "$JAR" "$SITE/api/admin/workers" | jq '.items[:4]'
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"anon_uuid":"<uuid>","reason":"testing"}' "$SITE/api/admin/workers/ban"
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -X DELETE "$SITE/api/admin/workers/ban/<ban id>"
```

---

## What a local instance cannot show

These differ on the real site, and are checked there (README, "Deploying"):

- Mail is really sent, and its links name the real address.
- Pages are HTTPS only: a page asked for over plain HTTP redirects, and the API
  over plain HTTP answers `426` rather than redirecting.
- `/api/dev/login` does not exist: `404`.
- Wordmaps, rack info tables and word info tables are built by a scheduled
  task every five minutes, not within seconds.
- Backups run nightly and **Backups** shows them; alarms mail on failure.
- Contributors connect from other machines.

## Known quirks

Behaviour a tester will meet that is known, so it is not reported as new:

- **Several admin pages show their "nothing here" text while loading**: Fleet,
  Input data, Bans and Derived data.
- **No confirmation** before **Ban worker**, **Insert N rows** (an import), or a
  rating pool's **Add**, **Remove** and **Recompute**.
- **An import in progress lives in one browser.** Another browser does not see
  it, and starting a new import leaves a staged one unreachable until it
  expires after 24 hours. There is no cancel.
- **Users → Delete appears on your own row**; the server refuses it.
- **Signed in, the home page still offers "Create an account"**, and `/login`
  and `/register` do not send you away.
- **A job deleted while its public page is open** stops updating without
  saying why (its admin page does say so).
- **A ban is easy to walk around**: a MAGPIE sending no identity gets a new
  one, and a banned account's owner can contribute anonymously. The Bans page
  says so.
