//! PostgreSQL System Catalog Virtual Table Executor
//!
//! Generates rows for pg_catalog virtual tables (pg_type, pg_namespace, pg_class, pg_attribute)
//! to support GUI clients like Beekeeper Studio, DBeaver, and pgAdmin.

use super::executor::{ExecutionError, Row, RowStream};
use async_stream::try_stream;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::pg_catalog::oids;
use raisin_storage::Storage;

/// Execute a pg_catalog virtual table scan
///
/// This function generates static rows for PostgreSQL system catalog tables.
/// It returns metadata about RaisinDB types and schema in PostgreSQL format.
///
/// # Arguments
///
/// * `table_name` - The pg_catalog table name (e.g., "pg_type", "pg_namespace")
/// * `storage` - Storage implementation for fetching workspace metadata
/// * `tenant_id` - Tenant identifier
/// * `repo_id` - Repository identifier
///
/// # Returns
///
/// A stream of rows containing the virtual table data
pub async fn execute_pg_catalog_scan<S: Storage + 'static>(
    table_name: &str,
    _storage: std::sync::Arc<S>,
    _tenant_id: &str,
    _repo_id: &str,
) -> Result<RowStream, ExecutionError> {
    match table_name.to_lowercase().as_str() {
        "pg_type" => execute_pg_type_scan().await,
        "pg_namespace" => execute_pg_namespace_scan().await,
        "pg_class" => execute_pg_class_scan().await,
        "pg_attribute" => execute_pg_attribute_scan().await,
        _ => Err(ExecutionError::Validation(format!(
            "Unknown pg_catalog table: {}",
            table_name
        ))),
    }
}

/// Check if a table name is a pg_catalog system table
pub fn is_pg_catalog_table(table_name: &str) -> bool {
    let name = table_name.to_lowercase();
    // Handle both "pg_type" and "pg_catalog.pg_type" formats
    let simple_name = if let Some(pos) = name.rfind('.') {
        &name[pos + 1..]
    } else {
        &name
    };

    matches!(
        simple_name,
        "pg_type" | "pg_namespace" | "pg_class" | "pg_attribute"
    )
}

/// Extract simple table name from potentially qualified name
pub fn get_simple_pg_catalog_table_name(table_name: &str) -> &str {
    if let Some(pos) = table_name.rfind('.') {
        &table_name[pos + 1..]
    } else {
        table_name
    }
}

/// Generate rows for pg_type - PostgreSQL data types
async fn execute_pg_type_scan() -> Result<RowStream, ExecutionError> {
    let stream = try_stream! {
        // Generate rows for common PostgreSQL types that GUI clients expect
        let types = vec![
            // (oid, typname, typnamespace, typlen, typbyval, typtype, typcategory)
            (oids::BOOL, "bool", oids::PG_CATALOG_NAMESPACE, 1, true, "b", "B"),
            (oids::INT2, "int2", oids::PG_CATALOG_NAMESPACE, 2, true, "b", "N"),
            (oids::INT4, "int4", oids::PG_CATALOG_NAMESPACE, 4, true, "b", "N"),
            (oids::INT8, "int8", oids::PG_CATALOG_NAMESPACE, 8, true, "b", "N"),
            (oids::FLOAT4, "float4", oids::PG_CATALOG_NAMESPACE, 4, true, "b", "N"),
            (oids::FLOAT8, "float8", oids::PG_CATALOG_NAMESPACE, 8, true, "b", "N"),
            (oids::TEXT, "text", oids::PG_CATALOG_NAMESPACE, -1, false, "b", "S"),
            (oids::VARCHAR, "varchar", oids::PG_CATALOG_NAMESPACE, -1, false, "b", "S"),
            (oids::TIMESTAMP, "timestamp", oids::PG_CATALOG_NAMESPACE, 8, true, "b", "D"),
            (oids::TIMESTAMPTZ, "timestamptz", oids::PG_CATALOG_NAMESPACE, 8, true, "b", "D"),
            (oids::JSONB, "jsonb", oids::PG_CATALOG_NAMESPACE, -1, false, "b", "U"),
            (oids::UUID, "uuid", oids::PG_CATALOG_NAMESPACE, 16, false, "b", "U"),
            (oids::OID, "oid", oids::PG_CATALOG_NAMESPACE, 4, true, "b", "N"),
            (oids::NAME, "name", oids::PG_CATALOG_NAMESPACE, 64, false, "b", "S"),
            (oids::CHAR, "char", oids::PG_CATALOG_NAMESPACE, 1, true, "b", "S"),
            (oids::REGPROC, "regproc", oids::PG_CATALOG_NAMESPACE, 4, true, "b", "N"),
        ];

        for (oid, typname, typnamespace, typlen, typbyval, typtype, typcategory) in types {
            let mut row = Row::new();
            row.insert("oid".to_string(), PropertyValue::Integer(oid));
            row.insert("typname".to_string(), PropertyValue::String(typname.to_string()));
            row.insert("typnamespace".to_string(), PropertyValue::Integer(typnamespace));
            row.insert("typowner".to_string(), PropertyValue::Integer(10)); // postgres superuser
            row.insert("typlen".to_string(), PropertyValue::Integer(typlen as i64));
            row.insert("typbyval".to_string(), PropertyValue::Boolean(typbyval));
            row.insert("typtype".to_string(), PropertyValue::String(typtype.to_string()));
            row.insert("typcategory".to_string(), PropertyValue::String(typcategory.to_string()));
            row.insert("typispreferred".to_string(), PropertyValue::Boolean(false));
            row.insert("typisdefined".to_string(), PropertyValue::Boolean(true));
            row.insert("typdelim".to_string(), PropertyValue::String(",".to_string()));
            row.insert("typrelid".to_string(), PropertyValue::Integer(0));
            row.insert("typelem".to_string(), PropertyValue::Integer(0));
            row.insert("typarray".to_string(), PropertyValue::Integer(0));
            row.insert("typinput".to_string(), PropertyValue::String(format!("{}in", typname)));
            row.insert("typoutput".to_string(), PropertyValue::String(format!("{}out", typname)));
            yield row;
        }
    };

    Ok(Box::pin(stream))
}

