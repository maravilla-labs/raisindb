//! Implementation methods for LogicalPlan

use super::plan_enum::LogicalPlan;
use super::supporting_types::SchemaColumn;

impl LogicalPlan {
    /// Get the output schema (columns and types) of this plan node
    pub fn schema(&self) -> Vec<SchemaColumn> {
        match self {
            LogicalPlan::Scan { schema, .. } => schema
                .columns
                .iter()
                .map(|c| SchemaColumn {
                    name: c.name.clone(),
                    data_type: c.data_type.clone(),
                })
                .collect(),
            LogicalPlan::TableFunction { schema, .. } => schema
                .columns
                .iter()
                .map(|c| SchemaColumn {
                    name: c.name.clone(),
                    data_type: c.data_type.clone(),
                })
                .collect(),
            LogicalPlan::Filter { input, .. } => input.schema(),
            LogicalPlan::Project { exprs, .. } => exprs
                .iter()
                .map(|e| SchemaColumn {
                    name: e.alias.clone(),
                    data_type: e.expr.data_type.clone(),
                })
                .collect(),
            LogicalPlan::Sort { input, .. } => input.schema(),
            LogicalPlan::Limit { input, .. } => input.schema(),
            LogicalPlan::Distinct { input, .. } => input.schema(),
            LogicalPlan::Aggregate {
                group_by,
                aggregates,
                ..
            } => {
                let mut cols = vec![];
                for (idx, expr) in group_by.iter().enumerate() {
                    cols.push(SchemaColumn {
                        name: format!("group_{}", idx),
                        data_type: expr.data_type.clone(),
                    });
                }
                for agg in aggregates {
                    cols.push(SchemaColumn {
                        name: agg.alias.clone(),
                        data_type: agg.return_type.clone(),
                    });
                }
                cols
            }
            LogicalPlan::Join { left, right, .. } => {
                let mut cols = left.schema();
                cols.extend(right.schema());
                cols
            }
            LogicalPlan::WithCTE { main_query, .. } => main_query.schema(),
            LogicalPlan::CTEScan { schema, .. } => schema
                .columns
                .iter()
                .map(|c| SchemaColumn {
                    name: c.name.clone(),
                    data_type: c.data_type.clone(),
                })
                .collect(),
            LogicalPlan::Subquery { schema, .. } => schema
                .columns
                .iter()
                .map(|c| SchemaColumn {
                    name: c.name.clone(),
                    data_type: c.data_type.clone(),
                })
                .collect(),
            LogicalPlan::Window {
                input,
                window_exprs,
            } => {
                let mut cols = input.schema();
                for window_expr in window_exprs {
                    cols.push(SchemaColumn {
                        name: window_expr.alias.clone(),
                        data_type: window_expr.return_type.clone(),
                    });
                }
                cols
            }
            LogicalPlan::LateralMap {
                input,
                column_name,
                function_expr,
            } => {
                let mut cols = input.schema();
                cols.push(SchemaColumn {
                    name: column_name.clone(),
                    data_type: function_expr.data_type.clone(),
                });
                cols
            }
            LogicalPlan::Insert { .. }
            | LogicalPlan::Update { .. }
            | LogicalPlan::Delete { .. }
            | LogicalPlan::Order { .. }
            | LogicalPlan::Move { .. }
            | LogicalPlan::Copy { .. }
            | LogicalPlan::Translate { .. }
            | LogicalPlan::Relate { .. }
            | LogicalPlan::Unrelate { .. } => vec![],
            LogicalPlan::SemiJoin { left, .. } => left.schema(),
            LogicalPlan::Empty => vec![],
        }
    }

    /// Get all inputs to this operator
    pub fn inputs(&self) -> Vec<&LogicalPlan> {
        match self {
            LogicalPlan::Scan { .. }
            | LogicalPlan::TableFunction { .. }
            | LogicalPlan::CTEScan { .. } => vec![],
            LogicalPlan::Filter { input, .. }
            | LogicalPlan::Project { input, .. }
            | LogicalPlan::Sort { input, .. }
            | LogicalPlan::Limit { input, .. }
            | LogicalPlan::Distinct { input, .. }
            | LogicalPlan::Aggregate { input, .. }
            | LogicalPlan::Subquery { input, .. }
            | LogicalPlan::Window { input, .. }
            | LogicalPlan::LateralMap { input, .. } => vec![input.as_ref()],
            LogicalPlan::Join { left, right, .. } => vec![left.as_ref(), right.as_ref()],
            LogicalPlan::SemiJoin { left, right, .. } => vec![left.as_ref(), right.as_ref()],
            LogicalPlan::WithCTE { ctes, main_query } => {
                let mut inputs = vec![];
                for (_, cte_plan) in ctes {
                    inputs.push(cte_plan.as_ref());
                }
                inputs.push(main_query.as_ref());
                inputs
            }
            LogicalPlan::Insert { .. }
            | LogicalPlan::Update { .. }
            | LogicalPlan::Delete { .. }
            | LogicalPlan::Order { .. }
            | LogicalPlan::Move { .. }
            | LogicalPlan::Copy { .. }
            | LogicalPlan::Translate { .. }
            | LogicalPlan::Relate { .. }
            | LogicalPlan::Unrelate { .. }
            | LogicalPlan::Empty => vec![],
        }
    }

    /// Visit all nodes in the plan tree (depth-first)
    pub fn accept<V: crate::logical_plan::visitor::PlanVisitor>(
        &self,
        visitor: &mut V,
    ) -> V::Result {
        visitor.visit(self)
    }
}
