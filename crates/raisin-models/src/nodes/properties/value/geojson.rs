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

//! GeoJSON geometry types (RFC 7946, plus two documented extensions).
//!
//! # The two extensions to RFC 7946
//!
//! 1. **Altitude.** Coordinates are [`Position`], which carries an optional third
//!    ordinate. RFC 7946 §3.1.1 explicitly allows this.
//! 2. **`srid`.** An optional `srid` member names the geometry's coordinate
//!    reference system, EWKT/EWKB style. RFC 7946 mandates WGS84 and forbids
//!    other CRSs, so this *is* an extension — and it is elided whenever the CRS
//!    is WGS84, which keeps all 4326 output strictly RFC-7946-conformant. That is
//!    what matters for interop with mapping libraries.
//!
//! # Why `srid` lives in the value and not beside it
//!
//! The SRID must travel *with* the geometry through function composition:
//! `ST_TRANSFORM(ST_UNION(a, b), 3857)` cannot work if the CRS lives on the node
//! rather than in the value. A sibling `location_srid` property is invisible to
//! `ST_SRID(<expression>)`.
//!
//! # Serde compatibility (do not break this)
//!
//! `srid` is `#[serde(default, skip_serializing_if = "Option::is_none")]` and is
//! declared **last** in every variant, and [`Position`] serializes a 2-D value as
//! a 2-element array. Together those mean every geometry that existed before
//! this extension serializes to **byte-identical** JSON and MessagePack: stored
//! node blobs are unchanged, `hash_property_value` is unchanged (so no
//! property-index churn), and CRDT convergence sees nothing new.
//!
//! It also survives the `#[serde(untagged)]` inference in
//! [`PropertyValue`](super::PropertyValue): `GeoJson` is `#[serde(tag = "type")]`,
//! an internally tagged enum discriminates on the *value* of `type` and ignores
//! unknown keys, so a value carrying `srid` still infers as `Geometry` rather
//! than falling through to `Object`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::position::Position;

/// GeoJSON geometry types.
///
/// Coordinates are `[longitude, latitude]` per GeoJSON spec (**not** lat/lon),
/// optionally with a third altitude ordinate. The CRS is WGS84 (EPSG:4326)
/// unless an explicit [`srid`](GeoJson::srid) says otherwise.
///
/// # Examples
///
/// ```json
/// {"type": "Point", "coordinates": [-122.4194, 37.7749]}
/// {"type": "Point", "coordinates": [-122.4194, 37.7749, 16.0]}
/// {"type": "Point", "coordinates": [2683000.0, 1247000.0], "srid": 2056}
/// ```
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(tag = "type")]
pub enum GeoJson {
    /// A single point: `[longitude, latitude]`
    Point {
        coordinates: Position,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        srid: Option<u32>,
    },

    /// A line of connected points
    LineString {
        coordinates: Vec<Position>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        srid: Option<u32>,
    },

    /// A closed polygon (first ring is exterior, rest are holes).
    /// Each ring is a list of `[lon, lat]` coordinates where first == last.
    Polygon {
        coordinates: Vec<Vec<Position>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        srid: Option<u32>,
    },

    /// Multiple points
    MultiPoint {
        coordinates: Vec<Position>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        srid: Option<u32>,
    },

    /// Multiple line strings
    MultiLineString {
        coordinates: Vec<Vec<Position>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        srid: Option<u32>,
    },

    /// Multiple polygons
    MultiPolygon {
        coordinates: Vec<Vec<Vec<Position>>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        srid: Option<u32>,
    },

    /// A collection of any geometry types
    GeometryCollection {
        geometries: Vec<GeoJson>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        srid: Option<u32>,
    },
}

impl GeoJson {
    /// Create a 2-D Point from longitude and latitude.
    pub fn point(lon: f64, lat: f64) -> Self {
        GeoJson::Point {
            coordinates: Position::new_2d(lon, lat),
            srid: None,
        }
    }

