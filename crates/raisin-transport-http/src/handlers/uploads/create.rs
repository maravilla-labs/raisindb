// SPDX-License-Identifier: BSL-1.1

//! Upload session creation handler.

use axum::{
    extract::{Extension, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{DateTime, Duration, Utc};

use raisin_core::upload_tokens::{now_unix_secs, UploadTokenClaims};
use raisin_models::auth::AuthContext;

use crate::{error::ApiError, middleware::TenantInfo, state::AppState};

use super::authorize::target_exists;
use super::authorize::{authorize_session_write, owner_of, validate_path};
use super::token::{consume_use, ensure_destination_free, principal, token_owner, Principal};

use super::types::{
    CreateUploadRequest, CreateUploadResponse, UploadSession, UploadSessionStatus,
    DEFAULT_SESSION_EXPIRATION_HOURS, UPLOAD_STORE, UPLOAD_TEMP_DIR,
};

/// How long a session opened by a token stays usable.
const TOKEN_SESSION_MINUTES: i64 = 60;

#[allow(clippy::too_many_arguments)]
pub(super) fn build_session(
    id: String,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    workspace: &str,
    path: String,
    filename: String,
    file_size: u64,
    content_type: Option<String>,
    node_type: &str,
    chunk_size: u64,
    metadata: serde_json::Value,
    owner: String,
    now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> UploadSession {
    let total_chunks = file_size.div_ceil(chunk_size.max(1)) as u32;
    UploadSession {
        temp_dir: format!("{}/{}", UPLOAD_TEMP_DIR, id),
        id,
        tenant_id: tenant_id.to_string(),
        repository: repository.to_string(),
        branch: branch.to_string(),
        workspace: workspace.to_string(),
        path,
        filename,
        file_size,
        content_type,
        node_type: node_type.to_string(),
        chunk_size,
        bytes_received: 0,
        chunks_completed: 0,
        total_chunks,
        status: UploadSessionStatus::Pending,
        metadata,
        created_at: now,
        updated_at: now,
        expires_at,
        owner,
    }
}

/// A body field that names something the token already fixes must agree with
/// it; an empty one is simply ignored in favour of the claim.
fn must_match(given: &str, claimed: &str, what: &str) -> Result<(), ApiError> {
    if given.is_empty() || given == claimed {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "TOKEN_SCOPE",
            format!("the upload token does not cover this {what}"),
        ))
    }
}

/// Token-bound checks on the request, before a use is taken.
fn check_token_request(
    claims: &UploadTokenClaims,
    req: &CreateUploadRequest,
) -> Result<(), ApiError> {
    if now_unix_secs() > claims.exp {
        return Err(ApiError::unauthorized("invalid or expired upload token"));
    }
    must_match(&req.repository, &claims.repo, "repository")?;
    must_match(&req.branch, &claims.branch, "branch")?;
    must_match(&req.workspace, &claims.workspace, "workspace")?;
    // The destination is the token's, whatever the body says; a body path that
    // is neither empty nor inside the claim is a client trying something.
    if !req.path.is_empty() {
        let in_claim = match (&claims.path, &claims.folder) {
            (Some(p), _) => &req.path == p,
            (_, Some(f)) => req
                .path
                .strip_prefix(f.as_str())
                .is_some_and(|rest| rest.starts_with('/')),
            _ => false,
        };
        if !in_claim {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "TOKEN_SCOPE",
                "the upload token does not cover this path",
            ));
        }
    }
    if req.file_size > claims.max_bytes {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "TOKEN_TOO_LARGE",
            format!("the upload token allows at most {} bytes", claims.max_bytes),
        ));
    }
    if !claims.mime_allowed(req.content_type.as_deref()) {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "TOKEN_MIME",
            "the upload token does not allow this content type",
        ));
    }
    Ok(())
}

