//! TO_JSON and TO_JSONB functions - convert values/rows to JSON

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use indexmap::IndexMap;
use raisin_error::Error;
use raisin_sql::analyzer::{Expr, Literal, TypedExpr};
use serde_json::Value as JsonValue;

/// Convert a value or table row to JSONB
///
/// # SQL Signature
/// `TO_JSON(expr) -> JSONB`
/// `TO_JSONB(expr) -> JSONB`
///
/// # Arguments
/// * `expr` - Expression to convert to JSON
///   - When given a table alias (e.g., `TO_JSON(t)`), returns all columns from that table as a JSON object
///   - When given a column, converts that column value to JSON
///
/// # Returns
/// * JSONB representation of the input
///
/// # Examples
/// ```sql
/// -- Convert entire table row to JSON
/// SELECT TO_JSON(t) FROM cypher('...') t
/// -> {"a_id": "123", "name": "Alice", ...}
///
/// -- Convert a column to JSON
/// SELECT TO_JSON(properties) FROM nodes
/// ```
pub struct ToJsonFunction;

impl SqlFunction for ToJsonFunction {
    fn name(&self) -> &str {
        "TO_JSON"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Json
    }

    fn signature(&self) -> &str {
        "TO_JSON(expr) -> JSONB"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count
        if args.len() != 1 {
            return Err(Error::Validation(
                "TO_JSON requires exactly 1 argument".to_string(),
            ));
        }

        let arg = &args[0];

        // Check if this is a table reference (where column == table)
        // This is how semantic analyzer marks table-only references
        if let Expr::Column { table, column } = &arg.expr {
            if table == column {
                // This is a table reference - bundle all columns from this table
                return table_row_to_json(row, table);
            }
        }

        // Otherwise, evaluate the expression and convert to JSON
        let value = eval_expr(arg, row)?;
        value_to_json(value)
    }
}

/// TO_JSONB is an alias for TO_JSON
pub struct ToJsonbFunction;

impl SqlFunction for ToJsonbFunction {
    fn name(&self) -> &str {
        "TO_JSONB"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Json
    }

    fn signature(&self) -> &str {
        "TO_JSONB(expr) -> JSONB"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // TO_JSONB is identical to TO_JSON - just delegate
        ToJsonFunction.evaluate(args, row)
    }
}

/// Convert all columns from a table to a JSON object
///
/// Gathers all columns that have the format "table_name.column_name"
/// and creates a JSON object with just the column names as keys.
fn table_row_to_json(row: &Row, table_name: &str) -> Result<Literal, Error> {
    let mut json_object = IndexMap::new();
    let prefix = format!("{}.", table_name);

    // Iterate through all columns in the row
    for (col_name, col_value) in row.columns.iter() {
        // Check if this column belongs to the specified table
        if let Some(unqualified_name) = col_name.strip_prefix(&prefix) {
            // Convert PropertyValue to JSON
            let json_value = property_value_to_json(col_value)?;
            json_object.insert(unqualified_name.to_string(), json_value);
        }
    }

    // If no columns found, return empty object
    if json_object.is_empty() {
        return Ok(Literal::JsonB(JsonValue::Object(serde_json::Map::new())));
    }

    // Convert IndexMap to serde_json::Map
    let mut map = serde_json::Map::new();
    for (key, value) in json_object {
        map.insert(key, value);
    }

    Ok(Literal::JsonB(JsonValue::Object(map)))
}

