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

            let scope = StorageScope::new(&ctx.tenant_id, &ctx.repo_id, &branch, &workspace);
            let node = ctx
                .deps
                .storage
                .nodes()
                .get_by_path(scope, &path, None)
                .await?
                .ok_or_else(|| {
                    Error::NotFound(format!("No asset at {workspace}:{path} on '{branch}'"))
                })?;
            // Must be a file-bearing node (an asset), or a mount-owned asset
            // the serve handler can fetch.
            let has_file = matches!(
                node.properties.get(raisin_core::DEFAULT_ASSET_PROPERTY),
                Some(raisin_models::nodes::properties::PropertyValue::Resource(_))
            ) || raisin_models::nodes::is_fetchable_mount_content(&node.properties);
            if !has_file {
                return Err(Error::NotFound(format!(
                    "{workspace}:{path} on '{branch}' has no file"
                )));
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
