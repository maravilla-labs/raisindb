//! DML statement plan builders.

use super::PlanBuilder;
use crate::analyzer::{
    AnalyzedDelete, AnalyzedInsert, AnalyzedRelate, AnalyzedTranslate, AnalyzedUnrelate,
    AnalyzedUpdate,
};
use crate::logical_plan::{error::Result, operators::LogicalPlan};

impl<'a> PlanBuilder<'a> {
    /// Build a logical plan for an INSERT statement
    ///
    /// Also handles UPSERT statements (when `insert.is_upsert` is true)
    pub(crate) fn build_insert(&self, insert: &AnalyzedInsert) -> Result<LogicalPlan> {
        Ok(LogicalPlan::Insert {
            target: insert.target.clone(),
            schema: insert.schema.clone(),
            columns: insert.columns.clone(),
            values: insert.values.clone(),
            is_upsert: insert.is_upsert,
        })
    }

    /// Build a logical plan for an UPDATE statement
    pub(crate) fn build_update(&self, update: &AnalyzedUpdate) -> Result<LogicalPlan> {
        Ok(LogicalPlan::Update {
            target: update.target.clone(),
            schema: update.schema.clone(),
            assignments: update.assignments.clone(),
            filter: update.filter.clone(),
            branch_override: update.branch_override.clone(),
        })
    }

    /// Build a logical plan for a DELETE statement
    pub(crate) fn build_delete(&self, delete: &AnalyzedDelete) -> Result<LogicalPlan> {
        Ok(LogicalPlan::Delete {
            target: delete.target.clone(),
            schema: delete.schema.clone(),
            filter: delete.filter.clone(),
            branch_override: delete.branch_override.clone(),
        })
    }

    /// Build a logical plan for an ORDER statement
    pub(crate) fn build_order(
        &self,
        order: &crate::analyzer::AnalyzedOrder,
    ) -> Result<LogicalPlan> {
        Ok(LogicalPlan::Order {
            source: order.source.clone(),
            target: order.target.clone(),
            position: order.position,
            workspace: Some(order.workspace.clone()),
            branch_override: order.branch_override.clone(),
        })
    }

    /// Build a logical plan for a MOVE statement
    pub(crate) fn build_move(
        &self,
        move_stmt: &crate::analyzer::AnalyzedMove,
    ) -> Result<LogicalPlan> {
        Ok(LogicalPlan::Move {
            source: move_stmt.source.clone(),
            target_parent: move_stmt.target_parent.clone(),
            workspace: Some(move_stmt.workspace.clone()),
            branch_override: move_stmt.branch_override.clone(),
        })
    }

    /// Build a logical plan for a COPY statement
    pub(crate) fn build_copy(
        &self,
        copy_stmt: &crate::analyzer::AnalyzedCopy,
    ) -> Result<LogicalPlan> {
        Ok(LogicalPlan::Copy {
            source: copy_stmt.source.clone(),
            target_parent: copy_stmt.target_parent.clone(),
            new_name: copy_stmt.new_name.clone(),
            recursive: copy_stmt.recursive,
            workspace: Some(copy_stmt.workspace.clone()),
            branch_override: copy_stmt.branch_override.clone(),
        })
    }

    /// Build a logical plan for a TRANSLATE statement
    pub(crate) fn build_translate(&self, translate: &AnalyzedTranslate) -> Result<LogicalPlan> {
        Ok(LogicalPlan::Translate {
            locale: translate.locale.clone(),
            node_translations: translate.node_translations.clone(),
            block_translations: translate.block_translations.clone(),
            filter: translate.filter.clone(),
            workspace: Some(translate.workspace.clone()),
            branch_override: translate.branch_override.clone(),
        })
    }

    /// Build a logical plan for a RELATE statement
    pub(crate) fn build_relate(&self, relate: &AnalyzedRelate) -> Result<LogicalPlan> {
        Ok(LogicalPlan::Relate {
            source: relate.source.clone(),
            target: relate.target.clone(),
            relation_type: relate.relation_type.clone(),
            weight: relate.weight,
            branch_override: relate.branch_override.clone(),
        })
    }

    /// Build a logical plan for an UNRELATE statement
    pub(crate) fn build_unrelate(&self, unrelate: &AnalyzedUnrelate) -> Result<LogicalPlan> {
        Ok(LogicalPlan::Unrelate {
            source: unrelate.source.clone(),
            target: unrelate.target.clone(),
            relation_type: unrelate.relation_type.clone(),
            branch_override: unrelate.branch_override.clone(),
        })
    }
}
