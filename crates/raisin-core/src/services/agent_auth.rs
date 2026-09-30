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

//! WHOSE PERMISSIONS an AI agent runs with.
//!
//! An agent reaches storage from two directions — a tool call executed as its
//! own job, and a tool call made inside a flow step — and both must answer this
//! question identically. They did not: the first honoured the agent's
//! `execution_context`, the second always ran as the whole system, so an agent
//! restricted in the UI still had full rights the moment it ran inside a
//! workflow. One resolver, used by both, is the only way that stays true.

use std::sync::Arc;

use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{NodeRepository, Storage, StorageScope};

use crate::services::permission_service::PermissionService;

/// What an agent's `execution_context` asked for, once it has been read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentExecution {
    /// Run with full system privileges (`"system"`, or an unrecognised value).
    System,
    /// Run under the agent's OWN roles and groups (`"agent"`).
    OwnRights,
    /// Run under WHOEVER CAUSED this execution (`"user"`, the schema default).
    /// Only resolvable where the caller can name that identity — see
    /// [`resolve_agent_context`]'s `triggering_user_id` parameter.
    CallerRights,
}

/// Read `execution_context` off an agent node.
pub fn execution_of(agent: &Node) -> AgentExecution {
    match agent.properties.get("execution_context") {
        Some(PropertyValue::String(value)) if value == "agent" => AgentExecution::OwnRights,
        Some(PropertyValue::String(value)) if value == "user" => AgentExecution::CallerRights,
        _ => AgentExecution::System,
    }
}