/// POST /api/uploads - Create upload session
pub async fn create_upload(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    headers: HeaderMap,
    Json(req): Json<CreateUploadRequest>,
) -> Result<(StatusCode, Json<CreateUploadResponse>), ApiError> {
    let tenant_id = &tenant_info.tenant_id;
    let principal = principal(&state, &headers, &tenant_info, auth)?;

    // Validate inputs
    if req.file_size == 0 {
        return Err(ApiError::validation_failed(
            "File size must be greater than 0",
        ));
    }

    if req.chunk_size == 0 || req.chunk_size > 100 * 1024 * 1024 {
        return Err(ApiError::validation_failed(
            "Chunk size must be between 1 byte and 100MB",
        ));
    }

    let upload_id = nanoid::nanoid!();
    let now = Utc::now();

    let session = match &principal {
        Principal::User(auth) => {
            validate_path(&req.path)?;
            if req.repository.is_empty() || req.workspace.is_empty() || req.filename.is_empty() {
                return Err(ApiError::validation_failed(
                    "repository, workspace, path and filename are required",
                ));
            }
            let session = build_session(
                upload_id.clone(),
                tenant_id,
                &req.repository,
                if req.branch.is_empty() {
                    "main"
                } else {
                    &req.branch
                },
                &req.workspace,
                req.path.clone(),
                req.filename.clone(),
                req.file_size,
                req.content_type.clone(),
                &req.node_type,
                req.chunk_size,
                req.metadata.clone(),
                owner_of(auth),
                now,
                now + Duration::hours(DEFAULT_SESSION_EXPIRATION_HOURS),
            );
            // The completion job runs as system, so the write is authorized here.
            authorize_session_write(&state, auth, &session).await?;
            session
        }
        Principal::Token(claims) => {
            check_token_request(claims, &req)?;
            // Authorization happened when the token was minted. Take the use
            // last, so a refused request does not burn it.
            let seq = consume_use(claims)
                .ok_or_else(|| ApiError::unauthorized("invalid or expired upload token"))?;
            let path = claims.destination(seq);
            validate_path(&path)?;
            let filename = path.rsplit('/').next().unwrap_or_default().to_string();
            let session = build_session(
                upload_id.clone(),
                tenant_id,
                &claims.repo,
                &claims.branch,
                &claims.workspace,
                path,
                filename,
                req.file_size,
                req.content_type.clone(),
                &claims.node_type,
                req.chunk_size,
                serde_json::Value::Null,
                token_owner(claims),
                now,
                now + Duration::minutes(TOKEN_SESSION_MINUTES),
            );
            // Create-only, and one writer per use.
            let exists = target_exists(&state, &session).await?;
            let in_flight = UPLOAD_STORE.has_active(&session.owner, &session.path).await;
            ensure_destination_free(exists, in_flight)?;
            session
        }
    };

    let total_chunks = session.total_chunks;
    let expires_at = session.expires_at;
    UPLOAD_STORE.put(session).await;

    tracing::info!(
        upload_id = %upload_id,
        file_size = req.file_size,
        total_chunks = total_chunks,
        "Created upload session"
    );

    Ok((
        StatusCode::CREATED,
        Json(CreateUploadResponse {
            upload_id: upload_id.clone(),
            upload_url: format!("/api/uploads/{}", upload_id),
            chunk_size: req.chunk_size,
            total_chunks,
            expires_at,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(path: Option<&str>, folder: Option<&str>) -> UploadTokenClaims {
        UploadTokenClaims {
            tenant_id: "default".into(),
            repo: "studio".into(),
            branch: "main".into(),
            workspace: "assets".into(),
            path: path.map(Into::into),
            folder: folder.map(Into::into),
            ext: "jpg".into(),
            node_type: "raisin:Asset".into(),
            max_bytes: 100,
            mimes: vec!["image/*".into()],
            exp: now_unix_secs() + 60,
            jti: "jtijtijtijti".into(),
            uses: 1,
        }
    }

    fn req(json: serde_json::Value) -> CreateUploadRequest {
        let mut base = serde_json::json!({"file_size": 10, "content_type": "image/jpeg"});
        base.as_object_mut()
            .unwrap()
            .extend(json.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    fn code(r: Result<(), ApiError>) -> String {
        r.unwrap_err().code.clone()
    }

    #[test]
    fn minimal_body_is_enough() {
        let c = claims(Some("/inbound/a.jpg"), None);
        assert!(check_token_request(&c, &req(serde_json::json!({}))).is_ok());
    }

    #[test]
    fn a_different_path_in_the_body_is_refused() {
        let c = claims(Some("/inbound/a.jpg"), None);
        assert!(
            check_token_request(&c, &req(serde_json::json!({"path": "/inbound/a.jpg"}))).is_ok()
        );
        for bad in [
            "/inbound/b.jpg",
            "/other/a.jpg",
            "/inbound/../a.jpg",
            "/inbound/a.jpg/x",
        ] {
            let r = check_token_request(&c, &req(serde_json::json!({"path": bad})));
            assert_eq!(code(r), "TOKEN_SCOPE", "{bad}");
        }
    }

    #[test]
    fn folder_claim_covers_only_its_own_segments() {
        let c = claims(None, Some("/inbound/phone"));
        assert!(check_token_request(
            &c,
            &req(serde_json::json!({"path": "/inbound/phone/x.jpg"}))
        )
        .is_ok());
        for bad in [
            "/inbound/phone",
            "/inbound/phone-evil/x.jpg",
            "/inbound/x.jpg",
        ] {
            let r = check_token_request(&c, &req(serde_json::json!({"path": bad})));
            assert_eq!(code(r), "TOKEN_SCOPE", "{bad}");
        }
    }

    #[test]
    fn other_repo_workspace_or_branch_is_refused() {
        let c = claims(Some("/inbound/a.jpg"), None);
        for body in [
            serde_json::json!({"repository": "other"}),
            serde_json::json!({"workspace": "people"}),
            serde_json::json!({"branch": "publish"}),
        ] {
            assert_eq!(code(check_token_request(&c, &req(body))), "TOKEN_SCOPE");
        }
        let ok = req(
            serde_json::json!({"repository": "studio", "workspace": "assets", "branch": "main"}),
        );
        assert!(check_token_request(&c, &ok).is_ok());
    }

    #[test]
    fn size_and_mime_limits() {
        let c = claims(Some("/inbound/a.jpg"), None);
        let big = req(serde_json::json!({"file_size": 101}));
        assert_eq!(code(check_token_request(&c, &big)), "TOKEN_TOO_LARGE");
        let exact = req(serde_json::json!({"file_size": 100}));
        assert!(check_token_request(&c, &exact).is_ok());
        let pdf = req(serde_json::json!({"content_type": "application/pdf"}));
        assert_eq!(code(check_token_request(&c, &pdf)), "TOKEN_MIME");
        let none = req(serde_json::json!({"content_type": null}));
        assert_eq!(code(check_token_request(&c, &none)), "TOKEN_MIME");
    }

    #[test]
    fn expired_token_cannot_open_a_session() {
        let mut c = claims(Some("/inbound/a.jpg"), None);
        c.exp = now_unix_secs() - 1;
        let r = check_token_request(&c, &req(serde_json::json!({})));
        assert_eq!(r.unwrap_err().status, StatusCode::UNAUTHORIZED);
    }
}