    /// Create a 3-D Point from longitude, latitude and altitude in metres.
    pub fn point_3d(lon: f64, lat: f64, alt: f64) -> Self {
        GeoJson::Point {
            coordinates: Position::new_3d(lon, lat, alt),
            srid: None,
        }
    }

    /// An empty geometry, canonically represented as an empty
    /// `GeometryCollection`. Pinned here so every layer agrees on one spelling.
    pub fn empty() -> Self {
        GeoJson::GeometryCollection {
            geometries: Vec::new(),
            srid: None,
        }
    }

    /// The RFC 7946 type name, e.g. `"MultiPolygon"`.
    pub fn geometry_type(&self) -> &'static str {
        match self {
            GeoJson::Point { .. } => "Point",
            GeoJson::LineString { .. } => "LineString",
            GeoJson::Polygon { .. } => "Polygon",
            GeoJson::MultiPoint { .. } => "MultiPoint",
            GeoJson::MultiLineString { .. } => "MultiLineString",
            GeoJson::MultiPolygon { .. } => "MultiPolygon",
            GeoJson::GeometryCollection { .. } => "GeometryCollection",
        }
    }

    /// The declared SRID, or `None` when the geometry is "unlabelled".
    ///
    /// `None` is deliberately distinct from `Some(4326)`: an unlabelled geometry
    /// *adopts* the other operand's SRID in a binary operation, which is what
    /// keeps every pre-existing query and dataset working unchanged.
    pub fn srid(&self) -> Option<u32> {
        match self {
            GeoJson::Point { srid, .. }
            | GeoJson::LineString { srid, .. }
            | GeoJson::Polygon { srid, .. }
            | GeoJson::MultiPoint { srid, .. }
            | GeoJson::MultiLineString { srid, .. }
            | GeoJson::MultiPolygon { srid, .. }
            | GeoJson::GeometryCollection { srid, .. } => *srid,
        }
    }

    /// Replace the SRID label. Purely metadata — coordinates are untouched, so
    /// this is `ST_SETSRID`, never `ST_TRANSFORM`.
    pub fn set_srid(&mut self, new_srid: Option<u32>) {
        let slot = match self {
            GeoJson::Point { srid, .. }
            | GeoJson::LineString { srid, .. }
            | GeoJson::Polygon { srid, .. }
            | GeoJson::MultiPoint { srid, .. }
            | GeoJson::MultiLineString { srid, .. }
            | GeoJson::MultiPolygon { srid, .. }
            | GeoJson::GeometryCollection { srid, .. } => srid,
        };
        *slot = new_srid;
    }

    /// Builder form of [`GeoJson::set_srid`].
    pub fn with_srid(mut self, new_srid: Option<u32>) -> Self {
        self.set_srid(new_srid);
        self
    }

    /// Check if this is a Point geometry.
    pub fn is_point(&self) -> bool {
        matches!(self, GeoJson::Point { .. })
    }

    /// Get the position if this is a Point (altitude preserved).
    pub fn as_point(&self) -> Option<Position> {
        match self {
            GeoJson::Point { coordinates, .. } => Some(*coordinates),
            _ => None,
        }
    }

    /// True when the geometry carries no coordinates at all.
    pub fn is_empty(&self) -> bool {
        match self {
            GeoJson::Point { .. } => false,
            GeoJson::LineString { coordinates, .. } | GeoJson::MultiPoint { coordinates, .. } => {
                coordinates.is_empty()
            }
            GeoJson::Polygon { coordinates, .. } | GeoJson::MultiLineString { coordinates, .. } => {
                coordinates.iter().all(|r| r.is_empty())
            }
            GeoJson::MultiPolygon { coordinates, .. } => {
                coordinates.iter().all(|p| p.iter().all(|r| r.is_empty()))
            }
            GeoJson::GeometryCollection { geometries, .. } => {
                geometries.iter().all(GeoJson::is_empty)
            }
        }
    }

    /// Run `f` over every position in the geometry, in document order.
    ///
    /// The one traversal primitive; [`Self::z_range`] and any future
    /// coordinate-wide predicate should use it rather than re-matching all seven
    /// variants.
    pub fn for_each_position(&self, f: &mut impl FnMut(&Position)) {
        match self {
            GeoJson::Point { coordinates, .. } => f(coordinates),
            GeoJson::LineString { coordinates, .. } | GeoJson::MultiPoint { coordinates, .. } => {
                coordinates.iter().for_each(f)
            }
            GeoJson::Polygon { coordinates, .. } | GeoJson::MultiLineString { coordinates, .. } => {
                coordinates.iter().flatten().for_each(f)
            }
            GeoJson::MultiPolygon { coordinates, .. } => {
                coordinates.iter().flatten().flatten().for_each(f)
            }
            GeoJson::GeometryCollection { geometries, .. } => {
                for g in geometries {
                    g.for_each_position(f);
                }
            }
        }
    }

    /// The `(min, max)` altitude across every position that carries one, or
    /// `None` when the geometry is entirely two-dimensional.
    ///
    /// This is the whole 3-D extent RaisinDB keeps: `geo-types`' `Coord` has no
    /// Z, so altitude is deliberately projected away at the boundary into the
    /// `geo` pipeline and re-read from here. A per-vertex Z vector could not
    /// survive `BooleanOps`, `Buffer` or `ConvexHull`, all of which invent new
    /// vertices.
    pub fn z_range(&self) -> Option<(f64, f64)> {
        let mut range: Option<(f64, f64)> = None;
        self.for_each_position(&mut |p| {
            if let Some(z) = p.z {
                range = Some(match range {
                    None => (z, z),
                    Some((lo, hi)) => (lo.min(z), hi.max(z)),
                });
            }
        });
        range
    }

    /// True when any position carries an altitude.
    pub fn is_3d(&self) -> bool {
        self.z_range().is_some()
    }

    /// Get the centroid for indexing and cheap distance approximation.
    ///
    /// Unlike the old implementation this covers **every** geometry type rather
    /// than returning `None` for `Multi*` and `GeometryCollection` — a `None`
    /// there meant "not indexable", which is how large geometries became
    /// invisible to the spatial index.
    ///
    /// This is the arithmetic mean of the relevant vertices, *not* the
    /// area-weighted centre of mass; `ST_CENTROID` uses `geo` for that. The
    /// altitude is preserved only for a Point.
    pub fn centroid(&self) -> Option<Position> {
        match self {
            GeoJson::Point { coordinates, .. } => Some(*coordinates),
            GeoJson::Polygon { coordinates, .. } => mean(coordinates.first()?.iter()),
            GeoJson::MultiPolygon { coordinates, .. } => {
                mean(coordinates.iter().filter_map(|p| p.first()).flatten())
            }
            GeoJson::GeometryCollection { geometries, .. } => {
                let parts: Vec<Position> =
                    geometries.iter().filter_map(GeoJson::centroid).collect();
                mean(parts.iter())
            }
            GeoJson::LineString { coordinates, .. } | GeoJson::MultiPoint { coordinates, .. } => {
                mean(coordinates.iter())
            }
            GeoJson::MultiLineString { coordinates, .. } => mean(coordinates.iter().flatten()),
        }
    }
}

/// Arithmetic mean of the x/y ordinates; `None` for an empty iterator. Altitude
/// is dropped, because averaging altitudes across a ring is not meaningful.
fn mean<'a>(positions: impl Iterator<Item = &'a Position>) -> Option<Position> {
    let mut n = 0u64;
    let (mut sx, mut sy) = (0.0f64, 0.0f64);
    for p in positions {
        sx += p.x;
        sy += p.y;
        n += 1;
    }
    if n == 0 {
        return None;
    }
    let n = n as f64;
    Some(Position::new_2d(sx / n, sy / n))
}
