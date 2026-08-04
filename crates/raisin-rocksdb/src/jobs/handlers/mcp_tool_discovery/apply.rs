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

//! Turning a reconcile plan into node writes.

use std::sync::Arc;

use raisin_core::services::node_service::NodeService;
use raisin_crypto::SecretBox;
use raisin_error::Result;
use raisin_mcp_protocol::client::{
    AuthKind, ExistingProxy, McpConnectionDescriptor, ProxyPlan, ReconcileAction, RemoteToolError,
    ServerHandshake,
};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{RepositoryManagementRepository, Storage};
use serde_json::{json, Value};

use super::{
    error_detail, health_status_for, CONNECTION_NODE_TYPE, DISCOVERY_ACTOR, FUNCTIONS_WORKSPACE,
};
use crate::RocksDBStorage;

/// NodeType of a generated proxy.
const FUNCTION_NODE_TYPE: &str = "raisin:Function";
/// NodeType of the folders proxies live under.
const FOLDER_NODE_TYPE: &str = "raisin:Folder";

/// Resolve a repo's default branch, falling back to `main`.
pub(crate) async fn default_branch(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
) -> String {
    storage
        .repository_management()
        .get_repository(tenant, repo)
        .await
        .ok()
        .flatten()
        .map(|r| r.config.default_branch)
        .unwrap_or_else(|| "main".to_string())
}

/// A system-auth `NodeService` stamped with the discovery actor.
pub(crate) fn system_service(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
) -> NodeService<RocksDBStorage> {
    let mut auth = AuthContext::system();
    auth.user_id = Some(DISCOVERY_ACTOR.to_string());
    NodeService::new_with_context(
        storage.clone(),
        tenant.to_string(),
        repo.to_string(),
        branch.to_string(),
        workspace.to_string(),
    )
    .with_auth(auth)
}

/// Decrypt the credential this connection presents, if any.
///
/// A blob we cannot decrypt yields `None` and is logged loudly: proceeding
/// unauthenticated turns a master-key mismatch into a bare 401 from the remote,
/// which is a far harder diagnosis than the real cause.
pub(crate) fn decrypt_credential(
    node: &Node,
    descriptor: &McpConnectionDescriptor,
) -> Option<String> {
    let (property, field) = match descriptor.auth_kind {
        AuthKind::None => return None,
        AuthKind::Static => ("credential_encrypted", "value"),
        AuthKind::Oauth => ("tokens_encrypted", "access_token"),
    };
    let ciphertext = match node.properties.get(property) {
        Some(PropertyValue::String(s)) if !s.is_empty() => s.clone(),
        _ => return None,
    };
    let master_key = raisin_crypto::master_key_with_embedding_fallback()
        .ok()
        .flatten()?;

    let plaintext = SecretBox::new(&master_key)
        .decrypt_json(&ciphertext)
        .ok()
        .and_then(|v| {
            v.get(field)
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|s| !s.is_empty())
        });
    if plaintext.is_none() {
        tracing::error!(
            node_path = %node.path,
            property,
            "mcp connection has a stored credential that could not be decrypted; discovery will \
             proceed UNAUTHENTICATED and the remote will reject it. RAISIN_MASTER_KEY most \
             likely differs from the key it was stored under."
        );
    }
    plaintext
}

