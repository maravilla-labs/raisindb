//! Helper utilities for node repository operations

use raisin_models::nodes::properties::PropertyValue;

/// Tombstone marker (single byte 'T' for debugging visibility)
pub(crate) const TOMBSTONE: &[u8] = b"T";

/// Check if a node value represents a tombstone (deleted node)
pub(crate) fn is_tombstone(value: &[u8]) -> bool {
    value == TOMBSTONE
}

/// Hash a property value for indexing
///
/// Creates a stable string representation suitable for use in property index keys.
/// For complex types, uses a consistent serialization format.
///
/// **Temporal Properties Optimization:**
/// Date/Timestamp values are encoded as zero-padded Unix nanosecond timestamps
/// to enable efficient lexicographic range scans for ORDER BY queries.
/// This allows O(limit) performance instead of O(n log n) for time-series queries.
pub(crate) fn hash_property_value(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Null => "null".to_string(),
        PropertyValue::String(s) => s.clone(),
        PropertyValue::Integer(i) => i.to_string(),
        PropertyValue::Float(f) => f.to_string(),
        PropertyValue::Decimal(d) => d.to_string(),
        PropertyValue::Boolean(b) => b.to_string(),
        PropertyValue::Date(d) => {
            // Encode as sortable Unix timestamp (nanoseconds since epoch)
            // Zero-padded to 20 digits for lexicographic ordering
            // Example: 2025-01-15T10:30:00Z → "01736937000000000000"
            //
            // This enables efficient range scans for ORDER BY created_at/updated_at
            // queries without requiring full table scan + in-memory sort.
            let nanos = d.timestamp_nanos_opt().unwrap_or(0);
            // Use i128 to handle full nanosecond range, format with leading zeros
            format!("{:020}", nanos as i128) // 20 digits handles ~2554 AD
        }
        PropertyValue::Url(u) => u.url.clone(),
        PropertyValue::Reference(r) => format!("ref:{}", r.id),
        PropertyValue::Resource(res) => format!("resource:{}", res.uuid),
        PropertyValue::Element(block) => format!("block:{}", block.uuid),
        PropertyValue::Composite(container) => format!("container:{}", container.uuid),
        PropertyValue::Vector(v) => {
            // For vectors, create a compact representation with dimensions
            // Don't serialize full vector to avoid huge index keys
            format!("vector:{}d", v.len())
        }
        PropertyValue::Geometry(g) => {
            // For geometry, use a compact representation with type
            // Full geometry is indexed via geohash separately
            use raisin_models::nodes::properties::GeoJson;
            let geom_type = match g {
                GeoJson::Point { .. } => "Point",
                GeoJson::LineString { .. } => "LineString",
                GeoJson::Polygon { .. } => "Polygon",
                GeoJson::MultiPoint { .. } => "MultiPoint",
                GeoJson::MultiLineString { .. } => "MultiLineString",
                GeoJson::MultiPolygon { .. } => "MultiPolygon",
                GeoJson::GeometryCollection { .. } => "GeometryCollection",
            };
            format!("geometry:{}", geom_type)
        }
        PropertyValue::Array(_) | PropertyValue::Object(_) => {
            // For complex types, use JSON serialization as hash
            serde_json::to_string(value).unwrap_or_else(|_| "invalid".to_string())
        }
    }
}
