//! PostgreSQL System Catalog Virtual Tables
//!
//! This module provides virtual table definitions for PostgreSQL system catalog tables
//! (pg_type, pg_namespace, pg_class, pg_attribute) to support GUI clients like
//! Beekeeper Studio, DBeaver, and pgAdmin.
//!
//! These are read-only virtual tables that return metadata about RaisinDB's schema.

use super::catalog::{ColumnDef, TableDef};
use super::types::DataType;

/// PostgreSQL schema OIDs
pub mod oids {
    /// pg_catalog schema OID
    pub const PG_CATALOG_NAMESPACE: i64 = 11;
    /// public schema OID
    pub const PUBLIC_NAMESPACE: i64 = 2200;
    /// information_schema OID
    pub const INFORMATION_SCHEMA_NAMESPACE: i64 = 12350;

    // PostgreSQL type OIDs (standard values)
    pub const BOOL: i64 = 16;
    pub const INT2: i64 = 21;
    pub const INT4: i64 = 23;
    pub const INT8: i64 = 20;
    pub const FLOAT4: i64 = 700;
    pub const FLOAT8: i64 = 701;
    pub const TEXT: i64 = 25;
    pub const VARCHAR: i64 = 1043;
    pub const TIMESTAMP: i64 = 1114;
    pub const TIMESTAMPTZ: i64 = 1184;
    pub const JSONB: i64 = 3802;
    pub const UUID: i64 = 2950;
    pub const OID: i64 = 26;
    pub const NAME: i64 = 19;
    pub const CHAR: i64 = 18;
    pub const REGPROC: i64 = 24;
    pub const ACLITEM_ARRAY: i64 = 1034;
}

/// Check if a table name is a pg_catalog system table
pub fn is_pg_catalog_table(schema: Option<&str>, table: &str) -> bool {
    let schema = schema.unwrap_or("");
    let is_pg_catalog = schema.eq_ignore_ascii_case("pg_catalog") || schema.is_empty();

    if !is_pg_catalog {
        return false;
    }

    matches!(
        table.to_lowercase().as_str(),
        "pg_type" | "pg_namespace" | "pg_class" | "pg_attribute" | "pg_tables" | "pg_indexes"
    )
}

/// Get the TableDef for a pg_catalog table
pub fn get_pg_catalog_table(table: &str) -> Option<TableDef> {
    match table.to_lowercase().as_str() {
        "pg_type" => Some(pg_type_table_def()),
        "pg_namespace" => Some(pg_namespace_table_def()),
        "pg_class" => Some(pg_class_table_def()),
        "pg_attribute" => Some(pg_attribute_table_def()),
        _ => None,
    }
}

