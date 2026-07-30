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

//! A single GeoJSON position, with optional altitude.
//!
//! # Why this is a newtype and not `[f64; 2]` or `Vec<f64>`
//!
//! [`GeoJson`](super::GeoJson) used `[f64; 2]` fixed arrays, which made altitude
//! *structurally* impossible: a three-element input silently lost its third
//! ordinate. `Vec<f64>` would allow it but costs a heap allocation per
//! coordinate, and the spatial index writes one geometry per node revision.
//!
//! [`Position`] is `Copy`, 24 bytes, allocation-free, and — the load-bearing
//! property — **serializes byte-identically to `[f64; 2]` whenever `z` is
//! `None`**. See the serde discussion below.
//!
//! # Serde compatibility contract (do not break this)
//!
//! Positions are written into stored node blobs (MessagePack) and hashed into
//! property-index keys. So:
//!
//! * A 2-D position emits a **2-element** array. Identical bytes to the old
//!   `[f64; 2]` in both JSON and MessagePack, so no stored blob changes, no
//!   `hash_property_value` change, no property-index churn and no CRDT churn.
//! * A 3-D position emits a **3-element** array, which is exactly what
//!   RFC 7946 §3.1.1 specifies for an elevation-carrying position.
//! * On read we accept 2, 3 or 4 elements. A fourth ("M") ordinate is tolerated
//!   and dropped, because RFC 7946 says implementations SHOULD NOT extend beyond
//!   three and we never *emit* one — but rejecting third-party data outright
//!   would be worse than ignoring an ordinate we have no use for.
//!
//! [`Deserialize`] is implemented with `deserialize_any` rather than
//! `deserialize_seq`. That is required, not stylistic: `GeoJson` is
//! `#[serde(tag = "type")]`, so serde buffers the whole object into its internal
//! `Content` representation and replays it through `ContentDeserializer`, which
//! only routes sequence content correctly for `deserialize_any`.

use std::fmt;

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::{SeqAccess, Visitor};
use serde::ser::SerializeTuple;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A GeoJSON position: `(x, y[, z])`.
///
/// For geographic CRSs that is `(longitude, latitude[, altitude in metres above
/// the WGS84 ellipsoid])`; for projected CRSs it is
/// `(easting, northing[, height])`. Axis order is pinned to x-then-y everywhere
/// in RaisinDB — never `(latitude, longitude)` — matching GeoJSON, PostGIS,
/// `geo-types` and every web mapping library.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    /// Longitude, or easting in a projected CRS.
    pub x: f64,
    /// Latitude, or northing in a projected CRS.
    pub y: f64,
    /// Altitude in metres, when the source carried one.
    ///
    /// `None` is *not* "altitude zero": it means the position is two
    /// dimensional, and `ST_Z` on it is SQL NULL.
    pub z: Option<f64>,
}

impl Position {
    /// A two-dimensional position.
    pub const fn new_2d(x: f64, y: f64) -> Self {
        Position { x, y, z: None }
    }

    /// A three-dimensional position.
    pub const fn new_3d(x: f64, y: f64, z: f64) -> Self {
        Position { x, y, z: Some(z) }
    }

    /// The horizontal ordinates, dropping any altitude.
    ///
    /// This is the projection into the 2-D world that `geo-types` (whose `Coord`
    /// has no Z at all) and the geohash index both live in.
    pub const fn xy(&self) -> [f64; 2] {
        [self.x, self.y]
    }

    /// True when this position carries an altitude.
    pub const fn is_3d(&self) -> bool {
        self.z.is_some()
    }

    /// 2 or 3 — the number of ordinates this position would serialize as.
    pub const fn ndims(&self) -> u8 {
        if self.z.is_some() {
            3
        } else {
            2
        }
    }

    /// Drop the altitude, returning a two-dimensional position.
    pub const fn to_2d(&self) -> Self {
        Position::new_2d(self.x, self.y)
    }

    /// Attach (or replace) an altitude.
    pub const fn with_z(&self, z: f64) -> Self {
        Position::new_3d(self.x, self.y, z)
    }
}

impl From<[f64; 2]> for Position {
    fn from([x, y]: [f64; 2]) -> Self {
        Position::new_2d(x, y)
    }
}

