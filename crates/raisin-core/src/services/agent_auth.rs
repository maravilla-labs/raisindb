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
    /// Run with full system privileges (`"system"`, and the legacy `"user"`
    /// default, whose per-user resolution has never been implemented).
    System,
    /// Run under the agent's OWN roles and groups.
    OwnRights,
}

/// Read `execution_context` off an agent node.
pub fn execution_of(agent: &Node) -> AgentExecution {
    match agent.properties.get("execution_context") {
        Some(PropertyValue::String(value)) if value == "agent" => AgentExecution::OwnRights,
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

/// The auth context a write BY THIS AGENT should carry.
///
/// * `Ok(Some(ctx))` — the agent runs under its own resolved permissions.
/// * `Ok(None)` — the agent did not ask for its own rights, or asked but was
///   granted nothing; the caller keeps whatever system context it would have
///   used. An agent that may do NOTHING is a silent, baffling failure, so an
///   empty grant list is treated as "not configured" rather than as a lockout.
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
        return Ok(None);
    };

    if execution_of(&agent) != AgentExecution::OwnRights {
        return Ok(None);
    }

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
    // (`node.created_by == auth.user_id`) and the authorship stamp agree on who
    // this is.
    Ok(Some(
        AuthContext::for_user(marker)
            .with_permissions(resolved)
            .with_agent(marker.to_string()),
    ))
}
