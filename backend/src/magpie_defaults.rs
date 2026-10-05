//! MAGPIE's defaults for every setting a task request states.
//!
//! A request used to leave most settings null, meaning "whatever the worker's
//! MAGPIE compiled in". That made a result a function of the task *and* of the
//! release that ran it: a release that changed a default played the same task
//! differently, and a version floor -- a minimum, not a pin -- could not keep
//! it out. So the server writes these values into the rows requests are built
//! from, once, when a player config or a job is created, and every request
//! states them. MAGPIE refuses a request that does not.
//!
//! Written into rows rather than applied at dispatch, so changing a value here
//! changes configs and jobs created afterwards only: an existing config keeps
//! playing as it did, and its ratings keep comparing like with like.
//!
//! Each value is MAGPIE's on `birdtest-contribute`: the `CONFIG_DEFAULT_*`
//! constants and `config_contribute_reset_player_settings` in
//! `src/impl/config.c`, and `DEFAULT_BINGO_BONUS` in `src/def/config_defs.h`.
//! Keep them in step with that build.

// Settings every player states, simulating or not.

/// `-s`: the contribute reset sorts on equity. A simming player sorts its
/// candidates too, before simulating them.
pub const SORT_STRATEGY: &str = "equity";
/// `-np` (`CONFIG_DEFAULT_NUM_PLAYS`). Read by a static player as well: an
/// opening-rack analysis sizes its move list from it.
pub const NUM_PLAYS: i32 = 100;
/// `shplies` (`CONFIG_DEFAULT_SHPLIES`).
pub const NUM_PLIES_RECORDED: i32 = 2;
/// `-mmargin` (`CONFIG_DEFAULT_EQ_MARGIN`), in points.
pub const MOVEGEN_MARGIN: f64 = 5.0;

// Simulation settings: stated for a simming player only, since a static one
// never reads them.

/// `-sc` (`CONFIG_DEFAULT_STOP_COND_PCT`).
pub const STOPPING_PCT: f64 = 99.0;
/// `-si`: the contribute reset turns inference on.
pub const USE_INFERENCE: bool = true;
/// `-mi` (`CONFIG_DEFAULT_MIN_PLAY_ITERATIONS`).
pub const MIN_PLAY_ITERATIONS: i32 = 500;
/// `-th`: the contribute reset's `BAI_THRESHOLD_GK16`.
pub const THRESHOLD: &str = "gk16";
/// `-sa`: the contribute reset's `BAI_SAMPLING_RULE_TOP_TWO_IDS`.
pub const SAMPLING_RULE: &str = "top_two_ids";
/// `-im` (`CONFIG_DEFAULT_EQ_MARGIN`), in points.
pub const INFERENCE_MARGIN: f64 = 5.0;
/// `-uwin` (`CONFIG_DEFAULT_UTILITY_W_WINPCT`).
pub const UTILITY_W_WINPCT: f64 = 1.0;
/// `-uspread` (`CONFIG_DEFAULT_UTILITY_W_SPREAD`).
pub const UTILITY_W_SPREAD: f64 = 0.5;
/// `-uspreadscale` (`CONFIG_DEFAULT_UTILITY_SPREAD_SCALE`).
pub const UTILITY_SPREAD_SCALE: f64 = 100.0;

// Endgame and pre-endgame solving: off unless a config asks for it. When it
// does, a setting it leaves out takes these, MAGPIE's own CLI defaults
// (`eplies`, `PEG_MAX_BAG`, and `autoplay_solver_settings_set_defaults` in
// `src/ent/autoplay_solver_settings.h`).

/// `-eplies1`/`-eplies2` when the endgame is on (MAGPIE's `eplies` default).
pub const ENDGAME_PLIES: i32 = 6;
/// `-pegbag1`/`-pegbag2` when PEG is on (`PEG_MAX_BAG`).
pub const PEG_MAX_BAG: i32 = 4;
/// `-pegtopk1`/`-pegtopk2`: survivors per halving stage.
pub const PEG_STAGE_TOP_K: [i32; 5] = [32, 16, 8, 4, 2];
/// `-pegstride1`/`-pegstride2`: full enumeration.
pub const PEG_SCENARIO_STRIDE: i32 = 1;
/// `-pegpess1`/`-pegpess2` false.
pub const PEG_OPP_MODEL: &str = "rational";
/// `-pegnested1`/`-pegnested2`.
pub const PEG_NESTED: bool = true;
/// `-pegncaps`.
pub const PEG_NESTED_CAND_CAPS: [i32; 3] = [8, 4, 2];
/// `-pegndepth` (`PEG_NESTED_DEFAULT_DEPTH`).
pub const PEG_NESTED_MAX_DEPTH: i32 = 1;
/// `-pegnstrides`: an inner pre-endgame's stride for a bag of 1, 2, 3 and 4
/// tiles (peg.c's bag-dependent default).
pub const PEG_NESTED_STRIDES: [i32; 4] = [1, 1, 5, 7];

// Run-wide settings, stored on the job.

/// `-bb` (`DEFAULT_BINGO_BONUS`).
pub const BINGO_BONUS: i32 = 50;
/// `-cutoff` (`CONFIG_DEFAULT_USER_CUTOFF`).
pub const SIM_CUTOFF: f64 = 0.005;
