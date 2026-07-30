//! Spatial index key functions (geohash-based)
//!
//! Keys for geospatial indexes enabling PostGIS-compatible ST_* queries.

use super::KeyBuilder;
use raisin_hlc::HLC;

/// Number of leading `\0`-separated string components before the geohash.
///
/// `{tenant}\0{repo}\0{branch}\0{workspace}\0geo\0{property}` — six parts.
const LEADING_PARTS: usize = 6;

/// Byte width of the descending HLC encoding embedded in a spatial index key.
const REVISION_WIDTH: usize = 16;

/// The parsed components of a spatial index key.
///
/// Borrowed from the key bytes; no allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSpatialKey<'a> {
    pub tenant_id: &'a str,
    pub repo_id: &'a str,
    pub branch: &'a str,
    pub workspace: &'a str,
    pub property_name: &'a str,
    /// The geohash cell label. Its length IS the index precision.
    pub geohash: &'a str,
    /// The revision this entry was written at.
    pub revision: HLC,
    pub node_id: &'a str,
}

impl ParsedSpatialKey<'_> {
    /// The geohash precision this entry was written at.
    pub fn precision(&self) -> usize {
        self.geohash.chars().count()
    }
}

/// Parse a spatial index key produced by [`spatial_index_key_versioned`].
///
/// # Why this exists
///
/// The descending HLC encoding contains **null bytes** whenever the timestamp or
/// counter has a `0xFF` complement byte (notably `counter == 0`, which yields
/// eight `0xFF`s — and `!u64::MAX == 0`, which yields eight zero bytes). So a
/// naive `key_str.split('\0')` produces a variable number of fragments and
/// `parts[6]` is NOT reliably the geohash. That is the same class of bug
/// `ordering::key_parse::parse_ordered_child_key` exists to prevent for
/// `ORDERED_CHILDREN`.
///
/// This parser walks the six fixed leading components, takes the geohash up to
/// the next separator, then consumes the revision as a **fixed 16-byte field**
/// so embedded nulls cannot confuse it.
///
/// Returns `None` if the key is malformed or not valid UTF-8 in its text parts.
pub fn parse_spatial_index_key(key: &[u8]) -> Option<ParsedSpatialKey<'_>> {
    let mut parts: Vec<&str> = Vec::with_capacity(LEADING_PARTS + 1);
    let mut cursor = 0usize;

    // Six leading text components plus the geohash = seven `\0`-terminated
    // strings, all guaranteed null-free by construction.
    for _ in 0..=LEADING_PARTS {
        let rel = key[cursor..].iter().position(|&b| b == 0)?;
        parts.push(std::str::from_utf8(&key[cursor..cursor + rel]).ok()?);
        cursor += rel + 1;
    }

    if parts[4] != "geo" {
        return None;
    }

    if key.len() < cursor + REVISION_WIDTH + 1 {
        return None;
    }
    let revision = HLC::decode_descending(&key[cursor..cursor + REVISION_WIDTH]).ok()?;
    cursor += REVISION_WIDTH;

    // Separator between revision and node id.
    if key[cursor] != 0 {
        return None;
    }
    cursor += 1;

    let node_id = std::str::from_utf8(&key[cursor..]).ok()?;

    Some(ParsedSpatialKey {
        tenant_id: parts[0],
        repo_id: parts[1],
        branch: parts[2],
        workspace: parts[3],
        property_name: parts[5],
        geohash: parts[6],
        revision,
        node_id,
    })
}

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
    fn test_parse_spatial_index_key_roundtrip() {
        let hlc = HLC::new(1705843009213693952, 42);
        let key = spatial_index_key_versioned(
            "tenant1", "repo1", "main", "ws1", "location", "9q8yyk", &hlc, "node123",
        );
        let parsed = parse_spatial_index_key(&key).expect("key must parse");
        assert_eq!(parsed.tenant_id, "tenant1");
        assert_eq!(parsed.repo_id, "repo1");
        assert_eq!(parsed.branch, "main");
        assert_eq!(parsed.workspace, "ws1");
        assert_eq!(parsed.property_name, "location");
        assert_eq!(parsed.geohash, "9q8yyk");
        assert_eq!(parsed.revision, hlc);
        assert_eq!(parsed.node_id, "node123");
        assert_eq!(parsed.precision(), 6);
    }

    /// The regression this parser exists for: `counter == 0` makes
    /// `!counter == u64::MAX`, but `timestamp_ms == u64::MAX` (or any byte-wise
    /// `0xFF` complement) puts NULL bytes inside the fixed-width revision
    /// field. A `split('\0')` parser then mis-identifies the geohash.
    #[test]
    fn test_parse_survives_null_bytes_in_revision() {
        // !timestamp_ms == 0 -> eight 0x00 bytes inside the revision.
        let hlc = HLC::new(u64::MAX, 0);
        let encoded = hlc.encode_descending();
        assert!(
            encoded.iter().any(|&b| b == 0),
            "precondition: has null bytes"
        );

        let key = spatial_index_key_versioned("t", "r", "main", "ws", "loc", "u0nc", &hlc, "n1");

        // The naive parser is wrong...
        let naive: Vec<&[u8]> = key.split(|&b| b == 0).collect();
        assert!(naive.len() > 9, "naive split over-fragments");

        // ...the real one is not.
        let parsed = parse_spatial_index_key(&key).expect("key must parse");
        assert_eq!(parsed.geohash, "u0nc");
        assert_eq!(parsed.node_id, "n1");
        assert_eq!(parsed.revision, hlc);
    }

    #[test]
    fn test_parse_rejects_foreign_key() {
        assert!(parse_spatial_index_key(b"not\0a\0spatial\0key").is_none());
    }

    #[test]
    fn test_spatial_index_property_isolation() {
        let loc_prefix = spatial_index_property_prefix("t1", "r1", "main", "ws1", "location");
        let geo_prefix = spatial_index_property_prefix("t1", "r1", "main", "ws1", "geometry");
        assert_ne!(loc_prefix, geo_prefix);
    }
}
