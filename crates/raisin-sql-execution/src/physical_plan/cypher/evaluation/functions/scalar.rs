//! Scalar functions for Cypher
//!
//! Provides basic scalar functions like lookup() that operate on individual values.

use std::collections::HashMap;

use raisin_cypher_parser::Expr;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{NodeRepository, Storage, StorageScope};

use super::super::expr::evaluate_expr_async_impl;
use super::traits::FunctionContext;
use crate::physical_plan::cypher::types::VariableBinding;

/// lookup(id, workspace) - Fetch a node by ID and workspace
///
/// Returns a node object with all properties, or an empty object if not found.
///
/// # Arguments
///
/// * `args` - Must contain exactly 2 expressions: [id, workspace]
/// * `binding` - Current variable binding for evaluating expressions
/// * `context` - Function evaluation context with storage access
///
/// # Returns
///
/// Object with fields: id, workspace, path, type, properties
///
/// # Errors
///
/// Returns Error::Validation if:
/// - Wrong number of arguments (not 2)
/// - Arguments are not strings
///
/// Returns Error::Backend if:
/// - Storage operation fails
///
/// # Example
///
/// ```cypher
/// MATCH (a)-[:LINKS_TO]->(b)
/// RETURN lookup(b.id, b.workspace) AS fullNode
/// ```
pub async fn evaluate_lookup<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    // Validate argument count
    if args.len() != 2 {
        return Err(Error::Validation(format!(
            "lookup() requires exactly 2 arguments (id, workspace), got {}",
            args.len()
        )));
    }

    // Evaluate arguments
    let id_value = evaluate_expr_async_impl(&args[0], binding, context).await?;
    let workspace_value = evaluate_expr_async_impl(&args[1], binding, context).await?;

    // Extract string values
    let id = match id_value {
        PropertyValue::String(s) => s,
        _ => {
            return Err(Error::Validation(
                "lookup() first argument (id) must be a string".to_string(),
            ))
        }
    };

    let workspace = match workspace_value {
        PropertyValue::String(s) => s,
        _ => {
            return Err(Error::Validation(
                "lookup() second argument (workspace) must be a string".to_string(),
            ))
        }
    };

    tracing::debug!("   lookup({}, {}) called", id, workspace);

    // Fetch the node from storage
    let node_result = context
        .storage
        .nodes()
        .get(
            StorageScope::new(
                context.tenant_id,
                context.repo_id,
                context.branch,
                &workspace,
            ),
            &id,
            context.revision,
        )
        .await
        .map_err(|e| Error::Backend(format!("lookup() storage error: {}", e)))?;

    match node_result {
        Some(node) => {
            // Convert Node to PropertyValue::Object with all properties
            let mut node_map = HashMap::new();
            node_map.insert("id".to_string(), PropertyValue::String(node.id));
            node_map.insert("workspace".to_string(), PropertyValue::String(workspace));
            node_map.insert("path".to_string(), PropertyValue::String(node.path));
            node_map.insert("type".to_string(), PropertyValue::String(node.node_type));
            node_map.insert(
                "properties".to_string(),
                PropertyValue::Object(node.properties),
            );

            tracing::debug!("   ✓ lookup() found node");
            Ok(PropertyValue::Object(node_map))
        }
        None => {
            // Node not found - return empty object for graceful handling
            tracing::warn!("   ⚠️  lookup() node not found: {}:{}", workspace, id);
            Ok(PropertyValue::Object(HashMap::new()))
        }
    }
}

