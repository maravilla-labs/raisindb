//! Index Lookup Join Executor
//!
//! Implements nested loop join with O(1) index lookups on the inner side.
//! This is optimal when joining on indexed columns (id, path) with a small outer input.
//!
//! Algorithm:
//! 1. For each row from the outer input:
//!    2. Extract the join key value
//!    3. Perform O(1) index lookup (NodeIdScan or PathIndexScan)
//!    4. Merge and output matching rows
//!
//! Complexity: O(n) where n = outer rows (each lookup is O(1))
//! Memory: O(1) - no hash table needed, streaming execution

use super::executor::{ExecutionContext, ExecutionError, Row, RowStream};
use super::operators::{IndexLookupType, PhysicalPlan};
use super::scan_executors::node_to_row;
use futures::stream::StreamExt;
use indexmap::IndexMap;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::JoinType;
use raisin_storage::{NodeRepository, Storage, StorageScope};

/// Execute an IndexLookupJoin
///
/// For each row from the outer input, performs an O(1) index lookup on the inner side.
/// This avoids the full table scan that would occur with HashJoin.
pub async fn execute_index_lookup_join<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (outer, join_type, outer_key_column, inner_lookup) = match plan {
        PhysicalPlan::IndexLookupJoin {
            outer,
            join_type,
            outer_key_column,
            inner_lookup,
        } => (outer, join_type, outer_key_column, inner_lookup),
        _ => {
            return Err(ExecutionError::Backend(
                "Invalid plan passed to execute_index_lookup_join".to_string(),
            ))
        }
    };

    // Only INNER and LEFT joins are supported
    if !matches!(join_type, JoinType::Inner | JoinType::Left) {
        return Err(ExecutionError::Validation(format!(
            "IndexLookupJoin only supports INNER and LEFT joins, got {:?}",
            join_type
        )));
    }

    // Execute outer input
    let outer_stream = super::executor::execute_plan(outer.as_ref(), ctx).await?;

    // Clone values we need for the async stream
    let join_type = join_type.clone();
    let outer_key_column = outer_key_column.clone();
    let inner_lookup = inner_lookup.clone();
    let storage = ctx.storage.clone();
    let max_revision = ctx.max_revision;
    let ctx_clone = ctx.clone();

    // Collect outer rows to process them
    let outer_rows: Vec<Row> = outer_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    tracing::debug!(
        "IndexLookupJoin: Processing {} outer rows with {} lookup on {}",
        outer_rows.len(),
        match inner_lookup.lookup_type {
            IndexLookupType::ById => "id",
            IndexLookupType::ByPath => "path",
        },
        inner_lookup.table
    );

    // Process each outer row and perform index lookups
    let mut output_rows = Vec::new();

    for outer_row in outer_rows {
        // Extract the join key from the outer row
        tracing::debug!(
            "IndexLookupJoin: Extracting column '{}' from row with columns: {:?}",
            outer_key_column,
            outer_row.columns.keys().collect::<Vec<_>>()
        );
        let key_value = extract_key_from_row(&outer_row, &outer_key_column);

        let key_str = match key_value {
            Some(PropertyValue::String(s)) => {
                tracing::debug!("IndexLookupJoin: Extracted key value: '{}'", s);
                s
            }
            Some(other) => {
                // Try to convert to string
                let s = format!("{:?}", other);
                tracing::debug!(
                    "IndexLookupJoin: Extracted non-string key value: {:?} -> '{}'",
                    other,
                    s
                );
                s
            }
            None => {
                tracing::warn!(
                    "IndexLookupJoin: Key column '{}' not found in outer row. Available columns: {:?}",
                    outer_key_column,
                    outer_row.columns.keys().collect::<Vec<_>>()
                );
                // Key not found in outer row - for LEFT join, emit outer row with no match
                if matches!(join_type, JoinType::Left) {
                    output_rows.push(outer_row);
                }
                continue;
            }
        };

        // Perform the index lookup
        tracing::debug!(
            "IndexLookupJoin: Looking up node by id='{}' in workspace='{}', branch='{}', table='{}'",
            key_str,
            inner_lookup.workspace,
            inner_lookup.branch,
            inner_lookup.table
        );
        let inner_row = match inner_lookup.lookup_type {
            IndexLookupType::ById => {
                lookup_by_id(
                    &storage,
                    &inner_lookup.tenant_id,
                    &inner_lookup.repo_id,
                    &inner_lookup.branch,
                    &inner_lookup.workspace,
                    &key_str,
                    max_revision.as_ref(),
                    inner_lookup.alias.as_ref().unwrap_or(&inner_lookup.table),
                    &inner_lookup.projection,
                    &ctx_clone,
                )
                .await?
            }
            IndexLookupType::ByPath => {
                lookup_by_path(
                    &storage,
                    &inner_lookup.tenant_id,
                    &inner_lookup.repo_id,
                    &inner_lookup.branch,
                    &inner_lookup.workspace,
                    &key_str,
                    max_revision.as_ref(),
                    inner_lookup.alias.as_ref().unwrap_or(&inner_lookup.table),
                    &inner_lookup.projection,
                    &ctx_clone,
                )
                .await?
            }
        };

        match inner_row {
            Some(inner) => {
                // Match found - merge rows
                output_rows.push(merge_rows(&outer_row, &inner));
            }
            None => {
                // No match - for LEFT join, emit outer row only
                if matches!(join_type, JoinType::Left) {
                    output_rows.push(outer_row);
                }
                // For INNER join, skip this row
            }
        }
    }

    tracing::debug!(
        "IndexLookupJoin: Produced {} output rows",
        output_rows.len()
    );

    // Convert Vec<Row> to stream
    Ok(Box::pin(futures::stream::iter(
        output_rows.into_iter().map(Ok),
    )))
}

