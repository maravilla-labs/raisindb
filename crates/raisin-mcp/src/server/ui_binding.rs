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

//! What an author WRITES to attach a widget to a tool.
//!
//! What the engine SERVES from it — uri form, `_meta.ui`, the read path — lives
//! in `dispatch::ui`, so a spec change lands in one of the two rather than both.

use serde::{Deserialize, Serialize};

use super::{UiCsp, UiPermissions};

/// How a widget was declared to be delivered, before SEP-1865 settled on one
/// answer.
///
/// **Deprecated: this no longer selects anything.** Both variants are served
/// identically, as inline HTML with mimeType `text/html;profile=mcp-app`. The
/// type survives only so a deployed node still parses — see
/// [`UiBinding::mode`] for why a parse failure here is not a local one.
///
/// It does retain one residual meaning: `UriList` implies the widget was
/// written against same-origin serving, so it defaults
/// [`UiBinding::base_href`] on. That is the migration, not a delivery choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiMode {
    /// Single self-contained document, returned inline. The only shape the
    /// spec defines.
    Html,
    /// Formerly a `text/uri-list` pointing at the static endpoint, which the
    /// host iframed with a real `src=`. SEP-1865 lists external URLs under
    /// "Content Types (deferred from MVP)", so no conformant host has to render
    /// one — which is exactly why such widgets appeared as a bare url.
    UriList,
}

/// A tool's optional MCP-UI binding: a delivery mode plus a workspace-relative
/// path (with an optional `#fragment`) to the widget's entry document, and the
/// MCP Apps (SEP-1865) resource metadata the widget advertises to hosts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiBinding {
    /// How the host should render the widget.
    ///
    /// **Deprecated and optional.** SEP-1865 defines exactly one delivery
    /// format — inline HTML with mimeType `text/html;profile=mcp-app` — and
    /// lists external URLs (`text/uri-list`) under "Content Types (deferred
    /// from MVP)". A widget is always delivered inline regardless of what this
    /// says; see [`UiMode`].
    ///
    /// Kept deserializable rather than removed: it was required with no serde
    /// default, so a deployed node carrying `mode: uri-list` would fail to
    /// parse, and a failed `UiBinding` parse cascades through `CustomTool` and
    /// `assemble_registry` to take down the WHOLE server at `initialize` — not
    /// just the one widget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<UiMode>,
    /// Name of a [`UiResource`] declared in the server's `ui_resources` list.
    ///
    /// The alternative to spelling the widget out inline. SEP-1865 makes the
    /// RESOURCE the unit that carries `csp` / `permissions` / `domain`, but this
    /// struct hangs off a TOOL — so several tools sharing one SPA each restated
    /// the whole block, and nothing reconciled them. Referencing a named
    /// resource states it once. Fields set here still win over the resource's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    /// Workspace-relative path to the entry document, optionally suffixed with a
    /// `#fragment`.
    ///
    /// The fragment is accepted and ignored: it names an in-app route, and a
    /// route is not a different resource. Which view a widget boots into comes
    /// from the host at runtime (the instantiating tool's name, then the
    /// `tool-input` notification), not from the URI.
    ///
    /// Defaulted rather than required so a binding may carry only `resource`;
    /// after [`McpServerDescriptor::resolve_ui`] it is always populated, and a
    /// binding still lacking one is dropped rather than served as a 404.
    #[serde(default)]
    pub entry: String,
    /// Workspace the entry path resolves in. Defaults to the session's active
    /// workspace (the server's first declared content workspace), which cannot
    /// always hold assets — e.g. a server whose primary workspace only allows
    /// document types keeps its widgets in a sibling asset workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Display name for the `ui://` resource in `resources/list`. Defaults to
    /// the tool name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Description of the UI resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// CSP domains the widget needs. When omitted, the engine declares this
    /// server's own origin for `connect`/`resource` so widget images and API
    /// calls served from the same RaisinDB instance work out of the box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub csp: Option<UiCsp>,
    /// Sandbox permissions the widget requests.
    ///
    /// Typed rather than a raw `Value` because the spec requires each granted
    /// member to be an EMPTY OBJECT — `{"camera": {}}`, not `{"camera": true}`.
    /// Passing the author's YAML through verbatim shipped whatever they wrote,
    /// and `camera: true` is the natural thing to write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<UiPermissions>,
    /// Stable sandbox origin the host should serve this widget from, e.g.
    /// `a904794854a047f6.claudemcpcontent.com`.
    ///
    /// Hosts otherwise pick a per-conversation origin. A stable one is what
    /// makes OAuth callbacks, CORS allowlists and API-key allowlists possible.
    ///
    /// The format is host-specific — Claude and ChatGPT use different domains —
    /// so this is passed through verbatim and left unset by default. A wrong
    /// value is worse than none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Whether the host should draw a visible border + background.
    #[serde(
        default,
        rename = "prefersBorder",
        skip_serializing_if = "Option::is_none"
    )]
    pub prefers_border: Option<bool>,
    /// Whether to inject a `<base href>` pointing at the entry document's
    /// directory, so the widget's RELATIVE urls keep resolving.
    ///
    /// A widget used to be reachable over HTTP at `/resources/...` and was
    /// framed same-origin, so `<img src="logo.png">` worked. Delivered inline
    /// per SEP-1865 it runs on the host's sandbox origin, where that same
    /// relative url resolves against the host and 404s. A `<base>` restores it.
    ///
    /// Defaults to ON for a binding that declared the legacy `mode: uri-list`
    /// (those are exactly the widgets written against same-origin serving) and
    /// OFF otherwise, so a spec-clean single-file widget gets a spec-clean
    /// document.
    ///
    /// The cost, and why this is not simply always on: with a `<base>`, a
    /// FRAGMENT-ONLY link (`<a href="#/inbox">`) resolves against the base url
    /// rather than the document, so a hash router built on anchors navigates
    /// away instead of switching view. Turn it on for relative assets; keep
    /// routing programmatic.
    #[serde(
        default,
        rename = "baseHref",
        alias = "base_href",
        skip_serializing_if = "Option::is_none"
    )]
    pub base_href: Option<bool>,
    /// Who may call this tool: `model` (the agent) and/or `app` (the widget).
    /// Defaults to both when omitted (the SEP-1865 default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Vec<String>>,
}

