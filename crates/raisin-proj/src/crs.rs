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

//! Coordinate reference system identity.
//!
//! A [`Crs`] is just a validated EPSG code. It is deliberately a thin newtype:
//! the *authority* on what a code means lives in the backends, not here, so that
//! enabling `proj-backend` widens coverage without changing this type.

use serde::{Deserialize, Serialize};

use crate::error::{ProjError, Result};

/// WGS84 geographic coordinates (longitude/latitude in degrees). The RaisinDB
/// storage default and the SRID that GeoJSON mandates (RFC 7946 §4).
pub const EPSG_WGS84: u32 = 4326;

/// WGS84 / Pseudo-Mercator — the projection every web slippy map uses. Metres.
pub const EPSG_WEB_MERCATOR: u32 = 3857;

/// Deprecated-but-common aliases that mean the same thing as [`EPSG_WEB_MERCATOR`].
/// `900913` was Google's original unofficial code; `3785` was the deprecated
/// "Popular Visualisation CRS / Mercator". Both are still emitted by older tools.
pub const WEB_MERCATOR_ALIASES: &[u32] = &[3857, 3785, 900913];

/// First EPSG code of the WGS84 / UTM northern-hemisphere block (zone 1 = 32601).
pub const EPSG_UTM_NORTH_BASE: u32 = 32600;

/// First EPSG code of the WGS84 / UTM southern-hemisphere block (zone 1 = 32701).
pub const EPSG_UTM_SOUTH_BASE: u32 = 32700;

/// A coordinate reference system, identified by EPSG code.
///
/// Construct with [`Crs::from_srid`] or parse a textual form with [`Crs::parse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Crs(u32);

impl Crs {
    /// WGS84 lon/lat.
    pub const WGS84: Crs = Crs(EPSG_WGS84);

    /// WGS84 / Pseudo-Mercator (metres).
    pub const WEB_MERCATOR: Crs = Crs(EPSG_WEB_MERCATOR);

    /// Wrap a numeric SRID. Any non-zero code is accepted here; whether it can
    /// actually be *transformed* is decided by the backends at transform time.
    pub fn from_srid(srid: u32) -> Self {
        Crs(srid)
    }

    /// The numeric EPSG code.
    pub fn srid(&self) -> u32 {
        self.0
    }

    /// Parse `EPSG:4326`, `epsg:4326`, `SRID=4326`, `urn:ogc:def:crs:EPSG::4326`
    /// or a bare `4326`.
    ///
    /// Note that OGC URN form and `CRS84` both denote WGS84 lon/lat ordering,
    /// which is what RaisinDB stores, so they normalise to 4326.
    pub fn parse(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(ProjError::UnrecognizedCrs {
                input: input.to_string(),
            });
        }

        // `CRS84` / `OGC:CRS84` are lon/lat WGS84 by definition.
        let upper = trimmed.to_ascii_uppercase();
        if upper == "CRS84" || upper == "OGC:CRS84" || upper == "WGS84" {
            return Ok(Crs::WGS84);
        }

        // Take the digits after the last ':' or '=', which handles EPSG:x,
        // SRID=x and the OGC URN form (which ends in '::x') uniformly.
        let tail = upper.rsplit([':', '=']).next().unwrap_or(&upper);

        // Guard against a prefix we do not understand claiming to be an EPSG
        // code, e.g. "ESRI:102100" is NOT EPSG 102100 in the EPSG registry.
        let has_known_prefix = upper.starts_with("EPSG")
            || upper.starts_with("SRID")
            || upper.starts_with("URN:OGC:DEF:CRS:EPSG")
            || upper.chars().all(|c| c.is_ascii_digit());
        if !has_known_prefix {
            return Err(ProjError::UnrecognizedCrs {
                input: input.to_string(),
            });
        }

