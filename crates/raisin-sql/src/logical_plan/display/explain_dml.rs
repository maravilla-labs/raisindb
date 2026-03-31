//! Explain formatting for DML and structural operators
//! (Insert, Update, Delete, Order, Move, Copy, Translate, Relate, Unrelate)

use super::super::operators::LogicalPlan;

/// Format DML/structural plan nodes as tree strings
pub(super) fn explain_dml_op(plan: &LogicalPlan, prefix: &str) -> String {
    match plan {
        LogicalPlan::Insert {
            target,
            columns,
            values,
            ..
        } => {
            let cols_str = if columns.is_empty() {
                "ALL COLUMNS".to_string()
            } else {
                columns.join(", ")
            };
            format!(
                "{}Insert: {} INTO {} ({} row{})",
                prefix,
                cols_str,
                target.table_name(),
                values.len(),
                if values.len() == 1 { "" } else { "s" }
            )
        }
        LogicalPlan::Update {
            target,
            assignments,
            filter,
            ..
        } => {
            let filter_str = if let Some(f) = filter {
                format!(" WHERE {:?}", f.expr)
            } else {
                " (ALL ROWS)".to_string()
            };
            format!(
                "{}Update: {} SET {} columns{}",
                prefix,
                target.table_name(),
                assignments.len(),
                filter_str
            )
        }
        LogicalPlan::Delete { target, filter, .. } => {
            let filter_str = if let Some(f) = filter {
                format!(" WHERE {:?}", f.expr)
            } else {
                " (ALL ROWS)".to_string()
            };
            format!("{}Delete: {}{}", prefix, target.table_name(), filter_str)
        }
        LogicalPlan::Order {
            source,
            target,
            position,
            workspace,
            branch_override,
        } => {
            let ws_str = workspace
                .as_ref()
                .map(|w| format!(" IN {}", w))
                .unwrap_or_default();
            let branch_str = branch_override
                .as_ref()
                .map(|b| format!(" ON BRANCH '{}'", b))
                .unwrap_or_default();
            format!(
                "{}Order: {:?} {:?} {:?}{}{}",
                prefix, source, position, target, ws_str, branch_str
            )
        }
        LogicalPlan::Move {
            source,
            target_parent,
            workspace,
            branch_override,
        } => {
            let ws_str = workspace
                .as_ref()
                .map(|w| format!(" IN {}", w))
                .unwrap_or_default();
            let branch_str = branch_override
                .as_ref()
                .map(|b| format!(" ON BRANCH '{}'", b))
                .unwrap_or_default();
            format!(
                "{}Move: {:?} TO {:?}{}{}",
                prefix, source, target_parent, ws_str, branch_str
            )
        }
        LogicalPlan::Copy {
            source,
            target_parent,
            new_name,
            recursive,
            workspace,
            branch_override,
        } => {
            let op = if *recursive { "Copy Tree" } else { "Copy" };
            let ws_str = workspace
                .as_ref()
                .map(|w| format!(" IN {}", w))
                .unwrap_or_default();
            let branch_str = branch_override
                .as_ref()
                .map(|b| format!(" ON BRANCH '{}'", b))
                .unwrap_or_default();
            let name_str = new_name
                .as_ref()
                .map(|n| format!(" AS '{}'", n))
                .unwrap_or_default();
            format!(
                "{}{}: {:?} TO {:?}{}{}{}",
                prefix, op, source, target_parent, name_str, ws_str, branch_str
            )
        }
        LogicalPlan::Translate {
            locale,
            node_translations,
            block_translations,
            filter,
            workspace,
            branch_override,
        } => {
            let ws_str = workspace
                .as_ref()
                .map(|w| format!(" IN {}", w))
                .unwrap_or_default();
            let branch_str = branch_override
                .as_ref()
                .map(|b| format!(" ON BRANCH '{}'", b))
                .unwrap_or_default();
            let filter_str = if filter.is_some() { " (filtered)" } else { "" };
            format!(
                "{}Translate: locale='{}' ({} node props, {} blocks){}{}{}",
                prefix,
                locale,
                node_translations.len(),
                block_translations.len(),
                filter_str,
                ws_str,
                branch_str
            )
        }
        LogicalPlan::Relate {
            source,
            target,
            relation_type,
            weight,
            branch_override,
        } => {
            let branch_str = branch_override
                .as_ref()
                .map(|b| format!(" ON BRANCH '{}'", b))
                .unwrap_or_default();
            let weight_str = weight.map(|w| format!(" WEIGHT {}", w)).unwrap_or_default();
            format!(
                "{}Relate: FROM {}:{} TO {}:{} TYPE '{}'{}{}",
                prefix,
                source.workspace,
                source.node_ref,
                target.workspace,
                target.node_ref,
                relation_type,
                weight_str,
                branch_str
            )
        }
        LogicalPlan::Unrelate {
            source,
            target,
            relation_type,
            branch_override,
        } => {
            let branch_str = branch_override
                .as_ref()
                .map(|b| format!(" ON BRANCH '{}'", b))
                .unwrap_or_default();
            let type_str = relation_type
                .as_ref()
                .map(|t| format!(" TYPE '{}'", t))
                .unwrap_or_default();
            format!(
                "{}Unrelate: FROM {}:{} TO {}:{}{}{}",
                prefix,
                source.workspace,
                source.node_ref,
                target.workspace,
                target.node_ref,
                type_str,
                branch_str
            )
        }
        LogicalPlan::Empty => format!("{}Empty (DDL)", prefix),
        // Query operators are handled elsewhere
        _ => unreachable!("explain_dml_op called for non-DML operator"),
    }
}
