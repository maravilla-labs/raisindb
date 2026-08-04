// SPDX-License-Identifier: BSL-1.1

//! OAuth 2.1 service-account flow for an MCP connection.
//!
//! Four steps, and the first is the point: **discover** probes the server with
//! no credential, reads the RFC 9728 challenge out of the 401, follows it to the
//! authorization server, and registers RaisinDB dynamically. An operator does
//! not paste endpoints or pre-register a client — for servers that support DCR
//! they paste nothing at all.
//!
//! `callback` is the one PUBLIC route in this module: it is a browser redirect
//! from the authorization server and cannot carry a bearer token. It is
//! authenticated by the single-use, TTL'd `state` the authenticated `start` call
//! minted, which also carries the PKCE verifier — the secret half of the
//! challenge, which never leaves this server.
//!
//! One identity per connection. Every caller of a proxy tool acts as that
//! service account; per-user delegation is deliberately out of scope.

use axum::{
    extract::{Path, Query, State},
    response::Response,
    Extension, Json,
};
use raisin_crypto::SecretBox;
use raisin_mcp::client::{
    self as mcp, AuthServerMetadata, Pkce, ProtectedResourceMetadata, RegisteredClient,
};
use raisin_models::auth::AuthContext;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    connection_service, egress_policy, load_connection, remote_error, require_admin, set_prop,
    str_prop,
};
use crate::error::ApiError;
use crate::handlers::integrations::state_store::{
    now_secs, StateKind, OAUTH_STATE_STORE, STATE_TTL_SECS,
};
use crate::middleware::TenantInfo;
use crate::state::AppState;

const ACTOR: &str = "mcp-connection-oauth";

/// Client name submitted at dynamic registration.
const CLIENT_NAME: &str = "RaisinDB";

/// A remote condition, reported rather than raised.
///
/// `discover` answers **200 with a structured report** for anything the remote
/// side did, exactly as `test` does. Two reasons, and the second is the one that
/// bit an operator: a non-2xx here renders as a toast that vanishes before it
/// can be read, and the console redirects the whole app to `/admin/login` on any
/// 401 — so an error status can navigate the operator away from the page mid-
/// flow with the diagnosis lost. Non-2xx stays for faults on OUR side only:
/// not-admin, unknown slug, missing master key.
fn finding(err: &raisin_mcp::client::RemoteToolError) -> Json<Value> {
    Json(json!({
        "ok": false,
        "requires_auth": true,
        "discovered": false,
        "error_code": err.code(),
        "error": err.to_string(),
    }))
}

/// The same, for a condition we detected rather than a transport error.
fn finding_message(code: &str, message: impl Into<String>) -> Json<Value> {
    Json(json!({
        "ok": false,
        "requires_auth": true,
        "discovered": false,
        "error_code": code,
        "error": message.into(),
    }))
}

