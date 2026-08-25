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
//! - `build_spatial` - Spatial scan construction and the residual-filter rule

mod build_scan;
mod build_spatial;
mod build_spatial_fallback;
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
        self.plan_scan_from_canonical(
            &canonical,
            table,
            alias,
            schema,
            workspace,
            branch,
            Some(filter.clone()),
            projection,
            context,
        )
    }

    /// Plan a scan from already-canonicalized predicates.
    ///
    /// `fallback_filter` is the residual expression used when the scan degrades
    /// to a `TableScan` (the original filter at the top level, or the combined
    /// substituted predicates for a `Union` branch).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn plan_scan_from_canonical(
        &self,
        canonical: &[CanonicalPredicate],
        table: &str,
        alias: &Option<String>,
        schema: Arc<TableSchema>,
        workspace: &str,
        branch: &str,
        fallback_filter: Option<TypedExpr>,
        projection: Option<Vec<String>>,
        context: &PlanContext,
    ) -> Result<PhysicalPlan, Error> {
        // Priority 0: expand an indexable `col IN (...)` into a Union of
        // per-value equality scans (each reusing the existing equality builders).
        if let Some(union) = self.try_expand_in_union(
            canonical,
            table,
            alias,
            &schema,
            workspace,
            branch,
            &projection,
        )? {
            return Ok(union);
        }

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
        let index_options = self.collect_index_options(&canonical, workspace, branch);

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

        // Fallback: Table scan.
        //
        // When the query DID carry an index-shaped spatial predicate and we still
        // ended up here, the reason is spatial-specific and the operator needs to
        // hear it — a generic "no matching index" would hide the one thing they
        // can act on.
        if let Some(CanonicalPredicate::SpatialDWithin {
            property_name,
            radius_meters,
            ..
        }) = canonical
            .iter()
            .find(|p| matches!(p, CanonicalPredicate::SpatialDWithin { .. }))
        {
            let availability = self.spatial_availability(workspace, branch, property_name);
            let detail = if availability.is_ready() {
                format!(
                    "indexed precisions {:?} cannot cover a {}m radius within the cell budget",
                    availability.precisions(),
                    radius_meters
                )
            } else {
                crate::physical_plan::catalog::explain_reason(&availability)
            };
            return Ok(self.build_spatial_fallback_scan(
                property_name,
                canonical,
                table,
                alias,
                schema,
                workspace,
                branch,
                projection,
                detail,
            ));
        }

        Ok(self.build_fallback_table_scan(
            canonical,
            table,
            alias,
            schema,
            workspace,
            branch,
            fallback_filter,
            projection,
        ))
    }

    /// Expand an indexable `col IN (...)` predicate into a `Union` of per-value
    /// equality scans. Returns `Ok(None)` when no indexable `IN` predicate is
    /// present, or when any resulting branch would not be index-backed (in which
    /// case the caller falls back to a single scan — never worse than today).
    #[allow(clippy::too_many_arguments)]
    fn try_expand_in_union(
        &self,
        canonical: &[CanonicalPredicate],
        table: &str,
        alias: &Option<String>,
        schema: &Arc<TableSchema>,
        workspace: &str,
        branch: &str,
        projection: &Option<Vec<String>>,
    ) -> Result<Option<PhysicalPlan>, Error> {
        // Find the first `IN` predicate whose column is plausibly indexed. The
        // per-branch post-check below is the hard correctness guarantee; this is
        // just a cheap pre-filter.
        let idx = canonical.iter().position(|p| match p {
            CanonicalPredicate::ColumnIn { column, .. } => self.column_in_indexable(column),
            CanonicalPredicate::JsonPropertyIn { .. } => {
                self.index_catalog.has_property_index() || !self.compound_indexes.is_empty()
            }
            _ => false,
        });
        let idx = match idx {
            Some(i) => i,
            None => return Ok(None),
        };

        // Build the per-value equality predicates (de-duplicating the literal list).
        let eq_preds: Vec<CanonicalPredicate> = match &canonical[idx] {
            CanonicalPredicate::ColumnIn {
                table: t,
                column,
                values,
            } => {
                let mut seen = Vec::new();
                let mut preds = Vec::new();
                for v in values {
                    let key = format!("{:?}", v.expr);
                    if seen.contains(&key) {
                        continue;
                    }
                    seen.push(key);
                    preds.push(CanonicalPredicate::ColumnEq {
                        table: t.clone(),
                        column: column.clone(),
                        value: v.clone(),
                    });
                }
                preds
            }
            CanonicalPredicate::JsonPropertyIn {
                table: t,
                json_col,
                key,
                values,
            } => {
                let mut seen: Vec<serde_json::Value> = Vec::new();
                let mut preds = Vec::new();
                for v in values {
                    if seen.contains(v) {
                        continue;
                    }
                    seen.push(v.clone());
                    preds.push(CanonicalPredicate::JsonPropertyEq {
                        table: t.clone(),
                        json_col: json_col.clone(),
                        key: key.clone(),
                        value: v.clone(),
                    });
                }
                preds
            }
            _ => return Ok(None),
        };

        if eq_preds.is_empty() {
            return Ok(None);
        }

        // Other predicates (everything except the IN being expanded) apply to
        // every branch.
        let others: Vec<CanonicalPredicate> = canonical
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != idx)
            .map(|(_, p)| p.clone())
            .collect();

        // Branches must not inherit LIMIT/ORDER pushdown — a Limit/Sort above the
        // Union handles global ordering and bounding.
        let branch_ctx = PlanContext::empty();

        let mut branches = Vec::with_capacity(eq_preds.len());
        for eq in eq_preds {
            let mut sub = others.clone();
            sub.push(eq);
            let fb = self.combine_canonical_predicates(&sub);
            let plan = self.plan_scan_from_canonical(
                &sub,
                table,
                alias,
                schema.clone(),
                workspace,
                branch,
                fb,
                projection.clone(),
                &branch_ctx,
            )?;

            // Hard guarantee: only emit a Union if EVERY branch is index-backed.
            // Otherwise N table scans would be strictly worse than one, so abandon
            // and let the caller build a single (warned) TableScan.
            if Self::plan_has_table_scan(&plan) {
                return Ok(None);
            }
            branches.push(plan);
        }

        if branches.len() == 1 {
            return Ok(Some(branches.pop().unwrap()));
        }

        tracing::info!(
            "   Expanding IN(..) into Union of {} indexed scans",
            branches.len()
        );
        Ok(Some(PhysicalPlan::Union { inputs: branches }))
    }

    /// Whether a `col IN (...)` over `column` can plausibly use an index.
    fn column_in_indexable(&self, column: &str) -> bool {
        match column.to_lowercase().as_str() {
            "path" => self.index_catalog.has_path_index(),
            "id" => true,
            "node_type" | "archetype" | "name" | "created_by" | "updated_by" | "created_at"
            | "updated_at" => {
                self.index_catalog.has_property_index() || !self.compound_indexes.is_empty()
            }
            _ => false,
        }
    }

    /// Recursively test whether a plan contains a `TableScan` (i.e. is not fully
    /// index-backed).
    fn plan_has_table_scan(plan: &PhysicalPlan) -> bool {
        if matches!(plan, PhysicalPlan::TableScan { .. }) {
            return true;
        }
        plan.inputs().iter().any(|p| Self::plan_has_table_scan(p))
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

        let (index_name, equality_columns, ascending, claims_order) =
            self.try_match_compound_index(canonical, order_by_ref)?;

        // A declaration is not a built index. Consult the persisted build state
        // and DECLINE unless it says Ready for this exact declaration.
        //
        // This must sit between the match and the plan, never earlier: the match
        // is what tells us WHICH index we would use, and availability is
        // per-index. And it must not be skipped for speed — below this point the
        // matched equality predicates are removed from the residual filter, so a
        // scan over an empty or stale keyspace yields missing rows with nothing
        // downstream to catch it.
        let availability = self.compound_availability(workspace, branch, &index_name);
        if !availability.is_ready() {
            tracing::warn!(
                index = %index_name,
                workspace = %workspace,
                detail = %availability.explain_reason(),
                "compound index matched the query but is not usable; falling back to another access path"
            );
            return None;
        }

        let used_props: std::collections::HashSet<String> = equality_columns
            .iter()
            .map(|(prop, _, _)| prop.clone())
            .collect();

        // The parent path this scan is pinned to, if `__parent_path` was one of
        // the matched columns. Only the ChildOf that produced it may be dropped;
        // a ChildOf over a DIFFERENT parent is not guaranteed by the index and
        // must stay a row-level filter.
        let matched_parent_path: Option<&str> = equality_columns
            .iter()
            .find(|(prop, _, _)| prop == "__parent_path")
            .map(|(_, value, _)| value.as_str());

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
                // A ChildOf consumed as the `__parent_path` column is guaranteed
                // exactly by the index: the key stores the raw parent path, so
                // there is no hashing and no collision to re-verify. Keeping it
                // would also put a Filter between the Sort and the scan, which
                // blocks sort elision — the index would be matched and then not
                // used for what it was matched for.
                CanonicalPredicate::ChildOf { parent_path } => {
                    matched_parent_path
                        != Some(super::compound_index::normalize_parent_path(parent_path))
                }
                _ => true,
            })
            .cloned()
            .collect();

        let remaining_filter = self.combine_canonical_predicates(&remaining);

        tracing::info!(
            "   Using CompoundIndexScan: index='{}', {} equality columns, ascending={}, claims_order={}",
            index_name,
            equality_columns.len(),
            ascending,
            claims_order
        );

        // LIMIT may only be pushed into the scan when its iteration order is the
        // requested order (claims_order) or when no ORDER BY is pending —
        // otherwise the scan would truncate in the wrong order before the Sort
        // above re-orders.
        let pushed_limit = if claims_order || context.order_by.is_none() {
            context.limit
        } else {
            None
        };

        Some(PhysicalPlan::CompoundIndexScan {
            tenant_id: self.default_tenant_id.to_string(),
            repo_id: self.default_repo_id.to_string(),
            branch: branch.to_string(),
            workspace: workspace.to_string(),
            table: table.to_string(),
            alias: alias.clone(),
            index_name,
            equality_columns,
            pre_sorted: claims_order,
            ascending,
            projection: projection.clone(),
            filter: remaining_filter,
            limit: pushed_limit,
        })
    }

    /// Collect all available index options with their selectivity scores.
    ///
    /// `workspace` is required because spatial availability is per (workspace,
    /// property): unlike every other index here, "is it usable" is a question
    /// about state on this node's disk, not about configuration.
    fn collect_index_options<'a>(
        &self,
        canonical: &'a [CanonicalPredicate],
        workspace: &str,
        branch: &str,
    ) -> Vec<(f64, &'a CanonicalPredicate)> {
        let mut index_options = Vec::new();

        for pred in canonical {
            let selectivity = self.estimate_selectivity(pred);

            let has_index = match pred {
                CanonicalPredicate::PrefixRange { .. } => self.index_catalog.has_path_index(),
                CanonicalPredicate::ChildOf { .. } => true,
                CanonicalPredicate::DescendantOf { .. } => self.index_catalog.has_path_index(),
                CanonicalPredicate::ColumnEq { column, .. } => {
                    // All pseudo-properties written by index_node_properties:
                    // __node_type, __archetype, __name, __created_by,
                    // __updated_by, __created_at, __updated_at.
                    let col_lower = column.to_lowercase();
                    matches!(
                        col_lower.as_str(),
                        "node_type"
                            | "archetype"
                            | "name"
                            | "created_by"
                            | "updated_by"
                            | "created_at"
                            | "updated_at"
                    ) && self.index_catalog.has_property_index()
                }
                CanonicalPredicate::JsonPropertyEq { .. } => {
                    self.index_catalog.has_property_index()
                }
                // IN predicates are handled by `try_expand_in_union`, not by
                // best-predicate selection. If expansion was abandoned they fall
                // through to a residual filter on a TableScan.
                CanonicalPredicate::ColumnIn { .. } => false,
                CanonicalPredicate::JsonPropertyIn { .. } => false,
                CanonicalPredicate::DepthEq { .. } => false,
                CanonicalPredicate::RangeCompare { column, .. } => {
                    let col_lower = column.to_lowercase();
                    (col_lower == "created_at" || col_lower == "updated_at")
                        && self.index_catalog.has_property_index()
                }
                CanonicalPredicate::JsonPropertyRange { .. } => {
                    self.index_catalog.has_property_index()
                }
                CanonicalPredicate::PropertyPrefixRange { .. } => {
                    self.index_catalog.has_property_index()
                }
                // The gate that closes the silent-empty trap. `has_spatial_index()`
                // used to answer a hardcoded `true` here ("the CF is always
                // present"), which is a statement about schema, not about
                // whether anything was ever indexed for this property — and
                // `build_spatial_scan` then stripped the predicate on the
                // strength of it. A scoped, fail-closed answer means an
                // unpopulated or stale index costs a full scan, not an empty
                // result set.
                CanonicalPredicate::SpatialDWithin {
                    property_name,
                    center_lon,
                    center_lat,
                    radius_meters,
                    ..
                } => {
                    let availability = self.spatial_availability(workspace, branch, property_name);
                    // Latitude matters: the same radius needs more geohash cells
                    // near the poles. This must ask the same question, with the
                    // same arguments, as `build_spatial_scan` — otherwise the
                    // "unreachable" degrade path there becomes reachable.
                    let usable = availability.is_ready()
                        && crate::physical_plan::catalog::radius_is_covered(
                            *center_lon,
                            *center_lat,
                            *radius_meters,
                            availability.precisions(),
                        );
                    if !usable {
                        // ONE warning naming workspace, property and reason. The
                        // query still returns the right rows (the predicate stays
                        // in the residual filter); this is the signpost.
                        let detail = if availability.is_ready() {
                            format!(
                                "no indexed geohash precision {:?} can cover a {}m radius \
                                 within the cell budget",
                                availability.precisions(),
                                radius_meters
                            )
                        } else {
                            crate::physical_plan::catalog::explain_reason(&availability)
                        };
                        tracing::warn!(
                            workspace = %workspace,
                            property = %property_name,
                            radius_meters = %radius_meters,
                            detail = %detail,
                            "ST_DWITHIN will NOT use the spatial index; the predicate is \
                             kept as a row-level filter (correct, but a full scan)"
                        );
                    }
                    usable
                }
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
