//! Aggregate functions for Cypher
//!
//! Provides marker implementations for aggregate functions (COUNT, SUM, AVG, etc.).
//! Actual aggregation logic is handled by the projection module - these functions
//! return marker values that signal to the executor that aggregation is needed.

use raisin_cypher_parser::Expr;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::Storage;

use super::registry::CypherFunction;
use super::traits::FunctionContext;
use crate::physical_plan::cypher::types::VariableBinding;

/// Evaluate aggregate function marker
///
/// This function is called during expression evaluation for aggregate functions.
/// It returns an error because aggregates should be handled by the projection
/// module, not during regular expression evaluation.
///
/// # Arguments
///
/// * `func` - The aggregate function variant
/// * `args` - Expression arguments (not used)
/// * `binding` - Current variable binding (not used)
/// * `context` - Function evaluation context (not used)
///
/// # Returns
///
/// Always returns Error::Validation indicating that aggregates should be
/// handled during projection, not during expression evaluation.
///
/// # Note
///
/// Aggregate functions (COUNT, SUM, AVG, MIN, MAX, COLLECT) require special
/// handling in the projection phase where they can accumulate values across
/// multiple bindings. They should never be called directly during expression
/// evaluation.
pub async fn evaluate_aggregate<S: Storage>(
    func: &CypherFunction,
    _args: &[Expr],
    _binding: &VariableBinding,
    _context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    // Should never be called directly - handled by projection module
    let func_name = match func {
        CypherFunction::Count => "COUNT",
        CypherFunction::Sum => "SUM",
        CypherFunction::Avg => "AVG",
        CypherFunction::Min => "MIN",
        CypherFunction::Max => "MAX",
        CypherFunction::Collect => "COLLECT",
        _ => "UNKNOWN",
    };

    Err(Error::Validation(format!(
        "{} is an aggregate function and should be handled during projection",
        func_name
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregate_function_enum() {
        // Test that aggregate functions are properly identified
        assert!(CypherFunction::Count.is_aggregate());
        assert!(CypherFunction::Sum.is_aggregate());
        assert!(CypherFunction::Avg.is_aggregate());
        assert!(CypherFunction::Min.is_aggregate());
        assert!(CypherFunction::Max.is_aggregate());
        assert!(CypherFunction::Collect.is_aggregate());

        // Non-aggregates should not be marked as aggregates
        assert!(!CypherFunction::Lookup.is_aggregate());
        assert!(!CypherFunction::Degree.is_aggregate());
    }
}