/// Extract key value from a row by column name
///
/// Tries multiple column name formats:
/// - Exact match: "column_name"
/// - Qualified: "qualifier.column_name"
/// - Case-insensitive matching
fn extract_key_from_row(row: &Row, column: &str) -> Option<PropertyValue> {
    // Try exact match first
    if let Some(value) = row.get(column) {
        return Some(value.clone());
    }

    // Try to find the column with any qualifier
    let column_lower = column.to_lowercase();
    for (key, value) in &row.columns {
        // Check if key ends with the column name (handles "table.column" format)
        let key_lower = key.to_lowercase();
        if key_lower == column_lower {
            return Some(value.clone());
        }
        if key_lower.ends_with(&format!(".{}", column_lower)) {
            return Some(value.clone());
        }
        // Also check for underscore separator (e.g., "table_column")
        if key_lower.ends_with(&format!("_{}", column_lower)) {
            return Some(value.clone());
        }
    }

    None
}

/// Perform O(1) node lookup by ID
async fn lookup_by_id<S: Storage>(
    storage: &std::sync::Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    max_revision: Option<&raisin_hlc::HLC>,
    qualifier: &str,
    projection: &Option<Vec<String>>,
    ctx: &ExecutionContext<S>,
) -> Result<Option<Row>, ExecutionError> {
    tracing::debug!(
        "lookup_by_id: tenant='{}', repo='{}', branch='{}', workspace='{}', node_id='{}'",
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id
    );
    let node_opt = storage
        .nodes()
        .get(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            node_id,
            max_revision,
        )
        .await
        .map_err(|e| ExecutionError::Backend(format!("IndexLookupJoin lookup error: {}", e)))?;

    match node_opt {
        Some(node) => {
            tracing::debug!(
                "lookup_by_id: Found node at path='{}' with id='{}'",
                node.path,
                node.id
            );
            // Skip root nodes
            if node.path == "/" {
                tracing::debug!("lookup_by_id: Skipping root node");
                return Ok(None);
            }
            // Convert node to row
            let row = node_to_row(&node, qualifier, workspace, projection, ctx, "default")
                .await
                .map_err(|e| {
                    ExecutionError::Backend(format!("IndexLookupJoin node_to_row error: {}", e))
                })?;
            Ok(Some(row))
        }
        None => {
            tracing::debug!("lookup_by_id: Node not found with id='{}'", node_id);
            Ok(None)
        }
    }
}

/// Perform O(1) node lookup by path
async fn lookup_by_path<S: Storage>(
    storage: &std::sync::Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    path: &str,
    max_revision: Option<&raisin_hlc::HLC>,
    qualifier: &str,
    projection: &Option<Vec<String>>,
    ctx: &ExecutionContext<S>,
) -> Result<Option<Row>, ExecutionError> {
    let node_opt = storage
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            path,
            max_revision,
        )
        .await
        .map_err(|e| {
            ExecutionError::Backend(format!("IndexLookupJoin path lookup error: {}", e))
        })?;

    match node_opt {
        Some(node) => {
            // Skip root nodes
            if node.path == "/" {
                return Ok(None);
            }
            // Convert node to row
            let row = node_to_row(&node, qualifier, workspace, projection, ctx, "default")
                .await
                .map_err(|e| {
                    ExecutionError::Backend(format!("IndexLookupJoin node_to_row error: {}", e))
                })?;
            Ok(Some(row))
        }
        None => Ok(None),
    }
}

/// Merge two rows into one
fn merge_rows(left: &Row, right: &Row) -> Row {
    let mut merged = IndexMap::new();

    // Add all left columns
    for (k, v) in &left.columns {
        merged.insert(k.clone(), v.clone());
    }

    // Add all right columns (may overwrite if same column name)
    for (k, v) in &right.columns {
        merged.insert(k.clone(), v.clone());
    }

    Row::from_map(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_key_from_row() {
        let mut columns = IndexMap::new();
        columns.insert(
            "target_id".to_string(),
            PropertyValue::String("abc123".to_string()),
        );
        columns.insert(
            "related.source_id".to_string(),
            PropertyValue::String("def456".to_string()),
        );
        let row = Row::from_map(columns);

        // Exact match
        assert_eq!(
            extract_key_from_row(&row, "target_id"),
            Some(PropertyValue::String("abc123".to_string()))
        );

        // Qualified match
        assert_eq!(
            extract_key_from_row(&row, "source_id"),
            Some(PropertyValue::String("def456".to_string()))
        );

        // Not found
        assert_eq!(extract_key_from_row(&row, "nonexistent"), None);
    }

    #[test]
    fn test_merge_rows() {
        let mut left_cols = IndexMap::new();
        left_cols.insert("id".to_string(), PropertyValue::Integer(1));
        left_cols.insert(
            "name".to_string(),
            PropertyValue::String("Alice".to_string()),
        );
        let left = Row::from_map(left_cols);

        let mut right_cols = IndexMap::new();
        right_cols.insert("city".to_string(), PropertyValue::String("NYC".to_string()));
        right_cols.insert("age".to_string(), PropertyValue::Integer(30));
        let right = Row::from_map(right_cols);

        let merged = merge_rows(&left, &right);

        assert_eq!(merged.columns.len(), 4);
        assert_eq!(merged.get("id"), Some(&PropertyValue::Integer(1)));
        assert_eq!(
            merged.get("name"),
            Some(&PropertyValue::String("Alice".to_string()))
        );
        assert_eq!(
            merged.get("city"),
            Some(&PropertyValue::String("NYC".to_string()))
        );
        assert_eq!(merged.get("age"), Some(&PropertyValue::Integer(30)));
    }
}
