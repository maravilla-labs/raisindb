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

//! Types and utility functions for AI tool call execution

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use std::collections::HashMap;
use std::sync::Arc;

/// Functions are always stored in the "functions" workspace
pub(super) const FUNCTIONS_WORKSPACE: &str = "functions";

/// Callback type for creating nodes through NodeService
///
/// This callback is provided by the transport layer which has access to NodeService.
/// Using NodeService ensures proper event publishing and trigger firing.
///
/// Arguments: (node, tenant_id, repo_id, branch, workspace)
/// Returns: Result<Node> - the created node
pub type NodeCreatorCallback = Arc<
    dyn Fn(
            Node,   // node to create
            String, // tenant_id
            String, // repo_id
            String, // branch
            String, // workspace
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Node>> + Send>>
        + Send
        + Sync,
>;

/// Convert PropertyValue to JSON Value
pub(super) fn property_value_to_json(pv: &PropertyValue) -> serde_json::Value {
    match pv {
        PropertyValue::Null => serde_json::Value::Null,
        PropertyValue::Boolean(b) => serde_json::Value::Bool(*b),
        PropertyValue::Integer(i) => serde_json::json!(i),
        PropertyValue::Float(f) => serde_json::json!(f),
        PropertyValue::Decimal(d) => serde_json::json!(d.to_string()),
        PropertyValue::String(s) => serde_json::Value::String(s.clone()),
        PropertyValue::Date(dt) => serde_json::Value::String(dt.to_string()),
        PropertyValue::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(property_value_to_json).collect())
        }
        PropertyValue::Object(obj) => {
            let map: serde_json::Map<String, serde_json::Value> = obj
                .iter()
                .map(|(k, v)| (k.clone(), property_value_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
        PropertyValue::Reference(r) => serde_json::json!({
            "raisin:ref": r.id,
            "raisin:path": r.path,
            "raisin:workspace": r.workspace
        }),
        PropertyValue::Url(u) => serde_json::json!({
            "raisin:url": u.url
        }),
        PropertyValue::Resource(r) => {
            // Resource is complex, just serialize as-is
            serde_json::to_value(r).unwrap_or(serde_json::Value::Null)
        }
        PropertyValue::Composite(c) => serde_json::to_value(c).unwrap_or(serde_json::Value::Null),
        PropertyValue::Element(e) => serde_json::to_value(e).unwrap_or(serde_json::Value::Null),
        PropertyValue::Vector(v) => serde_json::json!(v),
        PropertyValue::Geometry(g) => serde_json::to_value(g).unwrap_or(serde_json::Value::Null),
    }
}

/// Convert JSON Value to PropertyValue
pub(super) fn json_to_property_value(value: serde_json::Value) -> Result<PropertyValue> {
    match value {
        serde_json::Value::Null => Ok(PropertyValue::Null),
        serde_json::Value::Bool(b) => Ok(PropertyValue::Boolean(b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(PropertyValue::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(PropertyValue::Float(f))
            } else {
                Err(Error::Validation("Invalid number".to_string()))
            }
        }
        serde_json::Value::String(s) => Ok(PropertyValue::String(s)),
        serde_json::Value::Array(arr) => {
            let items: Result<Vec<_>> = arr.into_iter().map(json_to_property_value).collect();
            Ok(PropertyValue::Array(items?))
        }
        serde_json::Value::Object(obj) => {
            let mut map = HashMap::new();
            for (k, v) in obj {
                map.insert(k, json_to_property_value(v)?);
            }
            Ok(PropertyValue::Object(map))
        }
    }
}