/// Generate rows for pg_namespace - PostgreSQL schemas
async fn execute_pg_namespace_scan() -> Result<RowStream, ExecutionError> {
    let stream = try_stream! {
        // Generate rows for standard PostgreSQL schemas
        let namespaces = vec![
            (oids::PG_CATALOG_NAMESPACE, "pg_catalog", 10),
            (oids::PUBLIC_NAMESPACE, "public", 10),
            (oids::INFORMATION_SCHEMA_NAMESPACE, "information_schema", 10),
        ];

        for (oid, nspname, nspowner) in namespaces {
            let mut row = Row::new();
            row.insert("oid".to_string(), PropertyValue::Integer(oid));
            row.insert("nspname".to_string(), PropertyValue::String(nspname.to_string()));
            row.insert("nspowner".to_string(), PropertyValue::Integer(nspowner));
            row.insert("nspacl".to_string(), PropertyValue::Null);
            yield row;
        }
    };

    Ok(Box::pin(stream))
}

/// Generate rows for pg_class - Tables, indexes, views, etc.
///
/// This returns RaisinDB workspaces as tables in the public schema.
async fn execute_pg_class_scan() -> Result<RowStream, ExecutionError> {
    let stream = try_stream! {
        // Return a generic "nodes" table that represents RaisinDB's structure
        // In the future, we could dynamically list workspaces here
        let classes = vec![
            // (oid, relname, relnamespace, relkind, reltuples)
            // relkind: 'r' = ordinary table, 'i' = index, 'v' = view
            (16385i64, "nodes", oids::PUBLIC_NAMESPACE, "r", 0.0f64),
        ];

        for (oid, relname, relnamespace, relkind, reltuples) in classes {
            let mut row = Row::new();
            row.insert("oid".to_string(), PropertyValue::Integer(oid));
            row.insert("relname".to_string(), PropertyValue::String(relname.to_string()));
            row.insert("relnamespace".to_string(), PropertyValue::Integer(relnamespace));
            row.insert("reltype".to_string(), PropertyValue::Integer(0));
            row.insert("reloftype".to_string(), PropertyValue::Integer(0));
            row.insert("relowner".to_string(), PropertyValue::Integer(10));
            row.insert("relkind".to_string(), PropertyValue::String(relkind.to_string()));
            row.insert("reltuples".to_string(), PropertyValue::Float(reltuples));
            row.insert("relhasindex".to_string(), PropertyValue::Boolean(true));
            row.insert("relisshared".to_string(), PropertyValue::Boolean(false));
            row.insert("relpersistence".to_string(), PropertyValue::String("p".to_string()));
            row.insert("relnatts".to_string(), PropertyValue::Integer(12)); // Number of Node struct fields
            yield row;
        }
    };

    Ok(Box::pin(stream))
}

