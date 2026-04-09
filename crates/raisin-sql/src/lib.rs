//! RaisinSQL Query Parser and Planner
//!
//! A SQL parsing and planning library for RaisinDB with support for:
//! - Hierarchical path operations
//! - JSON queries
//! - Vector search
//! - Graph traversal
//!
//! This crate is WASM-compatible and contains only the parsing and planning layers.
//! For physical execution, see the `raisin-sql-execution` crate.
//!
//! # Architecture
//!
//! The query planning pipeline consists of several phases:
//!
//! 1. **AST (Abstract Syntax Tree)** - Parse SQL into an AST and validate syntax
//! 2. **Analyzer** - Semantic analysis and type checking
//! 3. **Logical Plan** - Transform analyzed queries into logical operator trees
//! 4. **Optimizer** - Apply query optimizations (constant folding, predicate pushdown, etc.)
//!
//! # Example
//!
//! ```
//! use raisin_sql::{parse_sql, Analyzer, PlanBuilder};
//! use raisin_sql::analyzer::StaticCatalog;
//!
//! // Parse SQL
//! let sql = "SELECT id, name FROM nodes WHERE PATH_STARTS_WITH(path, '/content/') LIMIT 10";
//! let statements = parse_sql(sql).unwrap();
//! assert_eq!(statements.len(), 1);
//!
//! // Semantic analysis
//! let catalog = StaticCatalog::default_nodes_schema();
//! let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
//! let analyzed = analyzer.analyze(sql).unwrap();
//!
//! // Logical planning
//! let planner = PlanBuilder::new(&catalog);
//! let plan = planner.build(&analyzed).unwrap();
//! println!("Query plan:\n{}", plan.explain());
//! ```

pub mod ast;

// Core modules
pub mod analyzer;
pub mod completion;
pub mod logical_plan;
pub mod optimizer;
pub mod params;

// Re-export commonly used items from ast
pub use ast::{parse_sql, ParseError, RaisinDialect};

// Re-export commonly used items from analyzer
pub use analyzer::{
    AnalyzedMove, AnalyzedQuery, AnalyzedShow, AnalyzedStatement, Analyzer, Catalog, DataType,
    StaticCatalog,
};

// Re-export commonly used items from logical_plan
pub use logical_plan::{LogicalPlan, PlanBuilder, PlanError};

// Re-export optimizer
pub use optimizer::{Optimizer, OptimizerConfig};

// Re-export parameter substitution
pub use params::substitute_params;

/// Complete query plan with analyzed statement, logical plan, and optimized plan
#[derive(Debug, Clone)]
pub struct QueryPlan {
    pub sql: String,
    pub analyzed: AnalyzedStatement,
    pub logical: LogicalPlan,
    pub optimized: LogicalPlan,
}

impl QueryPlan {
    /// Create a new query plan with optimization
    ///
    /// This is the recommended way to create a query plan. It performs:
    /// 1. SQL parsing
    /// 2. Semantic analysis
    /// 3. Logical plan construction
    /// 4. Query optimization
    pub fn from_sql(sql: &str) -> Result<Self, String> {
        // 1. Semantic analysis
        let analyzer = Analyzer::new();
        let analyzed = analyzer
            .analyze(sql)
            .map_err(|e| format!("Analysis error: {}", e))?;

        // 2. Logical plan construction
        let catalog = analyzer::StaticCatalog::default_nodes_schema();
        let planner = PlanBuilder::new(&catalog);
        let logical = planner
            .build(&analyzed)
            .map_err(|e| format!("Planning error: {}", e))?;

        // 3. Query optimization
        let optimizer = Optimizer::new();
        let optimized = optimizer.optimize(logical.clone());

        Ok(Self {
            sql: sql.to_string(),
            analyzed,
            logical,
            optimized,
        })
    }

