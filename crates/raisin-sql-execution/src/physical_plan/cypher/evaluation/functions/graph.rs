//! Graph analysis functions for Cypher
//!
//! Provides basic graph metrics like degree centrality for individual nodes.

use raisin_cypher_parser::Expr;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{RelationRepository, Storage};

use super::super::expr::evaluate_expr_async_impl;
use super::traits::FunctionContext;
use crate::physical_plan::cypher::types::VariableBinding;

/// Helper: Extract node ID and workspace from PropertyValue
fn extract_node_id_workspace(node_value: &PropertyValue) -> Result<(String, String), Error> {
    match node_value {
        PropertyValue::Object(ref map) => {
            let id = map
                .get("id")
                .and_then(|v| match v {
                    PropertyValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    Error::Validation("Node object must have an 'id' field".to_string())
                })?;

            let workspace = map
                .get("workspace")
                .and_then(|v| match v {
                    PropertyValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    Error::Validation("Node object must have a 'workspace' field".to_string())
                })?;

            Ok((id, workspace))
        }
        _ => Err(Error::Validation(
            "Degree functions require a node object as argument".to_string(),
        )),
    }
}

/// degree(node) - Count all relationships (in + out) for a node
///
/// Returns the total number of relationships connected to the node.
///
/// # Arguments
///
/// * `args` - Must contain exactly 1 expression: the node variable
/// * `binding` - Current variable binding for evaluating expressions
/// * `context` - Function evaluation context with storage access
///
/// # Returns
///
/// Number representing total degree (in + out)
///
/// # Errors
///
/// Returns Error::Validation if:
/// - Wrong number of arguments (not 1)
/// - Argument is not a node object
/// - Node object missing id or workspace fields
///
/// Returns Error::Backend if:
/// - Storage operation fails
///
/// # Example
///
/// ```cypher
/// MATCH (n)
/// RETURN n.id, degree(n) AS totalDegree
/// ```
pub async fn evaluate_degree<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if args.len() != 1 {
        return Err(Error::Validation(format!(
            "degree() requires exactly 1 argument (node variable), got {}",
            args.len()
        )));
    }

    let node_value = evaluate_expr_async_impl(&args[0], binding, context).await?;
    let (node_id, workspace) = extract_node_id_workspace(&node_value)?;

    tracing::debug!("   degree({}:{}) called", workspace, node_id);

    let out_count = context
        .storage
        .relations()
        .get_outgoing_relations(
            raisin_storage::StorageScope::new(
                context.tenant_id,
                context.repo_id,
                context.branch,
                &workspace,
            ),
            &node_id,
            context.revision,
        )
        .await
        .map_err(|e| Error::Backend(e.to_string()))?
        .len();

    let in_count = context
        .storage
        .relations()
        .get_incoming_relations(
            raisin_storage::StorageScope::new(
                context.tenant_id,
                context.repo_id,
                context.branch,
                &workspace,
            ),
            &node_id,
            context.revision,
        )
        .await
        .map_err(|e| Error::Backend(e.to_string()))?
        .len();

    let total = in_count + out_count;
    tracing::debug!("   ✓ degree() = {}", total);

    Ok(PropertyValue::Integer(total as i64))
}

/// inDegree(node) - Count incoming relationships for a node
///
/// Returns the number of incoming relationships to the node.
///
/// # Arguments
///
/// * `args` - Must contain exactly 1 expression: the node variable
/// * `binding` - Current variable binding for evaluating expressions
/// * `context` - Function evaluation context with storage access
///
/// # Returns
///
/// Number representing in-degree
///
/// # Errors
///
/// Returns Error::Validation if:
/// - Wrong number of arguments (not 1)
/// - Argument is not a node object
/// - Node object missing id or workspace fields
///
/// Returns Error::Backend if:
/// - Storage operation fails
///
/// # Example
///
/// ```cypher
/// MATCH (n)
/// RETURN n.id, inDegree(n) AS incomingLinks
/// ```
pub async fn evaluate_indegree<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if args.len() != 1 {
        return Err(Error::Validation(format!(
            "inDegree() requires exactly 1 argument (node variable), got {}",
            args.len()
        )));
    }

    let node_value = evaluate_expr_async_impl(&args[0], binding, context).await?;
    let (node_id, workspace) = extract_node_id_workspace(&node_value)?;

    tracing::debug!("   inDegree({}:{}) called", workspace, node_id);

    let in_count = context
        .storage
        .relations()
        .get_incoming_relations(
            raisin_storage::StorageScope::new(
                context.tenant_id,
                context.repo_id,
                context.branch,
                &workspace,
            ),
            &node_id,
            context.revision,
        )
        .await
        .map_err(|e| Error::Backend(e.to_string()))?
        .len();

    tracing::debug!("   ✓ inDegree() = {}", in_count);

    Ok(PropertyValue::Integer(in_count as i64))
}

/// outDegree(node) - Count outgoing relationships for a node
///
/// Returns the number of outgoing relationships from the node.
///
/// # Arguments
///
/// * `args` - Must contain exactly 1 expression: the node variable
/// * `binding` - Current variable binding for evaluating expressions
/// * `context` - Function evaluation context with storage access
///
/// # Returns
///
/// Number representing out-degree
///
/// # Errors
///
/// Returns Error::Validation if:
/// - Wrong number of arguments (not 1)
/// - Argument is not a node object
/// - Node object missing id or workspace fields
///
/// Returns Error::Backend if:
/// - Storage operation fails
///
/// # Example
///
/// ```cypher
/// MATCH (n)
/// RETURN n.id, outDegree(n) AS outgoingLinks
/// ```
pub async fn evaluate_outdegree<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if args.len() != 1 {
        return Err(Error::Validation(format!(
            "outDegree() requires exactly 1 argument (node variable), got {}",
            args.len()
        )));
    }

    let node_value = evaluate_expr_async_impl(&args[0], binding, context).await?;
    let (node_id, workspace) = extract_node_id_workspace(&node_value)?;

    tracing::debug!("   outDegree({}:{}) called", workspace, node_id);

    let out_count = context
        .storage
        .relations()
        .get_outgoing_relations(
            raisin_storage::StorageScope::new(
                context.tenant_id,
                context.repo_id,
                context.branch,
                &workspace,
            ),
            &node_id,
            context.revision,
        )
        .await
        .map_err(|e| Error::Backend(e.to_string()))?
        .len();

    tracing::debug!("   ✓ outDegree() = {}", out_count);

    Ok(PropertyValue::Integer(out_count as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_node_id_workspace() {
        // This test just ensures the helper function signature is correct
        // Full testing requires a mock PropertyValue
    }
}
