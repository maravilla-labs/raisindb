// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! MCP resources: read-only, addressable content plus live subscriptions.
//!
//! Where a [`Tool`](crate::registry::Tool) is an action, a resource is a named
//! blob a client can read. Resources are addressed by a `raisin://` URI whose
//! authority is the workspace and whose path maps onto a node path:
//! `raisin://{workspace}/{node/path}`.
//!
//! [`NodeResourceProvider::read`] serves the current node content (RLS-scoped
//! via [`FunctionApi`]); [`NodeResourceProvider::subscribe`] bridges the
//! [`EventSource`] into a stream of `resources/updated` notifications filtered to
//! the subscribed URI.

use std::sync::Arc;

use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use raisin_functions::FunctionApi;

use crate::error::{McpError, Result};
use crate::identity::McpIdentity;
use crate::protocol::ResourceUpdatedNotification;
use crate::services::{NodeChange, SharedAssetReader, SharedEventSource};

/// URI scheme used to address RaisinDB content as MCP resources.
pub const RESOURCE_SCHEME: &str = "raisin";

/// Descriptor for a resource, returned by `resources/list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDescriptor {
    /// Fully-qualified `raisin://` URI of the resource.
    pub uri: String,
    /// Display name.
    pub name: String,
    /// Optional human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// MIME type of the resource contents.
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

/// The decoded contents of a resource, returned by `resources/read`.
///
/// Mirrors MCP's `TextResourceContents` / `BlobResourceContents`: `text` carries
/// UTF-8 payloads (JSON node contents, an HTML widget, a `text/uri-list`), and
/// `blob` carries base64-encoded binary payloads (an image, a PDF). Exactly one
/// is populated for a given read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceContents {
    /// URI the contents were read from.
    pub uri: String,
    /// MIME type of `text` / `blob`.
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    /// Text body (JSON for node contents, HTML/uri-list for widget resources).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Base64-encoded binary body, for byte-for-byte asset reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

/// Build a `raisin://` resource URI from a workspace and node path.
pub fn resource_uri(workspace: &str, path: &str) -> String {
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    format!("{RESOURCE_SCHEME}://{workspace}/{trimmed}")
}

/// Split a `raisin://{workspace}/{path}` URI back into its components.
///
/// Returns the workspace authority and the absolute node path.
pub fn parse_resource_uri(uri: &str) -> Result<(String, String)> {
    let prefix = format!("{RESOURCE_SCHEME}://");
    let rest = uri
        .strip_prefix(&prefix)
        .ok_or_else(|| McpError::invalid_params(format!("not a {RESOURCE_SCHEME} URI: {uri}")))?;
    match rest.split_once('/') {
        Some((workspace, path)) => {
            if workspace.is_empty() {
                return Err(McpError::invalid_params(format!(
                    "missing workspace in URI: {uri}"
                )));
            }
            Ok((workspace.to_string(), format!("/{path}")))
        }
        // `raisin://{workspace}` with no path addresses the workspace root.
        None => {
            if rest.is_empty() {
                return Err(McpError::invalid_params(format!(
                    "missing workspace in URI: {uri}"
                )));
            }
            Ok((rest.to_string(), "/".to_string()))
        }
    }
}

/// Read-only provider that serves RaisinDB nodes as MCP resources and bridges
/// live node-change events into resource-update notifications.
pub struct NodeResourceProvider {
    backend: Arc<dyn FunctionApi>,
    events: Option<SharedEventSource>,
    assets: Option<SharedAssetReader>,
    exposed_workspaces: Vec<String>,
}

impl NodeResourceProvider {
    /// Wrap a data backend as a read-only resource provider.
    pub fn new(backend: Arc<dyn FunctionApi>) -> Self {
        Self {
            backend,
            events: None,
            assets: None,
            exposed_workspaces: Vec::new(),
        }
    }

    /// The workspaces this server exposes as resources — the server
    /// descriptor's `data.workspaces`.
    ///
    /// Without this the provider admits only the session's ACTIVE workspace,
    /// which is `data.workspaces` first entry. `resources/list` advertises a
    /// root for EVERY declared workspace, so every root after the first was
    /// advertised and then refused on read:
    ///
    /// ```text
    /// Unauthorized: workspace mismatch: resource `assets` not in session scope `stories`
    /// ```
    ///
    /// A server must not offer what it will not serve. Leaving this empty keeps
    /// the old active-workspace-only behaviour, so a caller that does not set it
    /// is unaffected.
    pub fn with_exposed_workspaces(mut self, workspaces: Vec<String>) -> Self {
        self.exposed_workspaces = workspaces;
        self
    }

