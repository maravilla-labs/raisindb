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

//! Deciding what tool discovery should write.
//!
//! Deliberately a PURE function of (what exists, what the server offers). No
//! storage, no I/O — so the two properties the whole design leans on can be
//! tested directly:
//!
//! 1. **Paths are stable.** An agent references a proxy by path. If a refresh
//!    could rename one, every agent holding it breaks silently
//!    (`resolveToolsParallel` returns `null` for a missing node and the tool
//!    just vanishes). So the slug is derived deterministically from the remote
//!    name, and collision suffixes are assigned in sorted remote-name order —
//!    never in the order the server happened to list them.
//! 2. **A steady state writes nothing.** Discovery may run hourly forever. If
//!    an unchanged tool produced a write, each connection would mint thousands
//!    of function revisions a year for no reason.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::connection::ToolFilter;
use super::session::RemoteToolDescriptor;

/// Max length of a generated tool slug, before any collision suffix.
const MAX_TOOL_SLUG: usize = 48;

/// Slug used when a remote name sanitizes away to nothing.
const FALLBACK_SLUG: &str = "tool";

/// What discovery intends to do about one tool.
#[derive(Debug, Clone, PartialEq)]
pub enum ReconcileAction {
    /// No proxy exists yet.
    Create(ProxyPlan),
    /// A proxy exists but its schema, filter state or missing-state changed.
    Update(ProxyPlan),
    /// Nothing changed — write NOTHING. This is the churn guard.
    Unchanged(ProxyPlan),
    /// The tool is gone upstream. Disable the proxy; never delete it.
    MarkMissing {
        /// Remote name that disappeared.
        remote_name: String,
        /// Path of the proxy to disable.
        function_path: String,
    },
}

impl ReconcileAction {
    /// Whether acting on this requires a write.
    pub fn is_write(&self) -> bool {
        !matches!(self, Self::Unchanged(_))
    }

    /// The remote name this action concerns.
    pub fn remote_name(&self) -> &str {
        match self {
            Self::Create(plan) | Self::Update(plan) | Self::Unchanged(plan) => &plan.remote_name,
            Self::MarkMissing { remote_name, .. } => remote_name,
        }
    }
}

/// Everything needed to write one proxy `raisin:Function` node.
#[derive(Debug, Clone, PartialEq)]
pub struct ProxyPlan {
    /// VERBATIM remote name. The only thing ever sent in `tools/call`.
    pub remote_name: String,
    /// Sanitized last path segment.
    pub tool_slug: String,
    /// `{connection_slug}__{tool_slug}` — unique across connections.
    pub function_name: String,
    /// `/mcp/{connection_slug}/{tool_slug}` in the functions workspace.
    pub function_path: String,
    /// Hash over the tool's identity + schemas; drives the churn guard.
    pub schema_hash: String,
    /// Whether the tool filter exposes this tool.
    pub enabled: bool,
    /// Display title.
    pub title: String,
    /// Remote description, verbatim.
    pub description: Option<String>,
    /// Remote `inputSchema`, verbatim.
    pub input_schema: Value,
    /// Remote `outputSchema`, verbatim.
    pub output_schema: Option<Value>,
}

/// A proxy that already exists, as read off the connection's record.
#[derive(Debug, Clone, PartialEq)]
pub struct ExistingProxy {
    /// Remote name it proxies.
    pub remote_name: String,
    /// Its node path.
    pub function_path: String,
    /// Hash recorded at its last write.
    pub schema_hash: String,
    /// Whether it is currently enabled.
    pub enabled: bool,
    /// `active` | `missing` | `conflict`.
    pub state: String,
}