/// Read the connection's recorded proxies.
pub(crate) fn existing_proxies(node: &Node) -> Vec<ExistingProxy> {
    let Some(PropertyValue::Array(_)) = node.properties.get("discovered_tools") else {
        return Vec::new();
    };
    let value = node
        .properties
        .get("discovered_tools")
        .and_then(|pv| serde_json::to_value(pv).ok())
        .unwrap_or(Value::Null);

    value
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| {
                    Some(ExistingProxy {
                        remote_name: e.get("remote_name")?.as_str()?.to_string(),
                        function_path: e
                            .get("function_path")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        schema_hash: e
                            .get("schema_hash")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        enabled: e.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                        state: e
                            .get("state")
                            .and_then(Value::as_str)
                            .unwrap_or("active")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Record a failed probe on the connection without touching its proxies.
///
/// A remote being down must NOT disable the tools an agent already has: the
/// listing is unknown, not empty, and treating it as empty would tear down every
/// proxy on a transient outage and rebuild it minutes later.
pub(crate) async fn record_health(
    svc: &NodeService<RocksDBStorage>,
    mut node: Node,
    code: &str,
    err: &RemoteToolError,
) -> Result<()> {
    let health = json!({
        "status": health_status_for(code),
        "checked_at": chrono::Utc::now().to_rfc3339(),
        "error_code": code,
        "error": error_detail(err),
    });
    set_prop(&mut node, "health", health)?;
    svc.update_node(node).await
}

/// Apply a reconcile plan: proxy writes, then ONE connection write.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn apply_actions(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    descriptor: &McpConnectionDescriptor,
    connection: &Node,
    actions: &[ReconcileAction],
    handshake: &ServerHandshake,
    remote_count: usize,
) -> Result<Value> {
    let functions = system_service(storage, tenant, repo, branch, FUNCTIONS_WORKSPACE);

    ensure_folders(&functions, &descriptor.slug).await?;

    let (mut created, mut updated, mut unchanged, mut missing, mut conflicts) = (0, 0, 0, 0, 0);
    let mut records: Vec<Value> = Vec::with_capacity(actions.len());

    for action in actions {
        match action {
            ReconcileAction::Unchanged(plan) => {
                unchanged += 1;
                records.push(tool_record(plan, "active"));
            }
            ReconcileAction::Create(plan) | ReconcileAction::Update(plan) => {
                let node = build_proxy(descriptor, plan);
                let is_create = matches!(action, ReconcileAction::Create(_));
                let result = if is_create {
                    functions.create(node).await
                } else {
                    write_existing(&functions, node, &plan.function_path).await
                };
                match result {
                    Ok(()) => {
                        if is_create {
                            created += 1;
                        } else {
                            updated += 1;
                        }
                        records.push(tool_record(plan, "active"));
                    }
                    Err(e) => {
                        // A unique-name clash with a hand-written function must
                        // not fail the whole sync — report it and keep going.
                        conflicts += 1;
                        tracing::warn!(
                            slug = %descriptor.slug,
                            remote_tool = %plan.remote_name,
                            path = %plan.function_path,
                            error = %e,
                            "mcp discovery: could not write proxy function"
                        );
                        records.push(tool_record(plan, "conflict"));
                    }
                }
            }
            ReconcileAction::MarkMissing {
                remote_name,
                function_path,
            } => {
                missing += 1;
                if let Err(e) = disable_proxy(&functions, function_path).await {
                    tracing::warn!(path = %function_path, error = %e, "mcp discovery: could not disable missing proxy");
                }
                records.push(json!({
                    "remote_name": remote_name,
                    "function_path": function_path,
                    "enabled": false,
                    "state": "missing",
                }));
            }
        }
    }

    let connections = system_service(storage, tenant, repo, branch, super::SYSTEM_WORKSPACE);
    let mut node = connection.clone();
    set_prop(&mut node, "discovered_tools", Value::Array(records))?;
    set_prop(
        &mut node,
        "protocol_version",
        json!(handshake.protocol_version),
    )?;
    set_prop(
        &mut node,
        "health",
        json!({
            "status": "ok",
            "checked_at": chrono::Utc::now().to_rfc3339(),
            "server_info": handshake.server_info.as_ref().and_then(|i| serde_json::to_value(i).ok()),
            "capabilities": serde_json::to_value(&handshake.capabilities).ok(),
            "tool_count": remote_count,
        }),
    )?;
    set_prop(
        &mut node,
        "last_synced_at",
        json!(chrono::Utc::now().to_rfc3339()),
    )?;
    connections.update_node(node).await?;

    let summary = json!({
        "created": created,
        "updated": updated,
        "unchanged": unchanged,
        "missing": missing,
        "conflicts": conflicts,
        "tool_count": remote_count,
    });
    tracing::info!(slug = %descriptor.slug, summary = %summary, "mcp tool discovery complete");
    Ok(summary)
}

/// Create `/mcp` and `/mcp/{slug}` if absent.
async fn ensure_folders(svc: &NodeService<RocksDBStorage>, slug: &str) -> Result<()> {
    for (path, name, parent) in [
        ("/mcp".to_string(), "mcp".to_string(), None),
        (
            format!("/mcp/{slug}"),
            slug.to_string(),
            Some("mcp".to_string()),
        ),
    ] {
        if svc.get_by_path(&path).await?.is_some() {
            continue;
        }
        let mut folder = Node {
            node_type: FOLDER_NODE_TYPE.to_string(),
            name: name.clone(),
            path,
            parent,
            ..Default::default()
        };
        set_prop(&mut folder, "title", json!(name))?;
        // A concurrent discovery may have created it between the probe and
        // here; that is success, not failure.
        if let Err(e) = svc.create(folder).await {
            tracing::debug!(error = %e, "mcp discovery: folder create raced (harmless)");
        }
    }
    Ok(())
}

/// Build the proxy `raisin:Function` node for one planned tool.
fn build_proxy(descriptor: &McpConnectionDescriptor, plan: &ProxyPlan) -> Node {
    let mut node = Node {
        node_type: FUNCTION_NODE_TYPE.to_string(),
        // Node name and `props.name` MUST match: the JS resolver reads the node
        // name and the Rust one reads `props.name`, so a mismatch shows the
        // model two different tool names for the same tool.
        name: plan.function_name.clone(),
        path: plan.function_path.clone(),
        parent: Some(descriptor.slug.clone()),
        ..Default::default()
    };

    let _ = set_prop(&mut node, "name", json!(plan.function_name));
    let _ = set_prop(&mut node, "title", json!(plan.title));
    if let Some(description) = &plan.description {
        let _ = set_prop(&mut node, "description", json!(description));
    }
    // Never executed — `mcp_proxy` short-circuits the executor before the
    // language is ever read — but the property is required by the schema.
    let _ = set_prop(&mut node, "language", json!("javascript"));
    let _ = set_prop(&mut node, "execution_mode", json!("async"));
    let _ = set_prop(&mut node, "enabled", json!(plan.enabled));
    let _ = set_prop(&mut node, "category", json!("mcp"));
    let _ = set_prop(&mut node, "input_schema", plan.input_schema.clone());
    if let Some(output_schema) = &plan.output_schema {
        let _ = set_prop(&mut node, "output_schema", output_schema.clone());
    }
    let _ = set_prop(
        &mut node,
        "mcp_proxy",
        json!({
            "connection": format!("{}/{}", super::CONNECTIONS_ROOT, descriptor.slug),
            "connection_slug": descriptor.slug,
            // VERBATIM: the slug is a local path segment, never what we send.
            "remote_tool": plan.remote_name,
            "schema_hash": plan.schema_hash,
            "discovered_at": chrono::Utc::now().to_rfc3339(),
            "state": "active",
        }),
    );
    node
}

/// Overwrite an existing proxy, preserving its node id.
async fn write_existing(svc: &NodeService<RocksDBStorage>, fresh: Node, path: &str) -> Result<()> {
    match svc.get_by_path(path).await? {
        Some(current) => {
            let mut updated = fresh;
            updated.id = current.id;
            updated.created_at = current.created_at;
            updated.created_by = current.created_by;
            svc.update_node(updated).await
        }
        // Recorded but gone — recreate it.
        None => svc.create(fresh).await,
    }
}

/// Disable a proxy whose remote tool disappeared.
///
/// Disabled, never deleted: a missing node makes the tool vanish from any agent
/// referencing it with no diagnosis, while a disabled one stays visible in the
/// console and in the connection's tool list.
async fn disable_proxy(svc: &NodeService<RocksDBStorage>, path: &str) -> Result<()> {
    let Some(mut node) = svc.get_by_path(path).await? else {
        return Ok(());
    };
    set_prop(&mut node, "enabled", json!(false))?;
    if let Some(PropertyValue::Object(_)) = node.properties.get("mcp_proxy") {
        let mut proxy = node
            .properties
            .get("mcp_proxy")
            .and_then(|pv| serde_json::to_value(pv).ok())
            .unwrap_or(json!({}));
        proxy["state"] = json!("missing");
        set_prop(&mut node, "mcp_proxy", proxy)?;
    }
    svc.update_node(node).await
}

/// The connection-side record for one tool.
fn tool_record(plan: &ProxyPlan, state: &str) -> Value {
    json!({
        "remote_name": plan.remote_name,
        "function_name": plan.function_name,
        "function_path": plan.function_path,
        "schema_hash": plan.schema_hash,
        "enabled": plan.enabled && state == "active",
        "state": state,
    })
}

/// Write a JSON value into a node property.
fn set_prop(node: &mut Node, key: &str, value: Value) -> Result<()> {
    let pv = serde_json::from_value::<PropertyValue>(value)
        .map_err(|e| raisin_error::Error::Validation(format!("failed to encode `{key}`: {e}")))?;
    node.properties.insert(key.to_string(), pv);
    Ok(())
}

/// Whether a node is a connection (used by the periodic scan).
pub(crate) fn is_connection(node: &Node) -> bool {
    node.node_type == CONNECTION_NODE_TYPE
}
