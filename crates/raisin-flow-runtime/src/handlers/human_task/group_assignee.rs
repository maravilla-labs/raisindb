// SPDX-License-Identifier: BSL-1.1

//! Native group assignment for human tasks.
//!
//! Groups are permission principals, not inbox folders. A group task therefore
//! resolves the group's current user members and creates one correlated task in
//! each member's real inbox. Completion aggregation is handled by `service`.

use crate::types::{FlowCallbacks, FlowError, FlowNode, FlowResult};
use serde_json::Value;
use std::collections::{HashSet, VecDeque};

use super::INBOX_WORKSPACE;

const USERS_ROOT: &str = "/users";
const MAX_DISCOVERED_NODES: usize = 20_000;

#[derive(Debug, Clone)]
pub(super) struct GroupAssignment {
    pub group_path: String,
    pub members: Vec<String>,
    pub completion: String,
    pub required: usize,
    pub response_condition: Option<String>,
}

fn node_str<'a>(node: &'a Value, key: &str) -> Option<&'a str> {
    node.get(key).and_then(Value::as_str)
}

fn property<'a>(node: &'a Value, key: &str) -> Option<&'a Value> {
    node.get("properties").and_then(|p| p.get(key))
}

fn membership_matches(value: &str, group_path: &str, group_id: &str) -> bool {
    let normalized = value.trim_matches('/');
    normalized == group_path.trim_matches('/')
        || normalized == group_id
        || normalized.strip_prefix("groups/") == Some(group_id)
}

/// Resolve `assignee` as a group, returning `None` for users and agents.
pub(super) async fn resolve_group_assignment(
    callbacks: &dyn FlowCallbacks,
    step: &FlowNode,
    assignee: &str,
) -> FlowResult<Option<GroupAssignment>> {
    let Some(group) = callbacks
        .get_node_in_workspace(INBOX_WORKSPACE, assignee)
        .await?
    else {
        return Ok(None);
    };
    if node_str(&group, "node_type") != Some("raisin:Group") {
        return Ok(None);
    }

    let group_path = node_str(&group, "path").unwrap_or(assignee).to_string();
    let group_id = property(&group, "group_id")
        .and_then(Value::as_str)
        .or_else(|| group_path.rsplit('/').next())
        .unwrap_or("")
        .to_string();
    if group_id.is_empty() {
        return Err(FlowError::InvalidNodeConfiguration(format!(
            "Human task step '{}' selected group '{}' without a group_id",
            step.id, assignee
        )));
    }

    let mut queue = VecDeque::from([USERS_ROOT.to_string()]);
    let mut visited = HashSet::new();
    let mut members = Vec::new();
    let mut discovered = 0usize;

    while let Some(parent) = queue.pop_front() {
        if !visited.insert(parent.clone()) {
            continue;
        }
        for child in callbacks
            .list_children_in_workspace(INBOX_WORKSPACE, &parent)
            .await?
        {
            discovered += 1;
            if discovered > MAX_DISCOVERED_NODES {
                return Err(FlowError::InvalidNodeConfiguration(format!(
                    "Human task step '{}' stopped resolving group '{}': user tree exceeds {} nodes",
                    step.id, group_path, MAX_DISCOVERED_NODES
                )));
            }
            let Some(path) = node_str(&child, "path").map(String::from) else {
                continue;
            };
            if node_str(&child, "node_type") == Some("raisin:User") {
                let belongs = property(&child, "groups")
                    .and_then(Value::as_array)
                    .map(|groups| {
                        groups
                            .iter()
                            .filter_map(Value::as_str)
                            .any(|value| membership_matches(value, &group_path, &group_id))
                    })
                    .unwrap_or(false);
                if belongs {
                    members.push(path);
                }
            } else {
                queue.push_back(path);
            }
        }
    }

    members.sort();
    members.dedup();
    if members.is_empty() {
        return Err(FlowError::InvalidNodeConfiguration(format!(
            "Human task step '{}' selected group '{}' with no users",
            step.id, group_path
        )));
    }

    let completion = step
        .get_string("group_completion")
        .unwrap_or_else(|| "any".to_string());
    if !matches!(completion.as_str(), "any" | "all" | "quorum") {
        return Err(FlowError::InvalidNodeConfiguration(format!(
            "Human task step '{}' has invalid group_completion '{}': expected any, all, or quorum",
            step.id, completion
        )));
    }
    let required = match completion.as_str() {
        "all" => members.len(),
        "quorum" => step
            .get_property("group_quorum")
            .and_then(Value::as_u64)
            .unwrap_or(1) as usize,
        _ => 1,
    };
    if required == 0 || required > members.len() {
        return Err(FlowError::InvalidNodeConfiguration(format!(
            "Human task step '{}' requires {} responses from a group with {} users",
            step.id,
            required,
            members.len()
        )));
    }

    Ok(Some(GroupAssignment {
        group_path,
        members,
        completion,
        required,
        response_condition: step.get_string("response_condition"),
    }))
}
