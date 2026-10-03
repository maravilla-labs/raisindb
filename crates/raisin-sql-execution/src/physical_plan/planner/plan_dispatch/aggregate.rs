//! Aggregate planning and COUNT(*) optimizations
//!
//! Handles `LogicalPlan::Aggregate` dispatch, including:
//! - `CountScan` optimisation for `COUNT(*)` over unfiltered scans
//! - `PropertyIndexCountScan` for `COUNT(*)` over property-indexed scans
//! - Standard `HashAggregate` fallback

use super::super::{
    AggregateFunction, Error, LogicalPlan, PhysicalPlan, PhysicalPlanner, PlanContext,
};

impl PhysicalPlanner {
    /// Plan a `LogicalPlan::Aggregate` node.
    pub(in crate::physical_plan::planner) fn plan_aggregate(
        &self,
        input: &LogicalPlan,
        group_by: &[raisin_sql::analyzer::TypedExpr],
        aggregates: &[raisin_sql::logical_plan::AggregateExpr],
        context: &PlanContext,
    ) -> Result<PhysicalPlan, Error> {
        tracing::debug!(
            "Planning Aggregate: group_by={}, aggregates={}, first_agg={:?}",
            group_by.len(),
            aggregates.len(),
            if !aggregates.is_empty() {
                format!(
                    "func={:?}, args_len={}",
                    aggregates[0].func,
                    aggregates[0].args.len()
                )
            } else {
                "none".to_string()
            }
        );

        // Optimization: Detect COUNT(*) with no GROUP BY over a TableScan
        // Note: COUNT(*) is often converted to COUNT(1) by the analyzer, so we accept both
        // COUNT(expr) must skip NULL rows, so only a bare COUNT(*) (no
        // argument, or the literal `1` the analyzer substitutes for `*`) may
        // be answered from an index count.
        let is_count_star = aggregates.len() == 1
            && aggregates[0].func == AggregateFunction::Count
            && (aggregates[0].args.is_empty()
                || (aggregates[0].args.len() == 1
                    && matches!(
                        aggregates[0].args[0].expr,
                        raisin_sql::analyzer::Expr::Literal(raisin_sql::analyzer::Literal::Int(_))
                    )))
            // A FILTER (WHERE ...) clause must be evaluated per row — the
            // index-count pushdowns would silently drop it.
            && aggregates[0].filter.is_none();

        if group_by.is_empty() && is_count_star {
            if let Some(count_plan) = self.try_plan_count_scan(input)? {
                return Ok(count_plan);
            }
        }

        // Standard aggregate path - propagate COUNT(*) context if applicable
        let mut agg_context = context.clone();
        if is_count_star {
            agg_context = agg_context.with_count_star();
        }
        let physical_input = self.plan_with_context(input, &agg_context)?;

        // Optimization: Detect COUNT(*) over PropertyIndexScan
        if group_by.is_empty() && is_count_star {
            if let Some(count_plan) = Self::try_plan_property_index_count(&physical_input) {
                return Ok(count_plan);
            }
        }

        // Create HashAggregate physical plan
        Ok(PhysicalPlan::HashAggregate {
            input: Box::new(physical_input),
            group_by: group_by.to_vec(),
            aggregates: aggregates.to_vec(),
        })
    }

    /// Try to convert `COUNT(*)` over an unfiltered `Scan` to a `CountScan`.
    fn try_plan_count_scan(&self, input: &LogicalPlan) -> Result<Option<PhysicalPlan>, Error> {
        let input_type = match input {
            LogicalPlan::Scan { .. } => "Scan",
            LogicalPlan::Filter { .. } => "Filter",
            LogicalPlan::Project { .. } => "Project",
            LogicalPlan::Aggregate { .. } => "Aggregate",
            LogicalPlan::Join { .. } => "Join",
            LogicalPlan::Sort { .. } => "Sort",
            LogicalPlan::Limit { .. } => "Limit",
            _ => "Other",
        };
        tracing::debug!("COUNT(*) detected: input logical plan type: {}", input_type);

        if let LogicalPlan::Scan {
            table,
            workspace,
            max_revision,
            filter,
            ..
        } = input
        {
            // Only use CountScan if there's no filter
            if filter.is_none() {
                tracing::debug!("Optimizing COUNT(*) over unfiltered Scan to CountScan");
                return Ok(Some(PhysicalPlan::CountScan {
                    tenant_id: self.default_tenant_id.to_string(),
                    repo_id: self.default_repo_id.to_string(),
                    branch: self.default_branch.to_string(),
                    workspace: workspace.clone().unwrap_or_else(|| table.clone()),
                    max_revision: *max_revision,
                }));
            } else {
                tracing::debug!(
                    "COUNT(*) over filtered Scan - skipping CountScan, will try PropertyIndexCountScan"
                );
            }
        }

        Ok(None)
    }