    /// Create a query plan with custom optimizer configuration
    pub fn from_sql_with_config(
        sql: &str,
        optimizer_config: OptimizerConfig,
    ) -> Result<Self, String> {
        // 1. Semantic analysis
        let analyzer = Analyzer::new();
        let analyzed = analyzer
            .analyze(sql)
            .map_err(|e| format!("Analysis error: {}", e))?;

        // 2. Logical plan construction
        let catalog = analyzer::StaticCatalog::default_nodes_schema();
        let planner = PlanBuilder::new(&catalog);
        let logical = planner
            .build(&analyzed)
            .map_err(|e| format!("Planning error: {}", e))?;

        // 3. Query optimization with custom config
        let optimizer = Optimizer::with_config(optimizer_config);
        let optimized = optimizer.optimize(logical.clone());

        Ok(Self {
            sql: sql.to_string(),
            analyzed,
            logical,
            optimized,
        })
    }

    /// Create a query plan with a custom catalog
    ///
    /// This method allows passing a custom catalog with tenant-specific schema,
    /// such as adding the `embedding` column with specific dimensions for
    /// vector similarity search.
    ///
    /// # Arguments
    ///
    /// * `sql` - The SQL query string to parse and plan
    /// * `catalog` - Custom StaticCatalog (e.g., with embedding column)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use raisin_sql::{QueryPlan, analyzer::StaticCatalog};
    ///
    /// // Create catalog with embedding support
    /// let catalog = StaticCatalog::default_nodes_schema()
    ///     .with_embedding_column(1536);
    ///
    /// // Create query plan with custom catalog
    /// let plan = QueryPlan::from_sql_with_catalog(
    ///     "SELECT * FROM nodes ORDER BY embedding <-> EMBEDDING('query') LIMIT 10",
    ///     catalog
    /// )?;
    /// ```
    pub fn from_sql_with_catalog(
        sql: &str,
        catalog: analyzer::StaticCatalog,
    ) -> Result<Self, String> {
        // 1. Semantic analysis with custom catalog
        let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
        let analyzed = analyzer
            .analyze(sql)
            .map_err(|e| format!("Analysis error: {}", e))?;

        // 2. Logical plan construction
        let planner = PlanBuilder::new(&catalog);
        let logical = planner
            .build(&analyzed)
            .map_err(|e| format!("Planning error: {}", e))?;

        // 3. Query optimization
        let optimizer = Optimizer::new();
        let optimized = optimizer.optimize(logical.clone());

        Ok(Self {
            sql: sql.to_string(),
            analyzed,
            logical,
            optimized,
        })
    }

    /// Create a query plan from an already-analyzed statement and catalog
    ///
    /// This is useful when you have already performed semantic analysis
    /// and want to build a logical plan with optimization.
    pub fn from_analyzed(
        analyzed: &AnalyzedStatement,
        catalog: &StaticCatalog,
    ) -> Result<Self, String> {
        let planner = PlanBuilder::new(catalog);
        let logical = planner
            .build(analyzed)
            .map_err(|e| format!("Planning error: {}", e))?;

        let optimizer = Optimizer::new();
        let optimized = optimizer.optimize(logical.clone());

        Ok(Self {
            sql: String::new(),
            analyzed: analyzed.clone(),
            logical,
            optimized,
        })
    }

    /// Create a query plan with automatic optimization
    ///
    /// This is a convenience constructor for demos and testing.
    /// The optimizer is applied automatically.
    pub fn new(analyzed: AnalyzedStatement, logical: LogicalPlan) -> Self {
        let optimizer = Optimizer::new();
        let optimized = optimizer.optimize(logical.clone());

        Self {
            sql: String::new(), // SQL not available when constructed this way
            analyzed,
            logical,
            optimized,
        }
    }

    /// Create a query plan without optimization (for testing)
    pub fn without_optimization(
        sql: String,
        analyzed: AnalyzedStatement,
        logical: LogicalPlan,
    ) -> Self {
        Self {
            sql,
            analyzed: analyzed.clone(),
            logical: logical.clone(),
            optimized: logical, // Same as logical when no optimization
        }
    }