    /// Attach an event source, enabling [`subscribe`](Self::subscribe).
    pub fn with_events(mut self, events: SharedEventSource) -> Self {
        self.events = Some(events);
        self
    }

    /// Attach an asset reader, enabling raw-byte
    /// [`read_asset`](Self::read_asset) blob reads.
    pub fn with_asset_reader(mut self, assets: SharedAssetReader) -> Self {
        self.assets = Some(assets);
        self
    }

    /// Read a `raisin:Asset`'s raw bytes as a base64 `blob` resource.
    ///
    /// Unlike [`read`](Self::read) (which returns a node's properties as JSON
    /// `text`), this serves the asset byte-for-byte — the read the MCP-UI
    /// `mode: html` path and any client fetching an uploaded image/PDF need. The
    /// read is RLS-scoped by the [`AssetReader`](crate::services::AssetReader)
    /// implementation before any bytes are fetched.
    pub async fn read_asset(
        &self,
        identity: &McpIdentity,
        workspace: &str,
        path: &str,
    ) -> Result<ResourceContents> {
        use base64::Engine as _;

        let assets = self
            .assets
            .as_ref()
            .ok_or_else(|| McpError::not_found("asset reads are not enabled"))?;
        self.check_workspace(identity, workspace)?;

        let asset = assets.read_asset(identity, workspace, path).await?;
        Ok(ResourceContents {
            uri: resource_uri(workspace, path),
            mime_type: asset.mime_type,
            text: None,
            blob: Some(base64::engine::general_purpose::STANDARD.encode(asset.bytes)),
        })
    }

    /// Whether this provider can serve `resources/subscribe`.
    pub fn supports_subscribe(&self) -> bool {
        self.events.is_some()
    }

    /// Read a single resource by URI for the given identity.
    ///
    /// The URI's workspace must be one this server exposes (see
    /// [`check_workspace`](Self::check_workspace)).
    ///
    /// A workspace ROOT (`raisin://{ws}/`) reads as a LISTING of its top-level
    /// children, not as a node. There is no node at `/`, so the plain
    /// `node_get` this used to do could only ever 404 — while `resources/list`
    /// advertises exactly those roots, describing them as "Browse nodes in the
    /// `{ws}` workspace by path". Every advertised root was therefore
    /// unopenable: the resource showed up in the client and failed when
    /// clicked. Listing is what "browse" was already promising.
    pub async fn read(&self, identity: &McpIdentity, uri: &str) -> Result<ResourceContents> {
        let (workspace, path) = parse_resource_uri(uri)?;
        self.check_workspace(identity, &workspace)?;

        if is_root_path(&path) {
            let children = self
                .backend
                .node_get_children(&workspace, "/", CHILDREN_PAGE_LIMIT)
                .await?;
            let listing = serde_json::json!({
                "workspace": workspace,
                "path": "/",
                "children": children,
            });
            return Ok(ResourceContents {
                uri: uri.to_string(),
                mime_type: "application/json".to_string(),
                text: Some(serde_json::to_string(&listing)?),
                blob: None,
            });
        }

        let node: Option<Value> = self.backend.node_get(&workspace, &path).await?;
        let value =
            node.ok_or_else(|| McpError::not_found(format!("resource not found: {uri}")))?;

        Ok(ResourceContents {
            uri: uri.to_string(),
            mime_type: "application/json".to_string(),
            text: Some(serde_json::to_string(&value)?),
            blob: None,
        })
    }

    /// Subscribe to change notifications for resources matching `uri`.
    ///
    /// The returned stream yields a [`ResourceUpdatedNotification`] each time a
    /// node under the subscribed URI's workspace and path prefix changes. The
    /// `uri` may address a single node (`.../page`) or a subtree prefix; both the
    /// exact path and any descendant path match.
    pub fn subscribe(
        &self,
        identity: &McpIdentity,
        uri: &str,
    ) -> Result<impl Stream<Item = ResourceUpdatedNotification> + Send + 'static> {
        let events = self
            .events
            .as_ref()
            .ok_or_else(|| McpError::not_found("resource subscriptions are not enabled"))?;

        let (workspace, path) = parse_resource_uri(uri)?;
        self.check_workspace(identity, &workspace)?;

        let branch = identity.branch.clone();
        let prefix = normalize_prefix(&path);
        let stream = events.subscribe(identity);