/// Convert a Literal value to JSON
fn value_to_json(literal: Literal) -> Result<Literal, Error> {
    let json_value = match literal {
        Literal::Null => JsonValue::Null,
        Literal::Boolean(b) => JsonValue::Bool(b),
        Literal::Int(i) => JsonValue::Number(i.into()),
        Literal::BigInt(i) => JsonValue::Number(i.into()),
        Literal::Double(f) => serde_json::Number::from_f64(f)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Literal::Text(s) | Literal::Path(s) | Literal::Uuid(s) => JsonValue::String(s),
        Literal::JsonB(j) => j, // Already JSON
        Literal::Vector(v) => {
            let array: Vec<JsonValue> = v
                .into_iter()
                .map(|f| {
                    serde_json::Number::from_f64(f as f64)
                        .map(JsonValue::Number)
                        .unwrap_or(JsonValue::Null)
                })
                .collect();
            JsonValue::Array(array)
        }
        Literal::Geometry(geojson) => geojson, // Already JSON
        Literal::Timestamp(ts) => JsonValue::String(ts.to_rfc3339()),
        Literal::Interval(duration) => {
            // Convert duration to a simple string representation
            JsonValue::String(format!("{} seconds", duration.num_seconds()))
        }
        Literal::Parameter(p) => JsonValue::String(p),
    };

    Ok(Literal::JsonB(json_value))
}

/// Convert a PropertyValue to serde_json::Value
fn property_value_to_json(
    value: &raisin_models::nodes::properties::PropertyValue,
) -> Result<JsonValue, Error> {
    use raisin_models::nodes::properties::PropertyValue;

    let json_value = match value {
        PropertyValue::Null => JsonValue::Null,
        PropertyValue::Boolean(b) => JsonValue::Bool(*b),
        PropertyValue::Integer(n) => JsonValue::Number((*n).into()),
        PropertyValue::Float(n) => serde_json::Number::from_f64(*n)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        PropertyValue::Decimal(d) => {
            use std::str::FromStr;
            let f = f64::from_str(&d.to_string()).unwrap_or(0.0);
            serde_json::Number::from_f64(f)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null)
        }
        PropertyValue::String(s) => JsonValue::String(s.clone()),
        PropertyValue::Url(url) => JsonValue::String(url.url.clone()),
        PropertyValue::Date(dt) => JsonValue::String(dt.to_string()),
        PropertyValue::Vector(vec) => {
            let json_arr: Vec<JsonValue> = vec
                .iter()
                .map(|f| {
                    serde_json::Number::from_f64(*f as f64)
                        .map(JsonValue::Number)
                        .unwrap_or(JsonValue::Null)
                })
                .collect();
            JsonValue::Array(json_arr)
        }
        PropertyValue::Array(arr) => {
            let json_arr: Result<Vec<JsonValue>, Error> =
                arr.iter().map(property_value_to_json).collect();
            JsonValue::Array(json_arr?)
        }
        PropertyValue::Object(obj) => {
            let mut map = serde_json::Map::new();
            for (key, val) in obj {
                map.insert(key.clone(), property_value_to_json(val)?);
            }
            JsonValue::Object(map)
        }
        PropertyValue::Reference(r) => {
            // Convert reference to a JSON object with type and id
            let mut map = serde_json::Map::new();
            map.insert(
                "type".to_string(),
                JsonValue::String("reference".to_string()),
            );
            map.insert("id".to_string(), JsonValue::String(r.id.clone()));
            map.insert(
                "workspace".to_string(),
                JsonValue::String(r.workspace.clone()),
            );
            map.insert("path".to_string(), JsonValue::String(r.path.clone()));
            JsonValue::Object(map)
        }
        PropertyValue::Resource(res) => {
            // Convert resource to JSON - it's already serializable
            serde_json::to_value(res).map_err(|e| {
                Error::Validation(format!("Failed to convert Resource to JSON: {}", e))
            })?
        }
        PropertyValue::Composite(c) => {
            // Convert composite to JSON - it's already serializable
            serde_json::to_value(c).map_err(|e| {
                Error::Validation(format!("Failed to convert Composite to JSON: {}", e))
            })?
        }
        PropertyValue::Element(e) => {
            // Convert element to JSON - it's already serializable
            serde_json::to_value(e).map_err(|e| {
                Error::Validation(format!("Failed to convert Element to JSON: {}", e))
            })?
        }
        PropertyValue::Geometry(geojson) => {
            // Serialize GeoJson to serde_json::Value
            serde_json::to_value(geojson).map_err(|e| {
                Error::Validation(format!("Failed to convert Geometry to JSON: {}", e))
            })?
        }
    };

    Ok(json_value)
}
