// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Schema table DML operations (INSERT, UPDATE, DELETE).
//!
//! Handles operations on NodeTypes, Archetypes, and ElementTypes tables.

use crate::physical_plan::executor::ExecutionContext;
use indexmap::IndexMap;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::TypedExpr;
use raisin_storage::{
    ArchetypeRepository, CommitMetadata, ElementTypeRepository, NodeTypeRepository, Storage,
};

use super::helpers::{eval_expr_to_property_value, extract_name_from_filter};
use super::schema_builders::{
    apply_assignment_to_archetype, apply_assignment_to_elementtype, apply_assignment_to_nodetype,
    build_archetype_from_columns, build_elementtype_from_columns, build_nodetype_from_columns,
};

// =============================================================================
// INSERT Helpers
// =============================================================================

pub(super) async fn execute_insert_nodetypes<S: Storage>(
    columns: &[String],
    values: &[Vec<TypedExpr>],
    ctx: &ExecutionContext<S>,
) -> Result<(), Error> {
    let commit = CommitMetadata::system("SQL INSERT into NodeTypes");

    for row_values in values {
        let mut col_map = IndexMap::new();
        for (col_name, value_expr) in columns.iter().zip(row_values.iter()) {
            let prop_value = eval_expr_to_property_value(value_expr)?;
            col_map.insert(col_name.clone(), prop_value);
        }

        let node_type = build_nodetype_from_columns(&col_map)?;

        ctx.storage
            .node_types()
            .create(
                raisin_storage::BranchScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch),
                node_type,
                commit.clone(),
            )
            .await?;
    }

    Ok(())
}

pub(super) async fn execute_insert_archetypes<S: Storage>(
    columns: &[String],
    values: &[Vec<TypedExpr>],
    ctx: &ExecutionContext<S>,
) -> Result<(), Error> {
    let commit = CommitMetadata::system("SQL INSERT into Archetypes");

    for row_values in values {
        let mut col_map = IndexMap::new();
        for (col_name, value_expr) in columns.iter().zip(row_values.iter()) {
            let prop_value = eval_expr_to_property_value(value_expr)?;
            col_map.insert(col_name.clone(), prop_value);
        }

        let archetype = build_archetype_from_columns(&col_map)?;

        ctx.storage
            .archetypes()
            .create(
                raisin_storage::BranchScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch),
                archetype,
                commit.clone(),
            )
            .await?;
    }

    Ok(())
}

pub(super) async fn execute_insert_elementtypes<S: Storage>(
    columns: &[String],
    values: &[Vec<TypedExpr>],
    ctx: &ExecutionContext<S>,
) -> Result<(), Error> {
    let commit = CommitMetadata::system("SQL INSERT into ElementTypes");

    for row_values in values {
        let mut col_map = IndexMap::new();
        for (col_name, value_expr) in columns.iter().zip(row_values.iter()) {
            let prop_value = eval_expr_to_property_value(value_expr)?;
            col_map.insert(col_name.clone(), prop_value);
        }

        let element_type = build_elementtype_from_columns(&col_map)?;

        ctx.storage
            .element_types()
            .create(
                raisin_storage::BranchScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch),
                element_type,
                commit.clone(),
            )
            .await?;
    }

    Ok(())
}

// =============================================================================
// UPDATE Helpers
// =============================================================================

pub(super) async fn execute_update_nodetype<S: Storage>(
    name: &str,
    assignments: &[(String, TypedExpr)],
    ctx: &ExecutionContext<S>,
) -> Result<usize, Error> {
    let mut node_type = ctx
        .storage
        .node_types()
        .get(
            raisin_storage::BranchScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch),
            name,
            None,
        )
        .await?
        .ok_or_else(|| {
            Error::Validation(format!(
                "NodeType '{}' not found for UPDATE operation",
                name
            ))
        })?;

    for (col_name, value_expr) in assignments {
        let prop_value = eval_expr_to_property_value(value_expr)?;
        apply_assignment_to_nodetype(&mut node_type, col_name, prop_value)?;
    }

    let commit = CommitMetadata::system(format!("SQL UPDATE NodeType '{}'", name));
    ctx.storage
        .node_types()
        .update(
            raisin_storage::BranchScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch),
            node_type,
            commit,
        )
        .await?;

    Ok(1)
}

