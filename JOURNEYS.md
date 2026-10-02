# birdtest journeys

Every journey a visitor, a signed-in user and an admin can take on the
birdtest website, written as a checklist for a tester working on a **local
instance started with `scripts/dev.py`**. Working through it should leave you
confident the site can go live: each journey says what to do and what you
should see, including what the site must refuse.

[README.md](README.md) is how to run the site, [PLAN.md](PLAN.md) its design,
and [TESTING.md](TESTING.md) the automated tests, whose end-to-end suite covers
a handful of these journeys (`E-1`..`E-15`). This document is the whole
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
    --leavegen-job --opening-rack-job --games-job --pairs-job
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
| **Player configs** | `static-equity` and `static-score` on CSW24; `ab-static-equity` (the leave job's player) and `ab-static-equity-all` (the opening-rack job's) on CSW21_ab |
| **Jobs**, each active at 25% | "dev games" and "dev game pairs (first divergences saved)", `static-equity` against `static-score` on CSW24; "dev opening racks (english_ab)"; "dev leave generation (english_ab)", six generations |

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
| Games | Two players play each other; an SPRT test decides which is stronger | games |
| Game pairs | The same, in pairs of games with the first move swapped; also feeds ratings | pairs |
| Opening rack analysis | The ranked moves for every opening rack | racks |
| Leave generation | Leave values, built generation by generation from self-play | generations |

---

## Part 1: Visitors and users

Use a signed-out window for the V journeys unless one says otherwise.

### V-1 Find your way around

- [ ] **Expect** a header with **birdtest**, **Jobs**, **Ratings**, **Players**,
  **Contributors** and **Users**, and on the right **Sign in** and **Register**;
  a footer "birdtest — crowdsourced word game analysis".
- [ ] **Do** click each nav link. **Expect** the current one highlighted, and
  no page erroring.
- [ ] **Do** narrow the window to phone width. **Expect** the nav to wrap and no
  page to scroll sideways.
- [ ] **Expect** no **Admin** link unless signed in as an admin.

### V-2 The home page

- [ ] **Expect** "Crowdsourced word game analysis", the buttons **Browse jobs**
  and **Create an account**, and a "Contribute" card with a MAGPIE link and a
  sample `contribute.txt` whose `server` line is this site's own address.
- [ ] **Expect** "Active jobs" listing the seeded jobs still running as cards:
  name, status badge, "25% allocation". Clicking one opens its job page.
- [ ] (With every job deactivated, A-8) **expect** the section gone rather than
  an error.

### V-3 Browse the jobs

