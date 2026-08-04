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

//! The parsed `raisin:McpConnection` node: one remote MCP server's configuration.
//!
//! The outbound mirror of [`crate::server::McpServerDescriptor`], and written to
//! the same contract: property names match the `raisin:McpConnection` NodeType
//! verbatim, and parsing is forgiving about optional keys so one malformed
//! field does not take a whole connection offline.
//!
//! Secrets are NOT decrypted here. This type carries the ciphertext blobs
//! opaquely; only the HTTP layer holds the master key. Keeping the decrypt out
//! of the parser means no code path can accidentally serialize a descriptor and
//! leak a token — [`Self::redacted`] is the only way out, and it drops them.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;

use raisin_models::nodes::Node;

use super::error::{RemoteToolError, Result};
use super::transport::ExtraHeaders;
use crate::props::{bool_prop, non_empty_str_prop, obj_prop, str_array_prop, str_prop, u64_prop};

/// Default per-call timeout when the connection does not set one.
pub const DEFAULT_CALL_TIMEOUT_MS: u64 = 30_000;

/// Upper bound on a per-call timeout, so one connection cannot pin a job worker.
pub const MAX_CALL_TIMEOUT_MS: u64 = 120_000;

/// How RaisinDB authenticates to a remote server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthKind {
    /// No credential.
    None,
    /// A static bearer token or custom header.
    Static,
    /// OAuth 2.1 service account.
    Oauth,
}

impl AuthKind {
    fn parse(raw: Option<&str>) -> Self {
        match raw.unwrap_or("none").to_ascii_lowercase().as_str() {
            "static" => Self::Static,
            "oauth" => Self::Oauth,
            _ => Self::None,
        }
    }
}

/// Where a static credential is placed on the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StaticAuthScheme {
    /// `Authorization: Bearer <value>`.
    Bearer,
    /// A named header carrying the raw value, e.g. `X-Api-Key`.
    Header {
        /// Header name.
        name: String,
    },
}

impl StaticAuthScheme {
    /// Parse the non-secret `static_auth` block.
    fn parse(props: &Value) -> Self {
        let block = obj_prop(props, "static_auth");
        match non_empty_str_prop(&block, "scheme")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "header" => Self::Header {
                name: non_empty_str_prop(&block, "header_name")
                    .unwrap_or_else(|| "Authorization".to_string()),
            },
            _ => Self::Bearer,
        }
    }

    /// Render the credential onto request headers.
    fn headers(&self, secret: &str) -> ExtraHeaders {
        match self {
            Self::Bearer => vec![("Authorization".to_string(), format!("Bearer {secret}"))],
            Self::Header { name } => vec![(name.clone(), secret.to_string())],
        }
    }
}

/// Which of a server's tools to expose.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolFilter {
    /// When non-empty, ONLY these remote names are exposed.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Remote names never exposed. Applied after `allow`.
    #[serde(default)]
    pub deny: Vec<String>,
}

impl ToolFilter {
    /// Whether a remote tool should be exposed as an enabled proxy.
    ///
    /// Deny wins over allow: an operator adding a name to `deny` expects it
    /// gone, whatever else the config says.
    pub fn permits(&self, remote_name: &str) -> bool {
        if self.deny.iter().any(|d| d == remote_name) {
            return false;
        }
        self.allow.is_empty() || self.allow.iter().any(|a| a == remote_name)
    }
}

/// When tool discovery runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshPolicy {
    /// `manual` = only on demand; `interval` = also on a timer.
    pub mode: String,
    /// Seconds between scheduled refreshes when `mode == "interval"`.
    pub interval_secs: u64,
    /// Whether saving the connection triggers a discovery run.
    pub on_save: bool,
    /// Per-call timeout for `tools/call`, clamped to [`MAX_CALL_TIMEOUT_MS`].
    pub call_timeout_ms: u64,
}

impl Default for RefreshPolicy {
    fn default() -> Self {
        Self {
            mode: "manual".to_string(),
            interval_secs: 3_600,
            on_save: true,
            call_timeout_ms: DEFAULT_CALL_TIMEOUT_MS,
        }
    }
}

