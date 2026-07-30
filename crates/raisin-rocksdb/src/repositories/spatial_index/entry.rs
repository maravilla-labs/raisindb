//! The value record stored against each spatial index key.
//!
//! # Why this replaced `format!("{},{}", lon, lat)`
//!
//! The old value was an ad-hoc comma string parsed back by splitting on `,`.
//! Two problems. First, `format!("{}", f64)` is not byte-stable across values
//! that compare equal, so re-indexing a node at the same revision could produce
//! a *different* value and defeat the reindex job's idempotency. Second, it
//! carried nothing but the centroid, so a query had to fetch the full node
//! record for every candidate before it could reject one — the dominant cost of
//! a selective spatial scan.
//!
//! [`SpatialEntry`] carries enough to reject a candidate without any node fetch:
//! the bounding box, the altitude extent, and an optional discriminator bucket
//! (typically a floor/level label). It also carries the `policy_hash` that
//! produced it, which is how `VERIFY SPATIAL INDEX` detects cross-node skew.
//!
//! # On-disk compatibility
//!
//! Keys are unchanged. Values gained a version byte. [`SpatialEntry::decode`]
//! still accepts the legacy comma form as `v = 0` (centroid only), so an
//! upgraded server keeps answering queries against a not-yet-reindexed index
//! instead of silently returning nothing — which is exactly the failure mode this
//! whole pass exists to remove.

use raisin_models::nodes::properties::GeoJson;
use serde::{Deserialize, Serialize};

/// Current value format version.
pub const SPATIAL_ENTRY_VERSION: u8 = 1;

/// Marker byte for a legacy (pre-versioning) comma-encoded centroid value.
pub const SPATIAL_ENTRY_LEGACY_VERSION: u8 = 0;

/// First byte of a MessagePack 9-element fixarray — the encoding
/// `rmp_serde::to_vec` produces for [`SpatialEntry`]. Used to distinguish a v1
/// record from the legacy ASCII form (which always begins with a digit, `-`, or
/// `.`) and from the one-byte tombstone `b"T"`.
const MSGPACK_FIXARRAY_9: u8 = 0x99;

/// Geometry-type discriminant stored in the index, so a scan can answer
/// `ST_GEOMETRYTYPE`-shaped filters and pick a cheap path for points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SpatialGeometryKind {
    Point = 0,
    LineString = 1,
    Polygon = 2,
    MultiPoint = 3,
    MultiLineString = 4,
    MultiPolygon = 5,
    GeometryCollection = 6,
}

impl SpatialGeometryKind {
    /// Classify a stored geometry.
    pub fn of(geometry: &GeoJson) -> Self {
        match geometry {
            GeoJson::Point { .. } => Self::Point,
            GeoJson::LineString { .. } => Self::LineString,
            GeoJson::Polygon { .. } => Self::Polygon,
            GeoJson::MultiPoint { .. } => Self::MultiPoint,
            GeoJson::MultiLineString { .. } => Self::MultiLineString,
            GeoJson::MultiPolygon { .. } => Self::MultiPolygon,
            GeoJson::GeometryCollection { .. } => Self::GeometryCollection,
        }
    }

    /// Whether this kind is a single point (the cheap case: centroid == geometry).
    pub fn is_point(self) -> bool {
        matches!(self, Self::Point)
    }
}

