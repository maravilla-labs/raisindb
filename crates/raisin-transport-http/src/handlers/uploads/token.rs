// SPDX-License-Identifier: BSL-1.1

//! Upload tokens: minting, and accepting one in place of a user.
//!
//! A token is minted by a signed-in user who may create at the destination
//! (checked here, with the same logic a direct upload uses), and is then
//! honoured by the upload endpoints ONLY, as `Authorization: Upload <token>`.
//! The auth middleware never sees it as a credential (it is not `Bearer`), so
//! on every other route the request is just an unauthenticated one.

use std::collections::HashMap;
use std::sync::Mutex;

use axum::{
    extract::{Extension, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use raisin_core::upload_tokens::{
    mint_upload_token, now_unix_secs, verify_upload_token, UploadTokenClaims,
    DEFAULT_UPLOAD_TOKEN_TTL_SECS, MAX_UPLOAD_TOKEN_TTL_SECS, MAX_UPLOAD_TOKEN_USES,
};
use raisin_models::auth::AuthContext;

use crate::{error::ApiError, middleware::TenantInfo, state::AppState};

use super::authorize::{authorize_session_write, require_auth, require_owner, validate_path};
use super::create::build_session;
use super::types::UploadSession;

/// Largest file a token may allow.
const MAX_TOKEN_BYTES: u64 = 1024 * 1024 * 1024;
const DEFAULT_TOKEN_BYTES: u64 = 50 * 1024 * 1024;

/// Who is calling an upload endpoint.
pub(super) enum Principal {
    User(AuthContext),
    Token(Box<UploadTokenClaims>),
}

fn invalid_token() -> ApiError {
    // One answer for every reason, so a probe learns nothing.
    ApiError::unauthorized("invalid or expired upload token")
}

/// The token an `Authorization: Upload <token>` header carries, if any.
pub(super) fn token_from_headers(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Upload "))
        .map(str::trim)
}

/// Current signing secret, then the previous one when an operator is rotating
/// (`RAISINDB_SIGNING_SECRET_PREVIOUS`, accepted only while set).
fn verification_secrets(state: &AppState) -> Result<Vec<Vec<u8>>, ApiError> {
    let mut secrets = vec![state
        .get_signing_secret()
        .map_err(|e| ApiError::internal(e.to_string()))?];
    if let Ok(prev) = std::env::var("RAISINDB_SIGNING_SECRET_PREVIOUS") {
        if !prev.is_empty() {
            secrets.push(prev.into_bytes());
        }
    }
    Ok(secrets)
}

/// Resolve the caller: an upload token when the request carries one (and then
/// ONLY that, whatever else it sends), otherwise an authenticated user.
pub(super) fn principal(
    state: &AppState,
    headers: &HeaderMap,
    tenant: &TenantInfo,
    auth: Option<Extension<AuthContext>>,
) -> Result<Principal, ApiError> {
    match token_from_headers(headers) {
        Some(token) => {
            let secrets = verification_secrets(state)?;
            let claims = verify_upload_token(&secrets, &tenant.tenant_id, token)
                .map_err(|_| invalid_token())?;
            Ok(Principal::Token(Box::new(claims)))
        }
        None => Ok(Principal::User(require_auth(auth)?)),
    }
}

/// Owner string recorded on a token's session.
pub(super) fn token_owner(claims: &UploadTokenClaims) -> String {
    format!("token:{}", claims.jti)
}

/// May this caller touch this session?
pub(super) fn require_session_access(
    principal: &Principal,
    tenant: &TenantInfo,
    session: &UploadSession,
) -> Result<(), ApiError> {
    match principal {
        Principal::User(auth) => {
            // A user can never be the owner of a token's session.
            if session.owner.starts_with("token:") && !auth.is_system {
                return Err(ApiError::not_found("Upload session not found"));
            }
            require_owner(auth, tenant, session)
        }
        Principal::Token(c) => {
            if session.tenant_id != tenant.tenant_id
                || session.owner != token_owner(c)
                || session.repository != c.repo
                || session.workspace != c.workspace
            {
                return Err(ApiError::not_found("Upload session not found"));
            }
            Ok(())
        }
    }
}

/// jti -> (uses taken, token expiry). Process-local, like the session store the
/// sessions live in; entries are dropped once the token can no longer open a
/// session.
static TOKEN_USES: Lazy<Mutex<HashMap<String, (u32, u64)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Take one use of `claims`. `Some(seq)` (0-based) or `None` when exhausted.
pub(super) fn consume_use(claims: &UploadTokenClaims) -> Option<u32> {
    let now = now_unix_secs();
    let mut map = TOKEN_USES.lock().unwrap_or_else(|p| p.into_inner());
    map.retain(|_, (_, exp)| *exp >= now);
    let entry = map.entry(claims.jti.clone()).or_insert((0, claims.exp));
    if entry.0 >= claims.uses {
        return None;
    }
    let seq = entry.0;
    entry.0 += 1;
    Some(seq)
}

/// A token's destination must be fresh: nothing at the path yet, and no
/// session of this token already heading there. This is what stops a replayed
/// use (after a restart, or on another node, where the counter starts over)
/// from writing a second time, and what keeps an exact-path token create-only.
pub(super) fn ensure_destination_free(exists: bool, in_flight: bool) -> Result<(), ApiError> {
    if exists || in_flight {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "TOKEN_DESTINATION_TAKEN",
            "the upload token's destination is already taken",
        ));
    }
    Ok(())
}

