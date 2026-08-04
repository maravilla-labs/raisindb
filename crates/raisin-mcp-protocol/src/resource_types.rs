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

//! Resource URIs and payload types.
//!
//! The wire types only. The provider that actually serves RaisinDB nodes as
//! resources stays in `raisin-mcp`, because it needs a storage-backed
//! `FunctionApi` — these types are shared with the client, which needs to
//! *parse* a resource without being able to serve one.

use serde::{Deserialize, Serialize};

use crate::error::{McpError, Result};

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
