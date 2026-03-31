//! Logical plan operators
//!
//! Defines the logical operator tree representing relational algebra operations.

mod plan_enum;
mod plan_impl;
mod supporting_types;

// Re-export all public types to preserve the original module interface
pub use plan_enum::LogicalPlan;
pub use supporting_types::{
    AggregateExpr, AggregateFunction, DistinctSpec, FilterPredicate, ProjectionExpr, SchemaColumn,
    SortExpr, TableSchema, WindowExpr,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::{typed_expr::Literal, ColumnDef, DataType, Expr, TypedExpr};
    use std::sync::Arc;

    #[test]
    fn test_scan_schema() {
        let schema = Arc::new(TableSchema {
            table_name: "nodes".to_string(),
            columns: vec![
                ColumnDef {
                    name: "id".to_string(),
                    data_type: DataType::Text,
                    nullable: false,
                    generated: None,
                },
                ColumnDef {
                    name: "name".to_string(),
                    data_type: DataType::Text,
                    nullable: false,
                    generated: None,
                },
            ],
        });

        let plan = LogicalPlan::Scan {
            table: "nodes".to_string(),
            alias: None,
            schema: schema.clone(),
            filter: None,
            projection: None,
            workspace: None,
            max_revision: None,
            branch_override: None,
            locales: vec![],
        };

        let output = plan.schema();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].name, "id");
        assert_eq!(output[1].name, "name");
    }

    #[test]
    fn test_project_schema() {
        let scan = LogicalPlan::Scan {
            table: "nodes".to_string(),
            alias: None,
            schema: Arc::new(TableSchema {
                table_name: "nodes".to_string(),
                columns: vec![],
            }),
            filter: None,
            projection: None,
            workspace: None,
            max_revision: None,
            branch_override: None,
            locales: vec![],
        };

        let project = LogicalPlan::Project {
            input: Box::new(scan),
            exprs: vec![ProjectionExpr {
                expr: TypedExpr::new(Expr::Literal(Literal::Int(1)), DataType::Int),
                alias: "count".to_string(),
            }],
        };

        let output = project.schema();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].name, "count");
        assert_eq!(output[0].data_type, DataType::Int);
    }

    #[test]
    fn test_filter_preserves_schema() {
        let scan = LogicalPlan::Scan {
            table: "nodes".to_string(),
            alias: None,
            schema: Arc::new(TableSchema {
                table_name: "nodes".to_string(),
                columns: vec![ColumnDef {
                    name: "id".to_string(),
                    data_type: DataType::Text,
                    nullable: false,
                    generated: None,
                }],
            }),
            filter: None,
            projection: None,
            workspace: None,
            max_revision: None,
            branch_override: None,
            locales: vec![],
        };

        let filter = LogicalPlan::Filter {
            input: Box::new(scan),
            predicate: FilterPredicate::from_expr(TypedExpr::new(
                Expr::Literal(Literal::Boolean(true)),
                DataType::Boolean,
            )),
        };

        let output = filter.schema();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].name, "id");
    }

    #[test]
    fn test_inputs() {
        let scan = LogicalPlan::Scan {
            table: "nodes".to_string(),
            alias: None,
            schema: Arc::new(TableSchema {
                table_name: "nodes".to_string(),
                columns: vec![],
            }),
            filter: None,
            projection: None,
            workspace: None,
            max_revision: None,
            branch_override: None,
            locales: vec![],
        };

        assert_eq!(scan.inputs().len(), 0);

        let filter = LogicalPlan::Filter {
            input: Box::new(scan.clone()),
            predicate: FilterPredicate::from_expr(TypedExpr::new(
                Expr::Literal(Literal::Boolean(true)),
                DataType::Boolean,
            )),
        };

        assert_eq!(filter.inputs().len(), 1);
    }
}