/// Mint request.
#[derive(Debug, Deserialize)]
pub struct MintUploadTokenRequest {
    pub repository: String,
    #[serde(default = "default_branch")]
    pub branch: String,
    pub workspace: String,
    /// Exact destination (one use).
    pub path: Option<String>,
    /// Destination folder; the server names each file.
    pub folder: Option<String>,
    /// File extension for folder mode, e.g. `jpg`.
    #[serde(default)]
    pub extension: String,
    #[serde(default = "default_node_type")]
    pub node_type: String,
    pub max_bytes: Option<u64>,
    #[serde(default)]
    pub allowed_mimes: Vec<String>,
    pub ttl_seconds: Option<u64>,
    pub uses: Option<u32>,
}

fn default_branch() -> String {
    "main".to_string()
}
fn default_node_type() -> String {
    "raisin:Asset".to_string()
}

#[derive(Debug, Serialize)]
pub struct MintUploadTokenResponse {
    pub token: String,
    pub expires_at: DateTime<Utc>,
    /// Where the holder POSTs to open the upload session.
    pub upload_url: String,
    pub max_bytes: u64,
    pub uses: u32,
}

fn sanitize_extension(raw: &str) -> Result<String, ApiError> {
    let ext = raw.trim().trim_start_matches('.').to_ascii_lowercase();
    if ext.len() > 8 || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(ApiError::validation_failed(
            "extension must be 1-8 letters or digits",
        ));
    }
    Ok(ext)
}

