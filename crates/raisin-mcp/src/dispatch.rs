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

//! The dispatch engine: maps JSON-RPC methods onto the assembled tool set.
//!
//! The [`Dispatcher`] is the transport-agnostic core. It owns a resolved
//! [`McpServerDescriptor`], its assembled [`ToolRegistry`], and an optional
//! [`NodeResourceProvider`]. Given an [`McpIdentity`] and a decoded
//! [`JsonRpcRequest`] it enforces server- and tool-level scopes, routes the six
//! supported MCP methods, and returns the spec-correct result payload. The
//! transport ([`crate::transport`]) wraps that into a [`JsonRpcResponse`].

use serde_json::{json, Value};

use crate::error::{McpError, Result};
use crate::identity::McpIdentity;
use crate::protocol::{
    CallToolParams, CallToolResult, ContentBlock, InitializeResult, ListResourcesResult,
    ListToolsResult, ReadResourceParams, ReadResourceResult, ResourcesCapability,
    ServerCapabilities, ServerInfo, SubscribeResourceParams, ToolsCapability, PROTOCOL_VERSION,
};
use crate::registry::ToolRegistry;
use crate::resources::{NodeResourceProvider, ResourceContents};
use crate::server::{split_entry, McpServerDescriptor, UiBinding, UiMode};
use crate::services::SharedAssetReader;

/// URI scheme used to identify an MCP-UI widget resource carried in a tool
/// result. Distinct from [`crate::resources::RESOURCE_SCHEME`] (`raisin://`,
/// which addresses node content) — a `ui://` URI names a rendered widget, and
/// including the `#fragment` keeps two tools that share one SPA file but bind to
/// different routes on distinct, correctly-cached resource URIs.
const UI_RESOURCE_SCHEME: &str = "ui";

/// Routes decoded MCP methods to the tool registry and resource provider.
pub struct Dispatcher {
    descriptor: McpServerDescriptor,
    registry: ToolRegistry,
    resources: Option<NodeResourceProvider>,
    assets: Option<SharedAssetReader>,
    public_base: Option<String>,
}

impl Dispatcher {
    /// Build a dispatcher over a resolved descriptor and its tool registry.
    pub fn new(descriptor: McpServerDescriptor, registry: ToolRegistry) -> Self {
        Self {
            descriptor,
            registry,
            resources: None,
            assets: None,
            public_base: None,
        }
    }

    /// Set the server's externally-reachable base URL (`https://host[:port]`).
    ///
    /// A `mode: uri-list` widget resource is a URL the HOST iframes — a
    /// relative `/resources/...` path is meaningless outside the server's own
    /// origin, so the transport derives a base (config/env or the request's
    /// Host header) and the dispatcher prefixes it. Without one the URL stays
    /// relative (previous behavior).
    pub fn with_public_base(mut self, base: Option<String>) -> Self {
        self.public_base = base.map(|b| b.trim_end_matches('/').to_string());
        self
    }

    /// Attach a resource provider, enabling `resources/*` methods.
    pub fn with_resources(mut self, resources: NodeResourceProvider) -> Self {
        self.resources = Some(resources);
        self
    }

    /// Attach an asset reader, enabling `mode: html` MCP-UI widget resources.
    ///
    /// Without one, a tool declaring `ui: { mode: html }` still returns its
    /// structured result — the widget resource is simply omitted (graceful
    /// degradation), since serving it requires reading the entry asset's bytes.
    pub fn with_asset_reader(mut self, assets: SharedAssetReader) -> Self {
        self.assets = Some(assets);
        self
    }

    /// The resolved server descriptor.
    pub fn descriptor(&self) -> &McpServerDescriptor {
        &self.descriptor
    }

    /// The assembled tool registry.
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// The resource provider, if resources are enabled.
    pub fn resources(&self) -> Option<&NodeResourceProvider> {
        self.resources.as_ref()
    }

