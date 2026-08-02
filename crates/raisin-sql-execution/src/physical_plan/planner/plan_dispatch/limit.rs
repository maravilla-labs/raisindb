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
                // Try spatial k-NN first: ORDER BY ST_DISTANCE(...) LIMIT k. It is
                // recognised by geometry syntax and cannot collide with the vector
                // operators below, so the order between the two is arbitrary —
                // but a bounded index scan beats TopN either way.
                if let Some(spatial_plan) =
                    self.try_plan_spatial_knn(sort_input, sort_exprs, limit)?
                {
                    return Ok(spatial_plan);
                }

                // Try vector k-NN optimisation
                if let Some(vector_plan) =
                    self.try_plan_vector_knn(sort_input, sort_exprs, limit)?
                {
                    return Ok(vector_plan);
                }

                // Use TopN optimization (fallback for ORDER BY ... LIMIT)
                //
                // The child MUST be planned with the ORDER BY still in context.
                // A bare `with_limit` tells every scan below "there is no ORDER
                // BY", so a scan claims its OWN order, bounds itself in that
                // order, and the TopN above then sorts the wrong k rows — e.g.
                // `CHILD_OF('/x') ORDER BY name LIMIT 10` would return the first
                // ten children in EDITORIAL order, sorted by name. This mirrors
                // the propagation in the `LogicalPlan::Sort` arm, which exists
                // for exactly this reason.
                let mut topn_context = PlanContext::with_limit(limit);
                if let Some(first_sort) = sort_exprs.first() {
                    let is_asc = first_sort.ascending;
                    topn_context = topn_context.with_order_by_expr(first_sort.expr.clone(), is_asc);
                    if let Some(column_name) = Self::extract_column_name(&first_sort.expr) {
                        topn_context = topn_context.with_order_by(column_name, is_asc);
                    }
                }

                let input_plan = self.plan_with_context(sort_input, &topn_context)?;

                // The scan already emits the requested order, so re-sorting is
                // pure cost. A plain Limit preserves both the order and the bound.
                if Self::sort_is_satisfied_by_scan(sort_exprs, &input_plan) {
                    tracing::info!("⚡ Eliding TopN: scan already emits the requested order");
                    return Ok(PhysicalPlan::Limit {
                        input: Box::new(input_plan),
                        limit,
                        offset: 0,
                    });
                }

                return Ok(PhysicalPlan::TopN {
                    input: Box::new(input_plan),
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
        if Self::set_scan_limit(plan, limit, false) {
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

    /// Attempt to bound a scan operator anywhere in the physical plan subtree
    /// (direct scan, filter-wrapped, or project-wrapped).  Returns `true` if a
    /// scan was found and mutated.
    ///
    /// `filtered` becomes true once the walk crosses a `Filter`, i.e. rows can be
    /// discarded ABOVE the scan. An exact bound is unsafe there — the scan would
    /// hand up fewer rows than the query asked for — so those keep the
    /// conservative buffer. With no filter in between, the query's own `limit`
    /// IS the correct bound.
    ///
    /// This must never RAISE a bound the planner already chose. `build_child_of_scan`
    /// and friends compute an exact, order-aware limit; overwriting it with a
    /// large constant is what made LIMIT pushdown inert — the planner asked
    /// storage for 10 children and this function changed it to 200_000.
    fn set_scan_limit(plan: &mut PhysicalPlan, limit: usize, filtered: bool) -> bool {
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
                let candidate = if filtered { SCAN_LIMIT_BUFFER } else { limit };
                let bounded = match *scan_limit {
                    Some(existing) => existing.min(candidate),
                    None => candidate,
                };
                *scan_limit = Some(bounded);
                tracing::debug!(
                    "Bounded scan to {} (query LIMIT {}, filtered={})",
                    bounded,
                    limit,
                    filtered
                );
                true
            }

            // Pattern 2: Project-wrapped scans (including Project { Filter { Scan } }).
            // A projection never discards rows, so an exact bound stays exact.
            PhysicalPlan::Project {
                input: project_input,
                ..
            } => Self::set_scan_limit(project_input, limit, filtered),

            // Pattern 3: Filter-wrapped scans. Rows can be dropped above the
            // scan, so from here down only the buffer is safe.
            PhysicalPlan::Filter {
                input: filter_input,
                ..
            } => Self::set_scan_limit(filter_input, limit, true),

            _ => false,
        }
    }
}
