/**
 * The significance test's card: its verdict in a word or a name, player 1's
 * score interval drawn on a scale with an even score in the middle, and the
 * pairs or games played against the job's minimum and cap.
 */
import type { GameStats, TestResult } from '$lib/api';

/** A score per game as a percentage, to two places, as the card prints it: 0.57416 is "57.42%". */
export function testPct(score: number): string {
  return `${(score * 100).toFixed(2)}%`;
}

/** Where the interval and its mean sit on the chart, each 0 to 100 (%), and its ticks. */
export interface TestScale {
  lower: number;
  upper: number;
  mean: number;
  ticks: { at: number; label: string }[];
}

/** The half-widths the chart's scale snaps to, either side of 50%. */
const HALF_WIDTHS = [0.02, 0.05, 0.1, 0.15, 0.2, 0.3, 0.5];

/**
 * The chart's scale: an even score always in the middle, so the left half is
 * player 2 better and the right half player 1, and wide enough either side for
 * the interval and a little room, snapped to a round width so the ticks read
 * as round scores.
 */
export function testScale(test: Pick<TestResult, 'lower' | 'upper' | 'mean'>): TestScale {
  const reach = Math.max(Math.abs(test.lower - 0.5), Math.abs(test.upper - 0.5), Math.abs(test.mean - 0.5));
  const half = HALF_WIDTHS.find((h) => h >= reach * 1.12) ?? 0.5;
  const at = (score: number) => ((score - (0.5 - half)) / (2 * half)) * 100;
  return {
    lower: at(test.lower),
    upper: at(test.upper),
    mean: at(test.mean),
    // The ends, halfway in, and even: placed by their fraction of the chart
    // rather than through `at`, which can land a hair off 25 or 50.
    ticks: [-1, -0.5, 0, 0.5, 1].map((k) => {
      const score = 0.5 + k * half;
      // One place at most, and none when it is whole: "42.5%", "50%".
      return { at: (k + 1) * 50, label: `${Math.round(score * 1000) / 10}%` };
    })
  };
}

/**
 * The verdict tile: a word, or the better player's index (0 or 1) to name,
 * and the line under it; and the header chip's text, null where it names the
 * better player, and whether the test is still running.
 */
export interface Verdict {
  word: string | null;
  player: 0 | 1 | null;
  line: string;
  chip: string | null;
  running: boolean;
}

/**
 * The verdict, from where the test stands (`testState`): before the test may
 * act, while it runs, the player it found better, or why it has none.
 */
export function testVerdict(
  state: string,
  games: Pick<GameStats, 'unit' | 'units_completed' | 'min_units' | 'decided'> & { test: Pick<TestResult, 'confidence_pct'> }
): Verdict {
  const units = (n: number) => `${n.toLocaleString()} ${games.unit}${n === 1 ? '' : 's'}`;
  const better = (player: 0 | 1): Verdict => ({
    word: null,
    player,
    line: games.decided
      ? `is better, decided at ${units(games.decided.units)}`
      : `is better at ${games.test.confidence_pct}% confidence`,
    chip: null,
    running: false
  });
  switch (state) {
    case 'running':
      return games.units_completed < games.min_units
        ? { word: 'Too early', player: null, line: `acts from ${units(games.min_units)}`, chip: 'running', running: true }
        : { word: 'No verdict yet', player: null, line: 'interval still includes 50%', chip: 'running', running: true };
    case 'player1_better':
      return better(0);
    case 'player2_better':
      return better(1);
    case 'inconclusive':
      return {
        word: 'Neither better',
        player: null,
        line: 'cap reached, interval still includes 50%',
        chip: 'inconclusive',
        running: false
      };
    case 'paused':
      return {
        word: 'Paused',
        player: null,
        line: `no ${games.unit}s are being played while the job is inactive`,
        chip: 'paused',
        running: false
      };
    default:
      return {
        word: 'Not decided',
        player: null,
        line: 'the job was completed before the test decided',
        chip: 'not decided',
        running: false
      };
  }
}

/**
 * What job creation refuses of a significance test's confidence, or null: it
 * must be above 50 (at or below half it is no test) and below 100 (the
 * interval would never close). Checked here rather than by the input's `min`
 * and `max`, which can only state inclusive bounds and so either let 50 and
 * 100 through or block values the server takes (`min="50.1"` blocked 50.05).
 */
export function confidenceProblem(pct: number | null): string | null {
  if (pct == null || !Number.isFinite(pct) || !(pct > 50 && pct < 100)) {
    return 'The confidence must be above 50% and below 100%.';
  }
  return null;
}
