//! EXPLAIN and describe output for physical plans.
//!
//! Implements human-readable descriptions and multi-line EXPLAIN output
//! for the PhysicalPlan tree.
//!
//! NOTE: This file intentionally exceeds 300 lines because describe() is a single
//! exhaustive match over all PhysicalPlan variants. Splitting the match arms across
//! files would require public API changes or lose exhaustiveness checking.

use super::plan::PhysicalPlan;
use super::scan_types::IndexLookupType;

impl PhysicalPlan {
    /// Get a human-readable description of this physical plan
    pub fn describe(&self) -> String {
        match self {
            PhysicalPlan::TableScan {
                table,
                filter,
                projection,
                reason,
                ..
            } => {
                let mut desc = format!("TableScan: {} ({})", table, reason);
                if filter.is_some() {
                    desc.push_str(" [with filter]");
                }
                if let Some(cols) = projection {
                    desc.push_str(&format!(" [project: {}]", cols.join(", ")));
                }
                desc
            }
            PhysicalPlan::TableFunction { name, alias, .. } => {
                if let Some(alias_name) = alias {
                    format!("TableFunction: {} AS {}", name, alias_name)
                } else {
                    format!("TableFunction: {}", name)
                }
            }
            PhysicalPlan::PrefixScan { path_prefix, .. } => {
                format!("PrefixScan: prefix={}", path_prefix)
            }
            PhysicalPlan::PropertyIndexScan {
                property_name,
                property_value,
                ..
            } => {
                format!("PropertyIndexScan: {}={}", property_name, property_value)
            }
            PhysicalPlan::PropertyIndexCountScan {
                property_name,
                property_value,
                ..
            } => {
                format!(
                    "PropertyIndexCountScan: {}={}",
                    property_name, property_value
                )
            }
            PhysicalPlan::PropertyOrderScan {
                property_name,
                ascending,
                limit,
                ..
            } => {
                format!(
                    "PropertyOrderScan: {} {} limit_hint={}",
                    property_name,
                    if *ascending { "ASC" } else { "DESC" },
                    limit
                )
            }
            PhysicalPlan::PropertyRangeScan {
                property_name,
                lower_bound,
                upper_bound,
                ascending,
                ..
            } => {
                let lower_str = match lower_bound {
                    Some((val, true)) => format!(">= {}", val),
                    Some((val, false)) => format!("> {}", val),
                    None => String::new(),
                };
                let upper_str = match upper_bound {
                    Some((val, true)) => format!("<= {}", val),
                    Some((val, false)) => format!("< {}", val),
                    None => String::new(),
                };
                let range_str = match (lower_str.is_empty(), upper_str.is_empty()) {
                    (false, false) => format!("({} AND {})", lower_str, upper_str),
                    (false, true) => format!("({})", lower_str),
                    (true, false) => format!("({})", upper_str),
                    (true, true) => "(unbounded)".to_string(),
                };
                format!(
                    "PropertyRangeScan: {} {} {}",
                    property_name,
                    range_str,
                    if *ascending { "ASC" } else { "DESC" }
                )
            }
            PhysicalPlan::PathIndexScan { path, .. } => {
                format!("PathIndexScan: path={}", path)
            }
            PhysicalPlan::NodeIdScan { node_id, .. } => {
                format!("NodeIdScan: id={}", node_id)
            }
            PhysicalPlan::FullTextScan {
                language, query, ..
            } => {
                format!("FullTextScan: lang={}, query={}", language, query)
            }
            PhysicalPlan::NeighborsScan {
                source_node_id,
                direction,
                relation_type,
                ..
            } => {
                let rel_type_str = relation_type.as_deref().unwrap_or("*");
                format!(
                    "NeighborsScan: source={}, direction={}, type={}",
                    source_node_id, direction, rel_type_str
                )
            }
            PhysicalPlan::SpatialDistanceScan {
                property_name,
                center_lon,
                center_lat,
                radius_meters,
                ..
            } => {
                format!(
                    "SpatialDistanceScan: {}=ST_DWithin(POINT({:.4}, {:.4}), {:.0}m)",
                    property_name, center_lon, center_lat, radius_meters
                )
            }
            PhysicalPlan::SpatialKnnScan {
                property_name,
                center_lon,
                center_lat,
                k,
                ..
            } => {
                format!(
                    "SpatialKnnScan: {} nearest to POINT({:.4}, {:.4}), k={}",
                    property_name, center_lon, center_lat, k
                )
            }
            PhysicalPlan::ReferenceIndexScan {
                target_workspace,
                target_path,
                limit,
                ..
            } => {
                format!(
                    "ReferenceIndexScan: REFERENCES('{}:{}') limit={:?}",
                    target_workspace, target_path, limit
                )
            }
            PhysicalPlan::Filter { predicates, .. } => {
                format!("Filter: {} predicates", predicates.len())
            }
            PhysicalPlan::Project { exprs, .. } => {
                format!("Project: {} expressions", exprs.len())
            }
            PhysicalPlan::Sort { sort_exprs, .. } => {
                format!("Sort: {} expressions", sort_exprs.len())
            }
            PhysicalPlan::TopN {
                sort_exprs, limit, ..
            } => {
                format!("TopN: {} expressions, limit={}", sort_exprs.len(), limit)
            }
            PhysicalPlan::Limit { limit, offset, .. } => {
                format!("Limit: limit={}, offset={}", limit, offset)
            }
            PhysicalPlan::NestedLoopJoin {
                join_type,
                condition,
                ..
            } => {
                let cond_str = if condition.is_some() {
                    " [with condition]"
                } else {
                    ""
                };
                format!("NestedLoopJoin: {:?}{}", join_type, cond_str)
            }
            PhysicalPlan::HashJoin {
                join_type,
                left_keys,
                right_keys,
                ..
            } => {
                format!(
                    "HashJoin: {:?}, {} key(s)",
                    join_type,
                    left_keys.len().min(right_keys.len())
                )
            }
            PhysicalPlan::HashSemiJoin { anti, .. } => {
                if *anti {
                    "HashSemiJoin: anti (NOT IN)".to_string()
                } else {
                    "HashSemiJoin: semi (IN)".to_string()
                }
            }
            PhysicalPlan::IndexLookupJoin {
                join_type,
                outer_key_column,
                inner_lookup,
                ..
            } => {
                let lookup_type = match inner_lookup.lookup_type {
                    IndexLookupType::ById => "id",
                    IndexLookupType::ByPath => "path",
                };
                format!(
                    "IndexLookupJoin: {:?}, outer.{} -> {} lookup on {}",
                    join_type, outer_key_column, lookup_type, inner_lookup.table
                )
            }
            PhysicalPlan::HashAggregate {
                group_by,
                aggregates,
                ..
            } => {
                format!(
                    "HashAggregate: {} group(s), {} aggregate(s)",
                    group_by.len(),
                    aggregates.len()
                )
            }
            PhysicalPlan::WithCTE { ctes, .. } => {
                format!("WithCTE: {} CTE(s)", ctes.len())
            }
            PhysicalPlan::CTEScan { cte_name, .. } => {
                format!("CTEScan: {}", cte_name)
            }
            PhysicalPlan::VectorScan {
                table,
                vector_column,
                distance_metric,
                k,
                max_distance,
                ..
            } => {
                let mut desc = format!(
                    "VectorScan: table={}, column={}, k={}, metric={}",
                    table, vector_column, k, distance_metric
                );
                if let Some(threshold) = max_distance {
                    desc.push_str(&format!(", max_distance={:.2}", threshold));
                }
                desc
            }
            PhysicalPlan::CountScan { workspace, .. } => {
                format!("CountScan: workspace={}", workspace)
            }
            PhysicalPlan::Window { window_exprs, .. } => {
                format!("Window: {} window function(s)", window_exprs.len())
            }
            PhysicalPlan::PhysicalInsert {
                target,
                values,
                is_upsert,
                ..
            } => {
                let op = if *is_upsert { "Upsert" } else { "Insert" };
                format!(
                    "{}: {} INTO {} ({} row(s))",
                    op,
                    target.table_name(),
                    target.table_name(),
                    values.len()
                )
            }
            PhysicalPlan::PhysicalUpdate {
                target,
                assignments,
                filter,
                ..
            } => {
                let filter_str = if filter.is_some() {
                    " [with filter]"
                } else {
                    ""
                };
                format!(
                    "Update: {} SET {} column(s){}",
                    target.table_name(),
                    assignments.len(),
                    filter_str
                )
            }
            PhysicalPlan::PhysicalDelete { target, filter, .. } => {
                let filter_str = if filter.is_some() {
                    " [with filter]"
                } else {
                    ""
                };
                format!("Delete: {}{}", target.table_name(), filter_str)
            }
            PhysicalPlan::PhysicalOrder {
                source,
                target,
                position,
                ..
            } => {
                format!("Order: {} {} {}", source, position, target)
            }
            PhysicalPlan::PhysicalMove {
                source,
                target_parent,
                ..
            } => {
                format!("Move: {} TO {}", source, target_parent)
            }
            PhysicalPlan::PhysicalCopy {
                source,
                target_parent,
                new_name,
                recursive,
                ..
            } => {
                let op = if *recursive { "Copy Tree" } else { "Copy" };
                let name_str = new_name
                    .as_ref()
                    .map(|n| format!(" AS '{}'", n))
                    .unwrap_or_default();
                format!("{}: {} TO {}{}", op, source, target_parent, name_str)
            }
            PhysicalPlan::PhysicalTranslate {
                locale,
                node_translations,
                block_translations,
                filter,
                ..
            } => {
                let filter_str = if filter.is_some() {
                    " [with filter]"
                } else {
                    ""
                };
                format!(
                    "Translate: locale='{}' ({} node props, {} blocks){}",
                    locale,
                    node_translations.len(),
                    block_translations.len(),
                    filter_str
                )
            }
            PhysicalPlan::PhysicalRelate {
                source,
                target,
                relation_type,
                weight,
                ..
            } => {
                let weight_str = weight.map(|w| format!(" weight={}", w)).unwrap_or_default();
                format!(
                    "Relate: FROM {}:{} TO {}:{} TYPE '{}'{}",
                    source.workspace,
                    source.node_ref,
                    target.workspace,
                    target.node_ref,
                    relation_type,
                    weight_str
                )
            }
            PhysicalPlan::PhysicalUnrelate {
                source,
                target,
                relation_type,
                ..
            } => {
                let type_str = relation_type
                    .as_ref()
                    .map(|t| format!(" TYPE '{}'", t))
                    .unwrap_or_default();
                format!(
                    "Unrelate: FROM {}:{} TO {}:{}{}",
                    source.workspace, source.node_ref, target.workspace, target.node_ref, type_str
                )
            }
            PhysicalPlan::PhysicalRestore {
                node,
                revision,
                recursive,
                translations,
                ..
            } => {
                let mode = if *recursive { "TREE " } else { "" };
                let translations_str = translations
                    .as_ref()
                    .map(|t| format!(" TRANSLATIONS ({:?})", t))
                    .unwrap_or_default();
                format!(
                    "Restore: {}NODE {:?} TO REVISION {}{}",
                    mode, node, revision, translations_str
                )
            }
            PhysicalPlan::CompoundIndexScan {
                index_name,
                equality_columns,
                ascending,
                limit,
                ..
            } => {
                let cols_str = equality_columns
                    .iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "CompoundIndexScan: {} [{}] {} limit_hint={}",
                    index_name,
                    cols_str,
                    if *ascending { "ASC" } else { "DESC" },
                    limit
                        .map(|l| l.to_string())
                        .unwrap_or_else(|| "none".to_string())
                )
            }
            PhysicalPlan::Distinct { on_columns, .. } => {
                if on_columns.is_empty() {
                    "Distinct: all columns".to_string()
                } else {
                    format!("Distinct ON: {}", on_columns.join(", "))
                }
            }
            PhysicalPlan::LateralMap { column_name, .. } => {
                format!("LateralMap: AS {}", column_name)
            }
            PhysicalPlan::Empty => "Empty (DDL)".to_string(),
        }
    }

    /// Get a multi-line EXPLAIN output for this physical plan
    pub fn explain(&self) -> String {
        self.explain_impl(0)
    }

    fn explain_impl(&self, indent: usize) -> String {
        let prefix = "  ".repeat(indent);
        let mut output = format!("{}{}\n", prefix, self.describe());

        // Recursively explain children
        match self {
            PhysicalPlan::Filter { input, .. }
            | PhysicalPlan::Project { input, .. }
            | PhysicalPlan::Sort { input, .. }
            | PhysicalPlan::TopN { input, .. }
            | PhysicalPlan::Limit { input, .. }
            | PhysicalPlan::HashAggregate { input, .. }
            | PhysicalPlan::Window { input, .. }
            | PhysicalPlan::Distinct { input, .. }
            | PhysicalPlan::LateralMap { input, .. } => {
                output.push_str(&input.explain_impl(indent + 1));
            }
            // DML operations and empty plans have no inputs to explain
            PhysicalPlan::PhysicalInsert { .. }
            | PhysicalPlan::PhysicalUpdate { .. }
            | PhysicalPlan::PhysicalDelete { .. }
            | PhysicalPlan::PhysicalMove { .. }
            | PhysicalPlan::PhysicalCopy { .. }
            | PhysicalPlan::PhysicalTranslate { .. }
            | PhysicalPlan::PhysicalRelate { .. }
            | PhysicalPlan::PhysicalUnrelate { .. }
            | PhysicalPlan::PhysicalRestore { .. }
            | PhysicalPlan::Empty => {}

            PhysicalPlan::NestedLoopJoin { left, right, .. }
            | PhysicalPlan::HashJoin { left, right, .. }
            | PhysicalPlan::HashSemiJoin { left, right, .. } => {
                output.push_str(&left.explain_impl(indent + 1));
                output.push_str(&right.explain_impl(indent + 1));
            }
            PhysicalPlan::IndexLookupJoin {
                outer,
                inner_lookup,
                ..
            } => {
                output.push_str(&outer.explain_impl(indent + 1));
                // Show what index lookup will be performed
                let lookup_prefix = "  ".repeat(indent + 1);
                let lookup_type = match inner_lookup.lookup_type {
                    IndexLookupType::ById => "NodeIdScan",
                    IndexLookupType::ByPath => "PathIndexScan",
                };
                output.push_str(&format!(
                    "{}{}: {} (per outer row)\n",
                    lookup_prefix, lookup_type, inner_lookup.table
                ));
            }
            PhysicalPlan::WithCTE { ctes, main_query } => {
                // Explain each CTE
                for (name, cte_plan) in ctes {
                    let cte_prefix = "  ".repeat(indent + 1);
                    output.push_str(&format!("{}CTE '{}': \n", cte_prefix, name));
                    output.push_str(&cte_plan.explain_impl(indent + 2));
                }
                // Explain main query
                let main_prefix = "  ".repeat(indent + 1);
                output.push_str(&format!("{}Main Query:\n", main_prefix));
                output.push_str(&main_query.explain_impl(indent + 2));
            }
            _ => {}
        }

        output
    }
}
