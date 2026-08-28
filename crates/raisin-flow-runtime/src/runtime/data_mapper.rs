// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Data mapping and expression evaluation utility.
//!
//! This module provides the `DataMapper` struct which is responsible for
//! resolving variable references and evaluating expressions within flow steps.
//! It supports:
//! - Recursive resolution in JSON Objects and Arrays
//! - Whole-string expressions that keep their native JSON type
//!   (e.g. `"${input.count}"` resolves to a number)
//! - Inline interpolation inside larger strings
//!   (e.g. `"Hello {{ input.user.name }}!"`)
//! - Expression evaluation using `raisin-rel`
//!
//! Both `${expr}` and `{{ expr }}` markers are supported. The evaluation
//! context is built by [`FlowContext::to_json`] and exposes:
//! - `input.*` - the flow input / triggering node data
//! - `steps.<step_id>.*` - outputs of previously completed steps
//! - `trigger.*` - trigger info (event type, node path, ...)
//! - root-level flow variables
//! - `error.*` - error info when following an error edge

use crate::types::{FlowContext, FlowError, FlowResult};
use serde_json::Value;
use tracing::debug;

/// Utility for mapping data and evaluating expressions
pub struct DataMapper;

/// A segment of a template string: either literal text or an expression.
enum Segment<'a> {
    Literal(&'a str),
    Expr(&'a str),
}

impl DataMapper {
    /// Map a value by resolving any expressions or variables within it.
    ///
    /// This method recursively processes the input value:
    /// - Strings containing `${...}` or `{{...}}` are resolved (whole-string
    ///   expressions keep their native type, mixed strings are interpolated)
    /// - Objects and Arrays are processed recursively
    /// - Other values are returned as-is
    pub fn map(value: &Value, context: &FlowContext) -> FlowResult<Value> {
        match value {
            Value::String(s) => Self::resolve_string(s, context),
            Value::Array(arr) => {
                let mut resolved_arr = Vec::with_capacity(arr.len());
                for item in arr {
                    resolved_arr.push(Self::map(item, context)?);
                }
                Ok(Value::Array(resolved_arr))
            }
            Value::Object(obj) => {
                let mut resolved_obj = serde_json::Map::with_capacity(obj.len());
                for (k, v) in obj {
                    // We map values, but keys are kept as-is
                    resolved_obj.insert(k.clone(), Self::map(v, context)?);
                }
                Ok(Value::Object(resolved_obj))
            }
            _ => Ok(value.clone()),
        }
    }

    /// Resolve a string value, evaluating any embedded expressions.
    ///
    /// - A string that is (modulo surrounding whitespace) a single
    ///   `${expr}` / `{{ expr }}` resolves to the expression's native value.
    /// - A string mixing literal text and expressions resolves to a string
    ///   with each expression interpolated.
    /// - A string without expression markers is returned unchanged.
    fn resolve_string(s: &str, context: &FlowContext) -> FlowResult<Value> {
        if !s.contains("${") && !s.contains("{{") {
            return Ok(Value::String(s.to_string()));
        }

        let segments = Self::tokenize(s);

        // Whole-string expression: exactly one expression and only
        // whitespace literals around it -> keep native type.
        let expr_count = segments
            .iter()
            .filter(|seg| matches!(seg, Segment::Expr(_)))
            .count();
        if expr_count == 1
            && segments.iter().all(|seg| match seg {
                Segment::Expr(_) => true,
                Segment::Literal(lit) => lit.trim().is_empty(),
            })
        {
            let expr = segments
                .iter()
                .find_map(|seg| match seg {
                    Segment::Expr(e) => Some(*e),
                    Segment::Literal(_) => None,
                })
                .expect("expr_count == 1");
            return Self::evaluate_expression(expr, context);
        }

        if expr_count == 0 {
            // Contains "${" or "{{" but no complete expression - literal.
            return Ok(Value::String(s.to_string()));
        }

        // Inline interpolation: concatenate literals and stringified results.
        let mut out = String::with_capacity(s.len());
        for seg in segments {
            match seg {
                Segment::Literal(lit) => out.push_str(lit),
                Segment::Expr(expr) => {
                    let resolved = Self::evaluate_expression(expr, context)?;
                    out.push_str(&Self::stringify(&resolved));
                }
            }
        }
        Ok(Value::String(out))
    }

    /// Split a template string into literal and expression segments.
    ///
    /// `${...}` is matched with brace-depth counting (so nested braces in
    /// the expression are allowed). `{{ ... }}` is matched up to the next
    /// `}}`. Unterminated markers are treated as literal text.
    fn tokenize(s: &str) -> Vec<Segment<'_>> {
        let bytes = s.as_bytes();
        let mut segments = Vec::new();
        let mut lit_start = 0;
        let mut i = 0;

        while i < bytes.len() {
            if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                // ${...} with depth counting
                let expr_start = i + 2;
                let mut depth = 1usize;
                let mut j = expr_start;
                while j < bytes.len() && depth > 0 {
                    match bytes[j] {
                        b'{' => depth += 1,
                        b'}' => depth -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                if depth == 0 {
                    if lit_start < i {
                        segments.push(Segment::Literal(&s[lit_start..i]));
                    }
                    segments.push(Segment::Expr(s[expr_start..j - 1].trim()));
                    i = j;
                    lit_start = i;
                    continue;
                }
                // Unterminated - fall through as literal
            } else if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                // {{...}} up to the next }}
                let expr_start = i + 2;
                if let Some(rel_end) = s[expr_start..].find("}}") {
                    let expr_end = expr_start + rel_end;
                    if lit_start < i {
                        segments.push(Segment::Literal(&s[lit_start..i]));
                    }
                    segments.push(Segment::Expr(s[expr_start..expr_end].trim()));
                    i = expr_end + 2;
                    lit_start = i;
                    continue;
                }
                // Unterminated - fall through as literal
            }
            i += 1;
        }

        if lit_start < s.len() {
            segments.push(Segment::Literal(&s[lit_start..]));
        }
        segments
    }

    /// Convert a resolved value to its string form for interpolation.
    ///
    /// Strings are inserted raw (no quotes), null becomes the empty string,
    /// and other values use their compact JSON representation.
    fn stringify(value: &Value) -> String {
        match value {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }

    /// Evaluate a raisin-rel expression against the flow context.
    fn evaluate_expression(expr: &str, context: &FlowContext) -> FlowResult<Value> {
        debug!("Evaluating expression: {}", expr);

        // Build the evaluation context from the flow context (single source
        // of truth shared with condition evaluation).
        let eval_ctx = raisin_rel::EvalContext::from_json(context.to_json()).map_err(|e| {
            FlowError::ConditionEvaluation(format!("Invalid evaluation context: {}", e))
        })?;

        let result = raisin_rel::eval(expr, &eval_ctx).map_err(|e| {
            FlowError::ConditionEvaluation(format!("Evaluation failed for '{}': {}", expr, e))
        })?;

        Ok(result.to_json())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_test_context() -> FlowContext {
        let mut context = FlowContext::new(
            "test-id".to_string(),
            json!({
                "user": {
                    "name": "Alice",
                    "age": 30
                }
            }),
        );
        context.set_variable("status".to_string(), Value::String("active".to_string()));
        context.record_step_output("step_1".to_string(), json!({"result": 100}));
        context
    }

    /// A WHOLE NAMESPACE resolves as one value.
    ///
    /// This is the linchpin of a parallel fork carrying its parent's context:
    /// each branch runs as a CHILD FLOW INSTANCE with its own empty
    /// `step_outputs`, so a condition written before the fork
    /// (`steps.draft.approved`) resolves to nothing inside a branch unless the
    /// parent's whole `steps` map is handed down through `input_mapping`.
    /// If `"${steps}"` ever stopped resolving to the map itself, branch
    /// conditions would silently read null rather than fail.
    #[test]
    fn a_whole_namespace_resolves_as_one_value() {
        let context = create_test_context();
        let result = DataMapper::map(&json!("${steps}"), &context).unwrap();
        assert_eq!(result, json!({"step_1": {"result": 100}}));

        // And nested inside the mapping object a fork would actually emit.
        let mapping = json!({ "flow_input": "${input}", "__parent_steps": "${steps}" });
        let mapped = DataMapper::map(&mapping, &context).unwrap();
        assert_eq!(mapped["__parent_steps"]["step_1"]["result"], 100);
        assert_eq!(mapped["flow_input"]["user"]["name"], "Alice");
    }

    #[test]
    fn test_map_literal() {
        let context = create_test_context();
        let val = json!("hello");
        let result = DataMapper::map(&val, &context).unwrap();
        assert_eq!(result, json!("hello"));
    }

    #[test]
    fn test_map_input_expression() {
        let context = create_test_context();
        let val = json!("${input.user.name}");
        let result = DataMapper::map(&val, &context).unwrap();
        assert_eq!(result, json!("Alice"));
    }

    #[test]
    fn test_map_steps_expression() {
        let context = create_test_context();
        let val = json!("${steps.step_1.result}");
        let result = DataMapper::map(&val, &context).unwrap();
        assert_eq!(result, json!(100));
    }

    #[test]
    fn test_map_template_syntax() {
        let context = create_test_context();
        let val = json!("{{ input.user.name }}");
        let result = DataMapper::map(&val, &context).unwrap();
        assert_eq!(result, json!("Alice"));
    }

    #[test]
    fn test_map_math_expression() {
        let context = create_test_context();
        let val = json!("${input.user.age + 5}");
        let result = DataMapper::map(&val, &context).unwrap();
        assert_eq!(result, json!(35));
    }

    #[test]
    fn test_whole_string_keeps_native_type() {
        let context = create_test_context();
        // Number stays a number, object stays an object
        assert_eq!(
            DataMapper::map(&json!("${input.user.age}"), &context).unwrap(),
            json!(30)
        );
        assert_eq!(
            DataMapper::map(&json!("${input.user}"), &context).unwrap(),
            json!({"name": "Alice", "age": 30})
        );
        // Surrounding whitespace still counts as whole-string
        assert_eq!(
            DataMapper::map(&json!("  ${input.user.age}  "), &context).unwrap(),
            json!(30)
        );
    }

    #[test]
    fn test_inline_interpolation() {
        let context = create_test_context();
        let val = json!("Hello {{ input.user.name }}, you are ${input.user.age} years old");
        let result = DataMapper::map(&val, &context).unwrap();
        assert_eq!(result, json!("Hello Alice, you are 30 years old"));
    }

    #[test]
    fn test_inline_interpolation_non_string_values() {
        let context = create_test_context();
        let val = json!("user=${input.user} result=${steps.step_1.result}");
        let result = DataMapper::map(&val, &context).unwrap();
        let s = result.as_str().unwrap();
        assert!(s.contains("\"name\":\"Alice\""), "got: {}", s);
        assert!(s.ends_with("result=100"), "got: {}", s);
    }

    #[test]
    fn test_unterminated_marker_is_literal() {
        let context = create_test_context();
        assert_eq!(
            DataMapper::map(&json!("price is ${100"), &context).unwrap(),
            json!("price is ${100")
        );
        assert_eq!(
            DataMapper::map(&json!("brace {{ only"), &context).unwrap(),
            json!("brace {{ only")
        );
    }

    #[test]
    fn test_map_recursive_structure() {
        let context = create_test_context();
        let val = json!({
            "name": "${input.user.name}",
            "info": {
                "age_next_year": "${input.user.age + 1}"
            },
            "list": ["${status}", "literal"]
        });

        let result = DataMapper::map(&val, &context).unwrap();
        assert_eq!(result["name"], json!("Alice"));
        assert_eq!(result["info"]["age_next_year"], json!(31));
        assert_eq!(result["list"][0], json!("active"));
        assert_eq!(result["list"][1], json!("literal"));
    }

    #[test]
    fn test_object_result_roundtrip() {
        // Regression: objects/arrays used to be lossily debug-formatted
        let context = create_test_context();
        let result = DataMapper::map(&json!("${steps.step_1}"), &context).unwrap();
        assert_eq!(result, json!({"result": 100}));
    }
}
