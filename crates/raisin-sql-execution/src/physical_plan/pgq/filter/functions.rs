//! PGQ function call evaluation (e.g., CARDINALITY).

use raisin_sql::ast::Expr;

use super::Result;
use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::pgq::types::{SqlValue, VariableBinding};

/// Evaluate a function call expression
///
/// Handles PGQ-specific functions like CARDINALITY for path length.
pub(super) fn evaluate_function(
    name: &str,
    args: &[Expr],
    binding: &VariableBinding,
) -> Result<SqlValue> {
    let name_lower = name.to_lowercase();
    match name_lower.as_str() {
        "cardinality" => {
            // CARDINALITY(r) - returns the number of hops in a variable-length path
            // The path length is encoded in relation_type as "TYPE[length]"
            if args.len() != 1 {
                return Err(ExecutionError::Validation(
                    "CARDINALITY requires exactly one argument".into(),
                ));
            }

            // Get the variable name from the argument
            if let Some(var_name) = get_variable_name(&args[0]) {
                if let Some(rel) = binding.get_relation(&var_name) {
                    // Try to extract path length from encoded relation_type
                    if let Some(len) = extract_path_length(&rel.relation_type) {
                        return Ok(SqlValue::Integer(len));
                    }
                    // Single-hop relationship without encoding - return 1
                    return Ok(SqlValue::Integer(1));
                }
                // Variable not found as relation
                return Err(ExecutionError::Validation(format!(
                    "CARDINALITY argument '{}' is not a relationship variable",
                    var_name
                )));
            }
            Err(ExecutionError::Validation(
                "CARDINALITY requires a relationship variable as argument".into(),
            ))
        }
        _ => Err(ExecutionError::Validation(format!(
            "Unsupported function in PGQ expression: {}",
            name
        ))),
    }
}

/// Extract variable name from an expression
///
/// Returns the variable name if the expression is a simple variable reference.
fn get_variable_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::PropertyAccess {
            variable,
            properties,
            ..
        } if properties.is_empty() => Some(variable.clone()),
        _ => None,
    }
}

/// Extract path length from encoded relation_type
///
/// Variable-length paths encode their length in the relation_type string:
/// - "FRIENDS_WITH[2]" -> Some(2)
/// - "FRIENDS_WITH[3]" -> Some(3)
/// - "FRIENDS_WITH" -> None (single-hop, no encoding)
pub(super) fn extract_path_length(relation_type: &str) -> Option<i64> {
    if let Some(start) = relation_type.rfind('[') {
        if let Some(end) = relation_type.rfind(']') {
            if start < end {
                return relation_type[start + 1..end].parse().ok();
            }
        }
    }
    None
}
