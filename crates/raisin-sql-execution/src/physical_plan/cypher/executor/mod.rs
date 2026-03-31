//! Cypher query executor
//!
//! Executes Cypher queries by translating AST to storage operations.
//!
//! # Module Structure
//!
//! - `ordering` - ORDER BY, property comparison, and WHERE predicate attachment

mod ordering;

use std::sync::Arc;

use raisin_cypher_parser::{Clause, Expr, Query};
use raisin_storage::Storage;

use super::evaluation::{execute_where, FunctionContext};
use super::matching;
use super::projection::ProjectionEngine;
use super::types::{CypherContext, CypherRow, VariableBinding};
use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Cypher query executor
pub struct CypherExecutor<S: Storage> {
    storage: Arc<S>,
    context: CypherContext,
    projection_engine: ProjectionEngine<S>,
}

impl<S: Storage> CypherExecutor<S> {
    /// Create a new executor
    pub fn new(storage: Arc<S>, context: CypherContext) -> Self {
        let projection_engine = ProjectionEngine::new(
            Arc::clone(&storage),
            context.tenant_id.clone(),
            context.repo_id.clone(),
            context.branch.clone(),
            context.workspace_id.clone(),
            context.revision,
            Arc::clone(&context.parameters),
        );
        Self {
            storage,
            context,
            projection_engine,
        }
    }

    /// Execute a Cypher query
    pub async fn execute(&self, mut query: Query) -> Result<Vec<CypherRow>> {
        tracing::info!("CypherExecutor::execute() called");
        tracing::debug!("   Query has {} clauses", query.clauses.len());

        ordering::attach_match_where_predicates(&mut query.clauses);

        // Extract RETURN clause for final projection
        let return_clause = query.clauses.iter().find_map(|clause| {
            if let Clause::Return {
                items,
                distinct,
                order_by,
                skip,
                limit,
            } = clause
            {
                Some((items, *distinct, order_by, skip.clone(), limit.clone()))
            } else {
                None
            }
        });

        let mut bindings = vec![VariableBinding::new()];

        // Execute clauses in order
        for (idx, clause) in query.clauses.iter().enumerate() {
            tracing::debug!(
                "   Executing clause #{}: {:?}",
                idx + 1,
                std::mem::discriminant(clause)
            );
            bindings = self.execute_clause(clause, bindings).await?;
            tracing::debug!(
                "   Clause #{} complete: {} bindings",
                idx + 1,
                bindings.len()
            );
        }

        if bindings.is_empty() {
            tracing::warn!("   No bindings after execution");
            return Ok(vec![]);
        }

        tracing::info!("   Execution produced {} bindings", bindings.len());

        // Project bindings to result rows based on RETURN clause
        if let Some((return_items, distinct, order_by, skip, limit)) = return_clause {
            tracing::debug!(
                "   Projecting {} bindings using {} return items",
                bindings.len(),
                return_items.len()
            );
            let mut rows = self
                .projection_engine
                .project(&bindings, return_items)
                .await?;
            tracing::info!("   Projection complete: {} rows", rows.len());

            // Apply ORDER BY
            if !order_by.is_empty() {
                tracing::debug!("   Applying ORDER BY with {} expressions", order_by.len());
                ordering::apply_order_by(&mut rows, order_by, return_items)?;
            }

            // Apply SKIP
            if let Some(Expr::Literal(raisin_cypher_parser::Literal::Integer(skip_count))) = skip {
                tracing::debug!("   Applying SKIP {}", skip_count);
                if skip_count > 0 {
                    rows = rows.into_iter().skip(skip_count as usize).collect();
                }
            }

            // Apply LIMIT
            if let Some(Expr::Literal(raisin_cypher_parser::Literal::Integer(limit_count))) = limit
            {
                tracing::debug!("   Applying LIMIT {}", limit_count);
                rows.truncate(limit_count as usize);
            }

            tracing::info!("   Final result: {} rows", rows.len());
            Ok(rows)
        } else {
            tracing::warn!("   No RETURN clause found");
            Ok(vec![])
        }
    }

