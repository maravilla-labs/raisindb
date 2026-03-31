// SPDX-License-Identifier: BSL-1.1

//! Core types for result encoding.

use bytes::BytesMut;
use pgwire::api::results::{FieldFormat, FieldInfo};
use pgwire::types::ToSqlText;
use postgres_types::{to_sql_checked, IsNull, ToSql, Type};
use raisin_models::nodes::properties::PropertyValue;

/// Wrapper for pre-encoded binary data that bypasses postgres_types encoding.
/// This allows passing already-encoded bytes directly to pgwire's DataRowEncoder.
#[derive(Debug)]
pub(super) struct PreEncodedBinary(pub Vec<u8>);

impl ToSql for PreEncodedBinary {
    fn to_sql(
        &self,
        _type: &Type,
        out: &mut BytesMut,
    ) -> std::result::Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        out.extend_from_slice(&self.0);
        Ok(IsNull::No)
    }

    fn accepts(_ty: &Type) -> bool {
        true // Accept any PostgreSQL type - we've already encoded it correctly
    }

    to_sql_checked!();
}

impl ToSqlText for PreEncodedBinary {
    fn to_sql_text(
        &self,
        _ty: &Type,
        out: &mut BytesMut,
    ) -> std::result::Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        // Should never be called for binary format, but implement for safety
        out.extend_from_slice(&self.0);
        Ok(IsNull::No)
    }
}

/// Information about a column in the result set.
///
/// This is an intermediate representation that captures the column name
/// and a sample value for type inference before conversion to pgwire FieldInfo.
#[derive(Debug, Clone)]
pub struct ColumnInfo {
    /// Column name
    pub name: String,
    /// Sample value for type inference (typically from first row)
    pub sample_value: PropertyValue,
}

impl ColumnInfo {
    /// Create a new ColumnInfo
    pub fn new(name: String, sample_value: PropertyValue) -> Self {
        Self { name, sample_value }
    }
}

/// Result encoder for converting RaisinDB query results to pgwire format.
///
/// This struct provides methods to encode query results from the SQL execution
/// layer into the PostgreSQL wire protocol format.
#[derive(Debug, Clone)]
pub struct ResultEncoder {
    /// Default field format (text vs binary)
    pub default_format: FieldFormat,
}

impl ResultEncoder {
    /// Create a new ResultEncoder with text format (default for simple query protocol)
    pub fn new() -> Self {
        Self {
            default_format: FieldFormat::Text,
        }
    }

    /// Create a new ResultEncoder with binary format (for extended query protocol)
    pub fn with_binary_format() -> Self {
        Self {
            default_format: FieldFormat::Binary,
        }
    }
}

impl Default for ResultEncoder {
    fn default() -> Self {
        Self::new()
    }
}
