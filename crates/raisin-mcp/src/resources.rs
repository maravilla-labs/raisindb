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
}

impl NodeResourceProvider {
    /// Wrap a data backend as a read-only resource provider.
    pub fn new(backend: Arc<dyn FunctionApi>) -> Self {
        Self {
            backend,
            events: None,
            assets: None,
        }
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
    /// The URI's workspace must match the caller's active workspace.
    pub async fn read(&self, identity: &McpIdentity, uri: &str) -> Result<ResourceContents> {
        let (workspace, path) = parse_resource_uri(uri)?;
        self.check_workspace(identity, &workspace)?;

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

    /// Reject reads / subscriptions that cross out of the session's workspace.
    fn check_workspace(&self, identity: &McpIdentity, workspace: &str) -> Result<()> {
        if workspace != identity.workspace {
            return Err(McpError::unauthorized(format!(
                "workspace mismatch: resource `{workspace}` not in session scope `{}`",
                identity.workspace
            )));
        }
        Ok(())
    }
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
