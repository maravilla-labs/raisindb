// SPDX-License-Identifier: BSL-1.1

//! Schema and row encoding for pgwire format.

use crate::error::PgWireTransportError;
use crate::type_mapping::{encode_value_text, is_null, to_pg_type};
use crate::type_mapping_binary::encode_value_binary;
use crate::Result;
use pgwire::api::results::{DataRowEncoder, FieldFormat, FieldInfo};
use pgwire::messages::data::DataRow;
use raisin_sql_execution::Row;
use std::sync::Arc;

use super::types::{PreEncodedBinary, ResultEncoder};

impl ResultEncoder {
    /// Encode column schema to pgwire FieldInfo format.
    pub fn encode_schema(&self, columns: &[super::types::ColumnInfo]) -> Arc<Vec<FieldInfo>> {
        self.encode_schema_with_formats(columns, |_| self.default_format)
    }

    /// Encode column schema with per-column formats.
    ///
    /// This is used by the extended query protocol to respect client-requested
    /// result formats (text/binary) on a per-column basis.
    pub fn encode_schema_with_formats<F>(
        &self,
        columns: &[super::types::ColumnInfo],
        format_for: F,
    ) -> Arc<Vec<FieldInfo>>
    where
        F: Fn(usize) -> FieldFormat,
    {
        let fields: Vec<FieldInfo> = columns
            .iter()
            .enumerate()
            .map(|(idx, col)| {
                let pg_type = to_pg_type(&col.sample_value);
                FieldInfo::new(col.name.clone(), None, None, pg_type, format_for(idx))
            })
            .collect();

        Arc::new(fields)
    }

    /// Encode a single Row to pgwire DataRow format.
    pub fn encode_row(&self, row: &Row, schema: Arc<Vec<FieldInfo>>) -> Result<DataRow> {
        let mut encoder = DataRowEncoder::new(schema.clone());

        for field in schema.iter() {
            let column_name = field.name();
            let field_format = field.format();
            let pg_type = field.datatype();

            let value = row.get(column_name).ok_or_else(|| {
                PgWireTransportError::internal(format!("Column '{}' not found in row", column_name))
            })?;

            if is_null(value) {
                match field_format {
                    FieldFormat::Binary => {
                        encoder.encode_field(&None::<Vec<u8>>).map_err(|e| {
                            PgWireTransportError::internal(format!(
                                "Failed to encode NULL (binary) for column '{}': {}",
                                column_name, e
                            ))
                        })?;
                    }
                    FieldFormat::Text => {
                        encoder.encode_field(&None::<String>).map_err(|e| {
                            PgWireTransportError::internal(format!(
                                "Failed to encode NULL (text) for column '{}': {}",
                                column_name, e
                            ))
                        })?;
                    }
                }
            } else {
                match field_format {
                    FieldFormat::Binary => {
                        let binary_value = encode_value_binary(value, pg_type).map_err(|e| {
                            PgWireTransportError::internal(format!(
                                "Failed to binary-encode value for column '{}': {}",
                                column_name, e
                            ))
                        })?;
                        encoder
                            .encode_field(&Some(PreEncodedBinary(binary_value)))
                            .map_err(|e| {
                                PgWireTransportError::internal(format!(
                                    "Failed to encode binary value for column '{}': {}",
                                    column_name, e
                                ))
                            })?;
                    }
                    FieldFormat::Text => {
                        let text_value = encode_value_text(value)?;
                        encoder.encode_field(&Some(text_value)).map_err(|e| {
                            PgWireTransportError::internal(format!(
                                "Failed to encode text value for column '{}': {}",
                                column_name, e
                            ))
                        })?;
                    }
                }
            }
        }

        encoder.finish().map_err(|e| {
            PgWireTransportError::internal(format!("Failed to finish encoding row: {}", e))
        })
    }
}
