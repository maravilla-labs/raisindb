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

//! Write-time SRID normalisation: every indexed geometry is geohashed in WGS84.
//!
//! # The bug this closes
//!
//! Geohash cells are defined on **lon/lat degrees**. A geometry stored in a
//! projected CRS carries eastings and northings — EPSG:3857 coordinates over
//! Zurich are around `(950_668, 6_002_678)`. Handing those straight to the
//! geohash encoder does not produce a wrong-ish cell, it produces *no* cell at
//! all (they fail the `-180..=180 / -90..=90` domain check), so the geometry is
//! silently never indexed and `ST_DWITHIN` misses it forever with no signal.
//! Before this module existed, [`raisin_proj::normalize_for_index`] had zero
//! callers.
//!
//! # What is normalised, and what is not
//!
//! Only the **index cells** and the WGS84 centroid/bbox in the index value.
//! The stored geometry keeps its original coordinates and its original SRID —
//! this is a read of the geometry, never a rewrite of it. `SpatialEntry.srid`
//! still records the CRS the value is stored in.
//!
//! # Why only the built-in projection tier
//!
//! [`raisin_proj::normalize_for_index`] deliberately refuses to consult
//! `proj4rs-backend` or `proj-backend` even when they are compiled in. Index
//! bytes are replicated state in a masterless cluster: if normalisation used
//! "whatever backend happens to be linked", a node built with `proj-backend`
//! would index an EPSG:31370 geometry and a node without it would not, and the
//! same query would answer differently depending on which node replied. So the
//! accepted set is feature-independent by construction — EPSG:4326, EPSG:3857
//! (with its 3785/900913 aliases) and all 120 WGS84 UTM zones — and everything
//! else is rejected **on every build**. See [`normalize_geometry_for_index`].

use std::borrow::Cow;

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::{GeoJson, Position};
use raisin_proj::{Crs, ProjError};

/// The geometry to derive index cells from: WGS84 lon/lat, whatever the stored
/// CRS was.
///
/// Returns [`Cow::Borrowed`] for the overwhelmingly common unlabelled/EPSG:4326
/// case, so the normal write path pays no allocation at all. A projected CRS
/// yields an owned copy whose positions have been reprojected and whose SRID
/// label has been dropped (it is WGS84 now); altitudes ride along untouched,
/// because reprojecting the horizontal ordinates does not change a height in
/// metres.
///
/// # Errors
///
/// [`Error::Validation`] when the geometry's SRID is outside the built-in
/// projection tier. This is the loud failure that replaced silent success: a
/// stored-but-unindexable geometry is invisible to every spatial query, which is
/// worse than a rejected write. The message names the SRID, the CRS that *are*
/// accepted, and why enabling a wider Cargo feature deliberately does not help.
pub fn normalize_geometry_for_index(geometry: &GeoJson) -> Result<Cow<'_, GeoJson>> {
    let Some(srid) = geometry.srid() else {
        // Unlabelled means WGS84 by convention, everywhere in RaisinDB.
        return Ok(Cow::Borrowed(geometry));
    };
    let crs = Crs::from(srid);
    if crs == Crs::WGS84 {
        return Ok(Cow::Borrowed(geometry));
    }

    if !raisin_proj::is_index_normalizable(crs) {
        return Err(unsupported_srid(srid));
    }

    let mut failure: Option<ProjError> = None;
    let mut normalized = map_positions(geometry, &mut |p| {
        // A position the built-in tier cannot place (a pole in WebMercator, a
        // NaN) is dropped rather than failing the whole geometry: the existing
        // cover code already skips out-of-domain positions, and one bad vertex
        // in a large ring must not make the node unwritable. A geometry whose
        // positions ALL drop out yields no cells, which the caller reads as
        // "nothing to index" — the same outcome as an empty geometry.
        match raisin_proj::normalize_for_index(crs, p.x, p.y) {
            Ok((x, y)) => Some(Position { x, y, z: p.z }),
            Err(e) => {
                if failure.is_none() {
                    failure = Some(e);
                }
                None
            }
        }
    });

    if let Some(e) = failure {
        tracing::warn!(
            srid,
            error = %e,
            "dropped out-of-domain position(s) while normalising a geometry for the spatial index"
        );
    }

    normalized.set_srid(None);
    Ok(Cow::Owned(normalized))
}

