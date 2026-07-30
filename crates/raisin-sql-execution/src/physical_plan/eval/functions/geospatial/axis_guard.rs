//! The longitude/latitude swap guard.
//!
//! # Why this file exists at all
//!
//! Axis order is the classic GIS footgun, and RaisinDB's previous validation
//! caught only `|latitude| > 90`. So `ST_POINT(47.37, 8.54)` — Zurich with the
//! arguments reversed — passed silently and placed the point in Somalia. Both
//! ordinates are individually legal, so no range check can catch that pair; the
//! only defence is a heuristic plus a loud message.
//!
//! # The pinned convention
//!
//! `(x, y)` = `(longitude, latitude)` for geographic CRSs, `(easting, northing)`
//! for projected ones. Everywhere, no exceptions, and no per-EPSG-code axis
//! flipping. This deliberately diverges from the EPSG authority, which defines
//! EPSG:4326 as `(latitude, longitude)`: GeoJSON RFC 7946 §3.1.1, PostGIS,
//! `geo-types`, the `geohash` crate and every web mapping library are lon/lat,
//! and formal authority conformance would break interop with all of them and with
//! our own stored data. It is the same call PostGIS and GeoJSON made.
//!
//! # The three-way rule
//!
//! * `|x| <= 90` **and** `|y| > 90` → *certainly* swapped. Hard error, naming the
//!   corrected call.
//! * both within ±90 → *ambiguous*. Warn once per process per coordinate pair and
//!   accept. Silently accepting with no signal at all is the old behaviour, and it
//!   is how a whole dataset ends up in the Gulf of Guinea.
//! * `|y| > 90` with `|x| > 90` → always an error: no latitude exceeds ±90.

use std::collections::HashSet;
use std::sync::Mutex;

use raisin_error::Error;

/// Coordinate pairs already warned about, so a warning does not fire once per row
/// of a million-row scan. Keyed on the bit patterns, which is exact for f64.
static WARNED: Mutex<Option<HashSet<(u64, u64)>>> = Mutex::new(None);

/// Validate a geographic `(longitude, latitude)` pair for `fn_name`.
///
/// # Errors
///
/// * longitude outside ±180 or latitude outside ±90;
/// * a pair that is unambiguously reversed — the message names the corrected call
///   rather than merely reporting a range violation, because "latitude 185.4 out
///   of range" does not tell the user what they actually did wrong.
pub(super) fn check_lon_lat(fn_name: &str, lon: f64, lat: f64) -> Result<(), Error> {
    if !lon.is_finite() || !lat.is_finite() {
        return Err(Error::Validation(format!(
            "{fn_name}: coordinates must be finite numbers, got ({lon}, {lat})"
        )));
    }

    // Unambiguously reversed: the supposed latitude cannot be a latitude, while
    // the supposed longitude would be a perfectly good one.
    if lat.abs() > 90.0 && lon.abs() <= 90.0 {
        return Err(Error::Validation(format!(
            "{fn_name} takes (longitude, latitude); ({lon}, {lat}) looks reversed \
             — did you mean {fn_name}({lat}, {lon})?"
        )));
    }

    if !(-180.0..=180.0).contains(&lon) {
        return Err(Error::Validation(format!(
            "{fn_name}: longitude {lon} out of range [-180, 180]. Coordinate order \
             is (longitude, latitude)"
        )));
    }
    if !(-90.0..=90.0).contains(&lat) {
        return Err(Error::Validation(format!(
            "{fn_name}: latitude {lat} out of range [-90, 90]. Coordinate order is \
             (longitude, latitude)"
        )));
    }

    // Ambiguous: both ordinates are legal latitudes, so the pair *might* be
    // reversed and we cannot tell. Say so once rather than saying nothing.
    if lat.abs() <= 90.0 && lon.abs() <= 90.0 && warn_once(lon, lat) {
        tracing::warn!(
            longitude = lon,
            latitude = lat,
            function = fn_name,
            "Ambiguous coordinate pair: both ordinates are valid latitudes, so a \
             reversed (latitude, longitude) argument order cannot be detected. \
             RaisinDB always takes (longitude, latitude)."
        );
    }

    Ok(())
}

/// True the first time this exact pair is seen.
fn warn_once(lon: f64, lat: f64) -> bool {
    let key = (lon.to_bits(), lat.to_bits());
    match WARNED.lock() {
        Ok(mut guard) => guard.get_or_insert_with(HashSet::new).insert(key),
        // A poisoned mutex must not fail a query over a diagnostic; just stay
        // quiet.
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact regression: Zurich, reversed. This used to pass silently.
    #[test]
    fn a_certainly_reversed_pair_is_a_hard_error_naming_the_fix() {
        let err = check_lon_lat("ST_POINT", 47.37, 185.4).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("looks reversed"), "{msg}");
        assert!(
            msg.contains("ST_POINT(185.4, 47.37)"),
            "must name the corrected call: {msg}"
        );
    }

    #[test]
    fn correct_zurich_is_accepted() {
        assert!(check_lon_lat("ST_POINT", 8.54, 47.37).is_ok());
    }

    /// The genuinely undetectable case: both ordinates are valid latitudes, so
    /// this is accepted (with a warning) rather than rejected. Rejecting it would
    /// break every legitimate point between the tropics-ish band.
    #[test]
    fn an_ambiguous_pair_is_accepted() {
        assert!(check_lon_lat("ST_POINT", 47.37, 8.54).is_ok());
    }

    #[test]
    fn out_of_range_ordinates_are_rejected_with_the_convention_spelled_out() {
        for (lon, lat) in [(181.0, 0.0), (-181.0, 0.0), (200.0, 95.0)] {
            let msg = check_lon_lat("ST_POINT", lon, lat).unwrap_err().to_string();
            assert!(
                msg.contains("(longitude, latitude)"),
                "({lon}, {lat}): {msg}"
            );
        }
    }

    #[test]
    fn non_finite_ordinates_are_rejected() {
        assert!(check_lon_lat("ST_POINT", f64::NAN, 0.0).is_err());
        assert!(check_lon_lat("ST_POINT", 0.0, f64::INFINITY).is_err());
    }

    #[test]
    fn the_boundaries_themselves_are_valid() {
        for (lon, lat) in [(180.0, 90.0), (-180.0, -90.0), (0.0, 0.0)] {
            assert!(
                check_lon_lat("ST_POINT", lon, lat).is_ok(),
                "({lon}, {lat})"
            );
        }
    }

    #[test]
    fn the_ambiguity_warning_fires_only_once_per_pair() {
        assert!(warn_once(12.5, 34.5));
        assert!(!warn_once(12.5, 34.5));
        assert!(warn_once(12.5, 34.6));
    }
}
