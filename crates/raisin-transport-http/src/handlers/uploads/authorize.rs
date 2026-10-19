// SPDX-License-Identifier: BSL-1.1

//! Who may write to the path an upload targets.
//!
//! The upload job finishes under a system context (the caller is long gone by
//! then), so the permission check has to happen at the HTTP edge: at session
//! creation and again at completion. An upload is a node write, so it is
//! authorized as one: `create` on a path with no node yet, `update` on a path
//! that already holds one. Nothing here is upload-specific policy.

use axum::{http::StatusCode, Extension};
use raisin_core::services::rls_filter::{can_create_at_path, can_perform};
use raisin_core::PermissionService;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, PermissionScope};
use raisin_storage::{scope::StorageScope, NodeRepository, Storage};

use crate::{error::ApiError, state::AppState};

use super::types::UploadSession;
use crate::middleware::TenantInfo;

/// The caller's identity, as recorded on the session: a stable, non-empty
/// user id, or `system`. [`require_auth`] guarantees one exists.
pub(super) fn owner_of(auth: &AuthContext) -> String {
    if auth.is_system {
        "system".to_string()
    } else {
        auth.user_id.clone().unwrap_or_default()
    }
}

/// The request's `AuthContext`, or 401 when the middleware supplied none, the
/// caller is anonymous, or it has no stable identity (two such callers would
/// otherwise share ownership of each other's sessions).
pub(super) fn require_auth(auth: Option<Extension<AuthContext>>) -> Result<AuthContext, ApiError> {
    let auth = auth
        .map(|Extension(a)| a)
        .ok_or_else(|| ApiError::unauthorized("authentication required"))?;
    if !auth.is_system && (auth.is_anonymous || auth.user_id.as_deref().unwrap_or("").is_empty()) {
        return Err(ApiError::unauthorized(
            "uploads need an authenticated identity",
        ));
    }
    Ok(auth)
}

fn forbidden(path: &str, operation: &str) -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "FORBIDDEN",
        format!("not allowed to {operation} {path}"),
    )
}

/// An upload path is an absolute node path with no empty or relative segments.
pub(super) fn validate_path(path: &str) -> Result<(), ApiError> {
    let ok = path.starts_with('/')
        && path.len() > 1
        && path[1..]
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..");
    if ok {
        Ok(())
    } else {
        Err(ApiError::validation_failed(
            "Upload path must be an absolute node path without empty or relative segments",
        ))
    }
}

/// The decision, given what already sits at the path. Pure, so the matrix of
/// grants is testable without a server.
pub(super) fn authorize_write(
    auth: &AuthContext,
    scope: &PermissionScope,
    path: &str,
    node_type: &str,
    existing: Option<&Node>,
) -> Result<(), ApiError> {
    let allowed = match existing {
        Some(node) => can_perform(node, Operation::Update, auth, scope),
        None => can_create_at_path(path, node_type, auth, scope),
    };
    if allowed {
        Ok(())
    } else {
        Err(forbidden(
            path,
            if existing.is_some() {
                "overwrite"
            } else {
                "create"
            },
        ))
    }
}

/// The caller with permissions resolved for the repository and branch the
/// session TARGETS. The auth middleware resolves them for the repository in
/// the request path, and `/api/uploads` has none (so it is `default`): a grant
/// in another repository must neither apply here nor be needed.
fn with_session_permissions(
    auth: &AuthContext,
    resolved: Option<raisin_models::permissions::ResolvedPermissions>,
) -> AuthContext {
    let mut ctx = auth.clone();
    ctx.resolved_permissions = None;
    ctx.roles.clear();
    ctx.groups.clear();
    ctx.local_user_id = None;
    match resolved {
        Some(p) => ctx.with_permissions(p),
        None => ctx,
    }
}

/// Ancestors of `path` (excluding the root and the leaf), shallowest first.
fn ancestors_of(path: &str) -> Vec<String> {
    let parts: Vec<&str> = path[1..].split('/').collect();
    (1..parts.len())
        .map(|n| format!("/{}", parts[..n].join("/")))
        .collect()
}

/// The decision for the folders the completion job creates on the way
/// (`upsert_deep_node` makes each missing ancestor a `raisin:Folder`): each
/// must be creatable by the caller too.
pub(super) fn authorize_missing_ancestors(
    auth: &AuthContext,
    scope: &PermissionScope,
    missing: &[String],
) -> Result<(), ApiError> {
    for folder in missing {
        if !can_create_at_path(folder, "raisin:Folder", auth, scope) {
            return Err(forbidden(folder, "create"));
        }
    }
    Ok(())
}