        let srid: u32 = tail.parse().map_err(|_| ProjError::UnrecognizedCrs {
            input: input.to_string(),
        })?;
        if srid == 0 {
            return Err(ProjError::UnrecognizedCrs {
                input: input.to_string(),
            });
        }
        Ok(Crs::normalize(srid))
    }

    /// Collapse deprecated synonyms onto their canonical code so that
    /// `900913 -> 3857` is a no-op rather than an "unsupported pair".
    pub fn normalize(srid: u32) -> Self {
        if WEB_MERCATOR_ALIASES.contains(&srid) {
            Crs(EPSG_WEB_MERCATOR)
        } else {
            Crs(srid)
        }
    }

    /// True when this CRS holds angular lon/lat degrees rather than linear units.
    /// Only decidable here for the codes the built-in backend owns; other codes
    /// are answered by the active backend.
    pub fn is_geographic(&self) -> bool {
        self.0 == EPSG_WGS84
    }

    /// `Some((zone, north))` when this is a WGS84 UTM CRS (EPSG:326xx / 327xx).
    pub fn utm_zone(&self) -> Option<(u8, bool)> {
        for (base, north) in [(EPSG_UTM_NORTH_BASE, true), (EPSG_UTM_SOUTH_BASE, false)] {
            if self.0 > base && self.0 <= base + 60 {
                return Some(((self.0 - base) as u8, north));
            }
        }
        None
    }

    /// The EPSG code of the WGS84 UTM zone that best fits the given lon/lat.
    /// Useful for picking a metric CRS in which to do a planar measurement.
    pub fn best_utm_for(lon: f64, lat: f64) -> Self {
        let zone = (((lon + 180.0).rem_euclid(360.0) / 6.0).floor() as u32 + 1).clamp(1, 60);
        let base = if lat >= 0.0 {
            EPSG_UTM_NORTH_BASE
        } else {
            EPSG_UTM_SOUTH_BASE
        };
        Crs(base + zone)
    }
}

impl std::fmt::Display for Crs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EPSG:{}", self.0)
    }
}

impl From<u32> for Crs {
    fn from(srid: u32) -> Self {
        Crs::normalize(srid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_textual_forms() {
        for form in [
            "EPSG:4326",
            "epsg:4326",
            "SRID=4326",
            "urn:ogc:def:crs:EPSG::4326",
            "4326",
            "CRS84",
        ] {
            assert_eq!(Crs::parse(form).unwrap(), Crs::WGS84, "form {form}");
        }
    }

    #[test]
    fn rejects_unknown_authority_and_zero() {
        // ESRI:102100 is web mercator in ESRI's registry but NOT EPSG 102100;
        // silently accepting it would be exactly the kind of lie we are removing.
        assert!(Crs::parse("ESRI:102100").is_err());
        assert!(Crs::parse("EPSG:0").is_err());
        assert!(Crs::parse("").is_err());
        assert!(Crs::parse("EPSG:abc").is_err());
    }

    #[test]
    fn normalizes_web_mercator_aliases() {
        for alias in [3857u32, 3785, 900913] {
            assert_eq!(Crs::from(alias), Crs::WEB_MERCATOR);
        }
    }

    #[test]
    fn decodes_utm_zones() {
        assert_eq!(Crs::from_srid(32632).utm_zone(), Some((32, true)));
        assert_eq!(Crs::from_srid(32732).utm_zone(), Some((32, false)));
        assert_eq!(Crs::from_srid(32600).utm_zone(), None);
        assert_eq!(Crs::from_srid(32661).utm_zone(), None);
        assert_eq!(Crs::WGS84.utm_zone(), None);
    }

    #[test]
    fn picks_best_utm_zone() {
        // Zurich 8.54E 47.37N -> zone 32 north.
        assert_eq!(Crs::best_utm_for(8.54, 47.37), Crs::from_srid(32632));
        // Sydney 151.2E 33.87S -> zone 56 south.
        assert_eq!(Crs::best_utm_for(151.2, -33.87), Crs::from_srid(32756));
    }
}
