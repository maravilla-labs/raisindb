//! Plan display and debugging
//!
//! Provides pretty-printing functionality for logical plans.

mod explain_dml;
mod explain_query;

use super::operators::LogicalPlan;
use std::fmt;

impl LogicalPlan {
    /// Format plan as tree for debugging
    pub fn explain(&self) -> String {
        self.explain_with_indent(0)
    }

    pub(crate) fn explain_with_indent(&self, indent: usize) -> String {
        let prefix = "  ".repeat(indent);
        match self {
            // Query operators
            LogicalPlan::Scan { .. }
            | LogicalPlan::TableFunction { .. }
            | LogicalPlan::Filter { .. }
            | LogicalPlan::Project { .. }
            | LogicalPlan::Sort { .. }
            | LogicalPlan::Limit { .. }
            | LogicalPlan::Distinct { .. }
            | LogicalPlan::Aggregate { .. }
            | LogicalPlan::Join { .. }
            | LogicalPlan::SemiJoin { .. }
            | LogicalPlan::WithCTE { .. }
            | LogicalPlan::CTEScan { .. }
            | LogicalPlan::Subquery { .. }
            | LogicalPlan::Window { .. }
            | LogicalPlan::LateralMap { .. } => {
                explain_query::explain_query_op(self, &prefix, indent)
            }

            // DML and structural operators
            LogicalPlan::Insert { .. }
            | LogicalPlan::Update { .. }
            | LogicalPlan::Delete { .. }
            | LogicalPlan::Order { .. }
            | LogicalPlan::Move { .. }
            | LogicalPlan::Copy { .. }
            | LogicalPlan::Translate { .. }
            | LogicalPlan::Relate { .. }
            | LogicalPlan::Unrelate { .. }
            | LogicalPlan::Empty => explain_dml::explain_dml_op(self, &prefix),
        }
    }
}

impl fmt::Display for LogicalPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.explain())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::{
        typed_expr::{BinaryOperator, Literal},
        ColumnDef, DataType, Expr, TypedExpr,
    };
    use crate::logical_plan::operators::{FilterPredicate, ProjectionExpr, SortExpr, TableSchema};
    use std::sync::Arc;

    #[test]
    fn test_scan_explain() {
        let plan = LogicalPlan::Scan {
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

        let explain = plan.explain();
        assert!(explain.contains("Scan: nodes"));
    }

    #[test]
    fn test_filter_explain() {
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

        let filter = LogicalPlan::Filter {
            input: Box::new(scan),
            predicate: FilterPredicate::from_expr(TypedExpr::new(
                Expr::Literal(Literal::Boolean(true)),
                DataType::Boolean,
            )),
        };

        let explain = filter.explain();
        assert!(explain.contains("Filter:"));
        assert!(explain.contains("Scan: nodes"));
    }

    #[test]
    fn test_project_explain() {
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
            exprs: vec![
                ProjectionExpr {
                    expr: TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text),
                    alias: "id".to_string(),
                },
                ProjectionExpr {
                    expr: TypedExpr::column(
                        "nodes".to_string(),
                        "name".to_string(),
                        DataType::Text,
                    ),
                    alias: "name".to_string(),
                },
            ],
        };

        let explain = project.explain();
        assert!(explain.contains("Project:"));
        assert!(explain.contains("AS id"));
        assert!(explain.contains("AS name"));
        assert!(explain.contains("Scan: nodes"));
    }

    #[test]
    fn test_sort_explain() {
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
            exprs: vec![],
        };

        let sort = LogicalPlan::Sort {
            input: Box::new(project),
            sort_exprs: vec![SortExpr {
                expr: TypedExpr::column(
                    "nodes".to_string(),
                    "created_at".to_string(),
                    DataType::TimestampTz,
                ),
                ascending: false,
                nulls_first: true,
            }],
        };

        let explain = sort.explain();
        assert!(explain.contains("Sort:"));
        assert!(explain.contains("DESC"));
        assert!(explain.contains("Project:"));
    }

    #[test]
    fn test_limit_explain() {
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
            exprs: vec![],
        };

        let limit = LogicalPlan::Limit {
            input: Box::new(project),
            limit: 10,
            offset: 5,
        };

        let explain = limit.explain();
        assert!(explain.contains("Limit: 10 OFFSET 5"));
        assert!(explain.contains("Project:"));
    }

    #[test]
    fn test_complex_plan_explain() {
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
                Expr::BinaryOp {
                    left: Box::new(TypedExpr::column(
                        "nodes".to_string(),
                        "workspace".to_string(),
                        DataType::Text,
                    )),
                    op: BinaryOperator::Eq,
                    right: Box::new(TypedExpr::literal(Literal::Text("default".to_string()))),
                },
                DataType::Boolean,
            )),
        };

        let project = LogicalPlan::Project {
            input: Box::new(filter),
            exprs: vec![ProjectionExpr {
                expr: TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text),
                alias: "id".to_string(),
            }],
        };

        let sort = LogicalPlan::Sort {
            input: Box::new(project),
            sort_exprs: vec![SortExpr {
                expr: TypedExpr::column("nodes".to_string(), "name".to_string(), DataType::Text),
                ascending: true,
                nulls_first: false,
            }],
        };

        let limit = LogicalPlan::Limit {
            input: Box::new(sort),
            limit: 20,
            offset: 0,
        };

        let explain = limit.explain();

        assert!(explain.contains("Limit"));
        assert!(explain.contains("Sort"));
        assert!(explain.contains("Project"));
        assert!(explain.contains("Filter"));
        assert!(explain.contains("Scan"));

        let lines: Vec<&str> = explain.lines().collect();
        assert_eq!(lines.len(), 5);
        assert!(!lines[0].starts_with(' '));
        assert!(lines[4].starts_with("        "));
    }

    #[test]
    fn test_display_trait() {
        let plan = LogicalPlan::Scan {
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

        let display = format!("{}", plan);
        assert!(display.contains("Scan: nodes"));
    }
}
