import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { parseCgp, parseMove, placedSquares, seatToMove, splitLetters } from './cgp';

// Turn 3 of the first game in contract-fixtures/result-games.json, which is
// MAGPIE's own output: after 8G HUH, 9C FLEAMS and E9 (E)RUVIM.
const FIXTURE =
  '15/15/15/15/15/15/15/6HUH6/2FLEAMS7/4R10/4U10/4V10/4I10/4M10/15 ANORRRY/BGLOTTX 40/30 0';

describe('F-CGP-1 parseCgp', () => {
  it('reads the board row by row, both racks in seat order, the scores and the zeros', () => {
    const p = parseCgp(FIXTURE)!;
    expect(p.board).toHaveLength(15);
    expect(p.board.every((row) => row.length === 15)).toBe(true);
    expect(p.board[7].map((t) => t?.letter ?? '.').join('')).toBe('......HUH......');
    expect(p.board[8][2]).toEqual({ letter: 'F', blank: false });
    expect(p.board[13][4]).toEqual({ letter: 'M', blank: false });
    expect(p.board[0].every((t) => t === null)).toBe(true);
    expect(p.racks[0].map((t) => t.letter).join('')).toBe('ANORRRY');
    expect(p.racks[1].map((t) => t.letter).join('')).toBe('BGLOTTX');
    expect(p.scores).toEqual([40, 30]);
    expect(p.zeros).toBe(0);
  });

  it('reads the empty opening board, an empty rack and options after the four fields', () => {
    const p = parseCgp('15/15/15/15/15/15/15/15/15/15/15/15/15/15/15 AEINRST/ 0/0 0 lex NWL23;')!;
    expect(p.board.flat().every((t) => t === null)).toBe(true);
    expect(p.racks[0]).toHaveLength(7);
    expect(p.racks[1]).toEqual([]);
  });

  it('reads a blank on the board in lower case, a blank on a rack as ?, and bracketed letters as one tile', () => {
    const rows = Array(15).fill('15');
    rows[7] = '5c[L·L]Ç[ny]6';
    const p = parseCgp(`${rows.join('/')} A[QU]?/[L·L]?? 12/-4 3`)!;
    expect(p.board[7].slice(5, 9)).toEqual([
      { letter: 'C', blank: true },
      { letter: 'L·L', blank: false },
      { letter: 'Ç', blank: false },
      { letter: 'NY', blank: true }
    ]);
    expect(p.board[7]).toHaveLength(15);
    expect(p.racks[0]).toEqual([
      { letter: 'A', blank: false },
      { letter: 'QU', blank: false },
      { letter: '?', blank: true }
    ]);
    expect(p.racks[1].map((t) => t.letter)).toEqual(['L·L', '?', '?']);
    expect(p.scores).toEqual([12, -4]);
    expect(p.zeros).toBe(3);
  });

  it('refuses what MAGPIE could not have written', () => {
    const board = Array(15).fill('15').join('/');
    for (const bad of [
      '',
      `${board} AB/CD 0/0`, // three fields
      `${board} AB 0/0 0`, // one rack
      `${board} A[B/CD 0/0 0`, // an unclosed bracket
      `${board} AB/CD 0/x 0`,
      `${board.replace('15', '14')} AB/CD 0/0 0`, // a short row
      `${board.replace('15', '16')} AB/CD 0/0 0`,
      `${board.replace('15', '0A14')} AB/CD 0/0 0`,
      `15/15 AB/CD 0/0 0` // not square
    ]) {
      expect(parseCgp(bad), bad).toBeNull();
    }
  });
});

describe('F-CGP-2 parseMove', () => {
  it('reads a play across as row then column, and one down as column then row', () => {
    expect(parseMove('8G HUH')).toEqual({
      kind: 'play',
      row: 7,
      col: 6,
      vertical: false,
      squares: [
        { row: 7, col: 6, letter: 'H', through: false },
        { row: 7, col: 7, letter: 'U', through: false },
        { row: 7, col: 8, letter: 'H', through: false }
      ]
    });
    const down = parseMove('E9 (E)RUVIM');
    expect(down).toMatchObject({ kind: 'play', row: 8, col: 4, vertical: true });
    expect(down?.kind === 'play' && down.squares.map((s) => [s.row, s.col, s.letter, s.through])).toEqual([
      [8, 4, 'E', true],
      [9, 4, 'R', false],
      [10, 4, 'U', false],
      [11, 4, 'V', false],
      [12, 4, 'I', false],
      [13, 4, 'M', false]
    ]);
    expect(parseMove('10E RIM')).toMatchObject({ row: 9, col: 4, vertical: false });
    expect(parseMove('O15 A')).toMatchObject({ row: 14, col: 14, vertical: true });
  });

  it('reads letters played through, named or as dots, blanks and bracketed letters', () => {
    const play = parseMove('8D a(B[L·L])[ny].');
    expect(play?.kind === 'play' && play.squares.map((s) => [s.col, s.letter, s.through])).toEqual([
      [3, 'a', false],
      [4, 'B', true],
      [5, 'L·L', true],
      [6, 'ny', false],
      [7, null, true]
    ]);
  });

  it('reads an exchange and a pass, and refuses anything else', () => {
    expect(parseMove('(exch HIRUV)')).toEqual({ kind: 'exchange', tiles: ['H', 'I', 'R', 'U', 'V'] });
    expect(parseMove('(exch [QU]?)')).toEqual({ kind: 'exchange', tiles: ['QU', '?'] });
    expect(parseMove('pass')).toEqual({ kind: 'pass' });
    for (const bad of ['', '8 HUH', 'G HUH', '8G', 'GG HUH', '0G HUH', '8G (HUH', '8G HU)H', '8G ()']) {
      expect(parseMove(bad), bad).toBeNull();
    }
  });
});