/// Decide what to write for one connection.
///
/// `remote` is what `tools/list` returned; `existing` is what the connection
/// recorded last time.
pub fn reconcile_plan(
    connection_slug: &str,
    existing: &[ExistingProxy],
    remote: &[RemoteToolDescriptor],
    filter: &ToolFilter,
) -> Vec<ReconcileAction> {
    let plans = plan_proxies(connection_slug, remote, filter);
    let mut actions = Vec::with_capacity(plans.len() + existing.len());

    for plan in plans {
        let previous = existing.iter().find(|e| e.remote_name == plan.remote_name);
        actions.push(match previous {
            None => ReconcileAction::Create(plan),
            Some(previous)
                // Every field a write would change must be compared, not just
                // the hash: a tool re-enabled by a filter edit, or one that
                // reappeared after going missing, has an identical schema.
                if previous.schema_hash == plan.schema_hash
                    && previous.enabled == plan.enabled
                    && previous.state == "active" =>
            {
                ReconcileAction::Unchanged(plan)
            }
            Some(_) => ReconcileAction::Update(plan),
        });
    }

    // Anything previously known and not in this listing is gone upstream.
    for previous in existing {
        let still_there = remote.iter().any(|t| t.name == previous.remote_name);
        if !still_there && previous.state != "missing" {
            actions.push(ReconcileAction::MarkMissing {
                remote_name: previous.remote_name.clone(),
                function_path: previous.function_path.clone(),
            });
        }
    }

    actions
}

/// Build one plan per remote tool, with stable slugs.
pub fn plan_proxies(
    connection_slug: &str,
    remote: &[RemoteToolDescriptor],
    filter: &ToolFilter,
) -> Vec<ProxyPlan> {
    // Assign slugs in sorted remote-name order so a collision suffix does not
    // depend on the order the server listed its tools. A server that reorders
    // its listing between calls would otherwise swap `foo-2` and `foo-3`
    // between two agents' tools, silently.
    let mut ordered: Vec<&RemoteToolDescriptor> = remote.iter().collect();
    ordered.sort_by(|a, b| a.name.cmp(&b.name));

    let mut taken: Vec<String> = Vec::with_capacity(ordered.len());
    let mut slug_for: Vec<(String, String)> = Vec::with_capacity(ordered.len());
    for tool in &ordered {
        let slug = unique_slug(&slugify(&tool.name), &taken);
        taken.push(slug.clone());
        slug_for.push((tool.name.clone(), slug));
    }

    // Emit in the server's original order; only slug ASSIGNMENT is sorted.
    remote
        .iter()
        .map(|tool| {
            let tool_slug = slug_for
                .iter()
                .find(|(name, _)| name == &tool.name)
                .map(|(_, slug)| slug.clone())
                .unwrap_or_else(|| FALLBACK_SLUG.to_string());

            ProxyPlan {
                function_name: format!("{connection_slug}__{tool_slug}"),
                function_path: format!("/mcp/{connection_slug}/{tool_slug}"),
                schema_hash: schema_hash(tool),
                enabled: filter.permits(&tool.name),
                title: tool.title.clone().unwrap_or_else(|| tool.name.clone()),
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
                output_schema: tool.output_schema.clone(),
                remote_name: tool.name.clone(),
                tool_slug,
            }
        })
        .collect()
}

/// Hash a tool's identity and schemas.
///
/// Over CANONICAL json, not `Value::to_string()`. This workspace builds
/// `serde_json` with `preserve_order`, so a `Map` keeps insertion order rather
/// than sorting — two byte-identical schemas that merely arrived with their
/// keys in a different order would hash differently, and the churn guard would
/// rewrite every proxy on every refresh forever.
pub fn schema_hash(tool: &RemoteToolDescriptor) -> String {
    let identity = json!({
        "name": tool.name,
        "description": tool.description,
        "inputSchema": tool.input_schema,
        "outputSchema": tool.output_schema,
    });
    let mut hasher = Sha256::new();
    hasher.update(canonical_json(&identity).as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

/// Serialize a value with object keys sorted, recursively.
fn canonical_json(value: &Value) -> String {
    fn normalize(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let mut out = Map::new();
                for key in keys {
                    out.insert(key.clone(), normalize(&map[key]));
                }
                Value::Object(out)
            }
            // Array order is semantically meaningful in JSON Schema
            // (`required`, `enum`), so it is preserved, not sorted.
            Value::Array(items) => Value::Array(items.iter().map(normalize).collect()),
            other => other.clone(),
        }
    }
    normalize(value).to_string()
}

