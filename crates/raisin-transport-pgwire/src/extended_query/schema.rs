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
fn datatype_to_pg_type(data_type: &DataType) -> Type {
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
        DataType::Geometry => Type::TEXT,  // GeoJSON as text
        DataType::TSVector => Type::TEXT,
        DataType::TSQuery => Type::TEXT,
        DataType::Array(_) => Type::JSONB, // Arrays as JSONB
        DataType::Unknown => Type::TEXT,   // Default to text
        _ => Type::TEXT,
    }
}
