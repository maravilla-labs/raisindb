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

//! Mount bundles: a connector-authored preset of `raisin:VirtualMount`s, and
//! the rule that turns one into the mounts to create.
//!
//! # Why this is here and in the admin console
//!
//! The console's "Add bundle" dialog was the only thing that instantiated a
//! bundle, so a control plane setting up a tenant had to retype the dialog's
//! logic or click through it. This module is a port of the console's
//! `planBundle` (`packages/admin-console/src/api/integrations.ts`), which was
//! kept pure precisely so a server endpoint could adopt it unchanged. The two
//! must agree: the console's tests (`mountBundles.test.ts`) are mirrored below
//! case for case.
//!
//! Pure: no I/O. The HTTP handler reads the connector, the workspaces and the
//! existing mounts, and hands this module plain values.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Where an instance keeps its config, mounts and templates (`raisin:system`).
pub const INTEGRATIONS_ROOT: &str = "/integrations";
/// Package-owned connector templates. Never a mount's `integration_ref`.
pub const CONNECTORS_ROOT: &str = "/connectors";
/// Parent folder of every `raisin:VirtualMount` node.
pub const MOUNTS_ROOT: &str = "/mounts";
/// The node type the engine materialises the folders along a mount path as.
pub const FOLDER_TYPE: &str = "raisin:Folder";

/// A connector-authored preset of mounts. Schema: `raisin:Integration.mount_bundles`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MountBundle {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_root: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<BundlePrompt>,
    #[serde(default)]
    pub mounts: Vec<BundleEntry>,
}

/// One mount inside a bundle: everything a `raisin:VirtualMount` needs except
/// what the operator chooses at instantiation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BundleEntry {
    /// Mount name suffix; the mount is named `<connector>-<key>`.
    pub key: String,
    #[serde(default)]
    pub title: String,
    /// Appended to the chosen root: `mount_path = <root>/<subpath>`.
    #[serde(default)]
    pub subpath: String,
    /// Pre-selected when the caller names no entries.
    #[serde(default)]
    pub default: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_by: Vec<String>,
    /// Node types the mount materialises: checked against the workspace gate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub node_types: Vec<String>,
    /// Land this entry in a workspace of its own, overriding the bundle's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_workspace: Option<String>,
    /// Hang this entry under a different root than the operator's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapping_function: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolver_function: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_config: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_config: Option<Map<String, Value>>,
}

/// A question asked once and written onto every entry that needs the answer.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BundlePrompt {
    pub key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// `remote` | `select` | `text`. Informational on the server.
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browse: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// Entry KEYS this answer is written onto.
    #[serde(default)]
    pub applies_to: Vec<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_when: Option<BTreeMap<String, String>>,
    /// `sync_config.<key>` | `remote_root` | `account_ref` — a CLOSED set.
    pub target: String,
}

/// Parse a connector's `mount_bundles` property (as JSON). A malformed bundle
/// is skipped rather than failing the others, and reported by id/index.
pub fn parse_bundles(value: &Value) -> (Vec<MountBundle>, Vec<String>) {
    let mut bundles = Vec::new();
    let mut errors = Vec::new();
    let Some(items) = value.as_array() else {
        return (bundles, errors);
    };
    for (i, item) in items.iter().enumerate() {
        match serde_json::from_value::<MountBundle>(item.clone()) {
            Ok(b) => bundles.push(b),
            Err(e) => {
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("#{i}"));
                errors.push(format!("mount bundle {id}: {e}"));
            }
        }
    }
    (bundles, errors)
}

/// `/a//b/` → `/a/b`; `""` → `/`. Same normalisation as the mount editor.
pub fn normalize_mount_path(p: &str) -> String {
    let segs: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
    format!("/{}", segs.join("/"))
}

/// A blank answer means "unanswered": the console writes `""` for a cleared field.
fn answered<'a>(answers: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    answers.get(key).map(|v| v.trim()).filter(|v| !v.is_empty())
}

/// Is this prompt live, given the answers so far?
pub fn prompt_is_active(prompt: &BundlePrompt, answers: &BTreeMap<String, String>) -> bool {
    match &prompt.required_when {
        None => true,
        Some(when) => when
            .iter()
            .all(|(k, v)| answered(answers, k) == Some(v.as_str())),
    }
}

