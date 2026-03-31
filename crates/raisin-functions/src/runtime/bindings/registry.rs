// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Shared bindings registry for function runtimes
//!
//! This module defines the central registry of API method descriptors that
//! both QuickJS and Starlark runtimes use. Methods are defined ONCE here
//! and both runtimes generate their bindings from this single source.

use crate::api::FunctionApi;
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Describes an API method that can be bound to any runtime
#[derive(Clone)]
pub struct ApiMethodDescriptor {
    /// Internal name used in Rust (e.g., "nodes_get")
    pub internal_name: &'static str,

    /// JavaScript method name (e.g., "get" for raisin.nodes.get)
    pub js_name: &'static str,

    /// Python method name (snake_case, e.g., "get" for raisin.nodes.get)
    pub py_name: &'static str,

    /// Category/namespace (e.g., "nodes", "sql", "http")
    pub category: &'static str,

    /// Argument specifications
    pub args: Vec<ArgSpec>,

    /// Return type
    pub return_type: ReturnType,

    /// The invoker function that calls the actual API method
    pub invoker: InvokerFn,
}

/// Argument specification
#[derive(Clone, Copy, Debug)]
pub struct ArgSpec {
    /// Argument name
    pub name: &'static str,

    /// Argument type
    pub arg_type: ArgType,
}

impl ArgSpec {
    pub const fn new(name: &'static str, arg_type: ArgType) -> Self {
        Self { name, arg_type }
    }
}

/// Argument types supported by the binding layer
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgType {
    /// Required string argument
    String,
    /// Optional string argument
    OptionalString,
    /// Required JSON value (parsed from string in JS/Starlark)
    Json,
    /// Optional JSON value
    OptionalJson,
    /// Required u32 integer
    U32,
    /// Optional u32 integer
    OptionalU32,
    /// Required i64 integer
    I64,
    /// Optional i64 integer
    OptionalI64,
    /// Required boolean
    Bool,
    /// Optional boolean
    OptionalBool,
    /// String array (Vec<String>)
    StringArray,
    /// JSON array (Vec<Value>)
    JsonArray,
}

/// Return types for API methods
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReturnType {
    /// Returns JSON object
    Json,
    /// Returns JSON or null
    OptionalJson,
    /// Returns JSON array (empty on error)
    JsonArray,
    /// Returns boolean
    Bool,
    /// Returns i64 integer
    I64,
    /// Returns string
    String,
    /// Returns void (success indicator)
    Void,
}

/// Type alias for the invoker function
pub type InvokerFn =
    fn(Arc<dyn FunctionApi>, Vec<Value>) -> BoxFuture<'static, Result<InvokeResult>>;

/// Result of an invocation
#[derive(Debug, Clone)]
pub enum InvokeResult {
    /// JSON value result
    Json(Value),
    /// Optional JSON result (can be null)
    OptionalJson(Option<Value>),
    /// Array of JSON values
    JsonArray(Vec<Value>),
    /// Boolean result
    Bool(bool),
    /// Integer result
    I64(i64),
    /// String result
    String(String),
    /// Optional string result
    OptionalString(Option<String>),
    /// Void result (success)
    Void,
}

impl InvokeResult {
    /// Convert to JSON string for runtime consumption
    pub fn to_json_string(&self) -> String {
        match self {
            InvokeResult::Json(v) => {
                serde_json::to_string(v).unwrap_or_else(|_| "null".to_string())
            }
            InvokeResult::OptionalJson(Some(v)) => {
                serde_json::to_string(v).unwrap_or_else(|_| "null".to_string())
            }
            InvokeResult::OptionalJson(None) => "null".to_string(),
            InvokeResult::JsonArray(arr) => {
                serde_json::to_string(arr).unwrap_or_else(|_| "[]".to_string())
            }
            InvokeResult::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            InvokeResult::I64(n) => n.to_string(),
            InvokeResult::String(s) => {
                serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
            }
            InvokeResult::OptionalString(Some(s)) => {
                serde_json::to_string(s).unwrap_or_else(|_| "null".to_string())
            }
            InvokeResult::OptionalString(None) => "null".to_string(),
            InvokeResult::Void => "true".to_string(),
        }
    }

    /// Convert to serde_json::Value
    pub fn to_value(&self) -> Value {
        match self {
            InvokeResult::Json(v) => v.clone(),
            InvokeResult::OptionalJson(Some(v)) => v.clone(),
            InvokeResult::OptionalJson(None) => Value::Null,
            InvokeResult::JsonArray(arr) => Value::Array(arr.clone()),
            InvokeResult::Bool(b) => Value::Bool(*b),
            InvokeResult::I64(n) => Value::Number((*n).into()),
            InvokeResult::String(s) => Value::String(s.clone()),
            InvokeResult::OptionalString(Some(s)) => Value::String(s.clone()),
            InvokeResult::OptionalString(None) => Value::Null,
            InvokeResult::Void => Value::Bool(true),
        }
    }
}

/// Global registry of all API method bindings
pub struct BindingsRegistry {
    methods: Vec<ApiMethodDescriptor>,
}

impl Default for BindingsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BindingsRegistry {
    /// Create a new empty registry
    pub const fn new() -> Self {
        Self {
            methods: Vec::new(),
        }
    }

    /// Create registry with methods
    pub fn with_methods(methods: Vec<ApiMethodDescriptor>) -> Self {
        Self { methods }
    }

    /// Get all registered methods
    pub fn methods(&self) -> &[ApiMethodDescriptor] {
        &self.methods
    }

