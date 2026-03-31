//! Compound index loading and node_type extraction helpers.
//!
//! Provides utility functions for extracting node_type from WHERE clauses
//! and loading compound index definitions from NodeType storage.

use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_sql::analyzer::{AnalyzedStatement, BinaryOperator, Expr, Literal, TypedExpr};
use raisin_storage::{NodeTypeRepository, Storage};

/// Extract node_type value from analyzed query's WHERE clause
///
/// Searches the WHERE clause for `node_type = 'typename'` conditions
/// so we can load compound indexes for the matching NodeType.
pub(super) fn extract_node_type_from_analyzed(analyzed: &AnalyzedStatement) -> Option<String> {
    if let AnalyzedStatement::Query(ref q) = analyzed {
        if let Some(ref filter) = q.selection {
            return extract_node_type_from_expr(filter);
        }
    }
    None
}

/// Recursively search a TypedExpr for `node_type = 'value'` pattern
pub(super) fn extract_node_type_from_expr(expr: &TypedExpr) -> Option<String> {
    match &expr.expr {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => {
            // Check: node_type = 'X'
            if let Expr::Column { column, .. } = &left.expr {
                if column.eq_ignore_ascii_case("node_type") {
                    if let Expr::Literal(Literal::Text(s)) = &right.expr {
                        return Some(s.clone());
                    }
                }
            }
            // Check reversed: 'X' = node_type
            if let Expr::Column { column, .. } = &right.expr {
                if column.eq_ignore_ascii_case("node_type") {
                    if let Expr::Literal(Literal::Text(s)) = &left.expr {
                        return Some(s.clone());
                    }
                }
            }
            None
        }
        // Recurse into AND/OR expressions
        Expr::BinaryOp { left, right, .. } => {
            extract_node_type_from_expr(left).or_else(|| extract_node_type_from_expr(right))
        }
        _ => None,
    }
}

/// Load compound indexes from NodeType storage
///
/// Fetches the NodeType definition and extracts its compound_indexes field.
pub(super) async fn load_compound_indexes<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_type_name: &str,
) -> Option<Vec<CompoundIndexDefinition>> {
    match storage
        .node_types()
        .get(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            node_type_name,
            None,
        )
        .await
    {
        Ok(Some(node_type)) => {
            if let Some(ref indexes) = node_type.compound_indexes {
                if !indexes.is_empty() {
                    tracing::debug!(
                        "   Loaded {} compound indexes for '{}'",
                        indexes.len(),
                        node_type_name
                    );
                    return Some(indexes.clone());
                }
            }
            None
        }
        Ok(None) => {
            tracing::debug!("   NodeType '{}' not found", node_type_name);
            None
        }
        Err(e) => {
            tracing::warn!("   Failed to load NodeType '{}': {}", node_type_name, e);
            None
        }
    }
}