/// Prompts that apply: active, and wanted by at least one selected entry.
pub fn active_prompts<'a>(
    bundle: &'a MountBundle,
    keys: &[String],
    answers: &BTreeMap<String, String>,
) -> Vec<&'a BundlePrompt> {
    bundle
        .prompts
        .iter()
        .filter(|p| prompt_is_active(p, answers) && p.applies_to.iter().any(|k| keys.contains(k)))
        .collect()
}

/// Keys of active, required prompts still unanswered. Non-empty blocks create.
pub fn missing_prompt_keys(
    bundle: &MountBundle,
    keys: &[String],
    answers: &BTreeMap<String, String>,
) -> Vec<String> {
    active_prompts(bundle, keys, answers)
        .into_iter()
        .filter(|p| p.required && answered(answers, &p.key).is_none())
        .map(|p| p.key.clone())
        .collect()
}

/// The entries pre-selected when a caller names none.
pub fn default_keys(bundle: &MountBundle) -> Vec<String> {
    bundle
        .mounts
        .iter()
        .filter(|e| e.default)
        .map(|e| e.key.clone())
        .collect()
}

/// Keys the caller named that the bundle does not have.
///
/// The console silently ignores these (its picker cannot produce one). An API
/// caller can, and a typo there must not quietly create fewer mounts.
pub fn unknown_keys(bundle: &MountBundle, keys: &[String]) -> Vec<String> {
    let known: BTreeSet<&str> = bundle.mounts.iter().map(|e| e.key.as_str()).collect();
    keys.iter()
        .filter(|k| !known.contains(k.as_str()))
        .cloned()
        .collect()
}

/// The operator's choices for one instantiation.
#[derive(Debug, Clone)]
pub struct BundlePlanInput<'a> {
    /// The instance's node name (`maravilla-stripe`).
    pub integration_name: &'a str,
    /// The instance's title, used in each mount's title.
    pub integration_title: &'a str,
    /// The instance's path, which every mount's `integration_ref` names.
    pub integration_path: &'a str,
    pub bundle: &'a MountBundle,
    /// Which entries to create, by `key`, in any order (bundle order wins).
    pub keys: &'a [String],
    pub account_ref: Option<&'a str>,
    pub target_workspace: &'a str,
    pub target_branch: Option<&'a str>,
    /// Root folder every entry's `subpath` hangs under, e.g. `/stripe`.
    pub root: &'a str,
    /// Answers to `bundle.prompts`, by prompt key. Blank answers are ignored.
    pub answers: &'a BTreeMap<String, String>,
}

/// One mount to create.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct PlannedMount {
    /// The bundle entry this came from.
    pub key: String,
    pub name: String,
    pub title: String,
    pub integration_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_ref: Option<String>,
    pub target_workspace: String,
    pub target_branch: String,
    pub mount_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapping_function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolver_function: Option<String>,
    pub sync_config: Map<String, Value>,
    pub write_config: Map<String, Value>,
    pub enabled: bool,
    /// Not a mount property: what the workspace gate has to admit.
    #[serde(skip)]
    pub node_types: Vec<String>,
}

impl PlannedMount {
    /// The `raisin:VirtualMount` property map, in the console's
    /// `mountToProperties` shape. `state` is engine-owned and never written.
    pub fn properties(&self) -> Map<String, Value> {
        let mut p = Map::new();
        p.insert("title".into(), Value::String(self.title.clone()));
        p.insert(
            "integration_ref".into(),
            Value::String(self.integration_ref.clone()),
        );
        if let Some(a) = &self.account_ref {
            p.insert("account_ref".into(), Value::String(a.clone()));
        }
        p.insert(
            "target_workspace".into(),
            Value::String(self.target_workspace.clone()),
        );
        p.insert(
            "target_branch".into(),
            Value::String(self.target_branch.clone()),
        );
        p.insert("mount_path".into(), Value::String(self.mount_path.clone()));
        for (k, v) in [
            ("remote_root", &self.remote_root),
            ("mapping_function", &self.mapping_function),
            ("resolver_function", &self.resolver_function),
        ] {
            if let Some(v) = v {
                p.insert(k.into(), Value::String(v.clone()));
            }
        }
        p.insert(
            "sync_config".into(),
            Value::Object(self.sync_config.clone()),
        );
        p.insert(
            "write_config".into(),
            Value::Object(self.write_config.clone()),
        );
        p.insert("enabled".into(), Value::Bool(self.enabled));
        p
    }
}

