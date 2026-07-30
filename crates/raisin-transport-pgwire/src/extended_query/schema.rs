// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Schema inference from SQL statements.
//!
//! Uses the RaisinDB SQL analyzer to extract column names and types from
//! query projections without executing the query. This is used for
//! `DescribePortal` responses and empty result sets.

use crate::auth::ApiKeyValidator;
use pgwire::api::results::FieldInfo;
use pgwire::api::Type;
use pgwire::error::{ErrorInfo, PgWireError, PgWireResult};
use raisin_sql::analyzer::typed_expr::Expr;
use raisin_sql::analyzer::types::DataType;
use raisin_sql::{AnalyzedQuery, AnalyzedStatement, Analyzer};
use raisin_sql_execution::StaticCatalog;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use std::sync::Arc;
use tracing::{debug, warn};

use super::RaisinExtendedQueryHandler;

impl<S, V, P> RaisinExtendedQueryHandler<S, V, P>
where
    S: Storage + TransactionalStorage + 'static,
    V: ApiKeyValidator,
    P: pgwire::api::auth::ServerParameterProvider,
{
    /// Resolve a connection's workspace catalog and infer a statement's columns.
    ///
    /// Shared by `Describe(statement)` and `Describe(portal)` so a prepared
    /// statement and its portal can never disagree about the shape of the result —
    /// a disagreement there is a protocol error the client reports as
    /// "DataRow field count does not match the number of columns".
    ///
    /// Returns `None` whenever the shape cannot be established (no connection
    /// context yet, workspaces unreadable, statement not analysable). The caller
    /// then describes zero columns, which is the right answer for a statement that
    /// returns no rows and a harmless one otherwise, since the portal describe runs
    /// again with the parameters bound.
    pub(crate) async fn describe_sql_columns<C>(
        &self,
        client: &C,
        sql: &str,
    ) -> Option<Vec<FieldInfo>>
    where
        C: pgwire::api::ClientInfo,
    {
        use raisin_storage::scope::RepoScope;
        use raisin_storage::WorkspaceRepository;

        let context = self.auth_handler.get_context(client)?;
        let workspaces = self
            .storage
            .workspaces()
            .list(RepoScope::new(&context.tenant_id, &context.repository))
            .await
            .map_err(|e| warn!("Failed to fetch workspaces for describe: {e}"))
            .ok()?;

        let mut catalog = StaticCatalog::default_nodes_schema();
        for workspace in &workspaces {
            catalog.register_workspace(workspace.name.clone());
        }

        self.infer_schema_from_sql(sql, &Arc::new(catalog))
            .ok()
            .map(|fields| fields.to_vec())
    }

    /// Infer schema from SQL using the analyzer.
    ///
    /// This parses the SELECT statement and extracts column names and types
    /// from the projection, without executing the query.
    pub(crate) fn infer_schema_from_sql(
        &self,
        sql: &str,
        catalog: &Arc<StaticCatalog>,
    ) -> PgWireResult<Arc<Vec<FieldInfo>>> {
        // Create analyzer with the catalog
        let analyzer = Analyzer::with_catalog(Box::new((**catalog).clone()));

        // Analyze the SQL
        let analyzed = analyzer.analyze(sql).map_err(|e| {
            warn!("Failed to analyze SQL for schema inference: {}", e);
            PgWireError::UserError(Box::new(ErrorInfo::new(
                "ERROR".to_owned(),
                "42000".to_owned(),
                format!("Failed to analyze SQL: {}", e),
            )))
        })?;

        // Extract schema from analyzed query
        match analyzed {
            AnalyzedStatement::Query(query) => {
                let fields = query_projection_to_fields(&query);
                debug!("Inferred {} fields from SQL analysis", fields.len());
                Ok(Arc::new(fields))
            }
            AnalyzedStatement::Show(show) => {
                // SHOW statements return a single TEXT column named after the variable
                let field = FieldInfo::new(
                    show.variable.clone(),
                    None,
                    None,
                    Type::TEXT,
                    pgwire::api::results::FieldFormat::Text,
                );
                debug!("Inferred SHOW schema: column '{}'", show.variable);
                Ok(Arc::new(vec![field]))
            }
            _ => {
                // Non-query statements don't have a result schema
                Ok(Arc::new(Vec::new()))
            }
        }
    }
}

/// Convert an analyzed query projection to a vector of PostgreSQL [`FieldInfo`].
///
/// Each projection expression is mapped to a column name (from alias or
/// expression shape) and a PostgreSQL type (from the analyzer's `DataType`).
fn query_projection_to_fields(query: &AnalyzedQuery) -> Vec<FieldInfo> {
    query
        .projection
        .iter()
        .enumerate()
        .map(|(idx, (typed_expr, alias))| {
            // Determine column name: use alias if present, otherwise generate from expression
            let name = alias.clone().unwrap_or_else(|| {
                // Try to extract name from expression
                match &typed_expr.expr {
                    Expr::Column { column, .. } => column.clone(),
                    Expr::Literal(_) => format!("literal_{}", idx),
                    Expr::Function { name, .. } => name.clone(),
                    _ => format!("column_{}", idx),
                }
            });

            // Map DataType to PostgreSQL Type
            let pg_type = datatype_to_pg_type(&typed_expr.data_type);

            FieldInfo::new(
                name,
                None, // table_id
                None, // column_id
                pg_type,
                pgwire::api::results::FieldFormat::Text,
            )
        })
        .collect()
}

/// Map a RaisinDB [`DataType`] to a PostgreSQL [`Type`].
///
/// # This must agree with the value-level mapping
///
/// The simple-query path types a column from the *value* it produced, via
/// [`crate::type_mapping::property_value_to_pg_type`]; this function types it from
/// the *analyzed expression*, before any row exists. Both describe the same column,
/// so a disagreement means a prepared statement and a simple query hand a client
/// two different OIDs for one column — and the client then decodes the bytes two
/// different ways.
///
/// `DataType::Geometry` was exactly that: `TEXT` here, `JSONB` there. It is now
/// `JSONB` in both, which is also the honest description — a geometry goes on the
/// wire as GeoJSON, including its `srid` member. `type_mapping.rs`'s
/// `geometry_is_jsonb_on_both_paths` test pins the pair together.
pub(crate) fn datatype_to_pg_type(data_type: &DataType) -> Type {
    match data_type {
        DataType::Boolean => Type::BOOL,
        DataType::Int => Type::INT4,
        DataType::BigInt => Type::INT8,
        DataType::Double => Type::FLOAT8,
        DataType::Text => Type::TEXT,
        DataType::Uuid => Type::UUID,
        DataType::TimestampTz => Type::TIMESTAMPTZ,
        DataType::Interval => Type::INTERVAL,
        DataType::Path => Type::TEXT, // Paths are text in PostgreSQL
        DataType::JsonB => Type::JSONB,
        DataType::Vector(_) => Type::TEXT, // Vectors as text for now
        // GeoJSON, matching the simple-query path (type_mapping.rs). Was TEXT,
        // which contradicted it — see the doc comment above.
        DataType::Geometry => Type::JSONB,
        DataType::TSVector => Type::TEXT,
        DataType::TSQuery => Type::TEXT,
        DataType::Array(_) => Type::JSONB, // Arrays as JSONB
        DataType::Unknown => Type::TEXT,   // Default to text
        _ => Type::TEXT,
    }
}
