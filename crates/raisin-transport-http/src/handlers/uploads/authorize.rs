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
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, PermissionScope};
use raisin_storage::{scope::StorageScope, NodeRepository, Storage};

use crate::{error::ApiError, state::AppState};

use super::types::UploadSession;

/// The caller's identity, as recorded on the session. A system context is
/// `system`; anything else is its user id.
pub(super) fn owner_of(auth: &AuthContext) -> String {
    if auth.is_system {
        "system".to_string()
    } else {
        auth.user_id.clone().unwrap_or_default()
    }
}

/// The request's `AuthContext`, or 401 when the middleware supplied none.
pub(super) fn require_auth(auth: Option<Extension<AuthContext>>) -> Result<AuthContext, ApiError> {
    auth.map(|Extension(a)| a)
        .ok_or_else(|| ApiError::unauthorized("authentication required"))
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
    let existing = state
        .storage
        .nodes()
        .get_by_path(
            StorageScope::new(
                &session.tenant_id,
                &session.repository,
                &session.branch,
                &session.workspace,
            ),
            &session.path,
            None,
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to look up upload target: {e}")))?;
    let scope = PermissionScope::new(&session.workspace, &session.branch);
    authorize_write(
        auth,
        &scope,
        &session.path,
        &session.node_type,
        existing.as_ref(),
    )
}

/// A session belongs to the identity that opened it. Anyone else holding the
/// id (it travels in URLs and logs) is refused.
pub(super) fn require_owner(auth: &AuthContext, session: &UploadSession) -> Result<(), ApiError> {
    if auth.is_system || session.owner == owner_of(auth) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "this upload session belongs to another identity",
        ))
    }
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
}