/// Generate rows for pg_attribute - Table columns
///
/// This returns the columns of the RaisinDB Node struct.
async fn execute_pg_attribute_scan() -> Result<RowStream, ExecutionError> {
    let stream = try_stream! {
        // Columns of the "nodes" table (matches Node struct)
        // (attrelid, attname, atttypid, attlen, attnum, attnotnull)
        let table_oid = 16385i64; // Matches pg_class oid for "nodes"
        let attributes = vec![
            ("id", oids::TEXT, -1i32, 1i16, false),
            ("name", oids::TEXT, -1, 2, false),
            ("path", oids::TEXT, -1, 3, false),
            ("node_type", oids::TEXT, -1, 4, false),
            ("archetype", oids::TEXT, -1, 5, true),
            ("properties", oids::JSONB, -1, 6, false),
            ("parent", oids::TEXT, -1, 7, true),
            ("version", oids::INT4, 4, 8, false),
            ("created_at", oids::TIMESTAMPTZ, 8, 9, true),
            ("updated_at", oids::TIMESTAMPTZ, 8, 10, true),
            ("published_at", oids::TIMESTAMPTZ, 8, 11, true),
            ("published_by", oids::TEXT, -1, 12, true),
        ];

        for (attname, atttypid, attlen, attnum, nullable) in attributes {
            let mut row = Row::new();
            row.insert("attrelid".to_string(), PropertyValue::Integer(table_oid));
            row.insert("attname".to_string(), PropertyValue::String(attname.to_string()));
            row.insert("atttypid".to_string(), PropertyValue::Integer(atttypid));
            row.insert("attlen".to_string(), PropertyValue::Integer(attlen as i64));
            row.insert("attnum".to_string(), PropertyValue::Integer(attnum as i64));
            row.insert("attndims".to_string(), PropertyValue::Integer(0));
            row.insert("attnotnull".to_string(), PropertyValue::Boolean(!nullable));
            row.insert("atthasdef".to_string(), PropertyValue::Boolean(false));
            row.insert("attisdropped".to_string(), PropertyValue::Boolean(false));
            row.insert("atttypmod".to_string(), PropertyValue::Integer(-1));
            yield row;
        }
    };

    Ok(Box::pin(stream))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn test_is_pg_catalog_table() {
        assert!(is_pg_catalog_table("pg_type"));
        assert!(is_pg_catalog_table("pg_catalog.pg_type"));
        assert!(is_pg_catalog_table("PG_TYPE"));
        assert!(is_pg_catalog_table("pg_namespace"));
        assert!(!is_pg_catalog_table("users"));
        assert!(!is_pg_catalog_table("nodes"));
    }

    #[test]
    fn test_get_simple_pg_catalog_table_name() {
        assert_eq!(get_simple_pg_catalog_table_name("pg_type"), "pg_type");
        assert_eq!(
            get_simple_pg_catalog_table_name("pg_catalog.pg_type"),
            "pg_type"
        );
    }

    #[tokio::test]
    async fn test_pg_type_scan() {
        let stream = execute_pg_type_scan().await.unwrap();
        let rows: Vec<_> = stream.collect::<Vec<_>>().await;

        assert!(!rows.is_empty());
        // Check that bool type is present
        let bool_row = rows
            .iter()
            .find(|r| {
                r.as_ref()
                    .ok()
                    .and_then(|row| row.get("typname"))
                    .map(|v| matches!(v, PropertyValue::String(s) if s == "bool"))
                    .unwrap_or(false)
            })
            .expect("bool type should exist");

        let row = bool_row.as_ref().unwrap();
        assert_eq!(row.get("oid"), Some(&PropertyValue::Integer(oids::BOOL)));
    }

    #[tokio::test]
    async fn test_pg_namespace_scan() {
        let stream = execute_pg_namespace_scan().await.unwrap();
        let rows: Vec<_> = stream.collect::<Vec<_>>().await;

        assert_eq!(rows.len(), 3); // pg_catalog, public, information_schema
    }

    #[tokio::test]
    async fn test_pg_class_scan() {
        let stream = execute_pg_class_scan().await.unwrap();
        let rows: Vec<_> = stream.collect::<Vec<_>>().await;

        assert!(!rows.is_empty());
        // Check nodes table exists
        let nodes_row = rows.iter().find(|r| {
            r.as_ref()
                .ok()
                .and_then(|row| row.get("relname"))
                .map(|v| matches!(v, PropertyValue::String(s) if s == "nodes"))
                .unwrap_or(false)
        });
        assert!(nodes_row.is_some());
    }

    #[tokio::test]
    async fn test_pg_attribute_scan() {
        let stream = execute_pg_attribute_scan().await.unwrap();
        let rows: Vec<_> = stream.collect::<Vec<_>>().await;

        assert_eq!(rows.len(), 12); // 12 Node struct fields
    }
}
