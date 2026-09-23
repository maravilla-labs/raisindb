// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! A run's principal, as the `AuthContext` its operations execute under.
//!
//! Every operation runs with REAL rights, so row-level security applies — the
//! function executor treats a missing context as system, which is exactly
//! what must never happen by accident here. So this fails CLOSED: a principal
//! whose rights cannot be resolved gets an error, never a wider context.
//!
//! - `User`: that identity's resolved permissions.
//! - `Agent`: the agent's own rights when the agent node grants them (the
//!   shared `agent_auth` helper the flow and chat paths use); otherwise the
//!   user it acts for (`on_behalf_of`), marked with the agent for provenance.
//! - `System`: system — creatable only in-process, with a `SystemToken`.

use std::sync::Arc;

use raisin_agent_runtime::ids::{Principal, PrincipalKind, RunScope};
use raisin_models::auth::{agent_identity, AuthContext};

use crate::RocksDBStorage;

/// Workspace of agent nodes when an agent principal names only a path.
const AGENTS_WORKSPACE: &str = "functions";

/// The acting-user id of a run created by a system caller.
const SYSTEM_IDENTITY: &str = "system";

/// `"workspace:/path"` or `"/path"` (functions workspace).
fn split_agent(id: &str) -> (&str, &str) {
    match id.split_once(':') {
        Some((ws, path)) if path.starts_with('/') => (ws, path),
        _ => (AGENTS_WORKSPACE, id),
    }
}

async fn user_context(
    storage: &Arc<RocksDBStorage>,
    scope: &RunScope,
    user_id: &str,
    marker: Option<String>,
) -> Result<AuthContext, String> {
    // The reserved `system` identity is what a SYSTEM caller (an admin API key,
    // a function running in system context) records as the user it acts for:
    // only such a caller can create a run whose acting user is `system`
    // (`api::acting_user`). It has no identity node to resolve, and the rights
    // it authorized are exactly system rights, so that is what it gets.
    if user_id == SYSTEM_IDENTITY {
        let ctx = AuthContext::system();
        return Ok(match marker {
            Some(m) => ctx.with_agent(m),
            None => ctx,
        });
    }
    let service =
        raisin_core::services::permission_service::PermissionService::new(storage.clone());
    let fail =
        |e: raisin_error::Error| format!("cannot resolve permissions of user '{user_id}': {e}");
    // The identity (a JWT `sub`) is the canonical form. A user NODE id is
    // accepted too — messages name their sender by node — and resolves the
    // same user's permissions (ai-tools canonicalizes to the identity first).
    let resolved = match service
        .resolve_for_identity_id(&scope.tenant_id, &scope.repo_id, &scope.branch, user_id)
        .await
        .map_err(fail)?
    {
        Some(r) => r,
        None => service
            .resolve_for_user_id(&scope.tenant_id, &scope.repo_id, &scope.branch, user_id)
            .await
            .map_err(fail)?
            .ok_or_else(|| {
                format!("user '{user_id}' is not a resolvable identity in this repository")
            })?,
    };
    let ctx = AuthContext::for_user(user_id).with_permissions(resolved);
    Ok(match marker {
        Some(m) => ctx.with_agent(m),
        None => ctx,
    })
}

/// The context `principal`'s operations run under in `scope`.
pub async fn resolve_principal_auth(
    storage: &Arc<RocksDBStorage>,
    scope: &RunScope,
    principal: &Principal,
    run_marker: &str,
) -> Result<AuthContext, String> {
    match principal.kind {
        PrincipalKind::System => Ok(AuthContext::system().with_agent(run_marker.to_string())),
        PrincipalKind::User => {
            user_context(storage, scope, &principal.id, Some(run_marker.to_string())).await
        }
        PrincipalKind::Agent => {
            let (ws, path) = split_agent(&principal.id);
            let marker = agent_identity::with_origin(agent_identity::agent(path), Some(run_marker));
            let own = raisin_core::services::agent_auth::resolve_agent_context(
                storage,
                &scope.tenant_id,
                &scope.repo_id,
                &scope.branch,
                ws,
                path,
                &marker,
                principal.on_behalf_of.as_deref(),
            )
            .await?;
            if let Some(ctx) = own {
                return Ok(ctx);
            }
            match &principal.on_behalf_of {
                Some(user) => user_context(storage, scope, user, Some(marker)).await,
                None => Err(format!(
                    "agent '{}' grants no rights of its own and acts for no user; \
                     refusing to run it with system rights",
                    principal.id
                )),
            }
        }
    }
}
