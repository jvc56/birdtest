/**
 * Reading what MAGPIE writes about a position: the position itself as CGP
 * (`game_get_cgp_string`, `src/impl/cgp.c`) and a play as it names one
 * (`move_get_string`, `src/str/move_string.c`). The saved-positions board is
 * drawn from these, so nothing here guesses: a string MAGPIE could not have
 * written is `null`, and the page shows the text instead of a wrong board.
 */

/** One tile, on the board or on a rack. */
export interface Tile {
  /** The letter as the distribution names it, upper case: `A`, `L·L`; `?` for a blank on a rack. */
  letter: string;
  /** A blank. On the board it is the letter it was played as, written in lower case. */
  blank: boolean;
}

/** A position as CGP states it. */
export interface Position {
  /** Row by row from the top; `null` is an empty square. */
  board: (Tile | null)[][];
  /** In seat order -- player 1's first -- whoever is to move. */
  racks: [Tile[], Tile[]];
  scores: [number, number];
  /** Consecutive scoreless turns. */
  zeros: number;
}

/**
 * Letters as MAGPIE spells a run of them (`ld_ml_to_hl`): one character each,
 * except a letter of more than one, which is bracketed -- Catalan's `[L·L]`,
 * `[NY]`, `[QU]`. Characters, not UTF-16 units, so `Ç` is one. `null` for a
 * bracket that is unclosed, stray, nested or empty.
 */
export function splitLetters(text: string): string[] | null {
  const out: string[] = [];
  const chars = Array.from(text);
  for (let i = 0; i < chars.length; i++) {
    if (chars[i] === '[') {
      const end = chars.indexOf(']', i + 1);
      const inner = chars.slice(i + 1, end);
      if (end < 0 || inner.length === 0 || inner.includes('[')) return null;
      out.push(inner.join(''));
      i = end;
    } else if (chars[i] === ']') {
      return null;
    } else {
      out.push(chars[i]);
    }
  }
  return out;
}

/** A letter as written: lower case is a blank played as it (`l·l` is `L·L`), `?` a blank on a rack. */
function tile(letter: string): Tile {
  const upper = letter.toUpperCase();
  return { letter: upper, blank: letter === '?' || letter !== upper };
}

const INTEGER = /^-?\d+$/;

/**
 * A CGP string: the board's rows split by `/`, each a run of letters and of
 * digits counting empty squares (`6HUH6`, `15`); then both racks in seat
 * order, `rack1/rack2`; both scores, `18/30`; and the scoreless-turn count.
 * Anything after those four fields -- the options a CGP may carry -- is
 * ignored. The board must be square.
 */
export function parseCgp(cgp: string): Position | null {
  const fields = cgp.trim().split(/\s+/);
  if (fields.length < 4) return null;
  const [boardText, racksText, scoresText, zerosText] = fields;

  const rows = boardText.split('/');
  const board: (Tile | null)[][] = [];
  for (const rowText of rows) {
    const row: (Tile | null)[] = [];
    // Split into runs of digits and runs of anything else, then the letters.
    for (const run of rowText.match(/\d+|[^\d]+/g) ?? []) {
      if (/^\d+$/.test(run)) {
        const empty = Number(run);
        if (empty === 0) return null;
        for (let i = 0; i < empty; i++) row.push(null);
      } else {
        const letters = splitLetters(run);
        if (!letters) return null;
        row.push(...letters.map(tile));
      }
    }
    if (row.length !== rows.length) return null;
    board.push(row);
  }

  const racks = racksText.split('/');
  if (racks.length !== 2) return null;
  const [rack1, rack2] = racks.map(splitLetters);
  if (!rack1 || !rack2) return null;

  const scores = scoresText.split('/');
  if (scores.length !== 2 || !scores.every((s) => INTEGER.test(s)) || !INTEGER.test(zerosText)) {
    return null;
  }
  return {
    board,
    racks: [rack1.map(tile), rack2.map(tile)],
    scores: [Number(scores[0]), Number(scores[1])],
    zeros: Number(zerosText)
  };
}

