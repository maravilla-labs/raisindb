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

//! `raisin_models::GeoJson` <-> `geo::Geometry<f64>`, without going through serde.
//!
//! The storage write path converts one geometry per node revision, so this
//! direction is deliberately serde-free: no `serde_json::Value` is built, no
//! string keys are hashed, and the only allocations are the coordinate vectors
//! `geo` itself requires.
//!
//! The `serde_json` direction lives in [`crate::json`] and delegates to the
//! `geojson` crate instead, which is the right trade for the SQL evaluator where
//! the input is already a `Value`.

use geo::{
    Coord, Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
    Point, Polygon,
};
use raisin_models::nodes::properties::{GeoJson, Position};
use raisin_proj::Crs;

use crate::error::{GeometryError, Result};
use crate::geom::Geom;

/// Convert a stored geometry into a `geo` geometry.
///
/// `schema_default` supplies the SRID for a value that carries no `srid` member,
/// so a workspace working entirely in a projected CRS need not repeat it on every
/// write. Precedence is: explicit member > `schema_default` > EPSG:4326.
///
/// # Errors
///
/// [`GeometryError::NonFiniteCoordinate`] if any ordinate is NaN or infinite.
/// Rejecting here rather than downstream matters: a non-finite ordinate produces
/// plausible-looking garbage from every `geo` algorithm and would geohash to a
/// nonsense index cell.
pub fn to_geo_from_model(g: &GeoJson, schema_default: Option<u32>) -> Result<Geom> {
    let srid = g
        .srid()
        .or(schema_default)
        .map(Crs::from)
        .unwrap_or(Crs::WGS84);
    Ok(Geom {
        geometry: geometry_of(g)?,
        srid,
        z_range: g.z_range(),
    })
}

/// Convert a `geo` geometry back into the stored model.
///
/// Altitude is **not** restored: `geo` never carried it. Callers holding a
/// [`Geom`] with a `z_range` that must survive should re-apply it themselves.
///
/// The `srid` member is emitted only when the CRS is not WGS84, which keeps all
/// 4326 output strictly RFC-7946-conformant — the thing that matters for interop
/// with mapping libraries.
pub fn to_model(g: &Geom) -> Result<GeoJson> {
    let srid = (g.srid != Crs::WGS84).then(|| g.srid.srid());
    Ok(model_of(&g.geometry)?.with_srid(srid))
}

// --- model -> geo -------------------------------------------------------------

fn coord(p: &Position) -> Result<Coord<f64>> {
    if !p.x.is_finite() || !p.y.is_finite() {
        return Err(GeometryError::NonFiniteCoordinate { x: p.x, y: p.y });
    }
    Ok(Coord { x: p.x, y: p.y })
}

fn line(ps: &[Position]) -> Result<LineString<f64>> {
    let mut out = Vec::with_capacity(ps.len());
    for p in ps {
        out.push(coord(p)?);
    }
    Ok(LineString::new(out))
}

fn polygon(rings: &[Vec<Position>]) -> Result<Polygon<f64>> {
    let mut it = rings.iter();
    let exterior = match it.next() {
        Some(r) => line(r)?,
        // An empty ring list is a legitimately empty polygon, not an error.
        None => LineString::new(Vec::new()),
    };
    let mut interiors = Vec::with_capacity(rings.len().saturating_sub(1));
    for r in it {
        interiors.push(line(r)?);
    }
    Ok(Polygon::new(exterior, interiors))
}

fn geometry_of(g: &GeoJson) -> Result<Geometry<f64>> {
    Ok(match g {
        GeoJson::Point { coordinates, .. } => Geometry::Point(Point::from(coord(coordinates)?)),
        GeoJson::LineString { coordinates, .. } => Geometry::LineString(line(coordinates)?),
        GeoJson::Polygon { coordinates, .. } => Geometry::Polygon(polygon(coordinates)?),
        GeoJson::MultiPoint { coordinates, .. } => {
            let mut pts = Vec::with_capacity(coordinates.len());
            for p in coordinates {
                pts.push(Point::from(coord(p)?));
            }
            Geometry::MultiPoint(MultiPoint::new(pts))
        }
        GeoJson::MultiLineString { coordinates, .. } => {
            let mut ls = Vec::with_capacity(coordinates.len());
            for l in coordinates {
                ls.push(line(l)?);
            }
            Geometry::MultiLineString(MultiLineString::new(ls))
        }
        GeoJson::MultiPolygon { coordinates, .. } => {
            let mut ps = Vec::with_capacity(coordinates.len());
            for p in coordinates {
                ps.push(polygon(p)?);
            }
            Geometry::MultiPolygon(MultiPolygon::new(ps))
        }
        GeoJson::GeometryCollection { geometries, .. } => {
            let mut gs = Vec::with_capacity(geometries.len());
            for g in geometries {
                gs.push(geometry_of(g)?);
            }
            Geometry::GeometryCollection(GeometryCollection::new_from(gs))
        }
    })
}

// --- geo -> model -------------------------------------------------------------

fn pos(c: Coord<f64>) -> Position {
    Position::new_2d(c.x, c.y)
}

fn positions(ls: &LineString<f64>) -> Vec<Position> {
    ls.coords().copied().map(pos).collect()
}

fn rings(p: &Polygon<f64>) -> Vec<Vec<Position>> {
    let mut out = Vec::with_capacity(1 + p.interiors().len());
    out.push(positions(p.exterior()));
    out.extend(p.interiors().iter().map(positions));
    out
}

/// `geo::Geometry` has four variants GeoJSON does not: `Line`, `Rect`,
/// `Triangle` and (as a distinct thing) `Point` vs `MultiPoint`. The first three
/// are produced by `geo`'s own algorithms — `BoundingRect` returns a `Rect`,
/// triangulation returns `Triangle`s — so they must be widened here rather than
/// rejected, or `ST_ENVELOPE` would have no representable result.
fn model_of(g: &Geometry<f64>) -> Result<GeoJson> {
    Ok(match g {
        Geometry::Point(p) => GeoJson::Point {
            coordinates: pos(p.0),
            srid: None,
        },
        Geometry::LineString(ls) => GeoJson::LineString {
            coordinates: positions(ls),
            srid: None,
        },
        Geometry::Polygon(p) => GeoJson::Polygon {
            coordinates: rings(p),
            srid: None,
        },
        Geometry::MultiPoint(mp) => GeoJson::MultiPoint {
            coordinates: mp.iter().map(|p| pos(p.0)).collect(),
            srid: None,
        },
        Geometry::MultiLineString(mls) => GeoJson::MultiLineString {
            coordinates: mls.iter().map(positions).collect(),
            srid: None,
        },
        Geometry::MultiPolygon(mp) => GeoJson::MultiPolygon {
            coordinates: mp.iter().map(rings).collect(),
            srid: None,
        },
        Geometry::GeometryCollection(gc) => GeoJson::GeometryCollection {
            geometries: gc.iter().map(model_of).collect::<Result<Vec<_>>>()?,
            srid: None,
        },
        // Widened, not rejected — see the doc comment above.
        Geometry::Line(l) => GeoJson::LineString {
            coordinates: vec![pos(l.start), pos(l.end)],
            srid: None,
        },
        Geometry::Rect(r) => GeoJson::Polygon {
            coordinates: vec![positions(r.to_polygon().exterior())],
            srid: None,
        },
        Geometry::Triangle(t) => GeoJson::Polygon {
            coordinates: vec![positions(t.to_polygon().exterior())],
            srid: None,
        },
    })
}

#[cfg(test)]
mod tests;
