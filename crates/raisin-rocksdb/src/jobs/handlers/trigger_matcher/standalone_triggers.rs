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

//! Standalone trigger processing for raisin:Trigger nodes
//!
//! Processes standalone raisin:Trigger nodes that reference
//! function paths or workflow data.

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use std::sync::Arc;

use super::inline_triggers::{check_trigger_filters, InlineTriggerContext};
use crate::jobs::handlers::{FilterCheckResult, TriggerEvaluationResult, TriggerMatch};

/// Process standalone raisin:Trigger nodes
///
/// Each trigger node has its own `trigger_type`, `config`, `filters`, and
/// references to either a `function_path` or `function_flow` to invoke.
pub(super) async fn process_standalone_triggers<S: Storage + 'static>(
    storage: &Arc<S>,
    standalone_triggers: Vec<Node>,
    ctx: &InlineTriggerContext<'_>,
    matches: &mut Vec<TriggerMatch>,
    all_results: &mut Vec<TriggerEvaluationResult>,
) -> raisin_error::Result<()> {
    for trigger_node in standalone_triggers {
        process_single_standalone_trigger(storage, &trigger_node, ctx, matches, all_results)
            .await?;
    }

    Ok(())
}

/// Process a single standalone raisin:Trigger node
async fn process_single_standalone_trigger<S: Storage + 'static>(
    storage: &Arc<S>,
    trigger_node: &Node,
    ctx: &InlineTriggerContext<'_>,
    matches: &mut Vec<TriggerMatch>,
    all_results: &mut Vec<TriggerEvaluationResult>,
) -> raisin_error::Result<()> {
    let mut filter_checks = Vec::new();

    // Check if trigger is enabled
    let enabled = trigger_node
        .properties
        .get("enabled")
        .and_then(|v| match v {
            PropertyValue::Boolean(b) => Some(*b),
            _ => None,
        })
        .unwrap_or(true);

    filter_checks.push(FilterCheckResult {
        filter_name: "trigger_enabled".to_string(),
        passed: enabled,
        expected: Some(serde_json::json!(true)),
        actual: Some(serde_json::json!(enabled)),
        reason: if enabled {
            "Trigger is enabled".to_string()
        } else {
            "Trigger is disabled".to_string()
        },
    });

    let trigger_name = trigger_node
        .properties
        .get("name")
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| trigger_node.name.clone());

    if !enabled {
        all_results.push(TriggerEvaluationResult {
            trigger_path: trigger_node.path.clone(),
            trigger_name,
            matched: false,
            filter_checks,
            enqueued_job_id: None,
        });
        return Ok(());
    }

    // Check trigger type
    let trigger_type_str = trigger_node
        .properties
        .get("trigger_type")
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.as_str()),
            _ => None,
        });

    let is_node_event =
        trigger_type_str == Some("node_event") || trigger_type_str == Some("NodeEvent");

    filter_checks.push(FilterCheckResult {
        filter_name: "trigger_type".to_string(),
        passed: is_node_event,
        expected: Some(serde_json::json!("node_event")),
        actual: Some(serde_json::json!(trigger_type_str)),
        reason: if is_node_event {
            "Trigger type is node_event".to_string()
        } else {
            format!(
                "Trigger type {:?} is not node_event",
                trigger_type_str.unwrap_or("none")
            )
        },
    });

    if !is_node_event {
        all_results.push(TriggerEvaluationResult {
            trigger_path: trigger_node.path.clone(),
            trigger_name,
            matched: false,
            filter_checks,
            enqueued_job_id: None,
        });
        return Ok(());
    }

    // Get config for event_kinds check
    let config_json = trigger_node
        .properties
        .get("config")
        .and_then(|v| serde_json::to_value(v).ok())
        .unwrap_or_default();

    // Check event_kinds
    if !check_standalone_event_kinds(&config_json, ctx.event_type, &mut filter_checks) {
        all_results.push(TriggerEvaluationResult {
            trigger_path: trigger_node.path.clone(),
            trigger_name,
            matched: false,
            filter_checks,
            enqueued_job_id: None,
        });
        return Ok(());
    }

    // Get filters and evaluate
    let filters_json = trigger_node
        .properties
        .get("filters")
        .and_then(|v| serde_json::to_value(v).ok())
        .unwrap_or_default();

    let all_filters_pass =
        check_trigger_filters(storage, Some(&filters_json), ctx, &mut filter_checks).await?;

    all_results.push(TriggerEvaluationResult {
        trigger_path: trigger_node.path.clone(),
        trigger_name: trigger_name.clone(),
        matched: all_filters_pass,
        filter_checks,
        enqueued_job_id: None,
    });

    if all_filters_pass {
        build_trigger_match(storage, trigger_node, &trigger_name, ctx, matches).await?;
    }

    Ok(())
}

