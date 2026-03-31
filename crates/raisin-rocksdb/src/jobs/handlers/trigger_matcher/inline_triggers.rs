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

//! Inline trigger processing for raisin:Function nodes
//!
//! Processes triggers embedded within the `triggers` property array
//! of raisin:Function nodes.

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use std::sync::Arc;

use super::filters::{get_nested_property, glob_match, property_filter_matches};
use crate::jobs::handlers::{FilterCheckResult, TriggerEvaluationResult, TriggerMatch};

/// Context for evaluating inline triggers
pub(super) struct InlineTriggerContext<'a> {
    pub event_type: &'a str,
    pub node_id: &'a str,
    pub node_type: &'a str,
    pub node_path: &'a str,
    pub tenant_id: &'a str,
    pub repo_id: &'a str,
    pub branch: &'a str,
    pub workspace: &'a str,
}

/// Process inline triggers on raisin:Function nodes
///
/// Iterates through function nodes, checks if they are enabled,
/// parses their `triggers` array, and evaluates each trigger against
/// the event context.
pub(super) async fn process_inline_triggers<S: Storage + 'static>(
    storage: &Arc<S>,
    functions: Vec<Node>,
    ctx: &InlineTriggerContext<'_>,
    matches: &mut Vec<TriggerMatch>,
    all_results: &mut Vec<TriggerEvaluationResult>,
) -> raisin_error::Result<()> {
    for func in functions {
        let mut filter_checks = Vec::new();

        // Check if function is enabled
        let enabled = func
            .properties
            .get("enabled")
            .and_then(|v| match v {
                PropertyValue::Boolean(b) => Some(*b),
                _ => None,
            })
            .unwrap_or(true);

        filter_checks.push(FilterCheckResult {
            filter_name: "function_enabled".to_string(),
            passed: enabled,
            expected: Some(serde_json::json!(true)),
            actual: Some(serde_json::json!(enabled)),
            reason: if enabled {
                "Function is enabled".to_string()
            } else {
                "Function is disabled".to_string()
            },
        });

        if !enabled {
            all_results.push(TriggerEvaluationResult {
                trigger_path: func.path.clone(),
                trigger_name: "inline".to_string(),
                matched: false,
                filter_checks,
                enqueued_job_id: None,
            });
            continue;
        }

        // Get triggers from function properties
        let triggers_value = match func.properties.get("triggers") {
            Some(v) => v,
            None => continue,
        };

        // Convert PropertyValue to serde_json::Value for parsing
        let triggers_json = match serde_json::to_value(triggers_value) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Parse triggers array
        let triggers = match triggers_json.as_array() {
            Some(arr) => arr,
            None => continue,
        };

        for trigger in triggers {
            process_single_inline_trigger(
                storage,
                &func,
                trigger,
                &filter_checks,
                ctx,
                matches,
                all_results,
            )
            .await?;
        }
    }

    Ok(())
}