/// POST /api/uploads/tokens - mint an upload token for one destination.
pub async fn mint_token(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<MintUploadTokenRequest>,
) -> Result<(StatusCode, Json<MintUploadTokenResponse>), ApiError> {
    // Anonymous callers and upload tokens themselves are refused here.
    let auth = require_auth(auth)?;
    if req.repository.is_empty() || req.workspace.is_empty() {
        return Err(ApiError::validation_failed(
            "repository and workspace are required",
        ));
    }
    let (path, folder) = match (&req.path, &req.folder) {
        (Some(p), None) => (Some(p.clone()), None),
        (None, Some(f)) => (None, Some(f.trim_end_matches('/').to_string())),
        _ => {
            return Err(ApiError::validation_failed(
                "give exactly one of path and folder",
            ))
        }
    };
    validate_path(path.as_deref().or(folder.as_deref()).unwrap_or(""))?;

    let ttl = req
        .ttl_seconds
        .unwrap_or(DEFAULT_UPLOAD_TOKEN_TTL_SECS)
        .clamp(1, MAX_UPLOAD_TOKEN_TTL_SECS);
    let uses = if path.is_some() {
        1
    } else {
        req.uses.unwrap_or(1).clamp(1, MAX_UPLOAD_TOKEN_USES)
    };
    let max_bytes = req
        .max_bytes
        .unwrap_or(DEFAULT_TOKEN_BYTES)
        .clamp(1, MAX_TOKEN_BYTES);
    if req.allowed_mimes.len() > 20 || req.allowed_mimes.iter().any(|m| m.len() > 100) {
        return Err(ApiError::validation_failed("too many or too long mimes"));
    }
    let ext = sanitize_extension(&req.extension)?;

    let exp = now_unix_secs() + ttl;
    let claims = UploadTokenClaims {
        tenant_id: tenant_info.tenant_id.clone(),
        repo: req.repository.clone(),
        branch: req.branch.clone(),
        workspace: req.workspace.clone(),
        path,
        folder,
        ext,
        node_type: req.node_type.clone(),
        max_bytes,
        mimes: req.allowed_mimes.clone(),
        exp,
        jti: nanoid::nanoid!(24),
        uses,
    };

    // The minter must be allowed to create what the token will create.
    let dest = claims.destination(0);
    let filename = dest.rsplit('/').next().unwrap_or("").to_string();
    let now = Utc::now();
    let probe = build_session(
        "probe".to_string(),
        &claims.tenant_id,
        &claims.repo,
        &claims.branch,
        &claims.workspace,
        dest,
        filename,
        1,
        None,
        &claims.node_type,
        1,
        serde_json::Value::Null,
        "probe".to_string(),
        now,
        now,
    );
    authorize_session_write(&state, &auth, &probe).await?;

    let secret = state
        .get_signing_secret()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let token = mint_upload_token(&secret, &claims);
    tracing::info!(jti = %claims.jti, workspace = %claims.workspace, uses, "Minted upload token");

    Ok((
        StatusCode::CREATED,
        Json(MintUploadTokenResponse {
            token,
            expires_at: now + Duration::seconds(ttl as i64),
            upload_url: "/api/uploads".to_string(),
            max_bytes,
            uses,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(jti: &str, uses: u32, exp: u64) -> UploadTokenClaims {
        UploadTokenClaims {
            tenant_id: "default".into(),
            repo: "studio".into(),
            branch: "main".into(),
            workspace: "assets".into(),
            path: Some("/inbound/a.jpg".into()),
            folder: None,
            ext: String::new(),
            node_type: "raisin:Asset".into(),
            max_bytes: 10,
            mimes: vec![],
            exp,
            jti: jti.into(),
            uses,
        }
    }

    #[test]
    fn uses_run_out() {
        let c = claims("jti-uses-run-out", 2, now_unix_secs() + 60);
        assert_eq!(consume_use(&c), Some(0));
        assert_eq!(consume_use(&c), Some(1));
        assert_eq!(consume_use(&c), None);
        assert_eq!(consume_use(&c), None);
    }

    #[test]
    fn a_taken_destination_refuses_even_after_the_counter_reset() {
        // Fresh process: the counter hands out use 0 again, but the file the
        // first use produced is there, so the replay is refused.
        let c = claims("jti-replay", 1, now_unix_secs() + 60);
        assert_eq!(consume_use(&c), Some(0));
        TOKEN_USES.lock().unwrap().clear(); // restart / other node
        assert_eq!(consume_use(&c), Some(0));
        assert!(ensure_destination_free(true, false).is_err());
        // a use still in flight in this process is refused too
        assert!(ensure_destination_free(false, true).is_err());
        assert!(ensure_destination_free(false, false).is_ok());
        let e = ensure_destination_free(true, false).unwrap_err();
        assert_eq!(e.status, StatusCode::CONFLICT);
    }

    #[test]
    fn uses_are_per_token() {
        let a = claims("jti-a", 1, now_unix_secs() + 60);
        let b = claims("jti-b", 1, now_unix_secs() + 60);
        assert_eq!(consume_use(&a), Some(0));
        assert_eq!(consume_use(&b), Some(0));
        assert_eq!(consume_use(&a), None);
    }

    #[test]
    fn only_upload_scheme_is_a_token() {
        let mut h = HeaderMap::new();
        assert!(token_from_headers(&h).is_none());
        h.insert("authorization", "Bearer rut1.a.b".parse().unwrap());
        assert!(token_from_headers(&h).is_none());
        h.insert("authorization", "Upload rut1.a.b".parse().unwrap());
        assert_eq!(token_from_headers(&h), Some("rut1.a.b"));
    }

    fn session(owner: &str, repo: &str, ws: &str, tenant: &str) -> UploadSession {
        let now = Utc::now();
        build_session(
            "u1".into(),
            tenant,
            repo,
            "main",
            ws,
            "/inbound/a.jpg".into(),
            "a.jpg".into(),
            1,
            None,
            "raisin:Asset",
            1,
            serde_json::Value::Null,
            owner.into(),
            now,
            now,
        )
    }

    fn tenant(id: &str) -> TenantInfo {
        TenantInfo {
            tenant_id: id.into(),
            deployment_key: String::new(),
        }
    }

    #[test]
    fn a_token_reaches_only_its_own_session() {
        let c = claims("jti-own", 1, now_unix_secs() + 60);
        let p = Principal::Token(Box::new(c.clone()));
        let own = session(&token_owner(&c), "studio", "assets", "default");
        assert!(require_session_access(&p, &tenant("default"), &own).is_ok());
        // another token's session
        let other = session("token:someone-else", "studio", "assets", "default");
        assert!(require_session_access(&p, &tenant("default"), &other).is_err());
        // a user's session
        let users = session("u1", "studio", "assets", "default");
        assert!(require_session_access(&p, &tenant("default"), &users).is_err());
        // right owner, other tenant
        assert!(require_session_access(&p, &tenant("other"), &own).is_err());
    }

    #[test]
    fn a_user_cannot_take_over_a_token_session() {
        let c = claims("jti-user", 1, now_unix_secs() + 60);
        let own = session(&token_owner(&c), "studio", "assets", "default");
        let p = Principal::User(AuthContext::for_user("token:jti-user"));
        assert!(require_session_access(&p, &tenant("default"), &own).is_err());
    }

    #[test]
    fn extensions_are_sanitized() {
        assert_eq!(sanitize_extension(".JPG").unwrap(), "jpg");
        assert_eq!(sanitize_extension("").unwrap(), "");
        assert!(sanitize_extension("a/b").is_err());
        assert!(sanitize_extension("../x").is_err());
        assert!(sanitize_extension("waytoolongext").is_err());
    }
}
