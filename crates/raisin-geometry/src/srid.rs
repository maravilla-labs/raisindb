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

//! Reading, writing and comparing the SRID that travels with a geometry.
//!
//! Shared here rather than in the SQL crate so that the topological predicates,
//! the measurements and the CRS functions all phrase an SRID mismatch
//! identically — there are about twenty binary ST_\* functions and they must not
//! each invent their own wording.

use raisin_proj::Crs;
use serde_json::Value;

use crate::error::{GeometryError, Result};
use crate::geom::Geom;

/// The `srid` member of a geometry value, if it has one.
///
/// `Ok(None)` means **unlabelled**, which is deliberately distinct from
/// `Ok(Some(4326))`: an unlabelled geometry adopts the other operand's SRID in a
/// binary operation, which is what keeps every pre-existing query and every 4326
/// dataset working with no changes.
pub fn srid_member(v: &Value) -> Result<Option<Crs>> {
    match v.get("srid") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => {
            let code = n.as_u64().filter(|c| *c > 0 && *c <= u32::MAX as u64);
            match code {
                Some(c) => Ok(Some(Crs::from(c as u32))),
                None => Err(GeometryError::InvalidSrid {
                    reason: format!("{n} is not a positive EPSG code"),
                }),
            }
        }
        // A textual form is accepted because clients and CLI users type
        // "EPSG:2056". `Crs::parse` rejects other authorities on purpose:
        // ESRI:102100 is NOT EPSG:102100.
        Some(Value::String(s)) => Ok(Some(Crs::parse(s)?)),
        Some(other) => Err(GeometryError::InvalidSrid {
            reason: format!("expected a number or 'EPSG:<code>' string, got {other}"),
        }),
    }
}

/// The effective CRS of a geometry value.
///
/// Precedence: explicit `srid` member > `schema_default` (from the NodeType field
/// schema, then the workspace spatial default) > EPSG:4326.
pub fn srid_of(v: &Value, schema_default: Option<u32>) -> Result<Crs> {
    Ok(srid_member(v)?
        .or_else(|| schema_default.map(Crs::from))
        .unwrap_or(Crs::WGS84))
}

/// Stamp an SRID onto a geometry value without touching its coordinates.
///
/// This is `ST_SETSRID`, never `ST_TRANSFORM`. WGS84 removes the member instead
/// of writing `4326`, keeping 4326 output strictly RFC-7946-conformant.
pub fn with_srid(mut v: Value, srid: Crs) -> Value {
    if let Some(map) = v.as_object_mut() {
        if srid == Crs::WGS84 {
            map.remove("srid");
        } else {
            map.insert("srid".into(), Value::from(srid.srid()));
        }
    }
    v
}

/// The common CRS of two operands, or a mismatch error.
///
/// Both sides are already resolved [`Geom`]s here, so "unlabelled" has been lost;
/// use [`resolve_pair_srid`] when the raw values are still available and the
/// adoption rule matters.
pub fn require_same_srid(fn_name: &str, a: &Geom, b: &Geom) -> Result<Crs> {
    if a.srid == b.srid {
        return Ok(a.srid);
    }
    Err(GeometryError::SridMismatch {
        function: fn_name.to_string(),
        left: a.srid.srid(),
        right: b.srid.srid(),
    })
}

