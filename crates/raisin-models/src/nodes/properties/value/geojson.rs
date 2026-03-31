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

//! GeoJSON geometry types (RFC 7946 compliant).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// GeoJSON geometry types (RFC 7946 compliant).
///
/// Coordinates are `[longitude, latitude]` per GeoJSON spec (not lat/lon!).
/// All coordinates are in WGS84 (EPSG:4326).
///
/// # Examples
///
/// ```json
/// {"type": "Point", "coordinates": [-122.4194, 37.7749]}
/// ```
///
/// ```json
/// {"type": "Polygon", "coordinates": [[[-122.5, 37.7], [-122.3, 37.7], [-122.3, 37.8], [-122.5, 37.8], [-122.5, 37.7]]]}
/// ```
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(tag = "type")]
pub enum GeoJson {
    /// A single point: [longitude, latitude]
    Point { coordinates: [f64; 2] },

    /// A line of connected points
    LineString { coordinates: Vec<[f64; 2]> },

    /// A closed polygon (first ring is exterior, rest are holes)
    /// Each ring is a list of [lon, lat] coordinates where first == last
    Polygon { coordinates: Vec<Vec<[f64; 2]>> },

    /// Multiple points
    MultiPoint { coordinates: Vec<[f64; 2]> },

    /// Multiple line strings
    MultiLineString { coordinates: Vec<Vec<[f64; 2]>> },

    /// Multiple polygons
    MultiPolygon {
        coordinates: Vec<Vec<Vec<[f64; 2]>>>,
    },

    /// A collection of any geometry types
    GeometryCollection { geometries: Vec<GeoJson> },
}

impl GeoJson {
    /// Create a Point from longitude and latitude
    pub fn point(lon: f64, lat: f64) -> Self {
        GeoJson::Point {
            coordinates: [lon, lat],
        }
    }

    /// Get the centroid coordinates [lon, lat] for simple geometries
    pub fn centroid(&self) -> Option<[f64; 2]> {
        match self {
            GeoJson::Point { coordinates } => Some(*coordinates),
            GeoJson::LineString { coordinates } if !coordinates.is_empty() => {
                // Midpoint of first and last
                let first = coordinates.first()?;
                let last = coordinates.last()?;
                Some([(first[0] + last[0]) / 2.0, (first[1] + last[1]) / 2.0])
            }
            GeoJson::Polygon { coordinates } if !coordinates.is_empty() => {
                // Simple centroid of exterior ring
                let ring = coordinates.first()?;
                if ring.is_empty() {
                    return None;
                }
                let sum: [f64; 2] = ring.iter().fold([0.0, 0.0], |acc, coord| {
                    [acc[0] + coord[0], acc[1] + coord[1]]
                });
                let n = ring.len() as f64;
                Some([sum[0] / n, sum[1] / n])
            }
            _ => None,
        }
    }

    /// Check if this is a Point geometry
    pub fn is_point(&self) -> bool {
        matches!(self, GeoJson::Point { .. })
    }

    /// Get coordinates as [lon, lat] if this is a Point
    pub fn as_point(&self) -> Option<[f64; 2]> {
        match self {
            GeoJson::Point { coordinates } => Some(*coordinates),
            _ => None,
        }
    }
}
