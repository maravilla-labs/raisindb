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

//! OAuth 2.1 client for MCP servers that require authorization.
//!
//! The mirror of `handlers::oauth_as`, which is RaisinDB's own authorization
//! server. Here RaisinDB is the *client*, and the whole flow is bootstrapped
//! from a single 401:
//!
//! ```text
//! 401 + WWW-Authenticate: Bearer resource_metadata="https://…"   (RFC 9728)
//!   → GET that URL                    → authorization_servers[]
//!   → GET {as}/.well-known/oauth-authorization-server            (RFC 8414)
//!   → POST {registration_endpoint}    → client_id                (RFC 7591)
//!   → browser: {authorization_endpoint}?…&code_challenge=…       (PKCE)
//!   → POST {token_endpoint}           → access + refresh token
//! ```
//!
//! Dynamic client registration is what makes this work without an operator
//! pre-registering RaisinDB at every server: the redirect URI is submitted at
//! registration time rather than configured by hand on the provider's side.
//!
//! One shared service-account identity per connection. Every caller of a proxy
//! tool acts as that identity — per-user delegation is deliberately out of
//! scope, and the console says so.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::Url;

use super::egress::EgressPolicy;
use super::error::{RemoteToolError, Result};

/// Scope requested when a server advertises none.
pub const DEFAULT_SCOPE: &str = "mcp";

/// What a `401` told us about where to authorize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthChallenge {
    /// RFC 9728 protected-resource metadata URL, when the server gave one.
    pub resource_metadata: Option<String>,
    /// The `realm`, if present (display only).
    pub realm: Option<String>,
    /// Space-delimited scopes the server said it wants.
    pub scope: Option<String>,
}

/// RFC 9728 protected-resource metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProtectedResourceMetadata {
    /// Canonical resource identifier, echoed back as the `resource` parameter.
    #[serde(default)]
    pub resource: Option<String>,
    /// Authorization servers that can issue tokens for this resource.
    #[serde(rename = "authorization_servers", default)]
    pub authorization_servers: Vec<String>,
    /// Scopes the resource understands.
    #[serde(rename = "scopes_supported", default)]
    pub scopes_supported: Vec<String>,
}

/// RFC 8414 authorization-server metadata (the fields we use).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthServerMetadata {
    /// Issuer identifier.
    #[serde(default)]
    pub issuer: String,
    /// Where to send the user's browser.
    #[serde(default)]
    pub authorization_endpoint: String,
    /// Where to exchange the code and refresh tokens.
    #[serde(default)]
    pub token_endpoint: String,
    /// RFC 7591 dynamic client registration, when supported.
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    /// PKCE methods the server supports.
    #[serde(rename = "code_challenge_methods_supported", default)]
    pub code_challenge_methods_supported: Vec<String>,
    /// Scopes the server understands.
    #[serde(rename = "scopes_supported", default)]
    pub scopes_supported: Vec<String>,
    /// How the server expects a client to authenticate at the token endpoint.
    ///
    /// Load-bearing: a server that does not offer `none` REQUIRES client
    /// authentication, and registering as a public client against it produces a
    /// token exchange it rejects — Entra answers `AADSTS7000218: the request
    /// body must contain 'client_assertion' or 'client_secret'`.
    #[serde(rename = "token_endpoint_auth_methods_supported", default)]
    pub token_endpoint_auth_methods_supported: Vec<String>,
}

impl AuthServerMetadata {
    /// The auth method to request at registration.
    ///
    /// `none` — a public client protected by PKCE — is preferred where the
    /// server supports it: there is no secret to store or rotate. But it must be
    /// *offered*. Asserting it against a server that requires authentication
    /// yields a client whose every token exchange fails, with nothing on this
    /// side to explain why.
    ///
    /// An empty list means the server said nothing. RFC 8414 makes
    /// `client_secret_basic` the default then, but an MCP server that omits the
    /// field and accepts PKCE is far more common in practice, so `none` stays
    /// the assumption there.
    pub fn preferred_auth_method(&self) -> &'static str {
        let supported = &self.token_endpoint_auth_methods_supported;
        if supported.is_empty() || supported.iter().any(|m| m == "none") {
            return "none";
        }
        if supported.iter().any(|m| m == "client_secret_post") {
            return "client_secret_post";
        }
        if supported.iter().any(|m| m == "client_secret_basic") {
            return "client_secret_basic";
        }
        // Something we cannot satisfy (private_key_jwt, tls_client_auth).
        // Registering as public at least fails at registration, which is a far
        // better place to find out than after the operator has consented.
        "none"
    }

    /// Whether this server will demand a client secret at the token endpoint.
    pub fn requires_client_authentication(&self) -> bool {
        self.preferred_auth_method() != "none"
    }
}

