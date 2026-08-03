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

//! `tools/list` and `tools/call`.

use serde_json::{json, Value};

use super::{Dispatcher, LIST_TTL_MS};
use crate::error::{McpError, Result};
use crate::identity::McpIdentity;
use crate::protocol::{CallToolParams, CallToolResult, CACHE_SCOPE_PRIVATE, RESULT_TYPE_COMPLETE};

impl Dispatcher {
    pub(super) fn handle_tools_list(&self, identity: &McpIdentity) -> Result<Value> {
        let tools = self.registry.visible_descriptors(identity);
        // MCP Apps (SEP-1865): a ui-bound tool advertises its widget as a
        // predeclared `ui://` resource via `_meta.ui.resourceUri`; Apps-capable
        // hosts fetch it with resources/read and deliver tool results to the
        // rendered view themselves. The deprecated flat key is included for
        // hosts that still read the pre-GA shape.
        let mut entries = Vec::with_capacity(tools.len());
        for tool in tools {
            let mut value = serde_json::to_value(&tool)?;
            if let Some(ui) = &tool.ui {
                let uri = self.ui_uri_for(identity, ui);
                let mut meta = json!({ "resourceUri": uri });
                if let Some(visibility) = &ui.visibility {
                    meta["visibility"] = json!(visibility);
                }
                value["_meta"] = json!({ "ui": meta, "ui/resourceUri": uri });
            }
            entries.push(value);
        }
        let mut result = json!({
            "resultType": RESULT_TYPE_COMPLETE,
            "tools": entries,
            "ttlMs": LIST_TTL_MS,
            "cacheScope": CACHE_SCOPE_PRIVATE,
        });
        self.attach_server_info(&mut result);
        Ok(result)
    }

    pub(super) async fn handle_tools_call(
        &self,
        identity: &McpIdentity,
        request: &crate::protocol::JsonRpcRequest,
    ) -> Result<Value> {
        let params: CallToolParams = request.decode_params()?;

        let tool = self
            .registry
            .get(&params.name)
            .ok_or_else(|| McpError::not_found(format!("unknown tool: {}", params.name)))?;

        // Per-tool scope gate.
        let descriptor = tool.descriptor();
        let missing = identity.missing_scopes(&descriptor.scopes);
        if !missing.is_empty() {
            return Err(McpError::unauthorized(format!(
                "tool `{}` requires missing scopes: {}",
                params.name,
                missing.join(", ")
            )));
        }

        // Map a function-level failure onto an MCP `isError` result rather than a
        // protocol error; everything else propagates as a JSON-RPC error.
        match tool.call(identity, params.arguments).await {
            // A tool that declares an `outputSchema` returns a result conforming
            // to it, surfaced as `structuredContent` alongside the content block.
            Ok(value) => {
                // MCP Apps (SEP-1865): a tool result carries DATA only. The
                // widget is a predeclared `ui://` resource the host discovers
                // via `_meta.ui.resourceUri`, fetches with resources/read, and
                // feeds through `ui/notifications/tool-result` — nothing UI is
                // embedded here.
                let result = if descriptor.output_schema.is_some() {
                    CallToolResult::json_structured(value)
                } else {
                    CallToolResult::json(value)
                };
                Ok(serde_json::to_value(result)?)
            }
            Err(McpError::FunctionFailed(message)) => {
                Ok(serde_json::to_value(CallToolResult::error(message))?)
            }
            Err(err) => Err(err),
        }
    }
}