/// Authorize `auth` to write the node the session targets, reading the live
/// state of the path.
pub(super) async fn authorize_session_write(
    state: &AppState,
    auth: &AuthContext,
    session: &UploadSession,
) -> Result<(), ApiError> {
    if auth.is_system {
        return Ok(());
    }
    validate_path(&session.path)?;
    let uid = auth.user_id.clone().unwrap_or_default();
    let resolved = state
        .permission_service
        .resolve_for_identity_id(
            &session.tenant_id,
            &session.repository,
            &session.branch,
            &uid,
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to resolve permissions: {e}")))?;
    let auth = with_session_permissions(auth, resolved);

    let scope_of = || {
        StorageScope::new(
            &session.tenant_id,
            &session.repository,
            &session.branch,
            &session.workspace,
        )
    };
    let nodes = state.storage.nodes();
    let existing = nodes
        .get_by_path(scope_of(), &session.path, None)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to look up upload target: {e}")))?;
    let scope = PermissionScope::new(&session.workspace, &session.branch);
    authorize_write(
        &auth,
        &scope,
        &session.path,
        &session.node_type,
        existing.as_ref(),
    )?;
    if existing.is_none() {
        let mut missing = Vec::new();
        for ancestor in ancestors_of(&session.path).into_iter().rev() {
            let present = nodes
                .get_by_path(scope_of(), &ancestor, None)
                .await
                .map_err(|e| ApiError::internal(format!("Failed to look up parent: {e}")))?
                .is_some();
            if present {
                break;
            }
            missing.push(ancestor);
        }
        authorize_missing_ancestors(&auth, &scope, &missing)?;
    }
    Ok(())
}

/// A session belongs to the tenant and the identity that opened it. Anyone
/// else holding the id (it travels in URLs and logs) is refused.
pub(super) fn require_owner(
    auth: &AuthContext,
    tenant: &TenantInfo,
    session: &UploadSession,
) -> Result<(), ApiError> {
    if session.tenant_id != tenant.tenant_id || !(auth.is_system || session.owner == owner_of(auth))
    {
        // Same answer as an unknown id: do not confirm the session exists.
        return Err(ApiError::not_found("Upload session not found"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::permissions::{Permission, ResolvedPermissions};

    fn auth_with(user: &str, permissions: Vec<Permission>) -> AuthContext {
        AuthContext::for_user(user).with_permissions(ResolvedPermissions {
            user_id: user.to_string(),
            email: None,
            direct_roles: vec![],
            group_roles: vec![],
            effective_roles: vec![],
            groups: vec![],
            permissions,
            is_system_admin: false,
            resolved_at: None,
        })
    }

    fn scope() -> PermissionScope {
        PermissionScope::new("assets", "main")
    }

    fn existing(path: &str) -> Node {
        Node {
            id: "n1".into(),
            name: path.rsplit('/').next().unwrap().into(),
            path: path.into(),
            node_type: "raisin:Asset".into(),
            ..Default::default()
        }
    }

    fn grant(path: &str, ops: Vec<Operation>) -> Permission {
        Permission::new(path, ops).with_workspace("assets")
    }

    fn create(auth: &AuthContext, path: &str) -> Result<(), ApiError> {
        authorize_write(auth, &scope(), path, "raisin:Asset", None)
    }

    #[test]
    fn no_grant_is_refused() {
        let auth = auth_with("svc", vec![]);
        assert!(create(&auth, "/inbound/a.png").is_err());
        assert!(create(&auth, "/anywhere/a.png").is_err());
    }

    #[test]
    fn identity_without_permissions_is_refused() {
        let auth = AuthContext::for_user("svc");
        assert!(create(&auth, "/inbound/a.png").is_err());
    }

    #[test]
    fn inbound_grant_allows_inbound_and_nothing_else() {
        let auth = auth_with("svc", vec![grant("/inbound/**", vec![Operation::Create])]);
        assert!(create(&auth, "/inbound/a.png").is_ok());
        assert!(create(&auth, "/inbound/2026/10/a.png").is_ok());
        assert!(create(&auth, "/probe/a.png").is_err());
        assert!(create(&auth, "/a.png").is_err());
        assert!(create(&auth, "/inbound-evil/a.png").is_err());
    }

    #[test]
    fn overwrite_needs_update_not_create() {
        let auth = auth_with("svc", vec![grant("/inbound/**", vec![Operation::Create])]);
        let node = existing("/inbound/a.png");
        let denied = authorize_write(&auth, &scope(), &node.path, "raisin:Asset", Some(&node));
        assert!(denied.is_err());

        let auth = auth_with(
            "svc",
            vec![grant(
                "/inbound/**",
                vec![Operation::Create, Operation::Update],
            )],
        );
        assert!(authorize_write(&auth, &scope(), &node.path, "raisin:Asset", Some(&node)).is_ok());
        let outside = existing("/probe/a.png");
        assert!(authorize_write(
            &auth,
            &scope(),
            &outside.path,
            "raisin:Asset",
            Some(&outside)
        )
        .is_err());
    }

    #[test]
    fn grant_on_another_workspace_does_not_apply() {
        let auth = auth_with(
            "svc",
            vec![Permission::new("/inbound/**", vec![Operation::Create]).with_workspace("people")],
        );
        assert!(create(&auth, "/inbound/a.png").is_err());
    }

    #[test]
    fn system_context_is_always_allowed() {
        let auth = AuthContext::system();
        assert!(create(&auth, "/anywhere/a.png").is_ok());
        let node = existing("/anywhere/a.png");
        assert!(authorize_write(&auth, &scope(), &node.path, "raisin:Asset", Some(&node)).is_ok());
    }

    #[test]
    fn paths_must_be_absolute_and_clean() {
        assert!(validate_path("/inbound/a.png").is_ok());
        for bad in ["", "/", "a.png", "/a//b", "/a/../b", "/a/./b", "/a/"] {
            assert!(validate_path(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn owner_is_the_user_id_or_system() {
        assert_eq!(owner_of(&AuthContext::system()), "system");
        assert_eq!(owner_of(&AuthContext::for_user("u1")), "u1");
    }

    fn session(owner: &str, tenant: &str) -> UploadSession {
        serde_json::from_value(serde_json::json!({
            "id": "u1", "tenant_id": tenant, "repository": "studio", "branch": "main",
            "workspace": "assets", "path": "/inbound/a.png", "filename": "a.png",
            "file_size": 1, "content_type": null, "node_type": "raisin:Asset",
            "chunk_size": 1, "bytes_received": 0, "chunks_completed": 0, "total_chunks": 1,
            "status": "pending", "temp_dir": "/tmp/u1", "metadata": null,
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
            "expires_at": "2026-01-02T00:00:00Z", "owner": owner
        }))
        .expect("session")
    }

    fn tenant(id: &str) -> TenantInfo {
        TenantInfo {
            tenant_id: id.to_string(),
            deployment_key: String::new(),
        }
    }

    #[test]
    fn only_the_opener_in_the_same_tenant_owns_the_session() {
        let s = session("u1", "default");
        assert!(require_owner(&AuthContext::for_user("u1"), &tenant("default"), &s).is_ok());
        assert!(require_owner(&AuthContext::for_user("u2"), &tenant("default"), &s).is_err());
        // right user id, other tenant
        assert!(require_owner(&AuthContext::for_user("u1"), &tenant("other"), &s).is_err());
        // system passes the owner check, not the tenant one
        assert!(require_owner(&AuthContext::system(), &tenant("default"), &s).is_ok());
        assert!(require_owner(&AuthContext::system(), &tenant("other"), &s).is_err());
    }

    #[test]
    fn callers_without_a_stable_identity_are_refused() {
        let ext = |a: AuthContext| Some(Extension(a));
        assert!(require_auth(ext(AuthContext::for_user("u1"))).is_ok());
        assert!(require_auth(ext(AuthContext::system())).is_ok());
        assert!(require_auth(ext(AuthContext::for_user(""))).is_err());
        assert!(require_auth(ext(AuthContext::anonymous_user("anon"))).is_err());
        assert!(require_auth(None).is_err());
    }

    #[test]
    fn permissions_come_from_the_sessions_repository_not_the_request_one() {
        // Granted in the request's (default) repository, nothing in the target.
        let from_default = auth_with("svc", vec![grant("/inbound/**", vec![Operation::Create])]);
        assert!(create(&from_default, "/inbound/a.png").is_ok());
        let in_target = with_session_permissions(&from_default, None);
        assert!(create(&in_target, "/inbound/a.png").is_err());
        // And the target repository's own grant is what applies.
        let granted = with_session_permissions(
            &from_default,
            auth_with("svc", vec![grant("/other/**", vec![Operation::Create])])
                .permissions()
                .cloned(),
        );
        assert!(create(&granted, "/other/a.png").is_ok());
        assert!(create(&granted, "/inbound/a.png").is_err());
    }

    #[test]
    fn ancestors_run_shallowest_first() {
        assert_eq!(
            ancestors_of("/inbound/2026/10/a.png"),
            ["/inbound", "/inbound/2026", "/inbound/2026/10"]
        );
        assert!(ancestors_of("/a.png").is_empty());
    }

    #[test]
    fn a_missing_folder_must_itself_be_creatable() {
        // create on files only (a node-type-less glob would match folders too)
        let auth = auth_with("svc", vec![grant("/inbound/**", vec![Operation::Create])]);
        let missing = ["/inbound/2026".to_string()];
        assert!(authorize_missing_ancestors(&auth, &scope(), &missing).is_ok());
        let outside = ["/probe".to_string(), "/probe/x".to_string()];
        assert!(authorize_missing_ancestors(&auth, &scope(), &outside).is_err());
        assert!(authorize_missing_ancestors(&auth, &scope(), &[]).is_ok());
        let none = auth_with("svc", vec![]);
        assert!(authorize_missing_ancestors(&none, &scope(), &missing).is_err());
    }
}