impl AuthServerMetadata {
    /// Whether the server advertises PKCE S256.
    ///
    /// OAuth 2.1 requires PKCE, so an absent list is treated as support rather
    /// than refusal — several servers omit the field and accept S256 anyway.
    pub fn supports_s256(&self) -> bool {
        self.code_challenge_methods_supported.is_empty()
            || self
                .code_challenge_methods_supported
                .iter()
                .any(|m| m.eq_ignore_ascii_case("S256"))
    }

    /// Whether both endpoints needed for the flow are present.
    pub fn is_usable(&self) -> bool {
        !self.authorization_endpoint.is_empty() && !self.token_endpoint.is_empty()
    }
}

/// A registered (or pre-configured) OAuth client.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegisteredClient {
    /// Issued client identifier.
    pub client_id: String,
    /// Issued secret, when the server made us a confidential client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}

/// A PKCE verifier and its derived challenge.
#[derive(Debug, Clone)]
pub struct Pkce {
    /// The high-entropy secret, held until the token exchange.
    pub verifier: String,
    /// `BASE64URL(SHA256(verifier))`, sent on the authorize request.
    pub challenge: String,
}

impl Pkce {
    /// Derive the challenge for a verifier the caller generated.
    ///
    /// The verifier itself is not generated here: this crate has no RNG
    /// dependency, and the caller already mints a single-use `state` from the
    /// same source, so both come from one place.
    pub fn from_verifier(verifier: impl Into<String>) -> Self {
        let verifier = verifier.into();
        let digest = Sha256::digest(verifier.as_bytes());
        Self {
            challenge: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest),
            verifier,
        }
    }
}

/// Parse a `WWW-Authenticate` header into a challenge.
///
/// The inverse of the header this codebase's own MCP endpoint emits. Values may
/// be quoted or bare, and parameters may appear in any order.
pub fn parse_challenge(header: &str) -> Option<AuthChallenge> {
    let rest = header
        .trim()
        .strip_prefix("Bearer")
        .or_else(|| header.trim().strip_prefix("bearer"))?
        .trim_start();

    let mut challenge = AuthChallenge {
        resource_metadata: None,
        realm: None,
        scope: None,
    };
    for part in split_params(rest) {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_string();
        match key.trim().to_ascii_lowercase().as_str() {
            "resource_metadata" => challenge.resource_metadata = Some(value),
            "realm" => challenge.realm = Some(value),
            "scope" => challenge.scope = Some(value),
            _ => {}
        }
    }
    Some(challenge)
}

