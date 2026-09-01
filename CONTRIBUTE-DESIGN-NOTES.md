# Contribute design notes

Answers to four questions about the MAGPIE-side `contribute` implementation
(`birdtest-contribute` in the MAGPIE repo). Line references are to that branch.

---

## 1. Why does `AutoplayResults` need a `char *leave_results_json` field?

Because the producer and the consumer of that string are separated by a
function boundary that has nowhere else to carry it, and the data it is built
from is dead before the consumer runs.

The string is produced in `postgen_prebroadcast_func` (`src/impl/autoplay.c`),
the checkpoint callback that fires when a leavegen generation closes:

```c
autoplay_results_set_leave_results_json(
    lg_shared_data->primary_autoplay_results,
    rack_list_get_rack_equity_json(lg_shared_data->rack_list,
                                   lg_shared_data->ld));
```

That is the *only* moment the `RackList` is both fully populated for the
generation and still alive. It is consumed in `config_contribute_leave_gen`
(`src/impl/config.c`), after `config_autoplay` returns.

Everything in between is gone by then. `LeavegenSharedData` — which owns the
`RackList` — is created inside `autoplay()` and destroyed inside it, by
`autoplay_shared_data_destroy` at the end of the run. The callback itself gets
only a `void *` to `AutoplaySharedData`; it has no handle on the caller. So by
the time `config_contribute_leave_gen` regains control, there is nothing left
in autoplay.c to read the result out of.

So the question is really: *where can a leavegen run park a string so the
caller of `autoplay()` can pick it up?* The candidates:

**A file.** This is what the code did before: `-writerackequitycsv` wrote
`<klv>_rack_equity.csv` and the caller parsed it back. That flag, and the CSV
writer behind it, are gone now — a worker rendering JSON, writing it to disk,
reading it back, and parsing it, all to hand it to an HTTP POST, is a round
trip through the filesystem for data that never needed to leave the process.
It also makes the task depend on a writable data directory for a reason
unrelated to the lexicon data. (The forced racks went the same way in the same
change: they arrive in the task's JSON request and are handed to
`rack_list_create` as a plain array of strings, rather than being written to a
scratch file and read back by line.)

**A file-static in autoplay.c.** Mechanically this works, and it is what
"the data lives entirely in autoplay.c" would have to mean. But it is global
mutable state: two `autoplay()` runs in one process would clobber each other,
and MAGPIE has no other global like this. The `contribute` loop happens to run
one task at a time today, which makes it safe today — a property nothing
enforces and nothing states.

**An out-parameter on `autoplay()` / `config_autoplay()`.** This threads a
leavegen-only `char **` through two signatures that every autoplay caller uses,
so `autoplay games 100` grows a parameter that only `leavegen` ever writes.

**`AutoplayResults`.** It is the object that already exists for exactly this
purpose — it is *the* results channel out of an autoplay run. It already
outlives the run (owned by `Config`, not by `autoplay()`). It is already
reachable from the callback: `LeavegenSharedData` holds
`primary_autoplay_results` precisely so postgen can write into it. And the
caller already has it in hand. No new lifetime, no new plumbing, no new global.

The cost is one pointer on a struct that non-leavegen runs leave NULL, plus a
`free` in `autoplay_results_destroy`. That is the cheapest of the four.

One wrinkle worth knowing: the field is not reset by
`autoplay_results_reset`, which only resets recorders (see §3). It is freed and
replaced by `autoplay_results_set_leave_results_json` on every write, so a
multi-generation run keeps the last generation's string rather than leaking
each one.

---

## 2. Fixed-size `CAPTURED_*_STRING_SIZE` arrays vs. dynamic allocation

First, a correction to the framing: these arrays are not on the stack. They are
inline members of `CapturedPosition` and `CapturedPlay`, and both live in
heap-allocated arrays (`data->positions`, `position->plays`). The real
trade-off is *inline fixed-size field* vs. *pointer to a separate allocation*,
not stack vs. heap.

### Why the fixed size wins here

**It removes a malloc/free pair per string, at capture rate.** A position is
captured on every turn of every game. At ~22.5 turns a game, a 100-game batch
captures ~2,250 positions. Each position holds 3 strings (rack, CGP, previous
move) and each stored play holds 1 (the move). With a play cap of 15, that is
2,250 × (3 + 15) ≈ 40,000 strings. Inline, that is zero allocator calls; as
pointers it is 40,000 mallocs and 40,000 frees, all of them tiny, all of them
on the hot path, and all of them contending on the allocator across worker
threads.

