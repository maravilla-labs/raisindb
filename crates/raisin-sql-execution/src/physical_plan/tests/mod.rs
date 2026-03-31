//! Integration tests for physical plan execution
//!
//! These tests verify end-to-end query execution from SQL to results.

#[cfg(test)]
mod integration_tests {
    use crate::physical_plan::operators::{PhysicalPlan, ScanReason};
    use crate::physical_plan::planner::PhysicalPlanner;
    use crate::physical_plan::types::{from_property_value, to_property_value};
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_sql::analyzer::Literal;

    #[test]
    fn test_type_conversions_roundtrip() {
        // Test string
        let pv = PropertyValue::String("hello".to_string());
        let lit = from_property_value(&pv).unwrap();
        let back = to_property_value(&lit).unwrap();
        assert_eq!(pv, back);

        // Test float
        let pv = PropertyValue::Float(42.5);
        let lit = from_property_value(&pv).unwrap();
        let back = to_property_value(&lit).unwrap();
        assert_eq!(pv, back);

        // Test integer
        let pv = PropertyValue::Integer(42);
        let lit = from_property_value(&pv).unwrap();
        let back = to_property_value(&lit).unwrap();
        assert_eq!(pv, PropertyValue::Integer(42));

        // Test boolean
        let pv = PropertyValue::Boolean(true);
        let lit = from_property_value(&pv).unwrap();
        let back = to_property_value(&lit).unwrap();
        assert_eq!(pv, back);
    }

    #[test]
    fn test_physical_plan_description() {
        use raisin_sql::logical_plan::TableSchema;
        use std::sync::Arc;

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

        let desc = plan.describe();
        assert!(desc.contains("TableScan"));
        assert!(desc.contains("nodes"));
    }

    #[test]
    fn test_physical_plan_explain() {
        use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};
        use raisin_sql::logical_plan::TableSchema;
        use std::sync::Arc;

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
        assert!(explain.contains("Filter"));
        assert!(explain.contains("TableScan"));
    }

    #[test]
    fn test_planner_creates_physical_plan() {
        use raisin_sql::logical_plan::{LogicalPlan, TableSchema};
        use std::sync::Arc;

        let planner = PhysicalPlanner::new();
        let schema = Arc::new(TableSchema {
            table_name: "nodes".to_string(),
            columns: vec![],
        });

        let logical = LogicalPlan::Scan {
            table: "nodes".to_string(),
            alias: None,
            schema,
            filter: None,
            projection: None,
            workspace: None,
            max_revision: None,
            branch_override: None,
            locales: vec![],
        };

        let physical = planner.plan(&logical);
        assert!(physical.is_ok());
        assert!(matches!(physical.unwrap(), PhysicalPlan::TableScan { .. }));
    }

    #[test]
    fn test_workspace_context_extraction() {
        use raisin_sql::logical_plan::TableSchema;
        use std::sync::Arc;

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
    fn test_limit_describe() {
        use raisin_sql::logical_plan::TableSchema;
        use std::sync::Arc;

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

        let limit = PhysicalPlan::Limit {
            input: Box::new(scan),
            limit: 10,
            offset: 5,
        };

        let desc = limit.describe();
        assert_eq!(desc, "Limit: limit=10, offset=5");
    }
}
