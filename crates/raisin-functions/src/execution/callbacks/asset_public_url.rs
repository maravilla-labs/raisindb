// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `raisin.assets.publicUrl` — the anonymous address of a PUBLISHED asset.
//!
//! `(workspace, path, { branch? }) -> { url, origin, repo, branch, workspace, path }`
//!
//! The URL is `<tenant public origin>/api/repository/<repo>/<branch>/head/
//! <workspace><path>/raisin:display`, with no query string: no signature, no
//! expiry, so it can sit in a mail for years and the edge can cache it.
//!
//! # Security
//!
//! * **No origin from the caller.** The origin comes from operator
//!   configuration keyed by THIS execution's tenant id
//!   (`raisin_core::public_origin_for_tenant`). Nothing in `workspace`, `path`
//!   or `options` can influence the host, and no request header is consulted,
//!   which is also why it works from triggers, flows and queues.
//! * **No open redirect / injection.** The path is validated as a plain,
//!   normalised absolute path (no `.`/`..`, query, fragment, `%`, backslash,
//!   control characters) and every segment is percent-encoded; the workspace
//!   and repo are encoded too.
//! * **Branch allowlist.** Only `publish` unless the operator widens it with
//!   `RAISINDB_PUBLIC_ASSET_BRANCHES`. A working-copy (`main`) URL would
//!   advertise unpublished content as public.
//! * **Same answer for every caller.** The existence check reads storage
//!   directly, not through the caller's RLS, so the response never varies by
//!   who asks. It leaks nothing the URL itself would not: whether the
//!   anonymous request is served is decided by the serve handler, at read time.
//! * **Tenant isolation.** The scope is `(ctx.tenant_id, ctx.repo_id, branch,
//!   workspace)`; a function can only name assets of its own tenant/repo.
//! * **Fails closed.** Missing/invalid origin config, unmapped tenant, unknown
//!   asset, or a node without a `file` Resource all return an error, never a
//!   guessed URL.

use std::sync::Arc;

use raisin_binary::BinaryStorage;
use raisin_error::{Error, Result};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use serde_json::{json, Value};

use super::query_context::QueryContext;
use super::sql_generator;
use crate::api::AssetPublicUrlCallback;

/// Create the `raisin.assets.publicUrl` callback.
pub fn create_asset_public_url<S, B>(query_ctx: Arc<QueryContext<S, B>>) -> AssetPublicUrlCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(move |workspace: String, path: String, options: Value| {
        let ctx = query_ctx.clone();
        Box::pin(async move {
            let branch = options
                .get("branch")
                .and_then(|v| v.as_str())
                .filter(|b| !b.trim().is_empty())
                .unwrap_or("publish")
                .to_string();
            if !raisin_core::public_asset_branch_allowed(&branch) {
                return Err(Error::Validation(format!(
                    "publicUrl is only available for the 'publish' branch, not '{branch}'"
                )));
            }
            if workspace.trim().is_empty()
                || !workspace
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
            {
                return Err(Error::Validation(
                    "publicUrl: invalid workspace".to_string(),
                ));
            }
            raisin_core::validate_public_asset_path(&path)
                .map_err(|why| Error::Validation(format!("publicUrl: {why}")))?;

            // Origin first: an unconfigured server must say so even for an
            // asset that does not exist, so a misconfiguration is not
            // mistaken for a missing file.
            let origin = raisin_core::public_origin_for_tenant(&ctx.tenant_id)
                .map_err(|e| Error::Validation(format!("publicUrl: {e}")))?;

            // Three gates, ALL evaluated before any answer so the outcome (and
            // its cost) does not reveal which one failed:
            //   1. the CALLER's own row-level security on the requested branch;
            //   2. the tenant's ANONYMOUS role on that branch (same resolution
            //      the HTTP middleware uses), because the URL is served to
            //      anonymous visitors and must never 404 or expose a node the
            //      tenant did not make public;
            //   3. the repo-level anonymous switch (`anonymous_enabled == false`).
            let stmt = sql_generator::generate_select_by_path(&workspace, &path);
            let caller_ctx = QueryContext::new(
                ctx.deps.clone(),
                ctx.tenant_id.clone(),
                ctx.repo_id.clone(),
                branch.clone(),
                ctx.auth_context.clone(),
            );
            let caller_sees =
                matches!(caller_ctx.execute_query(&stmt).await, Ok(r) if !r.is_empty());

            let anon_sees = if anonymous_enabled(&ctx).await {
                match anonymous_auth(&ctx).await {
                    Some(anon) => {
                        let anon_ctx = QueryContext::new(
                            ctx.deps.clone(),
                            ctx.tenant_id.clone(),
                            ctx.repo_id.clone(),
                            branch.clone(),
                            Some(anon),
                        );
                        matches!(anon_ctx.execute_query(&stmt).await, Ok(r) if !r.is_empty())
                    }
                    None => false,
                }
            } else {
                false
            };

            let scope = StorageScope::new(&ctx.tenant_id, &ctx.repo_id, &branch, &workspace);
            let has_file = if caller_sees && anon_sees {
                match ctx
                    .deps
                    .storage
                    .nodes()
                    .get_by_path(scope, &path, None)
                    .await
                {
                    Ok(Some(node)) => {
                        matches!(
                            node.properties.get(raisin_core::DEFAULT_ASSET_PROPERTY),
                            Some(raisin_models::nodes::properties::PropertyValue::Resource(_))
                        ) || raisin_models::nodes::is_fetchable_mount_content(&node.properties)
                    }
                    _ => false,
                }
            } else {
                false
            };
            if !may_mint(caller_sees, anon_sees, has_file) {
                return Err(Error::NotFound(NOT_PUBLIC.to_string()));
            }

            let url = raisin_core::build_public_asset_url(
                &origin,
                &ctx.repo_id,
                &branch,
                &workspace,
                &path,
            );
            Ok(json!({
                "url": url,
                "origin": origin,
                "repo": ctx.repo_id,
                "branch": branch,
                "workspace": workspace,
                "path": path,
            }))
        })
    })
}

