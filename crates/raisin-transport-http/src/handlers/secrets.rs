// SPDX-License-Identifier: BSL-1.1

//! `/api/secrets/{repo}/{branch}` — the operator surface for the secret store.
//!
//! # Values go in; they never come out
//!
//! Every route below is admin-gated, and the route table has **no entry that
//! returns a plaintext value**. That is not an oversight to be filled in later:
//! reads return the `secret://` reference, and only server-side *use*
//! (`SecretStore::resolve`, see `raisin-rocksdb`'s `secret_store::resolve`)
//! turns a reference back into a credential. A read endpoint would make every
//! admin token a bearer of every credential in the branch, and would put
//! plaintext into access logs, proxy buffers and browser history.
//!
//! The listing types enforce this structurally rather than by careful coding:
//! `SecretMetadata` has no field that can hold ciphertext or plaintext, so
//! `list` / `list_versions` *cannot* leak one. The write responses carry only
//! `{ name, version }`.
//!
//! # Why the name is a wildcard capture
//!
//! A secret name may contain `/` — the store's convention for a vaulted schema
//! field is `node/{node_id}/{field.path}`. A single-segment `{name}` parameter
//! would 404 on exactly the names the auto-vault write path mints. So the name
//! is `{*name}`, the same wildcard form the node routes use for `{*node_path}`.
//!
//! An axum wildcard must be the LAST segment, which is why rotation is
//! `POST .../rotate/{*name}` and not `POST .../{name}/rotate`.
//!
//! # Clients send `/` literally, not `%2F`
//!
//! The wire form of `node/01H8XY/api_key` is
//! `/api/secrets/{repo}/{branch}/node/01H8XY/api_key`. The separators are real
//! path separators — that is the whole reason the parameter is a wildcard.
//!
//! `%2F` is **not** an escape that produces a one-segment name: axum
//! percent-decodes while capturing, so an encoded slash arrives as a literal
//! `/` and addresses the *same* secret. Both spellings therefore work and mean
//! one thing, which is the safe outcome — the dangerous reading would be a
//! client believing `%2F` names a *different* secret than `/` does. Pinned by
//! `percent_encoded_slashes_decode_to_the_same_name` in
//! `tests/all/http_secrets.rs`; prefer the literal form, since intermediaries
//! are known to normalise `%2F` inconsistently.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use serde::{Deserialize, Serialize};

use raisin_models::auth::AuthContext;
use raisin_rocksdb::secret_store::{SecretMetadata, SecretOwner, SecretScope, SecretStore};

use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Reject the request unless the caller is an admin.
///
/// Mirrors `handlers::integrations::require_admin`. Managing credentials is an
/// operator action; there is deliberately no per-workspace delegation of it.
fn require_admin(auth: Option<&AuthContext>) -> Result<(), ApiError> {
    let ok = auth.is_some_and(|a| {
        a.is_system
            || a.roles.iter().any(|r| r == "system_admin" || r == "admin")
            || a.permissions().is_some_and(|p| p.is_system_admin)
    });
    if ok {
        Ok(())
    } else {
        Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "Secret management requires an admin principal",
        ))
    }
}

/// The actor recorded as `created_by` on the written version. Resolved from the
/// auth context, never from the request body.
fn actor(auth: Option<&AuthContext>) -> SecretOwner {
    match auth {
        Some(ctx) => SecretOwner::actor(ctx.actor_id()),
        None => SecretOwner::default(),
    }
}

fn store(state: &AppState) -> Result<std::sync::Arc<SecretStore>, ApiError> {
    Ok(state.storage().secret_store()?)
}

/// `SecretError` → `ApiError`, through the store's own `raisin_error::Error`
/// mapping so the `Gone` / `Pending` split keeps its wording. Rust will not
/// chain the two `From` impls on its own, hence the helper rather than a bare
/// `?` at every call site.
fn api(e: raisin_rocksdb::secret_store::SecretError) -> ApiError {
    ApiError::from(raisin_error::Error::from(e))
}

/// Request body for `PUT /{name}` and `POST /rotate/{name}`.
#[derive(Debug, Deserialize)]
pub struct SetSecretRequest {
    /// The plaintext to seal. Sent, never returned.
    pub value: String,
}

/// What a write reports back. Carries no secret material by construction.
#[derive(Debug, Serialize)]
pub struct SecretWriteResponse {
    pub name: String,
    /// The ordinal of the version just written; `secret://{name}@{version}`
    /// pins it.
    pub version: u64,
}

/// The `secret://` reference for the version just written, so a caller can
/// store it in a property without re-deriving the syntax.
#[derive(Debug, Serialize)]
pub struct SecretPutResponse {
    #[serde(flatten)]
    pub written: SecretWriteResponse,
    /// `secret://{name}` — unpinned, so the property follows rotations.
    pub reference: String,
}

#[derive(Debug, Serialize)]
pub struct SecretListResponse {
    pub secrets: Vec<SecretMetadata>,
}

