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

//! `resources/*` and the subscription methods.

use serde_json::{json, Value};

use super::{Dispatcher, LIST_TTL_MS, READ_TTL_MS, UI_RESOURCE_SCHEME};
use crate::error::{McpError, Result};
use crate::identity::McpIdentity;
use crate::protocol::{
    ReadResourceParams, ReadResourceResult, SubscribeResourceParams, SubscriptionFilter,
    SubscriptionsListenParams, CACHE_SCOPE_PRIVATE, RESULT_TYPE_COMPLETE, UI_MIME_TYPE,
};
use crate::resources::ResourceContents;

impl Dispatcher {
    pub(super) fn handle_resources_list(&self, identity: &McpIdentity) -> Result<Value> {
        // The resource set is open-ended (any node path); advertise the workspace
        // roots the server's data policy exposes as browsable entry points.
        let mut resources = Vec::new();
        if self.resources.is_some() {
            for workspace in &self.descriptor.data_policy.workspaces {
                resources.push(serde_json::to_value(
                    crate::resources::ResourceDescriptor {
                        uri: crate::resources::resource_uri(workspace, "/"),
                        name: format!("{workspace} (workspace root)"),
                        description: Some(format!(
                            "Browse nodes in the `{workspace}` workspace by path."
                        )),
                        mime_type: "application/json".to_string(),
                    },
                )?);
            }
        }
        // MCP Apps (SEP-1865): predeclare each ui-bound tool's widget so hosts
        // can review and prefetch it. One SPA shared by several tools appears
        // once (deduped by resolved URI).
        let mut seen = std::collections::HashSet::new();
        for tool in self.registry.visible_descriptors(identity) {
            let Some(ui) = &tool.ui else { continue };
            let uri = self.ui_uri_for(identity, ui);
            if !seen.insert(uri.clone()) {
                continue;
            }
            let mut entry = json!({
                "uri": uri,
                "name": ui.name.clone().unwrap_or_else(|| tool.name.clone()),
                "mimeType": UI_MIME_TYPE,
            });
            if let Some(description) = &ui.description {
                entry["description"] = json!(description);
            }
            let meta = self.ui_resource_meta(ui);
            if meta.as_object().is_some_and(|m| !m.is_empty()) {
                entry["_meta"] = json!({ "ui": meta });
            }
            resources.push(entry);
        }
        let mut result = json!({
            "resultType": RESULT_TYPE_COMPLETE,
            "resources": resources,
            "ttlMs": LIST_TTL_MS,
            "cacheScope": CACHE_SCOPE_PRIVATE,
        });
        self.attach_server_info(&mut result);
        Ok(result)
    }

    /// `resources/templates/list` — the parameterized half of the resource
    /// space, as RFC 6570 URI templates.
    ///
    /// Standard MCP since 2024-11-05 and part of the `resources` capability, so
    /// a client that sees `resources` advertised may call it during discovery.
    /// This server answered `-32601 unknown method`, and a client treating that
    /// as a fatal discovery error never reached `resources/read` — so a
    /// perfectly good widget was never fetched, and the host reported it as a
    /// failure to load the app. Implementing it is not optional once the
    /// capability is advertised: for a server with no templates the correct
    /// answer is an empty list, never an error.
    ///
    /// Not identity-scoped: a template describes the SHAPE of the uri space,
    /// not its contents. Reading through one still goes through
    /// `resources/read`, where RLS applies per node.
    pub(super) fn handle_resource_templates_list(&self) -> Result<Value> {
        let mut templates = Vec::new();
        if self.resources.is_some() {
            for workspace in &self.descriptor.data_policy.workspaces {
                templates.push(json!({
                    // `{+path}` reserves `/` so a whole node path substitutes
                    // as one variable rather than being percent-encoded.
                    "uriTemplate": format!("{}://{workspace}/{{+path}}", crate::resources::RESOURCE_SCHEME),
                    "name": format!("{workspace} node"),
                    "description": format!(
                        "A node at a given path in the `{workspace}` workspace."
                    ),
                    "mimeType": "application/json",
                }));
            }
        }
        let mut result = json!({
            "resultType": RESULT_TYPE_COMPLETE,
            "resourceTemplates": templates,
            "ttlMs": LIST_TTL_MS,
            "cacheScope": CACHE_SCOPE_PRIVATE,
        });
        self.attach_server_info(&mut result);
        Ok(result)
    }

