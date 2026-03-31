//! PGQ Query Executor
//!
//! Main entry point for executing GRAPH_TABLE queries.

use std::sync::Arc;

use raisin_sql::ast::{GraphTableQuery, MatchClause, PatternElement, WhereClause};
use raisin_storage::Storage;

use super::context::PgqContext;
use super::filter::filter_bindings;
use super::matching::{
    analyze_pattern, execute_variable_length_pattern, match_single_hop, match_single_node,
    PatternStructure,
};
use super::projection::project_columns;
use super::types::{PgqRow, VariableBinding};
use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// PGQ Query Executor
///
/// Executes GRAPH_TABLE queries by:
/// 1. Matching graph patterns against storage
/// 2. Filtering by WHERE clause
/// 3. Projecting COLUMNS to flat SQL rows
pub struct PgqExecutor<S: Storage> {
    storage: Arc<S>,
    context: PgqContext,
}

impl<S: Storage> PgqExecutor<S> {
    /// Create a new PGQ executor
    pub fn new(storage: Arc<S>, context: PgqContext) -> Self {
        Self { storage, context }
    }

    /// Execute a GRAPH_TABLE query
    pub async fn execute(&self, query: GraphTableQuery) -> Result<Vec<PgqRow>> {
        tracing::info!(
            "PGQ: Executing GRAPH_TABLE on graph '{}'",
            query.effective_graph_name()
        );

        // 1. Execute MATCH clause
        let bindings = self.execute_match(&query.match_clause).await?;
        tracing::debug!("PGQ: MATCH produced {} bindings", bindings.len());

        if bindings.is_empty() {
            return Ok(vec![]);
        }

        // 2. Apply WHERE clause if present
        let filtered = if let Some(where_clause) = &query.where_clause {
            self.apply_where(where_clause, bindings).await?
        } else {
            bindings
        };
        tracing::debug!("PGQ: After WHERE: {} bindings", filtered.len());

        if filtered.is_empty() {
            return Ok(vec![]);
        }

        // 3. Project COLUMNS
        let rows = project_columns(
            &query.columns_clause,
            filtered,
            &self.storage,
            &self.context,
        )
        .await?;
        tracing::info!("PGQ: Produced {} result rows", rows.len());

        Ok(rows)
    }

    /// Execute MATCH clause
    async fn execute_match(&self, match_clause: &MatchClause) -> Result<Vec<VariableBinding>> {
        let mut all_bindings = Vec::new();

        for pattern in &match_clause.patterns {
            let structure = analyze_pattern(pattern)?;

            let bindings = match structure {
                PatternStructure::SingleNode(node) => {
                    // Single node pattern - extract unique nodes from relations
                    match_single_node(&node, &self.storage, &self.context).await?
                }

                PatternStructure::SingleHop {
                    source,
                    rel,
                    target,
                } => {
                    if rel.quantifier.is_some() {
                        // Variable-length path: (a)-[:TYPE*]->(b)
                        execute_variable_length_pattern(
                            &source,
                            &rel,
                            &target,
                            vec![VariableBinding::new()],
                            &self.storage,
                            &self.context,
                        )
                        .await?
                    } else {
                        // Single hop: (a)-[:TYPE]->(b)
                        match_single_hop(&source, &rel, &target, &self.storage, &self.context)
                            .await?
                    }
                }

                PatternStructure::Chain(elements) => {
                    // Multi-hop chain - execute incrementally
                    self.execute_chain(&elements).await?
                }
            };

            all_bindings.extend(bindings);
        }

        Ok(all_bindings)
    }

    /// Execute a chain of patterns: (a)-[r1]->(b)-[r2]->(c)
    async fn execute_chain(&self, elements: &[PatternElement]) -> Result<Vec<VariableBinding>> {
        // Parse chain into segments
        let mut segments = Vec::new();
        let mut i = 0;

        while i + 2 < elements.len() {
            if let (
                PatternElement::Node(source),
                PatternElement::Relationship(rel),
                PatternElement::Node(target),
            ) = (&elements[i], &elements[i + 1], &elements[i + 2])
            {
                segments.push((source.clone(), rel.clone(), target.clone()));
                i += 2; // Move to target, which becomes source for next segment
            } else {
                return Err(ExecutionError::Validation(
                    "Invalid chain pattern structure".into(),
                ));
            }
        }

        if segments.is_empty() {
            return Err(ExecutionError::Validation(
                "Chain pattern must have at least one hop".into(),
            ));
        }

        // Execute first segment
        let (first_source, first_rel, first_target) = &segments[0];
        let mut bindings = match_single_hop(
            first_source,
            first_rel,
            first_target,
            &self.storage,
            &self.context,
        )
        .await?;

        // Execute remaining segments, using previous target as new source
        for (_, rel, target) in segments.iter().skip(1) {
            let mut new_bindings = Vec::new();

            for binding in &bindings {
                // Get the target from previous hop
                if let Some(prev_target_var) = target.variable.as_ref() {
                    // The previous target becomes the new source
                    // We need to match from that node
                    let prev_segment_target_var = segments
                        .iter()
                        .find_map(|(_, _, t)| t.variable.as_ref())
                        .ok_or_else(|| {
                            ExecutionError::Validation(
                                "Chain requires variable bindings for intermediate nodes".into(),
                            )
                        })?;

                    if let Some(source_node) = binding.get_node(prev_segment_target_var) {
                        // Match from this node to target
                        let hop_bindings = super::matching::match_from_source(
                            source_node,
                            rel,
                            target,
                            &self.storage,
                            &self.context,
                        )
                        .await?;

                        // Merge bindings
                        for hop_binding in hop_bindings {
                            let mut merged = binding.clone();
                            // Copy new bindings from hop
                            if let Some(var) = &target.variable {
                                if let Some(node) = hop_binding.get_node(var) {
                                    merged.bind_node(var.clone(), node.clone());
                                }
                            }
                            if let Some(var) = &rel.variable {
                                if let Some(r) = hop_binding.get_relation(var) {
                                    merged.bind_relation(var.clone(), r.clone());
                                }
                            }
                            new_bindings.push(merged);
                        }
                    }
                }
            }

            bindings = new_bindings;
        }

        Ok(bindings)
    }

    /// Apply WHERE clause filter
    async fn apply_where(
        &self,
        where_clause: &WhereClause,
        bindings: Vec<VariableBinding>,
    ) -> Result<Vec<VariableBinding>> {
        filter_bindings(
            &where_clause.expression,
            bindings,
            &self.storage,
            &self.context,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_executor_creation() {
        // This is a compilation test - actual execution requires storage
        let context = PgqContext::new(
            "ws".into(),
            "tenant".into(),
            "repo".into(),
            "main".into(),
            None,
        );

        // Would need a mock storage to test fully
        // let executor = PgqExecutor::new(storage, context);
    }
}
