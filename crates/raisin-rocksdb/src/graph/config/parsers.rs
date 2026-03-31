//! Node property parsing helpers for graph algorithm configuration.

use super::super::types::{GraphScope, GraphTarget, RefreshConfig, TargetMode};
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;

/// Helper to get a string from PropertyValue
pub(super) fn get_string(v: &PropertyValue) -> Option<String> {
    match v {
        PropertyValue::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Helper to get a bool from PropertyValue
pub(super) fn get_bool(v: &PropertyValue) -> Option<bool> {
    match v {
        PropertyValue::Boolean(b) => Some(*b),
        _ => None,
    }
}

/// Helper to get an i64 from PropertyValue
fn get_i64(v: &PropertyValue) -> Option<i64> {
    match v {
        PropertyValue::Integer(i) => Some(*i),
        _ => None,
    }
}

/// Helper to get a string array from PropertyValue
fn get_string_array(v: &PropertyValue) -> Option<Vec<String>> {
    match v {
        PropertyValue::Array(arr) => Some(arr.iter().filter_map(get_string).collect()),
        _ => None,
    }
}

/// Parse target configuration from node properties
pub(super) fn parse_target(node: &Node) -> Result<GraphTarget> {
    let target_obj = node.properties.get("target").ok_or_else(|| {
        Error::Validation("GraphAlgorithmConfig missing 'target' property".to_string())
    })?;

    let target_map = match target_obj {
        PropertyValue::Object(map) => map,
        _ => return Err(Error::Validation("'target' must be an object".to_string())),
    };

    let mode_str = target_map
        .get("mode")
        .and_then(get_string)
        .ok_or_else(|| Error::Validation("'target.mode' is required".to_string()))?;

    let mode = match mode_str.as_str() {
        "branch" => TargetMode::Branch,
        "all_branches" => TargetMode::AllBranches,
        "revision" => TargetMode::Revision,
        "branch_pattern" => TargetMode::BranchPattern,
        _ => {
            return Err(Error::Validation(format!(
                "Invalid target mode: {}",
                mode_str
            )))
        }
    };

    let branches = target_map
        .get("branches")
        .and_then(get_string_array)
        .unwrap_or_default();

    let revisions = target_map
        .get("revisions")
        .and_then(get_string_array)
        .unwrap_or_default();

    let branch_pattern = target_map.get("branch_pattern").and_then(get_string);

    Ok(GraphTarget {
        mode,
        branches,
        revisions,
        branch_pattern,
    })
}

/// Parse scope configuration from node properties
pub(super) fn parse_scope(node: &Node) -> Result<GraphScope> {
    let scope_obj = node.properties.get("scope").ok_or_else(|| {
        Error::Validation("GraphAlgorithmConfig missing 'scope' property".to_string())
    })?;

    let scope_map = match scope_obj {
        PropertyValue::Object(map) => map,
        _ => return Err(Error::Validation("'scope' must be an object".to_string())),
    };

    let paths = scope_map
        .get("paths")
        .and_then(get_string_array)
        .unwrap_or_default();

    let node_types = scope_map
        .get("node_types")
        .and_then(get_string_array)
        .unwrap_or_default();

    let workspaces = scope_map
        .get("workspaces")
        .and_then(get_string_array)
        .unwrap_or_default();

    let relation_types = scope_map
        .get("relation_types")
        .and_then(get_string_array)
        .unwrap_or_default();

    Ok(GraphScope {
        paths,
        node_types,
        workspaces,
        relation_types,
    })
}

/// Parse refresh configuration from node properties
pub(super) fn parse_refresh(node: &Node) -> Result<RefreshConfig> {
    let refresh_obj = match node.properties.get("refresh") {
        Some(v) => v,
        None => return Ok(RefreshConfig::default()),
    };

    let refresh_map = match refresh_obj {
        PropertyValue::Object(map) => map,
        _ => return Ok(RefreshConfig::default()),
    };

    let ttl_seconds = refresh_map
        .get("ttl_seconds")
        .and_then(get_i64)
        .map(|i| i as u64)
        .unwrap_or(0);

    let on_branch_change = refresh_map
        .get("on_branch_change")
        .and_then(get_bool)
        .unwrap_or(false);

    let on_relation_change = refresh_map
        .get("on_relation_change")
        .and_then(get_bool)
        .unwrap_or(false);

    let cron = refresh_map.get("cron").and_then(get_string);

    Ok(RefreshConfig {
        ttl_seconds,
        on_branch_change,
        on_relation_change,
        cron,
    })
}

/// Convert PropertyValue to serde_json::Value
pub(super) fn property_value_to_json(value: &PropertyValue) -> Option<serde_json::Value> {
    match value {
        PropertyValue::String(s) => Some(serde_json::Value::String(s.clone())),
        PropertyValue::Integer(i) => Some(serde_json::Value::Number((*i).into())),
        PropertyValue::Float(f) => serde_json::Number::from_f64(*f).map(serde_json::Value::Number),
        PropertyValue::Boolean(b) => Some(serde_json::Value::Bool(*b)),
        PropertyValue::Null => Some(serde_json::Value::Null),
        PropertyValue::Array(arr) => {
            let json_arr: Vec<_> = arr.iter().filter_map(property_value_to_json).collect();
            Some(serde_json::Value::Array(json_arr))
        }
        PropertyValue::Object(map) => {
            let json_obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .filter_map(|(k, v)| property_value_to_json(v).map(|json| (k.clone(), json)))
                .collect();
            Some(serde_json::Value::Object(json_obj))
        }
        _ => None,
    }
}

/// Simple glob matching for branch patterns
pub(super) fn glob_match(pattern: &str, text: &str) -> bool {
    let glob = glob::Pattern::new(pattern);
    match glob {
        Ok(p) => p.matches(text),
        Err(_) => false,
    }
}
