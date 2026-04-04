//! Type casting operations for literals

use chrono::{DateTime, Utc};
use raisin_error::Error;
use raisin_sql::analyzer::{DataType, Literal};

/// Cast a literal to a target type
///
/// Supports conversions between:
/// - Numeric types (Int, BigInt, Double)
/// - Text to/from numeric types
/// - Text to Path (with validation)
/// - Boolean to/from Text
/// - JsonB to/from Text
/// - Primitive types (Int, BigInt, Double, Boolean, Path, Timestamp) to JsonB
/// - NULL handling (CAST(NULL AS any_type) = NULL per SQL standard)
pub(super) fn cast_literal(value: Literal, target_type: &DataType) -> Result<Literal, Error> {
    match (value, target_type.base_type()) {
        // NULL can be cast to any type and remains NULL (SQL standard)
        (Literal::Null, _) => Ok(Literal::Null),

        // Already correct type
        (v, _) if v.data_type() == *target_type.base_type() => Ok(v),

        // To Text
        (Literal::Int(i), DataType::Text) => Ok(Literal::Text(i.to_string())),
        (Literal::BigInt(i), DataType::Text) => Ok(Literal::Text(i.to_string())),
        (Literal::Double(f), DataType::Text) => Ok(Literal::Text(f.to_string())),
        (Literal::Boolean(b), DataType::Text) => Ok(Literal::Text(b.to_string())),
        (Literal::Path(p), DataType::Text) => Ok(Literal::Text(p)),
        (Literal::JsonB(j), DataType::Text) => Ok(Literal::Text(j.to_string())),

        // To Int
        (Literal::Text(s), DataType::Int) => {
            // First try parsing as integer directly
            if let Ok(i) = s.parse::<i32>() {
                return Ok(Literal::Int(i));
            }
            // If that fails, try parsing as float and truncate (handles "0.0", "123.45", etc.)
            // This matches MySQL behavior and is more user-friendly than PostgreSQL's strict approach
            s.parse::<f64>()
                .map(|f| Literal::Int(f as i32))
                .map_err(|_| Error::Validation(format!("Cannot cast '{}' to INT", s)))
        }
        (Literal::Double(f), DataType::Int) => Ok(Literal::Int(f as i32)),
        (Literal::BigInt(i), DataType::Int) => Ok(Literal::Int(i as i32)),

        // To BigInt
        (Literal::Text(s), DataType::BigInt) => {
            // First try parsing as integer directly
            if let Ok(i) = s.parse::<i64>() {
                return Ok(Literal::BigInt(i));
            }
            // If that fails, try parsing as float and truncate
            s.parse::<f64>()
                .map(|f| Literal::BigInt(f as i64))
                .map_err(|_| Error::Validation(format!("Cannot cast '{}' to BIGINT", s)))
        }
        (Literal::Int(i), DataType::BigInt) => Ok(Literal::BigInt(i as i64)),
        (Literal::Double(f), DataType::BigInt) => Ok(Literal::BigInt(f as i64)),

        // To Double
        (Literal::Int(i), DataType::Double) => Ok(Literal::Double(i as f64)),
        (Literal::BigInt(i), DataType::Double) => Ok(Literal::Double(i as f64)),
        (Literal::Text(s), DataType::Double) => s
            .parse::<f64>()
            .map(Literal::Double)
            .map_err(|_| Error::Validation(format!("Cannot cast '{}' to DOUBLE", s))),

        // To Boolean
        (Literal::Text(s), DataType::Boolean) => {
            let lower = s.to_lowercase();
            match lower.as_str() {
                "true" | "t" | "yes" | "y" | "1" => Ok(Literal::Boolean(true)),
                "false" | "f" | "no" | "n" | "0" => Ok(Literal::Boolean(false)),
                _ => Err(Error::Validation(format!("Cannot cast '{}' to BOOLEAN", s))),
            }
        }

        // To Path
        (Literal::Text(s), DataType::Path) => {
            // Validate path syntax
            if !s.starts_with('/') {
                return Err(Error::Validation(format!(
                    "Invalid path: must start with '/', got '{}'",
                    s
                )));
            }
            Ok(Literal::Path(s))
        }

        // To JsonB
        (Literal::Text(s), DataType::JsonB) => {
            // Parse JSON string
            serde_json::from_str(&s)
                .map(Literal::JsonB)
                .map_err(|e| Error::Validation(format!("Cannot cast '{}' to JSONB: {}", s, e)))
        }
        (Literal::Double(f), DataType::JsonB) => Ok(Literal::JsonB(serde_json::json!(f))),
        (Literal::Int(i), DataType::JsonB) => Ok(Literal::JsonB(serde_json::json!(i))),
        (Literal::BigInt(i), DataType::JsonB) => Ok(Literal::JsonB(serde_json::json!(i))),
        (Literal::Boolean(b), DataType::JsonB) => Ok(Literal::JsonB(serde_json::json!(b))),
        (Literal::Path(p), DataType::JsonB) => Ok(Literal::JsonB(serde_json::json!(p))),
        (Literal::Timestamp(ts), DataType::JsonB) => {
            Ok(Literal::JsonB(serde_json::json!(ts.to_rfc3339())))
        }

        // To TimestampTz - parse ISO 8601 and common date formats
        (Literal::Text(s), DataType::TimestampTz) => {
            parse_timestamp(&s).map(Literal::Timestamp).ok_or_else(|| {
                Error::Validation(format!(
                    "Cannot cast '{}' to TIMESTAMPTZ: invalid timestamp format",
                    s
                ))
            })
        }

        // From TimestampTz to Text - format as ISO 8601
        (Literal::Timestamp(ts), DataType::Text) => Ok(Literal::Text(ts.to_rfc3339())),

        // To Geometry (from GeoJSON text)
        (Literal::Text(s), DataType::Geometry) => serde_json::from_str::<serde_json::Value>(&s)
            .map(Literal::Geometry)
            .map_err(|e| {
                Error::Validation(format!(
                    "Cannot cast '{}' to GEOMETRY: invalid GeoJSON: {}",
                    s, e
                ))
            }),

        // To Geometry (from JsonB)
        (Literal::JsonB(v), DataType::Geometry) => Ok(Literal::Geometry(v)),

        // From Geometry to Text (GeoJSON serialization)
        (Literal::Geometry(v), DataType::Text) => Ok(Literal::Text(v.to_string())),

        (v, t) => Err(Error::Validation(format!("Cannot cast {:?} to {}", v, t))),
    }
}

