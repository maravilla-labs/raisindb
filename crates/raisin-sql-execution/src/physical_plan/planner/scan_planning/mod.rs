//! Scan method selection
//!
//! Analyzes filter predicates to choose the optimal scan access method:
//! - TableScan, PrefixScan, PathIndexScan, NodeIdScan
//! - PropertyIndexScan, CompoundIndexScan, FullTextScan, VectorScan
//!
//! # Module Structure
//!
//! - `selectivity` - Selectivity estimation per predicate type
//! - `index_selection` - Best predicate selection with ordering heuristics
//! - `build_scan` - Physical scan plan construction for each strategy

mod build_scan;
mod index_selection;
mod selectivity;

use super::{
    CanonicalPredicate, Error, PhysicalPlan, PhysicalPlanner, PlanContext, TableSchema, TypedExpr,
};
use std::sync::Arc;

impl PhysicalPlanner {
    /// Plan a scan with filter, choosing the best access method
    ///
    /// Selection strategy:
    /// 1. Full-text search gets highest priority (special case)
    /// 2. Node ID equality (O(1) direct lookup)
    /// 3. Path equality (O(1) path index lookup)
    /// 4. Compound index scan (for ORDER BY + filter patterns)
    /// 5. Among remaining indexes, choose the most selective
    /// 6. TableScan as fallback with appropriate reason
    pub(super) fn plan_scan_with_filter(
        &self,
        table: &str,
        alias: &Option<String>,
        schema: Arc<TableSchema>,
        workspace: &str,
        branch: &str,
        filter: &TypedExpr,
        projection: Option<Vec<String>>,
        context: &PlanContext,
    ) -> Result<PhysicalPlan, Error> {
        let canonical = self.analyze_filter(filter)?;

        // Priority 1: Full-text search gets absolute priority
        if let Some((language, query, limit)) = self.extract_fulltext_predicate(&canonical) {
            if self.index_catalog.has_fulltext_index() {
                return Ok(PhysicalPlan::FullTextScan {
                    tenant_id: self.default_tenant_id.to_string(),
                    repo_id: self.default_repo_id.to_string(),
                    branch: branch.to_string(),
                    workspace: workspace.to_string(),
                    table: table.to_string(),
                    alias: alias.clone(),
                    language,
                    query,
                    limit,
                    projection,
                });
            }
        }

        // Priority 2: Node ID equality (id = 'uuid')
        if let Some(id_value) = self.extract_id_predicate(&canonical) {
            let remaining = self.remove_id_predicate(&canonical);
            let remaining_filter = self.combine_canonical_predicates(&remaining);

            let mut scan = PhysicalPlan::NodeIdScan {
                tenant_id: self.default_tenant_id.to_string(),
                repo_id: self.default_repo_id.to_string(),
                branch: branch.to_string(),
                workspace: workspace.to_string(),
                table: table.to_string(),
                alias: alias.clone(),
                node_id: id_value.clone(),
                projection: projection.clone(),
            };

            if let Some(filter_expr) = remaining_filter {
                scan = PhysicalPlan::Filter {
                    input: Box::new(scan),
                    predicates: vec![filter_expr],
                };
            }

            tracing::info!(
                "   Using NodeIdScan for direct node lookup: id='{}'",
                id_value
            );

            return Ok(scan);
        }

        // Priority 3: Path equality (path = '/exact/path')
        if let Some(path_value) = self.extract_path_predicate(&canonical) {
            if self.index_catalog.has_path_index() {
                let remaining = self.remove_path_predicate(&canonical);
                let remaining_filter = self.combine_canonical_predicates(&remaining);

                let mut scan = PhysicalPlan::PathIndexScan {
                    tenant_id: self.default_tenant_id.to_string(),
                    repo_id: self.default_repo_id.to_string(),
                    branch: branch.to_string(),
                    workspace: workspace.to_string(),
                    table: table.to_string(),
                    alias: alias.clone(),
                    path: path_value.clone(),
                    projection: projection.clone(),
                };

                if let Some(filter_expr) = remaining_filter {
                    scan = PhysicalPlan::Filter {
                        input: Box::new(scan),
                        predicates: vec![filter_expr],
                    };
                }

                tracing::info!(
                    "   Using PathIndexScan for exact path lookup: path='{}'",
                    path_value
                );

                return Ok(scan);
            }
        }

        // Priority 4: Compound index scan
        if let Some(scan) = self.try_compound_index_scan(
            &canonical,
            context,
            table,
            alias,
            workspace,
            branch,
            &projection,
        ) {
            return Ok(scan);
        }

        // Collect all available index options with their selectivity
        let index_options = self.collect_index_options(&canonical);

        // Check if parent operator wants path ordering
        let ordering_by_path = context
            .order_by
            .as_ref()
            .map(|(col, _)| col == "__path" || col == "path")
            .unwrap_or(false);

        // Select best predicate using heuristics
        let options_with_refs: Vec<(&CanonicalPredicate, f64)> = index_options
            .iter()
            .map(|(sel, pred)| (*pred, *sel))
            .collect();

        let best_predicate = self.select_best_predicate(&options_with_refs, ordering_by_path);

        if let Some(best) = best_predicate {
            return self.build_scan_from_predicate(
                best,
                &canonical,
                table,
                alias,
                schema.clone(),
                workspace,
                branch,
                projection,
                context,
            );
        }

        // Fallback: Table scan
        Ok(self.build_fallback_table_scan(
            &canonical,
            table,
            alias,
            schema,
            workspace,
            branch,
            Some(filter.clone()),
            projection,
        ))
    }