    /// Try to convert `COUNT(*)` over a `PropertyIndexScan` — or a `Union` of
    /// bare `PropertyIndexScan`s (from `IN (...)` / same-column OR expansion) —
    /// to a `PropertyIndexCountScan`.
    ///
    /// The Union case is correct because `IN`/OR expansion branches are
    /// per-value equality scans over the same column: their row sets are
    /// disjoint, so the total count is the sum of the per-value index counts.
    fn try_plan_property_index_count(physical_input: &PhysicalPlan) -> Option<PhysicalPlan> {
        tracing::debug!(
            "COUNT(*) optimization: checking physical_input type: {}",
            physical_input.describe()
        );

        // NOT for pseudo-properties (`node_type`, `name`, `created_at`,
        // `IS_A`/`HAS_MIXIN`, …). The pushed-down count adds up index entries
        // without reading a node, and the property index can hold orphan LIVE
        // entries from historically buggy writers (the read/index plan's
        // critique, Phase 6 amendment). For a user property that is masked by
        // the JSON residual on the row path; a pseudo-property has no residual,
        // so the only guard is re-checking the decoded node
        // (`scan_executors::index_recheck`) — which a key count cannot do, and
        // which would cost exactly what the row path costs. So these COUNTs run
        // as an aggregate over the PropertyIndexScan, whose rows ARE re-checked:
        // an orphan can never inflate COUNT(*).
        //
        // The same holds for a JSON equality the scan verifies itself
        // (`verifies_value`): its re-check replaced the residual filter that
        // used to keep these COUNTs off the raw-key path, so they stay off it.
        let pseudo = |name: &str| {
            crate::physical_plan::scan_executors::index_recheck::is_pseudo_property(name)
        };

        if let PhysicalPlan::PropertyIndexScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_name,
            property_value,
            verifies_value,
            ..
        } = physical_input
        {
            if pseudo(property_name.as_str()) || *verifies_value {
                return None;
            }
            tracing::debug!("Optimizing COUNT(*) over PropertyIndexScan to PropertyIndexCountScan");
            return Some(PhysicalPlan::PropertyIndexCountScan {
                tenant_id: tenant_id.clone(),
                repo_id: repo_id.clone(),
                branch: branch.clone(),
                workspace: workspace.clone(),
                properties: vec![(property_name.clone(), property_value.clone())],
            });
        }

        // Union of bare PropertyIndexScans: every branch must be a plain
        // PropertyIndexScan (no residual Filter wrapper) in the same scope.
        if let PhysicalPlan::Union { inputs } = physical_input {
            let mut scope: Option<(String, String, String, String)> = None;
            let mut properties = Vec::with_capacity(inputs.len());
            for input in inputs {
                match input {
                    PhysicalPlan::PropertyIndexScan {
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        property_name,
                        property_value,
                        verifies_value,
                        ..
                    } => {
                        let branch_scope = (
                            tenant_id.clone(),
                            repo_id.clone(),
                            branch.clone(),
                            workspace.clone(),
                        );
                        match &scope {
                            None => scope = Some(branch_scope),
                            Some(s) if *s == branch_scope => {}
                            _ => return None,
                        }
                        if pseudo(property_name.as_str()) || *verifies_value {
                            return None;
                        }
                        properties.push((property_name.clone(), property_value.clone()));
                    }
                    _ => return None,
                }
            }
            let (tenant_id, repo_id, branch, workspace) = scope?;
            tracing::debug!(
                "Optimizing COUNT(*) over Union of {} PropertyIndexScans to summed PropertyIndexCountScan",
                properties.len()
            );
            return Some(PhysicalPlan::PropertyIndexCountScan {
                tenant_id,
                repo_id,
                branch,
                workspace,
                properties,
            });
        }

        None
    }
}
