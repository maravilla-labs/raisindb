// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Read-side scan for the reserved schema tables (`NodeTypes`, `Archetypes`,
//! `ElementTypes`, `Workspaces`).
//!
//! These tables previously had only a *write* path (INSERT/UPDATE/DELETE/DDL);
//! a `SELECT` fell through to the generic content-node DFS in
//! [`table_scan`](super::table_scan) and returned nothing. This module makes a
//! `SELECT` read the actual type registry via
//! [`Storage::archetypes`](raisin_storage::Storage) / `node_types` /
//! `element_types`, so `SELECT name, base_node_type, fields FROM Archetypes`
//! works from both the HTTP SQL API and the function runtime `raisin.sql`.
//!
//! Each registered type is serialized (serde) to a JSON object and mapped onto
//! the schema-table columns; `fields`/`meta` come through as JSONB. Callers that
//! need `extends`-merged (resolved) fields walk the `extends` chain themselves —
//! the raw stored `fields` plus `extends` are exposed here.

use async_stream::try_stream;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::catalog::SchemaTableKind;
use raisin_storage::{
    ArchetypeRepository, BranchScope, ElementTypeRepository, NodeTypeRepository, RepoScope,
    Storage, WorkspaceRepository,
};

use crate::physical_plan::eval::eval_expr;
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, Row, RowStream};
use raisin_error::Error;

/// Execute a scan against one of the reserved schema tables.
pub(super) async fn execute_schema_table_scan<S: Storage + 'static>(
    kind: SchemaTableKind,
    ctx: &ExecutionContext<S>,
    qualifier: String,
    filter: Option<raisin_sql::analyzer::TypedExpr>,
    projection: Option<Vec<String>>,
    limit: Option<usize>,
) -> Result<RowStream, ExecutionError> {
    let storage = ctx.storage.clone();
    let tenant_id = ctx.tenant_id.clone();
    let repo_id = ctx.repo_id.clone();
    let branch = ctx.branch.clone();
    let max_revision = ctx.max_revision.clone();

    tracing::info!("   SchemaTableScan: table='{}'", kind.table_name());

    Ok(Box::pin(try_stream! {
        let scope = BranchScope::new(&tenant_id, &repo_id, &branch);
        let rev = max_revision.as_ref();

        // List the registered types, serialize each to a JSON object.
        let rows_json: Vec<serde_json::Value> = match kind {
            SchemaTableKind::NodeTypes => storage
                .node_types()
                .list(scope, rev)
                .await?
                .iter()
                .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null))
                .collect(),
            SchemaTableKind::Archetypes => storage
                .archetypes()
                .list(scope, rev)
                .await?
                .iter()
                .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null))
                .collect(),
            SchemaTableKind::ElementTypes => storage
                .element_types()
                .list(scope, rev)
                .await?
                .iter()
                .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null))
                .collect(),
            // Workspaces are REPO-scoped, not branch-scoped: they have no
            // revision history and are shared across branches, so this takes a
            // RepoScope and ignores `rev`. The `__branch` column still comes
            // through from build_row, reporting the querying branch.
            SchemaTableKind::Workspaces => storage
                .workspaces()
                .list(RepoScope::new(&tenant_id, &repo_id))
                .await?
                .iter()
                .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null))
                .collect(),
        };

        let mut emitted = 0usize;
        for json in rows_json {
            // Build the FULL row first (every serde key + __branch) so a pushed-down
            // filter can reference any column even when projection narrows the output.
            let full = build_row(&json, &qualifier, &branch, None);

            if let Some(ref filter_expr) = filter {
                match eval_expr(filter_expr, &full) {
                    Ok(raisin_sql::analyzer::Literal::Boolean(true)) => {}
                    Ok(raisin_sql::analyzer::Literal::Boolean(false))
                    | Ok(raisin_sql::analyzer::Literal::Null) => continue,
                    Ok(other) => {
                        Err(Error::Validation(format!(
                            "Filter expression must return boolean, got {:?}",
                            other
                        )))?;
                        unreachable!();
                    }
                    Err(e) => {
                        Err(e)?;
                        unreachable!();
                    }
                }
            }

            let out = if projection.is_some() {
                build_row(&json, &qualifier, &branch, projection.as_ref())
            } else {
                full
            };
            yield out;
            emitted += 1;
            if let Some(lim) = limit {
                if emitted >= lim {
                    break;
                }
            }
        }
    }))
}

/// Build a [`Row`] from a serialized type object, keying columns `qualifier.col`.
///
/// When `projection` is `Some`, only those (unqualified) columns are emitted plus
/// `__branch` when requested; when `None`, every serde key is emitted plus
/// `__branch`.
fn build_row(
    json: &serde_json::Value,
    qualifier: &str,
    branch: &str,
    projection: Option<&Vec<String>>,
) -> Row {
    let mut row = Row::new();
    let want = |c: &str| projection.is_none_or(|p| p.iter().any(|x| x == c));

    if let Some(map) = json.as_object() {
        for (k, v) in map {
            if want(k) {
                row.insert(format!("{}.{}", qualifier, k), json_to_property_value(v));
            }
        }
    }
    if want("__branch") {
        row.insert(
            format!("{}.__branch", qualifier),
            PropertyValue::String(branch.to_string()),
        );
    }
    row
}

/// Plain, structural `serde_json::Value` → [`PropertyValue`] conversion (no
/// domain-type interpretation): objects/arrays recurse, numbers split
/// integer/float, everything else maps 1:1.
fn json_to_property_value(v: &serde_json::Value) -> PropertyValue {
    match v {
        serde_json::Value::Null => PropertyValue::Null,
        serde_json::Value::Bool(b) => PropertyValue::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                PropertyValue::Integer(i)
            } else {
                PropertyValue::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => PropertyValue::String(s.clone()),
        serde_json::Value::Array(arr) => {
            PropertyValue::Array(arr.iter().map(json_to_property_value).collect())
        }
        serde_json::Value::Object(map) => PropertyValue::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), json_to_property_value(v)))
                .collect(),
        ),
    }
}