    /// Try to match a compound index scan
    fn try_compound_index_scan(
        &self,
        canonical: &[CanonicalPredicate],
        context: &PlanContext,
        table: &str,
        alias: &Option<String>,
        workspace: &str,
        branch: &str,
        projection: &Option<Vec<String>>,
    ) -> Option<PhysicalPlan> {
        if self.compound_indexes.is_empty() {
            return None;
        }

        let order_by_ref = context
            .order_by
            .as_ref()
            .map(|(col, asc)| (col.as_str(), *asc));

        let (index_name, equality_columns, ascending) =
            self.try_match_compound_index(canonical, order_by_ref)?;

        let used_props: std::collections::HashSet<String> = equality_columns
            .iter()
            .map(|(prop, _)| prop.clone())
            .collect();

        let remaining: Vec<_> = canonical
            .iter()
            .filter(|p| match p {
                CanonicalPredicate::ColumnEq { column, .. } => {
                    let prop = if column.eq_ignore_ascii_case("node_type") {
                        "__node_type"
                    } else {
                        column.as_str()
                    };
                    !used_props.contains(prop)
                }
                CanonicalPredicate::JsonPropertyEq { key, .. } => !used_props.contains(key),
                _ => true,
            })
            .cloned()
            .collect();

        let remaining_filter = self.combine_canonical_predicates(&remaining);

        tracing::info!(
            "   Using CompoundIndexScan: index='{}', {} equality columns, ascending={}",
            index_name,
            equality_columns.len(),
            ascending
        );

        Some(PhysicalPlan::CompoundIndexScan {
            tenant_id: self.default_tenant_id.to_string(),
            repo_id: self.default_repo_id.to_string(),
            branch: branch.to_string(),
            workspace: workspace.to_string(),
            table: table.to_string(),
            alias: alias.clone(),
            index_name,
            equality_columns,
            pre_sorted: true,
            ascending,
            projection: projection.clone(),
            filter: remaining_filter,
            limit: context.limit,
        })
    }

    /// Collect all available index options with their selectivity scores
    fn collect_index_options<'a>(
        &self,
        canonical: &'a [CanonicalPredicate],
    ) -> Vec<(f64, &'a CanonicalPredicate)> {
        let mut index_options = Vec::new();

        for pred in canonical {
            let selectivity = self.estimate_selectivity(pred);

            let has_index = match pred {
                CanonicalPredicate::PrefixRange { .. } => self.index_catalog.has_path_index(),
                CanonicalPredicate::ChildOf { .. } => true,
                CanonicalPredicate::DescendantOf { .. } => self.index_catalog.has_path_index(),
                CanonicalPredicate::ColumnEq { column, .. } => {
                    let col_lower = column.to_lowercase();
                    (col_lower == "node_type"
                        || col_lower == "created_at"
                        || col_lower == "updated_at")
                        && self.index_catalog.has_property_index()
                }
                CanonicalPredicate::JsonPropertyEq { .. } => {
                    self.index_catalog.has_property_index()
                }
                CanonicalPredicate::DepthEq { .. } => false,
                CanonicalPredicate::RangeCompare { column, .. } => {
                    let col_lower = column.to_lowercase();
                    (col_lower == "created_at" || col_lower == "updated_at")
                        && self.index_catalog.has_property_index()
                }
                CanonicalPredicate::PropertyPrefixRange { .. } => {
                    self.index_catalog.has_property_index()
                }
                CanonicalPredicate::SpatialDWithin { .. } => self.index_catalog.has_spatial_index(),
                CanonicalPredicate::References { .. } => true,
                CanonicalPredicate::Other(_) => false,
            };

            if has_index {
                index_options.push((selectivity, pred));
            }
        }

        index_options
    }
}