/// `POST /api/mcp-connections/{repo}/{slug}/oauth/discover`
///
/// Probe → challenge → protected-resource metadata → AS metadata → DCR.
/// Everything discovered is non-secret and stored on the connection, except a
/// DCR-issued client secret which is encrypted.
///
/// Always 200 for a remote outcome — see [`finding`].
pub async fn discover(
    State(state): State<AppState>,
    Path((repo, slug)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;

    let (mut node, descriptor) =
        load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;
    // The policy guards every hop, not just this first one: each URL after it is
    // named by the remote side, so an allowlisted server could otherwise walk us
    // to a private address.
    let egress = egress_policy(&state);
    if let Err(e) = egress.guard(&descriptor.url).await {
        return Ok(finding(&e));
    }

    let http = raisin_functions::shared_http_client();

    // An unauthenticated probe. A server that needs OAuth answers 401 with the
    // metadata pointer; one that does not need it tells us that too.
    let response = http
        .post(descriptor.url.clone())
        .header(
            reqwest::header::ACCEPT,
            "application/json, text/event-stream",
        )
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }))
        .send()
        .await;
    let response = match response {
        Ok(response) => response,
        Err(e) => {
            return Ok(finding_message(
                "unreachable",
                format!("could not reach {}: {e}", descriptor.url),
            ))
        }
    };

    let probe_status = response.status();
    let challenged = probe_status == reqwest::StatusCode::UNAUTHORIZED;

    let challenge = response
        .headers()
        .get(reqwest::header::WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())
        .and_then(mcp::parse_challenge);

    // Candidates, most authoritative first: the challenge's own pointer, then
    // the RFC 9728 well-known paths.
    //
    // The probe's status deliberately does NOT gate this. A 200 does not mean
    // "no auth needed" — a server may keep `initialize` and even `tools/list`
    // public while requiring a token at `tools/call`, and we must never probe
    // `tools/call` to find out because a tool call has side effects. Reading the
    // challenge alone reported such a server as open, which is how an operator
    // was told "no auth needed" about a server that plainly needs it.
    let mut candidates: Vec<String> = Vec::new();
    if let Some(pointer) = challenge.as_ref().and_then(|c| c.resource_metadata.clone()) {
        candidates.push(pointer);
    }
    candidates.extend(mcp::protected_resource_metadata_urls(&descriptor.url));

    let mut resource: Option<ProtectedResourceMetadata> = None;
    let mut last_err: Option<String> = None;
    for candidate in &candidates {
        match mcp::fetch_protected_resource_metadata(&http, &egress, candidate).await {
            Ok(found) => {
                resource = Some(found);
                break;
            }
            // A 404 here is the normal answer for a server that publishes no
            // metadata; keep trying the remaining forms.
            Err(e) => last_err = Some(e.to_string()),
        }
    }

    let Some(resource) = resource else {
        // No metadata anywhere. NOW the probe's status is meaningful: a server
        // that answered 200 and advertises no authorization server is genuinely
        // open, and one that challenged us needs configuring by hand.
        return Ok(Json(json!({
            "ok": true,
            "requires_auth": challenged,
            "discovered": false,
            "status": probe_status.as_u16(),
            "message": if challenged {
                "server returned 401 but publishes no RFC 9728 protected-resource metadata; \
                 its endpoints must be configured by hand".to_string()
            } else {
                "server did not ask for authorization and publishes no protected-resource \
                 metadata; a static credential or none may be enough. If you know it requires \
                 OAuth, configure the client by hand.".to_string()
            },
            "detail": last_err,
        })));
    };

    let Some(issuer) = resource.authorization_servers.first().cloned() else {
        return Ok(finding_message(
            "protocol_error",
            "protected-resource metadata lists no authorization server",
        ));
    };

    let as_url = match mcp::as_metadata_url(&issuer) {
        Ok(url) => url,
        Err(e) => return Ok(finding(&e)),
    };
    let server: AuthServerMetadata = match mcp::fetch_as_metadata(&http, &egress, &as_url).await {
        Ok(server) => server,
        Err(e) => return Ok(finding(&e)),
    };
    if !server.is_usable() {
        return Ok(finding_message(
            "protocol_error",
            "authorization-server metadata is missing an authorization or token endpoint",
        ));
    }
    // Judge the endpoints we are about to STORE, not only the ones we dial now:
    // the token endpoint is used again by every later refresh, and the
    // authorization endpoint is handed to the operator's browser.
    for (what, endpoint) in [
        ("authorization endpoint", &server.authorization_endpoint),
        ("token endpoint", &server.token_endpoint),
    ] {
        if let Err(e) = mcp::guard_peer_endpoint(&egress, endpoint, what).await {
            return Ok(finding(&e));
        }
    }

    let scopes = mcp::resolve_scopes(&resource, &server, challenge.as_ref());
    let redirect_uri = redirect_uri(&repo);

    // Register dynamically when we can; otherwise the operator registers the
    // redirect URI by hand and pastes a client_id.
    let mut registered_now = false;
    let mut client = existing_client(&node);
    if client.client_id.is_empty() {
        if let Some(endpoint) = &server.registration_endpoint {
            // The server's own metadata decides whether we register as a public
            // client or a confidential one. Getting this wrong is invisible
            // until the token exchange — which happens AFTER the operator has
            // consented, the worst possible place to find out.
            let issued = mcp::register_client(
                &http,
                &egress,
                endpoint,
                CLIENT_NAME,
                &redirect_uri,
                &scopes,
                server.preferred_auth_method(),
            )
            .await;
            let issued = match issued {
                Ok(issued) => issued,
                Err(e) => return Ok(finding(&e)),
            };
            if let Some(secret) = &issued.client_secret {
                store_client_secret(&state, &mut node, secret)?;
            }
            client = issued;
            registered_now = true;
        }
    }

    set_prop(
        &mut node,
        "oauth_client",
        json!({
            "issuer": issuer,
            "authorization_endpoint": server.authorization_endpoint,
            "token_endpoint": server.token_endpoint,
            "registration_endpoint": server.registration_endpoint,
            "client_id": client.client_id,
            "scopes": scopes,
            "auth_method": server.preferred_auth_method(),
            "resource": resource.resource.clone().unwrap_or_else(|| descriptor.url.to_string()),
            "registered_at": chrono::Utc::now().to_rfc3339(),
        }),
    )?;
    set_prop(&mut node, "auth_kind", json!("oauth"))?;

    // Read before the node is moved into the write.
    //
    // The case that used to fail silently: the server demands client
    // authentication, dynamic registration issued no secret (or there was no
    // registration at all), and nothing surfaced that until the token exchange
    // returned a provider-specific code AFTER the operator had consented. Say it
    // here, while they can still act on it.
    let secret_stored = str_prop(&node, "oauth_client_secret_encrypted").is_some();
    let needs_secret = server.requires_client_authentication() && !secret_stored;

    connection_service(&state, &tenant.tenant_id, &repo, ACTOR)
        .update_node(node)
        .await?;

    Ok(Json(json!({
        "ok": true,
        "requires_auth": true,
        "discovered": true,
        "auth_method": server.preferred_auth_method(),
        "client_secret_required": server.requires_client_authentication(),
        "client_secret_set": secret_stored,
        "needs_manual_client_secret": needs_secret,
        "message": needs_secret.then(|| format!(
            "{issuer} requires client authentication at its token endpoint, but no client secret \
             is stored. Register RaisinDB at that provider with the redirect URI below, then save \
             the client id and secret here — otherwise the token exchange will be rejected after \
             you consent."
        )),
        "issuer": issuer,
        "authorization_endpoint": server.authorization_endpoint,
        "token_endpoint": server.token_endpoint,
        "supports_dynamic_registration": server.registration_endpoint.is_some(),
        "registered": registered_now || !client.client_id.is_empty(),
        "client_id": client.client_id,
        "scopes": scopes,
        "redirect_uri": redirect_uri,
    })))
}