/// A widget declared once on the server and referenced by name from any number
/// of tools.
///
/// SEP-1865 scopes `csp`, `permissions` and `domain` to the RESOURCE — they
/// describe the document, not the call that opened it. RaisinDB hung them off
/// the tool binding, which made a shared SPA express N conflicting policies for
/// one document; the engine resolved that by taking whichever tool came first
/// in registry order, silently. Declaring the widget here gives it one identity
/// and one policy, and lets a server host several distinct widgets without
/// restating a block per tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiResource {
    /// Name tools reference via `ui: { resource: <id> }`. Server-scoped.
    pub id: String,
    /// The widget itself. Every [`UiBinding`] field except `resource` applies;
    /// `visibility` is tool-level and ignored here.
    #[serde(flatten)]
    pub ui: UiBinding,
}

impl UiBinding {
    /// Fill anything this binding left unset from a referenced [`UiResource`].
    ///
    /// One-directional and non-destructive: an explicit field on the binding
    /// always wins, so a tool can override a shared widget's `prefersBorder`
    /// or narrow its CSP without redeclaring the rest.
    pub fn inherit_from(&mut self, base: &UiBinding) {
        if self.entry.is_empty() {
            self.entry = base.entry.clone();
        }
        if self.mode.is_none() {
            self.mode = base.mode;
        }
        if self.base_href.is_none() {
            self.base_href = base.base_href;
        }
        if self.workspace.is_none() {
            self.workspace = base.workspace.clone();
        }
        if self.name.is_none() {
            self.name = base.name.clone();
        }
        if self.description.is_none() {
            self.description = base.description.clone();
        }
        if self.csp.is_none() {
            self.csp = base.csp.clone();
        }
        if self.permissions.is_none() {
            self.permissions = base.permissions.clone();
        }
        if self.domain.is_none() {
            self.domain = base.domain.clone();
        }
        if self.prefers_border.is_none() {
            self.prefers_border = base.prefers_border;
        }
    }

    /// Whether to inject a `<base href>` for this widget. See [`Self::base_href`].
    pub fn wants_base_href(&self) -> bool {
        self.base_href
            .unwrap_or(matches!(self.mode, Some(UiMode::UriList)))
    }

    /// The resource-level metadata SEP-1865 scopes to the DOCUMENT rather than
    /// to the call that opened it.
    ///
    /// Two bindings that resolve to the same `ui://` uri describe one resource
    /// and must agree on all of it; `warn_on_ui_resource_conflicts` compares
    /// these and reports a disagreement instead of silently serving whichever
    /// tool the registry happened to order first.
    pub(crate) fn resource_facet(&self) -> UiResourceFacet<'_> {
        UiResourceFacet {
            csp: self.csp.as_ref(),
            permissions: self.permissions.as_ref(),
            domain: self.domain.as_deref(),
            prefers_border: self.prefers_border,
            base_href: self.wants_base_href(),
        }
    }

    /// Split [`entry`](Self::entry) into its file path and optional fragment.
    ///
    /// The fragment is everything after the *first* `#` (passed through verbatim,
    /// including the `#`-less remainder); the path is everything before it. A
    /// fragment never affects which file is read — only which in-app route the
    /// widget boots into.
    pub fn split_entry(&self) -> (&str, Option<&str>) {
        split_entry(&self.entry)
    }
}

/// Split a widget `entry` string into `(path, fragment)` on the first `#`.
pub fn split_entry(entry: &str) -> (&str, Option<&str>) {
    match entry.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (entry, None),
    }
}

/// The document-scoped half of a [`UiBinding`], borrowed for comparison.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UiResourceFacet<'a> {
    csp: Option<&'a UiCsp>,
    permissions: Option<&'a UiPermissions>,
    domain: Option<&'a str>,
    prefers_border: Option<bool>,
    base_href: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ui_binding_parses_optional_workspace() {
        let with: UiBinding = serde_json::from_value(json!({
            "mode": "html",
            "entry": "/widgets/order/index.html",
            "workspace": "assets"
        }))
        .expect("ui with workspace");
        assert_eq!(with.workspace.as_deref(), Some("assets"));

        let without: UiBinding = serde_json::from_value(json!({
            "mode": "uri-list",
            "entry": "site/widgets/order/index.html#/card"
        }))
        .expect("ui without workspace");
        assert_eq!(without.workspace, None);
        // Omitted workspace round-trips as absent, not null.
        let round = serde_json::to_value(&without).expect("serialize");
        assert!(round.get("workspace").is_none());
    }
}
