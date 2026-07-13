//! Physical scan plan construction
//!
//! Builds concrete PhysicalPlan nodes for each scan strategy based on the
//! selected canonical predicate. Handles remaining filter wrapping and
//! limit pushdown.

use super::super::{
    CanonicalPredicate, ComparisonOp, Error, Expr, Literal, PhysicalPlan, PhysicalPlanner,
    PlanContext, SortExpr, TableSchema, TypedExpr,
};
use raisin_sql::analyzer::{BinaryOperator, DataType};
use std::sync::Arc;

impl PhysicalPlanner {
    /// Build a scan plan from the selected best predicate
    ///
    /// Creates the appropriate PhysicalPlan variant and wraps with a Filter
    /// node if there are remaining predicates not covered by the index.
    pub(in super::super) fn build_scan_from_predicate(
        &self,
        best_predicate: &CanonicalPredicate,
        canonical: &[CanonicalPredicate],
        table: &str,
        alias: &Option<String>,
        schema: Arc<TableSchema>,
        workspace: &str,
        branch: &str,
        projection: Option<Vec<String>>,
        context: &PlanContext,
    ) -> Result<PhysicalPlan, Error> {
        match best_predicate {
            CanonicalPredicate::ColumnEq { .. } | CanonicalPredicate::JsonPropertyEq { .. } => self
                .build_property_index_scan(
                    canonical, table, alias, workspace, branch, projection, context,
                ),
            CanonicalPredicate::ChildOf { ref parent_path } => self.build_child_of_scan(
                parent_path,
                canonical,
                table,
                alias,
                workspace,
                branch,
                projection,
            ),
            CanonicalPredicate::DescendantOf {
                ref parent_path,
                max_depth,
            } => self.build_descendant_of_scan(
                parent_path,
                *max_depth,
                canonical,
                table,
                alias,
                workspace,
                branch,
                projection,
            ),
            CanonicalPredicate::References {
                ref target_workspace,
                ref target_path,
            } => self.build_reference_scan(
                target_workspace,
                target_path,
                canonical,
                table,
                alias,
                workspace,
                branch,
                projection,
                context,
            ),
            CanonicalPredicate::PrefixRange { .. } => {
                self.build_prefix_scan(canonical, table, alias, workspace, branch, projection)
            }
            CanonicalPredicate::RangeCompare { .. }
            | CanonicalPredicate::JsonPropertyRange { .. } => self.build_range_scan(
                best_predicate,
                canonical,
                table,
                alias,
                schema,
                workspace,
                branch,
                projection,
                context,
            ),
            CanonicalPredicate::PropertyPrefixRange {
                table: _,
                column,
                prefix,
            } => self.build_property_prefix_scan(
                column, prefix, canonical, table, alias, schema, workspace, branch, projection,
                context,
            ),
            CanonicalPredicate::SpatialDWithin {
                table: _,
                geometry_column: _,
                property_name,
                center_lon,
                center_lat,
                radius_meters,
            } => self.build_spatial_scan(
                property_name,
                *center_lon,
                *center_lat,
                *radius_meters,
                canonical,
                table,
                alias,
                workspace,
                branch,
                projection,
                context,
            ),
            _ => {
                // Shouldn't reach here given our filtering above
                Ok(self.build_fallback_table_scan(
                    canonical, table, alias, schema, workspace, branch, None, projection,
                ))
            }
        }
    }