/// The role/group ids an agent holds, tolerating the `/roles/x` path form the
/// user resolver also accepts.
pub fn granted_ids(agent: &Node, key: &str) -> Vec<String> {
    match agent.properties.get(key) {
        Some(PropertyValue::Array(values)) => values
            .iter()
            .filter_map(|value| match value {
                PropertyValue::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The roles and groups an agent's tools hold when the agent answers an
/// ANONYMOUS visitor: `anonymous.tool_roles` / `anonymous.tool_groups` when
/// set, else the agent's own `roles` / `groups`.
pub fn anonymous_tool_grant(agent: &Node) -> (Vec<String>, Vec<String>) {
    let from_block = |key: &str| -> Option<Vec<String>> {
        let Some(PropertyValue::Object(block)) = agent.properties.get("anonymous") else {
            return None;
        };
        match block.get(key) {
            Some(PropertyValue::Array(values)) => Some(
                values
                    .iter()
                    .filter_map(|value| match value {
                        PropertyValue::String(s) if !s.trim().is_empty() => {
                            Some(s.trim().to_string())
                        }
                        _ => None,
                    })
                    .collect(),
            ),
            _ => None,
        }
    };
    let roles = from_block("tool_roles");
    let groups = from_block("tool_groups");
    if roles.is_none() && groups.is_none() {
        return (granted_ids(agent, "roles"), granted_ids(agent, "groups"));
    }
    (roles.unwrap_or_default(), groups.unwrap_or_default())
}

/// Whether an acting identity is an anonymous visitor session
/// (`visitor:<key>`, see `raisin_models::auth::visitor`).
pub fn is_visitor_identity(id: Option<&str>) -> bool {
    id.is_some_and(|id| id.starts_with(raisin_models::auth::visitor::VISITOR_ID_PREFIX))
}

/// The auth context a write BY THIS AGENT should carry.
///
/// `triggering_user_id` is the raw actor id of whoever/whatever CAUSED this
/// execution — for a flow started by a node-change trigger, the actor stamped
/// on the write that fired it (see `raisin-rocksdb`'s
/// `transaction/commit/events.rs`, `metadata["actor"]`, carried through the
/// trigger-evaluation and flow-instance job chain). It is consulted only when
/// the agent asks for `CallerRights` (`execution_context: "user"`); `None`
/// means no such identity could be named for this call path (a timer trigger,
/// an API-started flow) and the caller keeps its existing context.
///
/// * `Ok(Some(ctx))` — the agent runs under its own resolved permissions, or
///   the resolved triggering user's.
/// * `Ok(None)` — the agent did not ask for elevated/caller rights, asked but
///   was granted nothing, or asked for `CallerRights` with no resolvable
///   identity; the caller keeps whatever system context it would have used.
///   An agent that may do NOTHING is a silent, baffling failure, so an empty
///   grant list is treated as "not configured" rather than as a lockout.
/// * `Err(_)` — the rights could not be resolved. The caller must FAIL CLOSED:
///   quietly inheriting system privileges is the one outcome nobody notices.
pub async fn resolve_agent_context<S>(
    storage: &Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    agent_path: &str,
    marker: &str,
    triggering_user_id: Option<&str>,
) -> Result<Option<AuthContext>, String>
where
    S: Storage + 'static,
{
    let agent = storage
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            agent_path,
            None,
        )
        .await
        .map_err(|error| format!("Failed to load agent '{}': {}", agent_path, error))?;

    // A missing agent is NOT an error here: the caller already has a perfectly
    // good system context and a marker naming what tried. Refusing the whole
    // call because an agent node moved would break flows that work today.
    let Some(agent) = agent else {
        // An anonymous visitor has no rights of its own to fall back to.
        if is_visitor_identity(triggering_user_id) {
            return Err(format!(
                "agent '{}' not found; an anonymous visitor's tools cannot run",
                agent_path
            ));
        }
        return Ok(None);
    };

    // AN ANONYMOUS VISITOR IS NEVER THE PRINCIPAL OF A TOOL CALL.
    //
    // For a person, `execution_context: "user"` means "the tools act with the
    // chatting user's rights". A visitor has none worth acting with, and
    // "system" would hand the open internet the whole repository. So a turn
    // made for a visitor always runs its tools under the agent's anonymous
    // tool grant, whatever `execution_context` says: exactly the roles the
    // site configured for this purpose (e.g. a read-only role over the pages
    // the assistant may cite). A tool whose own function node says
    // `execution_context: system` still elevates itself, as for everyone —
    // that is how a narrowly written tool (creating an inquiry) writes.
    if is_visitor_identity(triggering_user_id) {
        return resolve_visitor_tool_context(
            storage, tenant_id, repo_id, branch, &agent, agent_path, marker,
        )
        .await
        .map(Some);
    }

    match execution_of(&agent) {
        AgentExecution::System => Ok(None),
        AgentExecution::CallerRights => {
            resolve_caller_context(
                storage,
                tenant_id,
                repo_id,
                branch,
                agent_path,
                marker,
                triggering_user_id,
            )
            .await
        }
        AgentExecution::OwnRights => {
            let roles = granted_ids(&agent, "roles");
            let groups = granted_ids(&agent, "groups");
            if roles.is_empty() && groups.is_empty() {
                tracing::warn!(
                    agent_path = %agent_path,
                    "Agent is set to run under its own permissions but has no roles or groups; \
                     falling back to the caller's context"
                );
                return Ok(None);
            }

            let resolved = PermissionService::new(storage.clone())
                .resolve_for_principal_node(tenant_id, repo_id, branch, &agent)
                .await
                .map_err(|error| {
                    format!(
                        "Failed to resolve permissions for '{}': {}",
                        agent_path, error
                    )
                })?;

            tracing::info!(
                agent_path = %agent_path,
                roles = ?resolved.effective_roles,
                permissions = resolved.permissions.len(),
                "Agent executing under its own permissions"
            );

            // `user_id` is the agent's own marker, so an RLS condition
            // (`node.created_by == auth.user_id`) and the authorship stamp agree
            // on who this is.
            Ok(Some(
                AuthContext::for_user(marker)
                    .with_permissions(resolved)
                    .with_agent(marker.to_string()),
            ))
        }
    }
}

/// The context an anonymous visitor's tool calls run with: the agent's
/// anonymous tool grant, resolved like a user's roles. An agent that grants
/// nothing FAILS CLOSED — its tools do not run — instead of falling back to a
/// wider context.
async fn resolve_visitor_tool_context<S>(
    storage: &Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    agent: &Node,
    agent_path: &str,
    marker: &str,
) -> Result<AuthContext, String>
where
    S: Storage + 'static,
{
    let (roles, groups) = anonymous_tool_grant(agent);
    if roles.is_empty() && groups.is_empty() {
        return Err(format!(
            "agent '{}' answers anonymous visitors but grants their tools no roles \
             (set anonymous.tool_roles on the agent)",
            agent_path
        ));
    }
    let mut principal = agent.clone();
    principal.properties.insert(
        "roles".to_string(),
        PropertyValue::Array(roles.into_iter().map(PropertyValue::String).collect()),
    );
    principal.properties.insert(
        "groups".to_string(),
        PropertyValue::Array(groups.into_iter().map(PropertyValue::String).collect()),
    );
    let resolved = PermissionService::new(storage.clone())
        .resolve_for_principal_node(tenant_id, repo_id, branch, &principal)
        .await
        .map_err(|error| {
            format!(
                "Failed to resolve anonymous tool rights for '{}': {}",
                agent_path, error
            )
        })?;
    if resolved.is_system_admin {
        return Err(format!(
            "agent '{}' grants anonymous visitors' tools system_admin; refused",
            agent_path
        ));
    }
    tracing::info!(
        agent_path = %agent_path,
        roles = ?resolved.effective_roles,
        permissions = resolved.permissions.len(),
        "Agent tools executing for an anonymous visitor under the anonymous tool grant"
    );
    Ok(AuthContext::for_user(marker)
        .with_permissions(resolved)
        .with_agent(marker.to_string()))
}

/// Resolve `CallerRights`: run as whoever's write caused this execution.
async fn resolve_caller_context<S>(
    storage: &Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    agent_path: &str,
    marker: &str,
    triggering_user_id: Option<&str>,
) -> Result<Option<AuthContext>, String>
where
    S: Storage + 'static,
{
    let Some(user_id) =
        triggering_user_id.filter(|id| !id.is_empty() && *id != "anonymous" && *id != "system")
    else {
        tracing::warn!(
            agent_path = %agent_path,
            "Agent execution_context is \"user\", but no triggering identity is \
             available on this call path (e.g. a timer trigger or an API-started \
             flow); falling back to the caller's existing context"
        );
        return Ok(None);
    };

    // `created_by`/`updated_by` and a `NodeEvent`'s `actor` field are all
    // `AuthContext::actor_id()` — the identity_id from the JWT `sub` claim —
    // not a `raisin:User` node's own UUID. `resolve_for_identity_id` is the
    // resolver keyed on that; `resolve_for_user_id` (node UUID) would look up
    // the wrong thing entirely and silently find nothing.
    let resolved = PermissionService::new(storage.clone())
        .resolve_for_identity_id(tenant_id, repo_id, branch, user_id)
        .await
        .map_err(|error| {
            format!(
                "Failed to resolve permissions for triggering user '{}': {}",
                user_id, error
            )
        })?;

    let Some(resolved) = resolved else {
        tracing::warn!(
            agent_path = %agent_path,
            user_id = %user_id,
            "Triggering identity is not a resolvable raisin:User; falling back \
             to the caller's existing context"
        );
        return Ok(None);
    };

    tracing::info!(
        agent_path = %agent_path,
        user_id = %user_id,
        roles = ?resolved.effective_roles,
        permissions = resolved.permissions.len(),
        "Agent executing under the triggering user's permissions"
    );

    Ok(Some(
        AuthContext::for_user(user_id)
            .with_permissions(resolved)
            .with_agent(marker.to_string()),
    ))
}

#[cfg(test)]
mod visitor_tests {
    use super::*;
    use std::collections::HashMap;

    fn agent(props: serde_json::Value) -> Node {
        let properties: HashMap<String, PropertyValue> = props
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), PropertyValue::from_json(v)))
            .collect();
        Node {
            path: "/agents/site".into(),
            node_type: "raisin:AIAgent".into(),
            properties,
            ..Default::default()
        }
    }

    #[test]
    fn visitor_identities_are_recognised() {
        assert!(is_visitor_identity(Some("visitor:0123456789abcdef")));
        assert!(!is_visitor_identity(Some("user-1")));
        assert!(!is_visitor_identity(None));
    }

    #[test]
    fn the_anonymous_block_wins_over_the_agents_own_roles() {
        let a = agent(serde_json::json!({
            "roles": ["editor"],
            "anonymous": { "enabled": true, "tool_roles": ["site_reader"] }
        }));
        assert_eq!(
            anonymous_tool_grant(&a),
            (vec!["site_reader".to_string()], vec![])
        );
    }

    #[test]
    fn without_an_anonymous_block_the_agents_roles_apply() {
        let a = agent(serde_json::json!({ "roles": ["site_reader"], "groups": ["g"] }));
        assert_eq!(
            anonymous_tool_grant(&a),
            (vec!["site_reader".to_string()], vec!["g".to_string()])
        );
    }

    #[test]
    fn an_explicitly_empty_grant_stays_empty() {
        let a = agent(serde_json::json!({
            "roles": ["editor"],
            "anonymous": { "tool_roles": [] }
        }));
        assert_eq!(anonymous_tool_grant(&a), (vec![], vec![]));
    }
}
