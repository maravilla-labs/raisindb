// SPDX-License-Identifier: BSL-1.1

//! `POST /api/integrations/content/{repo}/{branch}/{ws}/by-id/{node_id}` —
//! fetch the bytes behind one mount-owned `raisin:Asset`.
//!
//! Sync writes attachment METADATA only: a `raisin:Asset` child per attachment
//! carrying its name, mime type and size, and no bytes. Downloading every
//! attachment of every synced message in the sync hot loop would multiply a
//! mailbox import by whole documents, and most attachments are never opened.
//! `file == null` on a mount-owned asset therefore means exactly "not fetched
//! yet", and this endpoint is what changes that.
//!
//! Addressed by NODE, not by mount: the caller is a viewer that has an asset in
//! front of it, and the mount is recoverable from the node's own `__mount_id`.
//! Requiring the mount id here would make every caller resolve something the
//! engine resolves anyway, and get it wrong for an asset whose mount was
//! renamed.
//!
//! SYNCHRONOUS, not a job. It answers "someone is opening this attachment now";
//! routed through the queue the caller would get a job id and no bytes, and
//! would poll for a file that is usually a few hundred KB and one HTTP call
//! away.
//!
//! ## Authorization
//!
//! The engine's fetch runs as the SYNC ACTOR with a system auth context — it has
//! to, because it writes the node and reads the mount's encrypted credential.
//! That means this endpoint cannot inherit its authorization from the engine and
//! MUST do its own: the node is read first through the caller's own context, so
//! row-level security decides. A node the caller cannot see is a 404 and no
//! provider call is made. Skipping that would turn an attachment fetch into a
//! read oracle over every mount in the repo — with the mount's own credential
//! paying for it.
//!
//! Deliberately NOT admin-only, unlike the rest of this module. Opening an
//! attachment is an ordinary content read that ordinary users perform; gating it
//! on `admin` would leave every non-admin looking at a permanent "not fetched
//! yet" placeholder with no way to act on it.

#![cfg(feature = "storage-rocksdb")]

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

#[derive(Debug, Default, Deserialize)]
pub struct FetchContentQuery {
    /// Re-fetch a node that already carries a `file`.
    ///
    /// Off by default so two viewers opening the same attachment do not both
    /// download it — the default answer for an already-fetched node is
    /// `already_present`, which is an ordinary outcome and not an error.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize)]
pub struct FetchContentResponse {
    /// `"stored"` — the bytes were fetched and stored on this call.
    /// `"already_present"` — the node already had them; nothing was fetched.
    pub status: &'static str,
    /// Bytes stored, absent when nothing was fetched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<usize>,
    /// Binary-store key of the stored object, absent when nothing was fetched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_key: Option<String>,
}

pub async fn fetch_mount_content(
    State(state): State<AppState>,
    Path((repo, branch, ws, node_id)): Path<(String, String, String, String)>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Query(q): Query<FetchContentQuery>,
) -> Result<Json<FetchContentResponse>, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();

    // Authorization, and the ONLY authorization on this path — see the module
    // note. Read through the caller's own context so row-level security applies;
    // the engine's own read that follows is a system read and would see
    // everything.
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);
    if nodes_svc.get(&node_id).await?.is_none() {
        // 404 whether it is absent or merely invisible: distinguishing the two
        // would confirm the existence of a node the caller may not see.
        return Err(ApiError::not_found(format!("node {node_id} not found")));
    }

    let rocksdb = state.rocksdb_storage.as_ref().ok_or_else(|| {
        ApiError::internal("RocksDB backend required for virtual-mount content fetch")
    })?;
    // "This deployment cannot fetch" and "there is nothing to fetch" are
    // different answers, and only the first is a misconfiguration — so it is
    // reported rather than folded into an empty result.
    let handler = rocksdb.virtual_mount_sync_handler().ok_or_else(|| {
        ApiError::internal(
            "the virtual-mount sync engine is not running; attachment content cannot be fetched",
        )
    })?;

    // The two branches are NOT the same value and must not be conflated: `branch`
    // addresses the materialized asset, `config_branch` is where the mount and
    // integration nodes live. Passing the target branch for both resolves no
    // mount at all.
    let target = raisin_rocksdb::ContentTarget {
        tenant: tenant_id,
        repo: &repo,
        config_branch: super::CONFIG_BRANCH,
        branch: &branch,
        workspace: &ws,
        node_id: &node_id,
    };

    match handler.fetch_content(target, q.force).await? {
        raisin_rocksdb::ContentFetch::Stored { bytes, storage_key } => {
            Ok(Json(FetchContentResponse {
                status: "stored",
                bytes: Some(bytes),
                storage_key: Some(storage_key),
            }))
        }
        raisin_rocksdb::ContentFetch::AlreadyPresent => Ok(Json(FetchContentResponse {
            status: "already_present",
            bytes: None,
            storage_key: None,
        })),
    }
}
