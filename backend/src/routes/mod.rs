pub mod account;
pub mod admin;
pub mod auth;
pub mod public;
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
    (per_page, page * per_page)
}
