// SPDX-License-Identifier: BSL-1.1

//! Bridge from the resolved request [`AuthContext`] to an [`McpIdentity`].
//!
//! The [`AuthContext`] is resolved either by the shared auth middleware (login
//! tokens, admin tokens, API keys → request extensions) or, for OAuth 2.1
//! resource-bound tokens, by [`super::auth::resolve_mcp_auth`]. This module
//! projects that context onto the transport-agnostic [`McpIdentity`] the MCP
//! engine gates tool access with — it does **not** re-validate any token.

use raisin_mcp::McpIdentity;
use raisin_models::auth::AuthContext;

/// Build the [`McpIdentity`] for an MCP session from the request context.
///
/// `auth` is the [`AuthContext`] resolved for the caller (or `None`/anonymous
/// when no usable credential was supplied). The granted scopes are derived from
/// the caller's effective roles plus group memberships, so a `raisin:McpServer`
/// (or custom tool) that requires a scope can name a role or group id. System
/// callers bypass scope gating and RLS entirely.
///
/// `consented_scopes`, when `Some`, is the narrowed set an OAuth 2.1 resource
/// owner consented to (the token's `scope` claim). The effective grants are then
/// the **intersection** of the caller's roles/groups with that set — consent can
/// only narrow a caller's reach, never widen it. `None` (login token, API key,
/// anonymous) leaves the full roles/groups as grants. RLS always uses the real
/// roles regardless, since data access is the database's own boundary.
pub(super) fn mcp_identity_from_auth(
    auth: Option<&AuthContext>,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    consented_scopes: Option<&[String]>,
) -> McpIdentity {
    let Some(ctx) = auth else {
        return McpIdentity::anonymous(repo)
            .with_tenant(tenant_id)
            .with_branch(branch)
            .with_workspace(workspace);
    };

    if ctx.is_system {
        // System context bypasses RLS and every scope gate.
        return McpIdentity::new("system", repo)
            .with_tenant(tenant_id)
            .with_branch(branch)
            .with_workspace(workspace)
            .as_system();
    }

    if ctx.is_anonymous || ctx.user_id.is_none() {
        let mut identity = McpIdentity::anonymous(repo)
            .with_tenant(tenant_id)
            .with_branch(branch)
            .with_workspace(workspace);
        identity = apply_grants(identity, ctx, consented_scopes);
        return identity;
    }

    let subject = ctx.user_id.clone().unwrap_or_default();
    let mut identity = McpIdentity::new(subject, repo)
        .with_tenant(tenant_id)
        .with_branch(branch)
        .with_workspace(workspace)
        .with_roles(ctx.roles.clone());
    identity = apply_grants(identity, ctx, consented_scopes);
    identity
}

/// Grant scopes from the caller's effective roles and groups.
///
/// Both roles and groups become scope strings so a server/tool requirement can
/// reference either. Roles are also retained on the identity for RLS. When
/// `consented_scopes` is `Some`, the grants are intersected with it so an OAuth
/// token only confers what the resource owner consented to.
fn apply_grants(
    identity: McpIdentity,
    ctx: &AuthContext,
    consented_scopes: Option<&[String]>,
) -> McpIdentity {
    let mut scopes = ctx
        .roles
        .iter()
        .chain(ctx.groups.iter())
        .cloned()
        .collect::<Vec<_>>();

    if let Some(consented) = consented_scopes {
        scopes.retain(|s| consented.iter().any(|c| c == s));
    }

    identity.with_scopes(scopes)
}