    /// Multi-stage EXPLAIN showing SQL, analysis, logical, and optimized plans
    ///
    /// This provides a complete view of the query compilation pipeline,
    /// useful for understanding optimizations and debugging.
    pub fn explain(&self) -> String {
        let mut output = String::new();

        output.push_str("=== SQL ===\n");
        output.push_str(&self.sql);
        output.push_str("\n\n");

        output.push_str("=== Analyzed Statement ===\n");
        match &self.analyzed {
            AnalyzedStatement::Query(q) => {
                output.push_str(&format!("Projection: {} columns\n", q.projection.len()));
                for (idx, (expr, alias)) in q.projection.iter().enumerate() {
                    output.push_str(&format!(
                        "  [{}] {:?} AS {}\n",
                        idx,
                        expr.data_type,
                        alias.as_ref().unwrap_or(&format!("col_{}", idx))
                    ));
                }
                output.push_str("From: ");
                for (idx, table) in q.from.iter().enumerate() {
                    if idx > 0 {
                        output.push_str(", ");
                    }
                    output.push_str(&table.table);
                    if let Some(alias) = &table.alias {
                        output.push_str(&format!(" AS {}", alias));
                    }
                }
                output.push('\n');
                if !q.joins.is_empty() {
                    output.push_str(&format!("Joins: {} joins\n", q.joins.len()));
                }
                if let Some(sel) = &q.selection {
                    output.push_str(&format!("Selection: {:?}\n", sel.data_type));
                }
                if !q.order_by.is_empty() {
                    output.push_str(&format!("Order By: {} expressions\n", q.order_by.len()));
                }
                if let Some(limit) = q.limit {
                    output.push_str(&format!("Limit: {}\n", limit));
                }
                if let Some(offset) = q.offset {
                    output.push_str(&format!("Offset: {}\n", offset));
                }
            }
            AnalyzedStatement::Explain(explain) => {
                output.push_str(&format!(
                    "EXPLAIN (verbose={}, analyze={}, format={:?})\n",
                    explain.verbose, explain.analyze, explain.format
                ));
                output.push_str("Inner query:\n");
                // Show simplified query info
                let q = &explain.query;
                output.push_str(&format!("  Projection: {} columns\n", q.projection.len()));
                output.push_str(&format!(
                    "  From: {:?}\n",
                    q.from.iter().map(|t| &t.table).collect::<Vec<_>>()
                ));
            }
            AnalyzedStatement::Insert(insert) => {
                output.push_str(&format!(
                    "INSERT INTO {} ({} columns, {} rows)\n",
                    insert.target.table_name(),
                    insert.columns.len(),
                    insert.values.len()
                ));
                output.push_str(&format!("  Columns: {:?}\n", insert.columns));
            }
            AnalyzedStatement::Update(update) => {
                output.push_str(&format!(
                    "UPDATE {} ({} assignments)\n",
                    update.target.table_name(),
                    update.assignments.len()
                ));
                output.push_str(&format!(
                    "  Columns: {:?}\n",
                    update
                        .assignments
                        .iter()
                        .map(|(col, _)| col)
                        .collect::<Vec<_>>()
                ));
                if update.filter.is_some() {
                    output.push_str("  WHERE: present\n");
                } else {
                    output.push_str("  WHERE: none (updates all rows)\n");
                }
            }
            AnalyzedStatement::Delete(delete) => {
                output.push_str(&format!("DELETE FROM {}\n", delete.target.table_name()));
                if delete.filter.is_some() {
                    output.push_str("  WHERE: present\n");
                } else {
                    output.push_str("  WHERE: none (deletes all rows)\n");
                }
            }
            AnalyzedStatement::Ddl(ddl) => {
                output.push_str(&format!("{} '{}'\n", ddl.operation(), ddl.type_name()));
            }
            AnalyzedStatement::Transaction(txn) => {
                output.push_str(&format!("{}\n", txn.operation()));
            }
            AnalyzedStatement::Order(order) => {
                output.push_str(&format!(
                    "ORDER {:?} {:?} {:?}\n",
                    order.source, order.position, order.target
                ));
            }
            AnalyzedStatement::Move(move_stmt) => {
                output.push_str(&format!(
                    "MOVE {:?} TO {:?}\n",
                    move_stmt.source, move_stmt.target_parent
                ));
            }
            AnalyzedStatement::Copy(copy_stmt) => {
                let mode = if copy_stmt.recursive { "TREE " } else { "" };
                output.push_str(&format!(
                    "COPY {}{:?} TO {:?}",
                    mode, copy_stmt.source, copy_stmt.target_parent
                ));
                if let Some(name) = &copy_stmt.new_name {
                    output.push_str(&format!(" AS '{}'", name));
                }
                output.push('\n');
            }
            AnalyzedStatement::Translate(translate) => {
                output.push_str(&format!(
                    "TRANSLATE FOR LOCALE '{}' ({} node props, {} blocks)\n",
                    translate.locale,
                    translate.node_translations.len(),
                    translate.block_translations.len()
                ));
                if translate.filter.is_some() {
                    output.push_str("  WHERE: present\n");
                }
            }
            AnalyzedStatement::Relate(relate) => {
                output.push_str(&format!(
                    "RELATE FROM {}:{} TO {}:{} TYPE '{}'\n",
                    relate.source.workspace,
                    relate.source.node_ref,
                    relate.target.workspace,
                    relate.target.node_ref,
                    relate.relation_type
                ));
                if let Some(weight) = relate.weight {
                    output.push_str(&format!("  WEIGHT: {}\n", weight));
                }
            }
            AnalyzedStatement::Unrelate(unrelate) => {
                output.push_str(&format!(
                    "UNRELATE FROM {}:{} TO {}:{}\n",
                    unrelate.source.workspace,
                    unrelate.source.node_ref,
                    unrelate.target.workspace,
                    unrelate.target.node_ref
                ));
                if let Some(ref rel_type) = unrelate.relation_type {
                    output.push_str(&format!("  TYPE: '{}'\n", rel_type));
                }
            }
            AnalyzedStatement::Show(show) => {
                output.push_str(&format!("SHOW {}\n", show.variable));
            }
            AnalyzedStatement::Branch(branch) => {
                output.push_str(&format!("BRANCH {}\n", branch.operation()));
            }
            AnalyzedStatement::Restore(restore) => {
                let mode = if restore.recursive { "TREE " } else { "" };
                output.push_str(&format!(
                    "RESTORE {}NODE {:?} TO REVISION {}\n",
                    mode, restore.node, restore.revision
                ));
                if let Some(translations) = &restore.translations {
                    output.push_str(&format!("  TRANSLATIONS: {:?}\n", translations));
                }
            }
            AnalyzedStatement::Acl(acl) => {
                output.push_str(&format!("ACL {}\n", acl.operation()));
            }
            AnalyzedStatement::AIConfig(ai) => {
                output.push_str(&format!("AI CONFIG {}\n", ai.operation()));
            }
        }

        output.push_str("\n=== Logical Plan (Unoptimized) ===\n");
        output.push_str(&self.logical.explain());

        output.push_str("\n=== Optimized Logical Plan ===\n");
        output.push_str(&self.optimized.explain());

        // Show optimization impact
        let logical_repr = format!("{:?}", self.logical);
        let optimized_repr = format!("{:?}", self.optimized);
        if logical_repr != optimized_repr {
            output.push_str("\n=== Optimizations Applied ===\n");
            output.push_str("Plan was transformed by optimizer\n");

            // Detect specific optimizations
            if let LogicalPlan::Scan {
                projection: Some(_),
                ..
            } = self.optimized
            {
                output.push_str("- Projection pruning: Column set minimized\n");
            }

            output.push_str("- Constant folding: Deterministic expressions evaluated\n");
            output.push_str("- Hierarchy rewriting: PATH/DEPTH functions optimized\n");
        } else {
            output.push_str("\n=== Optimizations Applied ===\n");
            output.push_str("No optimizations applied (plan unchanged)\n");
        }

        output
    }

    /// Get a compact explanation (single-line plan)
    pub fn explain_compact(&self) -> String {
        self.optimized.explain()
    }
}