    /// Capabilities advertised in the `initialize` response.
    pub fn capabilities(&self) -> ServerCapabilities {
        let resources = self.resources.as_ref().map(|provider| ResourcesCapability {
            subscribe: provider.supports_subscribe(),
            list_changed: false,
        });
        ServerCapabilities {
            tools: Some(ToolsCapability {
                list_changed: false,
            }),
            resources,
        }
    }

    /// Handle one decoded request for `identity`, returning the result payload
    /// (without the JSON-RPC envelope).
    ///
    /// Enforces the server-level scope gate first, then routes the method.
    pub async fn handle(
        &self,
        identity: &McpIdentity,
        request: &crate::protocol::JsonRpcRequest,
    ) -> Result<Value> {
        self.authorize_session(identity)?;

        match request.method.as_str() {
            "initialize" => self.handle_initialize(),
            "tools/list" => self.handle_tools_list(identity),
            "tools/call" => self.handle_tools_call(identity, request).await,
            "resources/list" => self.handle_resources_list(identity),
            "resources/read" => self.handle_resources_read(identity, request).await,
            "resources/subscribe" => self.handle_resources_subscribe(identity, request),
            other => Err(McpError::not_found(format!("unknown method: {other}"))),
        }
    }

    /// Reject sessions that may not open this server.
    ///
    /// A `public` server is open to anyone. Otherwise the caller must be
    /// authenticated — `public: false` means "not anonymous", even when the
    /// server declares no scopes — and must hold every scope the server requires.
    fn authorize_session(&self, identity: &McpIdentity) -> Result<()> {
        if self.descriptor.public {
            return Ok(());
        }
        if identity.is_anonymous() {
            return Err(McpError::unauthorized(
                "authentication required for this MCP server".to_string(),
            ));
        }
        let missing = identity.missing_scopes(&self.descriptor.scopes);
        if missing.is_empty() {
            Ok(())
        } else {
            Err(McpError::unauthorized(format!(
                "session is missing required scopes: {}",
                missing.join(", ")
            )))
        }
    }

    fn handle_initialize(&self) -> Result<Value> {
        let result = InitializeResult {
            protocol_version: PROTOCOL_VERSION.to_string(),
            capabilities: self.capabilities(),
            server_info: ServerInfo {
                name: self.descriptor.name.clone(),
                version: self.descriptor.version.clone(),
            },
            instructions: self.descriptor.instructions.clone(),
        };
        Ok(serde_json::to_value(result)?)
    }

    fn handle_tools_list(&self, identity: &McpIdentity) -> Result<Value> {
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
        Ok(json!({ "tools": entries }))
    }

    /// Canonical `ui://` URI for a binding under this session (fragment
    /// stripped — it names an in-app route, never a different resource).
    fn ui_uri_for(&self, identity: &McpIdentity, ui: &UiBinding) -> String {
        let workspace = ui.workspace.as_deref().unwrap_or(&identity.workspace);
        let (path, _fragment) = ui.split_entry();
        ui_resource_uri(workspace, path)
    }

    /// The SEP-1865 `_meta.ui` object for a widget RESOURCE (csp, permissions,
    /// prefersBorder).
    ///
    /// The server's own origin is ALWAYS declared for connect/resource, whether
    /// or not the binding declares a CSP of its own. A widget that cannot reach
    /// the instance it was served from is broken by construction — it loads its
    /// images and makes its API calls there — and the origin is not knowable at
    /// authoring time, since the same package is installed on every deployment.
    /// This used to be an either/or: declaring any `csp:` replaced the default
    /// and silently dropped the server origin, so a binding that listed only
    /// dev origins worked locally and could reach nothing once deployed.
    fn ui_resource_meta(&self, ui: &UiBinding) -> Value {
        let mut meta = serde_json::Map::new();
        let declared = match &ui.csp {
            Some(csp) if !csp.is_empty() => serde_json::to_value(csp).unwrap_or(Value::Null),
            _ => Value::Null,
        };
        let csp_value = csp_with_own_origin(declared, self.public_base.as_deref());
        if let Some(csp) = csp_value {
            meta.insert("csp".into(), csp);
        }
        if let Some(permissions) = &ui.permissions {
            meta.insert("permissions".into(), permissions.clone());
        }
        if let Some(prefers_border) = ui.prefers_border {
            meta.insert("prefersBorder".into(), json!(prefers_border));
        }
        Value::Object(meta)
    }

