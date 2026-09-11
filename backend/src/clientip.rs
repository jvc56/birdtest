//! Which address a request actually came from.
//!
//! Per-IP rate limits (registration, password reset, login, and identity
//! minting for workers that arrive with none) key on this, so it has to be the
//! contributor's address rather than the proxy's. Deployed, every request
//! reaches the backend through the ALB; locally, through Nginx. Keying on the
//! TCP peer in either case puts the whole site in one bucket -- ten
//! registrations an hour, for everyone.
//!
//! `X-Forwarded-For` is only as trustworthy as the hops that appended to it.
//! Each trusted proxy appends the address it received the connection from, so
//! with `n` trusted hops the client is the `n`th entry from the right, and
//! anything further left is whatever the client chose to send. With no trusted
//! hops configured the header is ignored entirely.

use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use axum::http::HeaderMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// The client address for `headers` arriving from `peer`, trusting
/// `trusted_hops` proxies in front of this process.
pub fn resolve(headers: &HeaderMap, peer: Option<IpAddr>, trusted_hops: usize) -> Option<IpAddr> {
    if trusted_hops == 0 {
        return peer;
    }
    // Repeated headers are one list, in order.
    let entries: Vec<&str> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect();
    if entries.len() < trusted_hops {
        // Fewer hops than configured: the request did not come through the
        // proxies at all, so the header says nothing trustworthy.
        return peer;
    }
    entries[entries.len() - trusted_hops].parse().ok().or(peer)
}

/// The resolved client address. Never rejects: a request with no connection
/// info (an in-process test) resolves to the unspecified address, which is a
/// single shared bucket rather than a bypass.
pub struct ClientIp(pub IpAddr);

#[axum::async_trait]
impl FromRequestParts<AppState> for ClientIp {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip());
        Ok(ClientIp(
            resolve(&parts.headers, peer, state.cfg.trusted_proxy_hops)
                .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(values: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for value in values {
            map.append("x-forwarded-for", value.parse().unwrap());
        }
        map
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn without_trusted_hops_the_header_is_ignored() {
        let spoofed = headers(&["1.2.3.4"]);
        assert_eq!(resolve(&spoofed, Some(ip("10.0.0.9")), 0), Some(ip("10.0.0.9")));
    }

    #[test]
    fn one_trusted_hop_takes_the_rightmost_entry() {
        // The ALB appends the address it saw; anything to its left is the
        // client's own claim and must not be believed.
        let forwarded = headers(&["6.6.6.6, 203.0.113.7"]);
        assert_eq!(resolve(&forwarded, Some(ip("10.0.0.9")), 1), Some(ip("203.0.113.7")));
    }

    #[test]
    fn two_trusted_hops_skip_the_inner_proxy() {
        let forwarded = headers(&["6.6.6.6, 203.0.113.7", "10.0.1.5"]);
        assert_eq!(resolve(&forwarded, Some(ip("10.0.0.9")), 2), Some(ip("203.0.113.7")));
    }

    #[test]
    fn a_request_that_skipped_the_proxy_falls_back_to_the_peer() {
        assert_eq!(resolve(&HeaderMap::new(), Some(ip("10.0.0.9")), 1), Some(ip("10.0.0.9")));
        assert_eq!(resolve(&headers(&["garbage"]), Some(ip("10.0.0.9")), 1), Some(ip("10.0.0.9")));
    }
}
