//! Table source building for the plan builder.

use std::sync::Arc;

use super::PlanBuilder;
use crate::analyzer::{AnalyzedQuery, ColumnDef};
use crate::logical_plan::{
    error::{PlanError, Result},
    operators::{LogicalPlan, TableSchema},
};

impl<'a> PlanBuilder<'a> {
    /// Build the plan node for one FROM/JOIN relation.
    ///
    /// Returns `(plan, unconsumed_filter)`. `filter` holds the conjuncts that
    /// reference only this relation, offered here as a push-down. **Only the
    /// `Scan` arm consumes it**; every other arm hands it straight back, and
    /// `build_query` folds whatever comes back into the top-level `Filter`.
    ///
    /// That shape is deliberate. The previous signature returned only the plan,
    /// so an arm that had no use for `filter` simply dropped it on the floor and
    /// the predicate vanished from the query with no error anywhere —
    /// `SELECT * FROM HYBRID_SEARCH('x', 10, 'library') WHERE node_type = 'X'`
    /// planned to `Project { TableFunction }` with no `Filter` node at all, and
    /// the published book ships exactly that pattern. Table functions, derived
    /// tables and CTE scans all had it.
    ///
    /// The fix is not a list of exclusions in `split_predicates_by_table` — that
    /// leaves the *next* relation kind silently dropping again. Here an arm has
    /// to actively consume the filter to make it disappear, so forgetting costs
    /// a redundant filter, never a missing one.
    pub(crate) fn build_table_source(
        &self,
        table_ref: &crate::analyzer::TableRef,
        query: &AnalyzedQuery,
        filter: Option<crate::analyzer::TypedExpr>,
    ) -> Result<(LogicalPlan, Option<crate::analyzer::TypedExpr>)> {
        // Lateral functions are handled as LateralMap in query builder, not as standalone scans
        if table_ref.lateral_function.is_some() {
            return Err(PlanError::InvalidPlan(
                "Lateral function cannot be used as standalone scan".into(),
            ));
        }

        // Check if this is a subquery (derived table)
        if let Some(subquery_ref) = &table_ref.subquery {
            // Build logical plan for the subquery
            let subquery_plan = self.build_query(&subquery_ref.query)?;

            let schema = Arc::new(TableSchema {
                table_name: subquery_ref.schema.name.clone(),
                columns: subquery_ref.schema.columns.clone(),
            });

            return Ok((
                LogicalPlan::Subquery {
                    input: Box::new(subquery_plan),
                    alias: table_ref
                        .alias
                        .clone()
                        .unwrap_or_else(|| table_ref.table.clone()),
                    schema,
                },
                filter,
            ));
        }

        if let Some(function) = &table_ref.table_function {
            let schema = Arc::new(TableSchema {
                table_name: function.schema.name.clone(),
                columns: function.schema.columns.clone(),
            });

            // The filter is handed back UNCONSUMED and also copied onto the
            // node as `filter`, which is ADVISORY only: an executor may use it
            // to narrow what it asks an index for, and may equally ignore it.
            // The authoritative `Filter` that `build_query` puts above this node
            // is what makes the query correct. A push-down that under-narrows
            // costs candidates; it can never cost rows.
            return Ok((
                LogicalPlan::TableFunction {
                    name: function.name.clone(),
                    alias: table_ref.alias.clone(),
                    args: function.args.clone(),
                    schema,
                    workspace: table_ref.workspace.clone(),
                    branch_override: query.branch_override.clone(),
                    max_revision: query.max_revision,
                    locales: query.locales.clone(),
                    filter: filter.clone(),
                },
                filter,
            ));
        }

        // Check if this table is a CTE reference
        for (cte_name, cte_query) in &query.ctes {
            if cte_name == &table_ref.table {
                // Build schema from CTE projection
                let columns: Vec<ColumnDef> = cte_query
                    .projection
                    .iter()
                    .map(|(expr, alias)| {
                        let col_name = alias
                            .clone()
                            .unwrap_or_else(|| Self::derive_column_name(expr));
                        ColumnDef {
                            name: col_name,
                            data_type: expr.data_type.clone(),
                            nullable: true, // CTEs are always nullable
                            generated: None,
                        }
                    })
                    .collect();

                let schema = Arc::new(TableSchema {
                    table_name: cte_name.clone(),
                    columns,
                });

                return Ok((
                    LogicalPlan::CTEScan {
                        cte_name: cte_name.clone(),
                        schema,
                        alias: table_ref.alias.clone(),
                    },
                    filter,
                ));
            }
        }

        // Try to get table definition (works for both regular tables and workspace tables)
        let table_def = if let Some(table) = self.catalog.get_table(&table_ref.table) {
            table.clone()
        } else if let Some(workspace_table) = self.catalog.get_workspace_table(&table_ref.table) {
            workspace_table
        } else {
            return Err(PlanError::TableNotFound(table_ref.table.clone()));
        };

        let schema = Arc::new(TableSchema {
            table_name: table_ref.table.clone(),
            columns: table_def
                .columns
                .iter()
                .map(|c| ColumnDef {
                    name: c.name.clone(),
                    data_type: c.data_type.clone(),
                    nullable: c.nullable,
                    generated: c.generated.clone(),
                })
                .collect(),
        });

        // The one arm that CONSUMES the filter.
        Ok((
            LogicalPlan::Scan {
                table: table_ref.table.clone(),
                alias: table_ref.alias.clone(),
                schema,
                workspace: table_ref.workspace.clone(),
                max_revision: query.max_revision,
                branch_override: query.branch_override.clone(),
                locales: query.locales.clone(),
                filter,
                projection: None,
            },
            None,
        ))
    }
}