impl RefreshPolicy {
    fn parse(props: &Value) -> Self {
        let block = obj_prop(props, "refresh_policy");
        let defaults = Self::default();
        if block.is_null() {
            return defaults;
        }
        Self {
            mode: non_empty_str_prop(&block, "mode").unwrap_or(defaults.mode),
            interval_secs: u64_prop(&block, "interval_secs")
                .filter(|s| *s > 0)
                .unwrap_or(defaults.interval_secs),
            on_save: bool_prop(&block, "on_save").unwrap_or(defaults.on_save),
            // Clamped, not rejected: a connection with an absurd timeout should
            // still work, just not hold a job worker for an hour.
            call_timeout_ms: u64_prop(&block, "call_timeout_ms")
                .filter(|ms| *ms > 0)
                .unwrap_or(defaults.call_timeout_ms)
                .min(MAX_CALL_TIMEOUT_MS),
        }
    }

    /// The per-call timeout as a [`Duration`].
    pub fn call_timeout(&self) -> Duration {
        Duration::from_millis(self.call_timeout_ms)
    }

    /// Whether discovery should also run on a timer.
    pub fn is_scheduled(&self) -> bool {
        self.mode.eq_ignore_ascii_case("interval")
    }
}

/// A parsed `raisin:McpConnection` descriptor.
#[derive(Debug, Clone)]
pub struct McpConnectionDescriptor {
    /// Human-readable title.
    pub title: String,
    /// URL-safe slug; namespaces every generated proxy's name and path.
    pub slug: String,
    /// Streamable HTTP endpoint.
    pub url: Url,
    /// Whether the connection is active.
    pub enabled: bool,
    /// Revision agreed on the last handshake.
    pub protocol_version: Option<String>,
    /// How to authenticate.
    pub auth_kind: AuthKind,
    /// Where a static credential goes.
    pub static_auth: StaticAuthScheme,
    /// Ciphertext of the static credential, still encrypted.
    pub credential_encrypted: Option<String>,
    /// Ciphertext of the OAuth token pair, still encrypted.
    pub tokens_encrypted: Option<String>,
    /// Access-token expiry, unix seconds.
    pub expires_at: Option<u64>,
    /// Non-secret OAuth client registration.
    pub oauth_client: Value,
    /// Which tools to expose.
    pub tool_filter: ToolFilter,
    /// When discovery runs.
    pub refresh_policy: RefreshPolicy,
    /// Names of tools discovered so far (full records stay on the node).
    pub discovered_tool_names: Vec<String>,
}

impl McpConnectionDescriptor {
    /// Parse a descriptor from a `raisin:McpConnection` [`Node`].
    pub fn from_node(node: &Node) -> Result<Self> {
        let props =
            serde_json::to_value(&node.properties).unwrap_or(Value::Object(Default::default()));
        Self::from_properties(&node.name, &props)
    }

    /// Parse from a node `name` and its raw `properties` object.
    ///
    /// The SQL-row path, where `properties` is the JSON column.
    pub fn from_properties(node_name: &str, props: &Value) -> Result<Self> {
        let slug = non_empty_str_prop(props, "slug").ok_or_else(|| {
            RemoteToolError::Config(format!(
                "raisin:McpConnection `{node_name}` is missing `slug`"
            ))
        })?;
        validate_slug(&slug)?;

        let raw_url = non_empty_str_prop(props, "url").ok_or_else(|| {
            RemoteToolError::Config(format!("raisin:McpConnection `{slug}` is missing `url`"))
        })?;
        let url = Url::parse(&raw_url).map_err(|e| {
            RemoteToolError::Config(format!(
                "raisin:McpConnection `{slug}` has an unparseable url `{raw_url}`: {e}"
            ))
        })?;

        Ok(Self {
            title: non_empty_str_prop(props, "title").unwrap_or_else(|| slug.clone()),
            slug,
            url,
            enabled: bool_prop(props, "enabled").unwrap_or(true),
            protocol_version: non_empty_str_prop(props, "protocol_version"),
            auth_kind: AuthKind::parse(str_prop(props, "auth_kind").as_deref()),
            static_auth: StaticAuthScheme::parse(props),
            credential_encrypted: non_empty_str_prop(props, "credential_encrypted"),
            tokens_encrypted: non_empty_str_prop(props, "tokens_encrypted"),
            expires_at: u64_prop(props, "expires_at"),
            oauth_client: obj_prop(props, "oauth_client"),
            tool_filter: parse_tool_filter(props),
            refresh_policy: RefreshPolicy::parse(props),
            discovered_tool_names: discovered_tool_names(props),
        })
    }