    pub(super) async fn handle_resources_read(
        &self,
        identity: &McpIdentity,
        request: &crate::protocol::JsonRpcRequest,
    ) -> Result<Value> {
        let params: ReadResourceParams = request.decode_params()?;
        // MCP Apps: `ui://{workspace}/{path}` resolves to a widget's HTML,
        // served with the Apps profile mime so the host renders it as a view.
        if let Some(rest) = params.uri.strip_prefix(&format!("{UI_RESOURCE_SCHEME}://")) {
            return self.read_ui_resource(identity, &params.uri, rest).await;
        }
        let provider = self
            .resources
            .as_ref()
            .ok_or_else(|| McpError::not_found("resources are not enabled"))?;
        let contents = provider.read(identity, &params.uri).await?;
        let mut result = serde_json::to_value(ReadResourceResult {
            result_type: RESULT_TYPE_COMPLETE.to_string(),
            contents: vec![contents],
            ttl_ms: READ_TTL_MS,
            cache_scope: CACHE_SCOPE_PRIVATE.to_string(),
        })?;
        self.attach_server_info(&mut result);
        Ok(result)
    }

    /// Legacy `resources/subscribe`: one URI per call, no filter, no stream id.
    pub(super) fn handle_resources_subscribe(
        &self,
        identity: &McpIdentity,
        request: &crate::protocol::JsonRpcRequest,
    ) -> Result<Value> {
        let provider = self
            .resources
            .as_ref()
            .ok_or_else(|| McpError::not_found("resources are not enabled"))?;
        let params: SubscribeResourceParams = request.decode_params()?;
        let _ = provider.subscribe(identity, &params.uri)?;
        Ok(json!({ "subscribed": true, "uri": params.uri }))
    }

    /// `subscriptions/listen` — the 2026-07-28 replacement for
    /// `resources/subscribe`.
    ///
    /// One long-lived stream now carries every notification type the client
    /// opts into, rather than one RPC per resource URI. Each type is opt-in and
    /// the server MUST NOT send one that was not requested, so the filter is
    /// validated here and echoed back — narrowed to what this server can
    /// actually produce — in `notifications/subscriptions/acknowledged`, which
    /// the transport must emit before any notification on the stream.
    pub(super) fn handle_subscriptions_listen(
        &self,
        identity: &McpIdentity,
        request: &crate::protocol::JsonRpcRequest,
    ) -> Result<Value> {
        let params: SubscriptionsListenParams = request.decode_params()?;
        let requested = params.notifications;

        let provider = self.resources.as_ref();
        let subscribe_supported = provider.is_some_and(|p| p.supports_subscribe());

        // Validate every requested resource subscription up front, so a bad URI
        // is reported now rather than as silence on the stream.
        let mut resource_subscriptions = Vec::new();
        if let (Some(provider), true) = (provider, subscribe_supported) {
            for uri in &requested.resource_subscriptions {
                let _ = provider.subscribe(identity, uri)?;
                resource_subscriptions.push(uri.clone());
            }
        }

        // Report only what we can honour. Echoing back a type we never emit
        // would promise a stream the client waits on forever, so each of these
        // is a statement about what this server actually produces:
        //   - tools:     yes, driven by `raisin:Function` node events.
        //   - prompts:   no prompt registry exists to change.
        //   - resources: the LIST is static; individual resource updates are
        //                delivered through `resource_subscriptions` instead.
        let granted = SubscriptionFilter {
            tools_list_changed: requested.tools_list_changed,
            prompts_list_changed: false,
            resources_list_changed: false,
            resource_subscriptions,
        };
        let mut result = json!({
            "resultType": RESULT_TYPE_COMPLETE,
            "acknowledged": granted,
        });
        self.attach_server_info(&mut result);
        Ok(result)
    }

    /// Open a live resource-update stream for `uri`, for the transport to drive.
    ///
    /// Unlike the `resources/subscribe` JSON-RPC method (which only confirms the
    /// subscription), this returns the actual stream of
    /// [`ResourceUpdatedNotification`](crate::protocol::ResourceUpdatedNotification)s
    /// so a transport can forward `notifications/resources/updated` frames.
    pub fn subscribe_resource(
        &self,
        identity: &McpIdentity,
        uri: &str,
    ) -> Result<
        impl futures::Stream<Item = crate::protocol::ResourceUpdatedNotification> + Send + 'static,
    > {
        self.authorize_session(identity)?;
        let provider = self
            .resources
            .as_ref()
            .ok_or_else(|| McpError::not_found("resources are not enabled"))?;
        provider.subscribe(identity, uri)
    }
}
