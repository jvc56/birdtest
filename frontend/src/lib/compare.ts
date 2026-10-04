/**
 * A row of a two-player table -- the match score, the pair outcomes -- where
 * each player has a column and, in every row, more is better for the player
 * who has it. The higher value is shown green and the lower red; equal values,
 * or a row without both, neither.
 */
export interface CompareRow {
  label: string;
  /** What the row means, as a tooltip on its label. */
  title?: string;
  /** Each player's value as shown, player 1 first. */
  values: [string, string];
  /** The values compared, or null where a player has none yet. */
  numbers: [number | null, number | null];
}

export type Standing = 'higher' | 'lower' | null;

/** Which of a row's two values is the higher, for its colour. */
export function standings(numbers: [number | null, number | null]): [Standing, Standing] {
  const [a, b] = numbers;
  if (a === null || b === null || a === b) return [null, null];
  return a > b ? ['higher', 'lower'] : ['lower', 'higher'];
}
