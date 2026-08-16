// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Where the MCP client is allowed to connect.
//!
//! An MCP connection URL is operator-supplied and then dialled by the server
//! itself, which is the textbook SSRF shape: `http://169.254.169.254/…` reaches
//! cloud instance metadata, and `http://10.0.0.5/` reaches anything on the
//! private network the database sits in.
//!
//! Deliberately NOT routed through a function's `network_policy`
//! (`raisin:Function.network_policy`): that policy is authored by whoever wrote
//! the function, and a generated proxy has no author to trust. This policy is
//! operator-owned, set in the server's `[mcp_client]` config.
//!
//! Checked TWICE — once when a connection is saved, once against the resolved
//! addresses just before dialling. The second check is what a save-time-only
//! guard misses: a hostname that resolved publicly at save time can be
//! re-pointed at `127.0.0.1` afterwards (DNS rebinding). A residual race
//! remains between our resolution and the one `reqwest` performs; closing it
//! entirely needs a pinning custom resolver, which is noted as follow-up work
//! rather than pretended away here.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

use super::error::{RemoteToolError, Result};

/// Operator-owned egress rules, from the server's `[mcp_client]` config.
#[derive(Debug, Clone, Default)]
pub struct EgressPolicy {
    /// Hosts the client may reach. Empty = any public host.
    ///
    /// An entry may be an exact host (`mcp.linear.app`) or a wildcard suffix
    /// (`*.example.com`, which matches sub-domains but NOT `example.com`).
    pub allowed_hosts: Vec<String>,
    /// Permit loopback and private addresses. Off by default; intended for
    /// local development and for deployments running an MCP server as a sidecar.
    pub allow_private_addresses: bool,
}

impl EgressPolicy {
    /// Validate a URL's scheme and host at save time.
    pub fn validate_url(&self, url: &Url) -> Result<()> {
        let host = url
            .host()
            .ok_or_else(|| RemoteToolError::Config("endpoint URL has no host".into()))?;

        match url.scheme() {
            "https" => {}
            // Plain http is only ever acceptable to a local sidecar, and only
            // when the operator has already opted into private addresses.
            "http" if self.allow_private_addresses => {}
            "http" => {
                return Err(RemoteToolError::Config(
                    "endpoint URL must use https (http is allowed only when \
                     [mcp_client] allow_private_addresses is enabled)"
                        .into(),
                ))
            }
            other => {
                return Err(RemoteToolError::Config(format!(
                    "unsupported endpoint scheme `{other}`; MCP connections are Streamable HTTP"
                )))
            }
        }

        let host_str = match &host {
            Host::Domain(domain) => domain.to_string(),
            Host::Ipv4(addr) => addr.to_string(),
            Host::Ipv6(addr) => addr.to_string(),
        };
        self.check_host_allowed(&host_str)?;

        // A literal IP in the URL can be judged immediately; a domain must wait
        // for resolution.
        match host {
            Host::Ipv4(addr) => self.check_address(IpAddr::V4(addr)),
            Host::Ipv6(addr) => self.check_address(IpAddr::V6(addr)),
            Host::Domain(_) => Ok(()),
        }
    }

    /// Validate a URL **and** the addresses its host resolves to, immediately
    /// before dialling it.
    ///
    /// This is the check every outbound request must go through. [`validate_url`]
    /// alone judges the scheme, the allowlist and a literal IP — it cannot see
    /// that `mcp.example.com` currently resolves to `127.0.0.1`, which is the
    /// whole DNS-rebinding shape.
    ///
    /// A residual race remains between this resolution and the one `reqwest`
    /// performs; closing it entirely needs a connection-pinning custom resolver,
    /// which is follow-up work rather than something pretended away here.
    ///
    /// [`validate_url`]: Self::validate_url
    pub async fn guard(&self, url: &Url) -> Result<()> {
        self.validate_url(url)?;
        self.guard_addresses(url).await
    }

