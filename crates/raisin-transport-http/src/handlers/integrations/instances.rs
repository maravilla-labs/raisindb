// SPDX-License-Identifier: BSL-1.1

//! `POST /api/integrations/{repo}/instances` — create a connector INSTANCE from a
//! package-shipped template, and its operator door
//! `POST /management/integrations/{repo}/instances`.
//!
//! Packages own `/connectors/<name>` and overwrite it on every install; an
//! operator's live connector is a COPY under `/integrations/<name>`, which no
//! package declares, so a re-install can never reach its credentials or its
//! connected accounts. The admin console makes that copy client-side
//! (`createIntegrationFromTemplate`). A control plane setting a tenant up has no
//! console, so the same copy lives here, with the same rule about what is NOT
//! inherited ([`INSTANCE_RESET_PROPERTIES`]).
//!
//! Idempotent: an instance that already exists with the template's
//! `provider_type` is answered with `created: false` and left untouched. One
//! that exists for a DIFFERENT provider is a conflict, never overwritten.
//!
//! The instance keeps the template's `enabled` flag. A managed template ships
//! `enabled: false` with an empty `client_id`, meaning "no OAuth client yet";
//! the control plane mints the client and flips it (flightdeck's
//! `reconcile_connectors`). Forcing `true` here would hide that state.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::integrations::bundle::{
    CONNECTORS_ROOT, INSTANCE_RESET_PROPERTIES, INTEGRATIONS_ROOT,
};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use serde::{Deserialize, Serialize};

use super::{config_service, connected_accounts, is_connect_broker_url, json_str, oauth_config};
use super::{require_admin, INTEGRATION_NODE_TYPE};
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::{AppState, Store};

const ACTOR: &str = "integration-instances";

/// Request body.
#[derive(Debug, Deserialize)]
pub struct CreateInstanceRequest {
    /// The template: its name (`maravilla-stripe`) or its path
    /// (`/connectors/maravilla-stripe`).
    pub template: String,
    /// Instance node name. Defaults to the template's name.
    #[serde(default)]
    pub name: Option<String>,
    /// Instance title. Defaults to the template's.
    #[serde(default)]
    pub title: Option<String>,
}

/// What the caller needs to take the next step.
#[derive(Debug, Serialize)]
pub struct InstanceView {
    /// The instance path: the `integration_path` every other call takes.
    pub path: String,
    pub name: String,
    pub template: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    /// `false` when the instance already existed.
    pub created: bool,
    pub enabled: bool,
    /// Consent is brokered by Maravilla Connect, so the control plane owns the
    /// OAuth client.
    pub managed: bool,
    /// `oauth_config.client_id` is set. A managed connector cannot start
    /// consent until it is.
    pub client_id_provisioned: bool,
    pub connected_accounts: usize,
}

/// `template` as a path under `/connectors`, or `None` when it names anything
/// else. A path outside `/connectors` is refused: copying an operator
/// instance would copy its client id and encrypted secret with it.
pub(crate) fn template_path(template: &str) -> Option<String> {
    let t = template.trim().trim_end_matches('/');
    let name = match t.strip_prefix(CONNECTORS_ROOT) {
        Some(rest) => rest.strip_prefix('/')?,
        None if !t.starts_with('/') => t,
        None => return None,
    };
    valid_name(name).then(|| format!("{CONNECTORS_ROOT}/{name}"))
}

/// A single path segment of the characters a connector name is made of.
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// The instance node a template becomes: the template's type and properties,
/// minus engine-owned state and secrets, with an optional title.
pub(crate) fn instance_from_template(template: &Node, name: &str, title: Option<&str>) -> Node {
    let mut properties = template.properties.clone();
    for key in INSTANCE_RESET_PROPERTIES {
        properties.remove(*key);
    }
    if let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) {
        properties.insert("title".into(), PropertyValue::String(t.to_string()));
    }
    Node {
        // The caller mints the id; `NodeService::create` does not.
        id: nanoid::nanoid!(),
        node_type: template.node_type.clone(),
        name: name.to_string(),
        path: format!("{INTEGRATIONS_ROOT}/{name}"),
        // `parent` is the parent's NAME, not its path.
        parent: Some(INTEGRATIONS_ROOT.trim_start_matches('/').to_string()),
        properties,
        ..Default::default()
    }
}