/// `POST /api/mcp-connections/{repo}/{slug}/oauth/start`
pub async fn start(
    State(state): State<AppState>,
    Path((repo, slug)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;

    let (node, descriptor) =
        load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;
    let cfg = &descriptor.oauth_client;

    let client_id = json_str(cfg, "client_id").ok_or_else(|| {
        ApiError::validation_failed(
            "connection has no OAuth client; run oauth/discover first".to_string(),
        )
    })?;
    let metadata = metadata_from(cfg);
    if !metadata.is_usable() {
        return Err(ApiError::validation_failed(
            "connection has no OAuth endpoints; run oauth/discover first",
        ));
    }

    // The verifier and the state come from ONE source, so a flow cannot end up
    // with a state that has no verifier.
    let state_token = nanoid::nanoid!(32);
    let pkce = Pkce::from_verifier(nanoid::nanoid!(64));

    let url = mcp::authorize_url(
        &metadata,
        &client_id,
        &redirect_uri(&repo),
        &state_token,
        &pkce,
        &json_strs(cfg, "scopes"),
        json_str(cfg, "resource").as_deref(),
    )
    .map_err(remote_error)?;

    OAUTH_STATE_STORE.insert_kind(
        state_token.clone(),
        tenant.tenant_id.clone(),
        repo.clone(),
        node.path.clone(),
        StateKind::McpConnection,
        Some(pkce.verifier),
        now_secs(),
        STATE_TTL_SECS,
    );

    Ok(Json(json!({ "auth_url": url, "state": state_token })))
}

/// Query parameters an authorization server sends back.
#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    /// Authorization code, on success.
    pub code: Option<String>,
    /// Opaque single-use state minted by `start`.
    pub state: Option<String>,
    /// OAuth error code, on failure.
    pub error: Option<String>,
    /// Human-readable failure detail.
    pub error_description: Option<String>,
}

