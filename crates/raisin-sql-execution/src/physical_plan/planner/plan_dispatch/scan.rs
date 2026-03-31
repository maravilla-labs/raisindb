//! Scan and TableFunction dispatch
//!
//! Handles planning of `LogicalPlan::Scan` and `LogicalPlan::TableFunction`
//! variants into their corresponding physical plan nodes.

use super::super::{
    Error, Expr, Literal, LogicalPlan, PhysicalPlan, PhysicalPlanner, PlanContext, ScanReason,
};

impl PhysicalPlanner {
    /// Plan a `LogicalPlan::Scan` node.
    ///
    /// Resolves workspace/branch defaults, then delegates to
    /// `plan_scan_with_filter` when a filter is present or falls back to a
    /// plain `TableScan`.
    pub(in crate::physical_plan::planner) fn plan_scan(
        &self,
        table: &str,
        alias: &Option<String>,
        schema: std::sync::Arc<raisin_sql::logical_plan::TableSchema>,
        workspace: &Option<String>,
        branch_override: &Option<String>,
        filter: &Option<raisin_sql::analyzer::TypedExpr>,
        projection: &Option<Vec<String>>,
        context: &PlanContext,
    ) -> Result<PhysicalPlan, Error> {
        // Use workspace from logical plan, or fall back to default
        let workspace_name = workspace
            .clone()
            .unwrap_or_else(|| self.default_workspace.to_string());

        // Use branch from SQL query (WHERE __branch = 'X'), or fall back to default repository branch
        // This enables cross-branch queries like: SELECT * FROM sites WHERE __branch = 'staging'
        let effective_branch = branch_override
            .clone()
            .unwrap_or_else(|| self.default_branch.to_string());

        // Analyze filter to select best scan method
        if let Some(filter_expr) = filter {
            self.plan_scan_with_filter(
                table,
                alias,
                schema,
                &workspace_name,
                &effective_branch,
                filter_expr,
                projection.clone(),
                context, // Pass through parent context
            )
        } else {
            // No filter in Scan node - use table scan
            // Note: Filter predicates may exist in a parent Filter operator
            Ok(PhysicalPlan::TableScan {
                tenant_id: self.default_tenant_id.to_string(),
                repo_id: self.default_repo_id.to_string(),
                branch: effective_branch,
                workspace: workspace_name,
                table: table.to_string(),
                alias: alias.clone(),
                schema,
                filter: None,
                projection: projection.clone(),
                limit: None,
                reason: ScanReason::UnsupportedPredicate {
                    details: "no filter predicates in Scan node".to_string(),
                },
            })
        }
    }

    /// Plan a `LogicalPlan::TableFunction` node.
    ///
    /// Recognises the `NEIGHBORS` table function and emits a `NeighborsScan`;
    /// all other table functions are passed through unchanged.
    pub(in crate::physical_plan::planner) fn plan_table_function(
        &self,
        name: &str,
        alias: &Option<String>,
        args: &[raisin_sql::analyzer::TypedExpr],
        schema: &std::sync::Arc<raisin_sql::logical_plan::TableSchema>,
        workspace: &Option<String>,
        branch_override: &Option<String>,
        max_revision: Option<raisin_hlc::HLC>,
    ) -> Result<PhysicalPlan, Error> {
        if name.eq_ignore_ascii_case("NEIGHBORS") {
            // Expect 3 arguments: start_id, direction, relation_type (nullable)
            let mut start_id = Self::extract_string_literal(args.first(), name, 0)?;
            let direction = Self::extract_string_literal(args.get(1), name, 1)?.to_uppercase();
            let relation_type = match args.get(2) {
                Some(expr) => match &expr.expr {
                    Expr::Literal(Literal::Null) => None,
                    Expr::Literal(Literal::Text(v)) | Expr::Literal(Literal::Path(v)) => {
                        // Treat empty string as no filter
                        if v.is_empty() {
                            None
                        } else {
                            Some(v.clone())
                        }
                    }
                    _ => {
                        return Err(Error::Validation(
                            "NEIGHBORS third argument must be relation type string or NULL"
                                .to_string(),
                        ))
                    }
                },
                None => None,
            };

            // Allow "workspace:/path" literal to override workspace for path-based traversal
            let mut workspace_name = workspace
                .clone()
                .unwrap_or_else(|| self.default_workspace.to_string());
            if let Some((ws, p)) = start_id.split_once(":/") {
                if !ws.is_empty() && p.starts_with('/') {
                    workspace_name = ws.to_string();
                    start_id = p.to_string();
                }
            }

            let branch = branch_override
                .clone()
                .unwrap_or_else(|| self.default_branch.to_string());

            Ok(PhysicalPlan::NeighborsScan {
                tenant_id: self.default_tenant_id.to_string(),
                repo_id: self.default_repo_id.to_string(),
                branch,
                alias: alias.clone(),
                source_workspace: workspace_name,
                source_node_id: start_id,
                direction,
                relation_type,
                projection: None,
                limit: None,
            })
        } else {
            Ok(PhysicalPlan::TableFunction {
                name: name.to_string(),
                alias: alias.clone(),
                args: args.to_vec(),
                schema: std::sync::Arc::clone(schema),
                workspace: workspace.clone(),
                branch_override: branch_override.clone(),
                max_revision,
            })
        }
    }
}