/// True when this build can produce index cells for `srid`.
///
/// Feature-independent — see the module docs. Exposed so a validation layer can
/// reject a geometry before it reaches the write batch.
pub fn is_indexable_srid(srid: u32) -> bool {
    raisin_proj::is_index_normalizable(Crs::from(srid))
}

/// The one wording for "this SRID cannot be spatially indexed".
///
/// Phrased here rather than reusing [`ProjError`]'s Display because that one
/// says "rebuild with --features ..." — true for a query-time transform, and
/// actively misleading for index normalisation, which ignores those features on
/// purpose.
fn unsupported_srid(srid: u32) -> Error {
    Error::Validation(format!(
        "geometry SRID {srid} cannot be indexed: spatial index keys are geohashes over \
         WGS84 lon/lat, and index-time normalisation is restricted to the built-in \
         projection tier (EPSG:4326, EPSG:3857 and the WGS84 UTM zones 32601-32660 / \
         32701-32760) so that every node in a cluster derives identical index bytes. \
         Enabling the 'proj4rs-backend' or 'proj-backend' Cargo features widens \
         ST_TRANSFORM but deliberately does NOT widen this set. Store the geometry in \
         one of those CRS, or apply ST_TRANSFORM(<geometry>, 4326) before writing."
    ))
}

/// Rebuild a geometry with every position passed through `f`.
///
/// `f` returning `None` drops that position. Structure is otherwise preserved,
/// including empty rings and nested `GeometryCollection`s, so the caller's
/// centroid/bbox pass sees the same shape it would have seen in WGS84.
fn map_positions<F>(geometry: &GeoJson, f: &mut F) -> GeoJson
where
    F: FnMut(&Position) -> Option<Position>,
{
    fn map_ring<F>(ring: &[Position], f: &mut F) -> Vec<Position>
    where
        F: FnMut(&Position) -> Option<Position>,
    {
        ring.iter().filter_map(f).collect()
    }

    match geometry {
        GeoJson::Point { coordinates, srid } => match f(coordinates) {
            Some(p) => GeoJson::Point {
                coordinates: p,
                srid: *srid,
            },
            // A Point whose only position is out of domain becomes an empty
            // geometry, which `cells_for_geometry` reads as "not indexable".
            None => GeoJson::empty(),
        },
        GeoJson::LineString { coordinates, srid } => GeoJson::LineString {
            coordinates: map_ring(coordinates, f),
            srid: *srid,
        },
        GeoJson::MultiPoint { coordinates, srid } => GeoJson::MultiPoint {
            coordinates: map_ring(coordinates, f),
            srid: *srid,
        },
        GeoJson::Polygon { coordinates, srid } => GeoJson::Polygon {
            coordinates: coordinates.iter().map(|r| map_ring(r, f)).collect(),
            srid: *srid,
        },
        GeoJson::MultiLineString { coordinates, srid } => GeoJson::MultiLineString {
            coordinates: coordinates.iter().map(|r| map_ring(r, f)).collect(),
            srid: *srid,
        },
        GeoJson::MultiPolygon { coordinates, srid } => GeoJson::MultiPolygon {
            coordinates: coordinates
                .iter()
                .map(|poly| poly.iter().map(|r| map_ring(r, f)).collect())
                .collect(),
            srid: *srid,
        },
        GeoJson::GeometryCollection { geometries, srid } => GeoJson::GeometryCollection {
            geometries: geometries.iter().map(|g| map_positions(g, f)).collect(),
            srid: *srid,
        },
    }
}

#[cfg(test)]
#[path = "normalize_tests.rs"]
mod tests;
