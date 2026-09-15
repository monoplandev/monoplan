//! Best-effort client IP for `devices.last_seen_ip`.
//!
//! In production the server sits behind Caddy on loopback, so the socket
//! peer is always 127.0.0.1 and the real address arrives in
//! `X-Forwarded-For` (Caddy's `reverse_proxy` sets it by default). The
//! header is trusted unconditionally: the only row it can influence is
//! the caller's own device row, so a spoofed value hides nothing from
//! anyone but the spoofer.

use std::net::{IpAddr, SocketAddr};

use axum::extract::ConnectInfo;
use axum::http::{Extensions, HeaderMap};

/// Leftmost `X-Forwarded-For` entry, else `X-Real-IP`, else the TCP peer
/// recorded by `into_make_service_with_connect_info`. `None` when none
/// of those parse as an IP (e.g. `tower::oneshot` in tests).
pub fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>) -> Option<String> {
    forwarded_for(headers)
        .or_else(|| header_ip(headers, "x-real-ip"))
        .or(peer)
        .map(|ip| ip.to_string())
}

/// TCP peer from request extensions, when the server was started with
/// connect info (absent under `tower::oneshot`).
pub fn peer_ip(extensions: &Extensions) -> Option<IpAddr> {
    extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip())
}

fn forwarded_for(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get("x-forwarded-for")?
        .to_str()
        .ok()?
        .split(',')
        .next()?
        .trim()
        .parse()
        .ok()
}

fn header_ip(headers: &HeaderMap, name: &str) -> Option<IpAddr> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn prefers_leftmost_forwarded_for() {
        let h = headers(&[("x-forwarded-for", "203.0.113.9, 10.0.0.1")]);
        assert_eq!(client_ip(&h, None).as_deref(), Some("203.0.113.9"));
    }

    #[test]
    fn falls_back_to_connect_info() {
        let mut ext = Extensions::new();
        ext.insert(ConnectInfo::<SocketAddr>("[::1]:4000".parse().unwrap()));
        let peer = peer_ip(&ext);
        assert_eq!(client_ip(&HeaderMap::new(), peer).as_deref(), Some("::1"));
    }

    #[test]
    fn garbage_header_is_ignored() {
        let peer: IpAddr = "127.0.0.1".parse().unwrap();
        let h = headers(&[("x-forwarded-for", "not-an-ip")]);
        assert_eq!(client_ip(&h, Some(peer)).as_deref(), Some("127.0.0.1"));
    }

    #[test]
    fn nothing_available() {
        assert_eq!(client_ip(&HeaderMap::new(), None), None);
        assert_eq!(peer_ip(&Extensions::new()), None);
    }
}