    /// Execute a single clause
    async fn execute_clause(
        &self,
        clause: &Clause,
        bindings: Vec<VariableBinding>,
    ) -> Result<Vec<VariableBinding>> {
        match clause {
            Clause::Match { pattern, .. } => self.execute_match(pattern, bindings).await,
            Clause::Where { condition } => self.execute_where(condition, bindings).await,
            Clause::Create { pattern } => self.execute_create(pattern, bindings).await,
            Clause::Return { .. } => Ok(bindings),
            _ => Err(ExecutionError::Validation(format!(
                "Unsupported Cypher clause: {:?}",
                clause
            ))),
        }
    }

    /// Execute MATCH clause
    async fn execute_match(
        &self,
        pattern: &raisin_cypher_parser::GraphPattern,
        bindings: Vec<VariableBinding>,
    ) -> Result<Vec<VariableBinding>> {
        let mut result_bindings = Vec::new();

        let starting_bindings = if bindings.is_empty() {
            vec![VariableBinding::new()]
        } else {
            bindings
        };

        for binding in starting_bindings {
            for path_pattern in &pattern.patterns {
                let matched = matching::match_path_pattern(
                    path_pattern,
                    pattern.where_clause.as_ref(),
                    binding.clone(),
                    &self.storage,
                    &self.context,
                )
                .await?;
                result_bindings.extend(matched);
            }
        }

        Ok(result_bindings)
    }

    /// Execute WHERE clause (filter bindings)
    async fn execute_where(
        &self,
        condition: &Expr,
        bindings: Vec<VariableBinding>,
    ) -> Result<Vec<VariableBinding>> {
        let context = FunctionContext {
            storage: &*self.storage,
            tenant_id: &self.context.tenant_id,
            repo_id: &self.context.repo_id,
            branch: &self.context.branch,
            workspace_id: &self.context.workspace_id,
            revision: self.context.revision.as_ref(),
            parameters: &self.context.parameters,
        };

        execute_where(condition, bindings, &context).await
    }