/// Sanitize a remote tool name into a path-safe slug.
///
/// MCP names are `^[a-zA-Z0-9_-]+$` per spec, but servers ship dots, slashes
/// and colons in practice. A `/` here would forge a path in the functions
/// workspace, so this is a safety boundary, not cosmetics. The original name is
/// never reconstructed from the slug — it is stored verbatim alongside.
pub fn slugify(remote_name: &str) -> String {
    let mut slug = String::with_capacity(remote_name.len());
    let mut last_was_dash = false;
    for ch in remote_name.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' {
            if last_was_dash || slug.is_empty() {
                continue;
            }
            last_was_dash = true;
        } else {
            last_was_dash = false;
        }
        slug.push(mapped);
        if slug.len() >= MAX_TOOL_SLUG {
            break;
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        FALLBACK_SLUG.to_string()
    } else {
        slug
    }
}

/// Suffix a slug until it does not collide with one already assigned.
fn unique_slug(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == base) {
        return base.to_string();
    }
    for n in 2..=9_999 {
        let candidate = format!("{base}-{n}");
        if !taken.iter().any(|t| t == &candidate) {
            return candidate;
        }
    }
    // Unreachable in practice; a server with 10k colliding names gets one
    // tool rather than an infinite loop.
    base.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> RemoteToolDescriptor {
        RemoteToolDescriptor {
            name: name.to_string(),
            title: None,
            description: Some(format!("does {name}")),
            input_schema: json!({ "type": "object" }),
            output_schema: None,
            annotations: None,
        }
    }

    fn existing(plan: &ProxyPlan) -> ExistingProxy {
        ExistingProxy {
            remote_name: plan.remote_name.clone(),
            function_path: plan.function_path.clone(),
            schema_hash: plan.schema_hash.clone(),
            enabled: plan.enabled,
            state: "active".to_string(),
        }
    }

    #[test]
    fn slugify_sanitizes_path_forging_characters() {
        assert_eq!(slugify("search_issues"), "search-issues");
        assert_eq!(slugify("a/b/../c"), "a-b-c");
        assert_eq!(slugify("Weird::Name.v2"), "weird-name-v2");
        assert_eq!(slugify("---"), "tool");
        assert_eq!(slugify(""), "tool");
        assert!(!slugify("x".repeat(200).as_str()).is_empty());
        assert!(slugify("x".repeat(200).as_str()).len() <= MAX_TOOL_SLUG);
    }

    #[test]
    fn slug_assignment_is_stable_under_remote_reordering() {
        // Two names that sanitize to the same slug. Whichever order the server
        // lists them, each must keep the SAME slug — an agent holds the path.
        let forward = vec![tool("a.b"), tool("a/b")];
        let reversed = vec![tool("a/b"), tool("a.b")];
        let filter = ToolFilter::default();

        let mut first = plan_proxies("c", &forward, &filter);
        let mut second = plan_proxies("c", &reversed, &filter);
        first.sort_by(|x, y| x.remote_name.cmp(&y.remote_name));
        second.sort_by(|x, y| x.remote_name.cmp(&y.remote_name));

        assert_eq!(
            first.iter().map(|p| &p.function_path).collect::<Vec<_>>(),
            second.iter().map(|p| &p.function_path).collect::<Vec<_>>(),
            "a reordered listing must not renumber collision suffixes"
        );
        // And they must actually be distinct.
        assert_ne!(first[0].function_path, first[1].function_path);
    }

    #[test]
    fn function_names_are_namespaced_by_connection() {
        let filter = ToolFilter::default();
        let a = plan_proxies("linear", &[tool("search")], &filter);
        let b = plan_proxies("github", &[tool("search")], &filter);

        // raisin:Function.name is unique AND enforced, so two connections each
        // exposing `search` would otherwise hard-fail the second discovery.
        assert_eq!(a[0].function_name, "linear__search");
        assert_eq!(b[0].function_name, "github__search");
        assert_ne!(a[0].function_path, b[0].function_path);
    }

    #[test]
    fn unchanged_tools_produce_no_write() {
        let filter = ToolFilter::default();
        let remote = vec![tool("alpha"), tool("beta")];
        let plans = plan_proxies("c", &remote, &filter);
        let existing: Vec<ExistingProxy> = plans.iter().map(existing).collect();

        let actions = reconcile_plan("c", &existing, &remote, &filter);

        assert_eq!(actions.len(), 2);
        assert!(
            actions.iter().all(|a| !a.is_write()),
            "a steady-state refresh must write nothing: {actions:?}"
        );
    }

    #[test]
    fn key_order_alone_does_not_trigger_a_rewrite() {
        // `preserve_order` means these two Values are NOT equal byte-wise,
        // but they are the same schema. Without canonicalization this rewrites
        // every proxy on every refresh, forever.
        let mut a = tool("x");
        a.input_schema = json!({ "type": "object", "properties": { "q": { "type": "string" } } });
        let mut b = tool("x");
        b.input_schema = json!({ "properties": { "q": { "type": "string" } }, "type": "object" });

        assert_eq!(schema_hash(&a), schema_hash(&b));
    }

    #[test]
    fn a_changed_schema_triggers_an_update() {
        let filter = ToolFilter::default();
        let before = vec![tool("alpha")];
        let existing: Vec<ExistingProxy> = plan_proxies("c", &before, &filter)
            .iter()
            .map(existing)
            .collect();

        let mut after = tool("alpha");
        after.input_schema = json!({ "type": "object", "required": ["q"] });

        let actions = reconcile_plan("c", &existing, &[after], &filter);
        assert!(matches!(actions[0], ReconcileAction::Update(_)));
    }

    #[test]
    fn a_disappeared_tool_is_marked_missing_not_deleted() {
        let filter = ToolFilter::default();
        let before = vec![tool("alpha"), tool("beta")];
        let existing: Vec<ExistingProxy> = plan_proxies("c", &before, &filter)
            .iter()
            .map(existing)
            .collect();

        let actions = reconcile_plan("c", &existing, &[tool("alpha")], &filter);

        let missing: Vec<&ReconcileAction> = actions
            .iter()
            .filter(|a| matches!(a, ReconcileAction::MarkMissing { .. }))
            .collect();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].remote_name(), "beta");
        // Deleting would make the tool vanish from any agent referencing it
        // with no error anywhere; disabling stays visible in the console.
        assert!(matches!(missing[0], ReconcileAction::MarkMissing { .. }));
    }

    #[test]
    fn a_tool_already_marked_missing_is_not_re_marked() {
        let filter = ToolFilter::default();
        let plans = plan_proxies("c", &[tool("alpha")], &filter);
        let mut stale = existing(&plans[0]);
        stale.state = "missing".to_string();

        let actions = reconcile_plan("c", &[stale], &[], &filter);
        assert!(actions.is_empty(), "a second refresh must not rewrite it");
    }

    #[test]
    fn a_reappearing_tool_is_updated_back_to_active() {
        let filter = ToolFilter::default();
        let remote = vec![tool("alpha")];
        let plans = plan_proxies("c", &remote, &filter);
        let mut gone = existing(&plans[0]);
        gone.state = "missing".to_string();
        gone.enabled = false;

        let actions = reconcile_plan("c", &[gone], &remote, &filter);
        assert!(
            matches!(actions[0], ReconcileAction::Update(_)),
            "an identical schema still needs a write to clear `missing`"
        );
    }

    #[test]
    fn the_tool_filter_decides_enablement_and_a_filter_edit_forces_a_write() {
        let remote = vec![tool("alpha"), tool("beta")];
        let open = ToolFilter::default();
        let existing: Vec<ExistingProxy> = plan_proxies("c", &remote, &open)
            .iter()
            .map(existing)
            .collect();

        let narrowed = ToolFilter {
            allow: vec!["alpha".into()],
            deny: vec![],
        };
        let actions = reconcile_plan("c", &existing, &remote, &narrowed);

        // `beta` is now filtered out: it must be rewritten as disabled, not
        // left enabled because its schema happens to be unchanged.
        let beta = actions
            .iter()
            .find(|a| a.remote_name() == "beta")
            .expect("beta must appear");
        assert!(matches!(beta, ReconcileAction::Update(plan) if !plan.enabled));
    }

    #[test]
    fn remote_name_is_carried_verbatim_never_the_slug() {
        let filter = ToolFilter::default();
        let plans = plan_proxies("c", &[tool("Weird::Name.v2")], &filter);

        assert_eq!(plans[0].tool_slug, "weird-name-v2");
        // tools/call must send the original, or the remote 404s the tool.
        assert_eq!(plans[0].remote_name, "Weird::Name.v2");
    }
}