/// pg_type table definition
/// Stores information about data types
fn pg_type_table_def() -> TableDef {
    TableDef {
        name: "pg_type".to_string(),
        columns: vec![
            ColumnDef {
                name: "oid".to_string(),
                data_type: DataType::BigInt,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "typname".to_string(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "typnamespace".to_string(),
                data_type: DataType::BigInt,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "typowner".to_string(),
                data_type: DataType::BigInt,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "typlen".to_string(),
                data_type: DataType::Int,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "typbyval".to_string(),
                data_type: DataType::Boolean,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "typtype".to_string(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "typcategory".to_string(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "typispreferred".to_string(),
                data_type: DataType::Boolean,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "typisdefined".to_string(),
                data_type: DataType::Boolean,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "typdelim".to_string(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "typrelid".to_string(),
                data_type: DataType::BigInt,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "typelem".to_string(),
                data_type: DataType::BigInt,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "typarray".to_string(),
                data_type: DataType::BigInt,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "typinput".to_string(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "typoutput".to_string(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
        ],
        primary_key: vec!["oid".to_string()],
        indexes: vec![],
    }
}

/// pg_namespace table definition
/// Stores information about schemas (namespaces)
fn pg_namespace_table_def() -> TableDef {
    TableDef {
        name: "pg_namespace".to_string(),
        columns: vec![
            ColumnDef {
                name: "oid".to_string(),
                data_type: DataType::BigInt,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "nspname".to_string(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "nspowner".to_string(),
                data_type: DataType::BigInt,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "nspacl".to_string(),
                data_type: DataType::Nullable(Box::new(DataType::Text)),
                nullable: true,
                generated: None,
            },
        ],
        primary_key: vec!["oid".to_string()],
        indexes: vec![],
    }
}

/// pg_class table definition
/// Stores information about tables, indexes, sequences, views
fn pg_class_table_def() -> TableDef {
    TableDef {
        name: "pg_class".to_string(),
        columns: vec![
            ColumnDef {
                name: "oid".to_string(),
                data_type: DataType::BigInt,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "relname".to_string(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "relnamespace".to_string(),
                data_type: DataType::BigInt,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "reltype".to_string(),
                data_type: DataType::BigInt,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "reloftype".to_string(),
                data_type: DataType::BigInt,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "relowner".to_string(),
                data_type: DataType::BigInt,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "relkind".to_string(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "reltuples".to_string(),
                data_type: DataType::Double,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "relhasindex".to_string(),
                data_type: DataType::Boolean,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "relisshared".to_string(),
                data_type: DataType::Boolean,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "relpersistence".to_string(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "relnatts".to_string(),
                data_type: DataType::Int,
                nullable: false,
                generated: None,
            },
        ],
        primary_key: vec!["oid".to_string()],
        indexes: vec![],
    }
}

/// pg_attribute table definition
/// Stores information about table columns
fn pg_attribute_table_def() -> TableDef {
    TableDef {
        name: "pg_attribute".to_string(),
        columns: vec![
            ColumnDef {
                name: "attrelid".to_string(),
                data_type: DataType::BigInt,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "attname".to_string(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "atttypid".to_string(),
                data_type: DataType::BigInt,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "attlen".to_string(),
                data_type: DataType::Int,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "attnum".to_string(),
                data_type: DataType::Int,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "attndims".to_string(),
                data_type: DataType::Int,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "attnotnull".to_string(),
                data_type: DataType::Boolean,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "atthasdef".to_string(),
                data_type: DataType::Boolean,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "attisdropped".to_string(),
                data_type: DataType::Boolean,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "atttypmod".to_string(),
                data_type: DataType::Int,
                nullable: false,
                generated: None,
            },
        ],
        primary_key: vec!["attrelid".to_string(), "attnum".to_string()],
        indexes: vec![],
    }
}

/// Standard node columns for RaisinDB workspace tables
/// These columns map to the Node struct
pub fn node_columns() -> Vec<(&'static str, DataType, bool)> {
    vec![
        ("id", DataType::Text, false),
        ("name", DataType::Text, false),
        ("path", DataType::Text, false),
        ("node_type", DataType::Text, false),
        ("archetype", DataType::Text, true),
        ("properties", DataType::JsonB, false),
        ("parent", DataType::Text, true),
        ("version", DataType::Int, false),
        ("created_at", DataType::TimestampTz, true),
        ("updated_at", DataType::TimestampTz, true),
        ("published_at", DataType::TimestampTz, true),
        ("published_by", DataType::Text, true),
    ]
}

/// Map RaisinDB DataType to PostgreSQL type OID
pub fn datatype_to_pg_oid(dt: &DataType) -> i64 {
    match dt {
        DataType::Boolean => oids::BOOL,
        DataType::Int => oids::INT4,
        DataType::BigInt => oids::INT8,
        DataType::Double => oids::FLOAT8,
        DataType::Text => oids::TEXT,
        DataType::TimestampTz => oids::TIMESTAMPTZ,
        DataType::JsonB => oids::JSONB,
        DataType::Uuid => oids::UUID,
        DataType::Nullable(inner) => datatype_to_pg_oid(inner),
        _ => oids::TEXT, // Fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_pg_catalog_table() {
        assert!(is_pg_catalog_table(Some("pg_catalog"), "pg_type"));
        assert!(is_pg_catalog_table(Some("pg_catalog"), "pg_namespace"));
        assert!(is_pg_catalog_table(None, "pg_type"));
        assert!(!is_pg_catalog_table(Some("public"), "pg_type"));
        assert!(!is_pg_catalog_table(Some("pg_catalog"), "users"));
    }

    #[test]
    fn test_get_pg_catalog_table() {
        assert!(get_pg_catalog_table("pg_type").is_some());
        assert!(get_pg_catalog_table("pg_namespace").is_some());
        assert!(get_pg_catalog_table("pg_class").is_some());
        assert!(get_pg_catalog_table("pg_attribute").is_some());
        assert!(get_pg_catalog_table("nonexistent").is_none());
    }

    #[test]
    fn test_pg_type_columns() {
        let table = pg_type_table_def();
        assert_eq!(table.name, "pg_type");
        assert!(table.get_column("oid").is_some());
        assert!(table.get_column("typname").is_some());
        assert!(table.get_column("typnamespace").is_some());
    }

    #[test]
    fn test_datatype_to_pg_oid() {
        assert_eq!(datatype_to_pg_oid(&DataType::Int), oids::INT4);
        assert_eq!(datatype_to_pg_oid(&DataType::Text), oids::TEXT);
        assert_eq!(datatype_to_pg_oid(&DataType::JsonB), oids::JSONB);
        assert_eq!(
            datatype_to_pg_oid(&DataType::Nullable(Box::new(DataType::Int))),
            oids::INT4
        );
    }
}
