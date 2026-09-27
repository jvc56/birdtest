pub mod account;
pub mod admin;
pub mod auth;
pub mod public;
pub mod ratings;
pub mod worker;

use serde::Serialize;

#[derive(Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// `-1` where computing an exact count would cost more than it is worth for
    /// the caller (the per-job result feeds, which are effectively unbounded).
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

const DEFAULT_PER_PAGE: i64 = 50;
const MAX_PER_PAGE: i64 = 500;

pub fn paginate(page: i64, per_page: Option<i64>) -> (i64, i64) {
    let per_page = per_page.unwrap_or(DEFAULT_PER_PAGE).clamp(1, MAX_PER_PAGE);
    let page = page.max(0);
    // Saturating: `?page=` is the caller's, and `page * per_page` past
    // i64::MAX wrapped to a negative OFFSET, which Postgres refuses -- a 500
    // for a request that is merely past the end.
    (per_page, page.saturating_mul(per_page))
}

/// A page addressed by a cursor rather than an offset.
///
/// The documented pagination convention is `?page=` and
/// `{items, total, page, per_page}`, and every list endpoint follows it except
/// this one. `GET /api/jobs/:id/results` reads a job's whole corpus — millions
/// of rows for a full opening-rack job — where `OFFSET` produces and discards
/// every row before the page asked for, so page *N* costs *N* pages. It already
/// deviates by returning `total = -1`, which is why the exception is made here
/// and nowhere else: the other lists are bounded by things that do not grow
/// like a job's results do.
#[derive(Serialize)]
pub struct CursorPage<T> {
    pub items: Vec<T>,
    /// Always `-1`: an exact count of a job's results costs more than it is
    /// worth to the caller.
    pub total: i64,
    pub per_page: i64,
    /// Pass back as `?cursor=` for the next page. Absent on the last one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// The separator inside a cursor. A unit separator cannot occur in a rack, a
/// UUID or a timestamp, so no value needs escaping.
const CURSOR_SEPARATOR: char = '\u{1f}';

/// Opaque on the wire: hex rather than the raw parts, so nobody builds one by
/// hand and depends on its shape.
pub fn encode_cursor(parts: &[String]) -> String {
    hex::encode(parts.join(&CURSOR_SEPARATOR.to_string()))
}

/// `None` for anything that is not a cursor this server produced. A bad cursor
/// reads as "start from the beginning" rather than as an error: it is an opaque
/// token, so a caller cannot be expected to fix one.
pub fn decode_cursor(raw: &str) -> Option<Vec<String>> {
    let bytes = hex::decode(raw).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    Some(text.split(CURSOR_SEPARATOR).map(str::to_string).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_past_the_end_of_i64_is_past_the_end() {
        assert_eq!(paginate(2, Some(50)), (50, 100));
        assert_eq!(paginate(-3, None), (DEFAULT_PER_PAGE, 0));
        assert_eq!(paginate(400_000_000_000_000_000, Some(500)), (500, i64::MAX));
    }
}