- [ ] **Do** open **Jobs**. **Expect** a table, newest first, with Name, Type,
  Status, Allocation ("25%"), Redundancy ("1×") and Progress ("value / max
  unit": games, pairs, racks or generations).
- [ ] **Expect** the progress to have moved when you reload a minute later.
- [ ] A job that workers keep declining, and none has completed in 24 hours,
  shows a red **stalled** pill with a tooltip saying so. (Hard to stage
  locally; note it if you see it.)
- [ ] Signed out, **expect** no **New job** button; as an admin, **expect** one
  beside the title, opening the job form.

### V-4 A job's page: what every job shows

Open "dev game pairs (first divergences saved)".

- [ ] **Expect** the job's name as the title with "Game pairs" beside it, then
  "CSW24 · classic · static, by equity vs static, by score".
- [ ] **Expect** cards for Status (a badge), Allocation, Tasks completed and
  Estimated time left; the redundancy is under **All settings**.
- [ ] **Expect** a Progress card: a bar of "pairs completed", the
  Available / Claimed / Completed task counts, when it was created, "Created
  by dev" and "requires MAGPIE ≥ …".
- [ ] **Expect** a Settings card of two tables, each showing its key rows: the
  job's type "Game pairs", variant, letter distribution, board, bingo bonus
  50, cap 100,000, SPRT "Elo -10 → 10" and records positions "first
  divergences"; then the players side by side, headed "Player 1" and "Player
  2" with each name linking to its config's page and its search under it
  ("static, by equity" / "static, by score"): lexicon, leaves, plies, plays
  considered, sort, recorder and plays recorded, the rows they differ in
  (sort) bold. On "dev games", which records no positions, plays recorded is
  in grey: it never reads it.
- [ ] **Do** press **All settings**. **Expect** both tables to grow: the job's
  adds sim cutoff, fewest pairs before the test is acted on (50,000), α, β,
  pairs per task, redundancy and oldest MAGPIE; the players' table every
  setting after the key rows (no simulation rows: neither simulates). The
  button now reads **Key settings only** and puts them back.
- [ ] At phone width, **expect** the tables to wrap (or scroll inside the
  card), never the page.
- [ ] **Do** **Download every setting as JSON**. **Expect** the same settings,
  with no user ids and no creator.
- [ ] **Expect** a Contributors table: `dev-contributor-1`,
  `dev-contributor-2` and "Anonymous · <16 characters>" rows, with compute time
  (on a wide screen) and tasks completed.
- [ ] **Expect** a link to the results as **paginated JSON**.
- [ ] **Do** open `/jobs/00000000-0000-4000-8000-000000000000`. **Expect** "no
  such job".

### V-5 A job's page updates live

- [ ] **Do** keep the job's page open and do nothing. **Expect** its counts,
  progress bar and win/loss/draw figures to change without reloading.
- [ ] **Do** `docker compose restart backend` and wait. **Expect** the page to
  recover by itself within a minute.

### V-6 Games and game-pairs jobs: the score and the test

On the same job:

- [ ] **Expect**, after Settings, a **Match score** card: player 1's W–L–D
  (e.g. "1,204–1,187–9"), the score out of the games played ("1,208.5 /
  2,400", a draw counting half), win %, the average spread signed ("+3.2"),
  "Average score: static-equity 412.3 · static-score 409.1", a
  win/loss/draw bar chart and "Player 1: W W (x%) · L L (y%) · D D (z%) — per
  game, over both games of every pair". The figures count games, not pairs.
- [ ] **Expect** after it an SPRT card: "running — LLR …, bounds [a, b]. SPRT
  is not acted on until 50,000 pairs are complete", with no W/L/D line of its
  own.
- [ ] **Expect** the pentanomial table in the SPRT card — "P1 lost both",
  "Lost one, drew one", "Split 1-1", "Won one, drew one", "P1 won both" — with
  counts and shares, and a line on how many pairs diverged.
- [ ] On "dev games", **expect** the same score and test cards, no "per game"
  note, and no pentanomial.
- [ ] On a job an admin has deactivated (A-8), **expect** "paused while the job
  is inactive: no pairs are being played, so the test is not moving".
- [ ] On "tester no test" (A-7), **expect** the Match score card and no SPRT
  card, and a "Game pairs" group with "Pairs to play 20" and "SPRT none" in
  place of the test's settings, all settings or not.
- [ ] On a games or pairs job's **Manage** page, **expect** the same Settings,
  Match score and (with a test) SPRT cards after Progress, and no one-line
  "SPRT … LLR" summary in Progress.

### V-7 A finished job says why it finished

A finished job shows a note under its title beginning "Finished <date>:".
Make the jobs in [A-7](#a-7-create-jobs), [A-9](#a-9-finish-purge-and-delete)
and [A-12](#a-12-a-leave-generation-job-start-to-finish), then check:

- [ ] a job stopped at its cap: "it reached its cap of 20 pairs before the
  SPRT decided (LLR …, bounds […])";
- [ ] a job whose test decided: "the SPRT passed (H1 accepted) after N pairs:
  LLR x reached the upper bound y" (or failed, and the lower bound);
- [ ] a job an admin forced: "an admin force-completed it before its test
  decided";
- [ ] a job without a test: "it played the 20 pairs it was set to";
- [ ] a leave-generation job: "its last generation was built".

### V-8 Opening-rack jobs: look up a rack

Open "dev opening racks (english_ab)".

- [ ] **Expect** "Racks analyzed 8 / 8" once it has finished (within minutes),
  a **Look up a rack** box, and under it "Analysed racks to try:" with its
  racks as buttons, each of A and B only.
- [ ] **Do** click one. **Expect** it fills the box and shows its ranked moves
  (#, Move, Score, Equity).
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
  merged in batches. Once done, "Finished …: its last generation was built".
- [ ] **Expect** a Settings card: a "Leave generation" group (generations 6,
  target per rack "100, 200, 500, 1,000, 1,000, 1,000", games per task 1,000;
  racks per task under **All settings**), with no lexicon or wordmap row of
  its own; then one column headed "Player", `ab-static-equity` linking to its
  config: search "static, by equity", lexicon CSW21_ab, wordmap "yes", rack
  info table "no" -- and
  its leaves and recorder in grey, under "Settings in grey are the player
  config's, and this job does not use them." **Do** **All settings**. **Expect**
  win %, plays and plies recorded and the move-gen margin grey too.

### V-10 Saved positions

Open "dev game pairs (first divergences saved)".

- [ ] Signed out, **expect** a "Saved positions" card: "This job keeps the
  turn where each game pair's two games first diverged. Sign in to search
  them."
  **Do** follow **Sign in**, as `dev-contributor-1`. **Expect** to land back on
  this job.
- [ ] Signed in, **expect** one turn of a game pair: "Turn T of a game pair,
  where the players first chose differently", then the pair's two games side
  by side (one above the other on a narrow window), "Game 1 of the pair ·
  PLAYER to move" and "Game 2 of the pair · PLAYER to move" -- `static-equity`
  in one, `static-score` in the other -- each with "rack … · after MOVE
  (score) · N moves ranked by static equity", its board with premium squares
  (TW, DW, TL, DL and a star on the centre), its tiles each with a letter and
  a score (a blank a red lower-case letter with none), the tiles MOVE placed
  outlined, both racks and scores with **to move** on the player whose rack it
  is, the CGP under the board, and the ranked moves below it. The two boards
  and racks are the same, and the moves at the top of the two lists differ.
  (A pairs job saving every turn shows a turn one game never reached alone,
  saying the other game has no position at that turn; a simmer's turns add a
  **Win %** column.)
- [ ] **Do** **Random position** a few times. **Expect** another position
  each time (now and then the same one again), the board, racks and scores agreeing with the CGP; on a turn 1
  position an empty board and nothing outlined, and after an exchange
  (`(exch …)`) or a pass nothing outlined either.
- [ ] **Do** search the rack on the board, typed in lower case and another
  order. **Expect** "Position 1 with the rack …, newest first" and a pair
  whose first game (or, if only the second holds it, whose second) has that
  rack -- a pair whose two games both hold it once, not twice; where there are more, **Next** and **Previous** step through
  them.
- [ ] **Do** search `QQQQQQQ`. **Expect** "No saved position has the rack
  QQQQQQQ."
- [ ] Narrow the window to phone width. **Expect** the board to shrink with
  it, nothing scrolling sideways.
- [ ] On "dev games", **expect** no such section.

### V-11 Ratings

- [ ] Before any pool exists, **expect** "No rating pools yet" and an
  explanation.
- [ ] After [A-13](#a-13-rating-pools), **do** open **Ratings** → the pool.
  **Expect** "classic · english · standard15 — anchored at 1500", "Last fit
  <time> (membership) over N pairs from 1 job, K iterations", and a warning
  if the fit did not converge.
- [ ] **Expect** a dot plot with ±1 standard-error bars and the anchor in
  amber, an "All configs" table (rating, ± SE, pairs; the anchor tagged), and
  "Where the model disagrees with the games".
- [ ] **Expect** the ratings to shift on a reload a few minutes later: the pool
  is refitted every two minutes while its jobs play.
- [ ] Signed out or as a contributor, **expect** no membership controls and no
  **New rating pool** button.

### V-12 Player configs

- [ ] **Do** open **Players**. **Expect** the four seeded configs, newest first:
  name, how it searches ("static, by equity", "static, by score"), lexicon
  (CSW24, or CSW21_ab for the `ab-` ones), leaves, created.
- [ ] **Do** open `static-equity`. **Expect** "static, by equity" beside its
  name, and a table of Lexicon, Leaves, Plies, Plays considered, Sort,
  Recorder and Plays recorded, and a JSON download; a simmer's (A-4) adds Win %
  model, Iterations (most), Stopping % and Inference. **Do** press **All
  settings**. **Expect** every setting in the same table, Lexicon still
  first.
- [ ] **Do** open `/player-configs/00000000-0000-4000-8000-000000000000`.
  **Expect** "no such player config".
- [ ] Signed out, **expect** no **New player config** button on **Players**; as
  an admin, **expect** one beside the title, opening the player-config form.

### V-13 Users and contributors

- [ ] **Do** open **Users**. **Expect** "Registered users": `dev` with an
  **admin** tag and the two contributor accounts, with tasks completed and when
  they joined — and no email addresses anywhere.
- [ ] **Do** open **Contributors**. **Expect** all four workers ranked by
  compute time, most first, the column marked ↓: the two accounts by name, the
  two anonymous ones as "Anonymous · <16 characters>", never as their UUID;
  each with its compute time read as "5h 20m" or "3d 4h" (the exact hours on
  hover), games, racks, tasks and last result.
- [ ] **Do** click **Games**, then **Racks**, then **Tasks**. **Expect** the
  list re-ranked by each, most first, the arrow moving with it; still all four.
- [ ] At phone width, **expect** a "Rank by" row of the four columns above the
  list and only the ranked column beside the name. **Do** choose **Games**.
  **Expect** the games column in its place, and no sideways scroll.

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
  "Anonymous · …" name on **Contributors**.
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

- [ ] **Do** click **Admin**. **Expect** the tabs New job, Player configs, New
  rating pool, Input data, Fleet, Users, Bans, Derived data, Backups and Audit
  log. `/admin` itself goes to the jobs list.
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
  (Started without `--leavegen-job` or `--opening-rack-job`, importing
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

- [ ] **Do** **Player configs** → **New**: `tester-static`, recorder **best**,
  the CSW24 lexicon and leaves, sort **equity**. **Expect** it listed, and on
  the public **Players** page.
- [ ] **Do** a simmer: tick **Simming player**, choose the win% model, 1 ply,
  100 iterations. **Expect** it saved and described "1-ply sim, 100
  iterations".
- [ ] **Do** tick **Simming player** without a win% model. **Expect** the
  browser to insist on one.
- [ ] **Do** set Stopping % to 100, then 0. **Expect** the browser to hold the
  form: "Stopping % must be above 0 and below 100."
- [ ] **Do** **Show advanced options**. **Expect** **Use wordmap** and **Use
  rack info table** ticked, with a note that a table costs a contributor about
  1.9 GB, and **Use word info table** unticked; ticking it shows a note that
  the server builds it once per lexicon.
- [ ] **Do** pair the CSW24 lexicon with the `FRA20` leaves. **Expect** "leaves
  … are not compatible with lexicon …".
- [ ] **Do** look at **Endgame and pre-endgame**. **Expect** **Solve the
  pre-endgame** greyed out until **Solve the endgame** is ticked, and unticked
  again when the endgame is.
- [ ] **Do** `tester-solver`: static, tick **Solve the endgame** (depth 6) and
  **Solve the pre-endgame** (bag 2). **Expect** it saved and described
  "static, by equity · 6-ply endgame · PEG ≤2"; on its page, with **All
  settings**, Endgame "6-ply endgame", Pre-endgame "bag ≤ 2" and the schedule
  MAGPIE defaults to (32, 16, 8, 4, 2; nested caps 8, 4, 2; strides 1, 1, 5,
  7).
- [ ] **Do** **Show pre-endgame schedule** and type `4, x` as stage survivors.
  **Expect** "Stage survivors: "x" is not a whole number." and nothing saved.
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
- [ ] **Do** make a player config with **Use word info table** ticked, and a
  games job with it. **Expect** a "Word info table" row named for the lexicon
  on **Derived data** and on the job's page, built within seconds, and the
  config's page to say "Word info table: yes" under **All settings**.

### A-7 Create jobs

**New job**, then **Create job**; each lands on the job's admin page,
**inactive**, with no allocation. The letter distribution and board layout
start on "Choose…": pick `english` and `standard15` each time.

- [ ] **Game pairs** "tester no test": `static-equity` vs `static-score`, 10
  pairs per batch. **Expect** **Run an SPRT** unticked, no α, β, Elo or "Min
  before SPRT" fields, and the target labelled "Pairs to play": set it to 20.
  It finishes once it has played them (V-7).
- [ ] **Game pairs** "tester cap": the same, with **Run an SPRT** ticked:
  **expect** the SPRT fields to appear and the target to read "Hard cap". Min
  before SPRT 20, hard cap 20. It finishes at its cap (V-7).
- [ ] **Game pairs** "tester decided": `static-equity` vs `static-score`, 10 per
  batch, **Run an SPRT** ticked, min before SPRT 0, cap 5,000, Elo low 0, Elo
  high 50. Its test should decide once it runs (V-7); if it has not after an
  hour, note how far the LLR got.
- [ ] **Games** "tester positions": the same two players, 2 per batch, 20 games
  to play,
  **Save the positions played** ticked. **Expect** the batch's maximum to drop
  to 1,000, and, once it runs, a saved-positions section on its page (V-10),
  and no **Only where each pair first diverges** option: a games job has no
  pairs.
- [ ] **Game pairs** "tester divergences": `static-equity` vs `static-score`, 5
  pairs per batch, 50 pairs to play, **Save the positions played** and then
  **Only where each pair first diverges** ticked (it appears only once saving is).
  **Expect** its settings to say "Records positions: first divergences", and
  on its page each saved position to be a pair's two games at one turn, the
  same board and rack, each player to move in one, their ranked moves
  differing at the top; **Random position** never shows a lone game.
- [ ] **Games**: **expect** the batch field to step by 2, and the browser to
  refuse an odd number.
- [ ] **Opening rack analysis** with `static-equity` (recorder best, 10 plays
  kept). **Expect** a warning that only one play per rack would be stored, and
  creation refused; with `static-equity-all` it is accepted.
- [ ] **Do** leave the letter distribution on "Choose…" and submit. **Expect**
  the browser to hold the form on that field.
- [ ] **Do** clear a number box and submit. **Expect** "Fill in every setting: …
  is empty."
- [ ] **Do** set Elo high below Elo low. **Expect** "must be a finite number
  greater than elo_low".

### A-8 Activate, share out and deactivate

The seeded games and pairs jobs hold 50% between them once the two english_ab
jobs have finished (100% while those run).

- [ ] **Do** activate "tester cap" at 60%. **Expect** "the other active jobs
  already allocate 50% — 50% is the most this job can take".
- [ ] **Do** deactivate "dev games" (its **Manage** page → **Deactivate**).
  **Expect** "Job deactivated." and **inactive** in the Status card, the same
  four cards as the public page's; its public page no longer says "live".
- [ ] **Do** activate "tester cap" at 25%. **Expect** "Job activated.", "Now:
  25%", and the workers taking its tasks.
- [ ] **Do** type 150, then 2.5, and **Activate**. **Expect** "Enter a
  whole-number allocation from 0 to 100."
- [ ] **Do** change an active job's share and **Activate** again. **Expect** the
  new share taken.
- [ ] **Do** deactivate every job but one, at 25%. **Expect** that job to get
  all the work: shares weigh only against the other active jobs.
- [ ] **Do** create a game-pairs job with **Min MAGPIE version** `9.9.9`, and
  make it the only active one. **Expect** every worker told its MAGPIE is too
  old and stopping. Deactivate it and restart the workers.

### A-9 Finish, purge and delete

- [ ] **Do** let "tester cap" run. **Expect** it completed within a few
  minutes, with the cap note (V-7), and Activate, Deactivate and Force
  complete disabled.
- [ ] **Do** activate "tester decided". **Expect** a decided test (V-7).
- [ ] **Do** **Force complete** a running job and confirm. **Expect** "Job
  force-completed." and "an admin force-completed it before its test
  decided".
- [ ] **Do** **Purge results** on a completed job and confirm. **Expect**
  "Results purged. The job is inactive: activate it to start over from its
  first task." and its counts at zero. On an active job: "Results purged; the
  job starts over from its first task."
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

- [ ] **Do** **Player configs** → **New**: `tester-ab`, recorder **best**, the
  `CSW21_ab` lexicon and leaves, sort **equity**, static.
- [ ] **Do** **New job**: Leave generation, "tester leaves", letter distribution
  `english_ab`, board `standard15`, player config `ab-static-equity-all`.
  **Expect** a red "ab-static-equity-all asks for a rack info table; leave
  generation plays statically on equity, without a rack info table or endgame
  solving." (Submitted anyway, the server refuses it the same way.) **Do** pick `tester-ab`, games per task 1,000, racks per task 50,
  occurrences per rack per generation "100, 200, 300". **Expect** "3
  generations." under the targets, and Redundancy greyed out: leave generation
  runs at redundancy 1. (**Do** type "100, 0" there first. **Expect** a red
  "Every target must be between 1 and 1,000,000, not 0.")
- [ ] **Do** make room (A-8) and activate it at 25%. **Expect** its wordmap built
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
  rewritten. (On an active job it is refused: "deactivate the job before
  forcing a rebuild …".)
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
  **Contributors** and job pages.
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
  `job.purged`, `job.deleted`, `job.export_started`,
  `input_data.import_staged`, `input_data.import_confirmed`,
  `input_data.import_nothing_new`,
  `rating_pool.created`, `rating_pool.member_added`, `user.deleted`,
  `worker.banned`, `worker.unbanned` and more.
- [ ] **Do** filter by action `job.activated`, then by target type `job`.
  **Expect** only those, and paging to keep the filter.

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
curl -s -o /dev/null -w '%{http_code}\n' "$SITE/api/jobs/$JOB/positions/random"  # 401: signed-in users only
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

**A job's saved positions** (the seeded "(positions saved)" job): one at
random, or those with one rack, up to 20 a page, newest first;
`next_cursor`, when present, fetches the next page (`&cursor=…`). The board
they are drawn on is public.

```bash
curl -s -b "$JAR" "$SITE/api/jobs/$JOB/positions/random" \
  | jq '{game_index, turn_number, rack, position, previous_move, best: .moves[0].move}'
curl -s -b "$JAR" "$SITE/api/jobs/$JOB/positions?rack=aeinrst&per_page=5" \
  | jq '.items[] | {game_index, turn_number, rack, best: .moves[0].move}'
curl -s "$SITE/api/jobs/$JOB/board" | jq '{start, first_row: .squares[0], letters: .letters[:3]}'
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

**Create a job, activate it, deactivate it.** Created inactive (`201`);
activation sets the share (`200`), and a share that would take the active
jobs past 100% is `409` — with the seed's games and pairs jobs active, 50% is
the most left. This one runs no SPRT and plays its 20 pairs; `"sprt_enabled": true`
turns the test on, with `min_pairs` then required, and `min_pairs`, α, β or
an Elo bound sent without it is a `400` naming each.

```bash
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"name":"cli test","job_type":"game_pairs","variant":"classic",
       "letterdist_id":"<letterdist id>","layout_id":"<layout id>",
       "player1_config_id":"<config id>","player2_config_id":"<config id>",
       "pairs_per_batch":10,"max_pairs":20}' \
  "$SITE/api/admin/jobs" | jq '.job.id'
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"allocation":4}' "$SITE/api/admin/jobs/$JOB/activate"
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -X POST "$SITE/api/admin/jobs/$JOB/deactivate"
```

**Operational health**: the fleet, the derived-file queue, the backups, and
the audit log filtered to one action.

```bash
curl -s -b "$JAR" "$SITE/api/admin/fleet"
curl -s -b "$JAR" "$SITE/api/admin/derived-data" | jq '.[] | {role, name, state}'
curl -s -b "$JAR" "$SITE/api/admin/backups"
curl -s -b "$JAR" "$SITE/api/admin/audit-log?action=job.activated&per_page=5"
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
