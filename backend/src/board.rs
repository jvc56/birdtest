//! A job's board as MAGPIE reads it: the layout file, parsed.
//!
//! Job creation needs only to know whether MAGPIE would load a layout
//! ([`layout_problem`]); the saved-positions page needs the layout itself, to
//! draw the premium squares under a captured position. One parser serves both,
//! so a board the page draws is exactly a board a job could be created on.

use serde::Serialize;

/// The board every MAGPIE the fleet runs is built for (`BOARD_DIM`): a claim
/// states its build's, and one of another size is sent away
/// (`unsupported_build`, `routes::worker`). A layout of another size, or one
/// MAGPIE otherwise refuses, loads on no worker: each fails the task, and after
/// five in a row `magpie contribute` stops.
pub const MAGPIE_BOARD_DIM: usize = 15;

/// One square of a layout, by MAGPIE's symbol for it (`bonus_square.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Square {
    /// ` `
    Normal,
    /// `'`
    DoubleLetter,
    /// `-`
    DoubleWord,
    /// `"`
    TripleLetter,
    /// `=`
    TripleWord,
    /// `^`
    QuadrupleLetter,
    /// `~`
    QuadrupleWord,
    /// `#`: a square no tile may be played on.
    Brick,
}

impl Square {
    fn from_symbol(symbol: char) -> Option<Self> {
        Some(match symbol {
            ' ' => Self::Normal,
            '\'' => Self::DoubleLetter,
            '-' => Self::DoubleWord,
            '"' => Self::TripleLetter,
            '=' => Self::TripleWord,
            '^' => Self::QuadrupleLetter,
            '~' => Self::QuadrupleWord,
            '#' => Self::Brick,
            _ => return None,
        })
    }
}

/// A layout MAGPIE would load.
#[derive(Debug, Clone, Serialize)]
pub struct BoardLayout {
    /// The square the first play must cover, `[row, column]` from zero.
    pub start: [usize; 2],
    /// `MAGPIE_BOARD_DIM` rows of `MAGPIE_BOARD_DIM` squares, top row first.
    pub squares: Vec<Vec<Square>>,
}

impl BoardLayout {
    /// Reads a layout as MAGPIE's loader (`board_layout.c`) does, and refuses
    /// what it refuses -- stricter than it only on parser quirks: the file
    /// split on newlines with empty lines ignored, a line's trailing `\r`
    /// dropped; a start square `row, col` inside the board; then exactly
    /// `BOARD_DIM` rows of `BOARD_DIM` bonus squares. The error says why.
    pub fn parse(content: &[u8]) -> Result<Self, String> {
        let text = String::from_utf8_lossy(content);
        let lines: Vec<&str> = text
            .split('\n')
            .filter(|line| !line.is_empty())
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .collect();
        if lines.len() != MAGPIE_BOARD_DIM + 1 {
            return Err(format!(
                "has {} rows; every MAGPIE build the fleet runs plays on {MAGPIE_BOARD_DIM}x{MAGPIE_BOARD_DIM}",
                lines.len().saturating_sub(1)
            ));
        }
        let coords: Vec<Option<usize>> = lines[0]
            .split(',')
            .filter(|part| !part.is_empty())
            .map(|part| {
                part.trim_matches([' ', '\t', '\n', '\r'])
                    .parse::<i64>()
                    .ok()
                    .filter(|v| (0..MAGPIE_BOARD_DIM as i64).contains(v))
                    .map(|v| v as usize)
            })
            .collect();
        let start = match coords[..] {
            [Some(row), Some(col)] => [row, col],
            _ => return Err(format!("has a start square MAGPIE cannot read: {:?}", lines[0])),
        };
        let mut squares = Vec::with_capacity(MAGPIE_BOARD_DIM);
        for (row, line) in lines[1..].iter().enumerate() {
            // Bytes, as MAGPIE counts them (a non-UTF-8 byte reads as three
            // here, after the lossy decode, and is refused either way).
            if line.len() != MAGPIE_BOARD_DIM {
                return Err(format!(
                    "row {} is {} squares wide, not {MAGPIE_BOARD_DIM}",
                    row + 1,
                    line.len()
                ));
            }
            let parsed: Result<Vec<Square>, char> =
                line.chars().map(|c| Square::from_symbol(c).ok_or(c)).collect();
            match parsed {
                Ok(parsed) => squares.push(parsed),
                Err(square) => {
                    return Err(format!(
                        "row {} has a square MAGPIE does not know: {square:?}",
                        row + 1
                    ))
                }
            }
        }
        Ok(Self { start, squares })
    }
}

/// Why MAGPIE would refuse this board layout, if it would.
pub fn layout_problem(content: &[u8]) -> Option<String> {
    BoardLayout::parse(content).err()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every symbol MAGPIE knows is read as its square, and the start square
    /// as `[row, column]`.
    #[test]
    fn a_layout_is_read_square_by_square() {
        let standard15 = include_str!("../../fixtures/versions/20260101/layouts/standard15.txt");
        let layout = BoardLayout::parse(standard15.as_bytes()).unwrap();
        assert_eq!(layout.start, [7, 7]);
        assert_eq!(layout.squares.len(), MAGPIE_BOARD_DIM);
        assert_eq!(layout.squares[0][0], Square::TripleWord);
        assert_eq!(layout.squares[0][3], Square::DoubleLetter);
        assert_eq!(layout.squares[1][1], Square::DoubleWord);
        assert_eq!(layout.squares[1][5], Square::TripleLetter);
        assert_eq!(layout.squares[7][7], Square::DoubleWord);
        assert_eq!(layout.squares[0][1], Square::Normal);

        let mut rows: Vec<String> = standard15.lines().map(str::to_string).collect();
        rows[0] = "3, 11".into();
        rows[1].replace_range(0..3, "^~#");
        let other = BoardLayout::parse(rows.join("\n").as_bytes()).unwrap();
        assert_eq!(other.start, [3, 11]);
        assert_eq!(
            other.squares[0][..3],
            [Square::QuadrupleLetter, Square::QuadrupleWord, Square::Brick]
        );
    }
}
