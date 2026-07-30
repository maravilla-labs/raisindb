//! The spatial FALLBACK: the plan a spatial predicate takes when the index
//! cannot answer it exactly.
//!
//! Three separate conditions route here, and they are deliberately one code
//! path: an index that was never built, a radius no indexed precision covers,
//! and a read at an explicit historical revision (where compaction has pruned
//! what the index would have needed). A fourth arrives at EXECUTION time — the
//! per-cell scan budget — which is why `build_spatial_scan` builds one of these
//! eagerly and hands it to the executor to run instead of failing.
//!
//! Every one of them produces the same thing: a full scan with EVERY predicate
//! retained as a row-level filter, annotated so EXPLAIN says what happened and
//! what to do about it. Slow and correct, with a signpost. Never fast and wrong.

use super::super::{CanonicalPredicate, PhysicalPlan, PhysicalPlanner, TableSchema};
use std::sync::Arc;

impl PhysicalPlanner {
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

        self.spatial_fallback_plan(
            property_name,
            canonical,
            table,
            alias,
            schema,
            workspace,
            branch,
            projection,
            detail,
        )
    }

    /// The fallback plan itself, WITHOUT the "we are degrading" warning.
    ///
    /// Split from [`Self::build_spatial_fallback_scan`] because
    /// `build_spatial_scan` builds a fallback EAGERLY, to hand the executor
    /// somewhere to go if the per-cell budget is exhausted mid-scan. That plan is
    /// usually never run, so logging at construction would put a spurious
    /// "spatial index not used" warning in front of every successful index query
    /// — training operators to ignore the one message that matters.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn spatial_fallback_plan(
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
        // EVERY predicate, including the spatial one, survives as a residual
        // filter. That is the whole point.
        let filter = self.combine_canonical_predicates(canonical);
        let wants_annotation = Self::projection_wants_spatial_columns(&projection);

        let scan = PhysicalPlan::TableScan {
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
        };

        if !wants_annotation {
            return scan;
        }

        // The pseudo-columns only have an answer if we know WHERE the query
        // measured from. A fallback planned for a predicate whose centre we
        // cannot recover (there is none in `canonical`) leaves them NULL rather
        // than inventing a centre.
        match Self::spatial_center(canonical, property_name) {
            Some((center_lon, center_lat)) => PhysicalPlan::SpatialAnnotate {
                input: Box::new(scan),
                property_name: property_name.to_string(),
                center_lon,
                center_lat,
            },
            None => scan,
        }
    }

    /// Whether the query asked for `__distance` or `__matched_path`.
    ///
    /// `None` — no projection pruning ran, so every column is live — counts as
    /// "yes". Annotating when nobody asked costs one distance computation per
    /// surviving row on an already-full scan; NOT annotating when somebody did
    /// ask returns a silent NULL, which is the failure this whole item is about.
    pub(super) fn projection_wants_spatial_columns(projection: &Option<Vec<String>>) -> bool {
        match projection {
            None => true,
            Some(columns) => columns
                .iter()
                .any(|c| c == "__distance" || c == "__matched_path"),
        }
    }

    /// The `(lon, lat)` a spatial predicate on `property_name` measures from.
    pub(in super::super) fn spatial_center(
        canonical: &[CanonicalPredicate],
        property_name: &str,
    ) -> Option<(f64, f64)> {
        canonical.iter().find_map(|predicate| match predicate {
            CanonicalPredicate::SpatialDWithin {
                property_name: name,
                center_lon,
                center_lat,
                ..
            } if name == property_name => Some((*center_lon, *center_lat)),
            _ => None,
        })
    }
}