fn provider_type(node: &Node) -> Option<String> {
    match node.properties.get("provider_type") {
        Some(PropertyValue::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

fn view(node: &Node, template: &str, created: bool) -> InstanceView {
    let cfg = oauth_config(node);
    InstanceView {
        path: node.path.clone(),
        name: node.name.clone(),
        template: template.to_string(),
        provider_type: provider_type(node),
        created,
        enabled: matches!(
            node.properties.get("enabled"),
            Some(PropertyValue::Boolean(true))
        ),
        managed: json_str(&cfg, "auth_url").is_some_and(|u| is_connect_broker_url(&u)),
        client_id_provisioned: json_str(&cfg, "client_id").is_some(),
        connected_accounts: connected_accounts(node).len(),
    }
}

/// Create the parent folder of a config node when it is missing. A race with
/// another caller is harmless: the child create below still succeeds.
pub(crate) async fn ensure_root_folder(svc: &raisin_core::NodeService<Store>, root: &str) {
    if matches!(svc.get_by_path(root).await, Ok(Some(_))) {
        return;
    }
    let name = root.trim_start_matches('/').to_string();
    let folder = Node {
        id: nanoid::nanoid!(),
        node_type: "raisin:Folder".to_string(),
        name: name.clone(),
        path: root.to_string(),
        parent: None,
        properties: [("title".to_string(), PropertyValue::String(name))]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    if let Err(e) = svc.create(folder).await {
        tracing::debug!(root, error = %e, "config root folder create skipped");
    }
}

/// Create (or find) the instance of a connector template.
pub async fn create_instance(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<CreateInstanceRequest>,
) -> Result<(StatusCode, Json<InstanceView>), ApiError> {
    require_admin(auth.as_deref())?;

    let tpl_path = template_path(&req.template).ok_or_else(|| {
        ApiError::validation_failed(format!(
            "`template` must name a connector template under {CONNECTORS_ROOT} \
             (e.g. `maravilla-stripe`), got `{}`",
            req.template
        ))
        .with_field("template")
    })?;

    let svc = config_service(&state, &tenant.tenant_id, &repo, ACTOR);
    let template = svc.get_by_path(&tpl_path).await?.ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "TEMPLATE_NOT_FOUND",
            format!("no connector template at {tpl_path}; install the package that ships it first"),
        )
    })?;
    if template.node_type != INTEGRATION_NODE_TYPE {
        return Err(ApiError::validation_failed(format!(
            "{tpl_path} is a {}, not a {INTEGRATION_NODE_TYPE}",
            template.node_type
        )));
    }

    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(&template.name)
        .to_string();
    if !valid_name(&name) {
        return Err(ApiError::validation_failed(format!(
            "instance name `{name}` must be one path segment of letters, digits, `-`, `_` or `.`"
        ))
        .with_field("name"));
    }

    let instance_path = format!("{INTEGRATIONS_ROOT}/{name}");
    if let Some(existing) = svc.get_by_path(&instance_path).await? {
        if existing.node_type == INTEGRATION_NODE_TYPE
            && provider_type(&existing) == provider_type(&template)
        {
            return Ok((StatusCode::OK, Json(view(&existing, &tpl_path, false))));
        }
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "INSTANCE_CONFLICT",
            format!(
                "{instance_path} already exists for a different provider ({}); pick another `name`",
                provider_type(&existing).unwrap_or_else(|| existing.node_type.clone())
            ),
        ));
    }

    ensure_root_folder(&svc, INTEGRATIONS_ROOT).await;
    let node = instance_from_template(&template, &name, req.title.as_deref());
    svc.create(node).await?;
    let created = svc.get_by_path(&instance_path).await?.ok_or_else(|| {
        ApiError::internal(format!(
            "{instance_path} was created but cannot be read back"
        ))
    })?;

    tracing::info!(
        tenant = %tenant.tenant_id,
        repo = %repo,
        template = %tpl_path,
        instance = %instance_path,
        "created connector instance from template"
    );
    Ok((StatusCode::CREATED, Json(view(&created, &tpl_path, true))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn template() -> Node {
        serde_json::from_value(json!({
            "id": "tpl-1",
            "name": "maravilla-stripe",
            "path": "/connectors/maravilla-stripe",
            "node_type": INTEGRATION_NODE_TYPE,
            "properties": {
                "title": "Stripe (Managed by Maravilla)",
                "provider_type": "stripe",
                "enabled": false,
                "oauth_config": { "auth_url": "https://connect.maravilla.cloud/authorize", "client_id": "" },
                "mount_bundles": [{ "id": "stripe-finance", "mounts": [] }],
                "connected_accounts": [{ "id": "leaked" }],
                "capabilities": { "read": true },
                "capabilities_checked_at": 1,
                "client_secret_encrypted": "c2VjcmV0",
                "config_secrets_encrypted": "c2VjcmV0",
                "config_secret_fields": ["api_key"]
            }
        }))
        .expect("template")
    }

    #[test]
    fn template_names_and_paths_resolve_under_connectors_only() {
        assert_eq!(
            template_path("maravilla-stripe").as_deref(),
            Some("/connectors/maravilla-stripe")
        );
        assert_eq!(
            template_path("/connectors/maravilla-stripe/").as_deref(),
            Some("/connectors/maravilla-stripe")
        );
        // Copying an operator instance would copy its credentials.
        assert_eq!(template_path("/integrations/maravilla-stripe"), None);
        assert_eq!(template_path("/connectors/a/b"), None);
        assert_eq!(template_path("../x"), None);
        assert_eq!(template_path(""), None);
    }

    #[test]
    fn an_instance_never_inherits_state_or_secrets() {
        let node = instance_from_template(&template(), "maravilla-stripe", None);
        assert_eq!(node.path, "/integrations/maravilla-stripe");
        assert_eq!(node.parent.as_deref(), Some("integrations"));
        assert_ne!(node.id, "tpl-1");
        assert!(!node.id.is_empty());
        for key in INSTANCE_RESET_PROPERTIES {
            assert!(!node.properties.contains_key(*key), "{key} was inherited");
        }
        // What the adapter author wrote travels; the managed flag is kept.
        assert!(node.properties.contains_key("mount_bundles"));
        assert!(node.properties.contains_key("oauth_config"));
        assert_eq!(
            node.properties.get("enabled"),
            Some(&PropertyValue::Boolean(false))
        );
    }

    #[test]
    fn a_title_override_replaces_only_the_title() {
        let node = instance_from_template(&template(), "stripe-eu", Some("  Stripe EU "));
        assert_eq!(
            node.properties.get("title"),
            Some(&PropertyValue::String("Stripe EU".into()))
        );
        assert_eq!(node.path, "/integrations/stripe-eu");
        let v = view(&node, "/connectors/maravilla-stripe", true);
        assert!(v.managed);
        assert!(!v.client_id_provisioned);
        assert!(!v.enabled);
        assert_eq!(v.connected_accounts, 0);
        assert_eq!(v.provider_type.as_deref(), Some("stripe"));
    }

    #[test]
    fn names_are_one_safe_segment() {
        assert!(valid_name("maravilla-stripe"));
        assert!(valid_name("stripe_eu.2"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name(".."));
        assert!(!valid_name("a b"));
        assert!(!valid_name(""));
    }
}