**It makes the writers bounded.** `rack_get_string`, `move_get_string` and
`game_get_cgp_string` all take `(char *dest, size_t dest_size)` and truncate.
That is why `append_bounded`/`append_int_bounded` exist at all. There is no
measure-then-allocate-then-format pass, and no `StringBuilder` churn: the
capture writes straight into its final home.

**It keeps the array contiguous and the position a value.** `data->positions`
is one block that `realloc`s by doubling. Growing it moves bytes; it does not
have to chase and re-point 3 pointers per element. Consolidation walks the
whole run's captures linearly, and every string it reads is in the same cache
lines as the struct that owns it.

**It makes the free path trivial.** `positions_data_free_contents` frees one
`plays` array per position, not four strings per position plus the arrays.

Note that the code does *not* apply this reasoning dogmatically. The plays list
per position **is** dynamically allocated, and the comment on the recorder says
why: a position can legally have hundreds of ranked plays, so there is no
honest fixed bound. The fixed-size choice is made where a tight bound exists
(a rack is `RACK_SIZE` tiles, a move covers at most `BOARD_DIM` squares, a CGP
is at most a full board plus two racks) and rejected where it does not.

### The size cost

The bounds are sized for the longest human-readable letter any distribution
MAGPIE actually ships (`MAX_SHIPPED_LETTER_BYTE_LENGTH = 4`, Catalan's `L·L`
with its U+00B7 middle dot), not for `MAX_LETTER_BYTE_LENGTH = 6`, which is
only the parser's ceiling. With `BOARD_DIM = 15`, `RACK_SIZE = 7`:

| Field | Formula | Bound | English worst case | English typical |
|---|---|---|---|---|
| `rack` | `RACK_SIZE * 4 + 1` | **29** | 8 | 8 |
| `previous_move` / `move` | `BOARD_DIM * (4+2) + 16` | **106** | ~36 | ~12 |
| `cgp` | `225*4 + 15 + 2*29 + 64` | **1037** | ~269 | ~130 |

English is one byte per letter, so the multiplier is 4× on everything that
scales with letter length. The dominant term is the CGP: 1,037 bytes reserved
where a full board (all 225 squares occupied — the true maximum, since empty
runs compress to digits) plus both racks, both scores and the scoreless count
needs about 269, and a real mid-game position needs about 130.

Per struct:

| | Bytes |
|---|---|
| `sizeof(CapturedPosition)` | 1,240 |
| — of which inline strings | 1,172 |
| `sizeof(CapturedPlay)` | 328 |
| — of which inline `move` | 106 |

For one position with 15 stored plays: `1,240 + 15 × 328` = **6,160 bytes**.

A pointer-based equivalent, for the same English position, would be roughly:
struct shrinks to ~110 bytes (3 pointers replacing 1,172 bytes of buffer), plus
~150 bytes of actual string data, plus 4 allocation headers at ~16–32 bytes
each — call it ~350 bytes, and 18 allocator round trips. So per position the
inline form costs roughly **4–5 KB more**, and per 1,000 positions roughly
**4–5 MB more**.

That is the honest number, and it is real: a 100-game batch sits around 6 MB
instead of around 1 MB. Whether that is the right trade depends on the ceiling,
not the average — and the ceiling here is bounded by the batch size the server
hands out, not by anything open-ended. Note that with the CGP bound now sized
to the shipped distributions rather than to the compile-time ceiling, most of
what remains is the per-play `move` buffer multiplied by the play cap, not the
position itself; cutting further would mean capturing the CGP's letters as
machine letters, or sizing the buffers from the loaded `LetterDistribution` at
run time, which brings back an allocation per capture.

---

## 3. Why was `autoplay_results_reset(primary);` removed?

It moved rather than vanished. `autoplay_results_consolidate` used to reset the
whole primary up front:

```c
cpthread_mutex_lock(&primary->mutex);
autoplay_results_reset(primary);            // <- removed
Recorder **recorder_list = ...
for (int i = 0; i < NUMBER_OF_AUTOPLAY_RECORDERS; i++) {
  if (!autoplay_results_list[0]->recorders[i]) continue;
  ...
```

and now resets each recorder inside the loop, after the guard:

```c
for (int i = 0; i < NUMBER_OF_AUTOPLAY_RECORDERS; i++) {
  if (!autoplay_results_list[0]->recorders[i]) continue;
  recorder_reset(primary->recorders[i]);    // <- moved here
  ...
```

