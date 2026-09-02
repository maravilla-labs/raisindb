// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `raisin.assets.signedUrl` — hand a media service a URL instead of bytes.
//!
//! # Why the engine mints it and not the function
//!
//! Staging a file for an out-of-process media service used to mean pulling the
//! bytes through the QuickJS isolate as base64: the buffer, its base64
//! expansion and the JS string are three copies of the file in the isolate's
//! heap at once, and a 1.5 MB mp4 was enough to end a production run with
//! `[JS] out of memory`. A URL is a few hundred bytes and the bytes stream
//! server-side, never entering the isolate at all.
//!
//! A function cannot produce that URL itself. `raisin.http.fetch` refuses
//! loopback, private and link-local addresses for EVERY function, so it cannot
//! call raisindb's own `raisin:sign` endpoint — and that refusal must not be
//! relaxed, because an allowlist there opens the address space for every
//! tenant's content code, not just ours. Same shape as `raisin.platform.hook`:
//! trusted server code does the reaching, the function names what it wants.
//!
//! # RLS
//!
//! A signed URL is a bearer grant: whoever holds it reads the bytes, with no
//! auth context of their own. So the mint must not exceed what the CALLER can
//! already read. The visibility check therefore goes through the same
//! `QueryContext` SQL path as `raisin.nodes.get`, carrying the function's auth
//! context, and RLS filters the row. A node the caller cannot SELECT produces
//! `NotFound` — the same answer as a node that does not exist, deliberately, so
//! the binding is not an existence oracle for rows RLS hides.
//!
//! Reaching for `storage.nodes().get_by_path()` first would have skipped RLS
//! entirely and minted a URL for anything in the workspace.

use std::sync::Arc;

use raisin_binary::BinaryStorage;
use raisin_error::{Error, Result};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use serde_json::{json, Value};

use super::query_context::QueryContext;
use super::sql_generator;
use crate::api::AssetSignedUrlCallback;

/// Default lifetime of a minted URL, in seconds.
///
/// Matches the `raisin:sign` HTTP endpoint's default. It is a machine-to-machine
/// grant for ONE fetch that the consumer performs immediately, not a browser
/// session — five minutes covers a slow queue hand-off and a retry without
/// leaving a live grant lying in a log for the rest of the day.
pub const DEFAULT_EXPIRES_IN_SECS: u64 = 300;

/// Ceiling on a caller-supplied lifetime, in seconds.
///
/// One hour, because the longest legitimate consumer is a media conversion that
/// queues behind other work before it fetches; anything beyond that is a
/// standing grant, and a signed URL cannot be revoked before it expires — there
/// is no revocation list, the signature alone is the authority. Clamped rather
/// than rejected: a caller asking for a day gets a working URL that expires in
/// an hour, which fails visibly at the fetch rather than at the mint.
pub const MAX_EXPIRES_IN_SECS: u64 = 3600;

/// Floor, so `expiresIn: 0` cannot mint a URL that is already dead.
const MIN_EXPIRES_IN_SECS: u64 = 1;

