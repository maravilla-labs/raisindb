//! COLUMNS Clause Projection
//!
//! Projects graph pattern matches to flat SQL rows.

use std::sync::Arc;

use raisin_sql::ast::{ColumnExpr, ColumnsClause, Expr};
use raisin_storage::Storage;

use super::aggregation::{
    evaluate_avg, evaluate_collect, evaluate_count, evaluate_max, evaluate_min, evaluate_sum,
    has_aggregates,
};
use super::context::PgqContext;
use super::filter::evaluate_expr;
use super::types::{PgqRow, SqlValue, VariableBinding};
use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Project COLUMNS clause to SQL rows
///
/// If the columns contain aggregates, performs grouping.
/// Otherwise, creates one row per binding.
pub async fn project_columns<S: Storage>(
    columns: &ColumnsClause,
    mut bindings: Vec<VariableBinding>,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<Vec<PgqRow>> {
    if has_aggregates(&columns.columns) {
        project_with_aggregates(columns, &mut bindings, storage, context).await
    } else {
        project_simple(columns, &mut bindings, storage, context).await
    }
}

/// Simple projection without aggregates
///
/// Creates one row per binding. Bindings with missing nodes are skipped.
async fn project_simple<S: Storage>(
    columns: &ColumnsClause,
    bindings: &mut [VariableBinding],
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<Vec<PgqRow>> {
    let mut rows = Vec::with_capacity(bindings.len());
    let mut skipped_count = 0;

    for binding in bindings.iter_mut() {
        // Check if any nodes in this binding are missing (tombstone/deleted)
        if binding.has_missing_nodes() {
            let missing_ids = binding.missing_node_ids();
            tracing::warn!(
                "PGQ: Skipping binding due to missing nodes (orphaned relations): {:?}",
                missing_ids
            );
            skipped_count += 1;
            continue;
        }

        let mut row = PgqRow::with_capacity(columns.columns.len());

        for col in &columns.columns {
            // Check if this is a wildcard expression
            if let Expr::Wildcard { qualifier, .. } = &col.expr {
                // Expand wildcard into multiple columns
                let expanded =
                    expand_wildcard_async(qualifier.as_deref(), binding, storage, context).await?;

                for (name, value) in expanded {
                    row.set(name, value);
                }
            } else {
                let value = evaluate_column_expr(&col.expr, binding, storage, context).await?;
                let name = get_column_name(col);
                row.set(name, value);
            }
        }

        rows.push(row);
    }

    if skipped_count > 0 {
        tracing::info!(
            "PGQ: Skipped {} bindings due to missing nodes (orphaned relations)",
            skipped_count
        );
    }

    Ok(rows)
}

/// Projection with aggregates
///
/// Currently supports simple aggregation over all bindings (no GROUP BY).
/// Bindings with missing nodes are filtered out before aggregation.
/// TODO: Add GROUP BY support for proper SQL semantics.
async fn project_with_aggregates<S: Storage>(
    columns: &ColumnsClause,
    bindings: &mut Vec<VariableBinding>,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<Vec<PgqRow>> {
    // Filter out bindings with missing nodes before aggregation
    let original_count = bindings.len();
    bindings.retain(|binding| {
        if binding.has_missing_nodes() {
            tracing::warn!(
                "PGQ: Filtering out binding with missing nodes before aggregation: {:?}",
                binding.missing_node_ids()
            );
            false
        } else {
            true
        }
    });

    let filtered_count = original_count - bindings.len();
    if filtered_count > 0 {
        tracing::info!(
            "PGQ: Filtered {} bindings with missing nodes before aggregation",
            filtered_count
        );
    }

    // For now, aggregate over all bindings (single group)
    let mut row = PgqRow::with_capacity(columns.columns.len());

    for col in &columns.columns {
        let value = evaluate_aggregate_column(&col.expr, bindings, storage, context).await?;
        let name = get_column_name(col);
        row.set(name, value);
    }

    Ok(vec![row])
}

/// Evaluate a column expression
async fn evaluate_column_expr<S: Storage>(
    expr: &Expr,
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    match expr {
        // Wildcards are handled specially in project_simple/project_with_aggregates
        // This shouldn't be reached, but handle gracefully
        Expr::Wildcard { .. } => Err(ExecutionError::Validation(
            "Wildcard (*) should be expanded before evaluation".into(),
        )),

        // Other expressions evaluated normally
        _ => evaluate_expr(expr, binding, storage, context).await,
    }
}

/// Evaluate a potentially aggregate column expression
async fn evaluate_aggregate_column<S: Storage>(
    expr: &Expr,
    bindings: &mut [VariableBinding],
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    match expr {
        Expr::FunctionCall {
            name,
            args,
            distinct,
            ..
        } => {
            let name_lower = name.to_lowercase();
            match name_lower.as_str() {
                "count" => evaluate_count(args, *distinct, bindings, storage, context).await,
                "collect" | "array_agg" => {
                    evaluate_collect(args, *distinct, bindings, storage, context).await
                }
                "sum" => evaluate_sum(args, bindings, storage, context).await,
                "avg" => evaluate_avg(args, bindings, storage, context).await,
                "min" => evaluate_min(args, bindings, storage, context).await,
                "max" => evaluate_max(args, bindings, storage, context).await,
                _ => {
                    // Non-aggregate function - evaluate for first binding only
                    if let Some(binding) = bindings.first_mut() {
                        evaluate_expr(expr, binding, storage, context).await
                    } else {
                        Ok(SqlValue::Null)
                    }
                }
            }
        }

        // Non-aggregate expressions in aggregate context
        // Use value from first binding (like SQL without GROUP BY)
        _ => {
            if let Some(binding) = bindings.first_mut() {
                evaluate_expr(expr, binding, storage, context).await
            } else {
                Ok(SqlValue::Null)
            }
        }
    }
}

/// Get column name from expression
fn get_column_name(col: &ColumnExpr) -> String {
    // Use alias if provided
    if let Some(alias) = &col.alias {
        return alias.clone();
    }

    // Otherwise derive from expression
    match &col.expr {
        Expr::PropertyAccess {
            variable,
            properties,
            ..
        } => {
            if properties.is_empty() {
                variable.clone()
            } else {
                format!("{}_{}", variable, properties.join("_"))
            }
        }
        Expr::FunctionCall { name, .. } => name.to_lowercase(),
        Expr::Wildcard { qualifier, .. } => qualifier.clone().unwrap_or_else(|| "*".into()),
        _ => "column".into(),
    }
}

/// Expand wildcard columns for a variable (async version that loads node data)
///
/// Returns columns for all system fields and properties.
pub async fn expand_wildcard_async<S: Storage>(
    qualifier: Option<&str>,
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<Vec<(String, SqlValue)>> {
    let mut result = Vec::new();

    match qualifier {
        Some(var) => {
            // Expand specific variable: n.*
            expand_single_var(&mut result, var, binding, storage, context).await?;
        }
        None => {
            // Expand all variables: *
            // First collect all variable names to avoid borrow issues
            let node_vars: Vec<String> = binding.node_vars().map(|s| s.to_string()).collect();
            let rel_vars: Vec<String> = binding.relation_vars().map(|s| s.to_string()).collect();

            // Expand each node variable (iterative, not recursive)
            for var in node_vars {
                expand_single_var(&mut result, &var, binding, storage, context).await?;
            }
            // Expand each relation variable
            for var in rel_vars {
                expand_single_var(&mut result, &var, binding, storage, context).await?;
            }
        }
    }

    Ok(result)
}

/// Helper to expand a single variable (avoids async recursion)
async fn expand_single_var<S: Storage>(
    result: &mut Vec<(String, SqlValue)>,
    var: &str,
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<()> {
    if let Some(node) = binding.get_node_mut(var) {
        // Ensure node data is loaded for full expansion
        node.ensure_loaded(storage, context).await?;

        // System fields
        result.push((format!("{}_id", var), SqlValue::String(node.id.clone())));
        result.push((
            format!("{}_workspace", var),
            SqlValue::String(node.workspace.clone()),
        ));
        result.push((
            format!("{}_node_type", var),
            SqlValue::String(node.node_type.clone()),
        ));

        // Loaded fields
        if let Some(path) = node.path() {
            result.push((format!("{}_path", var), SqlValue::String(path.into())));
        }
        if let Some(name) = node.name() {
            result.push((format!("{}_name", var), SqlValue::String(name.into())));
        }
    } else if let Some(rel) = binding.get_relation(var) {
        result.push((
            format!("{}_type", var),
            SqlValue::String(rel.relation_type.clone()),
        ));
        result.push((format!("{}_weight", var), rel.weight.into()));
    } else {
        return Err(ExecutionError::Validation(format!(
            "Unknown variable for wildcard: {}",
            var
        )));
    }
    Ok(())
}

/// Expand wildcard columns for a variable (sync version for tests)
///
/// Returns columns for all system fields and properties.
#[allow(dead_code)]
pub fn expand_wildcard(
    qualifier: Option<&str>,
    binding: &VariableBinding,
) -> Result<Vec<(String, SqlValue)>> {
    let mut result = Vec::new();

    match qualifier {
        Some(var) => {
            // Expand specific variable: n.*
            if let Some(node) = binding.get_node(var) {
                // System fields
                result.push((format!("{}_id", var), SqlValue::String(node.id.clone())));
                result.push((
                    format!("{}_workspace", var),
                    SqlValue::String(node.workspace.clone()),
                ));
                result.push((
                    format!("{}_node_type", var),
                    SqlValue::String(node.node_type.clone()),
                ));

                // If loaded, add more fields
                if node.is_loaded() {
                    if let Some(path) = node.path() {
                        result.push((format!("{}_path", var), SqlValue::String(path.into())));
                    }
                    if let Some(name) = node.name() {
                        result.push((format!("{}_name", var), SqlValue::String(name.into())));
                    }
                }
            } else if let Some(rel) = binding.get_relation(var) {
                result.push((
                    format!("{}_type", var),
                    SqlValue::String(rel.relation_type.clone()),
                ));
                result.push((format!("{}_weight", var), rel.weight.into()));
            } else {
                return Err(ExecutionError::Validation(format!(
                    "Unknown variable for wildcard: {}",
                    var
                )));
            }
        }
        None => {
            // Expand all variables: *
            for var in binding.node_vars() {
                let expanded = expand_wildcard(Some(var), binding)?;
                result.extend(expanded);
            }
            for var in binding.relation_vars() {
                let expanded = expand_wildcard(Some(var), binding)?;
                result.extend(expanded);
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical_plan::pgq::types::{NodeInfo, RelationInfo};
    use raisin_sql::ast::SourceSpan;

    #[test]
    fn test_get_column_name() {
        // With alias
        let col = ColumnExpr {
            expr: Expr::PropertyAccess {
                variable: "n".into(),
                properties: vec!["name".into()],
                span: SourceSpan::empty(),
            },
            alias: Some("user_name".into()),
            span: SourceSpan::empty(),
        };
        assert_eq!(get_column_name(&col), "user_name");

        // Without alias
        let col = ColumnExpr {
            expr: Expr::PropertyAccess {
                variable: "n".into(),
                properties: vec!["name".into()],
                span: SourceSpan::empty(),
            },
            alias: None,
            span: SourceSpan::empty(),
        };
        assert_eq!(get_column_name(&col), "n_name");

        // Function
        let col = ColumnExpr {
            expr: Expr::FunctionCall {
                name: "COUNT".into(),
                args: vec![],
                distinct: false,
                span: SourceSpan::empty(),
            },
            alias: None,
            span: SourceSpan::empty(),
        };
        assert_eq!(get_column_name(&col), "count");
    }

    #[test]
    fn test_expand_wildcard() {
        let mut binding = VariableBinding::new();
        binding.bind_node(
            "n".into(),
            NodeInfo::new("node-1".into(), "ws".into(), "User".into()),
        );
        binding.bind_relation(
            "r".into(),
            RelationInfo::new("FOLLOWS".into(), Some(0.9), "n".into(), "m".into()),
        );

        // Expand specific variable
        let expanded = expand_wildcard(Some("n"), &binding).unwrap();
        assert!(expanded.iter().any(|(k, _)| k == "n_id"));
        assert!(expanded.iter().any(|(k, _)| k == "n_workspace"));

        // Expand all
        let expanded = expand_wildcard(None, &binding).unwrap();
        assert!(expanded.iter().any(|(k, _)| k == "n_id"));
        assert!(expanded.iter().any(|(k, _)| k == "r_type"));
    }
}