/// Why a bundle could not be planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundlePlanError {
    /// A prompt names a target outside the closed set. Refused rather than
    /// dropped: a dropped answer creates a mount that looks configured and
    /// reads the wrong mailbox.
    UnsupportedPromptTarget { prompt: String, target: String },
}

impl std::fmt::Display for BundlePlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedPromptTarget { prompt, target } => write!(
                f,
                "mount bundle prompt `{prompt}` has an unsupported target '{target}' \
                 (expected sync_config.<key>, remote_root or account_ref)"
            ),
        }
    }
}

impl std::error::Error for BundlePlanError {}

/// Write one answer onto a planned mount, at the prompt's declared target.
fn apply_prompt_answer(
    mount: &mut PlannedMount,
    prompt: &BundlePrompt,
    value: &str,
) -> Result<(), BundlePlanError> {
    match prompt.target.as_str() {
        "remote_root" => mount.remote_root = Some(value.to_string()),
        "account_ref" => mount.account_ref = Some(value.to_string()),
        t => match t.strip_prefix("sync_config.") {
            Some(key) if !key.is_empty() && !key.contains('.') => {
                mount
                    .sync_config
                    .insert(key.to_string(), Value::String(value.to_string()));
            }
            _ => {
                return Err(BundlePlanError::UnsupportedPromptTarget {
                    prompt: prompt.key.clone(),
                    target: prompt.target.clone(),
                })
            }
        },
    }
    Ok(())
}

/// Turn a bundle plus the operator's choices into the mounts to create.
///
/// Port of the console's `planBundle`; keep the two in step.
pub fn plan_bundle(input: &BundlePlanInput<'_>) -> Result<Vec<PlannedMount>, BundlePlanError> {
    let root = normalize_mount_path(input.root);
    let bundle = input.bundle;
    // Only ACTIVE prompts are applied: an answer left behind by a prompt the
    // operator switched away from must not ride along on the mount.
    let prompts: Vec<&BundlePrompt> = bundle
        .prompts
        .iter()
        .filter(|p| prompt_is_active(p, input.answers))
        .collect();
    let mut out = Vec::new();
    for e in bundle.mounts.iter().filter(|e| input.keys.contains(&e.key)) {
        let entry_root = match &e.root_override {
            Some(r) => normalize_mount_path(r),
            None => root.clone(),
        };
        let mut mount = PlannedMount {
            key: e.key.clone(),
            name: format!("{}-{}", input.integration_name, e.key),
            title: format!("{} · {}", input.integration_title, e.title),
            integration_ref: if input.integration_path.is_empty() {
                format!("{INTEGRATIONS_ROOT}/{}", input.integration_name)
            } else {
                input.integration_path.to_string()
            },
            account_ref: input
                .account_ref
                .filter(|a| !a.is_empty())
                .map(str::to_string),
            target_workspace: e
                .target_workspace
                .clone()
                .filter(|w| !w.is_empty())
                .unwrap_or_else(|| input.target_workspace.to_string()),
            target_branch: input
                .target_branch
                .filter(|b| !b.is_empty())
                .unwrap_or("main")
                .to_string(),
            mount_path: normalize_mount_path(&format!("{entry_root}/{}", e.subpath)),
            remote_root: e.remote_root.clone().filter(|s| !s.is_empty()),
            mapping_function: e.mapping_function.clone().filter(|s| !s.is_empty()),
            resolver_function: e.resolver_function.clone().filter(|s| !s.is_empty()),
            sync_config: e.sync_config.clone().unwrap_or_default(),
            write_config: e.write_config.clone().unwrap_or_default(),
            enabled: true,
            node_types: e.node_types.clone(),
        };
        for p in &prompts {
            if !p.applies_to.contains(&e.key) {
                continue;
            }
            if let Some(value) = answered(input.answers, &p.key) {
                apply_prompt_answer(&mut mount, p, value)?;
            }
        }
        out.push(mount);
    }
    Ok(out)
}

