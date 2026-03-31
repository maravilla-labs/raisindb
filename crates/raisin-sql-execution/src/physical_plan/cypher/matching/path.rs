//! Path pattern matching
//!
//! Handles matching of path patterns (sequences of nodes and relationships).

use std::collections::HashSet;
use std::sync::Arc;

use raisin_cypher_parser::{BinOp, Expr, Literal, NodePattern, PatternElement, UnOp};
use raisin_storage::{ListOptions, NodeRepository, Storage, StorageScope};

use crate::physical_plan::cypher::types::{CypherContext, NodeInfo, VariableBinding};
use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Match a path pattern and return new bindings
///
/// Path patterns are sequences of nodes and relationships: (a)-[r]->(b)-[s]->(c)
/// Elements alternate: Node, Relationship, Node, Relationship, Node, ...
///
/// NEW APPROACH: Uses global relationship index for efficient cross-workspace queries
///
/// # Arguments
/// * `path` - The path pattern to match
/// * `binding` - Current variable binding to extend
/// * `storage` - Storage backend
/// * `context` - Cypher execution context
///
/// # Returns
/// New bindings with matched paths
pub async fn match_path_pattern<S: Storage>(
    path: &raisin_cypher_parser::PathPattern,
    graph_where_clause: Option<&Expr>,
    binding: VariableBinding,
    storage: &Arc<S>,
    context: &CypherContext,
) -> Result<Vec<VariableBinding>> {
    if path.elements.is_empty() {
        return Ok(vec![binding]);
    }

    // For single node pattern (a), scan all nodes with optional filtering
    // This is needed for graph algorithm queries like: MATCH (n) RETURN pageRank(n)
    if path.elements.len() == 1 {
        if let PatternElement::Node(node_pattern) = &path.elements[0] {
            return super::node::match_node_pattern(node_pattern, vec![binding], storage, context)
                .await;
        }
    }

    // For simple pattern (a)-[:TYPE]->(b), use global relationship scan
    // This is the most common case and most optimized path
    if path.elements.len() == 3 {
        if let (
            PatternElement::Node(source_pattern),
            PatternElement::Relationship(rel_pattern),
            PatternElement::Node(target_pattern),
        ) = (&path.elements[0], &path.elements[1], &path.elements[2])
        {
            // Check if this is a variable-length pattern
            if let Some(range) = &rel_pattern.range {
                tracing::info!(
                    "🔵 Variable-length pattern detected: *{:?}..{:?}",
                    range.min,
                    range.max
                );
                return super::variable_length::execute_variable_length_pattern(
                    source_pattern,
                    rel_pattern,
                    target_pattern,
                    vec![binding],
                    storage,
                    context,
                )
                .await;
            }

            // Check if source node is already bound (e.g. from a previous MATCH or Path Index lookup)
            // If source is bound, we should use vectorized expansion from that source
            // instead of a global scan.
            let source_var = source_pattern.variable.as_deref();

            if let Some(bound_var) = source_var.filter(|v| binding.has_node(v)) {
                tracing::info!(
                    "🔵 Source node '{}' is bound - using Vectorized Expansion",
                    bound_var
                );
                return super::relationship::match_relationship_pattern(
                    rel_pattern,
                    bound_var,
                    target_pattern,
                    vec![binding],
                    storage,
                    context,
                )
                .await;
            }

            // If source is NOT bound, attempt to seed bindings via Path Index constraints
            if let Some(source_var_name) = source_var {
                if let Some(seeded_bindings) = seed_frontier_from_path_filters(
                    source_pattern,
                    graph_where_clause,
                    &binding,
                    storage,
                    context,
                )
                .await?
                {
                    tracing::info!(
                        "🔵 Path-First Traversal: seeded {} start nodes for '{}'",
                        seeded_bindings.len(),
                        source_var_name
                    );

                    if seeded_bindings.is_empty() {
                        tracing::info!(
                            "   Path constraints produced no matches for '{}'",
                            source_var_name
                        );
                        return Ok(vec![]);
                    }

                    return super::relationship::match_relationship_pattern(
                        rel_pattern,
                        source_var_name,
                        target_pattern,
                        seeded_bindings,
                        storage,
                        context,
                    )
                    .await;
                }
            }

            // If no path constraints were found, fall back to global scan
            // Pass the WHERE clause so it can be applied after global scan
            return super::relationship::match_simple_relationship_pattern(
                source_pattern,
                rel_pattern,
                target_pattern,
                vec![binding],
                graph_where_clause,
                storage,
                context,
            )
            .await;
        }
    }

    // For multi-hop patterns (a)-[r]->(b)-[s]->(c), process sequentially
    // TODO: Implement multi-hop optimization
    Err(ExecutionError::Validation(
        "Multi-hop relationship patterns not yet implemented. Use simple patterns like (a)-[:TYPE]->(b)".to_string(),
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathConstraint {
    Exact(String),
    Prefix(String),
}

async fn seed_frontier_from_path_filters<S: Storage>(
    source_pattern: &NodePattern,
    graph_where_clause: Option<&Expr>,
    base_binding: &VariableBinding,
    storage: &Arc<S>,
    context: &CypherContext,
) -> Result<Option<Vec<VariableBinding>>> {
    let source_var = match &source_pattern.variable {
        Some(var) => var,
        None => return Ok(None),
    };

    let mut constraints = Vec::new();

    if let Some(property_constraint) = path_constraint_from_properties(source_pattern) {
        constraints.push(property_constraint);
    }

    if let Some(expr) = source_pattern.where_clause.as_ref() {
        collect_path_constraints_from_expr(expr, source_var, &mut constraints);
    }

    if let Some(expr) = graph_where_clause {
        collect_path_constraints_from_expr(expr, source_var, &mut constraints);
    }

    if constraints.is_empty() {
        return Ok(None);
    }

    let (mut exact_paths, mut prefix_paths) = partition_constraints(constraints);

    if exact_paths.is_empty() && prefix_paths.is_empty() {
        return Ok(None);
    }

    let mut candidate_nodes = Vec::new();

    if !exact_paths.is_empty() {
        for path in exact_paths.drain(..) {
            if let Some(node) = fetch_node_by_path(storage, context, &path).await? {
                candidate_nodes.push(node);
            }
        }
    } else {
        let list_options = list_options_for_context(context);
        for prefix in prefix_paths.drain(..) {
            let mut nodes = storage
                .nodes()
                .scan_by_path_prefix(
                    StorageScope::new(
                        &context.tenant_id,
                        &context.repo_id,
                        &context.branch,
                        &context.workspace_id,
                    ),
                    &prefix,
                    list_options.clone(),
                )
                .await
                .map_err(|e| ExecutionError::Backend(e.to_string()))?;
            candidate_nodes.append(&mut nodes);
        }
    }

    if candidate_nodes.is_empty() {
        // No nodes found at the specified path in this workspace.
        // Return None to trigger global scan fallback, which correctly
        // handles cross-workspace queries where the node may exist in
        // a different workspace than context.workspace_id.
        tracing::info!("   Path lookup found no nodes - falling back to global scan");
        return Ok(None);
    }

    let mut seen = HashSet::new();
    let mut seeded_bindings = Vec::new();

    for node in candidate_nodes {
        if !super::node::node_matches_pattern(&node, source_pattern) {
            continue;
        }

        let workspace = node
            .workspace
            .clone()
            .unwrap_or_else(|| context.workspace_id.clone());
        let key = (workspace.clone(), node.id.clone());
        if !seen.insert(key) {
            continue;
        }

        let node_info = NodeInfo {
            id: node.id.clone(),
            path: node.path.clone(),
            node_type: node.node_type.clone(),
            properties: node.properties.clone(),
            workspace: workspace.clone(),
        };

        let mut new_binding = base_binding.clone();
        new_binding.bind_node(source_var.clone(), node_info);
        seeded_bindings.push(new_binding);
    }

    Ok(Some(seeded_bindings))
}

async fn fetch_node_by_path<S: Storage>(
    storage: &Arc<S>,
    context: &CypherContext,
    path: &str,
) -> Result<Option<raisin_models::nodes::Node>> {
    let node_id_opt = storage
        .nodes()
        .get_node_id_by_path(
            StorageScope::new(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                &context.workspace_id,
            ),
            path,
            context.revision.as_ref(),
        )
        .await
        .map_err(|e| ExecutionError::Backend(e.to_string()))?;

    if let Some(node_id) = node_id_opt {
        let node = storage
            .nodes()
            .get(
                StorageScope::new(
                    &context.tenant_id,
                    &context.repo_id,
                    &context.branch,
                    &context.workspace_id,
                ),
                &node_id,
                context.revision.as_ref(),
            )
            .await
            .map_err(|e| ExecutionError::Backend(e.to_string()))?;
        Ok(node)
    } else {
        Ok(None)
    }
}

fn list_options_for_context(context: &CypherContext) -> ListOptions {
    if let Some(revision) = context.revision {
        ListOptions::at_revision(revision)
    } else {
        ListOptions::for_sql()
    }
}

fn partition_constraints(constraints: Vec<PathConstraint>) -> (Vec<String>, Vec<String>) {
    let mut exact = Vec::new();
    let mut prefix = Vec::new();

    for constraint in constraints {
        match constraint {
            PathConstraint::Exact(path) => {
                if !exact.contains(&path) {
                    exact.push(path);
                }
            }
            PathConstraint::Prefix(value) => {
                if !prefix.contains(&value) {
                    prefix.push(value);
                }
            }
        }
    }

    (exact, prefix)
}

fn path_constraint_from_properties(pattern: &NodePattern) -> Option<PathConstraint> {
    pattern.properties.as_ref().and_then(|props| {
        props.iter().find_map(|(key, expr)| {
            if key == "path" {
                if let Expr::Literal(Literal::String(path)) = expr {
                    Some(PathConstraint::Exact(path.clone()))
                } else {
                    None
                }
            } else {
                None
            }
        })
    })
}

fn collect_path_constraints_from_expr(expr: &Expr, var: &str, out: &mut Vec<PathConstraint>) {
    match expr {
        Expr::BinaryOp { left, op, right } => match op {
            BinOp::And | BinOp::Or => {
                collect_path_constraints_from_expr(left, var, out);
                collect_path_constraints_from_expr(right, var, out);
            }
            BinOp::StartsWith => {
                if let Some(prefix) = literal_if_path(left, right, var) {
                    out.push(PathConstraint::Prefix(prefix));
                }
                if let Some(prefix) = literal_if_path(right, left, var) {
                    out.push(PathConstraint::Prefix(prefix));
                }
            }
            BinOp::Eq => {
                if let Some(path) = literal_if_path(left, right, var) {
                    out.push(PathConstraint::Exact(path));
                }
                if let Some(path) = literal_if_path(right, left, var) {
                    out.push(PathConstraint::Exact(path));
                }
            }
            _ => {}
        },
        Expr::UnaryOp { op, expr: inner } => {
            if matches!(op, UnOp::Not) {
                return;
            }
            collect_path_constraints_from_expr(inner, var, out);
        }
        Expr::Case {
            operand,
            when_branches,
            else_branch,
        } => {
            if let Some(opnd) = operand {
                collect_path_constraints_from_expr(opnd, var, out);
            }
            for (when_expr, then_expr) in when_branches {
                collect_path_constraints_from_expr(when_expr, var, out);
                collect_path_constraints_from_expr(then_expr, var, out);
            }
            if let Some(else_expr) = else_branch {
                collect_path_constraints_from_expr(else_expr, var, out);
            }
        }
        Expr::List(items) => {
            for item in items {
                collect_path_constraints_from_expr(item, var, out);
            }
        }
        Expr::Map(entries) => {
            for (_, value) in entries {
                collect_path_constraints_from_expr(value, var, out);
            }
        }
        _ => {}
    }
}

fn literal_if_path(path_candidate: &Expr, other: &Expr, var: &str) -> Option<String> {
    if is_path_property(path_candidate, var) {
        extract_string_literal(other)
    } else {
        None
    }
}

fn is_path_property(expr: &Expr, var: &str) -> bool {
    match expr {
        Expr::Property {
            expr: inner,
            property,
        } => property == "path" && matches!(inner.as_ref(), Expr::Variable(name) if name == var),
        _ => false,
    }
}

fn extract_string_literal(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Literal(Literal::String(value)) => Some(value.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_prefix_constraint_from_where_clause() {
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Property {
                expr: Box::new(Expr::Variable("article".into())),
                property: "path".into(),
            }),
            op: BinOp::StartsWith,
            right: Box::new(Expr::Literal(Literal::String("/content/blog/".into()))),
        };

        let mut constraints = Vec::new();
        collect_path_constraints_from_expr(&expr, "article", &mut constraints);

        assert_eq!(
            constraints,
            vec![PathConstraint::Prefix("/content/blog/".into())]
        );
    }

    #[test]
    fn collects_exact_constraint_from_properties() {
        let node_pattern = NodePattern {
            variable: Some("article".into()),
            labels: vec!["Article".into()],
            properties: Some(vec![(
                "path".into(),
                Expr::Literal(Literal::String("/content/blog/post-1".into())),
            )]),
            where_clause: None,
        };

        let constraint = path_constraint_from_properties(&node_pattern);

        assert_eq!(
            constraint,
            Some(PathConstraint::Exact("/content/blog/post-1".into()))
        );
    }
}