    fn build_property_index_scan(
        &self,
        canonical: &[CanonicalPredicate],
        table: &str,
        alias: &Option<String>,
        workspace: &str,
        branch: &str,
        projection: Option<Vec<String>>,
        context: &PlanContext,
    ) -> Result<PhysicalPlan, Error> {
        if let Some((prop_name, prop_value)) = self.extract_property_predicate(canonical) {
            // JSON property equalities (JsonPropertyEq) stay in the residual
            // filter even though they drive the scan: the property index keys
            // hashed values (collisions possible) and pre-fix databases may
            // carry stale old-value entries, so the fetched row is re-verified.
            // (The nodes repository's own find_by_property does the same
            // re-check.)
            //
            // Pseudo-property equalities (node_type, archetype, name, ... —
            // prop_name is "__"-prefixed) ARE removed: their index keys embed
            // the RAW value (no hash, no collisions) and value changes
            // tombstone the old entry, so the scan is exact — keeping them
            // filter-free preserves the COUNT(*) index pushdown.
            let remaining: Vec<_> = if prop_name.starts_with("__") {
                canonical
                    .iter()
                    .filter(|p| {
                        !matches!(
                            p,
                            CanonicalPredicate::ColumnEq { column, .. }
                                if format!("__{}", column.to_lowercase()) == prop_name
                        )
                    })
                    .cloned()
                    .collect()
            } else {
                canonical.to_vec()
            };
            let remaining_filter = self.combine_canonical_predicates(&remaining);

            let mut scan = PhysicalPlan::PropertyIndexScan {
                tenant_id: self.default_tenant_id.to_string(),
                repo_id: self.default_repo_id.to_string(),
                branch: branch.to_string(),
                workspace: workspace.to_string(),
                table: table.to_string(),
                alias: alias.clone(),
                property_name: prop_name,
                property_value: prop_value,
                projection,
                limit: context.limit,
            };

            if let Some(filter_expr) = remaining_filter {
                scan = PhysicalPlan::Filter {
                    input: Box::new(scan),
                    predicates: vec![filter_expr],
                };
            }

            return Ok(scan);
        }
        Err(Error::Validation(
            "Failed to extract property predicate".to_string(),
        ))
    }