/// Check event_kinds for a standalone trigger node
///
/// Returns `true` if the event matches (or no filter exists).
fn check_standalone_event_kinds(
    config_json: &serde_json::Value,
    event_type: &str,
    filter_checks: &mut Vec<FilterCheckResult>,
) -> bool {
    let event_kinds = config_json.get("event_kinds").and_then(|v| v.as_array());
    let matches_event = if let Some(kinds) = event_kinds {
        kinds.iter().any(|k| {
            k.as_str()
                .map(|s| s.eq_ignore_ascii_case(event_type))
                .unwrap_or(false)
        })
    } else {
        true
    };

    filter_checks.push(FilterCheckResult {
        filter_name: "event_kinds".to_string(),
        passed: matches_event,
        expected: Some(serde_json::json!(event_kinds)),
        actual: Some(serde_json::json!(event_type)),
        reason: if matches_event {
            format!("{} matches event_kinds filter", event_type)
        } else {
            format!(
                "{} does not match event_kinds {:?}",
                event_type,
                event_kinds.map(|k| k.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            )
        },
    });

    matches_event
}

/// Build a TriggerMatch from a matched standalone trigger node
///
/// Resolves the function_flow reference to get workflow_data and
/// extracts function_path, priority, and max_retries.
async fn build_trigger_match<S: Storage + 'static>(
    storage: &Arc<S>,
    trigger_node: &Node,
    trigger_name: &str,
    ctx: &InlineTriggerContext<'_>,
    matches: &mut Vec<TriggerMatch>,
) -> raisin_error::Result<()> {
    let priority = trigger_node
        .properties
        .get("priority")
        .and_then(|v| match v {
            PropertyValue::Integer(i) => Some(*i as i32),
            _ => None,
        })
        .unwrap_or(0);

    // Resolve function_flow reference to get workflow_data
    let workflow_data = match trigger_node.properties.get("function_flow") {
        Some(PropertyValue::Reference(ref_val)) => {
            let flow_node = storage
                .nodes()
                .get(
                    StorageScope::new(ctx.tenant_id, ctx.repo_id, ctx.branch, &ref_val.workspace),
                    &ref_val.id,
                    None,
                )
                .await?;

            flow_node.and_then(|node| {
                node.properties
                    .get("workflow_data")
                    .and_then(|v| serde_json::to_value(v).ok())
            })
        }
        _ => None,
    };

    // Check for function_path
    let function_path = trigger_node
        .properties
        .get("function_path")
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        });

    let max_retries = trigger_node
        .properties
        .get("max_retries")
        .and_then(|v| match v {
            PropertyValue::Integer(i) => Some(*i as u32),
            _ => None,
        });

    if workflow_data.is_some() || function_path.is_some() {
        matches.push(TriggerMatch {
            function_path,
            trigger_name: trigger_name.to_string(),
            priority,
            trigger_path: Some(trigger_node.path.clone()),
            workflow_data,
            max_retries,
        });
    }

    Ok(())
}