/// Parse a timestamp string in various formats
/// Supports ISO 8601 and common date/datetime formats
pub(super) fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    // Try RFC 3339 first (ISO 8601 with timezone) e.g., "2024-01-15T10:30:00Z"
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }

    // Try ISO 8601 without timezone (assume UTC) e.g., "2024-01-15T10:30:00"
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(dt.and_utc());
    }

    // Try with milliseconds e.g., "2024-01-15T10:30:00.123"
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(dt.and_utc());
    }

    // Try date only e.g., "2024-01-15" (interpret as midnight UTC)
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return date.and_hms_opt(0, 0, 0).map(|dt| dt.and_utc());
    }

    // Try space-separated datetime e.g., "2024-01-15 10:30:00"
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc());
    }

    // Try with space-separated and milliseconds e.g., "2024-01-15 10:30:00.123"
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
        return Some(dt.and_utc());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[test]
    fn test_parse_timestamp_rfc3339_with_z() {
        let ts = parse_timestamp("2026-01-04T20:35:29Z");
        assert!(ts.is_some(), "Should parse RFC 3339 with Z suffix");

        let expected = chrono::DateTime::parse_from_rfc3339("2026-01-04T20:35:29Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(ts.unwrap(), expected);
    }

    #[test]
    fn test_parse_timestamp_rfc3339_with_offset() {
        let ts = parse_timestamp("2026-01-04T20:35:29+00:00");
        assert!(ts.is_some(), "Should parse RFC 3339 with +00:00 offset");

        let expected = chrono::DateTime::parse_from_rfc3339("2026-01-04T20:35:29+00:00")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(ts.unwrap(), expected);
    }

    #[test]
    fn test_parse_timestamp_rfc3339_with_microseconds_and_offset() {
        // This is the exact format from the bug report
        let ts = parse_timestamp("2026-01-04T20:35:29.900186+00:00");
        assert!(
            ts.is_some(),
            "Should parse RFC 3339 with microseconds and offset"
        );

        let expected = chrono::DateTime::parse_from_rfc3339("2026-01-04T20:35:29.900186+00:00")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(ts.unwrap(), expected);
    }

    #[test]
    fn test_parse_timestamp_rfc3339_with_microseconds_and_z() {
        let ts = parse_timestamp("2026-01-04T20:35:29.900186Z");
        assert!(
            ts.is_some(),
            "Should parse RFC 3339 with microseconds and Z"
        );

        let expected = chrono::DateTime::parse_from_rfc3339("2026-01-04T20:35:29.900186Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(ts.unwrap(), expected);
    }

    #[test]
    fn test_parse_timestamp_iso8601_without_timezone() {
        let ts = parse_timestamp("2026-01-04T20:35:29");
        assert!(ts.is_some(), "Should parse ISO 8601 without timezone");
    }

    #[test]
    fn test_parse_timestamp_with_milliseconds() {
        let ts = parse_timestamp("2026-01-04T20:35:29.123");
        assert!(ts.is_some(), "Should parse timestamp with milliseconds");
    }

    #[test]
    fn test_parse_timestamp_date_only() {
        let ts = parse_timestamp("2026-01-04");
        assert!(ts.is_some(), "Should parse date only");

        // Should be midnight UTC
        let expected = ts.unwrap();
        assert_eq!(expected.hour(), 0);
        assert_eq!(expected.minute(), 0);
        assert_eq!(expected.second(), 0);
    }

    #[test]
    fn test_parse_timestamp_space_separated() {
        let ts = parse_timestamp("2026-01-04 20:35:29");
        assert!(ts.is_some(), "Should parse space-separated datetime");
    }

    #[test]
    fn test_parse_timestamp_space_separated_with_milliseconds() {
        let ts = parse_timestamp("2026-01-04 20:35:29.123");
        assert!(
            ts.is_some(),
            "Should parse space-separated datetime with milliseconds"
        );
    }

    #[test]
    fn test_parse_timestamp_invalid() {
        assert!(parse_timestamp("not a timestamp").is_none());
        assert!(parse_timestamp("2026-13-04").is_none()); // Invalid month
        assert!(parse_timestamp("").is_none());
    }

    #[test]
    fn test_cast_text_to_timestamptz() {
        let result = cast_literal(
            Literal::Text("2026-01-04T20:35:29.900186+00:00".to_string()),
            &DataType::TimestampTz,
        );
        assert!(result.is_ok(), "Should cast text to timestamptz");

        match result.unwrap() {
            Literal::Timestamp(ts) => {
                assert_eq!(ts.year(), 2026);
                assert_eq!(ts.month(), 1);
                assert_eq!(ts.day(), 4);
            }
            _ => panic!("Expected Timestamp literal"),
        }
    }

    #[test]
    fn test_cast_invalid_text_to_timestamptz() {
        let result = cast_literal(
            Literal::Text("not a timestamp".to_string()),
            &DataType::TimestampTz,
        );
        assert!(
            result.is_err(),
            "Should fail to cast invalid text to timestamptz"
        );
    }

    #[test]
    fn test_cast_text_to_geometry() {
        let geojson = r#"{"type":"Point","coordinates":[52.207,52.9089]}"#;
        let result = cast_literal(Literal::Text(geojson.to_string()), &DataType::Geometry);
        assert!(result.is_ok());
        match result.unwrap() {
            Literal::Geometry(v) => assert_eq!(v["type"], "Point"),
            other => panic!("Expected Geometry, got {:?}", other),
        }
    }

    #[test]
    fn test_cast_invalid_text_to_geometry() {
        let result = cast_literal(Literal::Text("not json".to_string()), &DataType::Geometry);
        assert!(result.is_err());
    }

    #[test]
    fn test_cast_jsonb_to_geometry() {
        let geojson = serde_json::json!({"type": "Point", "coordinates": [52.207, 52.9089]});
        let result = cast_literal(Literal::JsonB(geojson.clone()), &DataType::Geometry);
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), Literal::Geometry(v) if v == geojson));
    }

    #[test]
    fn test_cast_geometry_to_text() {
        let geojson = serde_json::json!({"type": "Point", "coordinates": [52.207, 52.9089]});
        let result = cast_literal(Literal::Geometry(geojson.clone()), &DataType::Text);
        assert!(result.is_ok());
        match result.unwrap() {
            Literal::Text(s) => {
                let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
                assert_eq!(parsed, geojson);
            }
            other => panic!("Expected Text, got {:?}", other),
        }
    }

    #[test]
    fn test_cast_null_to_geometry() {
        let result = cast_literal(Literal::Null, &DataType::Geometry);
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), Literal::Null));
    }
}