/// Process a single inline trigger definition from a function node
async fn process_single_inline_trigger<S: Storage + 'static>(
    storage: &Arc<S>,
    func: &Node,
    trigger: &serde_json::Value,
    base_filter_checks: &[FilterCheckResult],
    ctx: &InlineTriggerContext<'_>,
    matches: &mut Vec<TriggerMatch>,
    all_results: &mut Vec<TriggerEvaluationResult>,
) -> raisin_error::Result<()> {
    let mut trigger_filter_checks = base_filter_checks.to_vec();

    // Check if trigger is enabled
    let trigger_enabled = trigger
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    trigger_filter_checks.push(FilterCheckResult {
        filter_name: "trigger_enabled".to_string(),
        passed: trigger_enabled,
        expected: Some(serde_json::json!(true)),
        actual: Some(serde_json::json!(trigger_enabled)),
        reason: if trigger_enabled {
            "Trigger is enabled".to_string()
        } else {
            "Trigger is disabled".to_string()
        },
    });

    let trigger_name = trigger
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();

    if !trigger_enabled {
        all_results.push(TriggerEvaluationResult {
            trigger_path: func.path.clone(),
            trigger_name,
            matched: false,
            filter_checks: trigger_filter_checks,
            enqueued_job_id: None,
        });
        return Ok(());
    }

    // Check trigger type
    let trigger_type = trigger
        .get("trigger_type")
        .and_then(|v| v.as_str())
        .or_else(|| trigger.get("type").and_then(|v| v.as_str()));

    let is_node_event = trigger_type == Some("node_event") || trigger_type == Some("NodeEvent");

    trigger_filter_checks.push(FilterCheckResult {
        filter_name: "trigger_type".to_string(),
        passed: is_node_event,
        expected: Some(serde_json::json!("node_event")),
        actual: Some(serde_json::json!(trigger_type)),
        reason: if is_node_event {
            "Trigger type is node_event".to_string()
        } else {
            format!(
                "Trigger type {:?} is not node_event",
                trigger_type.unwrap_or("none")
            )
        },
    });

    if !is_node_event {
        all_results.push(TriggerEvaluationResult {
            trigger_path: func.path.clone(),
            trigger_name,
            matched: false,
            filter_checks: trigger_filter_checks,
            enqueued_job_id: None,
        });
        return Ok(());
    }

    // Check event_kinds
    if !check_event_kinds(trigger, ctx.event_type, &mut trigger_filter_checks) {
        all_results.push(TriggerEvaluationResult {
            trigger_path: func.path.clone(),
            trigger_name,
            matched: false,
            filter_checks: trigger_filter_checks,
            enqueued_job_id: None,
        });
        return Ok(());
    }

    // Check filters
    let all_filters_pass = check_trigger_filters(
        storage,
        trigger.get("filters"),
        ctx,
        &mut trigger_filter_checks,
    )
    .await?;

    all_results.push(TriggerEvaluationResult {
        trigger_path: func.path.clone(),
        trigger_name: trigger_name.clone(),
        matched: all_filters_pass,
        filter_checks: trigger_filter_checks,
        enqueued_job_id: None,
    });

    if all_filters_pass {
        let priority = trigger
            .get("priority")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        let max_retries = trigger
            .get("max_retries")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);

        matches.push(TriggerMatch {
            function_path: Some(func.path.clone()),
            trigger_name,
            priority,
            trigger_path: None,
            workflow_data: None,
            max_retries,
        });
    }

    Ok(())
}