    /// Build the auth headers for a request, given the DECRYPTED secret.
    ///
    /// The caller decrypts; this only decides placement. `None` yields no
    /// headers, which is correct for an unauthenticated server and is also what
    /// happens when a credential is configured but missing — the resulting 401
    /// is a clearer diagnosis than a silent success against a public endpoint.
    pub fn auth_headers(&self, secret: Option<&str>) -> ExtraHeaders {
        match (self.auth_kind, secret) {
            (AuthKind::None, _) | (_, None) => Vec::new(),
            (AuthKind::Static, Some(secret)) => self.static_auth.headers(secret),
            (AuthKind::Oauth, Some(token)) => {
                vec![("Authorization".to_string(), format!("Bearer {token}"))]
            }
        }
    }

    /// Whether a stored OAuth access token is expired (or about to be).
    ///
    /// `skew_secs` of headroom avoids sending a token that expires in flight.
    pub fn token_expired(&self, now_secs: u64, skew_secs: u64) -> bool {
        match self.expires_at {
            Some(expiry) => expiry <= now_secs.saturating_add(skew_secs),
            // No expiry recorded means we cannot know; treat as valid and let a
            // 401 drive the refresh rather than refreshing on every call.
            None => false,
        }
    }

    /// Whether the connection is usable for a call right now.
    pub fn is_callable(&self) -> bool {
        self.enabled
    }

    /// A JSON view safe to return over the API: no ciphertext, no tokens.
    ///
    /// The ONLY serialization of a descriptor. Secrets are reported as booleans
    /// so the UI can render an "is set" indicator without ever receiving them.
    pub fn redacted(&self) -> Value {
        json!({
            "title": self.title,
            "slug": self.slug,
            "url": self.url.as_str(),
            "enabled": self.enabled,
            "protocol_version": self.protocol_version,
            "auth_kind": self.auth_kind,
            "static_auth": self.static_auth,
            "credential_set": self.credential_encrypted.is_some(),
            "oauth_connected": self.tokens_encrypted.is_some(),
            "oauth_client": self.oauth_client,
            "expires_at": self.expires_at,
            "tool_filter": self.tool_filter,
            "refresh_policy": self.refresh_policy,
            "tool_count": self.discovered_tool_names.len(),
        })
    }
}

/// Reject a slug that would produce an unusable or unsafe proxy path.
///
/// The slug is interpolated into `/mcp/{slug}/{tool}` and into a unique
/// function name, so a `/` or a `..` here would forge a path in the functions
/// workspace rather than merely look odd.
pub fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() || slug.len() > 48 {
        return Err(RemoteToolError::Config(
            "connection slug must be 1-48 characters".into(),
        ));
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(RemoteToolError::Config(format!(
            "connection slug `{slug}` must contain only lowercase letters, digits and hyphens"
        )));
    }
    if slug.starts_with('-') || slug.ends_with('-') {
        return Err(RemoteToolError::Config(format!(
            "connection slug `{slug}` must not start or end with a hyphen"
        )));
    }
    Ok(())
}

fn parse_tool_filter(props: &Value) -> ToolFilter {
    let block = obj_prop(props, "tool_filter");
    if block.is_null() {
        return ToolFilter::default();
    }
    ToolFilter {
        allow: str_array_prop(&block, "allow"),
        deny: str_array_prop(&block, "deny"),
    }
}