impl From<[f64; 3]> for Position {
    fn from([x, y, z]: [f64; 3]) -> Self {
        Position::new_3d(x, y, z)
    }
}

impl From<(f64, f64)> for Position {
    fn from((x, y): (f64, f64)) -> Self {
        Position::new_2d(x, y)
    }
}

impl From<(f64, f64, f64)> for Position {
    fn from((x, y, z): (f64, f64, f64)) -> Self {
        Position::new_3d(x, y, z)
    }
}

impl From<Position> for [f64; 2] {
    fn from(p: Position) -> Self {
        p.xy()
    }
}

/// Positional access, kept so that the pervasive `coordinates[0]` /
/// `coordinates[1]` idiom in the storage and index layers keeps compiling and
/// keeps meaning the same thing.
///
/// # Panics
///
/// Index `2` panics on a two-dimensional position, and anything above `2` always
/// panics. Prefer [`Position::z`] when the dimensionality is not already known —
/// this is a compatibility shim, not the recommended accessor.
impl std::ops::Index<usize> for Position {
    type Output = f64;

    fn index(&self, i: usize) -> &f64 {
        match i {
            0 => &self.x,
            1 => &self.y,
            2 => self
                .z
                .as_ref()
                .expect("Position index 2 on a 2-D position: check is_3d() first"),
            other => panic!("Position index {other} out of range (0..=2)"),
        }
    }
}

impl Serialize for Position {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // `serialize_tuple` is what `[f64; N]` uses, so the emitted bytes match
        // the previous `[f64; 2]` representation exactly for 2-D positions.
        match self.z {
            None => {
                let mut t = serializer.serialize_tuple(2)?;
                t.serialize_element(&self.x)?;
                t.serialize_element(&self.y)?;
                t.end()
            }
            Some(z) => {
                let mut t = serializer.serialize_tuple(3)?;
                t.serialize_element(&self.x)?;
                t.serialize_element(&self.y)?;
                t.serialize_element(&z)?;
                t.end()
            }
        }
    }
}

struct PositionVisitor;

impl<'de> Visitor<'de> for PositionVisitor {
    type Value = Position;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a GeoJSON position: an array of 2 or 3 numbers [x, y] or [x, y, z]")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Position, A::Error> {
        use serde::de::Error as _;

        let x: f64 = seq
            .next_element()?
            .ok_or_else(|| A::Error::custom("GeoJSON position is missing its x ordinate"))?;
        let y: f64 = seq
            .next_element()?
            .ok_or_else(|| A::Error::custom("GeoJSON position is missing its y ordinate"))?;
        let z: Option<f64> = seq.next_element()?;

        // RFC 7946 §3.1.1: a position MAY carry a third element and SHOULD NOT
        // carry more. Tolerate a fourth (some producers emit an "M" ordinate)
        // and drop it rather than failing a whole node read over it.
        while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}

        Ok(Position { x, y, z })
    }
}

impl<'de> Deserialize<'de> for Position {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Position, D::Error> {
        // MUST be `deserialize_any`: see the module docs — `GeoJson` is an
        // internally tagged enum, so this runs against serde's
        // `ContentDeserializer`, not the original format's deserializer.
        deserializer.deserialize_any(PositionVisitor)
    }
}

