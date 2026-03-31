// SPDX-License-Identifier: BSL-1.1

//\! Helper functions for property extraction and header conversion.

use std::collections::HashMap;

use axum::http::HeaderMap;
use raisin_models::nodes::properties::PropertyValue;

/// Extract string from PropertyValue
pub(super) fn property_as_string(prop: Option<&PropertyValue>) -> Option<String> {
    match prop {
        Some(PropertyValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Extract bool from PropertyValue
pub(super) fn property_as_bool(prop: Option<&PropertyValue>) -> Option<bool> {
    match prop {
        Some(PropertyValue::Boolean(b)) => Some(*b),
        _ => None,
    }
}

/// Convert headers to HashMap
pub(super) fn headers_to_map(headers: &HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.to_string(), v.to_string()))
        })
        .collect()
}

/// Extract query params from URL (if present in headers as referer or similar)
pub(super) fn extract_query_params(_headers: &HeaderMap) -> HashMap<String, String> {
    // Query params come from Axum Query extractor, not headers
    // This is a placeholder - actual query params are passed separately
    HashMap::new()
}

/// Get bool from header
pub(super) fn header_as_bool(headers: &HeaderMap, name: &str) -> Option<bool> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("true") || s == "1")
}
