# birdtest journeys

Every journey a visitor, a signed-in user and an admin can take on the
birdtest website, written as a checklist for a tester. Working through it
end to end should leave you confident the site can go live: each journey says
what to do and what you should see, including what the site must refuse.

[PLAN.md](PLAN.md) is the design, [README.md](README.md) how to run and deploy
the site, and [TESTING.md](TESTING.md) the automated tests. The automated
end-to-end suite covers a handful of these journeys (`E-1`..`E-14`); this
document is the whole surface, by hand.

## How to use this document

- Journeys are numbered (`V-3`, `U-7`, `A-12`) so a result can be reported
  against one. Each has checkboxes: tick what behaved as described, and note
  anything that did not, with the URL, the account, and what you saw.
- **Do** lines are actions; **Expect** lines are what must happen. Text in
  quotes is what the page should say, word for word or close to it.
- Test on a staging copy of the production stack if you can, then repeat the
  short [Before going live](#before-going-live) list on production itself.
- Try at least one pass at phone width (about 390 px wide): every page should
  be usable without scrolling sideways, and the nav wraps onto several lines.

### What you need

| | Why |
|---|---|
| Two browsers, or one normal and one private window | To be a visitor and a signed-in user at once |
| Three email addresses you can read | Two ordinary accounts (**alice**, **bob**) and an admin (**admin**) |
| Access to promote the first admin | In production, `scripts/prod-sql.sh`; locally, `psql` (README, "Doing it by hand") |
| A machine that can build and run MAGPIE | Contributions are what move jobs; see README, "Contributing with MAGPIE" |
| A MAGPIE-DATA tarball date | For the import, e.g. the `DATA_VERSION` in MAGPIE's `download_data.sh` |

A fresh deployment has no accounts, no data and no jobs. The admin journeys
build everything the visitor and user journeys need, so on a fresh stack do
[A-1](#a-1-become-the-first-admin) to [A-7](#a-7-create-and-run-a-job-of-every-type)
first. Locally, `./scripts/dev.py --reset-db` seeds a full set instead (six
jobs, three players, two contributor accounts), and `./scripts/dev.py --fresh`
starts empty, as production does.

Job types, as the site names them:

| Type | What it computes | Progress is counted in |
|---|---|---|
| Games | Two players play each other; an SPRT test decides which is stronger | games |
| Game pairs | The same, in pairs of games with the first move swapped; also feeds ratings | pairs |
| Opening rack analysis | The ranked moves for every opening rack | racks |
| Leave generation | Leave values, built generation by generation from self-play | generations |

---

## Part 1: Visitors and users

### V-1 Find your way around

- [ ] **Do** open the site signed out. **Expect** a header with **birdtest**,
  **Jobs**, **Ratings**, **Players**, **Contributors**, **Users**, and on the
  right **Sign in** and **Register**; a footer "birdtest — crowdsourced word
  game analysis".
- [ ] **Do** click each nav link. **Expect** the current one highlighted, and
  no page erroring.
- [ ] **Do** narrow the window to phone width. **Expect** the nav wraps and no
  page scrolls sideways.
- [ ] **Expect** no **Admin** link for a visitor or an ordinary user.

### V-2 The home page

- [ ] **Expect** "Crowdsourced word game analysis", the buttons **Browse jobs**
  and **Create an account**, and a "Contribute" card with a MAGPIE link and a
  sample `contribute.txt` whose `server` line is this site's own address.
- [ ] **Expect** an "Active jobs" section listing each active job as a card:
  its name (or its type when it has none), a status badge, and its allocation
  ("N% allocation"). Clicking a card opens its job page.
- [ ] With no active jobs, **expect** the section to be absent (not an error).

### V-3 Browse the jobs

- [ ] **Do** open **Jobs**. **Expect** a table, newest first, with Name, Type,
  Status, Allocation ("N%" or "—"), Redundancy ("N×") and Progress
  ("value / max unit": games, pairs, racks or generations).
- [ ] **Expect** a red **stalled** pill on a job that workers keep declining
  and none has completed in 24 hours, with a tooltip saying so. (Hard to stage;
  note if you see it.)
- [ ] With more than 50 jobs, **expect** **Previous** / **Next** and "Page N of
  M"; Previous is disabled on the first page.
- [ ] With no jobs, **expect** "No jobs yet."

### V-4 A job's page: what every job shows

Open any job from the list.

- [ ] **Expect** the job's name as the title (its type label beside it), a
  status badge, then "lexicon · variant · how each player searches", e.g.
  "static, by equity vs 1-ply sim, 100 iterations".
- [ ] **Expect** cards for Allocation, Redundancy, Results accepted and
  Estimated time left ("—" when unknown).
- [ ] **Expect** a Progress card with a bar labelled for the job type (see the
  table above), the Available / Claimed / Completed task counts, when it was
  created, "Created by …" and, when set, "requires MAGPIE ≥ …".
- [ ] **Expect** a Settings card: one line per player — its name, which links
  to the player config's page, and how it searches.
- [ ] **Do** open **All settings**. **Expect** a "Job" group (variant, letter
  distribution, board, bingo bonus, sim cutoff, redundancy, oldest MAGPIE), a
  group for the job's type, and a table of every player setting side by side,
  with those the players differ in shown in bold.
- [ ] **Do** click **Download every setting as JSON**. **Expect** a JSON file
  with the same settings, and no user ids or creator in it.
- [ ] **Expect** a Contributors table: username, or "Anonymous · " and a
  16-character pseudonym, with tasks completed; "No contributions yet." when
  empty.
- [ ] **Expect** a link to the results as **paginated JSON**.
- [ ] **Do** open `/jobs/<a made-up id>`. **Expect** "no such job".

### V-5 A job's page updates live

- [ ] **Do** keep an active job's page open while a MAGPIE contributor works on
  it. **Expect** the counts, progress bar and (for games) the win/loss/draw
  figures to change without reloading.
- [ ] **Do** stop the backend briefly (staging only) and start it again.
  **Expect** the page to recover by itself within a minute.

### V-6 Games and game-pairs jobs: the test

- [ ] **Expect** an SPRT card with a badge and a sentence saying where the
  test stands:
  - an active job below its minimum: "SPRT is not acted on until N pairs are
    complete.";
  - an active job past it: "The minimum of N pairs is reached; SPRT is
    checked as pairs arrive.";
  - an inactive job: "paused while the job is inactive: no pairs are being
    played, so the test is not moving";
  - a decided job: "Completed: passed (H1 accepted)" or "failed (H0
    accepted)", with its LLR and bounds.
- [ ] **Expect** a win/loss/draw bar chart and "Player 1: W W (x%) · L L (y%) ·
  D D (z%)".
- [ ] For a **game-pairs** job, **expect** the pentanomial table — "P1 lost
  both", "Lost one, drew one", "Split 1-1", "Won one, drew one", "P1 won both"
  — with counts and shares, and a line on how many pairs diverged.

### V-7 A finished job says why it finished

For each way a job can finish, **expect** a note under the title beginning
"Finished <date>:":

- [ ] the test passed or failed: "the SPRT passed (H1 accepted) after N pairs:
  LLR x reached the upper bound y" (or failed / lower bound);
- [ ] it hit its cap: "it reached its cap of N … before the SPRT decided";
- [ ] an admin forced it (A-9): "an admin force-completed it before its test
  decided";
- [ ] an opening-rack job: "every rack was analysed";
- [ ] a leave-generation job: "its last generation was built".

### V-8 Opening-rack jobs: look up a rack

- [ ] **Expect** "Racks analyzed N / total", a **Look up a rack** box, and
  under it "Analysed racks to try:" with up to ten racks as buttons.
- [ ] **Do** click one. **Expect** it fills the box and shows its ranked moves
  (#, Move, Score, Equity).
- [ ] **Do** type a rack in lower case and in another order (e.g. `tsrniea`).
  **Expect** the same result as the sorted rack.
- [ ] **Do** search a rack not analysed yet. **Expect** "No analysis stored for
  that rack yet."

### V-9 Leave-generation jobs

- [ ] **Expect** "Generation N of M — target T occurrences per rack", a line of
  tasks and games played this generation ("— live." while active), a bar of
  racks at target, and "Fewest occurrences so far: …" with when the figures
  were last merged.

### V-10 Saved positions (games and pairs jobs that save them)

Use a job created with positions saved (the seeded "static equity vs 1-ply sim
(positions saved)" locally; see [Known quirks](#known-quirks) for how to make
one in production).

- [ ] Signed out, **expect** a "Saved positions" card saying "Sign in to search
  them." **Do** follow the link and sign in. **Expect** to land back on the
  job's page.
- [ ] Signed in, **expect** the ten newest positions, each "Game G, turn T ·
  rack … · after MOVE (score) · N moves ranked", its board as a CGP string,
  and a table of ranked moves (with **Win %** when a simming player made it).
- [ ] **Do** click **Load more**. **Expect** ten more, until it disappears.
- [ ] **Do** search a rack shown on the page, typed in any order and case.
  **Expect** only positions with that rack, and a **Show all** button that
  clears the search.
- [ ] **Do** search a rack no position has. **Expect** "No saved position has
  the rack …".
- [ ] On a job that does not save positions, **expect** no such section.

### V-11 Ratings

- [ ] **Do** open **Ratings**. **Expect** a table of pools — name,
  "variant · letter distribution · board", member count, last computed (or
  "never") — or "No rating pools yet" with an explanation.
- [ ] **Do** open a pool. **Expect** "anchored at N", a line such as "Last fit
  <time> (membership) over N pairs from M jobs, K iterations" (or "Never
  computed."), and a warning if the fit did not converge.
- [ ] **Expect** a dot plot of ratings with ±1 standard-error bars, the anchor
  in amber, and configs with no chain of games to the anchor listed as
  "Unrated".
- [ ] **Expect** an "All configs" table (rating, ± SE, pairs; the anchor
  tagged) and a "Where the model disagrees with the games" table, or "No
  head-to-head results in this pool yet."
- [ ] **Expect** no rating-history chart, and no membership controls unless you
  are an admin.

### V-12 Player configs

- [ ] **Do** open **Players**. **Expect** every player config, newest first:
  name, how it searches, lexicon, leaves, created.
- [ ] **Do** open one. **Expect** its key settings — Search, Lexicon, Leaves,
  Win % (simmers only), Plays considered, Recorder ("best, 10 plays kept"),
  Wordmap, Rack info table — and every setting behind **All settings**, with a
  JSON download.
- [ ] For a config cloned onto newer data, **expect** "cloned from <link>" and
  that the link opens the original.
- [ ] **Do** open `/player-configs/<a made-up id>`. **Expect** "no such player
  config".

### V-13 Users and contributors

- [ ] **Do** open **Users**. **Expect** "Registered users": username (an
  **admin** tag on admins), joined, tasks completed, most tasks first, 50 a
  page. **Expect** no email addresses anywhere.
- [ ] **Do** open **Contributors**. **Expect** every worker that has completed a
  task, accounts and anonymous ones together, ranked, with its last result.
  Anonymous workers appear as "Anonymous · <pseudonym>", never as their UUID.

### U-1 Register

- [ ] **Do** open **Register** and submit with a username of 2 characters.
  **Expect** "must be between 3 and 32 characters".
- [ ] **Do** try an invalid email (`alice@`, `a..b@example.org`). **Expect**
  "must be a valid email address".
- [ ] **Do** try a weak password (`password123`). **Expect** "too weak — choose
  a longer, less predictable password". The strength hint under the field is
  only a guide; the server decides.
- [ ] **Do** register **alice** properly. **Expect** "Check your email".
- [ ] **Do** register again with alice's username. **Expect** "that username is
  taken" (also for a different case, e.g. `ALICE`).
- [ ] **Do** register a new username with alice's email. **Expect** the same
  "Check your email" page — the site does not reveal the address is in use —
  and alice's inbox to get a notice that an account already exists.
- [ ] **Do** register more than 10 times in an hour from one address.
  **Expect** "too many requests".

### U-2 Confirm the email address

- [ ] **Do** try to sign in before confirming. **Expect** "confirm your email
  address before signing in — check your inbox".
- [ ] **Do** open the link in the email. **Expect** "Email confirmed", then the
  sign-in page.
- [ ] **Do** open the same link again. **Expect** it still says confirmed.
- [ ] **Do** open `/confirm-email` with no code, and with a made-up code.
  **Expect** "That link is missing its confirmation code." and "that
  confirmation link is invalid or has expired" respectively.
- [ ] (Optional) a link older than 24 hours is refused as expired.

### U-3 Sign in and out

- [ ] **Do** sign in with a wrong password, and with a username that does not
  exist. **Expect** the same "incorrect username or password" for both.
- [ ] **Do** sign in as alice (the username in any case). **Expect** to land on
  **Account**, with alice's name in the header.
- [ ] **Do** open a page that needs sign-in while signed out, e.g. `/account`.
  **Expect** the sign-in page, and after signing in, to be sent back there.
- [ ] **Do** sign in with `?next=//evil.example` in the URL. **Expect** to land
  on **Account**, not another site.
- [ ] **Do** fail sign-in more than 10 times in a minute. **Expect** "too many
  requests".
- [ ] **Do** click **Sign out**. **Expect** the home page, **Sign in** back in
  the header, and `/account` sending you to sign in again.

### U-4 Reset a forgotten password

- [ ] **Do** click **Forgot password?** and enter alice's address. **Expect**
  "If that address has a confirmed account, a reset link is on its way. The
  link expires in 30 minutes." and an email naming alice's username.
- [ ] **Do** the same with an address that has no account. **Expect** the same
  message, and no email.
- [ ] **Do** open the link and set a weak password. **Expect** "too weak — …"
  and the link still usable. **Then** set a strong one. **Expect** the sign-in
  page; the old password no longer works and the new one does.
- [ ] **Expect** alice signed out in every other browser after the reset.
- [ ] **Do** reuse the link. **Expect** "that reset link is invalid or has
  expired".
- [ ] **Do** request more than 5 resets in an hour. **Expect** "too many
  requests".

### U-5 Your account

- [ ] **Do** open **Account** (click your username). **Expect** username,
  email, role ("contributor", or "admin") and tasks completed.
- [ ] Signed out, **expect** `/account` to send you to sign in.

### U-6 API keys

- [ ] **Do** click **Generate key** with a label. **Expect** a warning "Copy
  this key now — it is not shown again." with the key and its line for
  `contribute.txt` (`apikey …`), and the key listed with its label, created
  date, "Last used —" and "active".
- [ ] **Do** reload the page. **Expect** the key itself is gone; only its row
  remains.
- [ ] **Do** generate a key with no label. **Expect** "—" as its label.
- [ ] **Do** run MAGPIE with the key (U-8). **Expect** "Last used" to fill in.
- [ ] **Do** **Deactivate** it. **Expect** "inactive", and MAGPIE using it to be
  refused. **Do** **Activate** it. **Expect** it to work again.
- [ ] **Do** **Revoke** it and confirm. **Expect** it gone from the list and
  MAGPIE refused with it.
- [ ] A label longer than 100 characters, **expect** "the key's label is too
  long".

### U-7 Sign out everywhere

- [ ] **Do** sign in as alice in two browsers. In one, **Account** → **Sign out
  everywhere** → confirm. **Expect** that browser on the sign-in page, and the
  other signed out at its next page load.

### U-8 Contribute with MAGPIE

Follow README, "Contributing with MAGPIE", against the site under test.

- [ ] **Do** run `magpie contribute` with no API key. **Expect** it to claim
  and complete tasks, the job's counts to rise (V-5), and an "Anonymous · …"
  row in the job's contributors and on **Contributors**.
- [ ] **Do** run it with alice's key. **Expect** alice's username in those
  places instead, and alice's tasks completed to rise on **Account** and
  **Users**.
- [ ] **Do** run it against a job whose players use a wordmap and a rack info
  table. **Expect** the first task to take a few minutes longer while MAGPIE
  builds them, and then to run normally.
- [ ] **Do** run an old MAGPIE, below the version floor, if one is to hand.
  **Expect** it told to update rather than being given work.

### U-9 What a signed-in user may not do

- [ ] As alice, **do** open `/admin/jobs/new` (or any `/admin` page). **Expect**
  to be sent to the home page.
- [ ] As alice, **expect** no **Manage** button on job pages, no **New rating
  pool** button on Ratings, and no membership controls on a pool.

---

## Part 2: Admins

### A-1 Become the first admin

- [ ] **Do** register and confirm **admin** like any user (U-1, U-2).
- [ ] **Do** promote it: in production
  `scripts/prod-sql.sh "UPDATE users SET is_admin = true WHERE lower(username) = lower('admin') RETURNING username"`
  (locally, the `psql` command in README, "Doing it by hand"). There is no page
  or endpoint for this, by design.
- [ ] **Do** reload. **Expect** an **Admin** link in the header, and **Account**
  showing "admin". No new sign-in is needed.

### A-2 The admin area

- [ ] **Do** click **Admin**. **Expect** the tabs New job, Player configs, New
  rating pool, Input data, Fleet, Users, Bans, Derived data, Backups, Audit
  log. `/admin` itself goes to the jobs list.
- [ ] Signed out, **expect** any `/admin` page to send you to sign in and back.

### A-3 Import data

- [ ] **Do** open **Input data**. On a fresh stack, **expect** "Nothing
  imported yet."
- [ ] **Do** enter a version that does not exist (e.g. `19990101`) and **Fetch
  and diff**. **Expect** "Import failed: no data-19990101.tgz …".
- [ ] **Do** enter `2026` (not eight digits). **Expect** "tarball_date must be
  YYYYMMDD". **Do** enter a branch of `../x`. **Expect** "that is not a git ref
  name …".
- [ ] **Do** enter the real version and branch `main`. **Expect**
  "Downloading… X MiB, N files hashed." updating every second.
- [ ] **Do** leave the page mid-download and come back (or reload). **Expect**
  the import picked up where it is.
- [ ] **Expect**, when it finishes, "N new, N changed, N already known." and a
  table of the files to insert. **Do** **Insert N rows**. **Expect**
  "Confirmed. N rows inserted." and the files listed below with their role,
  name, version, digest and size.
- [ ] **Do** import the same version again. **Expect** everything "already
  known", and **Insert 0 rows**.
- [ ] **Do** click **Insert** twice quickly, or in two tabs. **Expect** the
  second refused: "this import is confirmed, not staged".
- [ ] **Expect** the **Delete** button disabled on every file a job, player
  config or rating pool uses ("Pinned by" above 0). **Do** delete an unpinned
  file and confirm. **Expect** it gone.

### A-4 Create player configs

- [ ] **Do** **Player configs** → **New**, and create a static player: a name,
  recorder **best**, the imported lexicon and leaves, sort **equity**.
  **Expect** it in the list, and on the public **Players** page.
- [ ] **Do** create a second static player sorting by **score**.
- [ ] **Do** create a simming player: tick **Simming player**, choose a win%
  model, 1 ply, 100 iterations. **Expect** it saved; its public page says
  "1-ply sim, 100 iterations".
- [ ] **Do** tick **Simming player** without choosing a win% model. **Expect**
  the browser to insist on one.
- [ ] **Do** open **Show advanced options**. **Expect** **Use wordmap** and
  **Use rack info table** both ticked by default, with a note that a table is
  built before a job using it can run and costs a contributor about 1.9 GB.
- [ ] **Do** save a config with leaves from a different lexicon family.
  **Expect** "leaves … are not compatible with lexicon …".
- [ ] **Do** set Stopping % to 100 on a simmer. **Expect** it refused (see
  [Known quirks](#known-quirks)).
- [ ] **Expect** no way to edit a config: they never change once made.

### A-5 Delete a player config

- [ ] **Do** delete a config no job or pool uses, and confirm. **Expect** it
  gone.
- [ ] **Do** delete one a job uses. **Expect** "a job, a rating pool, a rating
  history or a clone references this player config".

### A-6 Wordmaps and rack info tables

- [ ] After creating a job whose players use them (A-7), **do** open **Derived
  data**. **Expect** a wordmap and a rack info table "pending" or "building",
  and a banner "N files are still being built. Jobs that need them are
  waiting."
- [ ] **Expect** within about ten minutes (the builder runs every five) both
  "built", with size and SHA-256, and the job starting to hand out work.
- [ ] If a build fails three times, **expect** it red with its error, a banner
  saying builds have given up, and a **Retry** button that queues it again.

### A-7 Create and run a job of every type

For each type, **do** **New job**, fill it in, **Create job**. **Expect** to
land on the job's admin page, the job **inactive** with no allocation.

- [ ] **Game pairs**: two different players, 10 pairs per batch, minimum 100,
  cap 1,000.
- [ ] **Games**: **expect** the batch to be even — the field steps by 2, and the
  browser refuses an odd number — so each player moves first in half of every
  task's games.
- [ ] **Opening rack analysis**: a static player with recorder **best** that
  keeps more than one play. **Expect** a warning under the player that only
  one play per rack would be stored, and creation refused; choose a player
  with recorder **all** instead.
- [ ] **Leave generation**: a lexicon, 1 generation. **Expect** "Redundancy"
  greyed out: leave generation runs at redundancy 1.
- [ ] **Do** clear a number box and submit. **Expect** "Fill in every setting:
  … is empty."
- [ ] **Do** set Elo high below Elo low. **Expect** "must be a finite number
  greater than elo_low".
- [ ] **Do** create a job without a name. **Expect** the browser to ask for
  one.

### A-8 Activate, share out and deactivate

On a job's admin page (**Manage** from its public page):

- [ ] **Do** **Activate** at 30%. **Expect** "Job activated.", status
  **active**, "Now: 30%", and workers starting on it.
- [ ] **Do** activate other jobs until the total would pass 100%. **Expect**
  "the other active jobs already allocate N% — M% is the most this job can
  take".
- [ ] **Do** change an active job's allocation and **Activate** again.
  **Expect** the new share taken.
- [ ] **Do** type 150 or 2.5. **Expect** "Enter a whole-number allocation from 0
  to 100."
- [ ] **Do** leave only one job active, at any share below 100%. **Expect** it
  to get all the work: shares are weighed only against the other active jobs.
- [ ] **Do** **Deactivate**. **Expect** "Job deactivated.", status **inactive**,
  the job offered to no worker, and its SPRT card saying "paused".

### A-9 Finish, purge and delete

- [ ] **Do** **Force complete** a job and confirm. **Expect** "Job
  force-completed.", the note from V-7, and Activate, Deactivate and Force
  complete now disabled.
- [ ] **Do** **Purge results** on a completed job and confirm. **Expect**
  "Results purged. The job is inactive: activate it to start over from its
  first task.", and every count back at zero.
- [ ] **Do** purge an active job. **Expect** "Results purged; the job starts
  over from its first task."
- [ ] **Do** **Delete job** and confirm. **Expect** to land on the jobs list,
  without it.
- [ ] **Do** keep a job's admin page open in one tab and delete the job in
  another. **Expect** the first to say "This job no longer exists." with every
  action disabled.

### A-10 Export a finished job

- [ ] **Do** on a completed job, **Export results**. **Expect** "Building…",
  then "N rows · N MB · download · SHA-256 of the .gz …" and links valid for an
  hour. For a job that saved positions, a second download for them.
- [ ] **Do** download. **Expect** a gzipped file of one JSON object per line.
- [ ] **Expect** no export card on an active or inactive job.

### A-11 Leave-generation tools

On a leave-generation job's admin page:

- [ ] **Do** **Merge progress now**. **Expect** "Merged N staged results into M
  racks."
- [ ] **Do** **Check artifacts**. **Expect** "Checked N generations: 0
  rewritten, 0 differing from the recorded hash." and a table, one row a
  generation.
- [ ] **Do** **Force rebuild** while the job is active. **Expect** "deactivate
  the job before forcing a rebuild …". Deactivated, **expect** it to rewrite
  every generation.

### A-12 Data gaps

- [ ] **Do** run a MAGPIE whose data lacks a file a job needs (e.g. an older
  data install). **Expect** it to decline the job, and the job's admin page to
  list the file under "Data gaps" with the number of workers and declines.
  With none, "No worker has declined this job."

### A-13 Rating pools

- [ ] **Do** **New rating pool**: a name, the variant, letter distribution and
  board of the game-pairs jobs, an anchor, rating 1500, and the other players
  ticked. **Expect** the pool's page, fitted, with every player rated.
- [ ] **Do** create one with nothing ticked. **Expect** "Never computed." until
  a member is added.
- [ ] **Do** **Add a player config**. **Expect** it rated and every other
  rating moved: a new member's games are evidence for everyone.
- [ ] **Do** **Remove** a member. **Expect** its row gone and the others
  refitted. **Expect** no Remove button on the anchor.
- [ ] **Do** **Recompute**. **Expect** "Last fit … (manual)".
- [ ] **Expect** a pool's ratings to change on their own every few minutes
  while its game-pairs jobs are playing.

### A-14 Delete a user

- [ ] **Do** **Users** → **Delete** on bob, and confirm. **Expect** bob gone
  from the list, bob signed out, bob unable to sign in, and bob's API keys
  refused.
- [ ] **Expect** bob's results still counted, shown as `deleted-<id>` on
  **Contributors** and on job pages.
- [ ] **Do** delete your own account. **Expect** "you cannot delete your own
  account".

### A-15 Ban a worker

- [ ] **Do** **Bans**, **Select** an anonymous worker from "Known workers",
  give a reason, **Ban worker**. **Expect** "Banned …" and the ban listed.
- [ ] **Expect** that worker's MAGPIE refused its next claim: "this worker
  identity is banned".
- [ ] **Do** ban an account by user id. **Expect** MAGPIE running with that
  account's keys refused, while the person can still use the website.
- [ ] **Do** ban the same identity twice. **Expect** "that already exists".
- [ ] **Do** **Unban** and confirm. **Expect** "Lifted the ban on …" and the
  worker accepted again.

### A-16 Fleet, backups and the audit log

- [ ] **Do** open **Fleet**. **Expect** each MAGPIE version that claimed work
  in the last seven days, with its workers and claims.
- [ ] **Do** open **Backups**. On a fresh stack, **expect** "No successful
  backup has ever been recorded." in red. After the nightly backup has run,
  **expect** "Last successful backup N ago" and the run listed as "ok".
- [ ] **Do** open **Audit log**. **Expect** every admin action above, newest
  first: `job.created`, `job.activated`, `job.deactivated`, `job.completed`,
  `job.purged`, `job.deleted`, `job.export_started`,
  `input_data.import_staged`, `input_data.import_confirmed`,
  `rating_pool.created`, `rating_pool.member_added`, `user.deleted`,
  `worker.banned`, `worker.unbanned` and more.
- [ ] **Do** filter by action `job.activated`, then by target type `job`.
  **Expect** only those entries, and paging to keep the filter.

### A-17 One job, start to finish

A single run through everything a real job goes through:

- [ ] Import data (A-3) → create two players (A-4) → create a game-pairs job
  (A-7) → wait for its wordmap and table (A-6) → activate it (A-8) → run two
  MAGPIE contributors, one anonymous and one with an API key (U-8) → watch it
  live (V-5) → let the test decide or the cap end it (V-7) → export it (A-10)
  → rate its players (A-13) → check the audit log (A-16).

---

## Part 3: From the command line

The site is a single-page app over a JSON API, and everything the pages do can
be done with `curl`. These are a handful of examples — not the whole API,
which is in [PLAN.md](PLAN.md)'s API tables — each with what to expect, so a
run doubles as a check.

Set up once:

```bash
SITE=https://birdtest.example.org     # or http://localhost:5173 locally
JAR=$(mktemp)                          # cookie jar for a signed-in session
JOB=...                                # a job id, from its page's URL
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
curl -s -o /dev/null -w '%{http_code}\n' "$SITE/api/jobs/$JOB/positions"    # 401: signed-in users only
curl -s -o /dev/null -w '%{http_code}\n' "$SITE/api/dev/login?username=x"   # 404: must not exist in production
```

### A signed-in user

Sign in once; the session and a CSRF token land in the cookie jar. A request
that changes something (`POST`, `PATCH`, `DELETE`) must send the CSRF cookie
back in an `X-CSRF-Token` header — without it, expect `403` "missing CSRF
header".

```bash
curl -s -c "$JAR" -H 'Content-Type: application/json' \
  -d '{"username":"alice","password":"..."}' "$SITE/api/auth/login"
CSRF=$(awk '$6 == "birdtest_csrf" {print $7}' "$JAR")
```

Expect `{"username":"alice","is_admin":false}`; a wrong password is `401`, and
more than ten attempts a minute from one address `429`.

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

**Search a job's saved positions.** Up to 20 a page, newest first;
`next_cursor`, when present, fetches the next page (`&cursor=…`).

```bash
curl -s -b "$JAR" "$SITE/api/jobs/$JOB/positions?rack=AEINRST&per_page=5" \
  | jq '.items[] | {game_index, turn_number, rack, best: .moves[0].move}'
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

Sign in the same way as an admin (`"is_admin":true` in the answer).

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
activation sets the share (`200`), and one that would take the active jobs
past 100% is `409`. `capture_positions` makes a games or pairs job save its
positions, which the form cannot yet do.

```bash
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"name":"cli test","job_type":"game_pairs","variant":"classic",
       "letterdist_id":"<letterdist id>","layout_id":"<layout id>",
       "player1_config_id":"<config id>","player2_config_id":"<config id>",
       "pairs_per_batch":10,"min_pairs":100,"max_pairs":1000,
       "capture_positions":true}' \
  "$SITE/api/admin/jobs" | jq '.job.id'
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"allocation":10}' "$SITE/api/admin/jobs/$JOB/activate"
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
list (`GET /api/admin/workers`); the ban id comes back from the ban.

```bash
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"anon_uuid":"<uuid>","reason":"testing"}' "$SITE/api/admin/workers/ban"
curl -s -b "$JAR" "$SITE/api/admin/workers/bans"
curl -s -b "$JAR" -H "X-CSRF-Token: $CSRF" -X DELETE "$SITE/api/admin/workers/ban/<ban id>"
```

---

## Before going live

On the production stack itself, after the journeys pass on staging:

- [ ] The site loads over HTTPS, and a page asked for over plain HTTP
  redirects to it. The API over plain HTTP is refused (`426`) rather than
  redirected, so a MAGPIE configured with `server http://…` fails on its first
  request instead of sending its key in the clear:
  `curl -s -o /dev/null -w '%{http_code}\n' http://<host>/api/jobs` prints `426`.
- [ ] `/api/dev/login` answers `404` (Part 3, "What must be refused").
- [ ] Registration and password-reset mail arrives from the production sender,
  and its links point at the production address (U-1, U-4).
- [ ] The first admin exists (A-1) and nobody else is an admin (**Users**).
- [ ] **Derived data** builds a wordmap and a table (A-6): the scheduled builder
  task is running.
- [ ] **Backups** shows a successful run the morning after the first night
  (A-16), and the backup alarms are subscribed (README, "Deploying").
- [ ] A contributor outside your network completes a task (U-8).

## Known quirks

Behaviour a tester will meet that is known at the time of writing, so it is
not reported as new:

- **The New job form cannot save positions.** It never sends
  `capture_positions`; a job that saves them can be made through the API
  (Part 3) or, locally, by `dev.py --reset-db`.
- **Stopping % accepts 0 and 100 in the form**, and the server refuses both
  (it must be strictly between them).
- **Several admin pages show their "nothing here" text while loading**: Fleet,
  Input data, Bans and Derived data.
- **No confirmation** before **Ban worker**, **Insert N rows** (import), or a
  rating pool's **Add** / **Remove** / **Recompute**.
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