impl JsonSchema for Position {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Position".into()
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        "raisin::geo::Position".into()
    }

    /// Hand-written on purpose. A derived schema would describe an *object*
    /// with `x`/`y`/`z` keys, which is not the wire format and would silently
    /// break every generated client type.
    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": { "type": "number" },
            "minItems": 2,
            "maxItems": 3,
            "description": "GeoJSON position [x, y] or [x, y, z] — (longitude, latitude[, altitude])",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_d_json_is_a_two_element_array() {
        let p = Position::new_2d(-122.4194, 37.7749);
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "[-122.4194,37.7749]");
    }

    #[test]
    fn three_d_json_is_a_three_element_array() {
        let p = Position::new_3d(8.54, 47.37, 412.0);
        assert_eq!(serde_json::to_string(&p).unwrap(), "[8.54,47.37,412.0]");
    }

    /// The compatibility guarantee that lets this land without a data migration.
    #[test]
    fn two_d_bytes_are_identical_to_the_old_fixed_array() {
        let p = Position::new_2d(-122.4194, 37.7749);
        let legacy: [f64; 2] = [-122.4194, 37.7749];

        assert_eq!(
            rmp_serde::to_vec_named(&p).unwrap(),
            rmp_serde::to_vec_named(&legacy).unwrap(),
            "MessagePack (named) bytes must match the pre-Position representation"
        );
        assert_eq!(
            rmp_serde::to_vec(&p).unwrap(),
            rmp_serde::to_vec(&legacy).unwrap(),
            "MessagePack (compact) bytes must match too"
        );
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            serde_json::to_string(&legacy).unwrap()
        );
    }

    #[test]
    fn round_trips_through_json_and_messagepack() {
        for p in [
            Position::new_2d(0.0, 0.0),
            Position::new_2d(-179.9999, -89.9999),
            Position::new_3d(8.54, 47.37, -12.5),
        ] {
            let json: Position = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
            assert_eq!(json, p, "json round trip");

            let mp: Position =
                rmp_serde::from_slice(&rmp_serde::to_vec_named(&p).unwrap()).unwrap();
            assert_eq!(mp, p, "messagepack (named) round trip");

            let mp: Position = rmp_serde::from_slice(&rmp_serde::to_vec(&p).unwrap()).unwrap();
            assert_eq!(mp, p, "messagepack (compact) round trip");
        }
    }

    #[test]
    fn reads_legacy_two_element_arrays() {
        let p: Position = serde_json::from_str("[8.54, 47.37]").unwrap();
        assert_eq!(p, Position::new_2d(8.54, 47.37));
        assert!(!p.is_3d());
        assert_eq!(p.ndims(), 2);
    }

    #[test]
    fn reads_integer_ordinates() {
        // JSON `[8, 47]` has no decimal point; it must still deserialize.
        let p: Position = serde_json::from_str("[8, 47]").unwrap();
        assert_eq!(p, Position::new_2d(8.0, 47.0));
    }

    #[test]
    fn tolerates_and_drops_a_fourth_m_ordinate() {
        let p: Position = serde_json::from_str("[1.0, 2.0, 3.0, 4.0]").unwrap();
        assert_eq!(p, Position::new_3d(1.0, 2.0, 3.0));
    }

    #[test]
    fn rejects_a_position_shorter_than_two_ordinates() {
        assert!(serde_json::from_str::<Position>("[1.0]").is_err());
        assert!(serde_json::from_str::<Position>("[]").is_err());
        assert!(serde_json::from_str::<Position>("\"nope\"").is_err());
        assert!(serde_json::from_str::<Position>("{\"x\":1,\"y\":2}").is_err());
    }

    #[test]
    fn conversions_and_accessors() {
        assert_eq!(Position::from([1.0, 2.0]), Position::new_2d(1.0, 2.0));
        assert_eq!(
            Position::from([1.0, 2.0, 3.0]),
            Position::new_3d(1.0, 2.0, 3.0)
        );
        assert_eq!(Position::from((1.0, 2.0)), Position::new_2d(1.0, 2.0));
        assert_eq!(
            Position::from((1.0, 2.0, 3.0)),
            Position::new_3d(1.0, 2.0, 3.0)
        );

        let p = Position::new_3d(1.0, 2.0, 3.0);
        assert_eq!(p.xy(), [1.0, 2.0]);
        assert_eq!(p.to_2d(), Position::new_2d(1.0, 2.0));
        assert_eq!(p.with_z(9.0).z, Some(9.0));
        assert_eq!(<[f64; 2]>::from(p), [1.0, 2.0]);

        // Positional compatibility access.
        assert_eq!(p[0], 1.0);
        assert_eq!(p[1], 2.0);
        assert_eq!(p[2], 3.0);
    }

    #[test]
    #[should_panic(expected = "2-D position")]
    fn indexing_z_on_a_2d_position_panics_loudly() {
        let _ = Position::new_2d(1.0, 2.0)[2];
    }

    #[test]
    fn json_schema_describes_an_array_not_an_object() {
        let schema = schemars::schema_for!(Position);
        let value = serde_json::to_value(&schema).unwrap();
        assert_eq!(value["type"], "array");
        assert_eq!(value["minItems"], 2);
        assert_eq!(value["maxItems"], 3);
    }
}