/// The one answer for "missing", "caller cannot read it", "anonymous cannot
/// read it", "not on that branch" and "no file": no existence oracle.
const NOT_PUBLIC: &str = "No public asset at that path";

/// The whole policy, pure so tests pin it: a URL is minted only for an asset
/// the caller may read AND anonymous visitors may read AND that has bytes.
fn may_mint(caller_sees: bool, anon_sees: bool, has_file: bool) -> bool {
    caller_sees && anon_sees && has_file
}

/// Repo-level anonymous switch; an explicit `false` closes it, absence leaves
/// the decision to the anonymous role's grants (empty grants see nothing).
async fn anonymous_enabled<S, B>(ctx: &QueryContext<S, B>) -> bool
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let scope = StorageScope::new(&ctx.tenant_id, &ctx.repo_id, "main", "raisin:system");
    let path = format!("/config/repos/{}", ctx.repo_id);
    match ctx
        .deps
        .storage
        .nodes()
        .get_by_path(scope, &path, None)
        .await
    {
        Ok(Some(node)) if node.node_type == "raisin:RepoAuthConfig" => !matches!(
            node.properties.get("anonymous_enabled"),
            Some(raisin_models::nodes::properties::PropertyValue::Boolean(
                false
            ))
        ),
        _ => true,
    }
}

/// The anonymous principal exactly as the HTTP middleware builds it.
async fn anonymous_auth<S, B>(ctx: &QueryContext<S, B>) -> Option<raisin_models::auth::AuthContext>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let perms = raisin_core::PermissionService::new(ctx.deps.storage.clone())
        .resolve_anonymous_user(&ctx.tenant_id, &ctx.repo_id, "main")
        .await
        .ok()??;
    let user_id = perms.user_id.clone();
    Some(raisin_models::auth::AuthContext::anonymous_user(&user_id).with_permissions(perms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_by_caller_is_refused_like_a_missing_one() {
        assert!(!may_mint(false, true, true));
        assert!(!may_mint(false, false, false));
    }

    #[test]
    fn published_but_outside_the_anonymous_grant_is_closed() {
        assert!(!may_mint(true, false, true));
    }

    #[test]
    fn readable_by_both_with_a_file_mints() {
        assert!(may_mint(true, true, true));
        assert!(!may_mint(true, true, false));
    }
}
