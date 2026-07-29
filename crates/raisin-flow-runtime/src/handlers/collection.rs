// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Shared resolution of a "collection expression" to a list of items.
//!
//! Used by every step that iterates a runtime list - the loop step's
//! `collection` and the parallel container's `for_each` - so both accept
//! exactly the same forms:
//!
//! - a template expression (`"${input.items}"`, `"{{ steps.fetch.rows }}"`)
//! - a bare variable reference (`"$items"`, `"$.items"`)
//! - a JSON array literal (`"[1, 2, 3]"`)
//!
//! An object resolves to its entries as `{ "key": …, "value": … }` pairs so
//! a map can be iterated without a separate step type.

use serde_json::{json, Value};

use crate::types::{FlowContext, FlowError, FlowResult};

/// Resolve a collection expression against the flow context.
pub(crate) fn resolve_collection(expr: &str, context: &FlowContext) -> FlowResult<Vec<Value>> {
    // Template expressions: "${input.items}", "{{ steps.fetch.rows }}", ...
    if expr.contains("${") || expr.contains("{{") {
        let resolved = crate::runtime::DataMapper::map(&Value::String(expr.to_string()), context)?;
        return match resolved {
            Value::Array(arr) => Ok(arr),
            Value::Object(obj) => Ok(object_entries(&obj)),
            other => Err(FlowError::InvalidNodeConfiguration(format!(
                "Collection '{}' resolved to a non-iterable value: {}",
                expr, other
            ))),
        };
    }

    // Bare variable reference ("$items" / "$.items"), looked up in flow
    // variables first, then the flow input.
    let var_name = expr.trim_start_matches("$.").trim_start_matches('$');

    match context
        .variables
        .get(var_name)
        .or_else(|| context.input.get(var_name))
    {
        Some(Value::Array(arr)) => Ok(arr.clone()),
        Some(Value::Object(obj)) => Ok(object_entries(obj)),
        Some(_) => Err(FlowError::InvalidNodeConfiguration(format!(
            "Collection '{}' is not iterable",
            expr
        ))),
        // Not a reference at all - accept a JSON array literal.
        None => serde_json::from_str(expr).map_err(|_| {
            FlowError::InvalidNodeConfiguration(format!("Could not resolve collection: {}", expr))
        }),
    }
}

/// An object iterates as its `{ key, value }` entries.
fn object_entries(obj: &serde_json::Map<String, Value>) -> Vec<Value> {
    obj.iter()
        .map(|(k, v)| json!({"key": k, "value": v}))
        .collect()
}
