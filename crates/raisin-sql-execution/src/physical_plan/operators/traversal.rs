//! Tree traversal and introspection methods for physical plans.
//!
//! Provides methods to navigate the PhysicalPlan tree,
//! query operator properties, and extract execution context.

use super::plan::PhysicalPlan;

impl PhysicalPlan {
    /// Get all input operators
    pub fn inputs(&self) -> Vec<&PhysicalPlan> {
        match self {
            PhysicalPlan::Filter { input, .. }
            | PhysicalPlan::Project { input, .. }
            | PhysicalPlan::Sort { input, .. }
            | PhysicalPlan::TopN { input, .. }
            | PhysicalPlan::Limit { input, .. }
            | PhysicalPlan::Window { input, .. }
            | PhysicalPlan::Distinct { input, .. }
            | PhysicalPlan::LateralMap { input, .. } => vec![input.as_ref()],
            PhysicalPlan::NestedLoopJoin { left, right, .. }
            | PhysicalPlan::HashJoin { left, right, .. }
            | PhysicalPlan::HashSemiJoin { left, right, .. } => {
                vec![left.as_ref(), right.as_ref()]
            }
            PhysicalPlan::IndexLookupJoin { outer, .. } => {
                // Only the outer input is a plan; inner is dynamically created per row
                vec![outer.as_ref()]
            }
            PhysicalPlan::WithCTE { ctes, main_query } => {
                // WithCTE has inputs from all CTE plans plus the main query
                let mut inputs = vec![];
                for (_, cte_plan) in ctes {
                    inputs.push(cte_plan.as_ref());
                }
                inputs.push(main_query.as_ref());
                inputs
            }
            // DML operations and leaf nodes have no inputs
            PhysicalPlan::PhysicalInsert { .. }
            | PhysicalPlan::PhysicalUpdate { .. }
            | PhysicalPlan::PhysicalDelete { .. }
            | PhysicalPlan::PhysicalOrder { .. }
            | PhysicalPlan::PhysicalMove { .. }
            | PhysicalPlan::PhysicalCopy { .. }
            | PhysicalPlan::PhysicalTranslate { .. }
            | PhysicalPlan::PhysicalRelate { .. }
            | PhysicalPlan::PhysicalUnrelate { .. }
            | PhysicalPlan::PhysicalRestore { .. } => vec![],
            _ => vec![],
        }
    }

    /// Get mutable references to all input operators
    pub fn inputs_mut(&mut self) -> Vec<&mut PhysicalPlan> {
        match self {
            PhysicalPlan::Filter { input, .. }
            | PhysicalPlan::Project { input, .. }
            | PhysicalPlan::Sort { input, .. }
            | PhysicalPlan::TopN { input, .. }
            | PhysicalPlan::Limit { input, .. }
            | PhysicalPlan::Window { input, .. }
            | PhysicalPlan::Distinct { input, .. }
            | PhysicalPlan::LateralMap { input, .. } => vec![input.as_mut()],
            PhysicalPlan::NestedLoopJoin { left, right, .. }
            | PhysicalPlan::HashJoin { left, right, .. }
            | PhysicalPlan::HashSemiJoin { left, right, .. } => {
                vec![left.as_mut(), right.as_mut()]
            }
            PhysicalPlan::IndexLookupJoin { outer, .. } => {
                vec![outer.as_mut()]
            }
            PhysicalPlan::WithCTE { ctes, main_query } => {
                // WithCTE has inputs from all CTE plans plus the main query
                let mut inputs = vec![];
                for (_, cte_plan) in ctes {
                    inputs.push(cte_plan.as_mut());
                }
                inputs.push(main_query.as_mut());
                inputs
            }
            // DML operations and leaf nodes have no inputs
            PhysicalPlan::PhysicalInsert { .. }
            | PhysicalPlan::PhysicalUpdate { .. }
            | PhysicalPlan::PhysicalDelete { .. }
            | PhysicalPlan::PhysicalCopy { .. }
            | PhysicalPlan::PhysicalOrder { .. }
            | PhysicalPlan::PhysicalMove { .. }
            | PhysicalPlan::PhysicalTranslate { .. }
            | PhysicalPlan::PhysicalRelate { .. }
            | PhysicalPlan::PhysicalUnrelate { .. }
            | PhysicalPlan::PhysicalRestore { .. } => vec![],
            _ => vec![],
        }
    }

    /// Check if this is a scan operator
    pub fn is_scan(&self) -> bool {
        matches!(
            self,
            PhysicalPlan::TableScan { .. }
                | PhysicalPlan::PrefixScan { .. }
                | PhysicalPlan::PropertyIndexScan { .. }
                | PhysicalPlan::PropertyIndexCountScan { .. }
                | PhysicalPlan::PropertyOrderScan { .. }
                | PhysicalPlan::PropertyRangeScan { .. }
                | PhysicalPlan::CompoundIndexScan { .. }
                | PhysicalPlan::PathIndexScan { .. }
                | PhysicalPlan::NodeIdScan { .. }
                | PhysicalPlan::FullTextScan { .. }
                | PhysicalPlan::CTEScan { .. }
                | PhysicalPlan::VectorScan { .. }
        )
    }

    /// Extract the workspace context from this plan
    ///
    /// Returns (tenant_id, repo_id, branch, workspace) if available.
    /// This is useful for execution context setup.
    pub fn workspace_context(&self) -> Option<(&str, &str, &str, &str)> {
        match self {
            PhysicalPlan::TableScan {
                tenant_id,
                repo_id,
                branch,
                workspace,
                ..
            }
            | PhysicalPlan::PrefixScan {
                tenant_id,
                repo_id,
                branch,
                workspace,
                ..
            }
            | PhysicalPlan::FullTextScan {
                tenant_id,
                repo_id,
                branch,
                workspace,
                ..
            }
            | PhysicalPlan::VectorScan {
                tenant_id,
                repo_id,
                branch,
                workspace,
                ..
            } => Some((tenant_id, repo_id, branch, workspace)),
            PhysicalPlan::PropertyIndexScan {
                tenant_id,
                repo_id,
                branch,
                workspace,
                ..
            } => Some((tenant_id, repo_id, branch, workspace)),
            PhysicalPlan::PropertyIndexCountScan {
                tenant_id,
                repo_id,
                branch,
                workspace,
                ..
            } => Some((tenant_id, repo_id, branch, workspace)),
            PhysicalPlan::PropertyOrderScan {
                tenant_id,
                repo_id,
                branch,
                workspace,
                ..
            } => Some((tenant_id, repo_id, branch, workspace)),
            PhysicalPlan::PropertyRangeScan {
                tenant_id,
                repo_id,
                branch,
                workspace,
                ..
            } => Some((tenant_id, repo_id, branch, workspace)),
            PhysicalPlan::CompoundIndexScan {
                tenant_id,
                repo_id,
                branch,
                workspace,
                ..
            } => Some((tenant_id, repo_id, branch, workspace)),
            PhysicalPlan::PathIndexScan {
                tenant_id,
                repo_id,
                branch,
                workspace,
                ..
            } => Some((tenant_id, repo_id, branch, workspace)),
            PhysicalPlan::NodeIdScan {
                tenant_id,
                repo_id,
                branch,
                workspace,
                ..
            } => Some((tenant_id, repo_id, branch, workspace)),
            _ => {
                // For non-scan operators, recursively check inputs
                self.inputs()
                    .first()
                    .and_then(|input| input.workspace_context())
            }
        }
    }
}