/// Does a workspace type list admit `node_type`? Empty or `*` admits all,
/// the same rule the engine applies at write time.
pub fn type_list_admits(allowed: &[String], node_type: &str) -> bool {
    allowed.is_empty() || allowed.iter().any(|t| t == "*" || t == node_type)
}

/// Node types a workspace must admit for these mounts: the folders along each
/// path plus every entry's own types. Sorted, deduplicated.
pub fn needed_types<'a>(mounts: impl IntoIterator<Item = &'a PlannedMount>) -> Vec<String> {
    let mut need: BTreeSet<String> = BTreeSet::new();
    need.insert(FOLDER_TYPE.to_string());
    for m in mounts {
        for t in &m.node_types {
            need.insert(t.clone());
        }
    }
    need.into_iter().collect()
}

/// The first segment of a mount path (`/stripe/sessions` → `/stripe`): the
/// folder the engine creates at the workspace ROOT, gated by
/// `allowed_root_node_types` rather than `allowed_node_types`.
pub fn top_level_folder(mount_path: &str) -> Option<String> {
    normalize_mount_path(mount_path)
        .split('/')
        .find(|s| !s.is_empty())
        .map(|s| format!("/{s}"))
}

/// Whether a planned mount collides with an existing one: same name, or the
/// same workspace and path. Either makes a create fail or two mounts fight.
pub fn collides(
    planned: &PlannedMount,
    existing_name: &str,
    existing_ws: &str,
    existing_path: &str,
) -> bool {
    planned.name == existing_name
        || (planned.target_workspace == existing_ws
            && planned.mount_path == normalize_mount_path(existing_path))
}

