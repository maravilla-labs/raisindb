// SPDX-License-Identifier: BSL-1.1

//! Anonymous visitor chat over the WebSocket.
//!
//! A public website runs an agent chat straight from the browser: no site
//! server endpoint, no service identity. The pieces:
//!
//! * [`session`] — the session secret the browser keeps, and the home key it
//!   names (`/visitors/<key>` in `raisin:access_control`).
//! * [`config`] — the agent's `anonymous` settings. An agent without the flag
//!   refuses visitors.
//! * [`limits`] — per-session and per-IP rate limits, and the turn lease.
//! * [`daily`] — the limits that span a day and survive a restart: messages
//!   and new sessions per IP, and the agent's daily token budget.
//! * `crate::handlers::visitor_chat` — the two requests (`visitor_chat_start`,
//!   `visitor_chat_send`) and the event forwarder that streams a conversation's
//!   events to the one connection that owns it.
//!
//! Row-level security does the rest (`raisin_models::auth::visitor`): the
//! bound session may read its own home, nobody else may read it, and nobody
//! but the server writes it.

pub mod config;
pub mod daily;
pub mod limits;
pub mod session;

pub use config::AnonymousChatConfig;
pub use limits::VisitorLimits;

use std::collections::HashSet;

/// What a connection is bound to once it has started a visitor chat.
#[derive(Debug, Clone)]
pub struct VisitorBinding {
    /// The session key (`/visitors/<key>`).
    pub key: String,
    /// The session home path.
    pub home: String,
    /// The agent this session talks to (`/agents/<name>`).
    pub agent_path: String,
    /// Conversations this connection may send to and receive from.
    pub conversations: HashSet<String>,
    /// Conversations whose events are already being forwarded.
    pub forwarding: HashSet<String>,
}

/// The client IP of an upgrade request, best effort: the LAST
/// `X-Forwarded-For` entry (the one the nearest proxy appended), else
/// `X-Real-IP`. Used only to key rate limits.
pub fn client_ip(headers: &axum::http::HeaderMap) -> Option<String> {
    let forwarded = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit(',').next())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    forwarded
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|v| !v.is_empty())
        })
        .map(|v| v.chars().take(64).collect())
}

/// The proxies whose `X-Forwarded-For` entries this server believes: IP
/// addresses or CIDR blocks from `RAISINDB_TRUSTED_PROXIES` (comma separated,
/// e.g. `127.0.0.1,::1`). Empty (the default) trusts no header at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedProxies(Vec<(std::net::IpAddr, u8)>);

impl TrustedProxies {
    pub fn parse(list: &str) -> Self {
        let mut v = Vec::new();
        for item in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (addr, len) = match item.split_once('/') {
                Some((a, l)) => (a, l.parse::<u8>().ok()),
                None => (item, None),
            };
            let Ok(ip) = addr.parse::<std::net::IpAddr>() else {
                continue;
            };
            let ip = ip.to_canonical();
            let max = if ip.is_ipv4() { 32 } else { 128 };
            let len = len.unwrap_or(max);
            if len <= max {
                v.push((ip, len));
            }
        }
        Self(v)
    }

    pub fn from_env() -> Self {
        Self::parse(&std::env::var("RAISINDB_TRUSTED_PROXIES").unwrap_or_default())
    }

    fn contains(&self, ip: std::net::IpAddr) -> bool {
        use std::net::IpAddr::{V4, V6};
        let ip = ip.to_canonical();
        self.0.iter().any(|(net, len)| match (net, ip) {
            (V4(n), V4(i)) => prefix_eq(&n.octets(), &i.octets(), *len),
            (V6(n), V6(i)) => prefix_eq(&n.octets(), &i.octets(), *len),
            _ => false,
        })
    }
}

fn prefix_eq(a: &[u8], b: &[u8], bits: u8) -> bool {
    let full = (bits / 8) as usize;
    if a[..full] != b[..full] {
        return false;
    }
    let rem = bits % 8;
    rem == 0 || (a[full] ^ b[full]) >> (8 - rem) == 0
}

/// The client address a function may rely on (`context.caller.client_ip`).
///
/// Evidence-grade, so headers are believed only through a configured chain:
/// the TCP peer is the starting point and is the answer unless it is a
/// trusted proxy. Only then are the `X-Forwarded-For` entries read, from the
/// right, skipping further trusted proxies; the first address that is not one
/// is the client. An entry that is not an IP address ends the walk with
/// nothing (never a guess). `X-Real-IP` and a client-sent `X-Forwarded-For`
/// from an untrusted peer are ignored. Loopback is never reported: it names
/// the proxy, not a person.
pub fn verified_client_ip(
    peer: Option<std::net::IpAddr>,
    headers: &axum::http::HeaderMap,
    proxies: &TrustedProxies,
) -> Option<String> {
    let peer = peer?.to_canonical();
    let mut client = peer;
    if proxies.contains(peer) {
        let joined = headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>()
            .join(",");
        for hop in joined.rsplit(',').map(str::trim).filter(|h| !h.is_empty()) {
            let ip = hop.parse::<std::net::IpAddr>().ok()?.to_canonical();
            client = ip;
            if !proxies.contains(ip) {
                break;
            }
        }
    }
    if client.is_loopback() || client.is_unspecified() {
        return None;
    }
    Some(client.to_string())
}