/** One square of a play, in order along it. */
export interface PlayedSquare {
  row: number;
  col: number;
  /** The letter as written (lower case for a blank); `null` for a square played through and not named. */
  letter: string | null;
  /** Already on the board: played through, not placed by this play. */
  through: boolean;
}

export type Move =
  | { kind: 'pass' }
  | { kind: 'exchange'; tiles: string[] }
  | { kind: 'play'; row: number; col: number; vertical: boolean; squares: PlayedSquare[] };

/**
 * A play as MAGPIE names it. A placement starts at its first square: the row
 * number then the column letter for one across (`8G HUH`), the column letter
 * then the row for one down (`E9 (E)RUVIM`), counted from 1 and `A`. Its
 * letters follow, those already on the board in parentheses, or each as `.`
 * where MAGPIE had no board to name them from. An exchange is `(exch HIRUV)`
 * and a pass `pass`.
 */
export function parseMove(text: string): Move | null {
  const move = text.trim();
  if (move === 'pass') return { kind: 'pass' };
  const exchange = /^\(exch (.+)\)$/.exec(move);
  if (exchange) {
    const tiles = splitLetters(exchange[1]);
    return tiles ? { kind: 'exchange', tiles } : null;
  }

  const across = /^(\d+)([A-Z])\s+(\S+)$/.exec(move);
  const down = /^([A-Z])(\d+)\s+(\S+)$/.exec(move);
  let row: number, col: number, word: string;
  if (across) {
    [row, col, word] = [Number(across[1]) - 1, across[2].charCodeAt(0) - 65, across[3]];
  } else if (down) {
    [row, col, word] = [Number(down[2]) - 1, down[1].charCodeAt(0) - 65, down[3]];
  } else {
    return null;
  }
  if (row < 0) return null;
  const vertical = !across;

  const squares: PlayedSquare[] = [];
  const chars = Array.from(word);
  let through = false;
  const push = (letter: string | null, isThrough: boolean) => {
    const i = squares.length;
    squares.push({
      row: vertical ? row + i : row,
      col: vertical ? col : col + i,
      letter,
      through: isThrough
    });
  };
  for (let i = 0; i < chars.length; i++) {
    const c = chars[i];
    if (c === '(') {
      if (through) return null;
      through = true;
    } else if (c === ')') {
      if (!through) return null;
      through = false;
    } else if (c === '.') {
      push(null, true);
    } else if (c === '[') {
      const end = chars.indexOf(']', i + 1);
      if (end <= i + 1) return null;
      push(chars.slice(i + 1, end).join(''), through);
      i = end;
    } else if (c === ']') {
      return null;
    } else {
      push(c, through);
    }
  }
  if (through || squares.length === 0) return null;
  return { kind: 'play', row, col, vertical, squares };
}

/**
 * The squares a play placed tiles on, as `row,col` keys -- what the board
 * highlights for the previous move. Nothing for a pass or an exchange, or a
 * play that cannot be read.
 */
export function placedSquares(move: string | null | undefined): Set<string> {
  const parsed = move ? parseMove(move) : null;
  if (parsed?.kind !== 'play') return new Set();
  return new Set(parsed.squares.filter((s) => !s.through).map((s) => `${s.row},${s.col}`));
}

/**
 * Which seat holds `rack` -- the rack of the player to move, as a saved
 * position states it -- or `null` when neither does. Compared as tiles, in any
 * order. Both racks the same is the same picture either way; the first wins.
 */
export function seatToMove(position: Position, rack: string): 0 | 1 | null {
  const letters = splitLetters(rack);
  if (!letters) return null;
  const key = (tiles: string[]) => [...tiles].sort().join('\u0000');
  const wanted = key(letters.map((l) => tile(l).letter));
  const index = position.racks.findIndex((r) => key(r.map((t) => t.letter)) === wanted);
  return index === 0 || index === 1 ? index : null;
}
