//! Plan visitor pattern
//!
//! Provides visitor traits for traversing and transforming logical plans.

use super::operators::LogicalPlan;

/// Visitor pattern for traversing logical plans
pub trait PlanVisitor {
    type Result;

    fn visit(&mut self, plan: &LogicalPlan) -> Self::Result;
}

/// Mutable visitor for transforming plans
pub trait PlanRewriter {
    /// Rewrite a plan node, recursively rewriting children first
    fn rewrite(&mut self, plan: LogicalPlan) -> LogicalPlan {
        match plan {
            LogicalPlan::Scan { .. } => plan,
            LogicalPlan::TableFunction { .. } => plan,
            LogicalPlan::Filter { input, predicate } => {
                let new_input = self.rewrite(*input);
                LogicalPlan::Filter {
                    input: Box::new(new_input),
                    predicate,
                }
            }
            LogicalPlan::Project { input, exprs } => {
                let new_input = self.rewrite(*input);
                LogicalPlan::Project {
                    input: Box::new(new_input),
                    exprs,
                }
            }
            LogicalPlan::Sort { input, sort_exprs } => {
                let new_input = self.rewrite(*input);
                LogicalPlan::Sort {
                    input: Box::new(new_input),
                    sort_exprs,
                }
            }
            LogicalPlan::Limit {
                input,
                limit,
                offset,
            } => {
                let new_input = self.rewrite(*input);
                LogicalPlan::Limit {
                    input: Box::new(new_input),
                    limit,
                    offset,
                }
            }
            LogicalPlan::Distinct {
                input,
                distinct_spec,
            } => {
                let new_input = self.rewrite(*input);
                LogicalPlan::Distinct {
                    input: Box::new(new_input),
                    distinct_spec,
                }
            }
            LogicalPlan::Aggregate {
                input,
                group_by,
                aggregates,
            } => {
                let new_input = self.rewrite(*input);
                LogicalPlan::Aggregate {
                    input: Box::new(new_input),
                    group_by,
                    aggregates,
                }
            }
            LogicalPlan::Join {
                left,
                right,
                join_type,
                condition,
            } => {
                let new_left = self.rewrite(*left);
                let new_right = self.rewrite(*right);
                LogicalPlan::Join {
                    left: Box::new(new_left),
                    right: Box::new(new_right),
                    join_type,
                    condition,
                }
            }
            LogicalPlan::SemiJoin {
                left,
                right,
                left_key,
                right_key,
                anti,
            } => {
                let new_left = self.rewrite(*left);
                let new_right = self.rewrite(*right);
                LogicalPlan::SemiJoin {
                    left: Box::new(new_left),
                    right: Box::new(new_right),
                    left_key,
                    right_key,
                    anti,
                }
            }
            LogicalPlan::WithCTE { ctes, main_query } => {
                // Rewrite each CTE
                let new_ctes: Vec<(String, Box<LogicalPlan>)> = ctes
                    .into_iter()
                    .map(|(name, plan)| (name, Box::new(self.rewrite(*plan))))
                    .collect();

                // Rewrite main query
                let new_main_query = self.rewrite(*main_query);

                LogicalPlan::WithCTE {
                    ctes: new_ctes,
                    main_query: Box::new(new_main_query),
                }
            }
            LogicalPlan::CTEScan { .. } => {
                // CTE scans are leaf nodes, no rewriting needed
                plan
            }
            LogicalPlan::Subquery {
                input,
                alias,
                schema,
            } => {
                // Rewrite the subquery's input plan
                let new_input = self.rewrite(*input);
                LogicalPlan::Subquery {
                    input: Box::new(new_input),
                    alias,
                    schema,
                }
            }
            LogicalPlan::Window {
                input,
                window_exprs,
            } => {
                // Rewrite the input plan
                let new_input = self.rewrite(*input);
                LogicalPlan::Window {
                    input: Box::new(new_input),
                    window_exprs,
                }
            }
            LogicalPlan::LateralMap {
                input,
                function_expr,
                column_name,
            } => {
                let new_input = self.rewrite(*input);
                LogicalPlan::LateralMap {
                    input: Box::new(new_input),
                    function_expr,
                    column_name,
                }
            }
            LogicalPlan::Insert { .. } => {
                // INSERT nodes are leaf nodes, no rewriting needed
                plan
            }
            LogicalPlan::Update { .. } => {
                // UPDATE nodes are leaf nodes, no rewriting needed
                plan
            }
            LogicalPlan::Delete { .. } => {
                // DELETE nodes are leaf nodes, no rewriting needed
                plan
            }
            LogicalPlan::Order { .. } => {
                // ORDER nodes are leaf nodes, no rewriting needed
                plan
            }
            LogicalPlan::Move { .. } => {
                // MOVE nodes are leaf nodes, no rewriting needed
                plan
            }
            LogicalPlan::Copy { .. } => {
                // COPY nodes are leaf nodes, no rewriting needed
                plan
            }
            LogicalPlan::Translate { .. } => {
                // TRANSLATE nodes are leaf nodes, no rewriting needed
                plan
            }
            LogicalPlan::Relate { .. } => {
                // RELATE nodes are leaf nodes, no rewriting needed
                plan
            }
            LogicalPlan::Unrelate { .. } => {
                // UNRELATE nodes are leaf nodes, no rewriting needed
                plan
            }
            LogicalPlan::Empty => {
                // Empty nodes are leaf nodes, no rewriting needed
                plan
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::{typed_expr::Literal, ColumnDef, DataType, Expr, TypedExpr};
    use crate::logical_plan::operators::{FilterPredicate, ProjectionExpr, TableSchema};
    use std::sync::Arc;

    /// Example visitor that counts the number of nodes in a plan
    struct NodeCounter {
        count: usize,
    }

    impl PlanVisitor for NodeCounter {
        type Result = usize;

        fn visit(&mut self, plan: &LogicalPlan) -> Self::Result {
            self.count += 1;
            for input in plan.inputs() {
                self.visit(input);
            }
            self.count
        }
    }

    #[test]
    fn test_visitor_counts_nodes() {
        // Build a simple plan: Project(Filter(Scan))
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

        let project = LogicalPlan::Project {
            input: Box::new(filter),
            exprs: vec![],
        };

        let mut counter = NodeCounter { count: 0 };
        let count = counter.visit(&project);

        // Should visit 3 nodes: Project, Filter, Scan
        assert_eq!(count, 3);
    }

    /// Example rewriter that removes Filter nodes (for testing)
    struct FilterRemover;

    impl PlanRewriter for FilterRemover {
        fn rewrite(&mut self, plan: LogicalPlan) -> LogicalPlan {
            match plan {
                LogicalPlan::Filter { input, .. } => {
                    // Skip the filter, just return the rewritten input
                    self.rewrite(*input)
                }
                _ => {
                    // For other nodes, use default rewriting behavior
                    // We need to manually implement this since we can't call default impl
                    match plan {
                        LogicalPlan::Scan { .. } => plan,
                        LogicalPlan::TableFunction { .. } => plan,
                        LogicalPlan::Project { input, exprs } => {
                            let new_input = self.rewrite(*input);
                            LogicalPlan::Project {
                                input: Box::new(new_input),
                                exprs,
                            }
                        }
                        LogicalPlan::Sort { input, sort_exprs } => {
                            let new_input = self.rewrite(*input);
                            LogicalPlan::Sort {
                                input: Box::new(new_input),
                                sort_exprs,
                            }
                        }
                        LogicalPlan::Limit {
                            input,
                            limit,
                            offset,
                        } => {
                            let new_input = self.rewrite(*input);
                            LogicalPlan::Limit {
                                input: Box::new(new_input),
                                limit,
                                offset,
                            }
                        }
                        LogicalPlan::Distinct {
                            input,
                            distinct_spec,
                        } => {
                            let new_input = self.rewrite(*input);
                            LogicalPlan::Distinct {
                                input: Box::new(new_input),
                                distinct_spec,
                            }
                        }
                        LogicalPlan::Aggregate {
                            input,
                            group_by,
                            aggregates,
                        } => {
                            let new_input = self.rewrite(*input);
                            LogicalPlan::Aggregate {
                                input: Box::new(new_input),
                                group_by,
                                aggregates,
                            }
                        }
                        LogicalPlan::Join {
                            left,
                            right,
                            join_type,
                            condition,
                        } => {
                            let new_left = self.rewrite(*left);
                            let new_right = self.rewrite(*right);
                            LogicalPlan::Join {
                                left: Box::new(new_left),
                                right: Box::new(new_right),
                                join_type,
                                condition,
                            }
                        }
                        LogicalPlan::SemiJoin {
                            left,
                            right,
                            left_key,
                            right_key,
                            anti,
                        } => {
                            let new_left = self.rewrite(*left);
                            let new_right = self.rewrite(*right);
                            LogicalPlan::SemiJoin {
                                left: Box::new(new_left),
                                right: Box::new(new_right),
                                left_key,
                                right_key,
                                anti,
                            }
                        }
                        LogicalPlan::WithCTE { ctes, main_query } => {
                            let new_ctes: Vec<(String, Box<LogicalPlan>)> = ctes
                                .into_iter()
                                .map(|(name, plan)| (name, Box::new(self.rewrite(*plan))))
                                .collect();
                            let new_main_query = self.rewrite(*main_query);
                            LogicalPlan::WithCTE {
                                ctes: new_ctes,
                                main_query: Box::new(new_main_query),
                            }
                        }
                        LogicalPlan::CTEScan { .. } => plan,
                        LogicalPlan::Subquery {
                            input,
                            alias,
                            schema,
                        } => {
                            let new_input = self.rewrite(*input);
                            LogicalPlan::Subquery {
                                input: Box::new(new_input),
                                alias,
                                schema,
                            }
                        }
                        LogicalPlan::Window {
                            input,
                            window_exprs,
                        } => {
                            let new_input = self.rewrite(*input);
                            LogicalPlan::Window {
                                input: Box::new(new_input),
                                window_exprs,
                            }
                        }
                        LogicalPlan::LateralMap {
                            input,
                            function_expr,
                            column_name,
                        } => {
                            let new_input = self.rewrite(*input);
                            LogicalPlan::LateralMap {
                                input: Box::new(new_input),
                                function_expr,
                                column_name,
                            }
                        }
                        LogicalPlan::Insert { .. } => plan,
                        LogicalPlan::Update { .. } => plan,
                        LogicalPlan::Delete { .. } => plan,
                        LogicalPlan::Order { .. } => plan,
                        LogicalPlan::Move { .. } => plan,
                        LogicalPlan::Copy { .. } => plan,
                        LogicalPlan::Translate { .. } => plan,
                        LogicalPlan::Relate { .. } => plan,
                        LogicalPlan::Unrelate { .. } => plan,
                        LogicalPlan::Empty => plan,
                        LogicalPlan::Filter { .. } => unreachable!(),
                    }
                }
            }
        }
    }

    #[test]
    fn test_rewriter_removes_filter() {
        // Build a simple plan: Project(Filter(Scan))
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

        let project = LogicalPlan::Project {
            input: Box::new(filter),
            exprs: vec![ProjectionExpr {
                expr: TypedExpr::new(Expr::Literal(Literal::Int(1)), DataType::Int),
                alias: "col".to_string(),
            }],
        };

        let mut remover = FilterRemover;
        let rewritten = remover.rewrite(project);

        // The result should be Project(Scan) - filter removed
        match rewritten {
            LogicalPlan::Project { input, .. } => match *input {
                LogicalPlan::Scan { .. } => {} // Success!
                _ => panic!("Expected Scan, got: {:?}", input),
            },
            _ => panic!("Expected Project, got: {:?}", rewritten),
        }
    }
}