        Ok(stream.filter_map(move |change: NodeChange| {
            let matched = change.workspace == workspace
                && change.branch == branch
                && change_path_matches(&change, &prefix);
            let notification = matched.then(|| ResourceUpdatedNotification {
                uri: resource_uri(&change.workspace, change.path.as_deref().unwrap_or("/")),
            });
            async move { notification }
        }))
    }

    /// Reject reads / subscriptions for workspaces this server does not expose.
    ///
    /// Permitted set = the declared `data.workspaces` (see
    /// [`with_exposed_workspaces`](Self::with_exposed_workspaces)), falling back
    /// to the session's active workspace when none were declared. RLS still
    /// applies underneath every read; this gate only decides which workspaces
    /// the SERVER offers, and it must agree with what `resources/list`
    /// advertises or the server refuses its own listing.
    fn check_workspace(&self, identity: &McpIdentity, workspace: &str) -> Result<()> {
        let permitted = if self.exposed_workspaces.is_empty() {
            workspace == identity.workspace
        } else {
            self.exposed_workspaces.iter().any(|w| w == workspace)
        };
        if !permitted {
            let scope = if self.exposed_workspaces.is_empty() {
                identity.workspace.clone()
            } else {
                self.exposed_workspaces.join(", ")
            };
            return Err(McpError::unauthorized(format!(
                "workspace mismatch: resource `{workspace}` not in session scope `{scope}`"
            )));
        }
        Ok(())
    }
}

/// How many children a workspace-root listing returns.
///
/// A root read is a browse entry point, not a bulk export; an unbounded listing
/// of a large workspace would be both slow and useless in a client that renders
/// it as one blob. Callers that need more use the data tools, which paginate.
const CHILDREN_PAGE_LIMIT: Option<u32> = Some(200);

/// Whether `path` addresses the workspace root rather than a node.
fn is_root_path(path: &str) -> bool {
    path.is_empty() || path == "/"
}

/// Normalize a path into a prefix used for subtree matching (no trailing slash).
fn normalize_prefix(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Whether a change's path is the prefix itself or a descendant of it.
fn change_path_matches(change: &NodeChange, prefix: &str) -> bool {
    if prefix == "/" {
        return true;
    }
    match change.path.as_deref() {
        Some(path) => path == prefix || path.starts_with(&format!("{prefix}/")),
        None => false,
    }
}

#[cfg(test)]
mod exposure_tests {
    use super::*;
    use std::sync::Arc;

    fn backend() -> Arc<dyn FunctionApi> {
        Arc::new(raisin_functions::api::MockFunctionApi::new(
            serde_json::json!({}),
        ))
    }

    fn provider(exposed: &[&str]) -> NodeResourceProvider {
        // `check_workspace` and `is_root_path` are pure; the backend is never
        // reached by these assertions, so the mock API is enough to pin the
        // gate itself.
        NodeResourceProvider::new(backend())
            .with_exposed_workspaces(exposed.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn every_declared_workspace_is_readable_not_just_the_active_one() {
        // The regression: `resources/list` advertises a root per declared
        // workspace while reads admitted only `identity.workspace`, which is
        // `data.workspaces` FIRST entry — so Studio advertised stories/tags/
        // assets and refused two of the three with
        // "workspace mismatch: resource `assets` not in session scope `stories`".
        let identity = McpIdentity::anonymous("repo").with_workspace("stories");
        let p = provider(&["stories", "tags", "assets"]);
        for ws in ["stories", "tags", "assets"] {
            assert!(
                p.check_workspace(&identity, ws).is_ok(),
                "{ws} must be readable"
            );
        }
        assert!(
            p.check_workspace(&identity, "secrets").is_err(),
            "an undeclared workspace must still be refused"
        );
    }

    #[test]
    fn no_declaration_falls_back_to_the_active_workspace() {
        let identity = McpIdentity::anonymous("repo").with_workspace("stories");
        let p = NodeResourceProvider::new(backend());
        assert!(p.check_workspace(&identity, "stories").is_ok());
        assert!(p.check_workspace(&identity, "tags").is_err());
    }

    #[test]
    fn workspace_roots_are_recognised_as_listings() {
        // A root has no node behind it, so reading one as a node could only
        // 404 — which is what made every advertised root unopenable.
        assert!(is_root_path("/"));
        assert!(is_root_path(""));
        assert!(!is_root_path("/blueprints"));
    }
}
