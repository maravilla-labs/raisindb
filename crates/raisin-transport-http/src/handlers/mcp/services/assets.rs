// SPDX-License-Identifier: BSL-1.1

//! [`AssetReader`] backed by the RLS-scoped node service + binary storage.

use std::future::Future;
use std::pin::Pin;

use raisin_binary::BinaryStorage;
use raisin_mcp::{AssetBytes, AssetReader, McpError, McpIdentity};
use raisin_models::nodes::properties::PropertyValue;

use crate::state::AppState;

/// Reads `raisin:Asset` bytes for the MCP engine, RLS-scoped to the caller.
///
/// Mirrors the inline byte-serving path of the repository handler
/// (`handlers/repo/helpers.rs::get_property`): it resolves the node at
/// `(workspace, path)` through [`AppState::node_service_for_context`] — so RLS
/// denies a node the caller may not see — reads the `file` [`Resource`] property,
/// and only then fetches the underlying bytes from binary storage by
/// `storage_key`. The branch is taken from the calling identity; the tenant and
/// repo are pinned at construction.
pub(in crate::handlers::mcp) struct HttpAssetReader {
    state: AppState,
    tenant_id: String,
    repo: String,
    /// The caller's RESOLVED auth context, as resolved once for the request.
    ///
    /// Not rebuilt from the [`McpIdentity`]: that reconstruction keeps only the
    /// subject and roles, dropping groups, the local user id and — decisively —
    /// the resolved permissions, which RLS treats as deny-all. The symptom is
    /// silent, because a denied node is `None` and `None` reads as "not found".
    auth: Option<raisin_models::auth::AuthContext>,
}

impl HttpAssetReader {
    /// Bind an asset reader to the request's tenant / repo scope.
    pub(in crate::handlers::mcp) fn new(
        state: AppState,
        tenant_id: impl Into<String>,
        repo: impl Into<String>,
        auth: Option<raisin_models::auth::AuthContext>,
    ) -> Self {
        Self {
            state,
            tenant_id: tenant_id.into(),
            repo: repo.into(),
            auth,
        }
    }

    async fn run(
        &self,
        identity: &McpIdentity,
        workspace: &str,
        path: &str,
    ) -> raisin_mcp::Result<AssetBytes> {
        // Prefer the resolved context; fall back to the identity only when the
        // request carried none (an anonymous caller), where there is nothing to
        // resolve and the reconstruction is equivalent.
        let auth = self
            .auth
            .clone()
            .unwrap_or_else(|| identity.to_auth_context());

        let nodes_svc = self.state.node_service_for_context(
            &self.tenant_id,
            &self.repo,
            &identity.branch,
            workspace,
            Some(auth),
        );

        // RLS-scoped resolve: a node the caller cannot read surfaces as `None`.
        let node = nodes_svc
            .get_by_path(path)
            .await
            .map_err(|e| McpError::protocol(format!("resolving asset `{path}`: {e}")))?
            .ok_or_else(|| McpError::not_found(format!("asset not found: {path}")))?;

        let resource = match node.properties.get("file") {
            Some(PropertyValue::Resource(resource)) => resource,
            _ => {
                return Err(McpError::not_found(format!(
                    "node at `{path}` has no `file` resource"
                )))
            }
        };

        let storage_key = resource
            .metadata
            .as_ref()
            .and_then(|m| m.get("storage_key"))
            .and_then(|v| match v {
                PropertyValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .ok_or_else(|| McpError::not_found(format!("asset at `{path}` has no storage_key")))?;

        let bytes =
            self.state.bin.get(&storage_key).await.map_err(|e| {
                McpError::protocol(format!("reading asset bytes for `{path}`: {e}"))
            })?;

        let mime_type = resource.mime_type.clone().unwrap_or_else(|| {
            mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string()
        });

        Ok(AssetBytes {
            bytes: bytes.to_vec(),
            mime_type,
        })
    }
}

impl AssetReader for HttpAssetReader {
    fn read_asset<'a>(
        &'a self,
        identity: &'a McpIdentity,
        workspace: &'a str,
        path: &'a str,
    ) -> Pin<Box<dyn Future<Output = raisin_mcp::Result<AssetBytes>> + Send + 'a>> {
        Box::pin(self.run(identity, workspace, path))
    }
}
