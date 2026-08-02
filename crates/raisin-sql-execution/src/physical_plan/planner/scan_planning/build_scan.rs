//! Physical scan plan construction
//!
//! Builds concrete PhysicalPlan nodes for each scan strategy based on the
//! selected canonical predicate. Handles remaining filter wrapping and
//! limit pushdown.

use super::super::{
    CanonicalPredicate, ComparisonOp, Error, Expr, Literal, OrderCursor, PhysicalPlan,
    PhysicalPlanner, PlanContext, SortExpr, TableSchema, TypedExpr,
};
use raisin_sql::analyzer::{BinaryOperator, DataType};
use std::sync::Arc;

/// An editorial-order keyset cursor lifted out of a WHERE clause, together with
/// the column it came from so the residual filter can drop exactly that bound.
struct OrderKeyset {
    column: &'static str,
    cursor: OrderCursor,
}

impl OrderKeyset {
    /// True if `predicate` is the bound this cursor already enforces, and can
    /// therefore be dropped from the residual filter.
    fn covers(&self, predicate: &CanonicalPredicate) -> bool {
        let expected = match &self.cursor {
            OrderCursor::Label(value) | OrderCursor::TreeOrder(value) => value,
        };
        matches!(
            predicate,
            CanonicalPredicate::RangeCompare {
                column,
                op: ComparisonOp::Gt,
                value,
                ..
            } if column.eq_ignore_ascii_case(self.column)
                && matches!(&value.expr, Expr::Literal(Literal::Text(text)) if text == expected)
        )
    }
}

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
                context,
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
                context,
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
                property_name,
                center_lon,
                center_lat,
                radius_meters,
                exact,
                ..
            } => self.build_spatial_scan(
                property_name,
                *center_lon,
                *center_lat,
                *radius_meters,
                *exact,
                canonical,
                table,
                alias,
                schema,
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

            // This scan emits in property-index key order, which satisfies no
            // ORDER BY the query might have asked for, and a residual filter can
            // discard rows above it. Bounding it in either case truncates the
            // wrong rows or returns short. See `build_reference_scan`.
            let pushed_limit = if context.has_no_ordering() && remaining_filter.is_none() {
                context.limit
            } else {
                None
            };

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
                limit: pushed_limit,
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

    /// An editorial-order keyset cursor lifted out of the WHERE clause.
    ///
    /// `column` selects which ordering this scan can seek on: `__order` for a
    /// direct-children scan, `__tree_order` for a subtree walk.
    ///
    /// Only the strictly-exclusive forward form (`col > 'value'`) is turned into
    /// a seek, because that is exactly what the index cursor expresses. An
    /// inclusive (`>=`) or reverse (`<`) bound stays a row-level filter rather
    /// than being approximated — a keyset cursor that quietly shifted by one row
    /// would silently drop or duplicate a record.
    fn extract_order_keyset(
        canonical: &[CanonicalPredicate],
        column: &'static str,
    ) -> Option<OrderKeyset> {
        canonical.iter().find_map(|predicate| match predicate {
            CanonicalPredicate::RangeCompare {
                column: predicate_column,
                op: ComparisonOp::Gt,
                value,
                ..
            } if predicate_column.eq_ignore_ascii_case(column) => match &value.expr {
                Expr::Literal(Literal::Text(text)) => Some(OrderKeyset {
                    column,
                    cursor: match column {
                        "__tree_order" => OrderCursor::TreeOrder(text.clone()),
                        _ => OrderCursor::Label(text.clone()),
                    },
                }),
                _ => None,
            },
            _ => None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn build_child_of_scan(
        &self,
        parent_path: &str,
        canonical: &[CanonicalPredicate],
        table: &str,
        alias: &Option<String>,
        workspace: &str,
        branch: &str,
        projection: Option<Vec<String>>,
        context: &PlanContext,
    ) -> Result<PhysicalPlan, Error> {
        // A CHILD_OF scan walks the editorial-order index, so it can absorb an
        // `__order` keyset cursor directly into the seek.
        let order_cursor = Self::extract_order_keyset(canonical, "__order");

        // Remove only the predicates this scan actually guarantees. A ChildOf
        // over a DIFFERENT parent must stay a row-level filter, and an `__order`
        // bound is only dropped when the seek makes it exactly redundant.
        let remaining: Vec<_> = canonical
            .iter()
            .filter(|p| {
                !matches!(p, CanonicalPredicate::ChildOf { parent_path: pp } if pp == parent_path)
            })
            .filter(|p| !order_cursor.as_ref().is_some_and(|c| c.covers(p)))
            .cloned()
            .collect();

        let remaining_filter = self.combine_canonical_predicates(&remaining);

        let path_prefix = if parent_path == "/" {
            "/".to_string()
        } else {
            format!("{}/", parent_path.trim_end_matches('/'))
        };

        // The scan emits editorial order. It can therefore claim the query's
        // ORDER BY when that ORDER BY is `__order` (either direction), or when
        // there is no ORDER BY at all — in which case editorial order is exactly
        // the documented default for CHILD_OF.
        let (claims_editorial_order, order_descending) = match &context.order_by {
            Some((column, ascending)) if column.eq_ignore_ascii_case("__order") => {
                (true, !*ascending)
            }
            // No ORDER BY: editorial order is the documented default for
            // CHILD_OF, so the scan's own order is what the caller expects.
            None => (true, false),
            Some(_) => (false, false),
        };

        // LIMIT may only ride into the scan when the scan's order is the
        // requested order AND nothing above it can filter rows away; a residual
        // filter would otherwise consume part of the budget and return short.
        let pushed_limit = if claims_editorial_order && remaining_filter.is_none() {
            context.limit
        } else {
            None
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
            limit: pushed_limit,
            order_cursor: order_cursor.map(|keyset| keyset.cursor),
            order_descending,
            claims_editorial_order,
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
        context: &PlanContext,
    ) -> Result<PhysicalPlan, Error> {
        // A subtree scan walks in document order, so it can absorb an
        // `__tree_order` keyset cursor directly into the traversal resume.
        let order_cursor = Self::extract_order_keyset(canonical, "__tree_order");

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
            .filter(|p| !order_cursor.as_ref().is_some_and(|c| c.covers(p)))
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

        // The traversal emits document order, so it satisfies
        // `ORDER BY __tree_order ASC` natively. Ascending only — see
        // `order_descending` below.
        //
        // With NO ORDER BY at all, document order is the documented default for
        // a subtree scan (pre-order depth-first, matching table scans), so the
        // walk's own order is exactly what the caller expects and it is free to
        // bound itself. This mirrors `build_child_of_scan`, which has always
        // accepted the no-ORDER-BY case; without it a bare
        // `DESCENDANT_OF('/x') LIMIT 10` walked the ENTIRE subtree at ~2 RocksDB
        // seeks per node and threw all but ten rows away.
        //
        // `has_no_ordering` rather than `order_by.is_none()`: an ORDER BY whose
        // key is not a plain column name (e.g. `ST_DISTANCE(...)`) leaves
        // `order_by` empty but must still prevent the walk from truncating.
        let claims_editorial_order = matches!(
            context.order_by.as_ref(),
            Some((column, true)) if column.eq_ignore_ascii_case("__tree_order")
        ) || context.has_no_ordering();

        // As in build_child_of_scan: only bound the walk when its order is the
        // requested order and no residual filter can consume the budget.
        let pushed_limit = if claims_editorial_order && remaining_filter.is_none() {
            context.limit
        } else {
            None
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
            direct_children_only: false,
            limit: pushed_limit,
            order_cursor: order_cursor.map(|keyset| keyset.cursor),
            // A subtree walk is forward-only: document order has no meaningful
            // cheap reverse (it would mean reversing every sibling group at every
            // level), so DESC falls through to a Sort.
            order_descending: false,
            claims_editorial_order,
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

        // The reverse-reference index is ordered by SOURCE NODE ID, which is
        // arbitrary with respect to any ORDER BY the query asked for. Bounding
        // this scan is therefore only safe when nothing above it re-orders or
        // discards rows:
        //
        //   * an ORDER BY means the top-k must be chosen AFTER sorting, so
        //     truncating here returns an arbitrary subset (this is how
        //     `REFERENCES(..) ORDER BY published_at DESC LIMIT 200` came to
        //     return 200 referrers in UUID order rather than the newest 200);
        //   * a residual filter (DESCENDANT_OF, node_type, ...) discards rows
        //     above the scan, so an exact bound returns short.
        let pushed_limit = if context.has_no_ordering() && remaining_filter.is_none() {
            context.limit
        } else {
            None
        };

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
            limit: pushed_limit,
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
                // A PATH_STARTS_WITH scan is driven by the path index, and it
                // always keeps a residual filter, so it cannot claim editorial
                // order even when `has_depth_predicate` routes it through the
                // direct-children branch.
                order_cursor: None,
                order_descending: false,
                claims_editorial_order: false,
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
            // Emits in property-index order, which no ORDER BY above is known to
            // accept — a Sort/TopN would then re-sort a truncated, arbitrary set.
            //
            // NOTE: the executor passes this straight to `scan_property_range`,
            // so it bounds the FETCH, before the residual filter and RLS run.
            // A residual filter can therefore still return short here. That is
            // pre-existing and is fixed by the over-fetch-and-refill work, not
            // by this guard.
            limit: if context.has_no_ordering() {
                context.limit
            } else {
                None
            },
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
            // See build_property_range_scan: prefix order satisfies no ORDER BY
            // the query asked for, so bounding under one truncates arbitrarily.
            limit: if context.has_no_ordering() {
                context.limit
            } else {
                None
            },
        };

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
