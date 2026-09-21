// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! DML (Data Manipulation Language) Execution.
//!
//! Executes INSERT, UPDATE, DELETE, ORDER, MOVE, COPY, TRANSLATE,
//! RELATE, and UNRELATE operations on schema tables and workspace tables.
//!
//! # Module Structure
//!
//! - `helpers` - Common utility functions (expression evaluation, type conversion)
//! - `schema_builders` - Build and apply functions for NodeType, Archetype, ElementType
//! - `schema_dml` - Schema table INSERT/UPDATE/DELETE operations
//! - `node_helpers` - Node-specific types and helpers (NodeIdentifier, FilterComplexity)
//! - `workspace_dml` - Workspace INSERT/UPDATE/DELETE with transaction management
//! - `bulk_operations` - Batched bulk UPDATE/DELETE for complex WHERE clauses
//! - `order` - ORDER/REORDER execution for sibling reordering
//! - `move_copy` - MOVE and COPY execution for tree relocation/duplication
//! - `translate` - TRANSLATE execution for locale management
//! - `relations` - RELATE and UNRELATE execution for node relationships
//! - `initial_structure` - Automatic child creation from NodeType definitions

mod bulk_delete;
mod bulk_operations;
mod helpers;
#[cfg(test)]
mod helpers_tests;
mod initial_structure;
mod move_copy;
pub(crate) mod node_helpers;
mod order;
mod relations;
mod schema_builders;
mod schema_dml;
mod translate;
mod workspace_dml;
mod workspace_schema_dml;

use crate::physical_plan::executor::{ExecutionContext, Row, RowStream};
use futures::stream;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_sql::analyzer::{catalog::SchemaTableKind, DmlTableTarget, TypedExpr};
use raisin_sql::logical_plan::ProjectionExpr;
use raisin_storage::Storage;

// Re-export public types
pub use node_helpers::{classify_filter, FilterComplexity, NodeIdentifier};

// Re-export public operation functions
pub use bulk_delete::execute_bulk_delete_workspace;
pub use bulk_operations::execute_bulk_update_workspace;
pub use move_copy::{execute_copy, execute_move};
pub use order::execute_order;
pub use relations::{execute_relate, execute_unrelate};
pub use translate::execute_translate;

use helpers::extract_name_from_filter;
use schema_dml::*;
use workspace_dml::*;
use workspace_schema_dml::*;

/// DELETE on the reserved `Workspaces` table is refused.
///
/// INSERT and UPDATE go through `WorkspaceService::put` (see
/// `workspace_schema_dml`), which also builds the nodes table and seeds the
/// initial structure. Dropping a workspace discards every node in it, which is
/// the management API's decision, not a SQL row delete.
fn workspaces_are_read_only(op: &str) -> Error {
    Error::Validation(format!(
        "{op} is not supported on the reserved `Workspaces` table. \
         Remove a workspace through the management API."
    ))
}

/// Every write to a schema table changes the repository schema.
fn require_schema_write<S: Storage>(
    target: &DmlTableTarget,
    op: &str,
    ctx: &ExecutionContext<S>,
) -> Result<(), Error> {
    if let DmlTableTarget::SchemaTable(kind) = target {
        crate::schema_auth::require_schema_operator(
            ctx.auth_context.as_ref(),
            &format!("{op} on {kind:?}"),
        )?;
    }
    Ok(())
}

/// Execute a physical INSERT operation.
///
/// Inserts new rows into a schema table or workspace table.
/// When `is_upsert` is true, uses create-or-update semantics for workspace tables.
pub async fn execute_insert<
    'a,
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    target: &'a DmlTableTarget,
    columns: &'a [String],
    values: &'a [Vec<TypedExpr>],
    is_upsert: bool,
    returning: Option<&'a [ProjectionExpr]>,
    ctx: &'a ExecutionContext<S>,
) -> Result<RowStream, Error> {
    let row_count = values.len();
    let mut written: Vec<Node> = Vec::new();
    require_schema_write(target, "INSERT", ctx)?;
    if returning.is_some() {
        reject_returning_on_schema_table(target, "INSERT")?;
    }

    match target {
        DmlTableTarget::SchemaTable(kind) => match kind {
            SchemaTableKind::NodeTypes => {
                execute_insert_nodetypes(columns, values, ctx).await?;
            }
            SchemaTableKind::Archetypes => {
                execute_insert_archetypes(columns, values, ctx).await?;
            }
            SchemaTableKind::ElementTypes => {
                execute_insert_elementtypes(columns, values, ctx).await?;
            }
            // NB: distinct from DmlTableTarget::Workspace below, which is a
            // CONTENT workspace (a nodes table). This is the reserved
            // `Workspaces` schema table listing the workspace DEFINITIONS.
            SchemaTableKind::Workspaces => {
                execute_insert_workspaces(columns, values, ctx).await?;
            }
        },
        DmlTableTarget::Workspace(workspace) => {
            execute_insert_workspace(
                workspace,
                columns,
                values,
                is_upsert,
                returning.map(|_| &mut written),
                ctx,
            )
            .await?;
            if let Some(exprs) = returning {
                return returning_rows(workspace, &written, exprs);
            }
        }
    }

    let mut result_row = Row::new();
    result_row.insert(
        "affected_rows".to_string(),
        PropertyValue::Integer(row_count as i64),
    );

    Ok(Box::pin(stream::once(async move { Ok(result_row) })))
}

