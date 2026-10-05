/**
 * A row of a two-player table -- the match score, the pair outcomes -- where
 * each player has a column. The better value is shown green and the worse
 * red; equal values, or a row without both, neither. More is better unless
 * the row says otherwise (losses: fewer).
 */
export interface CompareRow {
  label: string;
  /** What the row means, as a tooltip on its label. */
  title?: string;
  /** Each player's value as shown, player 1 first. */
  values: [string, string];
  /** The values compared, or null where a player has none yet. */
  numbers: [number | null, number | null];
  /** Which way is better; more, unless stated. */
  better?: 'higher' | 'lower';
}

export type Standing = 'better' | 'worse' | null;

/** Which of a row's two values is the better, for its colour. */
export function standings(
  numbers: [number | null, number | null],
  better: 'higher' | 'lower' = 'higher'
): [Standing, Standing] {
  const [a, b] = numbers;
  if (a === null || b === null || a === b) return [null, null];
  return (a > b) === (better === 'higher') ? ['better', 'worse'] : ['worse', 'better'];
}
