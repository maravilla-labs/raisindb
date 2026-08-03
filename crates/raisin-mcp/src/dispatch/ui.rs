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

//! The MCP Apps (SEP-1865) widget contract.
//!
//! Everything that decides what a `ui://` resource IS lives here: the URI form,
//! the resource `_meta.ui` block, the read path, and the HTML/CSP handling that
//! feeds them. The rest of the dispatcher only routes.
//!
//! Kept separate because this is the surface that tracks an external spec — it
//! changes on someone else's schedule, and reviewing a widget change should not
//! mean reading the JSON-RPC router.

use serde_json::{json, Value};

use super::{Dispatcher, READ_TTL_MS, UI_RESOURCE_SCHEME};
use crate::error::{McpError, Result};
use crate::identity::McpIdentity;
use crate::protocol::{CACHE_SCOPE_PRIVATE, RESULT_TYPE_COMPLETE, UI_MIME_TYPE};
use crate::server::{split_entry, UiBinding};

impl Dispatcher {
    /// Canonical `ui://` URI for a binding under this session (fragment
    /// stripped — it names an in-app route, never a different resource).
    pub(super) fn ui_uri_for(&self, identity: &McpIdentity, ui: &UiBinding) -> String {
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
    pub(super) fn ui_resource_meta(&self, ui: &UiBinding) -> Value {
        let mut meta = serde_json::Map::new();
        let declared = match &ui.csp {
            Some(csp) if !csp.is_empty() => {
                sanitize_csp_lists(serde_json::to_value(csp).unwrap_or(Value::Null))
            }
            _ => Value::Null,
        };
        // `public_base` may be derived from the request's `x-forwarded-host`,
        // so it gets the same treatment as author-declared origins.
        let own_origin = self.public_base.as_deref().and_then(sanitize_origin);
        let csp_value = csp_with_own_origin(declared, own_origin.as_deref());
        if let Some(csp) = csp_value {
            meta.insert("csp".into(), csp);
        }
        if let Some(permissions) = ui.permissions.as_ref().filter(|p| !p.is_empty()) {
            // Serializes each granted member as `{}`, per SEP-1865.
            if let Ok(value) = serde_json::to_value(permissions) {
                meta.insert("permissions".into(), value);
            }
        }
        if let Some(domain) = ui.domain.as_deref().and_then(sanitize_domain) {
            meta.insert("domain".into(), json!(domain));
        }
        if let Some(prefers_border) = ui.prefers_border {
            meta.insert("prefersBorder".into(), json!(prefers_border));
        }
        Value::Object(meta)
    }

    /// Serve a `ui://` widget resource (MCP Apps SEP-1865).
    ///
    /// `rest` is `{workspace}/{entry-path}` (fragment tolerated and ignored —
    /// it names an in-app route, never a different file). The asset read is
    /// RLS-scoped to the caller like every other asset read. When the URI
    /// matches a declared tool binding, that binding's resource metadata
    /// (csp/permissions/prefersBorder) rides on the content item — the
    /// spec-preferred location, which takes precedence over listing metadata.
    pub(super) async fn read_ui_resource(
        &self,
        identity: &McpIdentity,
        uri: &str,
        rest: &str,
    ) -> Result<Value> {
        let (rest, _fragment) = split_entry(rest);
        let (workspace, path) = rest
            .split_once('/')
            .ok_or_else(|| McpError::not_found(format!("malformed ui resource uri: {uri}")))?;

        // Resolve the declaring binding FIRST — and REQUIRE one.
        //
        // The binding carries the mode, which decides what this resource even
        // is. It is also the ONLY authorization this path has: a `ui://` read
        // is reachable by any caller who can open the server, and it does not
        // pass through the per-tool `scopes` gate that `handle_tools_call`
        // applies. Defaulting a miss to `UiMode::Html` and reading the asset
        // anyway turned `ui://{any-workspace}/{any-path}` into an arbitrary
        // asset reader for every authenticated caller, bounded only by RLS —
        // the last line of defence, not the intended one. A widget belonging
        // to a tool gated behind `scopes: ["admin"]` was readable by anyone.
        //
        // `visible_descriptors` already filters by the caller's scopes, so
        // requiring a match here inherits that gate exactly.
        let canonical = {
            let (bare, _fragment) = split_entry(uri);
            bare.to_string()
        };
        let binding = self
            .registry
            .visible_descriptors(identity)
            .into_iter()
            .filter_map(|t| t.ui)
            .find(|ui| self.ui_uri_for(identity, ui) == canonical)
            .ok_or_else(|| {
                // Deliberately indistinguishable from a genuinely absent
                // resource: telling an unauthorized caller that a widget
                // exists is itself a disclosure.
                tracing::debug!(
                    uri = %uri,
                    "ui resource DENIED: no visible tool binding declares this URI"
                );
                McpError::not_found(format!("unknown ui resource: {uri}"))
            })?;
        // ONE delivery format. SEP-1865 defines inline HTML and nothing else —
        // external URLs sit under "Content Types (deferred from MVP)" — so a
        // binding that still says `mode: uri-list` is served inline like any
        // other. That mode is why ChatGPT printed a url instead of rendering a
        // widget: `text/uri-list` is not a content type it, or any conformant
        // host, is obliged to render.
        //
        // What `uri-list` DID provide was same-origin serving, which is what
        // made a widget's relative urls resolve. `wants_base_href` carries that
        // property across (defaulting on for exactly those bindings) without
        // the non-conformant content type.
        let Some(assets) = self.assets.as_ref() else {
            return Err(McpError::not_found("ui resources are not enabled"));
        };
        let asset = assets
            .read_asset(identity, workspace, &format!("/{path}"))
            .await?;
        let html = String::from_utf8_lossy(&asset.bytes).into_owned();
        let base_href = binding.wants_base_href().then(|| {
            self.public_base.as_deref().map(|base| {
                widget_base_href(base, &identity.repo, &identity.branch, workspace, path)
            })
        });
        let html = inject_widget_preamble(
            html,
            self.public_base.as_deref(),
            base_href.flatten().as_deref(),
        );
        let mut content = json!({
            "uri": uri,
            "mimeType": UI_MIME_TYPE,
            "text": html,
        });

        // Binding metadata (csp/permissions/prefersBorder) rides on the content
        // item — the spec-preferred location, which takes precedence over the
        // listing's copy.
        let meta = self.ui_resource_meta(&binding);
        if meta.as_object().is_some_and(|m| !m.is_empty()) {
            content["_meta"] = json!({ "ui": meta });
        }
        let mut result = json!({
            "resultType": RESULT_TYPE_COMPLETE,
            "contents": [content],
            "ttlMs": READ_TTL_MS,
            "cacheScope": CACHE_SCOPE_PRIVATE,
        });
        self.attach_server_info(&mut result);
        Ok(result)
    }
}

/// Absolute url of the DIRECTORY holding a widget's entry document, for
/// `<base href>`. Trailing slash is required: without it the last segment is
/// treated as a file name and dropped when relative urls resolve.
pub(super) fn widget_base_href(
    base: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    path: &str,
) -> String {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    match path.rsplit_once('/') {
        Some((dir, _file)) => format!("{base}/resources/{repo}/{branch}/{workspace}/{dir}/"),
        None => format!("{base}/resources/{repo}/{branch}/{workspace}/"),
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
pub(super) fn inject_server_origin(html: String, base: Option<&str>) -> String {
    inject_widget_preamble(html, base, None)
}

/// Insert the engine's `<head>` preamble: an optional `<base href>` followed by
/// the server-origin global.
///
/// Both go in one insertion, in this order, because both must precede the
/// document's own scripts and `<base>` must precede anything that resolves a
/// relative url. Inserting them separately would put the origin script first
/// and leave a `<base>` that arrives too late for markup above it.
///
/// An author-supplied `<base>` is respected: HTML resolves against the FIRST
/// one with an `href`, so injecting ours ahead of theirs would silently
/// override it. We skip instead.
pub(super) fn inject_widget_preamble(
    html: String,
    origin: Option<&str>,
    base_href: Option<&str>,
) -> String {
    let mut preamble = String::new();
    if let Some(href) = base_href {
        if find_tag_end(&html, "<base").is_none() {
            preamble.push_str(&format!("<base href=\"{}\">", escape_attribute(href)));
        }
    }
    if let Some(origin) = origin {
        preamble.push_str(&server_origin_script(origin));
    }
    if preamble.is_empty() {
        return html;
    }
    insert_into_head(html, &preamble)
}

/// Escape a value for an HTML double-quoted attribute. The href embeds a
/// workspace and asset path from node properties, so it is author-controlled.
fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The `<script>` defining `window.__RAISIN_SERVER_ORIGIN__`.
fn server_origin_script(base: &str) -> String {
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
    format!("<script>window.__RAISIN_SERVER_ORIGIN__={literal};</script>")
}

/// Insert `snippet` immediately after `<head...>`, else before the first
/// `<script>`, else at the very front.
///
/// The middle case matters for widgets built without a `<head>` (bundlers emit
/// bare fragments), where appending would land the preamble AFTER the code that
/// reads it.
fn insert_into_head(html: String, snippet: &str) -> String {
    let at = find_tag_end(&html, "<head").or_else(|| html.to_ascii_lowercase().find("<script"));
    let Some(at) = at else {
        return format!("{snippet}{html}");
    };
    let mut out = String::with_capacity(html.len() + snippet.len());
    out.push_str(&html[..at]);
    out.push_str(snippet);
    out.push_str(&html[at..]);
    out
}

/// Byte offset just past the closing `>` of the first `tag` (e.g. `<head`),
/// tolerating attributes. `None` when the tag is absent or unterminated.
pub(super) fn find_tag_end(html: &str, tag: &str) -> Option<usize> {
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

/// Longest origin we will emit. Nothing legitimate is close to this; the cap
/// exists so a pathological node property cannot bloat every `resources/read`.
const MAX_ORIGIN_LEN: usize = 253 + 16;

/// Accept an origin only if it is safe to interpolate into a CSP header.
///
/// Hosts build a real `Content-Security-Policy` header by joining these values
/// with spaces and semicolons, so a value containing `;`, a quote, or a newline
/// can terminate one directive and inject another. The values reach us from
/// node properties (author-controlled) and, for `public_base`, from the
/// `x-forwarded-host` request header (attacker-influenced) — neither is
/// trustworthy enough to pass through verbatim.
///
/// Deliberately strict and deliberately silent: a rejected origin is dropped,
/// never surfaced as an error. A malformed CSP entry must not turn a working
/// widget into a failed `resources/read`.
pub(super) fn sanitize_origin(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_ORIGIN_LEN {
        return None;
    }
    // Covers the CSP delimiters plus anything that could break out of a header
    // line. Control characters include CR/LF, so header splitting is covered.
    if value
        .chars()
        .any(|c| c.is_control() || matches!(c, ';' | ',' | '\'' | '"' | ' ' | '\\'))
    {
        return None;
    }
    // Require a real scheme://host. A bare host would be interpreted by hosts
    // as a relative source and match more than the author intended. Wildcard
    // subdomains (`https://*.example.com`) are legal and preserved.
    let (scheme, rest) = value.split_once("://")?;
    if !matches!(scheme, "http" | "https") || rest.is_empty() {
        return None;
    }
    // Reject anything with a path, query or fragment: CSP source expressions
    // are origins, and a stray `/` changes matching semantics.
    if rest.contains('/') || rest.contains('?') || rest.contains('#') {
        return None;
    }
    Some(value.to_string())
}

/// Accept a sandbox `domain` only if it is a bare hostname.
///
/// Unlike a CSP source this is NOT an origin — hosts published examples like
/// `a904794854a047f6.claudemcpcontent.com`, with no scheme. It still lands in
/// host-side configuration, so it gets the same "no delimiters, no control
/// characters" treatment, and a scheme or path is rejected rather than silently
/// reinterpreted.
pub(super) fn sanitize_domain(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_ORIGIN_LEN {
        return None;
    }
    if value.contains("://") || value.contains('/') {
        return None;
    }
    let ok = value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
    (ok && value.contains('.')).then(|| value.to_string())
}

/// Drop every unsafe entry from each domain list of a serialized [`UiCsp`].
pub(super) fn sanitize_csp_lists(mut declared: Value) -> Value {
    let Some(map) = declared.as_object_mut() else {
        return declared;
    };
    for list in map.values_mut() {
        if let Some(items) = list.as_array_mut() {
            items.retain(|item| item.as_str().is_some_and(|s| sanitize_origin(s).is_some()));
        }
    }
    declared
}

/// Merge the server's own origin into a widget CSP's connect/resource lists,
/// preserving whatever the binding declared and never duplicating.
///
/// `declared` is the serialized [`UiCsp`], or `Value::Null` when the binding
/// declared none. Returns `None` only when there is nothing to say at all — no
/// declared CSP and no known public base.
pub(super) fn csp_with_own_origin(declared: Value, base: Option<&str>) -> Option<Value> {
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

/// Build the `ui://` identifier URI for a widget resource (fragment preserved).
pub(super) fn ui_resource_uri(workspace: &str, entry: &str) -> String {
    let trimmed = entry.strip_prefix('/').unwrap_or(entry);
    format!("{UI_RESOURCE_SCHEME}://{workspace}/{trimmed}")
}
