//! Spatial scan construction, and the rule that decides when a spatial predicate
//! may leave the residual filter.
//!
//! Split out of `build_scan` because it carries the subsystem's central
//! correctness argument and deserves to be read on its own.

use super::super::{
    CanonicalPredicate, Error, Expr, Literal, PhysicalPlan, PhysicalPlanner, PlanContext,
    TableSchema,
};
use std::sync::Arc;

impl PhysicalPlanner {
    /// Build a `SpatialDistanceScan` for a proximity predicate.
    ///
    /// # THE INVARIANT
    ///
    /// **A predicate may be removed from the residual filter ONLY IF the chosen
    /// access path is a proven-complete, exact answer for it. In every other case
    /// the predicate stays, and the query returns correct rows even if slowly. A
    /// spatial query must never return FEWER rows than the truth.**
    ///
    /// Three conditions must all hold before `SpatialDWithin` may be stripped:
    ///
    /// 1. The index for this (workspace, property) is `Ready` — it has actually
    ///    been built. This is what the hardcoded `has_spatial_index() == true`
    ///    used to assert without knowing, and the reason an unpopulated index
    ///    returned zero rows silently instead of falling back.
    /// 2. Some indexed precision can cover the radius within the cell budget, so
    ///    the cell set the executor will build is a proven cover rather than a
    ///    partial one.
    /// 3. The predicate is `exact` — the scan window was not widened by reducing
    ///    a non-point query geometry to its centroid, and the comparison was not
    ///    a strict `ST_DISTANCE < r` whose boundary the scan's `<= r` includes.
    ///
    /// When any of them fails the caller has already refused to select this scan
    /// (see `collect_index_options`), so reaching here with a non-`Ready` index
    /// means only condition 2 or 3 failed: the scan is still a useful candidate
    /// source, and the predicate simply stays.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_spatial_scan(
        &self,
        property_name: &str,
        center_lon: f64,
        center_lat: f64,
        radius_meters: f64,
        exact: bool,
        canonical: &[CanonicalPredicate],
        table: &str,
        alias: &Option<String>,
        schema: Arc<TableSchema>,
        workspace: &str,
        branch: &str,
        projection: Option<Vec<String>>,
        context: &PlanContext,
    ) -> Result<PhysicalPlan, Error> {
        let availability = self.spatial_availability(workspace, branch, property_name);

        if !availability.is_ready() {
            // Should be unreachable — `collect_index_options` gates on the same
            // answer — but if a future caller forgets that gate, degrade loudly
            // instead of scanning an index that cannot answer.
            return Ok(self.build_spatial_fallback_scan(
                property_name,
                canonical,
                table,
                alias,
                schema,
                workspace,
                branch,
                projection,
                crate::physical_plan::catalog::explain_reason(&availability),
            ));
        }

        let precisions = availability.precisions().to_vec();
        let covering = crate::physical_plan::catalog::radius_is_covered(
            center_lon,
            center_lat,
            radius_meters,
            &precisions,
        );

        // Condition 1 + 2 + 3. Anything less keeps the predicate.
        let may_strip = exact && covering;

        let remaining: Vec<_> = if may_strip {
            canonical
                .iter()
                .filter(|p| !matches!(p, CanonicalPredicate::SpatialDWithin { .. }))
                .cloned()
                .collect()
        } else {
            canonical.to_vec()
        };

        let remaining_filter = self.combine_canonical_predicates(&remaining);

        // The scan emits ascending distance order, so it can claim the query's
        // ORDER BY when that ORDER BY is exactly ascending distance.
        let claims_distance_order =
            self.spatial_order_is_satisfied(context, property_name, center_lon, center_lat);

        // The discriminator pre-filter (e.g. floor = 'L2'). Purely a selectivity
        // device: it is derived from a SIBLING predicate that STAYS in the
        // residual filter, so a stale or absent bucket in the index costs a
        // missed optimisation, never a missed row.
        let bucket_eq = crate::physical_plan::catalog::bucket_property(&availability)
            .and_then(|bucket| Self::extract_bucket_value(canonical, bucket));

        // LIMIT may only ride into the scan when nothing above it can filter rows
        // away AND the scan's order is the requested order.
        //
        // Both halves are load-bearing and both were missing. A residual filter
        // above a bounded scan returns SHORT (the scan truncates to `limit`
        // candidates, the filter then discards some of them), and a pending
        // ORDER BY on anything but distance means the scan would truncate in
        // distance order and hand the Sort above it the wrong `limit` rows.
        let pushed_limit =
            if remaining_filter.is_none() && (claims_distance_order || context.has_no_ordering()) {
                context.limit
            } else {
                None
            };

        let mut scan = PhysicalPlan::SpatialDistanceScan {
            tenant_id: self.default_tenant_id.to_string(),
            repo_id: self.default_repo_id.to_string(),
            branch: branch.to_string(),
            workspace: workspace.to_string(),
            table: table.to_string(),
            alias: alias.clone(),
            property_name: property_name.to_string(),
            center_lon,
            center_lat,
            radius_meters,
            projection,
            limit: pushed_limit,
            claims_distance_order,
            precisions,
            bucket_eq,
        };

        if let Some(filter_expr) = remaining_filter {
            scan = PhysicalPlan::Filter {
                input: Box::new(scan),
                predicates: vec![filter_expr],
            };
        }

        tracing::info!(
            "   Using SpatialDistanceScan for ST_DWithin: property='{}', center=({}, {}), \
             radius={}m, exact={}, covering={}, predicate_stripped={}",
            property_name,
            center_lon,
            center_lat,
            radius_meters,
            exact,
            covering,
            may_strip
        );

        Ok(scan)
    }

    /// The value of the configured discriminator property among the sibling
    /// predicates, e.g. `("floor", "L2")` from
    /// `... AND properties->>'floor' = 'L2'`.
    ///
    /// Recognises both the JSON-property spelling and a plain column equality, so
    /// the floor filter works however it is written.
    pub(super) fn extract_bucket_value(
        canonical: &[CanonicalPredicate],
        bucket_property: &str,
    ) -> Option<(String, String)> {
        canonical.iter().find_map(|predicate| match predicate {
            CanonicalPredicate::JsonPropertyEq { key, value, .. }
                if key.eq_ignore_ascii_case(bucket_property) =>
            {
                let text = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                Some((bucket_property.to_string(), text))
            }
            CanonicalPredicate::ColumnEq { column, value, .. }
                if column.eq_ignore_ascii_case(bucket_property) =>
            {
                match &value.expr {
                    Expr::Literal(Literal::Text(text)) => {
                        Some((bucket_property.to_string(), text.clone()))
                    }
                    _ => None,
                }
            }
            _ => None,
        })
    }

    /// Whether the pending `ORDER BY` is exactly the ascending-distance order a
    /// spatial scan emits for this property and centre.
    pub(super) fn spatial_order_is_satisfied(
        &self,
        context: &PlanContext,
        property_name: &str,
        center_lon: f64,
        center_lat: f64,
    ) -> bool {
        let Some((expr, ascending)) = context.order_by_expr.as_ref() else {
            return false;
        };
        if !*ascending {
            return false;
        }
        // `ORDER BY __distance` refers to the column this scan emits.
        if matches!(
            context.order_by.as_ref().map(|(c, _)| c.as_str()),
            Some("__distance")
        ) {
            return true;
        }
        raisin_sql::optimizer::hierarchy_rewrite::extract_distance_order(expr).is_some_and(
            |order| {
                order.exact
                    && order.property_name == property_name
                    && order.center_lon == center_lon
                    && order.center_lat == center_lat
            },
        )
    }

    /// The visible spatial fallback: an ordinary scan with the spatial predicate
    /// retained as a row-level filter, annotated so EXPLAIN says what happened
    /// and what to do about it.
    ///
    /// Slow and correct, with a signpost. Never fast and wrong.
    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn build_spatial_fallback_scan(
        &self,
        property_name: &str,
        canonical: &[CanonicalPredicate],
        table: &str,
        alias: &Option<String>,
        schema: Arc<TableSchema>,
        workspace: &str,
        branch: &str,
        projection: Option<Vec<String>>,
        detail: String,
    ) -> PhysicalPlan {
        tracing::warn!(
            workspace = %workspace,
            property = %property_name,
            detail = %detail,
            "Spatial predicate NOT pushed to the spatial index; applying it per row \
             instead. Results are correct but the scan is full — run \
             `REBUILD SPATIAL INDEX FOR '{}' PROPERTY '{}'` (or \
             POST /api/admin/management/database/.../spatial/rebuild)",
            workspace,
            property_name
        );

        // EVERY predicate, including the spatial one, survives as a residual
        // filter. That is the whole point.
        let filter = self.combine_canonical_predicates(canonical);

        PhysicalPlan::TableScan {
            tenant_id: self.default_tenant_id.to_string(),
            repo_id: self.default_repo_id.to_string(),
            branch: branch.to_string(),
            workspace: workspace.to_string(),
            table: table.to_string(),
            alias: alias.clone(),
            schema,
            filter,
            projection,
            limit: None,
            reason: crate::physical_plan::operators::ScanReason::SpatialIndexUnusable {
                workspace: workspace.to_string(),
                property: property_name.to_string(),
                detail,
            },
        }
    }
}
