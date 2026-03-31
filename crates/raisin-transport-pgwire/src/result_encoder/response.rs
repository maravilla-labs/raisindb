// SPDX-License-Identifier: BSL-1.1

//! QueryResponse building and schema inference.

use futures::stream::{self, BoxStream};
use futures::StreamExt;
use pgwire::api::results::{FieldFormat, FieldInfo, QueryResponse};
use pgwire::error::PgWireResult;
use pgwire::messages::data::DataRow;
use raisin_sql_execution::Row;
use std::sync::Arc;

use super::types::{ColumnInfo, ResultEncoder};

impl ResultEncoder {
    /// Build a complete QueryResponse from rows and schema.
    pub fn build_query_response<'a>(
        rows: Vec<Row>,
        schema: Arc<Vec<FieldInfo>>,
    ) -> QueryResponse<'a> {
        let format = schema
            .first()
            .map(|f| f.format())
            .unwrap_or(FieldFormat::Text);

        let encoder = match format {
            FieldFormat::Binary => ResultEncoder::with_binary_format(),
            FieldFormat::Text => ResultEncoder::new(),
        };

        let schema_clone = schema.clone();
        let row_stream: BoxStream<'a, PgWireResult<DataRow>> = stream::iter(rows)
            .map(move |row| {
                encoder.encode_row(&row, schema_clone.clone()).map_err(|e| {
                    pgwire::error::PgWireError::UserError(Box::new(pgwire::error::ErrorInfo::new(
                        "ERROR".to_owned(),
                        "XX000".to_owned(),
                        e.to_string(),
                    )))
                })
            })
            .boxed();

        QueryResponse::new(schema, row_stream)
    }

    /// Build a QueryResponse from an async stream of Rows.
    pub fn build_query_response_from_stream<'a>(
        row_stream: raisin_sql_execution::RowStream,
        schema: Arc<Vec<FieldInfo>>,
    ) -> QueryResponse<'a> {
        let format = schema
            .first()
            .map(|f| f.format())
            .unwrap_or(FieldFormat::Text);

        let encoder = match format {
            FieldFormat::Binary => ResultEncoder::with_binary_format(),
            FieldFormat::Text => ResultEncoder::new(),
        };
        let schema_clone = schema.clone();

        let encoded_stream = row_stream.map(move |result| {
            result
                .map_err(|e| {
                    pgwire::error::PgWireError::UserError(Box::new(pgwire::error::ErrorInfo::new(
                        "ERROR".to_owned(),
                        "XX000".to_owned(),
                        format!("Execution error: {}", e),
                    )))
                })
                .and_then(|row| {
                    encoder.encode_row(&row, schema_clone.clone()).map_err(|e| {
                        pgwire::error::PgWireError::UserError(Box::new(
                            pgwire::error::ErrorInfo::new(
                                "ERROR".to_owned(),
                                "XX000".to_owned(),
                                e.to_string(),
                            ),
                        ))
                    })
                })
        });

        QueryResponse::new(schema, encoded_stream.boxed())
    }
}

/// Helper function to infer schema from the first row.
pub fn infer_schema_from_rows(rows: &[Row]) -> Vec<ColumnInfo> {
    if rows.is_empty() {
        return Vec::new();
    }

    let first_row = &rows[0];
    first_row
        .columns
        .iter()
        .map(|(name, value)| ColumnInfo::new(name.clone(), value.clone()))
        .collect()
}