/// Split header parameters on commas that are not inside quotes.
fn split_params(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in input.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            ',' if !in_quotes => {
                parts.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts
}

/// The conventional RFC 8414 metadata URL for an issuer.
///
/// Well-known segments go after the ORIGIN and before the issuer's path, which
/// is the part implementations most often get wrong for a path-bearing issuer.
pub fn as_metadata_url(issuer: &str) -> Result<String> {
    let url = Url::parse(issuer)
        .map_err(|e| RemoteToolError::Config(format!("unparseable issuer `{issuer}`: {e}")))?;
    let origin = format!(
        "{}://{}",
        url.scheme(),
        url.host_str()
            .ok_or_else(|| RemoteToolError::Config("issuer has no host".to_string()))?
    );
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    let path = url.path().trim_end_matches('/');
    Ok(format!(
        "{origin}{port}/.well-known/oauth-authorization-server{path}"
    ))
}

/// Where RFC 9728 protected-resource metadata lives for a resource URL.
///
/// **The well-known URI is the discovery mechanism; the `WWW-Authenticate`
/// challenge is only a hint.** Reading the hint alone means a server that
/// publishes its authorization server correctly but does not challenge an
/// unauthenticated probe is reported as needing no auth — which is wrong, and
/// unfixable by probing harder: the auth boundary is often at `tools/call`, and
/// calling a tool to find out is exactly what a connection test must never do.
///
/// Two forms, most specific first. RaisinDB's own server emits the
/// path-suffixed one, and it is the one that disambiguates when a host serves
/// several MCP endpoints.
pub fn protected_resource_metadata_urls(resource: &Url) -> Vec<String> {
    let origin = match resource.host_str() {
        Some(host) => {
            let port = resource.port().map(|p| format!(":{p}")).unwrap_or_default();
            format!("{}://{host}{port}", resource.scheme())
        }
        None => return Vec::new(),
    };

    let path = resource.path().trim_end_matches('/');
    let mut urls = Vec::new();
    if !path.is_empty() {
        urls.push(format!("{origin}/.well-known/oauth-protected-resource{path}"));
    }
    urls.push(format!("{origin}/.well-known/oauth-protected-resource"));
    urls
}

/// Fetch and parse RFC 9728 protected-resource metadata.
pub async fn fetch_protected_resource_metadata(
    http: &reqwest::Client,
    egress: &EgressPolicy,
    url: &str,
) -> Result<ProtectedResourceMetadata> {
    fetch_json(http, egress, url, "protected-resource metadata").await
}

/// Fetch and parse RFC 8414 authorization-server metadata.
pub async fn fetch_as_metadata(
    http: &reqwest::Client,
    egress: &EgressPolicy,
    url: &str,
) -> Result<AuthServerMetadata> {
    fetch_json(http, egress, url, "authorization-server metadata").await
}

/// Apply the egress policy to a URL the *remote side* chose.
///
/// Every URL in the discovery chain after the connection's own endpoint is
/// supplied by the peer: the `WWW-Authenticate` challenge names the metadata
/// document, that document names the issuer, and the issuer's metadata names the
/// registration and token endpoints. Without this, an allowlisted MCP server can
/// walk RaisinDB straight to `http://169.254.169.254/` — and, at the token
/// endpoint, POST a client secret there.
///
/// The allowlist applies to these hosts too. An authorization server usually
/// lives on a different host from the MCP endpoint (`auth.linear.app` vs
/// `mcp.linear.app`), so an operator who sets `allowed_hosts` must list both;
/// the error says so rather than failing opaquely.
pub async fn guard_peer_url(egress: &EgressPolicy, url: &str, what: &str) -> Result<()> {
    let parsed = Url::parse(url)
        .map_err(|e| RemoteToolError::Config(format!("unparseable {what} `{url}`: {e}")))?;
    egress.guard(&parsed).await.map_err(|e| {
        RemoteToolError::Config(format!(
            "{what} `{url}` is refused by the [mcp_client] egress policy: {e} \
             (an authorization server on a different host than the MCP endpoint \
             must be listed in allowed_hosts too)"
        ))
    })
}

async fn fetch_json<T: for<'de> Deserialize<'de>>(
    http: &reqwest::Client,
    egress: &EgressPolicy,
    url: &str,
    what: &str,
) -> Result<T> {
    guard_peer_url(egress, url, what).await?;
    let response = http.get(url).send().await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(RemoteToolError::from_status(status.as_u16(), &body, None));
    }
    serde_json::from_str(&body)
        .map_err(|e| RemoteToolError::Protocol(format!("malformed {what} at {url}: {e}")))
}

