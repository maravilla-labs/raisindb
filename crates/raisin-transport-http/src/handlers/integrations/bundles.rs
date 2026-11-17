// SPDX-License-Identifier: BSL-1.1

//! `POST /api/integrations/{repo}/mount-bundles` — instantiate a connector's
//! mount bundle into `raisin:VirtualMount` nodes — and its operator door
//! `POST /management/integrations/{repo}/mount-bundles`.
//!
//! The console's "Add bundle" dialog did this client-side, so a control plane
//! (flightdeck) had no way to create a tenant's Stripe mounts short of
//! retyping the dialog. The planning rule is shared, not copied:
//! [`raisin_models::nodes::integrations::bundle`] is the port of the console's
//! pure `planBundle`, with its tests.
//!
//! What this adds over the dialog, because an API caller cannot see a screen:
//!
//! - **The workspace gate refuses** (409 `WORKSPACE_GATE`) rather than warning.
//!   A target workspace that rejects a mount's node types makes the first walk
//!   report `ok` while writing nothing, then set `backfill_complete`: a fixable
//!   misconfiguration turned into a permanently empty mount. The root folder
//!   the engine creates at the workspace root is checked against
//!   `allowed_root_node_types` as well, which the dialog does not do.
//! - **Unknown entry keys refuse** (400) instead of being dropped.
//! - **Each new mount's first walk is enqueued at once** (the "sync now"
//!   job), instead of waiting for the next scheduler pass.
//! - **No connected account refuses** (409 `NO_CONNECTED_ACCOUNT`): mounts are
//!   created after consent, never before.
//!
//! Idempotent. A planned mount whose name, or whose workspace and path, is
//! already taken is reported `exists` and not touched, so a control plane can
//! re-run this after a partial failure. `dry_run: true` plans and gates without
//! writing anything. Creates are sequential with no rollback, as in the dialog:
//! a mount that exists is a mount that syncs, and the outcome list says which
//! landed.

use std::collections::{BTreeMap, BTreeSet};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::integrations::bundle::{
    self, BundlePlanInput, MountBundle, PlannedMount, CONNECTORS_ROOT, FOLDER_TYPE,
    INTEGRATIONS_ROOT, MOUNTS_ROOT,
};
use raisin_models::nodes::integrations::{
    AccountSelection, AccountSelectionError, ConnectedAccount,
};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::scope::RepoScope;
use raisin_storage::{Storage, WorkspaceRepository};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::instances::ensure_root_folder;
use super::{config_service, connected_accounts, json_prop, require_admin};
use super::{INTEGRATION_NODE_TYPE, MOUNT_NODE_TYPE};
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::{AppState, Store};

const ACTOR: &str = "integration-mount-bundles";

/// Request body.
#[derive(Debug, Default, Deserialize)]
pub struct InstantiateBundleRequest {
    /// The connector INSTANCE (`/integrations/<name>`), never a template.
    pub integration_path: String,
    /// Which bundle. Optional when the connector has exactly one.
    #[serde(default)]
    pub bundle_id: Option<String>,
    /// Entries to create, by key. Defaults to the entries marked `default`.
    #[serde(default)]
    pub keys: Option<Vec<String>>,
    /// Connection to sync. Optional when the connector has exactly one.
    #[serde(default)]
    pub account_ref: Option<String>,
    /// Defaults to the bundle's `default_workspace`.
    #[serde(default)]
    pub target_workspace: Option<String>,
    /// Branch the mounts materialise into. Defaults to `main`.
    #[serde(default)]
    pub target_branch: Option<String>,
    /// Root folder. Defaults to the bundle's `default_root`, else `/`.
    #[serde(default)]
    pub root: Option<String>,
    /// Answers to the bundle's prompts, by prompt key.
    #[serde(default)]
    pub answers: BTreeMap<String, String>,
    /// Plan and gate only; write nothing.
    #[serde(default)]
    pub dry_run: bool,
}