/// The `Origin` of an upgrade request.
pub fn origin(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.chars().take(256).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn the_nearest_proxys_entry_keys_the_rate_limit() {
        let mut h = HeaderMap::new();
        h.insert(
            "x-forwarded-for",
            HeaderValue::from_static("1.1.1.1, 10.0.0.9"),
        );
        assert_eq!(client_ip(&h).as_deref(), Some("10.0.0.9"));
        let mut h = HeaderMap::new();
        h.insert("x-real-ip", HeaderValue::from_static("2.2.2.2"));
        assert_eq!(client_ip(&h).as_deref(), Some("2.2.2.2"));
        assert_eq!(client_ip(&HeaderMap::new()), None);
    }

    fn hdr(xff: &[&str], real: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        for v in xff {
            h.append("x-forwarded-for", HeaderValue::from_str(v).unwrap());
        }
        if let Some(r) = real {
            h.insert("x-real-ip", HeaderValue::from_str(r).unwrap());
        }
        h
    }

    fn ip(s: &str) -> Option<std::net::IpAddr> {
        s.parse().ok()
    }

    #[test]
    fn a_peer_that_is_not_a_trusted_proxy_cannot_choose_its_address() {
        let proxies = TrustedProxies::parse("127.0.0.1,::1");
        let forged = hdr(&["6.6.6.6"], Some("7.7.7.7"));
        // direct client: its own socket address, headers ignored
        assert_eq!(
            verified_client_ip(ip("203.0.113.7"), &forged, &proxies).as_deref(),
            Some("203.0.113.7")
        );
        // no proxy list configured at all: headers are never read
        assert_eq!(
            verified_client_ip(
                ip("127.0.0.1"),
                &hdr(&["203.0.113.7"], None),
                &TrustedProxies::default()
            ),
            None
        );
        assert_eq!(
            verified_client_ip(ip("198.51.100.4"), &forged, &TrustedProxies::default()).as_deref(),
            Some("198.51.100.4")
        );
    }

    #[test]
    fn behind_a_trusted_proxy_the_forwarded_chain_is_walked_from_the_right() {
        let proxies = TrustedProxies::parse("127.0.0.1, 10.0.0.0/8");
        // proxy overwrote the header: one entry
        assert_eq!(
            verified_client_ip(ip("127.0.0.1"), &hdr(&["203.0.113.7"], None), &proxies).as_deref(),
            Some("203.0.113.7")
        );
        // a client-typed leading entry is left of what the proxy observed
        assert_eq!(
            verified_client_ip(
                ip("127.0.0.1"),
                &hdr(&["6.6.6.6, 203.0.113.7"], None),
                &proxies
            )
            .as_deref(),
            Some("203.0.113.7")
        );
        // trusted hops are skipped, repeated header lines are joined in order
        assert_eq!(
            verified_client_ip(
                ip("127.0.0.1"),
                &hdr(&["6.6.6.6", "203.0.113.7, 10.1.2.3"], Some("7.7.7.7")),
                &proxies
            )
            .as_deref(),
            Some("203.0.113.7")
        );
        // garbage where an address must be: nothing, never a guess
        assert_eq!(
            verified_client_ip(
                ip("127.0.0.1"),
                &hdr(&["203.0.113.7, <script>"], None),
                &proxies
            ),
            None
        );
        // X-Real-IP alone is not enough, and a loopback client is not reported
        assert_eq!(
            verified_client_ip(ip("127.0.0.1"), &hdr(&[], Some("7.7.7.7")), &proxies),
            None
        );
        // v4-mapped peer still matches the list
        assert_eq!(
            verified_client_ip(
                ip("::ffff:127.0.0.1"),
                &hdr(&["203.0.113.7"], None),
                &proxies
            )
            .as_deref(),
            Some("203.0.113.7")
        );
        assert_eq!(
            verified_client_ip(None, &hdr(&["203.0.113.7"], None), &proxies),
            None
        );
    }

    #[test]
    fn trusted_proxy_lists_parse_cidr_and_skip_junk() {
        let p = TrustedProxies::parse("10.0.0.0/8, nonsense, 192.168.1.1/40, ::1");
        assert!(p.contains("10.9.9.9".parse().unwrap()));
        assert!(!p.contains("11.0.0.1".parse().unwrap()));
        assert!(!p.contains("192.168.1.1".parse().unwrap()));
        assert!(p.contains("::1".parse().unwrap()));
    }
}