/// `GET /api/mcp-connections/{repo}/oauth/callback` — PUBLIC.
///
/// Authenticated by the single-use `state` parameter, because a browser
/// redirect cannot carry a bearer token.
pub async fn callback(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    match complete(&state, &repo, query).await {
        Ok(name) => relay(&repo, Ok(name)),
        Err(message) => relay(&repo, Err(message)),
    }
}

/// The callback's work, with failures as plain messages for the relay page.
async fn complete(state: &AppState, repo: &str, query: CallbackQuery) -> Result<String, String> {
    // State FIRST, before any value the authorization server sent is echoed
    // back into the relay page. The error branch used to return early, which
    // meant anyone who could get a browser to hit this URL with `?error=…`
    // reached the page's sink without ever holding a valid single-use state.
    let Some(state_token) = query.state else {
        return Err("callback is missing `state`".to_string());
    };
    let entry = OAUTH_STATE_STORE
        .consume(&state_token, StateKind::McpConnection, now_secs())
        .ok_or_else(|| "authorization state is unknown or expired; start again".to_string())?;
    if entry.repo != repo {
        return Err("authorization state does not belong to this flow".to_string());
    }

    if let Some(error) = query.error {
        let detail = query.error_description.unwrap_or_default();
        return Err(format!("{error}: {detail}"));
    }
    let Some(code) = query.code else {
        return Err("callback is missing `code`".to_string());
    };
    let verifier = entry
        .pkce_verifier
        .ok_or_else(|| "authorization state carried no PKCE verifier".to_string())?;

    let slug = entry
        .target_path
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string();
    let (mut node, descriptor) = load_connection(state, &entry.tenant_id, repo, &slug, ACTOR)
        .await
        .map_err(|e| e.into_message())?;

    let cfg = &descriptor.oauth_client;
    let metadata = metadata_from(cfg);
    let client = RegisteredClient {
        client_id: json_str(cfg, "client_id").unwrap_or_default(),
        client_secret: decrypt_client_secret(state, &node),
    };

    // Re-checked here rather than trusted from discovery: the endpoint was
    // stored by a call that may predate this guard, and this route is public.
    let tokens = mcp::exchange_code(
        &raisin_functions::shared_http_client(),
        &egress_policy(state),
        &metadata,
        &client,
        &code,
        &redirect_uri(repo),
        &verifier,
        json_str(cfg, "resource").as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;

    store_tokens(state, &mut node, &tokens).map_err(|e| e.into_message())?;
    connection_service(state, &entry.tenant_id, repo, ACTOR)
        .update_node(node)
        .await
        .map_err(|e| e.to_string())?;

    Ok(descriptor.title)
}

/// `POST /api/mcp-connections/{repo}/{slug}/oauth/disconnect`
pub async fn disconnect(
    State(state): State<AppState>,
    Path((repo, slug)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;

    let (mut node, _) = load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;
    node.properties.remove("tokens_encrypted");
    node.properties.remove("expires_at");
    // The client registration is kept: reconnecting should not need another
    // round of dynamic registration, and the client_id is not a secret.
    connection_service(&state, &tenant.tenant_id, &repo, ACTOR)
        .update_node(node)
        .await?;

    Ok(Json(json!({ "ok": true, "oauth_connected": false })))
}

/// Encrypt and store the token pair plus its absolute expiry.
fn store_tokens(
    state: &AppState,
    node: &mut raisin_models::nodes::Node,
    tokens: &mcp::TokenResponse,
) -> Result<(), ApiError> {
    let master_key = state.get_master_key().map_err(|_| {
        ApiError::internal("RAISIN_MASTER_KEY is not configured; cannot store MCP tokens")
    })?;
    let blob = SecretBox::new(&master_key)
        .encrypt_json(&json!({
            "access_token": tokens.access_token,
            "refresh_token": tokens.refresh_token,
        }))
        .map_err(|e| ApiError::internal(format!("failed to encrypt MCP tokens: {e}")))?;
    set_prop(node, "tokens_encrypted", json!(blob))?;

    match mcp::expires_at(tokens, now_secs()) {
        Some(expiry) => set_prop(node, "expires_at", json!(expiry))?,
        // No expiry means we cannot pre-emptively refresh; a 401 drives it.
        None => {
            node.properties.remove("expires_at");
        }
    }
    Ok(())
}

fn store_client_secret(
    state: &AppState,
    node: &mut raisin_models::nodes::Node,
    secret: &str,
) -> Result<(), ApiError> {
    let master_key = state.get_master_key().map_err(|_| {
        ApiError::internal(
            "RAISIN_MASTER_KEY is not configured; cannot store the OAuth client secret",
        )
    })?;
    let blob = SecretBox::new(&master_key)
        .encrypt_json(&json!({ "value": secret }))
        .map_err(|e| ApiError::internal(format!("failed to encrypt client secret: {e}")))?;
    set_prop(node, "oauth_client_secret_encrypted", json!(blob))
}

fn decrypt_client_secret(state: &AppState, node: &raisin_models::nodes::Node) -> Option<String> {
    let master_key = state.get_master_key().ok()?;
    let blob = str_prop(node, "oauth_client_secret_encrypted")?;
    SecretBox::new(&master_key)
        .decrypt_json(&blob)
        .ok()?
        .get("value")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn existing_client(node: &raisin_models::nodes::Node) -> RegisteredClient {
    let cfg = node
        .properties
        .get("oauth_client")
        .and_then(|pv| serde_json::to_value(pv).ok())
        .unwrap_or(Value::Null);
    RegisteredClient {
        client_id: json_str(&cfg, "client_id").unwrap_or_default(),
        client_secret: None,
    }
}

/// Rebuild AS metadata from what discovery stored on the connection.
pub(crate) fn metadata_from(cfg: &Value) -> AuthServerMetadata {
    AuthServerMetadata {
        issuer: json_str(cfg, "issuer").unwrap_or_default(),
        authorization_endpoint: json_str(cfg, "authorization_endpoint").unwrap_or_default(),
        token_endpoint: json_str(cfg, "token_endpoint").unwrap_or_default(),
        registration_endpoint: json_str(cfg, "registration_endpoint"),
        // Stored metadata predates the check; discovery already refused a
        // server without S256, so treat absence as support rather than
        // re-refusing on every refresh.
        code_challenge_methods_supported: Vec::new(),
        scopes_supported: json_strs(cfg, "scopes"),
        // Registration is already done by the time this is rebuilt; the token
        // exchange authenticates with the stored secret, not with this list.
        token_endpoint_auth_methods_supported: Vec::new(),
    }
}

pub(crate) fn json_str(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn json_strs(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The redirect URI for this repo — derived, never taken from stored config.
///
/// The authorization server redirects the browser to whatever we send, and the
/// callback is routed on exactly one path. A stored value pointing elsewhere
/// sends the operator to a 404 *after* they have consented.
fn redirect_uri(repo: &str) -> String {
    let (base, _) = crate::handlers::integrations::setup_info::base_or_placeholder();
    format!(
        "{}/api/mcp-connections/{}/oauth/callback",
        base.trim_end_matches('/'),
        repo
    )
}

/// A minimal self-closing page for the popup the console opened.
fn relay(repo: &str, outcome: Result<String, String>) -> Response {
    use axum::response::IntoResponse;

    let (payload, heading, detail) = match &outcome {
        Ok(name) => (
            json!({ "type": "raisin-oauth-result", "connected": name }),
            "Connection authorized",
            format!("{name} is connected. You can close this window."),
        ),
        Err(message) => (
            json!({ "type": "raisin-oauth-result", "error": message }),
            "Authorization failed",
            message.clone(),
        ),
    };
    // The console route is /admin/{repo}/mcp-connections — the SPA's basename
    // is "/admin" and the repo is the first segment. There is no "repos/"
    // segment; getting this wrong sends the operator to a 404 immediately
    // after they consented, which is the worst possible moment for it.
    let console = format!("/admin/{repo}/mcp-connections");

    // The payload rides in a data attribute and is JSON.parse'd, rather than
    // being interpolated into the <script> block. Serializing to JSON is NOT
    // enough there: inside a script element the HTML parser ends the block at
    // the first `</script`, whatever the JavaScript string context, so an
    // `error_description` containing one would break out and execute.
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>{heading}</title>\
         <body style=\"font:14px system-ui;padding:2rem\" data-oauth-result=\"{}\">\
         <h3>{heading}</h3><p>{}</p>\
         <p><a href=\"{}\">Back to connections</a></p>\
         <script>try{{\
           var r=JSON.parse(document.body.dataset.oauthResult);\
           window.opener&&window.opener.postMessage(r,window.location.origin);\
           window.close()\
         }}catch(e){{}}</script>",
        html_escape(&payload.to_string()),
        html_escape(&detail),
        html_escape(&console),
    );
    axum::response::Html(body).into_response()
}

/// Escape untrusted text before it lands in the relay page.
///
/// Everything interpolated into that page goes through this: the visible
/// detail, the JSON payload in its data attribute, and the console href. The
/// authorization server's `error_description` and the request's `repo` segment
/// are both values we did not write.
///
/// `&` must be replaced first or it would double-escape the entities the later
/// replacements introduce. `'` is escaped too, since an attribute elsewhere may
/// be single-quoted.
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_config_round_trips_into_metadata() {
        let cfg = json!({
            "issuer": "https://auth.test",
            "authorization_endpoint": "https://auth.test/authorize",
            "token_endpoint": "https://auth.test/token",
            "scopes": ["mcp", "read"],
        });
        let m = metadata_from(&cfg);
        assert!(m.is_usable());
        assert!(m.supports_s256());
        assert_eq!(m.scopes_supported, vec!["mcp", "read"]);
    }

    #[test]
    fn missing_endpoints_make_metadata_unusable() {
        assert!(!metadata_from(&json!({ "issuer": "https://auth.test" })).is_usable());
    }

    #[test]
    fn relay_escapes_a_hostile_error_description() {
        let escaped = html_escape("<script>alert(1)</script>");
        assert!(!escaped.contains("<script>"));
        assert!(escaped.contains("&lt;script&gt;"));
    }

    /// The break-out that JSON serialization alone does NOT prevent: inside a
    /// script element the HTML parser ends the block at the first `</script`.
    #[test]
    fn a_payload_cannot_close_the_script_block() {
        let hostile = json!({ "error": "</script><script>alert(1)</script>" });
        let embedded = html_escape(&hostile.to_string());
        assert!(!embedded.contains("</script"));
        assert!(!embedded.contains('<'));
    }

    /// `repo` comes from the URL path and lands in an href attribute.
    #[test]
    fn a_hostile_repo_cannot_escape_the_href_attribute() {
        let escaped = html_escape("x\" onmouseover=\"alert(1)");
        assert!(!escaped.contains('"'));
        assert!(escaped.contains("&quot;"));
    }

    #[test]
    fn escaping_does_not_double_encode() {
        // `&` first, or `<` would become `&amp;lt;`.
        assert_eq!(html_escape("a<b"), "a&lt;b");
        assert_eq!(html_escape("a&b"), "a&amp;b");
    }
}
