//! Physical Plan Operators
//!
//! Defines the physical operator tree that represents concrete execution strategies.
//! Each operator knows how to produce a stream of rows from its inputs.

mod describe;
mod plan;
mod scan_types;
mod traversal;

// Re-export all public types so the module's public API is unchanged.
pub use plan::PhysicalPlan;
pub use scan_types::{IndexLookupParams, IndexLookupType, ScanReason, VectorDistanceMetric};

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};
    use raisin_sql::logical_plan::TableSchema;
    use std::sync::Arc;

    #[test]
    fn test_describe_table_scan() {
        let plan = PhysicalPlan::TableScan {
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            workspace: "w1".to_string(),
            table: "nodes".to_string(),
            alias: None,
            schema: Arc::new(TableSchema {
                table_name: "nodes".to_string(),
                columns: vec![],
            }),
            filter: None,
            projection: None,
            limit: None,
            reason: ScanReason::NoIndexAvailable,
        };

        assert!(plan.describe().contains("TableScan: nodes"));
    }

    #[test]
    fn test_describe_prefix_scan() {
        let plan = PhysicalPlan::PrefixScan {
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            workspace: "w1".to_string(),
            table: "nodes".to_string(),
            alias: None,
            path_prefix: "/content/".to_string(),
            projection: None,
            direct_children_only: false,
            limit: None,
        };

        assert_eq!(plan.describe(), "PrefixScan: prefix=/content/");
    }

    #[test]
    fn test_is_scan() {
        let table_scan = PhysicalPlan::TableScan {
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            workspace: "w1".to_string(),
            table: "nodes".to_string(),
            alias: None,
            schema: Arc::new(TableSchema {
                table_name: "nodes".to_string(),
                columns: vec![],
            }),
            filter: None,
            projection: None,
            limit: None,
            reason: ScanReason::NoIndexAvailable,
        };

        assert!(table_scan.is_scan());

        let filter = PhysicalPlan::Filter {
            input: Box::new(table_scan),
            predicates: vec![],
        };

        assert!(!filter.is_scan());
    }

    #[test]
    fn test_workspace_context() {
        let plan = PhysicalPlan::TableScan {
            tenant_id: "tenant1".to_string(),
            repo_id: "repo1".to_string(),
            branch: "main".to_string(),
            workspace: "workspace1".to_string(),
            table: "nodes".to_string(),
            alias: None,
            schema: Arc::new(TableSchema {
                table_name: "nodes".to_string(),
                columns: vec![],
            }),
            filter: None,
            projection: None,
            limit: None,
            reason: ScanReason::NoIndexAvailable,
        };

        let ctx = plan.workspace_context();
        assert_eq!(ctx, Some(("tenant1", "repo1", "main", "workspace1")));
    }

    #[test]
    fn test_inputs() {
        let scan = PhysicalPlan::TableScan {
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            workspace: "w1".to_string(),
            table: "nodes".to_string(),
            alias: None,
            schema: Arc::new(TableSchema {
                table_name: "nodes".to_string(),
                columns: vec![],
            }),
            filter: None,
            projection: None,
            limit: None,
            reason: ScanReason::NoIndexAvailable,
        };

        // Scan has no inputs
        assert_eq!(scan.inputs().len(), 0);

        let filter = PhysicalPlan::Filter {
            input: Box::new(scan),
            predicates: vec![],
        };

        // Filter has one input
        assert_eq!(filter.inputs().len(), 1);
    }

    #[test]
    fn test_explain() {
        let scan = PhysicalPlan::TableScan {
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            workspace: "w1".to_string(),
            table: "nodes".to_string(),
            alias: None,
            schema: Arc::new(TableSchema {
                table_name: "nodes".to_string(),
                columns: vec![],
            }),
            filter: None,
            projection: None,
            limit: None,
            reason: ScanReason::NoIndexAvailable,
        };

        let filter = PhysicalPlan::Filter {
            input: Box::new(scan),
            predicates: vec![TypedExpr::new(
                Expr::Literal(Literal::Boolean(true)),
                DataType::Boolean,
            )],
        };

        let explain = filter.explain();
        assert!(explain.contains("Filter: 1 predicates"));
        assert!(explain.contains("TableScan: nodes"));
    }
}