    /// The ADDRESS half of [`guard`], without the scheme rule or the host
    /// allowlist.
    ///
    /// Split out so a second caller can reuse the SSRF check without inheriting
    /// MCP's other policy. `raisin.http.fetch` is that caller: a function may
    /// legitimately call plain `http://`, and its permitted hosts are already
    /// decided by the function's own `network_policy`, but it must still never
    /// be able to reach loopback, RFC1918 or link-local — `169.254.169.254` is
    /// cloud instance metadata, and adapters run privileged with `raisin.secrets`
    /// in reach.
    ///
    /// Keeping this in one place is deliberate. The codebase already carries two
    /// `is_url_allowed` implementations that drifted into disagreeing (see the
    /// tracking issue); a second copy of "is this address public" would drift the
    /// same way, and the failure mode would be an SSRF hole rather than a
    /// confusing error.
    ///
    /// [`guard`]: Self::guard
    pub async fn guard_addresses(&self, url: &Url) -> Result<()> {
        match url.host() {
            // A literal IP can be judged immediately — no DNS round trip for an
            // address that is already in the URL.
            Some(Host::Ipv4(addr)) => {
                self.validate_resolved(&addr.to_string(), &[IpAddr::V4(addr)])
            }
            Some(Host::Ipv6(addr)) => {
                self.validate_resolved(&addr.to_string(), &[IpAddr::V6(addr)])
            }
            Some(Host::Domain(domain)) => {
                let port = url.port_or_known_default().unwrap_or(443);
                let addresses: Vec<IpAddr> = tokio::net::lookup_host((domain, port))
                    .await
                    .map_err(|e| {
                        RemoteToolError::Transient(format!("could not resolve `{domain}`: {e}"))
                    })?
                    .map(|socket| socket.ip())
                    .collect();
                self.validate_resolved(domain, &addresses)
            }
            None => Err(RemoteToolError::Config("URL has no host".into())),
        }
    }

    /// Validate the addresses a host actually resolved to, just before dialling.
    pub fn validate_resolved(&self, host: &str, addresses: &[IpAddr]) -> Result<()> {
        if addresses.is_empty() {
            return Err(RemoteToolError::Transient(format!(
                "`{host}` did not resolve to any address"
            )));
        }
        // EVERY address must pass, not merely one: a host that returns both a
        // public and a private address would otherwise be dialled on whichever
        // the connector picks.
        for address in addresses {
            self.check_address(*address)?;
        }
        Ok(())
    }

    /// Whether the host matches the allowlist (empty allowlist permits all).
    fn check_host_allowed(&self, host: &str) -> Result<()> {
        if self.allowed_hosts.is_empty() {
            return Ok(());
        }
        let host = host.to_ascii_lowercase();
        let permitted = self.allowed_hosts.iter().any(|entry| {
            let entry = entry.trim().to_ascii_lowercase();
            match entry.strip_prefix("*.") {
                Some(suffix) => host.ends_with(&format!(".{suffix}")),
                None => host == entry,
            }
        });
        if permitted {
            Ok(())
        } else {
            Err(RemoteToolError::Config(format!(
                "host `{host}` is not in the [mcp_client] allowed_hosts list"
            )))
        }
    }

    /// Reject an address that is not publicly routable.
    fn check_address(&self, address: IpAddr) -> Result<()> {
        if self.allow_private_addresses || is_public(address) {
            return Ok(());
        }
        Err(RemoteToolError::Config(format!(
            "endpoint resolves to non-public address {address}; enable \
             [mcp_client] allow_private_addresses to permit it"
        )))
    }
}

/// Whether an address is publicly routable.
///
/// Written out rather than using the unstable `is_global()` so it compiles on
/// stable and so each excluded range is visible and reviewable.
fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(addr: Ipv4Addr) -> bool {
    let [a, b, ..] = addr.octets();
    !(addr.is_private()          // 10/8, 172.16/12, 192.168/16
        || addr.is_loopback()    // 127/8
        || addr.is_link_local()  // 169.254/16 — includes cloud metadata
        || addr.is_broadcast()
        || addr.is_documentation()
        || addr.is_unspecified() // 0.0.0.0
        || a == 0
        || (a == 100 && (64..128).contains(&b)) // 100.64/10 carrier-grade NAT
        || (a == 192 && b == 0)  // 192.0.0/24 IETF protocol assignments
        || (a == 198 && (18..20).contains(&b))  // 198.18/15 benchmarking
        || a >= 224) // multicast + reserved
}