/// Create the `raisin.assets.signedUrl` callback.
///
/// `(workspace, nodeIdOrPath, options) -> { url, expiresAt, expiresIn, command,
/// property, path }`
pub fn create_asset_signed_url<S, B>(query_ctx: Arc<QueryContext<S, B>>) -> AssetSignedUrlCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(move |workspace: String, node_ref: String, options: Value| {
        let ctx = query_ctx.clone();

        Box::pin(async move {
            let command = options
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("display")
                .to_string();
            if !raisin_core::is_valid_asset_command(&command) {
                return Err(Error::Validation(format!(
                    "signedUrl command must be 'download' or 'display', got '{command}'"
                )));
            }

            let property = options
                .get("property")
                .and_then(|v| v.as_str())
                .filter(|p| !p.trim().is_empty())
                .unwrap_or(raisin_core::DEFAULT_ASSET_PROPERTY)
                .to_string();

            let expires_in = options
                .get("expiresIn")
                .and_then(|v| v.as_u64())
                .unwrap_or(DEFAULT_EXPIRES_IN_SECS)
                .clamp(MIN_EXPIRES_IN_SECS, MAX_EXPIRES_IN_SECS);

            // RLS gate, and the only one — see the module docs. A path and an id
            // are both accepted because a trigger is handed whichever the event
            // carried; requiring one spelling would put the lookup in every
            // function instead of here.
            let stmt = if node_ref.starts_with('/') {
                sql_generator::generate_select_by_path(&workspace, &node_ref)
            } else {
                sql_generator::generate_select_by_id(&workspace, &node_ref)
            };
            let row = ctx.execute_query(&stmt).await?.into_iter().next();
            let row = row.ok_or_else(|| {
                Error::NotFound(format!("Asset not found or not readable: {node_ref}"))
            })?;

            // The signature is over the PATH, so an id lookup still has to end
            // up holding one. Taking it from the row rather than from the caller
            // also means a caller who passed a path gets the stored spelling
            // signed, not their own.
            let node_path = row
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    Error::Internal(format!("Asset row for {node_ref} carries no path"))
                })?
                .to_string();

            // Shape check against the TYPED node. RLS has already admitted this
            // row, so a direct read grants nothing new here — and the JSON
            // projection cannot tell a Resource from any other object, while the
            // mount exception below is expressed in typed properties.
            let scope = StorageScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch, &workspace);
            let node = ctx
                .deps
                .storage
                .nodes()
                .get_by_path(scope, &node_path, None)
                .await?
                .ok_or_else(|| Error::NotFound(format!("Asset not found: {node_path}")))?;

            // A mount-owned asset whose cache has expired has NO `file`, and
            // that is a cache state rather than an absence: the serve handler
            // fetches the bytes from the provider when the URL is read.
            // Refusing here would reject the very request that refills the
            // cache. Same exception the `raisin:sign` endpoint makes, and only
            // for `file` — a missing `thumbnail` is a derived artifact that was
            // never made, and no fetch conjures one.
            let missing_but_fetchable = property == raisin_core::DEFAULT_ASSET_PROPERTY
                && !node.properties.contains_key(&property)
                && raisin_models::nodes::is_fetchable_mount_content(&node.properties);

            if !missing_but_fetchable {
                match node.properties.get(&property) {
                    Some(raisin_models::nodes::properties::PropertyValue::Resource(_)) => {}
                    Some(_) => {
                        return Err(Error::Validation(format!(
                            "Node's '{property}' property is not a Resource"
                        )))
                    }
                    None => {
                        return Err(Error::NotFound(format!(
                            "Node has no '{property}' property"
                        )))
                    }
                }
            }

            // Fail CLOSED on both of these. An unsigned or dev-key URL would be
            // handed to another process, and a RELATIVE one is not resolvable
            // there at all — a hard-coded host would be worse still, because it
            // would silently point every deployment at one of them.
            let secret = raisin_crypto::signing_secret_or_dev().ok_or_else(|| {
                Error::Validation(
                    "RAISINDB_SIGNING_SECRET must be set to mint signed asset URLs \
                     (or run with --dev-mode)"
                        .to_string(),
                )
            })?;
            let base_url = raisin_core::configured_public_base_url().ok_or_else(|| {
                Error::Validation(
                    "RAISINDB_BASE_URL must be set to this server's public origin \
                     before signedUrl can mint an absolute URL"
                        .to_string(),
                )
            })?;

            let expires = now_secs() + expires_in;
            let signed = raisin_core::build_signed_asset_url(
                &secret,
                &ctx.tenant_id,
                &ctx.repo_id,
                &ctx.branch,
                &workspace,
                &node_path,
                &property,
                &command,
                expires,
                Some(&base_url),
            );

            // The URL itself is NOT logged: it is a bearer grant, and a log
            // aggregator is exactly the place one gets replayed from.
            tracing::debug!(
                workspace = %workspace,
                node_path = %node_path,
                property = %property,
                command = %command,
                expires_in,
                "Minted a signed asset URL for an out-of-process consumer"
            );

            Ok(json!({
                "url": signed.url,
                "expiresAt": chrono::DateTime::from_timestamp(expires as i64, 0)
                    .map(|dt| dt.to_rfc3339()),
                "expiresIn": expires_in,
                "command": command,
                "property": property,
                "path": node_path,
            }))
        })
    })
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cap is the whole point of accepting `expiresIn` at all — a signed URL
    /// cannot be revoked before it expires, so an unclamped caller value is a
    /// standing grant minted by tenant code.
    #[test]
    fn expires_in_is_clamped_at_both_ends() {
        let clamp = |v: u64| v.clamp(MIN_EXPIRES_IN_SECS, MAX_EXPIRES_IN_SECS);
        assert_eq!(clamp(0), MIN_EXPIRES_IN_SECS);
        assert_eq!(clamp(86_400), MAX_EXPIRES_IN_SECS);
        assert_eq!(clamp(120), 120);
    }
}
