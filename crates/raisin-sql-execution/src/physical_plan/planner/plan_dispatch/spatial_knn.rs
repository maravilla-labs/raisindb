//! Spatial k-NN optimization within LIMIT planning.
//!
//! Converts `ORDER BY ST_DISTANCE(<geom>, <const>) [ASC] LIMIT k` into a
//! [`PhysicalPlan::SpatialKnnScan`]. Before this, `SpatialKnnScan` was
//! unreachable dead code — the variant, the executor and the storage method all
//! existed, but no planner site ever constructed one, so every nearest-neighbour
//! query was a full scan plus a TopN.
//!
//! # Why only ASC with a LIMIT
//!
//! The index walks outward from the centre, so it produces nearest-first. That
//! gives a bounded access path for "the k nearest" and nothing at all for "the k
//! FARTHEST" (which needs every row) or for an unbounded distance sort (same).
//! Both fall through to TopN / Sort, which is correct.

use super::super::{Error, LogicalPlan, PhysicalPlan, PhysicalPlanner};
use raisin_sql::optimizer::hierarchy_rewrite::extract_distance_order;

impl PhysicalPlanner {
    /// Try to optimise `Limit { Sort { ... } }` into a `SpatialKnnScan`.
    ///
    /// Returns `Ok(None)` when the pattern does not apply, so the caller falls
    /// back to vector k-NN detection and then TopN.
    pub(in crate::physical_plan::planner) fn try_plan_spatial_knn(
        &self,
        sort_input: &LogicalPlan,
        sort_exprs: &[raisin_sql::logical_plan::SortExpr],
        limit: usize,
    ) -> Result<Option<PhysicalPlan>, Error> {
        if sort_exprs.len() != 1 || limit == usize::MAX {
            return Ok(None);
        }
        let sort = &sort_exprs[0];
        if !sort.ascending {
            return Ok(None);
        }

        // The ordering key must be an ST_DISTANCE against a constant POINT. A
        // non-point centre is `exact == false`: centre-of-envelope distance is not
        // the requested distance, so its order is not the requested order and we
        // must not claim it.
        let order = match extract_distance_order(&sort.expr) {
            Some(order) if order.exact => order,
            _ => return Ok(None),
        };

        // Look through a Project to find the scan — but REMEMBER it, because it
        // has to go back on top.
        //
        // A scan emits fully qualified column names (`places.name`, see
        // `node_to_row`'s "Column Naming" section); the `Project` above it is what
        // turns those into the names the SELECT list asked for. Returning the bare
        // `SpatialKnnScan` therefore produced the right ROWS under the wrong
        // COLUMN NAMES — `SELECT name ... ORDER BY ST_DISTANCE(...) LIMIT k` came
        // back with a `places.name` column and no `name`, which every client reads
        // as a null. Caught by
        // `spatial_query_test::order_by_st_distance_limit_k_is_ordered_and_index_backed`.
        let (scan_input, projection_exprs) = match sort_input {
            LogicalPlan::Project { input, exprs } => (input.as_ref(), Some(exprs.clone())),
            other => (other, None),
        };

        // A k-NN scan answers "the k nearest, unconditionally". A WHERE clause
        // would have to be applied AFTER the scan, and the scan has already
        // discarded everything past the k-th nearest — so the answer would be
        // "the matching subset of the k nearest", not "the k nearest matching".
        // Bail out and let TopN handle it rather than silently returning fewer
        // rows than asked for.
        //
        // The one filter shape that IS compatible is a `ST_DWITHIN` on the same
        // property and centre, which only bounds the radius; that is planned as a
        // distance scan with the Sort elided instead (see `build_spatial_scan`).
        let LogicalPlan::Scan {
            table,
            alias,
            workspace,
            branch_override,
            projection,
            filter,
            max_revision,
            ..
        } = scan_input
        else {
            return Ok(None);
        };

        // A read at an explicit historical revision must not touch the spatial
        // index: compaction prunes superseded entries beyond a retention window,
        // so the index is exact at HEAD and only approximate behind it. Falling
        // through to a full scan plus TopN is exact at any revision. See the
        // matching gate in `build_spatial_scan`.
        if max_revision.is_some() {
            tracing::debug!(
                "ORDER BY ST_DISTANCE not planned as SpatialKnnScan: the query reads at an                  explicit historical revision, where the pruned spatial index is not exact"
            );
            return Ok(None);
        }
        if filter.is_some() {
            tracing::debug!(
                "ST_DISTANCE ORDER BY not planned as SpatialKnnScan: the scan carries a \
                 filter, which would be applied after k-NN truncation and could return \
                 fewer than {} rows",
                limit
            );
            return Ok(None);
        }

        let workspace_name = workspace
            .clone()
            .unwrap_or_else(|| self.default_workspace.to_string());
        let effective_branch = branch_override
            .clone()
            .unwrap_or_else(|| self.default_branch.to_string());

        // Same fail-closed gate as the radius path: an unbuilt index must not be
        // asked for the nearest anything.
        let availability =
            self.spatial_availability(&workspace_name, &effective_branch, &order.property_name);
        if !availability.is_ready() {
            tracing::warn!(
                workspace = %workspace_name,
                property = %order.property_name,
                detail = %crate::physical_plan::catalog::explain_reason(&availability),
                "ORDER BY ST_DISTANCE will NOT use the spatial index; falling back to a \
                 full scan plus TopN (correct, but unbounded)"
            );
            return Ok(None);
        }

        tracing::info!(
            "   Using SpatialKnnScan: property='{}', center=({}, {}), k={}",
            order.property_name,
            order.center_lon,
            order.center_lat,
            limit
        );

        let scan = PhysicalPlan::SpatialKnnScan {
            tenant_id: self.default_tenant_id.to_string(),
            repo_id: self.default_repo_id.to_string(),
            branch: effective_branch,
            workspace: workspace_name,
            table: table.clone(),
            alias: alias.clone(),
            property_name: order.property_name,
            center_lon: order.center_lon,
            center_lat: order.center_lat,
            k: limit,
            projection: projection.clone(),
            claims_distance_order: true,
            precisions: availability.precisions().to_vec(),
        };

        // Put the SELECT list back on top. The Sort and the Limit are what this
        // scan replaces; the projection is not, and dropping it renames every
        // output column.
        Ok(Some(match projection_exprs {
            Some(exprs) => PhysicalPlan::Project {
                input: Box::new(scan),
                exprs,
            },
            None => scan,
        }))
    }
}
