//! Aggregate function naming utilities
//!
//! This module provides functions for generating canonical column names for aggregate
//! functions. These names are used to match aggregate results with their expressions.

use raisin_sql::analyzer::{Expr, TypedExpr};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Generate a canonical column name for an aggregate function
///
/// This is used to match aggregate results with their expressions in the row data.
///
/// # Arguments
/// * `func_name` - The aggregate function name (COUNT, SUM, etc.)
/// * `args` - The function arguments
///
/// # Returns
/// Canonical column name string
///
/// # Examples
/// ```ignore
/// generate_aggregate_column_name("COUNT", &[]) -> "count_star"
/// generate_aggregate_column_name("SUM", &[col_expr]) -> "sum_table_column"
/// ```
///
/// # Notes
/// - COUNT always uses "count_star" for consistency
/// - Other aggregates use function name + argument description
pub fn generate_aggregate_column_name(func_name: &str, args: &[TypedExpr]) -> String {
    // Special case: COUNT always uses "count_star" for consistency
    // COUNT(*) and COUNT(expr) are represented differently in different contexts
    // but should map to the same canonical name
    if func_name.to_uppercase() == "COUNT" {
        return "count_star".to_string();
    }

    if args.is_empty() {
        // Other aggregates with no args (unlikely)
        format!("{}_star", func_name.to_lowercase())
    } else if args.len() == 1 {
        // Most aggregates have one argument
        let arg_name = match &args[0].expr {
            Expr::Column { table, column } => format!("{}.{}", table, column),
            _ => "expr".to_string(),
        };
        format!(
            "{}_{}",
            func_name.to_lowercase(),
            arg_name.replace('.', "_")
        )
    } else {
        format!("{}_multi", func_name.to_lowercase())
    }
}

/// Generate canonical column name for a function expression (for GROUP BY lookups)
///
/// Must match the logic in hash_aggregate.rs extract_column_name
///
/// # Arguments
/// * `func_name` - The function name
/// * `args` - The function arguments
/// * `filter` - Optional FILTER clause
///
/// # Returns
/// Canonical column name string including filter hash if present
pub fn generate_function_column_name(
    func_name: &str,
    args: &[TypedExpr],
    filter: &Option<Box<TypedExpr>>,
) -> String {
    let func_name_upper = func_name.to_uppercase();

    let base_name = if args.is_empty() {
        // No arguments (e.g., NOW())
        format!("{}()", func_name_upper)
    } else if args.len() == 1 {
        // Single argument - try to extract column name recursively
        let arg_name = extract_arg_name(&args[0]);
        format!("{}({})", func_name_upper, arg_name)
    } else {
        // Multiple arguments
        format!("{}(...)", func_name_upper)
    };

    // Include FILTER clause in canonical name to distinguish filtered aggregates
    // This must match the logic in hash_aggregate.rs generate_canonical_aggregate_name
    if let Some(ref filter_expr) = filter {
        // Hash the filter expression to create a unique suffix
        let mut hasher = DefaultHasher::new();
        format!("{:?}", filter_expr).hash(&mut hasher);
        let filter_hash = hasher.finish();
        format!("{}_filter_{:x}", base_name, filter_hash)
    } else {
        base_name
    }
}

/// Recursively extract argument name for canonical function naming
fn extract_arg_name(expr: &TypedExpr) -> String {
    match &expr.expr {
        Expr::Column { table, column } => format!("{}.{}", table, column),
        Expr::Function {
            name, args, filter, ..
        } => {
            // Recursive case for nested functions
            generate_function_column_name(name, args, filter)
        }
        _ => "...".to_string(),
    }
}