/// Decide the CRS for a binary operation over two raw geometry values, applying
/// the adoption rule.
///
/// Rules, in order:
/// 1. both unlabelled → `schema_default`, else EPSG:4326;
/// 2. exactly one labelled → the other **adopts** that label (this is what makes
///    `ST_INTERSECTS(properties->>'loc', ST_POINT(...))` keep working when the
///    stored value is labelled and the literal is not);
/// 3. both labelled and equal → that CRS;
/// 4. both labelled and different → [`GeometryError::SridMismatch`].
///
/// Rule 4 is an error rather than an implicit transform for two reasons: it
/// hides a data-modelling mistake, and on a build without the required
/// projection backend the implicit transform would fail — making a query's
/// success depend on Cargo features.
pub fn resolve_pair_srid(
    fn_name: &str,
    a: &Value,
    b: &Value,
    schema_default: Option<u32>,
) -> Result<Crs> {
    let fallback = schema_default.map(Crs::from).unwrap_or(Crs::WGS84);
    match (srid_member(a)?, srid_member(b)?) {
        (None, None) => Ok(fallback),
        (Some(s), None) | (None, Some(s)) => Ok(s),
        (Some(l), Some(r)) if l == r => Ok(l),
        (Some(l), Some(r)) => Err(GeometryError::SridMismatch {
            function: fn_name.to_string(),
            left: l.srid(),
            right: r.srid(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{Geometry, Point};
    use serde_json::json;

    fn geom(srid: u32) -> Geom {
        Geom::new(Geometry::Point(Point::new(0.0, 0.0)), Crs::from(srid))
    }

    #[test]
    fn unlabelled_is_distinct_from_explicit_4326() {
        let unlabelled = json!({"type":"Point","coordinates":[1,2]});
        let labelled = json!({"type":"Point","coordinates":[1,2],"srid":4326});
        assert_eq!(srid_member(&unlabelled).unwrap(), None);
        assert_eq!(srid_member(&labelled).unwrap(), Some(Crs::WGS84));
        // Both still resolve to 4326 as the effective CRS.
        assert_eq!(srid_of(&unlabelled, None).unwrap(), Crs::WGS84);
        assert_eq!(srid_of(&labelled, None).unwrap(), Crs::WGS84);
    }

    #[test]
    fn schema_default_fills_an_unlabelled_value_only() {
        let unlabelled = json!({"type":"Point","coordinates":[1,2]});
        assert_eq!(
            srid_of(&unlabelled, Some(2056)).unwrap(),
            Crs::from_srid(2056)
        );

        let labelled = json!({"type":"Point","coordinates":[1,2],"srid":3857});
        assert_eq!(srid_of(&labelled, Some(2056)).unwrap(), Crs::WEB_MERCATOR);
    }

    #[test]
    fn textual_srid_forms_are_accepted_but_foreign_authorities_are_not() {
        for form in ["EPSG:2056", "epsg:2056", "SRID=2056", "2056"] {
            let v = json!({"type":"Point","coordinates":[1,2],"srid": form});
            assert_eq!(
                srid_member(&v).unwrap(),
                Some(Crs::from_srid(2056)),
                "form {form}"
            );
        }
        // ESRI:102100 is WebMercator in ESRI's registry but NOT EPSG:102100;
        // accepting it would be exactly the sort of lie this work removes.
        let v = json!({"type":"Point","coordinates":[1,2],"srid":"ESRI:102100"});
        assert!(srid_member(&v).is_err());
    }

    #[test]
    fn a_nonsense_srid_member_is_an_error_not_a_silent_4326() {
        for bad in [json!(0), json!(-1), json!(1.5), json!(true), json!([4326])] {
            let v = json!({"type":"Point","coordinates":[1,2],"srid": bad});
            assert!(srid_member(&v).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn with_srid_writes_and_removes_the_member() {
        let v = json!({"type":"Point","coordinates":[1,2]});
        let stamped = with_srid(v.clone(), Crs::from_srid(2056));
        assert_eq!(stamped["srid"], 2056);
        assert_eq!(stamped["coordinates"], json!([1, 2]), "coords untouched");

        let cleared = with_srid(stamped, Crs::WGS84);
        assert!(cleared.get("srid").is_none());
    }

    #[test]
    fn mismatch_message_names_the_function_and_both_codes() {
        let err = require_same_srid("ST_INTERSECTS", &geom(4326), &geom(3857)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ST_INTERSECTS"), "{msg}");
        assert!(msg.contains("4326") && msg.contains("3857"), "{msg}");
        assert!(
            msg.contains("ST_TRANSFORM"),
            "must say how to fix it: {msg}"
        );
    }

    #[test]
    fn identical_and_aliased_srids_are_not_a_mismatch() {
        assert_eq!(
            require_same_srid("ST_EQUALS", &geom(3857), &geom(900_913)).unwrap(),
            Crs::WEB_MERCATOR
        );
    }

    #[test]
    fn an_unlabelled_operand_adopts_the_labelled_one() {
        let bare = json!({"type":"Point","coordinates":[1,2]});
        let utm = json!({"type":"Point","coordinates":[1,2],"srid":32632});

        assert_eq!(
            resolve_pair_srid("ST_DWITHIN", &bare, &utm, None).unwrap(),
            Crs::from_srid(32632)
        );
        assert_eq!(
            resolve_pair_srid("ST_DWITHIN", &utm, &bare, None).unwrap(),
            Crs::from_srid(32632)
        );
        assert_eq!(
            resolve_pair_srid("ST_DWITHIN", &bare, &bare, None).unwrap(),
            Crs::WGS84
        );
        assert_eq!(
            resolve_pair_srid("ST_DWITHIN", &bare, &bare, Some(2056)).unwrap(),
            Crs::from_srid(2056)
        );
    }

    #[test]
    fn two_different_explicit_srids_are_a_hard_error() {
        let a = json!({"type":"Point","coordinates":[1,2],"srid":4326});
        let b = json!({"type":"Point","coordinates":[1,2],"srid":3857});
        assert!(matches!(
            resolve_pair_srid("ST_CONTAINS", &a, &b, None),
            Err(GeometryError::SridMismatch { .. })
        ));
    }
}
