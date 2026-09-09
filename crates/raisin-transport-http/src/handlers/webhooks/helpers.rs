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

/// Parse the request's raw query string into the map a trigger sees as
/// `input.http.query`.
///
/// This used to take the `HeaderMap` and return an empty map unconditionally,
/// which is why a trigger fired by `?x=1` observed `query: {}` with no error
/// anywhere. Query parameters are not in the headers; they are in the URI, so
/// the raw string is threaded down from the handler.
///
/// Repeated keys keep the LAST value, matching how `Query<T>` deserializes and
/// how the same map is built elsewhere. `+` decodes to a space and percent
/// escapes are decoded, both per `application/x-www-form-urlencoded`.
pub(super) fn parse_query_params(raw_query: Option<&str>) -> HashMap<String, String> {
    let Some(raw) = raw_query else {
        return HashMap::new();
    };

    url::form_urlencoded::parse(raw.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_string_reaches_the_trigger() {
        // Regression: this returned an empty map for every request, so a
        // trigger fired by `?x=1` saw `query: {}`.
        let params = parse_query_params(Some("x=1&name=hello%20world&flag"));
        assert_eq!(params.get("x").map(String::as_str), Some("1"));
        assert_eq!(params.get("name").map(String::as_str), Some("hello world"));
        assert_eq!(params.get("flag").map(String::as_str), Some(""));
    }

    #[test]
    fn no_query_string_is_an_empty_map() {
        assert!(parse_query_params(None).is_empty());
        assert!(parse_query_params(Some("")).is_empty());
    }

    #[test]
    fn a_repeated_key_keeps_the_last_value() {
        let params = parse_query_params(Some("a=1&a=2"));
        assert_eq!(params.get("a").map(String::as_str), Some("2"));
    }

    #[test]
    fn plus_decodes_to_a_space() {
        let params = parse_query_params(Some("q=two+words"));
        assert_eq!(params.get("q").map(String::as_str), Some("two words"));
    }
}

/// Get bool from header
pub(super) fn header_as_bool(headers: &HeaderMap, name: &str) -> Option<bool> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("true") || s == "1")
}