    fn build_child_of_scan(
        &self,
        parent_path: &str,
        canonical: &[CanonicalPredicate],
        table: &str,
        alias: &Option<String>,
        workspace: &str,
        branch: &str,
        projection: Option<Vec<String>>,
    ) -> Result<PhysicalPlan, Error> {
        // Remove only the ChildOf predicate this scan actually guarantees; a
        // ChildOf over a DIFFERENT parent must stay a row-level filter.
        let remaining: Vec<_> = canonical
            .iter()
            .filter(|p| {
                !matches!(p, CanonicalPredicate::ChildOf { parent_path: pp } if pp == parent_path)
            })
            .cloned()
            .collect();

        let remaining_filter = self.combine_canonical_predicates(&remaining);

        let path_prefix = if parent_path == "/" {
            "/".to_string()
        } else {
            format!("{}/", parent_path.trim_end_matches('/'))
        };

        let mut scan = PhysicalPlan::PrefixScan {
            tenant_id: self.default_tenant_id.to_string(),
            repo_id: self.default_repo_id.to_string(),
            branch: branch.to_string(),
            workspace: workspace.to_string(),
            table: table.to_string(),
            alias: alias.clone(),
            path_prefix,
            projection,
            direct_children_only: true,
            limit: None,
        };

        if let Some(filter_expr) = remaining_filter {
            scan = PhysicalPlan::Filter {
                input: Box::new(scan),
                predicates: vec![filter_expr],
            };
        }

        Ok(scan)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_descendant_of_scan(
        &self,
        parent_path: &str,
        max_depth: Option<i64>,
        canonical: &[CanonicalPredicate],
        table: &str,
        alias: &Option<String>,
        workspace: &str,
        branch: &str,
        projection: Option<Vec<String>>,
    ) -> Result<PhysicalPlan, Error> {
        // Remove only the DescendantOf predicate this scan guarantees (matching
        // parent path AND depth; the depth bound is re-added as a filter below).
        // A DescendantOf over a DIFFERENT parent/depth must stay a row-level
        // filter.
        let remaining: Vec<_> = canonical
            .iter()
            .filter(|p| {
                !matches!(
                    p,
                    CanonicalPredicate::DescendantOf { parent_path: pp, max_depth: md }
                        if pp == parent_path && *md == max_depth
                )
            })
            .cloned()
            .collect();

        let mut remaining_filter = self.combine_canonical_predicates(&remaining);

        let path_prefix = if parent_path == "/" {
            "/".to_string()
        } else {
            format!("{}/", parent_path.trim_end_matches('/'))
        };

        // If max_depth is specified, add a depth filter
        if let Some(depth) = max_depth {
            use raisin_sql::analyzer::{DataType, FunctionCategory, FunctionSignature};

            let depth_filter = TypedExpr::new(
                Expr::Function {
                    name: "DESCENDANT_OF".to_string(),
                    args: vec![
                        TypedExpr::literal(Literal::Path(parent_path.to_string())),
                        TypedExpr::literal(Literal::BigInt(depth)),
                    ],
                    signature: FunctionSignature {
                        name: "DESCENDANT_OF".to_string(),
                        params: vec![DataType::Path, DataType::BigInt],
                        return_type: DataType::Boolean,
                        is_deterministic: true,
                        category: FunctionCategory::Hierarchy,
                    },
                    filter: None,
                },
                DataType::Boolean,
            );

            remaining_filter = match remaining_filter {
                Some(existing) => Some(TypedExpr::new(
                    Expr::BinaryOp {
                        left: Box::new(existing),
                        op: BinaryOperator::And,
                        right: Box::new(depth_filter),
                    },
                    DataType::Boolean,
                )),
                None => Some(depth_filter),
            };
        }

        let mut scan = PhysicalPlan::PrefixScan {
            tenant_id: self.default_tenant_id.to_string(),
            repo_id: self.default_repo_id.to_string(),
            branch: branch.to_string(),
            workspace: workspace.to_string(),
            table: table.to_string(),
            alias: alias.clone(),
            path_prefix,
            projection,
            direct_children_only: false,
            limit: None,
        };

        if let Some(filter_expr) = remaining_filter {
            scan = PhysicalPlan::Filter {
                input: Box::new(scan),
                predicates: vec![filter_expr],
            };
        }

        Ok(scan)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_reference_scan(
        &self,
        target_workspace: &str,
        target_path: &str,
        canonical: &[CanonicalPredicate],
        table: &str,
        alias: &Option<String>,
        workspace: &str,
        branch: &str,
        projection: Option<Vec<String>>,
        context: &PlanContext,
    ) -> Result<PhysicalPlan, Error> {
        // Remove only the References predicate this scan guarantees; a
        // References over a DIFFERENT target must stay a row-level filter.
        let remaining: Vec<_> = canonical
            .iter()
            .filter(|p| {
                !matches!(
                    p,
                    CanonicalPredicate::References { target_workspace: tw, target_path: tp }
                        if tw == target_workspace && tp == target_path
                )
            })
            .cloned()
            .collect();

        let remaining_filter = self.combine_canonical_predicates(&remaining);

        let mut scan = PhysicalPlan::ReferenceIndexScan {
            tenant_id: self.default_tenant_id.to_string(),
            repo_id: self.default_repo_id.to_string(),
            branch: branch.to_string(),
            workspace: workspace.to_string(),
            table: table.to_string(),
            alias: alias.clone(),
            target_workspace: target_workspace.to_string(),
            target_path: target_path.to_string(),
            projection,
            limit: context.limit,
        };

        if let Some(filter_expr) = remaining_filter {
            scan = PhysicalPlan::Filter {
                input: Box::new(scan),
                predicates: vec![filter_expr],
            };
        }

        Ok(scan)
    }

    fn build_prefix_scan(
        &self,
        canonical: &[CanonicalPredicate],
        table: &str,
        alias: &Option<String>,
        workspace: &str,
        branch: &str,
        projection: Option<Vec<String>>,
    ) -> Result<PhysicalPlan, Error> {
        if let Some(prefix) = self.extract_prefix_predicate(canonical) {
            // Keep ALL PrefixRange predicates as row-level PATH_STARTS_WITH
            // filters. The prefix may end in a NAME fragment (e.g. LIKE
            // '/job/t-%'), for which the executor scans a superset (the
            // containing directory / raw path-index prefix); the residual
            // filter re-applies the exact string-prefix semantics, so any
            // scan imprecision is filtered. It also excludes the parent row
            // itself for '/parent/'-style prefixes, and applies additional
            // PrefixRange predicates beyond the one driving the scan.
            let remaining = canonical.to_vec();

            let has_depth_predicate = remaining
                .iter()
                .any(|p| matches!(p, CanonicalPredicate::DepthEq { .. }));

            let remaining_filter = self.combine_canonical_predicates(&remaining);

            let mut scan = PhysicalPlan::PrefixScan {
                tenant_id: self.default_tenant_id.to_string(),
                repo_id: self.default_repo_id.to_string(),
                branch: branch.to_string(),
                workspace: workspace.to_string(),
                table: table.to_string(),
                alias: alias.clone(),
                path_prefix: prefix,
                projection,
                direct_children_only: has_depth_predicate,
                limit: None,
            };

            if let Some(filter_expr) = remaining_filter {
                scan = PhysicalPlan::Filter {
                    input: Box::new(scan),
                    predicates: vec![filter_expr],
                };
            }

            return Ok(scan);
        }
        Err(Error::Validation(
            "Failed to extract prefix predicate".to_string(),
        ))
    }

    /// Resolve the indexed property name for a range predicate, and encode its
    /// bound value in the property-index key encoding.
    ///
    /// Returns `None` when the predicate is not a range on an indexable target.
    fn range_target_and_bound(
        &self,
        pred: &CanonicalPredicate,
    ) -> Option<(String, ComparisonOp, String)> {
        match pred {
            CanonicalPredicate::RangeCompare {
                column, op, value, ..
            } => {
                let property_name = match column.to_lowercase().as_str() {
                    "created_at" => "__created_at",
                    "updated_at" => "__updated_at",
                    _ => return None,
                };
                let lit = self.evaluate_constant_expr(value)?;
                let encoded = match lit {
                    Literal::Timestamp(ts) => {
                        let nanos = ts.timestamp_nanos_opt().unwrap_or(0);
                        format!("{:020}", nanos as i128)
                    }
                    Literal::Int(i) => format!("{:020}", i),
                    _ => return None,
                };
                Some((property_name.to_string(), *op, encoded))
            }
            // JSON property ranges compare raw strings — the property index
            // stores `hash_property_value` (the raw string for text values), so
            // the bound is the literal itself.
            CanonicalPredicate::JsonPropertyRange { key, op, value, .. } => {
                Some((key.clone(), *op, value.clone()))
            }
            _ => None,
        }
    }

    /// Build a PropertyRangeScan for a range predicate (timestamp column or JSON
    /// property).
    ///
    /// Merges ALL range predicates on the same property into combined
    /// lower/upper bounds (so `a >= x AND a <= y` and BETWEEN apply both bounds),
    /// and strips ONLY the consumed predicates from the residual filter — range
    /// predicates on other columns stay as row-level filters.
    #[allow(clippy::too_many_arguments)]
    fn build_range_scan(
        &self,
        best_predicate: &CanonicalPredicate,
        canonical: &[CanonicalPredicate],
        table: &str,
        alias: &Option<String>,
        schema: Arc<TableSchema>,
        workspace: &str,
        branch: &str,
        projection: Option<Vec<String>>,
        context: &PlanContext,
    ) -> Result<PhysicalPlan, Error> {
        let (property_name, _, _) =
            self.range_target_and_bound(best_predicate).ok_or_else(|| {
                Error::Validation("Range scan not supported for this predicate".to_string())
            })?;

        // Merge every range predicate on the same property; remember which
        // canonical entries were consumed so only those leave the residual.
        let mut lower_bound: Option<(String, bool)> = None;
        let mut upper_bound: Option<(String, bool)> = None;
        let mut consumed = vec![false; canonical.len()];

        for (i, pred) in canonical.iter().enumerate() {
            let Some((prop, op, encoded)) = self.range_target_and_bound(pred) else {
                continue;
            };
            if prop != property_name {
                continue;
            }
            let inclusive = op.is_inclusive();
            if op.is_lower_bound() {
                // Keep the tightest (greatest) lower bound.
                let tighter = match &lower_bound {
                    None => true,
                    Some((existing, existing_incl)) => {
                        encoded > *existing
                            || (encoded == *existing && *existing_incl && !inclusive)
                    }
                };
                if tighter {
                    lower_bound = Some((encoded, inclusive));
                }
            } else {
                // Keep the tightest (smallest) upper bound.
                let tighter = match &upper_bound {
                    None => true,
                    Some((existing, existing_incl)) => {
                        encoded < *existing
                            || (encoded == *existing && *existing_incl && !inclusive)
                    }
                };
                if tighter {
                    upper_bound = Some((encoded, inclusive));
                }
            }
            consumed[i] = true;
        }

        let remaining: Vec<_> = canonical
            .iter()
            .enumerate()
            .filter(|(i, _)| !consumed[*i])
            .map(|(_, p)| p.clone())
            .collect();

        let remaining_filter = self.combine_canonical_predicates(&remaining);
        // Scan direction: forward when a lower bound exists (or both), reverse
        // for upper-bound-only scans (matches the previous single-bound behavior).
        let ascending = lower_bound.is_some();

        let scan = PhysicalPlan::PropertyRangeScan {
            tenant_id: self.default_tenant_id.to_string(),
            repo_id: self.default_repo_id.to_string(),
            branch: branch.to_string(),
            workspace: workspace.to_string(),
            table: table.to_string(),
            alias: alias.clone(),
            schema,
            projection,
            filter: remaining_filter,
            property_name,
            lower_bound,
            upper_bound,
            ascending,
            limit: context.limit,
        };

        Ok(scan)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_property_prefix_scan(
        &self,
        column: &str,
        prefix: &str,
        canonical: &[CanonicalPredicate],
        table: &str,
        alias: &Option<String>,
        schema: Arc<TableSchema>,
        workspace: &str,
        branch: &str,
        projection: Option<Vec<String>>,
        context: &PlanContext,
    ) -> Result<PhysicalPlan, Error> {
        let property_name = match column.to_lowercase().as_str() {
            "node_type" => "__node_type".to_string(),
            other => other.to_string(),
        };

        let lower_value = prefix.to_string();
        let upper_value = {
            let mut chars: Vec<char> = prefix.chars().collect();
            if let Some(last) = chars.last_mut() {
                *last = char::from_u32(*last as u32 + 1).unwrap_or(*last);
            }
            chars.into_iter().collect::<String>()
        };

        // Keep every predicate (including the driving PropertyPrefixRange) as a
        // row-level residual filter — same reasoning as build_property_index_scan:
        // the range scan is an access path, the filter is the source of truth.
        let remaining = canonical.to_vec();
        let remaining_filter = self.combine_canonical_predicates(&remaining);

        let scan = PhysicalPlan::PropertyRangeScan {
            tenant_id: self.default_tenant_id.to_string(),
            repo_id: self.default_repo_id.to_string(),
            branch: branch.to_string(),
            workspace: workspace.to_string(),
            table: table.to_string(),
            alias: alias.clone(),
            schema,
            projection,
            filter: remaining_filter,
            property_name,
            lower_bound: Some((lower_value, true)),
            upper_bound: Some((upper_value, false)),
            ascending: true,
            limit: context.limit,
        };

        Ok(scan)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_spatial_scan(
        &self,
        property_name: &str,
        center_lon: f64,
        center_lat: f64,
        radius_meters: f64,
        canonical: &[CanonicalPredicate],
        table: &str,
        alias: &Option<String>,
        workspace: &str,
        branch: &str,
        projection: Option<Vec<String>>,
        context: &PlanContext,
    ) -> Result<PhysicalPlan, Error> {
        let remaining: Vec<_> = canonical
            .iter()
            .filter(|p| !matches!(p, CanonicalPredicate::SpatialDWithin { .. }))
            .cloned()
            .collect();

        let remaining_filter = self.combine_canonical_predicates(&remaining);

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
            limit: context.limit,
        };

        if let Some(filter_expr) = remaining_filter {
            scan = PhysicalPlan::Filter {
                input: Box::new(scan),
                predicates: vec![filter_expr],
            };
        }

        tracing::info!(
            "   Using SpatialDistanceScan for ST_DWithin: property='{}', center=({}, {}), radius={}m",
            property_name, center_lon, center_lat, radius_meters
        );

        Ok(scan)
    }

    /// Build a fallback TableScan when no index is suitable
    pub(in super::super) fn build_fallback_table_scan(
        &self,
        canonical: &[CanonicalPredicate],
        table: &str,
        alias: &Option<String>,
        schema: Arc<TableSchema>,
        workspace: &str,
        branch: &str,
        filter: Option<TypedExpr>,
        projection: Option<Vec<String>>,
    ) -> PhysicalPlan {
        let reason = self.determine_scan_reason(canonical);
        tracing::warn!(
            table = %table,
            workspace = %workspace,
            reason = %reason,
            "Query degraded to a full TableScan (no usable index for its predicates); \
             this is slow under load — consider an indexed predicate (path/id =, \
             node_type/property =, or an IN over those)"
        );
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
            reason,
        }
    }
}