    /// Execute CREATE clause
    async fn execute_create(
        &self,
        _pattern: &raisin_cypher_parser::GraphPattern,
        _bindings: Vec<VariableBinding>,
    ) -> Result<Vec<VariableBinding>> {
        Err(ExecutionError::Validation(
            "CREATE clause not yet implemented".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_cypher_parser::parse_query;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_models::nodes::{Node, RelationRef};
    use raisin_rocksdb::RocksDBStorage;
    use raisin_storage::{BranchRepository, CreateNodeOptions, NodeRepository, RelationRepository};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::TempDir;

    const TENANT: &str = "test-tenant";
    const REPO: &str = "test-repo";
    const BRANCH: &str = "main";
    const WORKSPACE: &str = "workspace-a";
    const REL_TYPE: &str = "LINKS_TO";

    #[tokio::test]
    async fn match_uses_path_property_seed() {
        let (executor, _db_dir) = build_executor().await;
        let query = parse_query(
            "MATCH (article:Article {path: '/content/blog/premium'})-[:LINKS_TO]->(asset:Asset)\
             \nRETURN article.path AS article_path, asset.path AS asset_path",
        )
        .expect("query parses");

        let rows = executor.execute(query).await.expect("execution succeeds");
        assert_eq!(rows.len(), 1, "path filter should return a single match");
        assert_eq!(
            rows[0].columns,
            vec!["article_path".to_string(), "asset_path".to_string()]
        );
        assert_eq!(
            rows[0].values,
            vec![
                PropertyValue::String("/content/blog/premium".into()),
                PropertyValue::String("/assets/premium".into()),
            ]
        );
    }

    #[tokio::test]
    async fn match_uses_where_prefix_seed() {
        let (executor, _db_dir) = build_executor().await;
        let query = parse_query(
            "MATCH (article:Article)-[:LINKS_TO]->(asset:Asset)\
             \nWHERE article.path STARTS WITH '/content/blog/pre'\
             \nRETURN article.path AS article_path",
        )
        .expect("query parses");

        let rows = executor.execute(query).await.expect("execution succeeds");
        assert_eq!(
            rows.len(),
            1,
            "prefix filter should only seed premium article"
        );
        assert_eq!(
            rows[0].values,
            vec![PropertyValue::String("/content/blog/premium".into())]
        );
    }

    #[tokio::test]
    async fn where_clause_consumes_parameters() {
        let mut params = HashMap::new();
        params.insert(
            "filter_path".to_string(),
            PropertyValue::String("/content/blog/basic".into()),
        );

        let (executor, _db_dir) = build_executor_with_params(params).await;
        let query = parse_query(
            "MATCH (article:Article)-[:LINKS_TO]->(asset:Asset)\
             \nWHERE article.path = $filter_path\
             \nRETURN asset.path AS asset_path, $filter_path AS requested_path",
        )
        .expect("query parses");

        let rows = executor.execute(query).await.expect("execution succeeds");
        assert_eq!(rows.len(), 1, "parameter should narrow matches to one");
        assert_eq!(
            rows[0].columns,
            vec!["asset_path".to_string(), "requested_path".to_string()]
        );
        assert_eq!(
            rows[0].values,
            vec![
                PropertyValue::String("/assets/basic".into()),
                PropertyValue::String("/content/blog/basic".into()),
            ]
        );
    }

    async fn build_executor() -> (CypherExecutor<RocksDBStorage>, TempDir) {
        build_executor_with_params(HashMap::new()).await
    }

    async fn build_executor_with_params(
        parameters: HashMap<String, PropertyValue>,
    ) -> (CypherExecutor<RocksDBStorage>, TempDir) {
        let temp_dir = TempDir::new().expect("temp directory");
        let storage = Arc::new(RocksDBStorage::new(temp_dir.path()).expect("rocksdb storage"));
        init_branch(&storage).await;
        populate_graph(&storage).await;
        let context = test_context().with_parameters(parameters);
        (CypherExecutor::new(storage, context), temp_dir)
    }

    async fn init_branch(storage: &Arc<RocksDBStorage>) {
        storage
            .branches()
            .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
            .await
            .expect("branch initialized");
    }

    async fn populate_graph<S: raisin_storage::Storage>(storage: &Arc<S>) {
        insert_node(storage, "article-basic", "/content/blog/basic", "Article").await;
        insert_node(
            storage,
            "article-premium",
            "/content/blog/premium",
            "Article",
        )
        .await;
        insert_node(storage, "asset-basic", "/assets/basic", "Asset").await;
        insert_node(storage, "asset-premium", "/assets/premium", "Asset").await;

        relate(storage, "article-basic", "Article", "asset-basic", "Asset").await;
        relate(
            storage,
            "article-premium",
            "Article",
            "asset-premium",
            "Asset",
        )
        .await;
    }

    async fn insert_node<S: raisin_storage::Storage>(
        storage: &Arc<S>,
        id: &str,
        path: &str,
        node_type: &str,
    ) {
        let mut node = Node::default();
        node.id = id.to_string();
        node.name = id.to_string();
        node.path = path.to_string();
        node.node_type = node_type.to_string();
        node.workspace = Some(WORKSPACE.to_string());

        storage
            .nodes()
            .create(
                TENANT,
                REPO,
                BRANCH,
                WORKSPACE,
                node,
                relaxed_create_options(),
            )
            .await
            .expect("node created");
    }

    async fn relate<S: raisin_storage::Storage>(
        storage: &Arc<S>,
        source_id: &str,
        source_type: &str,
        target_id: &str,
        target_type: &str,
    ) {
        let relation = RelationRef::simple(
            target_id.to_string(),
            WORKSPACE.to_string(),
            target_type.to_string(),
            REL_TYPE.to_string(),
        );

        storage
            .relations()
            .add_relation(
                TENANT,
                REPO,
                BRANCH,
                WORKSPACE,
                source_id,
                source_type,
                relation,
            )
            .await
            .expect("relation created");
    }

    fn relaxed_create_options() -> CreateNodeOptions {
        CreateNodeOptions {
            validate_schema: false,
            validate_parent_allows_child: false,
            validate_workspace_allows_type: false,
            operation_meta: None,
        }
    }

    fn test_context() -> CypherContext {
        CypherContext::new(
            WORKSPACE.into(),
            TENANT.into(),
            REPO.into(),
            BRANCH.into(),
            None,
        )
    }
}