fn discovered_tool_names(props: &Value) -> Vec<String> {
    props
        .get("discovered_tools")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| non_empty_str_prop(e, "remote_name"))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props() -> Value {
        json!({
            "title": "Linear",
            "slug": "linear",
            "url": "https://mcp.linear.app/mcp",
            "auth_kind": "static",
            "static_auth": { "scheme": "header", "header_name": "X-Api-Key" },
            "credential_encrypted": "Y2lwaGVy",
            "tool_filter": { "allow": ["search_issues"], "deny": ["delete_all"] },
            "refresh_policy": { "mode": "interval", "interval_secs": 900, "on_save": false },
            "discovered_tools": [ { "remote_name": "search_issues" } ],
        })
    }

    #[test]
    fn parses_a_full_connection() {
        let conn = McpConnectionDescriptor::from_properties("linear", &props()).unwrap();

        assert_eq!(conn.slug, "linear");
        assert_eq!(conn.auth_kind, AuthKind::Static);
        assert!(conn.enabled, "enabled defaults to true when absent");
        assert_eq!(conn.refresh_policy.interval_secs, 900);
        assert!(!conn.refresh_policy.on_save);
        assert!(conn.refresh_policy.is_scheduled());
        assert_eq!(conn.discovered_tool_names, vec!["search_issues"]);
    }

    #[test]
    fn requires_slug_and_url() {
        assert!(
            McpConnectionDescriptor::from_properties("x", &json!({ "url": "https://a.b" }))
                .is_err()
        );
        assert!(McpConnectionDescriptor::from_properties("x", &json!({ "slug": "a" })).is_err());
    }

    #[test]
    fn rejects_a_slug_that_could_forge_a_path() {
        for slug in ["../evil", "a/b", "UPPER", "trailing-", "-leading", ""] {
            assert!(
                validate_slug(slug).is_err(),
                "`{slug}` must be rejected: it lands in a function path"
            );
        }
        assert!(validate_slug("linear-2").is_ok());
    }

    #[test]
    fn static_credential_lands_in_the_configured_header() {
        let conn = McpConnectionDescriptor::from_properties("linear", &props()).unwrap();
        assert_eq!(
            conn.auth_headers(Some("k3y")),
            vec![("X-Api-Key".to_string(), "k3y".to_string())]
        );
    }

    #[test]
    fn bearer_is_the_default_static_scheme() {
        let mut raw = props();
        raw["static_auth"] = json!({});
        let conn = McpConnectionDescriptor::from_properties("linear", &raw).unwrap();
        assert_eq!(
            conn.auth_headers(Some("t0k")),
            vec![("Authorization".to_string(), "Bearer t0k".to_string())]
        );
    }

    #[test]
    fn no_credential_yields_no_headers() {
        let conn = McpConnectionDescriptor::from_properties("linear", &props()).unwrap();
        assert!(conn.auth_headers(None).is_empty());
    }

    #[test]
    fn redaction_never_emits_a_secret() {
        let mut raw = props();
        raw["tokens_encrypted"] = json!("dG9rZW5z");
        let conn = McpConnectionDescriptor::from_properties("linear", &raw).unwrap();

        let redacted = conn.redacted().to_string();
        assert!(
            !redacted.contains("Y2lwaGVy"),
            "credential ciphertext leaked"
        );
        assert!(!redacted.contains("dG9rZW5z"), "token ciphertext leaked");
        assert!(redacted.contains("\"credential_set\":true"));
        assert!(redacted.contains("\"oauth_connected\":true"));
    }

    #[test]
    fn deny_beats_allow() {
        let filter = ToolFilter {
            allow: vec!["a".into(), "b".into()],
            deny: vec!["b".into()],
        };
        assert!(filter.permits("a"));
        assert!(!filter.permits("b"));
        assert!(
            !filter.permits("c"),
            "an empty-allow match must not leak in"
        );
        assert!(ToolFilter::default().permits("anything"));
    }

    #[test]
    fn call_timeout_is_clamped_not_rejected() {
        let mut raw = props();
        raw["refresh_policy"] = json!({ "call_timeout_ms": 999_999_999_u64 });
        let conn = McpConnectionDescriptor::from_properties("linear", &raw).unwrap();
        assert_eq!(conn.refresh_policy.call_timeout_ms, MAX_CALL_TIMEOUT_MS);
    }

    #[test]
    fn token_expiry_uses_skew() {
        let mut raw = props();
        raw["expires_at"] = json!(1_000_u64);
        let conn = McpConnectionDescriptor::from_properties("linear", &raw).unwrap();

        assert!(!conn.token_expired(900, 60));
        assert!(
            conn.token_expired(950, 60),
            "must refresh before it expires"
        );
        assert!(conn.token_expired(1_000, 0));
    }

    #[test]
    fn a_missing_expiry_is_not_treated_as_expired() {
        let conn = McpConnectionDescriptor::from_properties("linear", &props()).unwrap();
        assert!(!conn.token_expired(u64::MAX, 60));
    }
}