/// The value stored against a spatial index key.
///
/// Encoded with `rmp_serde::to_vec` (compact array form). Every field is
/// fixed-width or a length-prefixed string, and f64s go out as IEEE-754 bits, so
/// **the encoding is byte-stable**: re-indexing an unchanged geometry at the same
/// revision reproduces identical bytes and the write is a no-op rather than a new
/// MVCC generation. The reindex job's idempotency depends on that.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpatialEntry {
    /// Format version. See [`SPATIAL_ENTRY_VERSION`].
    pub v: u8,
    /// WGS84 centroid longitude.
    pub lon: f64,
    /// WGS84 centroid latitude.
    pub lat: f64,
    /// WGS84 bounding box: `[min_lon, min_lat, max_lon, max_lat]`.
    pub bbox: [f64; 4],
    /// Altitude extent in metres, when the geometry carries any Z ordinate.
    pub z: Option<(f64, f64)>,
    /// The SRID the geometry was **stored** in. Index keys are always 4326;
    /// this preserves what the user wrote so `SELECT` can return it verbatim.
    pub srid: u32,
    /// Geometry-type discriminant.
    pub gtype: SpatialGeometryKind,
    /// The configured discriminator value for this node, e.g. a floor label
    /// `"L2"`. Lets a floor-filtered proximity query reject candidates without a
    /// node fetch. `None` when no `bucket_property` is configured or the node
    /// does not carry it.
    pub bucket: Option<String>,
    /// The [`raisin_models::nodes::properties::SpatialPolicy::policy_hash`] that
    /// produced this entry. A mismatch against the live policy is what schedules
    /// a reindex, and what makes cross-node skew detectable.
    pub policy_hash: u64,
}

impl SpatialEntry {
    /// Encode to the on-disk value form.
    pub fn encode(&self) -> Vec<u8> {
        // Infallible in practice for this concrete shape; fall back to the
        // legacy centroid form rather than dropping the entry, because an
        // unindexed geometry is a silent wrong answer while a v0 entry is merely
        // less selective.
        rmp_serde::to_vec(self).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "SpatialEntry encode failed; writing legacy centroid form");
            format!("{},{}", self.lon, self.lat).into_bytes()
        })
    }

    /// Decode an on-disk value, accepting both the current form and the legacy
    /// `"{lon},{lat}"` string.
    ///
    /// Returns `None` for a tombstone or an unparseable value. Callers must
    /// already have excluded tombstones by their own check; this is belt and
    /// braces.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() {
            return None;
        }

        if bytes[0] == MSGPACK_FIXARRAY_9 {
            match rmp_serde::from_slice::<Self>(bytes) {
                Ok(entry) => return Some(entry),
                Err(e) => {
                    tracing::warn!(error = %e, "Malformed SpatialEntry value; skipping");
                    return None;
                }
            }
        }

        Self::decode_legacy(bytes)
    }

    /// Parse the pre-versioning `"{lon},{lat}"` value.
    ///
    /// Everything the newer format adds is absent, so the entry degrades
    /// gracefully: no bbox pre-filter (the bbox collapses to the centroid), no
    /// bucket pre-filter, unknown SRID (assumed 4326, which every legacy entry
    /// was), and `policy_hash == 0` so `VERIFY` reports it as needing a rebuild.
    fn decode_legacy(bytes: &[u8]) -> Option<Self> {
        let text = std::str::from_utf8(bytes).ok()?;
        let (lon_s, lat_s) = text.split_once(',')?;
        let lon: f64 = lon_s.trim().parse().ok()?;
        let lat: f64 = lat_s.trim().parse().ok()?;
        if !lon.is_finite() || !lat.is_finite() {
            return None;
        }
        Some(Self {
            v: SPATIAL_ENTRY_LEGACY_VERSION,
            lon,
            lat,
            bbox: [lon, lat, lon, lat],
            z: None,
            srid: 4326,
            gtype: SpatialGeometryKind::Point,
            bucket: None,
            policy_hash: 0,
        })
    }

    /// Whether this entry was read from a legacy value and therefore carries no
    /// bbox / bucket selectivity.
    pub fn is_legacy(&self) -> bool {
        self.v == SPATIAL_ENTRY_LEGACY_VERSION
    }

    /// Reject on the configured bucket discriminator.
    ///
    /// A legacy entry has no bucket, so it can never be rejected — correct, since
    /// rejecting it would drop a real match.
    pub fn matches_bucket(&self, wanted: Option<&str>) -> bool {
        match wanted {
            None => true,
            Some(w) => {
                if self.is_legacy() {
                    return true;
                }
                self.bucket.as_deref() == Some(w)
            }
        }
    }
}
