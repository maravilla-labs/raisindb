//! Plan builder
//!
//! Converts analyzed statements into logical plans.

mod expr_helpers;
mod mutation;
mod predicate;
mod query;
mod table_source;

use crate::analyzer::{AnalyzedStatement, Catalog};

use super::{error::Result, operators::LogicalPlan};

/// Plan builder that converts analyzed statements to logical plans
pub struct PlanBuilder<'a> {
    catalog: &'a dyn Catalog,
}

impl<'a> PlanBuilder<'a> {
    /// Create a new plan builder with the given catalog
    pub fn new(catalog: &'a dyn Catalog) -> Self {
        Self { catalog }
    }

    /// Build a logical plan from an analyzed statement
    pub fn build(&self, stmt: &AnalyzedStatement) -> Result<LogicalPlan> {
        match stmt {
            AnalyzedStatement::Query(query) => self.build_query(query),
            AnalyzedStatement::Explain(explain) => {
                // For EXPLAIN, we still build the plan normally
                // The actual explain output is generated at execution time
                self.build_query(&explain.query)
            }
            AnalyzedStatement::Insert(insert) => self.build_insert(insert),
            AnalyzedStatement::Update(update) => self.build_update(update),
            AnalyzedStatement::Delete(delete) => self.build_delete(delete),
            AnalyzedStatement::Order(order) => self.build_order(order),
            AnalyzedStatement::Move(move_stmt) => self.build_move(move_stmt),
            AnalyzedStatement::Copy(copy_stmt) => self.build_copy(copy_stmt),
            AnalyzedStatement::Translate(translate) => self.build_translate(translate),
            AnalyzedStatement::Relate(relate) => self.build_relate(relate),
            AnalyzedStatement::Unrelate(unrelate) => self.build_unrelate(unrelate),
            AnalyzedStatement::Ddl(_) => {
                // DDL statements bypass the logical plan - they're executed directly
                Ok(LogicalPlan::Empty)
            }
            AnalyzedStatement::Transaction(_) => {
                // Transaction statements bypass the logical plan - they're executed directly
                Ok(LogicalPlan::Empty)
            }
            AnalyzedStatement::Show(_) => {
                // SHOW statements bypass the logical plan - they're executed directly
                Ok(LogicalPlan::Empty)
            }
            AnalyzedStatement::Branch(_) => {
                // Branch statements bypass the logical plan - they're executed directly
                Ok(LogicalPlan::Empty)
            }
            AnalyzedStatement::Restore(_) => {
                // Restore statements bypass the logical plan - they're executed directly
                Ok(LogicalPlan::Empty)
            }
            AnalyzedStatement::Acl(_) => {
                // ACL statements bypass the logical plan - they're executed directly
                Ok(LogicalPlan::Empty)
            }
        }
    }
}

#[cfg(test)]
mod tests;