    async fn handle_tools_call(
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

    fn handle_resources_list(&self, identity: &McpIdentity) -> Result<Value> {
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
                "mimeType": ui_mime_type(ui.mode),
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
        Ok(json!({ "resources": resources }))
    }

    async fn handle_resources_read(
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
        Ok(serde_json::to_value(ReadResourceResult {
            contents: vec![contents],
        })?)
    }

    /// Serve a `ui://` widget resource (MCP Apps SEP-1865).
    ///
    /// `rest` is `{workspace}/{entry-path}` (fragment tolerated and ignored —
    /// it names an in-app route, never a different file). The asset read is
    /// RLS-scoped to the caller like every other asset read. When the URI
    /// matches a declared tool binding, that binding's resource metadata
    /// (csp/permissions/prefersBorder) rides on the content item — the
    /// spec-preferred location, which takes precedence over listing metadata.
    async fn read_ui_resource(
        &self,
        identity: &McpIdentity,
        uri: &str,
        rest: &str,
    ) -> Result<Value> {
        let (rest, _fragment) = split_entry(rest);
        let (workspace, path) = rest
            .split_once('/')
            .ok_or_else(|| McpError::not_found(format!("malformed ui resource uri: {uri}")))?;

        // Resolve the declaring binding FIRST: it carries the mode, which
        // decides what this resource even is. Serving every binding inline
        // regardless of mode is what made `mode: uri-list` a silent no-op —
        // a multi-file widget was handed to the host as `srcdoc`, where its
        // relative `./assets/*.js` URLs resolve against a null origin and load
        // nothing.
        let canonical = {
            let (bare, _fragment) = split_entry(uri);
            bare.to_string()
        };
        let binding = self
            .registry
            .visible_descriptors(identity)
            .into_iter()
            .filter_map(|t| t.ui)
            .find(|ui| self.ui_uri_for(identity, ui) == canonical);
        let mode = binding.as_ref().map_or(UiMode::Html, |ui| ui.mode);

        let mut content = match mode {
            UiMode::Html => {
                let Some(assets) = self.assets.as_ref() else {
                    return Err(McpError::not_found("ui resources are not enabled"));
                };
                let asset = assets
                    .read_asset(identity, workspace, &format!("/{path}"))
                    .await?;
                let html = String::from_utf8_lossy(&asset.bytes).into_owned();
                let html = inject_server_origin(html, self.public_base.as_deref());
                json!({
                    "uri": uri,
                    "mimeType": ui_mime_type(UiMode::Html),
                    "text": html,
                })
            }
            // The host iframes this URL with a real `src=`, so the widget is
            // same-origin with the server and its relative asset URLs resolve.
            // An absolute URL is required: the host is not on our origin, so a
            // relative path would resolve against the host's.
            UiMode::UriList => {
                let base = self.public_base.as_deref().ok_or_else(|| {
                    McpError::not_found(
                        "a uri-list widget needs the server's public base URL; set RAISINDB_BASE_URL"
                            .to_string(),
                    )
                })?;
                let url = format!(
                    "{base}/resources/{}/{}/{workspace}/{path}",
                    identity.repo, identity.branch
                );
                json!({
                    "uri": uri,
                    "mimeType": ui_mime_type(UiMode::UriList),
                    "text": url,
                })
            }
        };

        // Binding metadata (csp/permissions/prefersBorder) rides on the content
        // item — the spec-preferred location, which takes precedence over the
        // listing's copy.
        if let Some(ui) = binding {
            let meta = self.ui_resource_meta(&ui);
            if meta.as_object().is_some_and(|m| !m.is_empty()) {
                content["_meta"] = json!({ "ui": meta });
            }
        }
        Ok(json!({ "contents": [content] }))
    }

    fn handle_resources_subscribe(
        &self,
        identity: &McpIdentity,
        request: &crate::protocol::JsonRpcRequest,
    ) -> Result<Value> {
        let provider = self
            .resources
            .as_ref()
            .ok_or_else(|| McpError::not_found("resources are not enabled"))?;
        let params: SubscribeResourceParams = request.decode_params()?;

        // Validate the subscription up front; the transport owns the resulting
        // stream (it forwards `notifications/resources/updated` frames). The
        // engine confirms the subscription synchronously here.
        let _ = provider.subscribe(identity, &params.uri)?;
        Ok(json!({ "subscribed": true, "uri": params.uri }))
    }
}

impl Dispatcher {
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

/// Convenience: wrap a raw result value as a single-JSON-block tool result.
pub fn tool_content(value: Value) -> Vec<ContentBlock> {
    vec![ContentBlock::json(value)]
}

/// Build the `ui://` identifier URI for a widget resource (fragment preserved).
fn ui_resource_uri(workspace: &str, entry: &str) -> String {
    let trimmed = entry.strip_prefix('/').unwrap_or(entry);
    format!("{UI_RESOURCE_SCHEME}://{workspace}/{trimmed}")
}

#[cfg(test)]
mod ui_tests {
    use super::*;