describe('F-CGP-3 placedSquares and seatToMove', () => {
  it('highlights the tiles a play placed, not those it played through, and nothing for a pass or exchange', () => {
    expect([...placedSquares('E9 (E)RUVIM')]).toEqual(['9,4', '10,4', '11,4', '12,4', '13,4']);
    expect([...placedSquares('E5 MURR(E)')]).toEqual(['4,4', '5,4', '6,4', '7,4']);
    expect(placedSquares('(exch HIRUV)').size).toBe(0);
    expect(placedSquares('pass').size).toBe(0);
    expect(placedSquares(null).size).toBe(0);
    expect(placedSquares('nonsense').size).toBe(0);
  });

  it("finds the seat holding the mover's rack, in any order", () => {
    const p = parseCgp(FIXTURE)!;
    expect(seatToMove(p, 'BGLOTTX')).toBe(1);
    expect(seatToMove(p, 'YRRRONA')).toBe(0);
    expect(seatToMove(p, 'AEINRST')).toBeNull();
    const blanks = parseCgp(FIXTURE.replace('ANORRRY/', 'ANORRY?/'))!;
    expect(seatToMove(blanks, 'ANORRY?')).toBe(0);
  });

  it('splits letters as MAGPIE spells them', () => {
    expect(splitLetters('AÇ[L·L]?')).toEqual(['A', 'Ç', 'L·L', '?']);
    expect(splitLetters('')).toEqual([]);
    for (const bad of ['[', ']', '[]', '[A[B]]']) expect(splitLetters(bad), bad).toBeNull();
  });
});

// What the board is drawn from, read from both writers: MAGPIE itself (the
// contract fixture) and the fake worker the e2e suite plays with (its captured
// submission, regenerated as backend/src/jobs/testdata/README.md says).
describe('F-CGP-4 captured positions from MAGPIE and the fake worker', () => {
  const root = join(__dirname, '..', '..', '..');
  const sources: [string, { positions: CapturedPosition[] }][] = [
    ['MAGPIE', JSON.parse(readFileSync(join(root, 'contract-fixtures', 'result-games.json'), 'utf8')).result],
    [
      'the fake worker',
      JSON.parse(
        readFileSync(join(root, 'backend', 'src', 'jobs', 'testdata', 'fake_worker_games_captured.json'), 'utf8')
      )
    ]
  ];

  for (const [writer, { positions }] of sources) {
    it(`reads every position ${writer} wrote, and each previous move is the difference from the turn before`, () => {
      expect(positions.length).toBeGreaterThan(3);
      const byTurn = new Map(positions.map((p) => [`${p.game_index}:${p.turn_number}`, p]));
      for (const p of positions) {
        const position = parseCgp(p.position);
        expect(position, p.position).not.toBeNull();
        expect(seatToMove(position!, p.rack), p.position).not.toBeNull();
        const before = byTurn.get(`${p.game_index}:${p.turn_number - 1}`);
        if (!before) continue;
        const earlier = parseCgp(before.position)!;
        const placed = placedSquares(p.previous_move);
        const move = parseMove(p.previous_move!);
        expect(move, p.previous_move).not.toBeNull();
        for (let r = 0; r < position!.board.length; r++) {
          for (let c = 0; c < position!.board.length; c++) {
            const now = position!.board[r][c];
            if (placed.has(`${r},${c}`)) {
              // Placed by the move: empty before, and the letter it names.
              expect(earlier.board[r][c], `${p.previous_move} at ${r},${c}`).toBeNull();
              const square = move?.kind === 'play' && move.squares.find((s) => s.row === r && s.col === c);
              expect(square && now && (now.blank ? now.letter.toLowerCase() : now.letter)).toBe(
                square && square.letter
              );
            } else {
              expect(now, `${p.previous_move} left ${r},${c} alone`).toEqual(earlier.board[r][c]);
            }
          }
        }
        // The mover's score went up by the move's.
        const mover = seatToMove(earlier, before.rack)!;
        expect(position!.scores[mover] - earlier.scores[mover]).toBe(p.previous_move_score);
      }
    });
  }
});

interface CapturedPosition {
  game_index: number;
  turn_number: number;
  rack: string;
  position: string;
  previous_move?: string;
  previous_move_score?: number;
  moves: { move: string }[];
}
