//! Helper functions for CSE plan rewriting

use crate::logical_plan::LogicalPlan;

/// Get the table qualifier for referencing intermediate projection columns
///
/// This determines what table name to use when creating column references
/// to the intermediate projection layer.
pub(crate) fn get_table_qualifier(plan: &LogicalPlan) -> String {
    match plan {
        LogicalPlan::Project { input, .. } => {
            // Use the input's table qualifier
            get_table_qualifier(input)
        }
        LogicalPlan::Scan { table, alias, .. } => {
            // Use alias if present, otherwise table name
            alias.clone().unwrap_or_else(|| table.clone())
        }
        LogicalPlan::TableFunction { alias, name, .. } => {
            alias.clone().unwrap_or_else(|| name.clone())
        }
        LogicalPlan::Subquery { alias, .. }
        | LogicalPlan::CTEScan {
            alias: Some(alias), ..
        } => alias.clone(),
        LogicalPlan::CTEScan { cte_name, .. } => cte_name.clone(),
        // For other node types, recursively check input
        LogicalPlan::Filter { input, .. }
        | LogicalPlan::Sort { input, .. }
        | LogicalPlan::Limit { input, .. }
        | LogicalPlan::Distinct { input, .. }
        | LogicalPlan::Aggregate { input, .. }
        | LogicalPlan::Window { input, .. }
        | LogicalPlan::LateralMap { input, .. } => get_table_qualifier(input),
        LogicalPlan::Join { left, .. } => get_table_qualifier(left),
        LogicalPlan::SemiJoin { left, .. } => get_table_qualifier(left),
        LogicalPlan::WithCTE { main_query, .. } => get_table_qualifier(main_query),
        // DML operations don't have meaningful table qualifiers (they're leaf nodes)
        LogicalPlan::Insert { target, .. }
        | LogicalPlan::Update { target, .. }
        | LogicalPlan::Delete { target, .. } => target.table_name(),
        // ORDER has no table qualifier
        LogicalPlan::Order { .. } => "order".to_string(),
        // MOVE has no table qualifier
        LogicalPlan::Move { .. } => "move".to_string(),
        // COPY has no table qualifier
        LogicalPlan::Copy { .. } => "copy".to_string(),
        // TRANSLATE has no table qualifier
        LogicalPlan::Translate { .. } => "translate".to_string(),
        // RELATE has no table qualifier
        LogicalPlan::Relate { .. } => "relate".to_string(),
        // UNRELATE has no table qualifier
        LogicalPlan::Unrelate { .. } => "unrelate".to_string(),
        // Empty plan has no table qualifier
        LogicalPlan::Empty => "empty".to_string(),
    }
}
