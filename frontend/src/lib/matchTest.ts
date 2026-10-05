/**
 * The significance test's card: player 1's score interval drawn on a scale around an
 * even score, and the sentence that says what it means with the players'
 * names.
 */
import type { TestResult } from '$lib/api';
import { scorePct } from '$lib/format';

/** Where the interval, its mean and an even score sit on the bar, each 0 to 100 (%). */
export interface TestBar {
  lower: number;
  upper: number;
  mean: number;
  even: number;
  /** The scores at the bar's two ends. */
  from: number;
  to: number;
}

/**
 * The bar's scale: the interval and an even score, with a margin of a tenth of
 * that span either side (at least a point), within the scores that exist. So
 * a narrow interval far from 50% and one that straddles it both fill the bar.
 */
export function testBar(test: Pick<TestResult, 'lower' | 'upper' | 'mean'>): TestBar {
  const lo = Math.min(test.lower, 0.5);
  const hi = Math.max(test.upper, 0.5);
  const margin = Math.max((hi - lo) / 10, 0.01);
  const from = Math.max(0, lo - margin);
  const to = Math.min(1, hi + margin);
  const at = (score: number) => ((score - from) / (to - from)) * 100;
  return { lower: at(test.lower), upper: at(test.upper), mean: at(test.mean), even: at(0.5), from, to };
}

/**
 * What the interval says, in a sentence: player 1's score with its range,
 * then the decision, named for the players.
 */
export function testSentence(test: TestResult, names: [string, string]): string {
  const score = `${names[0]} scores ${scorePct(test.mean)} per game (${test.confidence_pct}% interval ${scorePct(test.lower)} to ${scorePct(test.upper)}).`;
  switch (test.status) {
    case 'player1_better':
      return `${score} ${names[0]} is better at ${test.confidence_pct}% confidence.`;
    case 'player2_better':
      return `${score} ${names[1]} is better at ${test.confidence_pct}% confidence.`;
    case 'inconclusive':
      return `${score} Neither is better at ${test.confidence_pct}% confidence: the difference is no larger than the interval.`;
    default:
      return score;
  }
}
