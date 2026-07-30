// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! [`Geom`]: a `geo` geometry plus the two things `geo` cannot carry — its CRS
//! and its vertical extent.

use geo::{BoundingRect, CoordsIter, Geometry};
use raisin_proj::Crs;

/// A geometry ready for `geo`'s algorithms, with the metadata `geo` drops.
///
/// # Why `z_range` and not per-vertex Z
///
/// `geo_types::Coord` is strictly two dimensional; there is no Z. So altitude is
/// **deliberately projected away** at this boundary, and every Z-aware SQL
/// function reads it back off `z_range` (or off the original JSON) rather than
/// out of `geometry`.
///
/// A parallel `Vec<Option<f64>>` aligned to `geo`'s traversal order was the
/// obvious alternative and is unmaintainable: `BooleanOps`, `Buffer` and
/// `ConvexHull` all invent new vertices, so any such vector desynchronises on
/// the first operation. An interval is the most that survives.
#[derive(Debug, Clone, PartialEq)]
pub struct Geom {
    /// The geometry, strictly 2-D, in the coordinate space of [`Self::srid`].
    pub geometry: Geometry<f64>,
    /// The coordinate reference system the coordinates are expressed in.
    pub srid: Crs,
    /// `(min, max)` altitude in metres if the source carried any, else `None`.
    pub z_range: Option<(f64, f64)>,
}

impl Geom {
    /// A geometry in the given CRS with no vertical extent.
    pub fn new(geometry: Geometry<f64>, srid: Crs) -> Self {
        Geom {
            geometry,
            srid,
            z_range: None,
        }
    }

    /// A WGS84 lon/lat geometry — the RaisinDB default CRS.
    pub fn wgs84(geometry: Geometry<f64>) -> Self {
        Geom::new(geometry, Crs::WGS84)
    }

    /// Attach a vertical extent.
    pub fn with_z_range(mut self, z_range: Option<(f64, f64)>) -> Self {
        self.z_range = z_range;
        self
    }

    /// Replace the geometry, keeping the CRS and vertical extent.
    ///
    /// The idiom for a `geo` operation that preserves coordinate space, e.g.
    /// `g.map_geometry(hull.into())`.
    pub fn map_geometry(&self, geometry: Geometry<f64>) -> Self {
        Geom {
            geometry,
            srid: self.srid,
            z_range: self.z_range,
        }
    }

    /// True when the CRS holds angular lon/lat degrees rather than linear units.
    ///
    /// This is the switch for geodesic-vs-planar measurement semantics: on a
    /// geographic CRS `ST_LENGTH` is Haversine metres, on a projected one it is
    /// native CRS units.
    pub fn is_geographic(&self) -> bool {
        self.srid.is_geographic()
    }

    /// True when the geometry has no coordinates.
    pub fn is_empty(&self) -> bool {
        self.geometry.coords_iter().next().is_none()
    }

    /// The 2-D bounding box, or `None` for an empty geometry.
    pub fn envelope(&self) -> Option<Envelope> {
        self.geometry.bounding_rect().map(|r| Envelope {
            min_x: r.min().x,
            min_y: r.min().y,
            max_x: r.max().x,
            max_y: r.max().y,
        })
    }
}

/// An axis-aligned bounding box in the geometry's own coordinate space.
///
/// Used for the *envelope* phase of a two-phase spatial query: the index yields
/// a superset of candidates by bounding box, and the original predicate is then
/// re-evaluated per row. That is why an envelope may never replace a predicate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Envelope {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Envelope {
    /// `[min_x, min_y, max_x, max_y]` — the layout the spatial index stores.
    pub fn to_array(self) -> [f64; 4] {
        [self.min_x, self.min_y, self.max_x, self.max_y]
    }

    /// True when the two boxes share any area (touching edges count).
    pub fn intersects(&self, other: &Envelope) -> bool {
        self.min_x <= other.max_x
            && self.max_x >= other.min_x
            && self.min_y <= other.max_y
            && self.max_y >= other.min_y
    }

    /// The greatest distance from the box centre to a corner, in coordinate
    /// units.
    ///
    /// This is the inflation a planner must add when it reduces a non-point
    /// query geometry to its centroid: centroid ± this radius still contains the
    /// whole geometry, so the index keeps yielding a superset.
    pub fn circumradius(&self) -> f64 {
        let dx = (self.max_x - self.min_x) / 2.0;
        let dy = (self.max_y - self.min_y) / 2.0;
        dx.hypot(dy)
    }
}

/// The one canonical spelling of an empty geometry.
///
/// Pinned in a single place because undefined emptiness was the largest source
/// of spurious "unsupported geometry" errors in the previous implementation:
/// every function accepts this and **propagates** emptiness instead of failing.
pub const EMPTY_GEOMETRY_JSON: &str = r#"{"type":"GeometryCollection","geometries":[]}"#;

/// An empty geometry as a JSON value.
pub fn empty() -> serde_json::Value {
    serde_json::json!({ "type": "GeometryCollection", "geometries": [] })
}

/// True when the geometry has no coordinates.
pub fn is_empty(g: &Geom) -> bool {
    g.is_empty()
}

/// The 2-D bounding box of a geometry, or `None` when it is empty.
pub fn envelope_of(g: &Geom) -> Option<Envelope> {
    g.envelope()
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{LineString, Point};

    #[test]
    fn empty_geometry_json_matches_the_constant() {
        assert_eq!(
            serde_json::to_string(&empty()).unwrap(),
            EMPTY_GEOMETRY_JSON
        );
    }

    #[test]
    fn emptiness_is_detected_across_nesting() {
        let g = Geom::wgs84(Geometry::GeometryCollection(Default::default()));
        assert!(is_empty(&g));
        assert_eq!(envelope_of(&g), None);

        let p = Geom::wgs84(Geometry::Point(Point::new(1.0, 2.0)));
        assert!(!is_empty(&p));
    }

    #[test]
    fn envelope_and_derived_quantities() {
        let ls = Geometry::LineString(LineString::from(vec![(0.0, 0.0), (4.0, 3.0)]));
        let e = Geom::wgs84(ls).envelope().unwrap();
        assert_eq!(e.to_array(), [0.0, 0.0, 4.0, 3.0]);
        // Half-diagonal of a 4x3 box.
        assert!((e.circumradius() - 2.5).abs() < 1e-12);

        let other = Envelope {
            min_x: 4.0,
            min_y: 3.0,
            max_x: 9.0,
            max_y: 9.0,
        };
        assert!(e.intersects(&other), "touching corners intersect");
        assert!(!e.intersects(&Envelope {
            min_x: 5.0,
            min_y: 5.0,
            max_x: 6.0,
            max_y: 6.0
        }));
    }

    #[test]
    fn map_geometry_preserves_crs_and_z() {
        let g = Geom::new(Geometry::Point(Point::new(1.0, 2.0)), Crs::from_srid(32632))
            .with_z_range(Some((10.0, 20.0)));
        let mapped = g.map_geometry(Geometry::Point(Point::new(3.0, 4.0)));
        assert_eq!(mapped.srid, Crs::from_srid(32632));
        assert_eq!(mapped.z_range, Some((10.0, 20.0)));
        assert!(!mapped.is_geographic());
    }
}