The change was made in `95a9f66e`, when the positions recorder briefly became a
single structure shared live across every worker thread. In that design
`positions_data_consolidate` was a genuine no-op — every capture had already
landed in the one shared list — so the primary's positions recorder was *not* a
blank merge target the way every other recorder is. It held the entire run's
data. Resetting it before "merging" would have thrown the run away.

The reset had to be scoped rather than deleted, because the other recorders
(game data, FJ, win%, leaves) genuinely do need clearing: consolidation sums
per-thread totals into the primary, so a primary carrying a previous
consolidation's numbers would double-count. Putting `recorder_reset` inside the
loop, after the `continue` guard, says exactly the right thing: *reset the
merge targets you are about to merge into, and nothing else.*

That is still the reason it stays where it is. The positions recorder has since
gone back to per-thread arrays with a real consolidate step (`01a8e704`), so
today the two forms would behave the same for the recorders a run actually has.
But the in-loop form is the one that states the invariant, and it is the one
that keeps working if a recorder ever again holds state that consolidation does
not rebuild. It also stops the reset from touching the primary's positions
shared JSON on iterations that skip the merge — `positions_data_reset` frees
`shared_data->json` when the recorder owns the shared data, which is a real
side effect on an object other code reads.

Worth noting for completeness: `autoplay_results_reset` only resets recorders.
It does not touch `leave_results_json` or `cached_json`, so this change has no
bearing on either.

---

## 4. Why does contribute leavegen need `leavegen_max_games`? Doesn't leavegen already have a max-games-per-generation setting?

It does not. That is the whole reason the field exists.

The `leavegen` command takes two required positional arguments and one
optional:

```
leavegen <min_rack_targets> <games_before_force_draw_start> [forced_racks_file]
```

Neither of the first two is a game cap:

- **`min_rack_targets`** is a comma-separated list with one entry *per
  generation* — the usual invocation is `100,200,500,1000,1000,1000`, meaning
  six generations whose targets are that every rack occur at least 100 times,
  then 200, then 500, then 1,000 three times. `autoplay()` derives `num_gens`
  from the number of entries in this list.
- **`games_before_force_draw_start`** is the one that *looks* like a game
  count, and is probably the source of the impression. It is how many games
  into a generation to play before forced draws begin — a warm-up, not a limit.
  It is compared against `iter_count - gen_start_games` in
  `game_runner_start_new_game`, and it only ever turns forcing *on*.

A generation ends when `target_min_leave_count_reached` returns true, i.e. when
`rack_list_get_racks_below_target_count() == 0`. Nothing else stops it. That is
visible in `autoplay()`:

```c
first_gen_num_games =
    args->leavegen_max_games > 0 ? args->leavegen_max_games : UINT64_MAX;
```

With `leavegen_max_games == 0` — the CLI's behavior, unchanged — the iteration
cap is `UINT64_MAX`. A hand-run `leavegen` is unbounded in games by design:
you say what coverage you want and it plays until it has it.

That is fine for an interactive run on a full, unrestricted rack universe,
where every rack is drawable and coverage arrives at a predictable rate. It is
not fine for a distributed task. A `leave_generation` task gets a *subset* of
racks (`forced_racks`, sized by the server's `racks_per_task`) and the
generation's target belongs to the server, which is accumulating counts across
every task in the generation — no single task can reach it or even observe
whether it has been reached globally. Without a cap, a task whose forced-rack
subset happens to be slow to fill would run forever: it would hold its claim,
miss no heartbeat, and never submit.

`leavegen_max_games` is that cap, set from the request's `num_games`. The task
then terminates on whichever comes first:

1. its own forced racks all reach `target_rack_count` — an early-out, since
   further games cannot change what this task reports; or
2. `num_games` games are played.

Either way `postgen_prebroadcast_func` runs at the checkpoint after the
generation loop, `leave_results_json` is set, and the racks that did occur are
reported. The server folds them into `leave_rack_progress` and decides on its
own whether the generation is finished.

### One subtlety

`leavegen_max_games` caps the **whole run**, not each generation.
`shared_data->max_iter_count` is set once from `first_gen_num_games` and is
never raised between generations — only `gen_start_games` moves. For the
contribute path this is exactly right, because a task is a single generation
(`num_gens == 1`, one target), so whole-run and per-generation are the same
thing. It would matter if a multi-generation run ever set the field; nothing
does today, and the field's comment now says so.
