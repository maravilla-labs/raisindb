//! DML (Data Manipulation Language) dispatch
//!
//! Converts logical DML plan nodes (Insert, Update, Delete, Order, Move,
//! Copy, Translate, Relate, Unrelate) into their physical counterparts.
//! These are thin 1-to-1 mappings with no optimisation logic.

use super::super::{Error, LogicalPlan, PhysicalPlan, PhysicalPlanner};

impl PhysicalPlanner {
    /// Plan a DML `LogicalPlan` variant.
    ///
    /// Returns `Some(plan)` for DML nodes, `None` for non-DML nodes.
    pub(in crate::physical_plan::planner) fn try_plan_dml(
        &self,
        logical: &LogicalPlan,
    ) -> Option<Result<PhysicalPlan, Error>> {
        match logical {
            LogicalPlan::Insert {
                target,
                schema,
                columns,
                values,
                is_upsert,
            } => Some(Ok(PhysicalPlan::PhysicalInsert {
                target: target.clone(),
                schema: schema.clone(),
                columns: columns.clone(),
                values: values.clone(),
                is_upsert: *is_upsert,
            })),

            LogicalPlan::Update {
                target,
                schema,
                assignments,
                filter,
                branch_override,
            } => Some(Ok(PhysicalPlan::PhysicalUpdate {
                target: target.clone(),
                schema: schema.clone(),
                assignments: assignments.clone(),
                filter: filter.clone(),
                branch_override: branch_override.clone(),
            })),

            LogicalPlan::Delete {
                target,
                schema,
                filter,
                branch_override,
            } => Some(Ok(PhysicalPlan::PhysicalDelete {
                target: target.clone(),
                schema: schema.clone(),
                filter: filter.clone(),
                branch_override: branch_override.clone(),
            })),

            LogicalPlan::Order {
                source,
                target,
                position,
                workspace,
                branch_override,
            } => Some(Ok(PhysicalPlan::PhysicalOrder {
                source: source.clone(),
                target: target.clone(),
                position: *position,
                workspace: workspace.clone(),
                branch_override: branch_override.clone(),
            })),

            LogicalPlan::Move {
                source,
                target_parent,
                workspace,
                branch_override,
            } => Some(Ok(PhysicalPlan::PhysicalMove {
                source: source.clone(),
                target_parent: target_parent.clone(),
                workspace: workspace.clone(),
                branch_override: branch_override.clone(),
            })),

            LogicalPlan::Copy {
                source,
                target_parent,
                new_name,
                recursive,
                workspace,
                branch_override,
            } => Some(Ok(PhysicalPlan::PhysicalCopy {
                source: source.clone(),
                target_parent: target_parent.clone(),
                new_name: new_name.clone(),
                recursive: *recursive,
                workspace: workspace.clone(),
                branch_override: branch_override.clone(),
            })),

            LogicalPlan::Translate {
                locale,
                node_translations,
                block_translations,
                filter,
                workspace,
                branch_override,
            } => Some(Ok(PhysicalPlan::PhysicalTranslate {
                locale: locale.clone(),
                node_translations: node_translations.clone(),
                block_translations: block_translations.clone(),
                filter: filter.clone(),
                workspace: workspace.clone(),
                branch_override: branch_override.clone(),
            })),

            LogicalPlan::Relate {
                source,
                target,
                relation_type,
                weight,
                branch_override,
            } => Some(Ok(PhysicalPlan::PhysicalRelate {
                source: source.clone(),
                target: target.clone(),
                relation_type: relation_type.clone(),
                weight: *weight,
                branch_override: branch_override.clone(),
            })),

            LogicalPlan::Unrelate {
                source,
                target,
                relation_type,
                branch_override,
            } => Some(Ok(PhysicalPlan::PhysicalUnrelate {
                source: source.clone(),
                target: target.clone(),
                relation_type: relation_type.clone(),
                branch_override: branch_override.clone(),
            })),

            _ => None,
        }
    }
}
