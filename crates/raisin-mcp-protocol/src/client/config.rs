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

//! Operator-owned MCP client settings — the TOML `[mcp_client]` section.
//!
//! Defined here, next to the policy it configures, rather than mirrored into
//! `raisin-server`'s config module. `TriggerSafetyConfig` is mirrored there
//! because its owning crate sits behind an optional feature; `raisin-mcp` does
//! not, so there is no reason to keep two copies of these four fields in step
//! by hand.
//!
//! Everything here is a *deployment* concern (where may we connect, how much
//! may we buffer). Per-connection settings live on the `raisin:McpConnection`
//! node instead, so adding a connection never needs a server restart.

use serde::{Deserialize, Serialize};

use super::egress::EgressPolicy;
use super::transport::DEFAULT_MAX_RESPONSE_BYTES;

/// Default per-request timeout when a connection does not override it.
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// The `[mcp_client]` TOML section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpClientConfig {
    /// Hosts the client may reach; empty means any PUBLIC host.
    ///
    /// Empty is not "any host at all" — the private-address check still
    /// applies. An allowlist narrows further, to named third parties.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,

    /// Permit loopback and private addresses. Off by default.
    ///
    /// Turning this on in a deployment that accepts operator-supplied
    /// connection URLs makes the server a proxy into its own private network,
    /// so it is opt-in and intended for local development or a sidecar MCP
    /// server on a known address.
    #[serde(default)]
    pub allow_private_addresses: bool,

    /// Cap on a single response body.
    #[serde(default = "default_max_response_bytes")]
    pub max_response_bytes: usize,

    /// Per-request timeout when the connection does not set `call_timeout_ms`.
    #[serde(default = "default_timeout_ms")]
    pub default_timeout_ms: u64,
}

fn default_max_response_bytes() -> usize {
    DEFAULT_MAX_RESPONSE_BYTES
}

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

impl Default for McpClientConfig {
    fn default() -> Self {
        Self {
            allowed_hosts: Vec::new(),
            allow_private_addresses: false,
            max_response_bytes: default_max_response_bytes(),
            default_timeout_ms: default_timeout_ms(),
        }
    }
}

impl McpClientConfig {
    /// The egress rules this configuration implies.
    pub fn egress_policy(&self) -> EgressPolicy {
        EgressPolicy {
            allowed_hosts: self.allowed_hosts.clone(),
            allow_private_addresses: self.allow_private_addresses,
        }
    }
}

/// The operator's `[mcp_client]` settings for this process.
///
/// Deliberately a process global rather than a field threaded through every
/// caller. The value comes from the server's TOML, never from a request, and
/// the paths that need it — the HTTP handlers, the discovery job, the token
/// refresh sweep, the function executor — live in crates that cannot all reach
/// one another (`raisin-rocksdb` cannot depend on `raisin-functions`). Threading
/// it produced four copies of the same value, and a path that missed one would
/// silently run under the permissive default.
static INSTALLED: std::sync::OnceLock<McpClientConfig> = std::sync::OnceLock::new();

/// Install the operator's settings. Called once from the server binary; later
/// calls are ignored so a test cannot be surprised by a mid-run change.
pub fn install_config(config: McpClientConfig) {
    let _ = INSTALLED.set(config);
}

/// The installed settings, or the safe default.
///
/// The default refuses loopback and private addresses, so a deployment that
/// never installs a config cannot be talked into dialling its own network.
pub fn installed_config() -> &'static McpClientConfig {
    INSTALLED.get_or_init(McpClientConfig::default)
}

/// The installed egress policy. The single source every dial path reads.
pub fn installed_egress_policy() -> EgressPolicy {
    installed_config().egress_policy()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_section_is_safe_by_default() {
        let config: McpClientConfig = toml::from_str("").expect("empty section must parse");
        assert!(
            !config.allow_private_addresses,
            "private egress must never be on unless asked for"
        );
        assert!(config.allowed_hosts.is_empty());
        assert_eq!(config.max_response_bytes, DEFAULT_MAX_RESPONSE_BYTES);
    }

    #[test]
    fn parses_a_populated_section() {
        let config: McpClientConfig = toml::from_str(
            r#"
            allowed_hosts = ["mcp.linear.app", "*.example.com"]
            allow_private_addresses = true
            max_response_bytes = 1024
            default_timeout_ms = 5000
        "#,
        )
        .expect("populated section must parse");

        assert_eq!(config.allowed_hosts.len(), 2);
        assert_eq!(config.max_response_bytes, 1024);
        assert_eq!(config.default_timeout_ms, 5000);

        let policy = config.egress_policy();
        assert!(policy.allow_private_addresses);
        assert_eq!(policy.allowed_hosts.len(), 2);
    }
}