pub(super) async fn execute_update_archetype<S: Storage>(
    name: &str,
    assignments: &[(String, TypedExpr)],
    ctx: &ExecutionContext<S>,
) -> Result<usize, Error> {
    let mut archetype = ctx
        .storage
        .archetypes()
        .get(
            raisin_storage::BranchScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch),
            name,
            None,
        )
        .await?
        .ok_or_else(|| {
            Error::Validation(format!(
                "Archetype '{}' not found for UPDATE operation",
                name
            ))
        })?;

    for (col_name, value_expr) in assignments {
        let prop_value = eval_expr_to_property_value(value_expr)?;
        apply_assignment_to_archetype(&mut archetype, col_name, prop_value)?;
    }

    let commit = CommitMetadata::system(format!("SQL UPDATE Archetype '{}'", name));
    ctx.storage
        .archetypes()
        .update(
            raisin_storage::BranchScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch),
            archetype,
            commit,
        )
        .await?;

    Ok(1)
}

pub(super) async fn execute_update_elementtype<S: Storage>(
    name: &str,
    assignments: &[(String, TypedExpr)],
    ctx: &ExecutionContext<S>,
) -> Result<usize, Error> {
    let mut element_type = ctx
        .storage
        .element_types()
        .get(
            raisin_storage::BranchScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch),
            name,
            None,
        )
        .await?
        .ok_or_else(|| {
            Error::Validation(format!(
                "ElementType '{}' not found for UPDATE operation",
                name
            ))
        })?;

    for (col_name, value_expr) in assignments {
        let prop_value = eval_expr_to_property_value(value_expr)?;
        apply_assignment_to_elementtype(&mut element_type, col_name, prop_value)?;
    }

    let commit = CommitMetadata::system(format!("SQL UPDATE ElementType '{}'", name));
    ctx.storage
        .element_types()
        .update(
            raisin_storage::BranchScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch),
            element_type,
            commit,
        )
        .await?;

    Ok(1)
}

// =============================================================================
// DELETE Helpers
// =============================================================================

pub(super) async fn execute_delete_nodetype<S: Storage>(
    name: &str,
    ctx: &ExecutionContext<S>,
) -> Result<usize, Error> {
    let commit = CommitMetadata::system(format!("SQL DELETE NodeType '{}'", name));
    let result = ctx
        .storage
        .node_types()
        .delete(
            raisin_storage::BranchScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch),
            name,
            commit,
        )
        .await?;
    Ok(if result.is_some() { 1 } else { 0 })
}

pub(super) async fn execute_delete_archetype<S: Storage>(
    name: &str,
    ctx: &ExecutionContext<S>,
) -> Result<usize, Error> {
    let commit = CommitMetadata::system(format!("SQL DELETE Archetype '{}'", name));
    let result = ctx
        .storage
        .archetypes()
        .delete(
            raisin_storage::BranchScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch),
            name,
            commit,
        )
        .await?;
    Ok(if result.is_some() { 1 } else { 0 })
}

pub(super) async fn execute_delete_elementtype<S: Storage>(
    name: &str,
    ctx: &ExecutionContext<S>,
) -> Result<usize, Error> {
    let commit = CommitMetadata::system(format!("SQL DELETE ElementType '{}'", name));
    let result = ctx
        .storage
        .element_types()
        .delete(
            raisin_storage::BranchScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch),
            name,
            commit,
        )
        .await?;
    Ok(if result.is_some() { 1 } else { 0 })
}