/// Register this RaisinDB instance as an OAuth client (RFC 7591).
///
/// `redirect_uri` is submitted here rather than configured on the provider's
/// side — that is the entire point of dynamic registration, and why an operator
/// only has to paste a URL for servers that do NOT support it.
pub async fn register_client(
    http: &reqwest::Client,
    egress: &EgressPolicy,
    registration_endpoint: &str,
    client_name: &str,
    redirect_uri: &str,
    scopes: &[String],
    auth_method: &str,
) -> Result<RegisteredClient> {
    let body = json!({
        "client_name": client_name,
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        // Taken from the server's own metadata, never asserted. Hard-coding
        // `none` registered a PUBLIC client against every server — including
        // ones that require authentication at the token endpoint — and the
        // exchange then failed with a provider-specific code long after the
        // operator had already consented.
        "token_endpoint_auth_method": auth_method,
        "scope": scopes.join(" "),
    });

    guard_peer_url(egress, registration_endpoint, "registration endpoint").await?;
    let response = http.post(registration_endpoint).json(&body).send().await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(RemoteToolError::from_status(status.as_u16(), &text, None));
    }

    let value: Value = serde_json::from_str(&text).map_err(|e| {
        RemoteToolError::Protocol(format!("malformed client registration response: {e}"))
    })?;
    let client_id = value
        .get("client_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            RemoteToolError::Protocol("client registration returned no client_id".into())
        })?
        .to_string();

    Ok(RegisteredClient {
        client_id,
        client_secret: value
            .get("client_secret")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    })
}

/// Build the authorize URL the operator's browser is sent to.
///
/// `resource` is RFC 8707 resource indication: it pins the issued token to THIS
/// MCP endpoint, so a token minted for one server cannot be replayed at another.
#[allow(clippy::too_many_arguments)]
pub fn authorize_url(
    metadata: &AuthServerMetadata,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    pkce: &Pkce,
    scopes: &[String],
    resource: Option<&str>,
) -> Result<String> {
    if !metadata.supports_s256() {
        return Err(RemoteToolError::Config(
            "authorization server does not support PKCE S256, which OAuth 2.1 requires".into(),
        ));
    }
    let mut url = Url::parse(&metadata.authorization_endpoint)
        .map_err(|e| RemoteToolError::Config(format!("unparseable authorization_endpoint: {e}")))?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code");
        q.append_pair("client_id", client_id);
        q.append_pair("redirect_uri", redirect_uri);
        q.append_pair("state", state);
        q.append_pair("code_challenge", &pkce.challenge);
        q.append_pair("code_challenge_method", "S256");
        if !scopes.is_empty() {
            q.append_pair("scope", &scopes.join(" "));
        }
        if let Some(resource) = resource {
            q.append_pair("resource", resource);
        }
    }
    Ok(url.to_string())
}

/// Tokens returned by the token endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenResponse {
    /// The bearer token to present on MCP requests.
    pub access_token: String,
    /// Refresh token, when the server issued one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Lifetime in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<u64>,
    /// Granted scopes, which may be narrower than requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// Exchange an authorization code for tokens.
pub async fn exchange_code(
    http: &reqwest::Client,
    egress: &EgressPolicy,
    metadata: &AuthServerMetadata,
    client: &RegisteredClient,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
    resource: Option<&str>,
) -> Result<TokenResponse> {
    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("client_id", client.client_id.clone()),
        ("code_verifier", verifier.to_string()),
    ];
    if let Some(secret) = &client.client_secret {
        form.push(("client_secret", secret.clone()));
    }
    if let Some(resource) = resource {
        form.push(("resource", resource.to_string()));
    }
    post_token(http, egress, &metadata.token_endpoint, &form).await
}

/// Exchange a refresh token for a fresh access token.
pub async fn refresh_tokens(
    http: &reqwest::Client,
    egress: &EgressPolicy,
    metadata: &AuthServerMetadata,
    client: &RegisteredClient,
    refresh_token: &str,
    resource: Option<&str>,
) -> Result<TokenResponse> {
    let mut form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.to_string()),
        ("client_id", client.client_id.clone()),
    ];
    if let Some(secret) = &client.client_secret {
        form.push(("client_secret", secret.clone()));
    }
    if let Some(resource) = resource {
        form.push(("resource", resource.to_string()));
    }
    post_token(http, egress, &metadata.token_endpoint, &form).await
}

