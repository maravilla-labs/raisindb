// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Starlark gateway module providing __raisin_call and JSON helpers

use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::values::{none::NoneType, tuple::UnpackTuple, Heap, Value};

use super::conversions::{json_to_starlark, starlark_value_to_json};
use super::thread_local::{push_log, CURRENT_API, CURRENT_HANDLE};
use crate::runtime::bindings::methods::registry;
use crate::types::LogEntry;

/// Gateway function module that provides __raisin_call
#[starlark_module]
pub(super) fn raisin_gateway_module(builder: &mut GlobalsBuilder) {
    /// Call a RaisinDB API method by name
    ///
    /// Args:
    ///   method: The internal method name (e.g., "nodes_get_by_id")
    ///   args: List of arguments to pass to the method
    ///
    /// Returns:
    ///   JSON string with the result
    fn __raisin_call<'v>(method: &str, args: Value<'v>) -> anyhow::Result<String> {
        // Get API and handle from thread-local storage
        let api = CURRENT_API.with(|cell| cell.borrow().clone());
        let handle = CURRENT_HANDLE.with(|cell| cell.borrow().clone());

        let api = api.ok_or_else(|| anyhow::anyhow!("RaisinDB API not initialized"))?;
        let handle = handle.ok_or_else(|| anyhow::anyhow!("Tokio handle not available"))?;

        // Look up the method in the registry
        let reg = registry();
        let method_desc = reg
            .find_by_internal_name(method)
            .ok_or_else(|| anyhow::anyhow!("Unknown method: {}", method))?;

        // Convert Starlark args list to JSON
        let json_args: Vec<serde_json::Value> =
            if let Some(list) = starlark::values::list::ListRef::from_value(args) {
                list.iter().map(starlark_value_to_json).collect()
            } else {
                return Err(anyhow::anyhow!(
                    "Expected list argument, got: {}",
                    args.get_type()
                ));
            };

        // Call the invoker using block_in_place to avoid "Cannot start a runtime from within a runtime" panic.
        // This is needed because StarlarkRuntime::execute() is async, and the Starlark evaluator is sync.
        // block_in_place tells tokio to park the current task while we block.
        let invoker = method_desc.invoker;
        let result = tokio::task::block_in_place(|| {
            handle.block_on(async move { (invoker)(api, json_args).await })
        });

        match result {
            Ok(invoke_result) => Ok(invoke_result.to_json_string()),
            Err(e) => {
                // Return error as JSON string
                let error_json = serde_json::json!({
                    "error": true,
                    "message": e.to_string()
                });
                Ok(serde_json::to_string(&error_json)
                    .unwrap_or_else(|_| r#"{"error":true,"message":"Unknown error"}"#.to_string()))
            }
        }
    }

    /// Parse a JSON string into a Starlark dict/list/value
    fn json_decode<'v>(s: &str, heap: &'v Heap) -> anyhow::Result<Value<'v>> {
        let json_val: serde_json::Value =
            serde_json::from_str(s).map_err(|e| anyhow::anyhow!("JSON parse error: {}", e))?;
        Ok(json_to_starlark(heap, &json_val))
    }

    /// Encode a Starlark value to JSON string
    fn json_encode(value: Value) -> anyhow::Result<String> {
        let json_val = starlark_value_to_json(value);
        serde_json::to_string(&json_val).map_err(|e| anyhow::anyhow!("JSON encode error: {}", e))
    }

    /// Print values to console (captured as log entries)
    fn print(#[starlark(args)] args: UnpackTuple<Value>) -> anyhow::Result<NoneType> {
        let parts: Vec<String> = args
            .items
            .iter()
            .map(|v| {
                // For strings, use the raw content without quotes (like Python's print)
                if let Some(s) = v.unpack_str() {
                    s.to_string()
                } else {
                    v.to_string()
                }
            })
            .collect();
        let message = parts.join(" ");
        tracing::info!(target: "starlark_print", "{}", message);
        push_log(LogEntry::info(&message));
        Ok(NoneType)
    }

    /// Internal function to log with a specific level
    fn __raisin_log<'v>(level: &str, args: Value<'v>) -> anyhow::Result<NoneType> {
        // Extract items from the list argument
        let items: Vec<Value> =
            if let Some(list) = starlark::values::list::ListRef::from_value(args) {
                list.iter().collect()
            } else {
                return Err(anyhow::anyhow!(
                    "Expected list argument, got: {}",
                    args.get_type()
                ));
            };

        let parts: Vec<String> = items
            .iter()
            .map(|v| {
                // For strings, use the raw content without quotes (like Python's print)
                if let Some(s) = v.unpack_str() {
                    s.to_string()
                } else {
                    v.to_string()
                }
            })
            .collect();
        let message = parts.join(" ");

        match level {
            "debug" => {
                tracing::debug!(target: "starlark_log", "{}", message);
                push_log(LogEntry::debug(&message));
            }
            "info" => {
                tracing::info!(target: "starlark_log", "{}", message);
                push_log(LogEntry::info(&message));
            }
            "warn" => {
                tracing::warn!(target: "starlark_log", "{}", message);
                push_log(LogEntry::warn(&message));
            }
            "error" => {
                tracing::error!(target: "starlark_log", "{}", message);
                push_log(LogEntry::error(&message));
            }
            _ => {
                tracing::info!(target: "starlark_log", "{}", message);
                push_log(LogEntry::info(&message));
            }
        }
        Ok(NoneType)
    }
}