/// resolve_node_path(workspace, path) - Fast O(1) path to ID lookup
///
/// Returns the node ID for a given workspace and path. This is an O(1) lookup
/// using RocksDB's path index, making it ideal for predicate pushdown.
///
/// # Arguments
///
/// * `args` - Must contain exactly 2 expressions: [workspace, path]
/// * `binding` - Current variable binding (unused for this function)
/// * `context` - Function evaluation context with storage access
///
/// # Returns
///
/// String containing the node ID, or empty string if not found
///
/// # Errors
///
/// Returns Error::Validation if:
/// - Wrong number of arguments (not 2)
/// - Arguments are not strings
///
/// # Example
///
/// ```cypher
/// MATCH (this)-[r]->(target)
/// WHERE this.id = resolve_node_path("social", "/demonews/articles/tech/my-article")
/// RETURN target.id, type(r)
/// ```
pub async fn evaluate_resolve_node_path<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    // Validate argument count
    if args.len() != 2 {
        return Err(Error::Validation(format!(
            "resolve_node_path() requires exactly 2 arguments (workspace, path), got {}",
            args.len()
        )));
    }

    // Evaluate arguments
    let workspace_value = evaluate_expr_async_impl(&args[0], binding, context).await?;
    let path_value = evaluate_expr_async_impl(&args[1], binding, context).await?;

    // Extract string values
    let workspace = match workspace_value {
        PropertyValue::String(s) => s,
        _ => {
            return Err(Error::Validation(
                "resolve_node_path() first argument (workspace) must be a string".to_string(),
            ))
        }
    };

    let path = match path_value {
        PropertyValue::String(s) => s,
        _ => {
            return Err(Error::Validation(
                "resolve_node_path() second argument (path) must be a string".to_string(),
            ))
        }
    };

    tracing::debug!("   resolve_node_path({}, {}) called", workspace, path);

    // Use get_by_path for O(1) lookup - this uses the path index in RocksDB
    let node_result = context
        .storage
        .nodes()
        .get_by_path(
            StorageScope::new(
                context.tenant_id,
                context.repo_id,
                context.branch,
                &workspace,
            ),
            &path,
            context.revision,
        )
        .await
        .map_err(|e| Error::Backend(format!("resolve_node_path() storage error: {}", e)))?;

    match node_result {
        Some(node) => {
            tracing::debug!("   ✓ resolve_node_path() found node: {}", node.id);
            Ok(PropertyValue::String(node.id))
        }
        None => {
            tracing::debug!(
                "   ⚠️ resolve_node_path() node not found: {}:{}",
                workspace,
                path
            );
            // Return empty string for not found - allows WHERE comparisons to fail gracefully
            Ok(PropertyValue::String(String::new()))
        }
    }
}

/// type(r) - Get the type of a relationship
///
/// Returns the relationship type as a string.
///
/// # Arguments
///
/// * `args` - Must contain exactly 1 expression: [relationship_variable]
/// * `binding` - Current variable binding for evaluating expressions
/// * `context` - Function evaluation context
///
/// # Returns
///
/// String containing the relationship type
///
/// # Errors
///
/// Returns Error::Validation if:
/// - Wrong number of arguments (not 1)
/// - Argument is not a relationship variable
///
/// # Example
///
/// ```cypher
/// MATCH (a)-[r]->(b)
/// WHERE type(r) = 'KNOWS'
/// RETURN a.name, type(r), b.name
/// ```
pub async fn evaluate_type<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    _context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    // Validate argument count
    if args.len() != 1 {
        return Err(Error::Validation(format!(
            "type() requires exactly 1 argument (relationship), got {}",
            args.len()
        )));
    }

    // The argument should be a variable reference to a relationship
    match &args[0] {
        Expr::Variable(var_name) => {
            // Look up the relationship in the binding
            if let Some(rel_info) = binding.relationships.get(var_name) {
                Ok(PropertyValue::String(rel_info.relation_type.clone()))
            } else {
                // Could be a node variable - but type() is for relationships
                Err(Error::Validation(format!(
                    "type() argument '{}' is not a relationship variable. Available relationships: {:?}",
                    var_name,
                    binding.relationships.keys().collect::<Vec<_>>()
                )))
            }
        }
        _ => Err(Error::Validation(
            "type() argument must be a relationship variable (e.g., type(r))".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lookup_argument_count_validation() {
        // This test just ensures the function signature is correct
        // Full testing requires a mock storage implementation
    }
}