fn is_public_v6(addr: Ipv6Addr) -> bool {
    let first = addr.segments()[0];
    !(addr.is_loopback()
        || addr.is_unspecified()
        || (first & 0xfe00) == 0xfc00   // fc00::/7 unique local
        || (first & 0xffc0) == 0xfe80   // fe80::/10 link local
        || (first & 0xff00) == 0xff00   // ff00::/8 multicast
        // An IPv4-mapped address must be judged as the IPv4 address it carries,
        // or ::ffff:127.0.0.1 walks straight past every check above.
        || addr.to_ipv4_mapped().is_some_and(|v4| !is_public_v4(v4)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> EgressPolicy {
        EgressPolicy::default()
    }

    #[test]
    fn requires_https_by_default() {
        let url = Url::parse("http://mcp.example.com/mcp").unwrap();
        assert!(open().validate_url(&url).is_err());
    }

    #[test]
    fn allows_https() {
        let url = Url::parse("https://mcp.example.com/mcp").unwrap();
        assert!(open().validate_url(&url).is_ok());
    }

    #[test]
    fn rejects_non_http_schemes() {
        for raw in ["file:///etc/passwd", "ftp://example.com", "gopher://x.y"] {
            let url = Url::parse(raw).unwrap();
            assert!(open().validate_url(&url).is_err(), "{raw} must be rejected");
        }
    }

    #[test]
    fn rejects_literal_private_addresses() {
        for raw in [
            "https://127.0.0.1/mcp",
            "https://10.0.0.5/mcp",
            "https://192.168.1.1/mcp",
            "https://172.16.0.1/mcp",
            // The one that reaches cloud instance metadata.
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/mcp",
            "https://[fd00::1]/mcp",
        ] {
            let url = Url::parse(raw).unwrap();
            assert!(open().validate_url(&url).is_err(), "{raw} must be rejected");
        }
    }

    #[test]
    fn ipv4_mapped_ipv6_does_not_bypass_the_check() {
        // ::ffff:127.0.0.1 is loopback wearing an IPv6 hat.
        let url = Url::parse("https://[::ffff:127.0.0.1]/mcp").unwrap();
        assert!(open().validate_url(&url).is_err());
    }

    #[test]
    fn private_addresses_pass_when_explicitly_enabled() {
        let policy = EgressPolicy {
            allow_private_addresses: true,
            ..EgressPolicy::default()
        };
        assert!(policy
            .validate_url(&Url::parse("http://127.0.0.1:3000/mcp").unwrap())
            .is_ok());
    }

    #[test]
    fn allowlist_matches_exactly_or_by_wildcard_suffix() {
        let policy = EgressPolicy {
            allowed_hosts: vec!["mcp.linear.app".into(), "*.example.com".into()],
            ..EgressPolicy::default()
        };
        let ok = |u: &str| policy.validate_url(&Url::parse(u).unwrap()).is_ok();

        assert!(ok("https://mcp.linear.app/mcp"));
        assert!(ok("https://a.example.com/mcp"));
        assert!(ok("https://deep.a.example.com/mcp"));
        // A wildcard must not match the bare apex, and must not match a host
        // that merely ENDS with the string.
        assert!(!ok("https://example.com/mcp"));
        assert!(!ok("https://notexample.com/mcp"));
        assert!(!ok("https://evil.com/mcp"));
    }

    #[test]
    fn resolution_check_rejects_if_any_address_is_private() {
        let policy = open();
        let public: IpAddr = "93.184.216.34".parse().unwrap();
        let private: IpAddr = "10.1.2.3".parse().unwrap();

        assert!(policy.validate_resolved("x.test", &[public]).is_ok());
        // A split-horizon answer must not be dialled just because one of the
        // addresses happened to be fine.
        assert!(policy
            .validate_resolved("x.test", &[public, private])
            .is_err());
        assert!(policy.validate_resolved("x.test", &[]).is_err());
    }

    /// `guard` must not be a thin alias for `validate_url`: it has to reject a
    /// hostname that RESOLVES somewhere private, which is the rebinding shape
    /// the save-time check cannot see. `localhost` is the one name every
    /// machine resolves to loopback.
    #[tokio::test]
    async fn guard_rejects_a_hostname_that_resolves_to_loopback() {
        let url = Url::parse("https://localhost/mcp").unwrap();
        // The URL itself is unremarkable — a domain, over https.
        assert!(open().validate_url(&url).is_ok());
        // Resolving it is what exposes the target.
        assert!(open().guard(&url).await.is_err());
    }

    #[tokio::test]
    async fn guard_still_rejects_a_literal_metadata_address_without_resolving() {
        let url = Url::parse("https://169.254.169.254/latest/meta-data/").unwrap();
        assert!(open().guard(&url).await.is_err());
    }

    #[test]
    fn carrier_grade_nat_and_benchmark_ranges_are_not_public() {
        assert!(!is_public("100.64.0.1".parse().unwrap()));
        assert!(!is_public("198.18.0.1".parse().unwrap()));
        assert!(is_public("100.128.0.1".parse().unwrap()));
    }
}