/// Properties of a template that must NOT be carried into a new instance:
/// engine-owned state and anything secret. The console's
/// `createIntegrationFromTemplate` resets the same set.
pub const INSTANCE_RESET_PROPERTIES: &[&str] = &[
    "connected_accounts",
    "capabilities",
    "capabilities_checked_at",
    "client_secret_encrypted",
    "config_secrets_encrypted",
    "config_secret_fields",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bundle(v: Value) -> MountBundle {
        serde_json::from_value(v).expect("bundle")
    }

    fn stripe() -> MountBundle {
        bundle(json!({
            "id": "stripe-finance",
            "title": "Stripe",
            "default_workspace": "finance",
            "default_root": "/stripe",
            "mounts": [
                {
                    "key": "sessions", "title": "Checkout sessions", "subpath": "sessions",
                    "default": true, "node_types": ["stripe:CheckoutSession"],
                    "mapping_function": "/mappers/stripe-default",
                    "sync_config": { "resource": "checkout_sessions", "mode": "hybrid", "interval_seconds": 300 },
                    "write_config": { "writeback": "off", "mode": "submit", "command_node_types": ["stripe:CheckoutSession"] }
                },
                {
                    "key": "refunds", "title": "Refunds", "subpath": "refunds", "default": true,
                    "node_types": ["stripe:Refund"], "mapping_function": "/mappers/stripe-default",
                    "sync_config": { "resource": "refunds", "mode": "hybrid" },
                    "write_config": { "writeback": "off", "mode": "submit", "command_node_types": ["stripe:Refund"] }
                },
                {
                    "key": "products", "title": "Products", "subpath": "products",
                    "node_types": ["stripe:Product"], "mapping_function": "/mappers/stripe-default",
                    "sync_config": { "resource": "products", "mode": "hybrid" },
                    "write_config": { "writeback": "off", "mode": "state_only" }
                }
            ]
        }))
    }

    fn ms365() -> MountBundle {
        bundle(json!({
            "id": "ms365",
            "title": "Microsoft 365",
            "prompts": [
                { "key": "mailbox", "title": "Mailbox", "type": "text",
                  "applies_to": ["inbox", "outbox"], "target": "sync_config.mailbox" },
                { "key": "drive_scope", "title": "Drive", "type": "select", "options": ["me", "site"],
                  "applies_to": ["onedrive"], "target": "sync_config.drive_scope" },
                { "key": "site_id", "title": "Site", "type": "remote", "browse": "site",
                  "applies_to": ["onedrive"], "required": true,
                  "required_when": { "drive_scope": "site" }, "target": "sync_config.site_id" }
            ],
            "mounts": [
                { "key": "inbox", "title": "Inbox", "subpath": "mail/inbox", "default": true,
                  "sync_config": { "resource": "mail" } },
                { "key": "outbox", "title": "Outbox", "subpath": "mail/outbox",
                  "sync_config": { "resource": "mail" } },
                { "key": "onedrive", "title": "OneDrive", "subpath": "onedrive",
                  "target_workspace": "assets", "root_override": "/drives",
                  "sync_config": { "resource": "files" } }
            ]
        }))
    }

    fn keys(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    fn answers(a: &[(&str, &str)]) -> BTreeMap<String, String> {
        a.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn plan(
        b: &MountBundle,
        k: &[&str],
        root: &str,
        a: &BTreeMap<String, String>,
    ) -> Vec<PlannedMount> {
        let k = keys(k);
        plan_bundle(&BundlePlanInput {
            integration_name: "maravilla-stripe",
            integration_title: "Stripe",
            integration_path: "/integrations/maravilla-stripe",
            bundle: b,
            keys: &k,
            account_ref: Some("acct-1"),
            target_workspace: "finance",
            target_branch: None,
            root,
            answers: a,
        })
        .expect("plan")
    }

    #[test]
    fn normalize_collapses_slashes_and_never_returns_empty() {
        assert_eq!(normalize_mount_path(""), "/");
        assert_eq!(normalize_mount_path("/"), "/");
        assert_eq!(
            normalize_mount_path("stripe//sessions/"),
            "/stripe/sessions"
        );
    }

    #[test]
    fn mints_one_mount_per_selected_key_under_the_chosen_root() {
        let b = stripe();
        let m = plan(&b, &["sessions", "refunds"], "/stripe/", &BTreeMap::new());
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].name, "maravilla-stripe-sessions");
        assert_eq!(m[0].title, "Stripe · Checkout sessions");
        assert_eq!(m[0].mount_path, "/stripe/sessions");
        assert_eq!(m[0].integration_ref, "/integrations/maravilla-stripe");
        assert_eq!(m[0].account_ref.as_deref(), Some("acct-1"));
        assert_eq!(m[0].target_workspace, "finance");
        assert_eq!(m[0].target_branch, "main");
        assert!(m[0].enabled);
        assert_eq!(
            m[0].write_config.get("command_node_types"),
            Some(&json!(["stripe:CheckoutSession"]))
        );
        assert_eq!(m[1].mount_path, "/stripe/refunds");
    }

    #[test]
    fn ignores_keys_the_bundle_does_not_know_and_keeps_bundle_order() {
        let b = stripe();
        let m = plan(
            &b,
            &["products", "nope", "sessions"],
            "/stripe",
            &BTreeMap::new(),
        );
        let names: Vec<&str> = m.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(names, vec!["sessions", "products"]);
        assert_eq!(
            unknown_keys(&b, &keys(&["products", "nope"])),
            keys(&["nope"])
        );
    }

    #[test]
    fn defaults_are_the_entries_marked_default() {
        assert_eq!(default_keys(&stripe()), keys(&["sessions", "refunds"]));
    }

    #[test]
    fn properties_match_the_console_shape_and_never_carry_state() {
        let b = stripe();
        let m = plan(&b, &["sessions"], "/stripe", &BTreeMap::new());
        let p = Value::Object(m[0].properties());
        assert_eq!(p["mount_path"], json!("/stripe/sessions"));
        assert_eq!(p["mapping_function"], json!("/mappers/stripe-default"));
        assert_eq!(p["sync_config"]["resource"], json!("checkout_sessions"));
        assert_eq!(p["sync_config"]["interval_seconds"], json!(300));
        assert_eq!(p["enabled"], json!(true));
        assert!(p.get("state").is_none());
        assert!(p.get("remote_root").is_none());
        assert!(
            p.get("key").is_none(),
            "planner bookkeeping is not a property"
        );
    }

    #[test]
    fn lands_an_entry_in_its_own_workspace_and_root() {
        let b = ms365();
        let m = plan(
            &b,
            &["inbox", "onedrive"],
            "/microsoft-365",
            &BTreeMap::new(),
        );
        assert_eq!(m[0].target_workspace, "finance");
        assert_eq!(m[0].mount_path, "/microsoft-365/mail/inbox");
        assert_eq!(m[1].target_workspace, "assets");
        assert_eq!(m[1].mount_path, "/drives/onedrive");
    }

    #[test]
    fn writes_an_answer_only_onto_the_entries_that_ask_for_it() {
        let b = ms365();
        let a = answers(&[("mailbox", "sales@example.test")]);
        let m = plan(&b, &["inbox", "outbox", "onedrive"], "/m", &a);
        assert_eq!(m[0].sync_config["mailbox"], json!("sales@example.test"));
        assert_eq!(m[1].sync_config["mailbox"], json!("sales@example.test"));
        assert!(m[2].sync_config.get("mailbox").is_none());
    }

    #[test]
    fn ignores_a_blank_answer_rather_than_writing_an_empty_string() {
        let b = ms365();
        let m = plan(&b, &["inbox"], "/m", &answers(&[("mailbox", "  ")]));
        assert!(m[0].sync_config.get("mailbox").is_none());
    }

    #[test]
    fn drops_an_answer_whose_prompt_is_no_longer_active() {
        let b = ms365();
        let a = answers(&[("drive_scope", "me"), ("site_id", "site-1")]);
        let m = plan(&b, &["onedrive"], "/m", &a);
        assert_eq!(m[0].sync_config["drive_scope"], json!("me"));
        assert!(m[0].sync_config.get("site_id").is_none());
    }

    #[test]
    fn applies_a_conditional_answer_once_its_condition_holds() {
        let b = ms365();
        let a = answers(&[("drive_scope", "site"), ("site_id", "site-1")]);
        let m = plan(&b, &["onedrive"], "/m", &a);
        assert_eq!(m[0].sync_config["site_id"], json!("site-1"));
    }

    #[test]
    fn refuses_a_target_outside_the_closed_set() {
        for target in [
            "enabled",
            "write_config.mode",
            "sync_config.",
            "sync_config.a.b",
        ] {
            let b = bundle(json!({
                "id": "x",
                "prompts": [{ "key": "p", "applies_to": ["a"], "target": target }],
                "mounts": [{ "key": "a", "subpath": "a" }]
            }));
            let k = keys(&["a"]);
            let a = answers(&[("p", "v")]);
            let err = plan_bundle(&BundlePlanInput {
                integration_name: "c",
                integration_title: "C",
                integration_path: "",
                bundle: &b,
                keys: &k,
                account_ref: None,
                target_workspace: "w",
                target_branch: None,
                root: "/",
                answers: &a,
            })
            .expect_err(target);
            assert!(err.to_string().contains("unsupported target"), "{target}");
        }
    }

    #[test]
    fn writes_remote_root_and_account_ref_targets() {
        let b = bundle(json!({
            "id": "x",
            "prompts": [
                { "key": "root", "applies_to": ["a"], "target": "remote_root" },
                { "key": "acct", "applies_to": ["a"], "target": "account_ref" }
            ],
            "mounts": [{ "key": "a", "subpath": "a" }]
        }));
        let m = plan(
            &b,
            &["a"],
            "/",
            &answers(&[("root", "INBOX"), ("acct", "acct-2")]),
        );
        assert_eq!(m[0].remote_root.as_deref(), Some("INBOX"));
        assert_eq!(m[0].account_ref.as_deref(), Some("acct-2"));
        assert_eq!(m[0].mount_path, "/a");
    }

    #[test]
    fn hides_a_conditional_prompt_until_its_condition_holds_and_asks_only_what_is_selected() {
        let b = ms365();
        let none = BTreeMap::new();
        let visible: Vec<&str> = active_prompts(&b, &keys(&["onedrive"]), &none)
            .iter()
            .map(|p| p.key.as_str())
            .collect();
        assert_eq!(visible, vec!["drive_scope"]);
        let visible: Vec<&str> = active_prompts(&b, &keys(&["inbox"]), &none)
            .iter()
            .map(|p| p.key.as_str())
            .collect();
        assert_eq!(visible, vec!["mailbox"]);
    }

    #[test]
    fn reports_a_required_prompt_as_missing_only_while_it_is_active() {
        let b = ms365();
        let k = keys(&["onedrive"]);
        assert!(missing_prompt_keys(&b, &k, &answers(&[("drive_scope", "me")])).is_empty());
        assert_eq!(
            missing_prompt_keys(&b, &k, &answers(&[("drive_scope", "site")])),
            keys(&["site_id"])
        );
        assert!(missing_prompt_keys(
            &b,
            &k,
            &answers(&[("drive_scope", "site"), ("site_id", "s")])
        )
        .is_empty());
        // Not selected → not asked, even while the condition holds.
        assert!(
            missing_prompt_keys(&b, &keys(&["inbox"]), &answers(&[("drive_scope", "site")]))
                .is_empty()
        );
    }

    #[test]
    fn workspace_gate_rules_match_the_engine() {
        assert!(type_list_admits(&[], "stripe:Refund"));
        assert!(type_list_admits(&["*".into()], "stripe:Refund"));
        assert!(type_list_admits(&["stripe:Refund".into()], "stripe:Refund"));
        assert!(!type_list_admits(
            &["studio:Folder".into()],
            "raisin:Folder"
        ));
        let b = stripe();
        let m = plan(&b, &["sessions", "refunds"], "/stripe", &BTreeMap::new());
        assert_eq!(
            needed_types(&m),
            keys(&["raisin:Folder", "stripe:CheckoutSession", "stripe:Refund"])
        );
        assert_eq!(
            top_level_folder("/stripe/sessions").as_deref(),
            Some("/stripe")
        );
        assert_eq!(top_level_folder("/"), None);
    }

    #[test]
    fn collision_is_by_name_or_by_workspace_and_path() {
        let b = stripe();
        let m = &plan(&b, &["sessions"], "/stripe", &BTreeMap::new())[0];
        assert!(collides(m, "maravilla-stripe-sessions", "x", "/y"));
        assert!(collides(m, "other", "finance", "/stripe//sessions/"));
        assert!(!collides(m, "other", "stories", "/stripe/sessions"));
    }

    #[test]
    fn a_malformed_bundle_is_skipped_and_reported_not_fatal() {
        let (ok, errs) = parse_bundles(&json!([
            { "id": "good", "mounts": [] },
            { "id": "bad", "mounts": "nope" }
        ]));
        assert_eq!(ok.len(), 1);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("bad"));
        assert!(parse_bundles(&Value::Null).0.is_empty());
    }

    /// The shipped connector templates must parse: a template that does not
    /// is a bundle the endpoint can never instantiate.
    #[test]
    fn shipped_connector_templates_parse() {
        for (name, src) in [
            (
                "gmail",
                include_str!(
                    "../../../../../builtin-packages/imap-adapter/content/_raisin__system/connectors/gmail/.node.yaml"
                ),
            ),
            (
                "imap",
                include_str!(
                    "../../../../../builtin-packages/imap-adapter/content/_raisin__system/connectors/imap/.node.yaml"
                ),
            ),
        ] {
            let node: serde_yaml::Value = serde_yaml::from_str(src).expect(name);
            let bundles = serde_json::to_value(&node["properties"]["mount_bundles"]).expect(name);
            let (ok, errs) = parse_bundles(&bundles);
            assert!(errs.is_empty(), "{name}: {errs:?}");
            assert!(!ok.is_empty(), "{name} ships a bundle");
            for b in &ok {
                let k: Vec<String> = b.mounts.iter().map(|e| e.key.clone()).collect();
                // Every prompt target is in the closed set.
                let a: BTreeMap<String, String> =
                    b.prompts.iter().map(|p| (p.key.clone(), "x".to_string())).collect();
                plan_bundle(&BundlePlanInput {
                    integration_name: name,
                    integration_title: name,
                    integration_path: "",
                    bundle: b,
                    keys: &k,
                    account_ref: None,
                    target_workspace: "workplace",
                    target_branch: None,
                    root: "/r",
                    answers: &a,
                })
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            }
        }
    }
}