/// Check event_kinds filter on a trigger JSON value
///
/// Returns `true` if the event matches (or no event_kinds filter exists).
fn check_event_kinds(
    trigger: &serde_json::Value,
    event_type: &str,
    filter_checks: &mut Vec<FilterCheckResult>,
) -> bool {
    let event_kinds = trigger
        .get("event_kinds")
        .or_else(|| trigger.get("config").and_then(|c| c.get("event_kinds")))
        .and_then(|v| v.as_array());

    let matches_event = if let Some(kinds) = event_kinds {
        kinds.iter().any(|k| {
            k.as_str()
                .map(|s| s.eq_ignore_ascii_case(event_type))
                .unwrap_or(false)
        })
    } else {
        true // No filter means all events match
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

/// Check all trigger filters (node_types, paths, workspaces, property_filters)
///
/// Returns `true` if all filters pass.
pub(super) async fn check_trigger_filters<S: Storage + 'static>(
    storage: &Arc<S>,
    filters: Option<&serde_json::Value>,
    ctx: &InlineTriggerContext<'_>,
    filter_checks: &mut Vec<FilterCheckResult>,
) -> raisin_error::Result<bool> {
    let f = match filters {
        Some(f) => f,
        None => return Ok(true),
    };

    let mut all_filters_pass = true;

    // Check node_types filter
    if let Some(node_types) = f.get("node_types").and_then(|v| v.as_array()) {
        let matches_type = node_types
            .iter()
            .any(|t| t.as_str().map(|s| s == ctx.node_type).unwrap_or(false));

        filter_checks.push(FilterCheckResult {
            filter_name: "node_types".to_string(),
            passed: matches_type,
            expected: Some(serde_json::json!(node_types)),
            actual: Some(serde_json::json!(ctx.node_type)),
            reason: if matches_type {
                format!("{} matches node_types filter", ctx.node_type)
            } else {
                format!(
                    "{} does not match node_types {:?}",
                    ctx.node_type,
                    node_types
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                )
            },
        });

        if !matches_type {
            all_filters_pass = false;
        }
    }

    // Check paths filter
    if all_filters_pass {
        if let Some(paths) = f.get("paths").and_then(|v| v.as_array()) {
            let matches_path = paths.iter().any(|p| {
                p.as_str()
                    .map(|pattern| glob_match(pattern, ctx.node_path))
                    .unwrap_or(false)
            });

            filter_checks.push(FilterCheckResult {
                filter_name: "paths".to_string(),
                passed: matches_path,
                expected: Some(serde_json::json!(paths)),
                actual: Some(serde_json::json!(ctx.node_path)),
                reason: if matches_path {
                    format!("{} matches paths filter", ctx.node_path)
                } else {
                    format!(
                        "{} does not match paths {:?}",
                        ctx.node_path,
                        paths.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()
                    )
                },
            });

            if !matches_path {
                all_filters_pass = false;
            }
        }
    }

    // Check workspaces filter
    if all_filters_pass {
        if let Some(workspaces) = f.get("workspaces").and_then(|v| v.as_array()) {
            let matches_workspace = workspaces.iter().any(|w| {
                w.as_str()
                    .map(|s| s == ctx.workspace || s == "*")
                    .unwrap_or(false)
            });

            filter_checks.push(FilterCheckResult {
                filter_name: "workspaces".to_string(),
                passed: matches_workspace,
                expected: Some(serde_json::json!(workspaces)),
                actual: Some(serde_json::json!(ctx.workspace)),
                reason: if matches_workspace {
                    format!("{} matches workspaces filter", ctx.workspace)
                } else {
                    format!(
                        "{} does not match workspaces {:?}",
                        ctx.workspace,
                        workspaces
                            .iter()
                            .filter_map(|v| v.as_str())
                            .collect::<Vec<_>>()
                    )
                },
            });

            if !matches_workspace {
                all_filters_pass = false;
            }
        }
    }

    // Check property_filters
    if all_filters_pass {
        if let Some(prop_filters) = f.get("property_filters").and_then(|v| v.as_object()) {
            let node_opt = storage
                .nodes()
                .get(
                    StorageScope::new(ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace),
                    ctx.node_id,
                    None,
                )
                .await?;

            match node_opt {
                Some(node) => {
                    for (key, expected_value) in prop_filters {
                        let prop_matches =
                            property_filter_matches(&node.properties, key, expected_value);

                        let actual_value = get_nested_property(&node.properties, key)
                            .and_then(|v| serde_json::to_value(v).ok());

                        filter_checks.push(FilterCheckResult {
                            filter_name: format!("property_filter:{}", key),
                            passed: prop_matches,
                            expected: Some(expected_value.clone()),
                            actual: actual_value.clone(),
                            reason: if prop_matches {
                                format!("Property {} matches filter", key)
                            } else {
                                format!(
                                    "Property {} does not match: expected {:?}, got {:?}",
                                    key, expected_value, actual_value
                                )
                            },
                        });

                        if !prop_matches {
                            all_filters_pass = false;
                        }
                    }
                }
                None => {
                    filter_checks.push(FilterCheckResult {
                        filter_name: "property_filters".to_string(),
                        passed: false,
                        expected: Some(serde_json::json!(prop_filters)),
                        actual: None,
                        reason: "Node not found for property filter check".to_string(),
                    });
                    all_filters_pass = false;
                }
            }
        }
    }

    Ok(all_filters_pass)
}
