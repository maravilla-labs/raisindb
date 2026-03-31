//! Spatial index key functions (geohash-based)
//!
//! Keys for geospatial indexes enabling PostGIS-compatible ST_* queries.

use super::KeyBuilder;
use raisin_hlc::HLC;

/// Spatial index key (revision-aware with geohash)
pub fn spatial_index_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property_name: &str,
    geohash: &str,
    revision: &HLC,
    node_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("geo")
        .push(property_name)
        .push(geohash)
        .push_revision(revision)
        .push(node_id)
        .build()
}

/// Spatial index prefix for property
pub fn spatial_index_property_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property_name: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("geo")
        .push(property_name)
        .build_prefix()
}

/// Spatial index prefix for geohash
pub fn spatial_index_geohash_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property_name: &str,
    geohash_prefix: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("geo")
        .push(property_name)
        .push(geohash_prefix)
        .build_prefix()
}

/// Spatial index workspace prefix
pub fn spatial_index_workspace_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("geo")
        .build_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spatial_index_key_encoding() {
        let hlc = HLC::new(1705843009213693952, 42);
        let key = spatial_index_key_versioned(
            "tenant1",
            "repo1",
            "main",
            "workspace1",
            "location",
            "9q8yyk",
            &hlc,
            "node123",
        );
        assert!(!key.is_empty());
        let key_str = String::from_utf8_lossy(&key);
        assert!(key_str.contains("geo"));
        assert!(key_str.contains("location"));
        assert!(key_str.contains("9q8yyk"));
    }

    #[test]
    fn test_spatial_index_geohash_prefix_ordering() {
        let prefix_short =
            spatial_index_geohash_prefix("t1", "r1", "main", "ws1", "location", "9q8");
        let prefix_long =
            spatial_index_geohash_prefix("t1", "r1", "main", "ws1", "location", "9q8yyk");
        assert!(prefix_long.starts_with(&prefix_short[..prefix_short.len() - 1]));
    }

    #[test]
    fn test_spatial_index_property_isolation() {
        let loc_prefix = spatial_index_property_prefix("t1", "r1", "main", "ws1", "location");
        let geo_prefix = spatial_index_property_prefix("t1", "r1", "main", "ws1", "geometry");
        assert_ne!(loc_prefix, geo_prefix);
    }
}
