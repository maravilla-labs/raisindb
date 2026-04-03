//! Property value coercion for LocationField
//!
//! Converts `{lat, lng}` objects to GeoJSON `PropertyValue::Geometry(Point)`
//! when the archetype schema defines the field as `LocationField`.
//!
//! This is necessary because serde's untagged deserialization cannot distinguish
//! `{lat: 35.67, lng: 139.65}` from a generic object — it always deserializes
//! as `PropertyValue::Object`. This module runs after deserialization to coerce
//! such values into `PropertyValue::Geometry(GeoJson::Point)` based on schema info.

use raisin_core::services::archetype_resolver::ArchetypeResolver;
use raisin_error::Result;
use raisin_models::nodes::properties::{GeoJson, PropertyValue};
use raisin_models::nodes::types::element::field_types::FieldSchema;
use raisin_models::nodes::Node;
use raisin_storage::Storage;
use std::collections::HashMap;

use crate::transaction::RocksDBTransaction;

/// Coerce location properties from `{lat, lng}` objects to GeoJSON geometry.
///
/// If the node has an archetype, resolves it (with inheritance) to find
/// `LocationField` definitions, then converts matching `PropertyValue::Object({lat, lng})`
/// values to `PropertyValue::Geometry(GeoJson::Point { coordinates: [lng, lat] })`.
pub(super) async fn coerce_location_fields(
    tx: &RocksDBTransaction,
    node: &mut Node,
) -> Result<()> {
    let archetype_name = match node.archetype.as_deref() {
        Some(name) if !name.is_empty() => name,
        _ => return Ok(()),
    };

    // Skip if node has no properties at all
    if node.properties.is_empty() {
        return Ok(());
    }

    let (tenant_id, repo_id, branch) = super::metadata::extract_metadata(tx)?;

    // Resolve the archetype with full inheritance to get all fields
    let resolver = ArchetypeResolver::new(
        tx.storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
    );

    // NOTE: During package install, archetypes must be committed before content nodes.
    // If the archetype is not yet available (e.g., staged in same transaction), coercion
    // is skipped — the geometry will be stored as Object and can be re-indexed later.
    let resolved = match resolver.resolve(archetype_name).await {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };

    // Find LocationField names from resolved fields (includes inherited)
    let location_field_names: Vec<String> = resolved
        .resolved_fields
        .iter()
        .filter_map(|field| match field {
            FieldSchema::LocationField { base, .. } => Some(base.name.clone()),
            _ => None,
        })
        .collect();

    if location_field_names.is_empty() {
        return Ok(());
    }

    // Coerce and validate matching properties
    for field_name in &location_field_names {
        if let Some(value) = node.properties.get(field_name) {
            match try_coerce_to_geometry(value, field_name)? {
                Some(coerced) => {
                    node.properties.insert(field_name.clone(), coerced);
                }
                None => {}
            }
        }
    }

    Ok(())
}

/// Try to coerce a PropertyValue to Geometry, validating coordinates.
///
/// Handles:
/// - `{lat, lng}` objects -> GeoJson::Point (swaps to [lng, lat] per GeoJSON spec)
/// - `{latitude, longitude}` objects -> GeoJson::Point
/// - Already a Geometry -> validates coordinates, returns None
fn try_coerce_to_geometry(
    value: &PropertyValue,
    field_name: &str,
) -> Result<Option<PropertyValue>> {
    match value {
        // Already geometry — validate it
        PropertyValue::Geometry(geo) => {
            validate_geometry_coordinates(geo, field_name)?;
            Ok(None)
        }

        PropertyValue::Object(map) => try_coerce_object_to_point(map, field_name),

        PropertyValue::Null => Ok(None),

        _ => Err(raisin_error::Error::Validation(format!(
            "LocationField '{}' must be a {{lat, lng}} object or GeoJSON geometry, got {:?}",
            field_name,
            std::mem::discriminant(value),
        ))),
    }
}

/// Validate that coordinates in a GeoJSON geometry are within valid WGS84 ranges.
fn validate_geometry_coordinates(geo: &GeoJson, field_name: &str) -> Result<()> {
    match geo {
        GeoJson::Point { coordinates } => {
            validate_coordinate(coordinates[0], coordinates[1], field_name)
        }
        GeoJson::LineString { coordinates } => {
            for coord in coordinates {
                validate_coordinate(coord[0], coord[1], field_name)?;
            }
            Ok(())
        }
        GeoJson::Polygon { coordinates } => {
            for ring in coordinates {
                for coord in ring {
                    validate_coordinate(coord[0], coord[1], field_name)?;
                }
            }
            Ok(())
        }
        GeoJson::MultiPoint { coordinates } => {
            for coord in coordinates {
                validate_coordinate(coord[0], coord[1], field_name)?;
            }
            Ok(())
        }
        _ => Ok(()), // GeometryCollection etc. — skip deep validation
    }
}

/// Validate a single [lon, lat] coordinate pair.
fn validate_coordinate(lon: f64, lat: f64, field_name: &str) -> Result<()> {
    if !(-180.0..=180.0).contains(&lon) {
        return Err(raisin_error::Error::Validation(format!(
            "LocationField '{}': longitude {} is out of range [-180, 180]",
            field_name, lon
        )));
    }
    if !(-90.0..=90.0).contains(&lat) {
        return Err(raisin_error::Error::Validation(format!(
            "LocationField '{}': latitude {} is out of range [-90, 90]",
            field_name, lat
        )));
    }
    Ok(())
}

/// Try to coerce an object map with lat/lng keys to a GeoJson Point.
fn try_coerce_object_to_point(
    map: &HashMap<String, PropertyValue>,
    field_name: &str,
) -> Result<Option<PropertyValue>> {
    // Try {lat, lng} pattern
    if let (Some(lat), Some(lng)) = (extract_f64(map.get("lat")), extract_f64(map.get("lng"))) {
        validate_coordinate(lng, lat, field_name)?;
        return Ok(Some(PropertyValue::Geometry(GeoJson::point(lng, lat))));
    }

    // Try {latitude, longitude} pattern
    if let (Some(lat), Some(lng)) = (
        extract_f64(map.get("latitude")),
        extract_f64(map.get("longitude")),
    ) {
        validate_coordinate(lng, lat, field_name)?;
        return Ok(Some(PropertyValue::Geometry(GeoJson::point(lng, lat))));
    }

    Ok(None)
}

/// Extract an f64 from a PropertyValue (Integer or Float).
fn extract_f64(value: Option<&PropertyValue>) -> Option<f64> {
    match value? {
        PropertyValue::Float(f) => Some(*f),
        PropertyValue::Integer(i) => Some(*i as f64),
        _ => None,
    }
}