/// RETURNING only makes sense for content nodes; schema tables are managed
/// through DDL and have no node row to project.
fn reject_returning_on_schema_table(target: &DmlTableTarget, op: &str) -> Result<(), Error> {
    if matches!(target, DmlTableTarget::SchemaTable(_)) {
        return Err(Error::Validation(format!(
            "{op} ... RETURNING is only supported on workspace (node) tables"
        )));
    }
    Ok(())
}

/// One output row per written node, each RETURNING expression evaluated
/// against the node the same way a SELECT projection would be.
fn returning_rows(
    workspace: &str,
    nodes: &[Node],
    exprs: &[ProjectionExpr],
) -> Result<RowStream, Error> {
    let mut rows = Vec::with_capacity(nodes.len());
    for node in nodes {
        let source = helpers::node_to_row(node, workspace);
        let mut out = Row::new();
        for proj in exprs {
            let value = helpers::eval_expr_with_row_to_property_value(&proj.expr, &source)?;
            out.insert(proj.alias.clone(), value);
        }
        rows.push(Ok(out));
    }
    Ok(Box::pin(stream::iter(rows)))
}

/// Execute a physical UPDATE operation.
///
/// Updates existing rows in a schema table or workspace table.
pub async fn execute_update<
    'a,
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    target: &'a DmlTableTarget,
    assignments: &'a [(String, TypedExpr)],
    filter: &'a Option<TypedExpr>,
    returning: Option<&'a [ProjectionExpr]>,
    ctx: &'a ExecutionContext<S>,
) -> Result<RowStream, Error> {
    require_schema_write(target, "UPDATE", ctx)?;
    if returning.is_some() {
        reject_returning_on_schema_table(target, "UPDATE")?;
    }
    let mut written: Vec<Node> = Vec::new();
    let affected = match target {
        DmlTableTarget::SchemaTable(kind) => {
            let name = extract_name_from_filter(filter)?;
            match kind {
                SchemaTableKind::NodeTypes => {
                    execute_update_nodetype(&name, assignments, ctx).await?
                }
                SchemaTableKind::Archetypes => {
                    execute_update_archetype(&name, assignments, ctx).await?
                }
                SchemaTableKind::ElementTypes => {
                    execute_update_elementtype(&name, assignments, ctx).await?
                }
                SchemaTableKind::Workspaces => {
                    execute_update_workspaces(&name, assignments, ctx).await?
                }
            }
        }
        DmlTableTarget::Workspace(workspace) => {
            let n = execute_update_workspace(
                workspace,
                assignments,
                filter,
                returning.map(|_| &mut written),
                ctx,
            )
            .await?;
            if let Some(exprs) = returning {
                return returning_rows(workspace, &written, exprs);
            }
            n
        }
    };

    let mut result_row = Row::new();
    result_row.insert(
        "affected_rows".to_string(),
        PropertyValue::Integer(affected as i64),
    );

    Ok(Box::pin(stream::once(async move { Ok(result_row) })))
}

/// Execute a physical DELETE operation.
///
/// Deletes rows from a schema table or workspace table.
pub async fn execute_delete<
    'a,
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    target: &'a DmlTableTarget,
    filter: &'a Option<TypedExpr>,
    returning: Option<&'a [ProjectionExpr]>,
    ctx: &'a ExecutionContext<S>,
) -> Result<RowStream, Error> {
    require_schema_write(target, "DELETE", ctx)?;
    if returning.is_some() {
        reject_returning_on_schema_table(target, "DELETE")?;
    }
    let mut deleted: Vec<Node> = Vec::new();
    let affected = match target {
        DmlTableTarget::SchemaTable(kind) => {
            let name = extract_name_from_filter(filter)?;
            match kind {
                SchemaTableKind::NodeTypes => execute_delete_nodetype(&name, ctx).await?,
                SchemaTableKind::Archetypes => execute_delete_archetype(&name, ctx).await?,
                SchemaTableKind::ElementTypes => execute_delete_elementtype(&name, ctx).await?,
                SchemaTableKind::Workspaces => return Err(workspaces_are_read_only("DELETE")),
            }
        }
        DmlTableTarget::Workspace(workspace) => {
            let n =
                execute_delete_workspace(workspace, filter, returning.map(|_| &mut deleted), ctx)
                    .await?;
            if let Some(exprs) = returning {
                return returning_rows(workspace, &deleted, exprs);
            }
            n
        }
    };

    let mut result_row = Row::new();
    result_row.insert(
        "affected_rows".to_string(),
        PropertyValue::Integer(affected as i64),
    );

    Ok(Box::pin(stream::once(async move { Ok(result_row) })))
}
