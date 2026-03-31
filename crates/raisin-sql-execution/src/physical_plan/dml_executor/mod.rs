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
mod initial_structure;
mod move_copy;
mod node_helpers;
mod order;
mod relations;
mod schema_builders;
mod schema_dml;
mod translate;
mod workspace_dml;

use crate::physical_plan::executor::{ExecutionContext, Row, RowStream};
use futures::stream;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{catalog::SchemaTableKind, DmlTableTarget, TypedExpr};
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
    ctx: &'a ExecutionContext<S>,
) -> Result<RowStream, Error> {
    let row_count = values.len();

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
        },
        DmlTableTarget::Workspace(workspace) => {
            execute_insert_workspace(workspace, columns, values, is_upsert, ctx).await?;
        }
    }

    let mut result_row = Row::new();
    result_row.insert(
        "affected_rows".to_string(),
        PropertyValue::Integer(row_count as i64),
    );

    Ok(Box::pin(stream::once(async move { Ok(result_row) })))
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
    ctx: &'a ExecutionContext<S>,
) -> Result<RowStream, Error> {
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
            }
        }
        DmlTableTarget::Workspace(workspace) => {
            execute_update_workspace(workspace, assignments, filter, ctx).await?
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
    ctx: &'a ExecutionContext<S>,
) -> Result<RowStream, Error> {
    let affected = match target {
        DmlTableTarget::SchemaTable(kind) => {
            let name = extract_name_from_filter(filter)?;
            match kind {
                SchemaTableKind::NodeTypes => execute_delete_nodetype(&name, ctx).await?,
                SchemaTableKind::Archetypes => execute_delete_archetype(&name, ctx).await?,
                SchemaTableKind::ElementTypes => execute_delete_elementtype(&name, ctx).await?,
            }
        }
        DmlTableTarget::Workspace(workspace) => {
            execute_delete_workspace(workspace, filter, ctx).await?
        }
    };

    let mut result_row = Row::new();
    result_row.insert(
        "affected_rows".to_string(),
        PropertyValue::Integer(affected as i64),
    );

    Ok(Box::pin(stream::once(async move { Ok(result_row) })))
}
