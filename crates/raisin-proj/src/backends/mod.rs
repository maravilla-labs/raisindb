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

//! Backend selection.
//!
//! Backends are tried highest-fidelity first, so that enabling `proj-backend`
//! upgrades accuracy for codes the built-in also covers (datum-correct results
//! rather than WGS84-assuming ones) without any call-site change.

use crate::crs::Crs;
use crate::error::{ProjError, Result};

#[cfg(feature = "proj-backend")]
pub mod proj_backend;

#[cfg(feature = "proj4rs-backend")]
pub mod proj4rs_backend;

/// Reject inputs that lie outside the transform's domain, *before* any backend
/// sees them, so that all tiers agree on what is representable.
///
/// This exists because the backends do **not** agree on their own. Measured on
/// libproj 9.6.2: EPSG:4326 -> EPSG:3857 at the north pole returns a finite
/// northing of 242_528_680.94 m — over twelve times the ~20_037_508 m half-height
/// of the entire Mercator world, i.e. geometric nonsense rather than an error.
/// The built-in backend rejects the same input. A finite-value check cannot
/// catch this (the value *is* finite), so the domain has to be enforced here.
///
/// Letting that value through would be worse than a wrong answer: a northing of
/// 242_000 km geohashes to a garbage cell and silently corrupts the spatial
/// index. `ST_TRANSFORM` promises never to return approximate coordinates, and
/// that promise must not depend on which Cargo features are enabled.
pub(crate) fn check_input_domain(from: Crs, to: Crs, x: f64, y: f64) -> Result<()> {
    if !x.is_finite() || !y.is_finite() {
        return Err(ProjError::OutOfDomain {
            x,
            y,
            srid: from.srid(),
            reason: "coordinate is not finite",
        });
    }

    if from.is_geographic() {
        if y.abs() > 90.0 {
            return Err(ProjError::OutOfDomain {
                x,
                y,
                srid: from.srid(),
                reason: "latitude exceeds ±90°; coordinate order is (longitude, latitude)",
            });
        }
        // EPSG:3857 is only defined to ±85.0511287798066°; beyond that the
        // northing diverges towards infinity and carries no useful meaning.
        if to.srid() == crate::crs::EPSG_WEB_MERCATOR
            && y.abs() > crate::builtin::mercator::WEB_MERCATOR_LAT_LIMIT
        {
            return Err(ProjError::OutOfDomain {
                x,
                y,
                srid: to.srid(),
                reason: "latitude exceeds the ±85.0511287798066° Mercator limit",
            });
        }
    }

    Ok(())
}

/// Transform a coordinate using the most capable compiled backend that supports
/// the pair.
pub fn transform(from: Crs, to: Crs, x: f64, y: f64) -> Result<(f64, f64)> {
    // Enforced for every tier so a feature flag cannot change the answer.
    check_input_domain(from, to, x, y)?;

    // Tier 3: full PROJ. Authoritative when present.
    #[cfg(feature = "proj-backend")]
    {
        match proj_backend::transform(from, to, x, y) {
            Ok(v) => return Ok(v),
            // A genuine domain error is final — do not retry on a weaker backend
            // and risk returning a less correct answer for the same input.
            Err(e @ ProjError::OutOfDomain { .. }) => return Err(e),
            Err(e) => {
                tracing::debug!(error = %e, "proj backend declined; trying next");
            }
        }
    }

    // Tier 2: pure-Rust broad EPSG.
    #[cfg(feature = "proj4rs-backend")]
    {
        match proj4rs_backend::transform(from, to, x, y) {
            Ok(v) => return Ok(v),
            Err(e @ ProjError::OutOfDomain { .. }) => return Err(e),
            Err(e) => {
                tracing::debug!(error = %e, "proj4rs backend declined; trying built-in");
            }
        }
    }

    // Tier 1: always compiled in.
    crate::builtin::transform(from, to, x, y)
}

/// Whether any compiled backend claims support for this pair.
pub fn can_transform(from: Crs, to: Crs) -> bool {
    #[cfg(feature = "proj-backend")]
    if proj_backend::supports(from) && proj_backend::supports(to) {
        return true;
    }
    #[cfg(feature = "proj4rs-backend")]
    if proj4rs_backend::supports(from) && proj4rs_backend::supports(to) {
        return true;
    }
    crate::builtin::supports(from) && crate::builtin::supports(to)
}

/// Suppress the unused-import warning in a default build, where neither optional
/// backend module exists and `ProjError` is only referenced from the cfg blocks.
#[allow(dead_code)]
fn _assert_error_in_scope(e: ProjError) -> ProjError {
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These must hold under EVERY feature combination. Run them with
    /// `--features proj4rs-backend` and `--features proj-backend` too; the whole
    /// point is that the answer does not depend on the enabled backend.
    #[test]
    fn mercator_domain_is_enforced_identically_on_every_backend() {
        // The pole: libproj happily returns a finite 242_528_680.94 m northing
        // here, the built-in refuses. The guard makes both refuse.
        let err = transform(Crs::WGS84, Crs::WEB_MERCATOR, 0.0, 90.0).unwrap_err();
        assert!(
            matches!(err, ProjError::OutOfDomain { .. }),
            "pole must be out of domain, got {err:?}"
        );

        // Just inside the limit still works, so the guard is not over-eager.
        let (_, y) = transform(Crs::WGS84, Crs::WEB_MERCATOR, 0.0, 85.0).unwrap();
        assert!(
            (19_000_000.0..21_000_000.0).contains(&y),
            "northing at 85° looks wrong: {y}"
        );
    }

    #[test]
    fn a_swapped_lat_lon_pair_is_rejected_rather_than_projected() {
        // ST_POINT(47.37, 8.54) for Zurich is the classic swapped-argument bug:
        // latitude 8.54 is legal, so this specific pair cannot be caught here.
        // What must be caught is the unambiguous case, where the supposed
        // latitude is outside ±90°.
        let err = transform(Crs::WGS84, Crs::WEB_MERCATOR, 47.37, 185.4).unwrap_err();
        assert!(matches!(err, ProjError::OutOfDomain { .. }), "{err:?}");
    }

    #[test]
    fn non_finite_input_never_reaches_a_backend() {
        for (x, y) in [(f64::NAN, 0.0), (0.0, f64::INFINITY)] {
            assert!(matches!(
                transform(Crs::WGS84, Crs::WEB_MERCATOR, x, y),
                Err(ProjError::OutOfDomain { .. })
            ));
        }
    }
}