/// What happened to one planned mount.
#[derive(Debug, Serialize, PartialEq)]
pub struct MountOutcome {
    pub key: String,
    pub name: String,
    /// The mount node's path in `raisin:system`.
    pub path: String,
    pub target_workspace: String,
    pub mount_path: String,
    /// `created` | `exists` | `failed` | `planned` (dry run).
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The properties written (or, in a dry run, that would be).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mount: Option<PlannedMount>,
    /// For a mount created here: its first walk, enqueued at once rather than
    /// left for the scheduler. `queued`, `already_running`, or the reason the
    /// enqueue failed (the mount still exists and the scheduler picks it up).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_sync: Option<String>,
}

/// Response body.
#[derive(Debug, Serialize)]
pub struct InstantiateBundleResponse {
    pub integration_path: String,
    pub bundle_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_ref: Option<String>,
    pub dry_run: bool,
    pub created: usize,
    pub existing: usize,
    pub failed: usize,
    pub mounts: Vec<MountOutcome>,
    /// Gate failures. Non-empty only in a dry run: otherwise they are a 409.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blocked: Vec<String>,
    /// Things that will not stop the mounts being created but will stop them
    /// working, e.g. a Stripe connection without `checkout_success_url`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Pick the bundle a request means.
pub(crate) fn select_bundle(
    bundles: Vec<MountBundle>,
    bundle_id: Option<&str>,
) -> Result<MountBundle, ApiError> {
    let ids = || {
        bundles
            .iter()
            .map(|b| b.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match bundle_id.filter(|b| !b.is_empty()) {
        Some(id) => {
            let known = ids();
            bundles.into_iter().find(|b| b.id == id).ok_or_else(|| {
                ApiError::new(
                    StatusCode::NOT_FOUND,
                    "BUNDLE_NOT_FOUND",
                    format!("this connector has no mount bundle `{id}` (has: {known})"),
                )
            })
        }
        None => match bundles.len() {
            0 => Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "BUNDLE_NOT_FOUND",
                "this connector (and its template) declares no mount bundles",
            )),
            1 => Ok(bundles.into_iter().next().expect("one bundle")),
            _ => Err(ApiError::validation_failed(format!(
                "this connector has several mount bundles; name one in `bundle_id` ({})",
                ids()
            ))
            .with_field("bundle_id")),
        },
    }
}

/// The entry keys a request selects, validated against the bundle.
pub(crate) fn select_keys(
    bundle: &MountBundle,
    keys: Option<&[String]>,
) -> Result<Vec<String>, ApiError> {
    let keys = match keys {
        Some(k) => k.to_vec(),
        None => bundle::default_keys(bundle),
    };
    let unknown = bundle::unknown_keys(bundle, &keys);
    if !unknown.is_empty() {
        return Err(ApiError::validation_failed(format!(
            "bundle `{}` has no entries {}; it has {}",
            bundle.id,
            unknown.join(", "),
            bundle
                .mounts
                .iter()
                .map(|e| e.key.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .with_field("keys"));
    }
    if keys.is_empty() {
        return Err(ApiError::validation_failed(format!(
            "bundle `{}` marks no entry as default; name the entries in `keys`",
            bundle.id
        ))
        .with_field("keys"));
    }
    Ok(keys)
}

/// Map the shared account-selection rule onto HTTP.
pub(crate) fn account_error(e: AccountSelectionError) -> ApiError {
    match e {
        AccountSelectionError::NoAccounts => ApiError::new(
            StatusCode::CONFLICT,
            "NO_CONNECTED_ACCOUNT",
            "the connector has no connected account yet; complete the consent \
             (oauth/start) before creating its mounts",
        ),
        AccountSelectionError::NotFound { .. } => {
            ApiError::validation_failed(e.to_string()).with_field("account_ref")
        }
        AccountSelectionError::Ambiguous { .. } => ApiError::new(
            StatusCode::BAD_REQUEST,
            "ACCOUNT_REQUIRED",
            format!("{e}; name one in `account_ref`"),
        ),
    }
}

/// Warnings the console shows beside the Create button, as data.
///
/// The one provider-specific check the dialog keeps: a checkout session needs
/// a return URL, which is read off the CONNECTION rather than the bundle.
pub(crate) fn warnings_for(planned: &[PlannedMount], account: &ConnectedAccount) -> Vec<String> {
    let mut out = Vec::new();
    let sells = planned.iter().any(|m| {
        m.sync_config.get("resource").and_then(Value::as_str) == Some("checkout_sessions")
    });
    let has_return_url = account
        .config
        .as_ref()
        .and_then(|c| c.get("checkout_success_url"))
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());
    if sells && !has_return_url {
        out.push(format!(
            "connection `{}` has no checkout_success_url: every checkout session node must then \
             carry its own success_url or its creation fails. Set it with PATCH \
             connections/{{account_id}}.",
            account.id
        ));
    }
    out
}

/// One existing mount, as much of it as collision detection needs.
pub(crate) struct ExistingMount {
    pub name: String,
    pub target_workspace: String,
    pub mount_path: String,
}

impl ExistingMount {
    fn from_node(n: &Node) -> Self {
        let s = |k: &str| match n.properties.get(k) {
            Some(PropertyValue::String(s)) => s.clone(),
            _ => String::new(),
        };
        Self {
            name: n.name.clone(),
            target_workspace: s("target_workspace"),
            mount_path: s("mount_path"),
        }
    }
}

/// Which planned mounts are already there.
pub(crate) fn already_present(planned: &PlannedMount, existing: &[ExistingMount]) -> bool {
    existing
        .iter()
        .any(|e| bundle::collides(planned, &e.name, &e.target_workspace, &e.mount_path))
}

/// One destination's gate verdict, as text. Pure: the caller supplies the
/// workspace's type lists (or `None` when the workspace does not exist) and
/// which top-level folders are missing.
pub(crate) fn gate_problems(
    workspace: &str,
    allowed: Option<(&[String], &[String])>,
    needed: &[String],
    missing_top_folders: &[String],
) -> Vec<String> {
    let Some((node_types, root_types)) = allowed else {
        return vec![format!(
            "workspace `{workspace}` does not exist in this repository"
        )];
    };
    let mut out = Vec::new();
    let refused: Vec<&str> = needed
        .iter()
        .filter(|t| !bundle::type_list_admits(node_types, t))
        .map(String::as_str)
        .collect();
    if !refused.is_empty() {
        out.push(format!(
            "workspace `{workspace}` does not allow node types {}; install or reinstall the \
             package whose workspace patch adds them",
            refused.join(", ")
        ));
    }
    if !missing_top_folders.is_empty() && !bundle::type_list_admits(root_types, FOLDER_TYPE) {
        out.push(format!(
            "workspace `{workspace}` does not allow `{FOLDER_TYPE}` at its root, so the engine \
             cannot create {}; create the folder first or allow the type",
            missing_top_folders.join(", ")
        ));
    }
    out
}

/// Bundles for an instance: its own, else its template's (same `provider_type`).
///
/// An instance minted before its package learned about bundles carries none,
/// and recreating it (and re-consenting) to get them is not acceptable; the
/// console's dialog falls back the same way.
async fn bundles_for(
    svc: &raisin_core::NodeService<Store>,
    node: &Node,
) -> Result<(Vec<MountBundle>, Vec<String>), ApiError> {
    let (own, errs) = bundle::parse_bundles(&json_prop(node, "mount_bundles"));
    if !own.is_empty() {
        return Ok((own, errs));
    }
    let provider = match node.properties.get("provider_type") {
        Some(PropertyValue::String(s)) => s.clone(),
        _ => return Ok((Vec::new(), errs)),
    };
    let prefix = format!("{CONNECTORS_ROOT}/");
    for t in svc.list_by_type(INTEGRATION_NODE_TYPE).await? {
        if !t.path.starts_with(&prefix) {
            continue;
        }
        if !matches!(t.properties.get("provider_type"), Some(PropertyValue::String(p)) if *p == provider)
        {
            continue;
        }
        let (found, terrs) = bundle::parse_bundles(&json_prop(&t, "mount_bundles"));
        if !found.is_empty() {
            return Ok((found, terrs));
        }
    }
    Ok((Vec::new(), errs))
}

fn string_prop(node: &Node, key: &str) -> Option<String> {
    match node.properties.get(key) {
        Some(PropertyValue::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// A fresh `raisin:VirtualMount` node for a planned mount.
pub(crate) fn mount_node(m: &PlannedMount) -> Result<Node, ApiError> {
    let mut properties = std::collections::HashMap::new();
    for (k, v) in m.properties() {
        let pv = serde_json::from_value::<PropertyValue>(v)
            .map_err(|e| ApiError::internal(format!("failed to encode mount `{k}`: {e}")))?;
        properties.insert(k, pv);
    }
    Ok(Node {
        id: nanoid::nanoid!(),
        node_type: MOUNT_NODE_TYPE.to_string(),
        name: m.name.clone(),
        path: format!("{MOUNTS_ROOT}/{}", m.name),
        parent: Some(MOUNTS_ROOT.trim_start_matches('/').to_string()),
        properties,
        ..Default::default()
    })
}

/// Instantiate a mount bundle.
pub async fn instantiate_bundle(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<InstantiateBundleRequest>,
) -> Result<Json<InstantiateBundleResponse>, ApiError> {
    require_admin(auth.as_deref())?;
    let tenant_id = tenant.tenant_id.as_str();
    let svc = config_service(&state, tenant_id, &repo, ACTOR);

    // ---- the connector ----
    let node = svc
        .get_by_path(&req.integration_path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(req.integration_path.clone()))?;
    if node.node_type != INTEGRATION_NODE_TYPE {
        return Err(ApiError::validation_failed(format!(
            "{} is a {}, not a {INTEGRATION_NODE_TYPE}",
            node.path, node.node_type
        ))
        .with_field("integration_path"));
    }
    if !node.path.starts_with(&format!("{INTEGRATIONS_ROOT}/")) {
        return Err(ApiError::validation_failed(format!(
            "mounts point at a connector INSTANCE under {INTEGRATIONS_ROOT}, not at a template \
             ({}); create the instance first",
            node.path
        ))
        .with_field("integration_path"));
    }

    let (bundles, parse_errors) = bundles_for(&svc, &node).await?;
    if !parse_errors.is_empty() {
        tracing::warn!(connector = %node.path, errors = ?parse_errors, "skipped malformed mount bundles");
    }
    let bundle = select_bundle(bundles, req.bundle_id.as_deref())?;
    let keys = select_keys(&bundle, req.keys.as_deref())?;

    // ---- the connection ----
    let accounts: Vec<ConnectedAccount> = connected_accounts(&node)
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();
    let account = AccountSelection::resolve(&accounts, req.account_ref.as_deref())
        .map_err(account_error)?
        .clone();

    // ---- the operator's choices ----
    let target_workspace = req
        .target_workspace
        .clone()
        .or_else(|| bundle.default_workspace.clone())
        .filter(|w| !w.is_empty())
        .ok_or_else(|| {
            ApiError::validation_failed(format!(
                "bundle `{}` suggests no workspace; name one in `target_workspace`",
                bundle.id
            ))
            .with_field("target_workspace")
        })?;
    let root = req
        .root
        .clone()
        .or_else(|| bundle.default_root.clone())
        .unwrap_or_else(|| "/".to_string());

    let missing = bundle::missing_prompt_keys(&bundle, &keys, &req.answers);
    if !missing.is_empty() {
        return Err(ApiError::validation_failed(format!(
            "bundle `{}` needs answers for {}",
            bundle.id,
            missing.join(", ")
        ))
        .with_field("answers"));
    }

    let integration_title = string_prop(&node, "title").unwrap_or_else(|| node.name.clone());
    let planned = bundle::plan_bundle(&BundlePlanInput {
        integration_name: &node.name,
        integration_title: &integration_title,
        integration_path: &node.path,
        bundle: &bundle,
        keys: &keys,
        account_ref: Some(account.id.as_str()),
        target_workspace: &target_workspace,
        target_branch: req.target_branch.as_deref(),
        root: &root,
        answers: &req.answers,
    })
    .map_err(|e| ApiError::validation_failed(e.to_string()))?;

    // ---- the workspace gate, once per destination ----
    let mut blocked = Vec::new();
    let destinations: BTreeSet<&str> = planned
        .iter()
        .map(|m| m.target_workspace.as_str())
        .collect();
    for ws in destinations {
        let mounts: Vec<&PlannedMount> = planned
            .iter()
            .filter(|m| m.target_workspace == ws)
            .collect();
        let def = state
            .storage()
            .workspaces()
            .get(RepoScope::new(tenant_id, &repo), ws)
            .await?;
        let mut missing_top = Vec::new();
        if def.is_some() {
            let tops: BTreeSet<(String, String)> = mounts
                .iter()
                .filter_map(|m| {
                    bundle::top_level_folder(&m.mount_path).map(|t| (m.target_branch.clone(), t))
                })
                .collect();
            for (branch, top) in tops {
                let ws_svc = state.node_service_for_context(
                    tenant_id,
                    &repo,
                    &branch,
                    ws,
                    Some(AuthContext::system()),
                );
                if ws_svc.get_by_path(&top).await?.is_none() {
                    missing_top.push(top);
                }
            }
        }
        let needed = bundle::needed_types(mounts.iter().copied());
        blocked.extend(gate_problems(
            ws,
            def.as_ref().map(|d| {
                (
                    d.allowed_node_types.as_slice(),
                    d.allowed_root_node_types.as_slice(),
                )
            }),
            &needed,
            &missing_top,
        ));
    }
    if !blocked.is_empty() && !req.dry_run {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "WORKSPACE_GATE",
            format!(
                "no mount was created: {}. A mount whose workspace refuses its types reports ok \
                 while importing nothing.",
                blocked.join("; ")
            ),
        ));
    }

    let warnings = warnings_for(&planned, &account);

    // ---- create, skipping what is already there ----
    let existing: Vec<ExistingMount> = svc
        .list_by_type(MOUNT_NODE_TYPE)
        .await?
        .iter()
        .map(ExistingMount::from_node)
        .collect();

    if !req.dry_run {
        ensure_root_folder(&svc, MOUNTS_ROOT).await;
    }
    let mut outcomes = Vec::with_capacity(planned.len());
    for m in planned {
        let mut outcome = MountOutcome {
            key: m.key.clone(),
            name: m.name.clone(),
            path: format!("{MOUNTS_ROOT}/{}", m.name),
            target_workspace: m.target_workspace.clone(),
            mount_path: m.mount_path.clone(),
            status: "planned",
            message: None,
            mount: None,
            first_sync: None,
        };
        if already_present(&m, &existing) {
            outcome.status = "exists";
        } else if req.dry_run {
            outcome.mount = Some(m);
        } else {
            match mount_node(&m) {
                Ok(n) => {
                    let mount_id = n.id.clone();
                    match svc.create(n).await {
                        Ok(()) => {
                            outcome.status = "created";
                            outcome.mount = Some(m);
                            // The first walk now, not at the next scheduler pass.
                            // Same job, dedup key and branch as "sync now".
                            outcome.first_sync = Some(
                                match super::manage::enqueue_mount_sync(
                                    &state, tenant_id, &repo, &mount_id, "delta",
                                )
                                .await
                                {
                                    Ok(Some(_)) => "queued".to_string(),
                                    Ok(None) => "already_running".to_string(),
                                    Err(e) => {
                                        tracing::warn!(mount = %outcome.name, error = %e.message, "first sync not enqueued; the scheduler will pick the mount up");
                                        format!("not_enqueued: {}", e.message)
                                    }
                                },
                            );
                        }
                        Err(e) => {
                            outcome.status = "failed";
                            outcome.message = Some(e.to_string());
                        }
                    }
                }
                Err(e) => {
                    outcome.status = "failed";
                    outcome.message = Some(e.into_message());
                }
            }
        }
        outcomes.push(outcome);
    }

    let count = |s: &str| outcomes.iter().filter(|o| o.status == s).count();
    let response = InstantiateBundleResponse {
        integration_path: node.path.clone(),
        bundle_id: bundle.id.clone(),
        account_ref: Some(account.id.clone()),
        dry_run: req.dry_run,
        created: count("created"),
        existing: count("exists"),
        failed: count("failed"),
        mounts: outcomes,
        blocked,
        warnings,
    };
    tracing::info!(
        tenant = %tenant_id,
        repo = %repo,
        connector = %response.integration_path,
        bundle = %response.bundle_id,
        created = response.created,
        existing = response.existing,
        failed = response.failed,
        dry_run = response.dry_run,
        "instantiated mount bundle"
    );
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stripe() -> MountBundle {
        serde_json::from_value(json!({
            "id": "stripe-finance",
            "default_workspace": "finance",
            "default_root": "/stripe",
            "mounts": [
                { "key": "sessions", "title": "Checkout sessions", "subpath": "sessions", "default": true,
                  "node_types": ["stripe:CheckoutSession"],
                  "sync_config": { "resource": "checkout_sessions", "mode": "hybrid" } },
                { "key": "refunds", "title": "Refunds", "subpath": "refunds", "default": true,
                  "node_types": ["stripe:Refund"], "sync_config": { "resource": "refunds" } },
                { "key": "products", "title": "Products", "subpath": "products",
                  "node_types": ["stripe:Product"], "sync_config": { "resource": "products" } }
            ]
        }))
        .unwrap()
    }

    fn planned(keys: &[&str]) -> Vec<PlannedMount> {
        let b = stripe();
        let k: Vec<String> = keys.iter().map(|s| s.to_string()).collect();
        bundle::plan_bundle(&BundlePlanInput {
            integration_name: "maravilla-stripe",
            integration_title: "Stripe",
            integration_path: "/integrations/maravilla-stripe",
            bundle: &b,
            keys: &k,
            account_ref: Some("acct-1"),
            target_workspace: "finance",
            target_branch: None,
            root: "/stripe",
            answers: &BTreeMap::new(),
        })
        .unwrap()
    }

    fn account(config: Value) -> ConnectedAccount {
        serde_json::from_value(json!({ "id": "acct-1", "config": config })).unwrap()
    }

    #[test]
    fn a_sole_bundle_is_chosen_and_several_need_an_id() {
        assert_eq!(
            select_bundle(vec![stripe()], None).unwrap().id,
            "stripe-finance"
        );
        let mut other = stripe();
        other.id = "other".into();
        let err = select_bundle(vec![stripe(), other.clone()], None).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            select_bundle(vec![stripe(), other], Some("other"))
                .unwrap()
                .id,
            "other"
        );
        let err = select_bundle(vec![stripe()], Some("nope")).unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert_eq!(
            select_bundle(vec![], None).unwrap_err().status,
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn keys_default_to_the_marked_entries_and_typos_refuse() {
        let b = stripe();
        assert_eq!(select_keys(&b, None).unwrap(), vec!["sessions", "refunds"]);
        let k = vec!["products".to_string()];
        assert_eq!(select_keys(&b, Some(&k)).unwrap(), k);
        let typo = vec!["sesions".to_string()];
        assert_eq!(
            select_keys(&b, Some(&typo)).unwrap_err().status,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            select_keys(&b, Some(&[])).unwrap_err().status,
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn mounts_wait_for_consent() {
        let err = account_error(AccountSelectionError::NoAccounts);
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.code, "NO_CONNECTED_ACCOUNT");
        let err = account_error(AccountSelectionError::Ambiguous {
            candidates: vec!["a".into(), "b".into()],
        });
        assert_eq!(err.code, "ACCOUNT_REQUIRED");
    }

    #[test]
    fn a_checkout_mount_without_a_return_url_warns() {
        let p = planned(&["sessions"]);
        assert_eq!(warnings_for(&p, &account(json!({}))).len(), 1);
        assert!(warnings_for(
            &p,
            &account(json!({ "checkout_success_url": "https://x.test/ok" }))
        )
        .is_empty());
        // Not selling: nothing to warn about.
        assert!(warnings_for(&planned(&["products"]), &account(json!({}))).is_empty());
    }

    #[test]
    fn existing_mounts_are_recognised_by_name_or_destination() {
        let p = &planned(&["sessions"])[0];
        let by_name = ExistingMount {
            name: "maravilla-stripe-sessions".into(),
            target_workspace: "elsewhere".into(),
            mount_path: "/x".into(),
        };
        let by_dest = ExistingMount {
            name: "hand-made".into(),
            target_workspace: "finance".into(),
            mount_path: "/stripe/sessions".into(),
        };
        let unrelated = ExistingMount {
            name: "hand-made".into(),
            target_workspace: "finance".into(),
            mount_path: "/stripe/refunds".into(),
        };
        assert!(already_present(p, &[by_name]));
        assert!(already_present(p, &[by_dest]));
        assert!(!already_present(p, &[unrelated]));
    }

    #[test]
    fn the_gate_names_what_the_workspace_refuses() {
        let p = planned(&["sessions", "refunds"]);
        let needed = bundle::needed_types(&p);
        let studio_finance: Vec<String> =
            vec!["studio:Folder".into(), "stripe:CheckoutSession".into()];
        let problems = gate_problems("finance", Some((&studio_finance, &[])), &needed, &[]);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("raisin:Folder"));
        assert!(problems[0].contains("stripe:Refund"));
        assert!(!problems[0].contains("stripe:CheckoutSession"));

        let all: Vec<String> = vec!["*".into()];
        assert!(
            gate_problems("finance", Some((&all, &[])), &needed, &["/stripe".into()]).is_empty()
        );

        // The root folder is gated by the ROOT list, and only when it is missing.
        let roots: Vec<String> = vec!["studio:Folder".into()];
        assert_eq!(
            gate_problems(
                "finance",
                Some((&all, &roots)),
                &needed,
                &["/stripe".into()]
            )
            .len(),
            1
        );
        assert!(gate_problems("finance", Some((&all, &roots)), &needed, &[]).is_empty());

        let missing = gate_problems("finance", None, &needed, &[]);
        assert!(missing[0].contains("does not exist"));
    }

    #[test]
    fn a_mount_node_lives_under_mounts_and_carries_no_state() {
        let p = &planned(&["sessions"])[0];
        let n = mount_node(p).unwrap();
        assert_eq!(n.node_type, MOUNT_NODE_TYPE);
        assert_eq!(n.path, "/mounts/maravilla-stripe-sessions");
        assert_eq!(n.parent.as_deref(), Some("mounts"));
        assert!(!n.id.is_empty());
        assert!(!n.properties.contains_key("state"));
        assert_eq!(
            n.properties.get("account_ref"),
            Some(&PropertyValue::String("acct-1".into()))
        );
        assert_eq!(
            n.properties.get("enabled"),
            Some(&PropertyValue::Boolean(true))
        );
        // Round-trips through the JSON the sync engine reads.
        let back = super::json_prop(&n, "sync_config");
        assert_eq!(back["resource"], json!("checkout_sessions"));
    }

    #[test]
    fn the_request_needs_only_the_instance_path() {
        let r: InstantiateBundleRequest =
            serde_json::from_value(json!({ "integration_path": "/integrations/maravilla-stripe" }))
                .unwrap();
        assert!(r.keys.is_none());
        assert!(!r.dry_run);
        assert!(r.answers.is_empty());
    }
}
