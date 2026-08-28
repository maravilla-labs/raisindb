//! Helper utilities for node repository operations

use raisin_models::nodes::properties::PropertyValue;

/// Tombstone marker (single byte 'T' for debugging visibility)
pub(crate) const TOMBSTONE: &[u8] = b"T";

/// Check if a node value represents a tombstone (deleted node)
pub(crate) fn is_tombstone(value: &[u8]) -> bool {
    value == TOMBSTONE
}

/// Where a node inside a moved subtree lands, or `None` when it is not in that
/// subtree at all.
///
/// # Why this returns an Option
///
/// A subtree move rewrites each descendant's path by swapping the root's old
/// prefix for its new one. That is only meaningful for a node whose path
/// actually starts with the old prefix, and the walk that produces the
/// descendant list reads ORDERED_CHILDREN — a DIFFERENT index from the one that
/// holds the paths. When those two disagree (a child whose ordered-children
/// entry under its old parent was never tombstoned, e.g. because the parent
/// lookup that writes that tombstone resolved a path the index had not caught up
/// with), the walk reports a node that has long since moved elsewhere.
///
/// This used to be written as `strip_prefix(..).unwrap_or(&node.path)`, which
/// turned that disagreement into silent corruption: the node's own ABSOLUTE path
/// was appended to the new root path, producing addresses like
///
/// ```text
/// /site/moved/page//site/elsewhere/orphan
/// ```
///
/// The node still answered by id, but no parent listed it and no scan found it —
/// and the next subtree delete above it removed it for good. Returning `None`
/// lets the caller leave such a node exactly where it is and log the
/// inconsistency, which is always better than relocating it to nowhere.
pub(crate) fn moved_descendant_path(
    node_path: &str,
    old_root_path: &str,
    new_root_path: &str,
) -> Option<String> {
    let relative = node_path.strip_prefix(&format!("{}/", old_root_path))?;
    Some(format!("{}/{}", new_root_path, relative))
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
