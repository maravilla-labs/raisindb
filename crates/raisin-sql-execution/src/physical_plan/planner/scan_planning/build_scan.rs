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
            CanonicalPredicate::RangeCompare {
                table: _,
                column,
                op,
                value,
            } => self.build_range_scan(
                column, op, value, canonical, table, alias, schema, workspace, branch, projection,
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
            let remaining = self.remove_property_predicate(canonical, &prop_name);
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
        let remaining: Vec<_> = canonical
            .iter()
            .filter(|p| !matches!(p, CanonicalPredicate::ChildOf { .. }))
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
        let remaining: Vec<_> = canonical
            .iter()
            .filter(|p| !matches!(p, CanonicalPredicate::DescendantOf { .. }))
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
        let remaining: Vec<_> = canonical
            .iter()
            .filter(|p| !matches!(p, CanonicalPredicate::References { .. }))
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
            let remaining = self.remove_prefix_predicate(canonical);

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

    #[allow(clippy::too_many_arguments)]
    fn build_range_scan(
        &self,
        column: &str,
        op: &ComparisonOp,
        value: &TypedExpr,
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
            "created_at" => "__created_at",
            "updated_at" => "__updated_at",
            _ => {
                return Err(Error::Validation(format!(
                    "Range scan not supported for column: {}",
                    column
                )))
            }
        };

        let bound_value = if let Some(lit) = self.evaluate_constant_expr(value) {
            match lit {
                Literal::Timestamp(ts) => {
                    let nanos = ts.timestamp_nanos_opt().unwrap_or(0);
                    format!("{:020}", nanos as i128)
                }
                Literal::Int(i) => format!("{:020}", i),
                _ => {
                    return Err(Error::Validation(format!(
                        "Unsupported literal type for range comparison: {:?}",
                        lit
                    )))
                }
            }
        } else {
            return Err(Error::Validation(
                "Could not evaluate constant expression for range comparison".to_string(),
            ));
        };

        let is_inclusive = op.is_inclusive();
        let (lower_bound, upper_bound) = if op.is_lower_bound() {
            (Some((bound_value, is_inclusive)), None)
        } else {
            (None, Some((bound_value, is_inclusive)))
        };

        let remaining: Vec<_> = canonical
            .iter()
            .filter(|p| !matches!(p, CanonicalPredicate::RangeCompare { .. }))
            .cloned()
            .collect();

        let remaining_filter = self.combine_canonical_predicates(&remaining);
        let ascending = op.is_lower_bound();

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
            property_name: property_name.to_string(),
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

        let remaining: Vec<_> = canonical
            .iter()
            .filter(|p| !matches!(p, CanonicalPredicate::PropertyPrefixRange { .. }))
            .cloned()
            .collect();

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