    #[test]
    fn ui_resource_uri_strips_leading_slash() {
        assert_eq!(
            ui_resource_uri("assets", "/widgets/x/index.html"),
            "ui://assets/widgets/x/index.html"
        );
    }

    #[test]
    fn each_mode_declares_its_own_mime_type() {
        // Both modes must be served AND listed as what they are. `uri-list`
        // used to be parsed and then ignored: every binding was served inline
        // as html, so a multi-file widget reached the host as `srcdoc` and its
        // relative asset URLs resolved against a null origin.
        assert_eq!(ui_mime_type(UiMode::Html), "text/html;profile=mcp-app");
        assert_eq!(ui_mime_type(UiMode::UriList), "text/uri-list");
        assert_ne!(ui_mime_type(UiMode::Html), ui_mime_type(UiMode::UriList));
    }

    #[test]
    fn server_origin_is_defined_before_the_first_script() {
        let html = "<!doctype html><html><head><meta charset=\"utf-8\" />\
                    <script type=\"module\">boot()</script></head><body></body></html>";
        let out = inject_server_origin(
            html.to_string(),
            Some("https://solutas.rdb.maravilla.cloud"),
        );

        let global = out.find("__RAISIN_SERVER_ORIGIN__").unwrap();
        let boot = out.find("boot()").unwrap();
        assert!(global < boot, "widget code must not run before the global");
        assert!(out
            .contains(r#"window.__RAISIN_SERVER_ORIGIN__="https://solutas.rdb.maravilla.cloud";"#));
    }

    #[test]
    fn server_origin_precedes_a_head_less_fragment() {
        // Bundlers emit documents with no <head>; appending would define the
        // global after the code that reads it.
        let out = inject_server_origin(
            "<script>boot()</script>".to_string(),
            Some("https://x.test"),
        );
        assert!(out.find("__RAISIN_SERVER_ORIGIN__").unwrap() < out.find("boot()").unwrap());
    }

    #[test]
    fn header_tag_is_not_mistaken_for_head() {
        let out = inject_server_origin("<header>hi</header>".to_string(), Some("https://x.test"));
        assert!(out.starts_with("<script>window.__RAISIN_SERVER_ORIGIN__"));
    }

    #[test]
    fn origin_is_escaped_and_absent_base_changes_nothing() {
        let out = inject_server_origin(
            "<head></head>".to_string(),
            Some("https://x.test/\"</script>"),
        );
        // The injected <script> must contain exactly one `</script>` — its own
        // terminator. JSON quoting alone does NOT achieve this: the HTML parser
        // finds `</script` before JS is tokenized, so an origin carrying one
        // would close the element early and spill the rest as markup.
        assert_eq!(out.matches("</script>").count(), 1);
        assert!(out.contains("\\u003C/script\\u003E"), "must be \\u-escaped");

        let html = "<head></head><script>boot()</script>";
        assert_eq!(inject_server_origin(html.to_string(), None), html);
    }

    #[test]
    fn own_origin_is_added_alongside_declared_domains() {
        // The regression: a binding that declares any csp used to REPLACE the
        // default, dropping the server's own origin. A package authored against
        // localhost then shipped to a deployment whose widget could reach
        // nothing — every image and API call blocked by the host's sandbox.
        let declared = json!({
            "connectDomains": ["http://localhost:5173"],
            "resourceDomains": ["http://localhost:8080"],
            "frameDomains": ["http://localhost:5173"],
        });
        let merged =
            csp_with_own_origin(declared, Some("https://solutas.rdb.maravilla.cloud")).unwrap();

        assert_eq!(
            merged["connectDomains"],
            json!([
                "http://localhost:5173",
                "https://solutas.rdb.maravilla.cloud"
            ])
        );
        assert_eq!(
            merged["resourceDomains"],
            json!([
                "http://localhost:8080",
                "https://solutas.rdb.maravilla.cloud"
            ])
        );
        // frameDomains is NOT widened: framing the server's own origin is not
        // implied by serving the widget, and granting it would be a privilege
        // the author never asked for.
        assert_eq!(merged["frameDomains"], json!(["http://localhost:5173"]));
    }

    #[test]
    fn own_origin_is_not_duplicated() {
        let declared = json!({ "connectDomains": ["https://example.test"] });
        let merged = csp_with_own_origin(declared, Some("https://example.test")).unwrap();
        assert_eq!(merged["connectDomains"], json!(["https://example.test"]));
    }

    #[test]
    fn undeclared_csp_still_gets_the_server_origin() {
        let merged = csp_with_own_origin(Value::Null, Some("https://example.test")).unwrap();
        assert_eq!(merged["connectDomains"], json!(["https://example.test"]));
        assert_eq!(merged["resourceDomains"], json!(["https://example.test"]));
    }

    #[test]
    fn no_base_and_no_declaration_says_nothing() {
        assert!(csp_with_own_origin(Value::Null, None).is_none());
        // ...but a declaration alone is still passed through verbatim.
        let declared = json!({ "connectDomains": ["https://example.test"] });
        assert_eq!(
            csp_with_own_origin(declared.clone(), None).unwrap(),
            declared
        );
    }
}

/// The resource mime type a widget of `mode` is served as.
///
/// `html` is the MCP Apps profile the host renders via `srcdoc`; `uri-list` is
/// a URL the host iframes with a real `src=`. Both must be declared
/// consistently in `resources/list` and `resources/read` — a host that
/// prefetched one type and received the other has to discard it.
///
/// Note `text/uri-list` corresponds to the spec's `externalUrl` content type,
/// which SEP-1865 explicitly DEFERS from the MVP ("Content Types (deferred from
/// MVP): `externalUrl`"). Hosts are therefore not obliged to render it; prefer
/// `mode: html` unless the widget genuinely needs multi-file serving.
fn ui_mime_type(mode: UiMode) -> &'static str {
    match mode {
        UiMode::Html => "text/html;profile=mcp-app",
        UiMode::UriList => "text/uri-list",
    }
}

/// Inline `window.__RAISIN_SERVER_ORIGIN__` into a widget document.
///
/// A widget is authored once and installed on every deployment, so it cannot
/// know at build time which instance will serve it. Hardcoding one is the trap:
/// the Studio widget shipped with `http://localhost:8080` baked in, worked for
/// its author, and on every real deployment probed an origin that was not the
/// server — reporting itself unreachable while the MCP session underneath was
/// perfectly healthy. The server DOES know its origin (it derives one for the
/// CSP already), so it states it here rather than leaving each widget to guess.
///
/// The global is defined before any other script in the document, so a widget
/// may read it synchronously at module scope. It is `const`-free and idempotent
/// on re-read since the document is re-served per read.
///
/// Returns `html` untouched when no public base is known — a widget that has
/// its own fallback keeps working exactly as before.
fn inject_server_origin(html: String, base: Option<&str>) -> String {
    let Some(base) = base else {
        return html;
    };
    // JSON-encode for the JS string, THEN escape the HTML-significant
    // characters as `\uXXXX`. JSON alone is not enough: the HTML parser looks
    // for the literal `</script` before JavaScript is ever tokenized, so a
    // `</script>` inside a correctly-quoted JS string still closes the element
    // and everything after it becomes markup. `base` can come from the request's
    // Host header when the deployment trusts forwarded headers, so treat it as
    // untrusted. `<` is the same string value to JS, and inert to HTML.
    let literal = serde_json::to_string(base)
        .unwrap_or_else(|_| "\"\"".to_string())
        .replace('<', "\\u003C")
        .replace('>', "\\u003E")
        .replace('&', "\\u0026");
    let snippet = format!("<script>window.__RAISIN_SERVER_ORIGIN__={literal};</script>");

    // Insert immediately after <head...>, else before the first <script>, else
    // prepend. The middle case matters for widgets built without a <head>
    // (bundlers emit bare fragments), where appending would land the global
    // AFTER the code that reads it.
    if let Some(pos) = find_tag_end(&html, "<head") {
        let mut out = String::with_capacity(html.len() + snippet.len());
        out.push_str(&html[..pos]);
        out.push_str(&snippet);
        out.push_str(&html[pos..]);
        return out;
    }
    match html.to_ascii_lowercase().find("<script") {
        Some(pos) => {
            let mut out = String::with_capacity(html.len() + snippet.len());
            out.push_str(&html[..pos]);
            out.push_str(&snippet);
            out.push_str(&html[pos..]);
            out
        }
        None => format!("{snippet}{html}"),
    }
}

/// Byte offset just past the closing `>` of the first `tag` (e.g. `<head`),
/// tolerating attributes. `None` when the tag is absent or unterminated.
fn find_tag_end(html: &str, tag: &str) -> Option<usize> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find(tag)?;
    // Reject `<header>` and friends: the char after the tag name must end the
    // name, not continue it.
    let after_name = html.as_bytes().get(start + tag.len())?;
    if after_name.is_ascii_alphanumeric() || *after_name == b'-' {
        return None;
    }
    let close = html[start..].find('>')? + start;
    Some(close + 1)
}

/// Merge the server's own origin into a widget CSP's connect/resource lists,
/// preserving whatever the binding declared and never duplicating.
///
/// `declared` is the serialized [`UiCsp`], or `Value::Null` when the binding
/// declared none. Returns `None` only when there is nothing to say at all — no
/// declared CSP and no known public base.
fn csp_with_own_origin(declared: Value, base: Option<&str>) -> Option<Value> {
    let Some(base) = base else {
        return match declared {
            Value::Null => None,
            other => Some(other),
        };
    };

    let mut csp = match declared {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    for key in ["connectDomains", "resourceDomains"] {
        let list = csp
            .entry(key.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        let Some(domains) = list.as_array_mut() else {
            continue;
        };
        if !domains.iter().any(|d| d.as_str() == Some(base)) {
            domains.push(json!(base));
        }
    }
    Some(Value::Object(csp))
}