async fn post_token(
    http: &reqwest::Client,
    egress: &EgressPolicy,
    token_endpoint: &str,
    form: &[(&str, String)],
) -> Result<TokenResponse> {
    // The most sensitive hop in the chain: this POST carries the authorization
    // code or the refresh token, and the client secret when one was issued.
    guard_peer_url(egress, token_endpoint, "token endpoint").await?;
    let response = http.post(token_endpoint).form(form).send().await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        // Surface the OAuth `error` / `error_description` rather than a bare
        // status: `invalid_grant` vs `invalid_client` is the whole diagnosis.
        let detail = serde_json::from_str::<Value>(&text)
            .ok()
            .map(|v| {
                let code = v
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown_error");
                match v.get("error_description").and_then(Value::as_str) {
                    Some(desc) => format!("{code}: {desc}"),
                    None => code.to_string(),
                }
            })
            .unwrap_or_else(|| text.clone());
        return Err(RemoteToolError::from_status(status.as_u16(), &detail, None));
    }

    let tokens: TokenResponse = serde_json::from_str(&text)
        .map_err(|e| RemoteToolError::Protocol(format!("malformed token response: {e}")))?;
    if tokens.access_token.is_empty() {
        return Err(RemoteToolError::Protocol(
            "token response carried no access_token".into(),
        ));
    }
    Ok(tokens)
}

/// Absolute expiry for a token response, given the current time.
pub fn expires_at(tokens: &TokenResponse, now_secs: u64) -> Option<u64> {
    tokens.expires_in.map(|ttl| now_secs.saturating_add(ttl))
}