    /// Get methods by category
    pub fn methods_by_category(&self, category: &str) -> Vec<&ApiMethodDescriptor> {
        self.methods
            .iter()
            .filter(|m| m.category == category)
            .collect()
    }

    /// Get all unique categories
    pub fn categories(&self) -> Vec<&'static str> {
        let mut cats: Vec<&'static str> = self.methods.iter().map(|m| m.category).collect();
        cats.sort();
        cats.dedup();
        cats
    }

    /// Find a method by internal name
    pub fn find_by_internal_name(&self, name: &str) -> Option<&ApiMethodDescriptor> {
        self.methods.iter().find(|m| m.internal_name == name)
    }

    /// Get all internal names (for parity verification)
    pub fn all_internal_names(&self) -> Vec<&'static str> {
        self.methods.iter().map(|m| m.internal_name).collect()
    }
}

/// Helper to parse arguments from Value array
pub struct ArgParser<'a> {
    args: &'a [Value],
    pos: usize,
}

impl<'a> ArgParser<'a> {
    pub fn new(args: &'a [Value]) -> Self {
        Self { args, pos: 0 }
    }

    /// Get next string argument
    pub fn string(&mut self) -> Result<String> {
        let val = self.args.get(self.pos).ok_or_else(|| {
            raisin_error::Error::Validation(format!("Missing argument at position {}", self.pos))
        })?;
        self.pos += 1;
        val.as_str().map(|s| s.to_string()).ok_or_else(|| {
            raisin_error::Error::Validation(format!("Expected string at position {}", self.pos - 1))
        })
    }

    /// Get next optional string argument
    pub fn optional_string(&mut self) -> Result<Option<String>> {
        let val = self.args.get(self.pos);
        self.pos += 1;
        match val {
            Some(Value::Null) | None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(v) if v.is_string() => Ok(v.as_str().map(|s| s.to_string())),
            _ => Ok(None),
        }
    }

    /// Get next JSON value argument
    pub fn json(&mut self) -> Result<Value> {
        let val = self.args.get(self.pos).cloned().unwrap_or(Value::Null);
        self.pos += 1;
        Ok(val)
    }

    /// Get next optional JSON value argument
    pub fn optional_json(&mut self) -> Result<Option<Value>> {
        let val = self.args.get(self.pos).cloned();
        self.pos += 1;
        match val {
            Some(Value::Null) | None => Ok(None),
            Some(v) => Ok(Some(v)),
        }
    }

    /// Get next u32 argument
    pub fn u32(&mut self) -> Result<u32> {
        let val = self.args.get(self.pos).ok_or_else(|| {
            raisin_error::Error::Validation(format!("Missing argument at position {}", self.pos))
        })?;
        self.pos += 1;
        val.as_u64()
            .map(|n| n as u32)
            .or_else(|| val.as_i64().map(|n| n as u32))
            .ok_or_else(|| {
                raisin_error::Error::Validation(format!(
                    "Expected u32 at position {}",
                    self.pos - 1
                ))
            })
    }

    /// Get next optional u32 argument
    pub fn optional_u32(&mut self) -> Result<Option<u32>> {
        let val = self.args.get(self.pos);
        self.pos += 1;
        match val {
            Some(Value::Null) | None => Ok(None),
            Some(v) => Ok(v
                .as_u64()
                .map(|n| n as u32)
                .or_else(|| v.as_i64().map(|n| n as u32))),
        }
    }

    /// Get next i64 argument
    pub fn i64(&mut self) -> Result<i64> {
        let val = self.args.get(self.pos).ok_or_else(|| {
            raisin_error::Error::Validation(format!("Missing argument at position {}", self.pos))
        })?;
        self.pos += 1;
        val.as_i64().ok_or_else(|| {
            raisin_error::Error::Validation(format!("Expected i64 at position {}", self.pos - 1))
        })
    }

    /// Get next JSON array argument
    pub fn json_array(&mut self) -> Result<Vec<Value>> {
        let val = self
            .args
            .get(self.pos)
            .cloned()
            .unwrap_or(Value::Array(vec![]));
        self.pos += 1;
        match val {
            Value::Array(arr) => Ok(arr),
            Value::Null => Ok(vec![]),
            _ => Err(raisin_error::Error::Validation(format!(
                "Expected array at position {}",
                self.pos - 1
            ))),
        }
    }

    /// Get next boolean argument
    pub fn bool(&mut self) -> Result<bool> {
        let val = self.args.get(self.pos).ok_or_else(|| {
            raisin_error::Error::Validation(format!("Missing argument at position {}", self.pos))
        })?;
        self.pos += 1;
        val.as_bool().ok_or_else(|| {
            raisin_error::Error::Validation(format!("Expected bool at position {}", self.pos - 1))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invoke_result_to_json() {
        assert_eq!(InvokeResult::Bool(true).to_json_string(), "true");
        assert_eq!(InvokeResult::Bool(false).to_json_string(), "false");
        assert_eq!(InvokeResult::I64(42).to_json_string(), "42");
        assert_eq!(InvokeResult::Void.to_json_string(), "true");
        assert_eq!(InvokeResult::OptionalJson(None).to_json_string(), "null");
        assert_eq!(
            InvokeResult::String("hello".to_string()).to_json_string(),
            "\"hello\""
        );
    }

    #[test]
    fn test_arg_parser() {
        let args = vec![
            Value::String("workspace".to_string()),
            Value::String("/path".to_string()),
            Value::Number(10.into()),
        ];

        let mut parser = ArgParser::new(&args);
        assert_eq!(parser.string().unwrap(), "workspace");
        assert_eq!(parser.string().unwrap(), "/path");
        assert_eq!(parser.u32().unwrap(), 10);
    }
}
