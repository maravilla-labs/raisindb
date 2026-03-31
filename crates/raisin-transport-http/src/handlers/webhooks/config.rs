// SPDX-License-Identifier: BSL-1.1

//! HTTP trigger configuration parsing and path parameter extraction.

use std::collections::HashMap;

use raisin_functions::{HttpMethod, HttpRouteMode, HttpTriggerConfig};
use raisin_models::nodes::Node;

use crate::error::ApiError;

use super::helpers::property_as_string;

/// Parse HTTP trigger configuration from node properties
pub(super) fn parse_http_config(node: &Node) -> Result<HttpTriggerConfig, ApiError> {
    let config = node
        .properties
        .get("config")
        .ok_or_else(|| ApiError::validation_failed("Trigger has no config"))?;

    // Convert to JSON value for flexible parsing
    let config_value = serde_json::to_value(config)
        .map_err(|e| ApiError::internal(format!("Failed to serialize config: {}", e)))?;

    // Try strict deserialization first
    if let Ok(http_config) = serde_json::from_value::<HttpTriggerConfig>(config_value.clone()) {
        return Ok(http_config);
    }

    // Fall back to manual parsing for frontend-stored format
    let config_obj = config_value
        .as_object()
        .ok_or_else(|| ApiError::validation_failed("Config is not an object"))?;

    // Parse methods - handle both Vec<HttpMethod> and Vec<String> formats
    let methods = if let Some(methods_val) = config_obj.get("methods") {
        if let Some(methods_arr) = methods_val.as_array() {
            methods_arr
                .iter()
                .filter_map(|v| {
                    v.as_str().and_then(|s| match s.to_uppercase().as_str() {
                        "GET" => Some(HttpMethod::GET),
                        "POST" => Some(HttpMethod::POST),
                        "PUT" => Some(HttpMethod::PUT),
                        "PATCH" => Some(HttpMethod::PATCH),
                        "DELETE" => Some(HttpMethod::DELETE),
                        _ => None,
                    })
                })
                .collect()
        } else {
            vec![HttpMethod::POST] // Default
        }
    } else {
        vec![HttpMethod::POST] // Default
    };

    // Parse optional fields
    let route_mode = config_obj
        .get("route_mode")
        .and_then(|v| v.as_str())
        .map(|s| match s {
            "script" => HttpRouteMode::Script,
            _ => HttpRouteMode::Config,
        })
        .unwrap_or_default();

    let path_pattern = config_obj
        .get("path_pattern")
        .and_then(|v| v.as_str())
        .map(String::from);

    let path_suffix = config_obj
        .get("path_suffix")
        .and_then(|v| v.as_str())
        .map(String::from);

    let default_sync = config_obj
        .get("default_sync")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    Ok(HttpTriggerConfig {
        methods,
        route_mode,
        path_pattern,
        path_suffix,
        default_sync,
    })
}

/// Parse path parameters using matchit
pub(super) fn parse_path_params(
    pattern: &str,
    actual_path: &str,
) -> Result<HashMap<String, String>, ApiError> {
    let mut router = matchit::Router::new();

    // Ensure pattern starts with /
    let normalized_pattern = if pattern.starts_with('/') {
        pattern.to_string()
    } else {
        format!("/{}", pattern)
    };

    router
        .insert(&normalized_pattern, ())
        .map_err(|e| ApiError::internal(format!("Invalid path pattern '{}': {}", pattern, e)))?;

    // Ensure path starts with /
    let normalized_path = if actual_path.starts_with('/') {
        actual_path.to_string()
    } else {
        format!("/{}", actual_path)
    };

    // Handle empty path
    let path_to_match = if normalized_path == "/" && normalized_pattern != "/" {
        return Ok(HashMap::new());
    } else {
        normalized_path
    };

    match router.at(&path_to_match) {
        Ok(matched) => {
            let mut params = HashMap::new();
            for (key, value) in matched.params.iter() {
                params.insert(key.to_string(), value.to_string());
            }
            Ok(params)
        }
        Err(_) => {
            // Path doesn't match pattern - return empty params (script can handle it)
            Ok(HashMap::new())
        }
    }
}

/// Find the target function or flow from trigger node
pub(super) fn find_trigger_target(
    trigger_node: &Node,
) -> Result<(String, Option<serde_json::Value>), ApiError> {
    // Check for function_flow first (preferred)
    if let Some(flow) = trigger_node.properties.get("function_flow") {
        let flow_value = serde_json::to_value(flow)
            .map_err(|e| ApiError::internal(format!("Failed to serialize flow: {}", e)))?;
        // For flow, we don't have a single function path
        return Ok((String::new(), Some(flow_value)));
    }

    // Fall back to function_path
    if let Some(path) = property_as_string(trigger_node.properties.get("function_path")) {
        return Ok((path, None));
    }

    Err(ApiError::validation_failed(
        "Trigger has no function_path or function_flow configured",
    ))
}