/// One secret's metadata: the newest version's fields at the top level, plus
/// the full version list.
///
/// The newest version is FLATTENED rather than nested under a key, so a client
/// that only wants "what is this secret" reads `name` / `version` /
/// `created_at` / `deleted` directly — the body IS a `SecretMetadata`, with
/// `versions` alongside it. A client that wants the history reads `versions`.
/// Nesting it would have forced every caller to know which shape it was getting.
#[derive(Debug, Serialize)]
pub struct SecretVersionsResponse {
    /// The newest version. Its `deleted` flag is what says whether the name is
    /// currently retired.
    #[serde(flatten)]
    pub latest: SecretMetadata,
    /// Every version, newest first. Tombstones included and flagged `deleted`,
    /// so an operator can tell a retired name from one that never existed.
    pub versions: Vec<SecretMetadata>,
}

#[derive(Debug, Serialize)]
pub struct SecretDeleteResponse {
    pub name: String,
    /// The tombstone's ordinal. Prior versions stay readable through a pinned
    /// reference, so an older node revision still resolves.
    pub version: u64,
    pub deleted: bool,
}

/// `PUT /api/secrets/{repo}/{branch}/{*name}` — create, or append a version.
pub async fn put_secret(
    State(state): State<AppState>,
    Path((repo, branch, name)): Path<(String, String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<SetSecretRequest>,
) -> Result<Json<SecretPutResponse>, ApiError> {
    require_admin(auth.as_deref())?;
    let scope = SecretScope::new(&tenant.tenant_id, &repo, &branch);
    let (name, version) = store(&state)?
        .put(&scope, &name, req.value.as_bytes(), &actor(auth.as_deref()))
        .map_err(api)?;
    Ok(Json(SecretPutResponse {
        reference: raisin_models::secret_ref::format_secret_ref(&name, None),
        written: SecretWriteResponse { name, version },
    }))
}

/// `POST /api/secrets/{repo}/{branch}/rotate/{*name}` — new version, stamped
/// `rotated_at`.
///
/// Deliberately an append, not a replacement: anything holding a pinned
/// `secret://name@N` — an older node revision, a running flow — keeps working.
pub async fn rotate_secret(
    State(state): State<AppState>,
    Path((repo, branch, name)): Path<(String, String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<SetSecretRequest>,
) -> Result<Json<SecretPutResponse>, ApiError> {
    require_admin(auth.as_deref())?;
    let scope = SecretScope::new(&tenant.tenant_id, &repo, &branch);
    let (name, version) = store(&state)?
        .rotate(&scope, &name, req.value.as_bytes(), &actor(auth.as_deref()))
        .map_err(api)?;
    Ok(Json(SecretPutResponse {
        reference: raisin_models::secret_ref::format_secret_ref(&name, None),
        written: SecretWriteResponse { name, version },
    }))
}

/// `GET /api/secrets/{repo}/{branch}` — metadata for the newest version of
/// every secret in the branch. Never ciphertext, never plaintext.
pub async fn list_secrets(
    State(state): State<AppState>,
    Path((repo, branch)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<SecretListResponse>, ApiError> {
    require_admin(auth.as_deref())?;
    let scope = SecretScope::new(&tenant.tenant_id, &repo, &branch);
    let secrets = store(&state)?.list(&scope).map_err(api)?;
    Ok(Json(SecretListResponse { secrets }))
}

/// `GET /api/secrets/{repo}/{branch}/{*name}` — every version of one secret,
/// metadata only.
pub async fn get_secret_metadata(
    State(state): State<AppState>,
    Path((repo, branch, name)): Path<(String, String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<SecretVersionsResponse>, ApiError> {
    require_admin(auth.as_deref())?;
    let scope = SecretScope::new(&tenant.tenant_id, &repo, &branch);
    let versions = store(&state)?.list_versions(&scope, &name).map_err(api)?;
    if versions.is_empty() {
        return Err(ApiError::not_found(format!("No secret named '{name}'")));
    }
    // Newest first, so entry 0 IS the current state of the name — including
    // whether it is retired.
    let latest = versions[0].clone();
    Ok(Json(SecretVersionsResponse { latest, versions }))
}

/// `DELETE /api/secrets/{repo}/{branch}/{*name}` — append a tombstone.
///
/// Deleting an unknown name is not an error: the tombstone is exactly what a
/// converging replica needs to see.
pub async fn delete_secret(
    State(state): State<AppState>,
    Path((repo, branch, name)): Path<(String, String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<SecretDeleteResponse>, ApiError> {
    require_admin(auth.as_deref())?;
    let scope = SecretScope::new(&tenant.tenant_id, &repo, &branch);
    let version = store(&state)?
        .delete(&scope, &name, &actor(auth.as_deref()))
        .map_err(api)?;
    Ok(Json(SecretDeleteResponse {
        name,
        version,
        deleted: true,
    }))
}
