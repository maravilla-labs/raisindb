//! Limit planning and limit-pushdown optimizations
//!
//! Handles `LogicalPlan::Limit` dispatch, including:
//! - Property-ordered scan optimization (`try_plan_property_order`)
//! - Vector k-NN detection (delegated to `vector_knn`)
//! - TopN fallback for `ORDER BY ... LIMIT k`
//! - Limit pushdown into scan operators for early termination

use super::super::{
    Error, LogicalPlan, PhysicalPlan, PhysicalPlanner, PlanContext, SCAN_LIMIT_BUFFER,
};

impl PhysicalPlanner {
    /// Plan a `LogicalPlan::Limit` node.
    pub(in crate::physical_plan::planner) fn plan_limit(
        &self,
        input: &LogicalPlan,
        limit: usize,
        offset: usize,
        context: &PlanContext,
    ) -> Result<PhysicalPlan, Error> {
        // Try property-order optimisation first
        if limit != usize::MAX {
            if let Some(ordered_plan) = self.try_plan_property_order(input, limit, offset)? {
                return Ok(ordered_plan);
            }
        }

        // Check for VectorScan optimization: ORDER BY (vector_col <op> query) LIMIT k
        if offset == 0 {
            if let LogicalPlan::Sort {
                input: sort_input,
                sort_exprs,
            } = input
            {
                // Try vector k-NN optimisation
                if let Some(vector_plan) =
                    self.try_plan_vector_knn(sort_input, sort_exprs, limit)?
                {
                    return Ok(vector_plan);
                }

                // Use TopN optimization (fallback for ORDER BY ... LIMIT)
                let topn_context = PlanContext::with_limit(limit);
                return Ok(PhysicalPlan::TopN {
                    input: Box::new(self.plan_with_context(sort_input, &topn_context)?),
                    sort_exprs: sort_exprs.clone(),
                    limit,
                });
            }
        }

        // Regular Limit - propagate limit to child operators for scan selection
        let mut new_context = context.clone();
        if limit != usize::MAX {
            // When offset is present, scans need to return limit + offset rows
            // so the Limit operator can skip offset and still return limit rows
            let scan_limit = limit.saturating_add(offset);
            new_context.limit = Some(scan_limit);
            tracing::debug!(
                "Propagating LIMIT {} (limit={} + offset={}) to child operators",
                scan_limit,
                limit,
                offset
            );
        }
        let mut input_plan = self.plan_with_context(input, &new_context)?;

        // Optimization: Push LIMIT down into scan operators for early termination
        // Use SCAN_LIMIT_BUFFER to account for post-scan filtering
        if offset == 0 {
            if let Some(pushed) = Self::try_push_limit_into_scan(&mut input_plan, limit) {
                return Ok(pushed);
            }
        }

        Ok(PhysicalPlan::Limit {
            input: Box::new(input_plan),
            limit,
            offset,
        })
    }

    /// Try to push a LIMIT into the innermost scan operator of `plan`.
    ///
    /// Returns `Some(Limit { ... })` when the pushdown succeeded, `None` when
    /// no pushable scan was found (the caller should emit a regular Limit).
    fn try_push_limit_into_scan(plan: &mut PhysicalPlan, limit: usize) -> Option<PhysicalPlan> {
        if Self::set_scan_limit(plan, limit) {
            // Take ownership through a swap so we can wrap in Limit
            let mut owned = PhysicalPlan::Empty;
            std::mem::swap(plan, &mut owned);
            return Some(PhysicalPlan::Limit {
                input: Box::new(owned),
                limit,
                offset: 0,
            });
        }
        None
    }

    /// Attempt to set `SCAN_LIMIT_BUFFER` on a scan operator anywhere in the
    /// physical plan subtree (direct scan, filter-wrapped, or
    /// project-wrapped).  Returns `true` if a scan was found and mutated.
    fn set_scan_limit(plan: &mut PhysicalPlan, limit: usize) -> bool {
        match plan {
            // Pattern 1: Direct scans
            PhysicalPlan::PropertyIndexScan {
                limit: ref mut scan_limit,
                ..
            }
            | PhysicalPlan::TableScan {
                limit: ref mut scan_limit,
                ..
            }
            | PhysicalPlan::PrefixScan {
                limit: ref mut scan_limit,
                ..
            }
            | PhysicalPlan::NeighborsScan {
                limit: ref mut scan_limit,
                ..
            } => {
                *scan_limit = Some(SCAN_LIMIT_BUFFER);
                tracing::debug!(
                    "Pushed LIMIT {} (buffered to {}) into {} (direct)",
                    limit,
                    SCAN_LIMIT_BUFFER,
                    plan.describe()
                );
                true
            }

            // Pattern 2: Project-wrapped scans (including Project { Filter { Scan } })
            PhysicalPlan::Project {
                input: project_input,
                ..
            } => Self::set_scan_limit(project_input, limit),

            // Pattern 3: Filter-wrapped scans
            PhysicalPlan::Filter {
                input: filter_input,
                ..
            } => Self::set_scan_limit(filter_input, limit),

            _ => false,
        }
    }
}