/// Scopes to request: what the resource asked for, else the AS's list, else a default.
pub fn resolve_scopes(
    resource: &ProtectedResourceMetadata,
    server: &AuthServerMetadata,
    challenge: Option<&AuthChallenge>,
) -> Vec<String> {
    if let Some(scope) = challenge.and_then(|c| c.scope.as_ref()) {
        let parsed: Vec<String> = scope.split_whitespace().map(str::to_string).collect();
        if !parsed.is_empty() {
            return parsed;
        }
    }
    if !resource.scopes_supported.is_empty() {
        return resource.scopes_supported.clone();
    }
    if !server.scopes_supported.is_empty() {
        return server.scopes_supported.clone();
    }
    vec![DEFAULT_SCOPE.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE SSRF this guard exists for. Every URL in the discovery chain after
    /// the connection's own endpoint is named by the peer: the challenge names
    /// the metadata document, that names the issuer, and the issuer's metadata
    /// names the registration and token endpoints. An allowlisted MCP server
    /// must not be able to walk RaisinDB to cloud instance metadata — least of
    /// all at the token endpoint, which receives a client secret.
    #[tokio::test]
    async fn a_peer_supplied_url_cannot_point_at_a_private_address() {
        let http = reqwest::Client::new();
        let policy = EgressPolicy::default();

        let metadata_url = "http://169.254.169.254/latest/meta-data/";
        let err = fetch_protected_resource_metadata(&http, &policy, metadata_url)
            .await
            .expect_err("link-local metadata must be refused");
        assert!(matches!(err, RemoteToolError::Config(_)), "got {err:?}");

        let err = fetch_as_metadata(&http, &policy, "https://10.1.2.3/.well-known/x")
            .await
            .expect_err("private address must be refused");
        assert!(matches!(err, RemoteToolError::Config(_)), "got {err:?}");

        let metadata = AuthServerMetadata {
            token_endpoint: "https://127.0.0.1/token".to_string(),
            ..Default::default()
        };
        let err = refresh_tokens(
            &http,
            &policy,
            &metadata,
            &RegisteredClient::default(),
            "rt",
            None,
        )
        .await
        .expect_err("a loopback token endpoint must never receive the refresh token");
        assert!(matches!(err, RemoteToolError::Config(_)), "got {err:?}");
    }

    /// The policy must not be bypassable by naming an allowlisted host and then
    /// redirecting the chain to one that is not on the list.
    #[tokio::test]
    async fn the_allowlist_applies_to_peer_supplied_urls_too() {
        let policy = EgressPolicy {
            allowed_hosts: vec!["mcp.linear.app".into()],
            ..EgressPolicy::default()
        };
        let err = guard_peer_url(&policy, "https://evil.test/.well-known/x", "issuer")
            .await
            .expect_err("an off-allowlist host must be refused");
        // The message has to explain the two-host case or an operator will read
        // a correct refusal as a broken feature.
        assert!(err.to_string().contains("allowed_hosts"), "got {err}");
    }

    #[test]
    fn well_known_urls_are_derived_most_specific_first() {
        let url = Url::parse("https://example.test/_mcp").unwrap();
        assert_eq!(
            protected_resource_metadata_urls(&url),
            vec![
                "https://example.test/.well-known/oauth-protected-resource/_mcp".to_string(),
                "https://example.test/.well-known/oauth-protected-resource".to_string(),
            ]
        );
    }

    /// The well-known segments go after the ORIGIN, so a port survives and a
    /// path-less endpoint yields only the bare form.
    #[test]
    fn well_known_urls_keep_the_port_and_handle_a_bare_root() {
        let url = Url::parse("https://example.test:8443/").unwrap();
        assert_eq!(
            protected_resource_metadata_urls(&url),
            vec!["https://example.test:8443/.well-known/oauth-protected-resource".to_string()]
        );
    }

    #[test]
    fn parses_the_challenge_this_codebase_emits() {
        let header = "Bearer resource_metadata=\"https://rdb.test/.well-known/oauth-protected-resource/mcp/r/main/s\"";
        let challenge = parse_challenge(header).expect("must parse");
        assert_eq!(
            challenge.resource_metadata.as_deref(),
            Some("https://rdb.test/.well-known/oauth-protected-resource/mcp/r/main/s")
        );
    }

    #[test]
    fn parses_multiple_parameters_in_any_order() {
        let header =
            r#"Bearer realm="mcp", scope="read write", resource_metadata="https://a.test/m""#;
        let c = parse_challenge(header).unwrap();
        assert_eq!(c.realm.as_deref(), Some("mcp"));
        assert_eq!(c.scope.as_deref(), Some("read write"));
        assert_eq!(c.resource_metadata.as_deref(), Some("https://a.test/m"));
    }

    #[test]
    fn a_comma_inside_quotes_does_not_split_a_parameter() {
        let c = parse_challenge(r#"Bearer scope="a,b", realm="x""#).unwrap();
        assert_eq!(c.scope.as_deref(), Some("a,b"));
        assert_eq!(c.realm.as_deref(), Some("x"));
    }

    #[test]
    fn a_non_bearer_challenge_is_not_ours() {
        assert!(parse_challenge("Basic realm=\"x\"").is_none());
    }

    #[test]
    fn pkce_challenge_matches_the_rfc_7636_test_vector() {
        // RFC 7636 Appendix B.
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(
            pkce.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn well_known_goes_before_a_path_bearing_issuer() {
        // The part implementations most often get wrong.
        assert_eq!(
            as_metadata_url("https://auth.test/tenant1").unwrap(),
            "https://auth.test/.well-known/oauth-authorization-server/tenant1"
        );
        assert_eq!(
            as_metadata_url("https://auth.test").unwrap(),
            "https://auth.test/.well-known/oauth-authorization-server"
        );
        assert_eq!(
            as_metadata_url("https://auth.test:8443/").unwrap(),
            "https://auth.test:8443/.well-known/oauth-authorization-server"
        );
    }

    fn metadata() -> AuthServerMetadata {
        AuthServerMetadata {
            issuer: "https://auth.test".into(),
            authorization_endpoint: "https://auth.test/authorize".into(),
            token_endpoint: "https://auth.test/token".into(),
            registration_endpoint: Some("https://auth.test/register".into()),
            code_challenge_methods_supported: vec!["S256".into()],
            scopes_supported: vec![],
            token_endpoint_auth_methods_supported: vec![],
        }
    }

    /// Fault 4a: hard-coding `none` registered a PUBLIC client against every
    /// server. Entra requires client authentication at the token endpoint, so
    /// the exchange was rejected with AADSTS7000218 — AFTER the operator had
    /// already consented.
    #[test]
    fn the_auth_method_comes_from_the_servers_own_metadata() {
        let mut m = metadata();

        // Says nothing: PKCE-protected public client is the right assumption for
        // an MCP server, and the common case.
        assert_eq!(m.preferred_auth_method(), "none");
        assert!(!m.requires_client_authentication());

        // Explicitly offers `none` — take it, there is no secret to store.
        m.token_endpoint_auth_methods_supported =
            vec!["none".into(), "client_secret_post".into()];
        assert_eq!(m.preferred_auth_method(), "none");

        // Does NOT offer `none`: this server requires authentication, and a
        // public registration here is what produced AADSTS7000218.
        m.token_endpoint_auth_methods_supported =
            vec!["client_secret_post".into(), "client_secret_basic".into()];
        assert_eq!(m.preferred_auth_method(), "client_secret_post");
        assert!(m.requires_client_authentication());

        // Basic only.
        m.token_endpoint_auth_methods_supported = vec!["client_secret_basic".into()];
        assert_eq!(m.preferred_auth_method(), "client_secret_basic");

        // Only methods we cannot satisfy. Fail at registration, which is a far
        // better place to find out than after consent.
        m.token_endpoint_auth_methods_supported = vec!["private_key_jwt".into()];
        assert_eq!(m.preferred_auth_method(), "none");
    }

    #[test]
    fn authorize_url_carries_pkce_state_and_resource() {
        let pkce = Pkce::from_verifier("v".repeat(43));
        let url = authorize_url(
            &metadata(),
            "cid",
            "https://rdb.test/cb",
            "st4te",
            &pkce,
            &["mcp".to_string()],
            Some("https://mcp.test/mcp"),
        )
        .unwrap();

        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&format!("code_challenge={}", pkce.challenge)));
        assert!(url.contains("state=st4te"));
        assert!(url.contains("response_type=code"));
        // RFC 8707: pins the token to this MCP endpoint so it cannot be
        // replayed at another server.
        assert!(url.contains("resource=https%3A%2F%2Fmcp.test%2Fmcp"));
    }

    #[test]
    fn a_server_without_s256_is_refused() {
        let mut m = metadata();
        m.code_challenge_methods_supported = vec!["plain".into()];
        let pkce = Pkce::from_verifier("v".repeat(43));
        assert!(authorize_url(&m, "c", "https://r/cb", "s", &pkce, &[], None).is_err());
    }

    #[test]
    fn an_absent_pkce_list_is_treated_as_support() {
        // OAuth 2.1 mandates PKCE; several servers simply omit the field.
        let mut m = metadata();
        m.code_challenge_methods_supported = vec![];
        assert!(m.supports_s256());
    }

    #[test]
    fn metadata_without_endpoints_is_unusable() {
        assert!(!AuthServerMetadata::default().is_usable());
        assert!(metadata().is_usable());
    }

    #[test]
    fn scope_resolution_prefers_the_challenge_then_resource_then_server() {
        let resource = ProtectedResourceMetadata {
            scopes_supported: vec!["res".into()],
            ..Default::default()
        };
        let mut server = metadata();
        server.scopes_supported = vec!["srv".into()];

        let challenge = AuthChallenge {
            resource_metadata: None,
            realm: None,
            scope: Some("a b".into()),
        };
        assert_eq!(
            resolve_scopes(&resource, &server, Some(&challenge)),
            vec!["a", "b"]
        );
        assert_eq!(resolve_scopes(&resource, &server, None), vec!["res"]);
        assert_eq!(
            resolve_scopes(&ProtectedResourceMetadata::default(), &server, None),
            vec!["srv"]
        );
        assert_eq!(
            resolve_scopes(
                &ProtectedResourceMetadata::default(),
                &AuthServerMetadata::default(),
                None
            ),
            vec![DEFAULT_SCOPE]
        );
    }

    #[test]
    fn expiry_is_absolute_and_saturating() {
        let tokens = TokenResponse {
            expires_in: Some(3600),
            ..Default::default()
        };
        assert_eq!(expires_at(&tokens, 1_000), Some(4_600));
        assert_eq!(expires_at(&TokenResponse::default(), 1_000), None);
    }
}
