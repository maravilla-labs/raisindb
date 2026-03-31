// SPDX-License-Identifier: BSL-1.1

//! Result encoding for PostgreSQL wire protocol.
//!
//! Converts RaisinDB query results (Row structures from raisin-sql-execution)
//! into pgwire format using the FieldInfo and DataRowEncoder types from the pgwire crate.

mod encoding;
mod response;
mod types;

// Re-export public API
pub use response::infer_schema_from_rows;
pub use types::{ColumnInfo, ResultEncoder};

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use pgwire::api::results::FieldFormat;
    use postgres_types::Type;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_sql_execution::Row;

    #[test]
    fn test_encode_schema() {
        let encoder = ResultEncoder::new();
        let columns = vec![
            ColumnInfo::new("id".to_string(), PropertyValue::Integer(1)),
            ColumnInfo::new(
                "name".to_string(),
                PropertyValue::String("test".to_string()),
            ),
            ColumnInfo::new("active".to_string(), PropertyValue::Boolean(true)),
        ];

        let schema = encoder.encode_schema(&columns);

        assert_eq!(schema.len(), 3);
        assert_eq!(schema[0].name(), "id");
        assert_eq!(schema[0].datatype(), &Type::INT8);
        assert_eq!(schema[1].name(), "name");
        assert_eq!(schema[1].datatype(), &Type::TEXT);
        assert_eq!(schema[2].name(), "active");
        assert_eq!(schema[2].datatype(), &Type::BOOL);
    }

    #[test]
    fn test_encode_row() {
        let encoder = ResultEncoder::new();

        let columns = vec![
            ColumnInfo::new("id".to_string(), PropertyValue::Integer(1)),
            ColumnInfo::new(
                "name".to_string(),
                PropertyValue::String("test".to_string()),
            ),
        ];
        let schema = encoder.encode_schema(&columns);

        let mut row_data = IndexMap::new();
        row_data.insert("id".to_string(), PropertyValue::Integer(42));
        row_data.insert(
            "name".to_string(),
            PropertyValue::String("Alice".to_string()),
        );
        let row = Row::from_map(row_data);

        let result = encoder.encode_row(&row, schema);
        assert!(result.is_ok());
    }

    #[test]
    fn test_encode_row_with_null() {
        let encoder = ResultEncoder::new();

        let columns = vec![
            ColumnInfo::new("id".to_string(), PropertyValue::Integer(1)),
            ColumnInfo::new("name".to_string(), PropertyValue::Null),
        ];
        let schema = encoder.encode_schema(&columns);

        let mut row_data = IndexMap::new();
        row_data.insert("id".to_string(), PropertyValue::Integer(42));
        row_data.insert("name".to_string(), PropertyValue::Null);
        let row = Row::from_map(row_data);

        let result = encoder.encode_row(&row, schema);
        assert!(result.is_ok());
    }

    #[test]
    fn test_encode_row_missing_column() {
        let encoder = ResultEncoder::new();

        let columns = vec![
            ColumnInfo::new("id".to_string(), PropertyValue::Integer(1)),
            ColumnInfo::new(
                "name".to_string(),
                PropertyValue::String("test".to_string()),
            ),
        ];
        let schema = encoder.encode_schema(&columns);

        let mut row_data = IndexMap::new();
        row_data.insert("id".to_string(), PropertyValue::Integer(42));
        let row = Row::from_map(row_data);

        let result = encoder.encode_row(&row, schema);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_infer_schema_from_rows() {
        let mut row1_data = IndexMap::new();
        row1_data.insert("id".to_string(), PropertyValue::Integer(1));
        row1_data.insert(
            "name".to_string(),
            PropertyValue::String("Alice".to_string()),
        );
        let row1 = Row::from_map(row1_data);

        let mut row2_data = IndexMap::new();
        row2_data.insert("id".to_string(), PropertyValue::Integer(2));
        row2_data.insert("name".to_string(), PropertyValue::String("Bob".to_string()));
        let row2 = Row::from_map(row2_data);

        let rows = vec![row1, row2];
        let columns = infer_schema_from_rows(&rows);

        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0].name, "id");
        assert_eq!(columns[1].name, "name");
    }

    #[test]
    fn test_infer_schema_from_empty_rows() {
        let rows: Vec<Row> = vec![];
        let columns = infer_schema_from_rows(&rows);
        assert_eq!(columns.len(), 0);
    }

    #[test]
    fn test_build_query_response() {
        let encoder = ResultEncoder::new();

        let columns = vec![
            ColumnInfo::new("id".to_string(), PropertyValue::Integer(1)),
            ColumnInfo::new(
                "name".to_string(),
                PropertyValue::String("test".to_string()),
            ),
        ];
        let schema = encoder.encode_schema(&columns);

        let mut row1_data = IndexMap::new();
        row1_data.insert("id".to_string(), PropertyValue::Integer(1));
        row1_data.insert(
            "name".to_string(),
            PropertyValue::String("Alice".to_string()),
        );

        let mut row2_data = IndexMap::new();
        row2_data.insert("id".to_string(), PropertyValue::Integer(2));
        row2_data.insert("name".to_string(), PropertyValue::String("Bob".to_string()));

        let rows = vec![Row::from_map(row1_data), Row::from_map(row2_data)];

        let response = ResultEncoder::build_query_response(rows, schema.clone());

        assert_eq!(response.row_schema().len(), 2);
        assert_eq!(response.command_tag(), "SELECT");
    }

    #[test]
    fn test_encoder_with_binary_format() {
        let encoder = ResultEncoder::with_binary_format();
        assert_eq!(encoder.default_format, FieldFormat::Binary);

        let columns = vec![ColumnInfo::new("id".to_string(), PropertyValue::Integer(1))];
        let schema = encoder.encode_schema(&columns);
        assert_eq!(schema[0].format(), FieldFormat::Binary);
    }
}
