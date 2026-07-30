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

//! Unit tests for ST_SRID / ST_SETSRID / ST_TRANSFORM.
//!
//! # Reference values, not round trips
//!
//! A round-trip-only test (`4326 -> 3857 -> 4326` returns the input) passes even
//! when both directions are wrong in the same way — an inverted sign, a swapped
//! axis pair or a wrong ellipsoid all cancel. So every projection assertion below
//! is against an **externally derived** number, and the derivation is stated:
//!
//! * **EPSG:3857** — from the closed-form definition of Pseudo-Mercator, which is
//!   spherical Mercator on the WGS84 semi-major axis `a = 6378137`:
//!   `x = a·λ`, `y = a·ln(tan(π/4 + φ/2))`, λ and φ in radians. That *is* the CRS
//!   definition, so it is ground truth rather than a second opinion.
//!   `20037508.342789244` is the well-known Mercator half-width, `a·π`.
//! * **WGS84 / UTM** — from a 6th-order Krüger series, cross-checked on the
//!   central meridian against Simpson quadrature of the exact meridian-arc
//!   integral `M(φ) = a(1−e²)∫₀^φ (1−e² sin²t)^(−3/2) dt`; the two agreed to
//!   1.3 × 10⁻⁷ m. The quadrature involves no series expansion at all, and its
//!   `M(90°) = 10001965.729 m` matches the published WGS84 quarter meridian. The
//!   implementation under test uses a **3rd**-order Krüger series, so the tolerance
//!   below (5 mm) is what separates "different truncation order" from "wrong".
//!
//! Zurich is used throughout precisely because `(8.54, 47.37)` and `(47.37, 8.54)`
//! are both individually plausible but land 4500 km apart, in different UTM zones,
//! so a swapped-axis regression cannot pass any of these assertions.
//!
//! These are unit tests, and per the brief unit tests are not proof of work —
//! `spatial_crs_test.rs` proves the same properties against a real server across
//! HTTP and WebSocket.

use super::*;
use crate::physical_plan::eval::functions::traits::SqlFunction;
use crate::physical_plan::executor::Row;
use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};
use serde_json::{json, Value};

// --- reference values (see the module doc comment for their provenance) -------

/// Zurich, WGS84 lon/lat.
const ZURICH: (f64, f64) = (8.54, 47.37);
/// Zurich in EPSG:3857, from `x = a·λ`, `y = a·ln(tan(π/4 + φ/2))`.
const ZURICH_3857: (f64, f64) = (950_668.451_374_556_3, 6_002_677.997_532_715);
/// Zurich in EPSG:32632 (WGS84 / UTM zone 32N), 6th-order Krüger.
const ZURICH_UTM32N: (f64, f64) = (465_270.423_099_666_7, 5_246_384.775_981_838);

/// Sydney Opera House, WGS84 lon/lat — a southern-hemisphere, eastern-longitude
/// case, so a sign error in either ordinate shows up.
const SYDNEY: (f64, f64) = (151.2153, -33.8568);
/// Sydney in EPSG:3857.
const SYDNEY_3857: (f64, f64) = (16_833_210.196_152_102, -4_009_589.934_222_665);
/// Sydney in EPSG:32756 (WGS84 / UTM zone 56S), 6th-order Krüger. Note the
/// 10 000 000 m southern false northing.
const SYDNEY_UTM56S: (f64, f64) = (334_900.569_652_263_2, 6_252_288.752_888_292);

/// `a·π`, the half-width of the whole Mercator world.
const MERCATOR_HALF_WIDTH: f64 = 20_037_508.342_789_244;

/// The implementation carries the Krüger series to 3rd order; the reference to
/// 6th. 5 mm is generous for that difference and far tighter than any real bug.
const UTM_TOLERANCE_M: f64 = 0.005;
/// Pseudo-Mercator is closed form, so agreement should be to floating-point noise.
const MERCATOR_TOLERANCE_M: f64 = 1e-6;

// --- harness -----------------------------------------------------------------

fn geom_arg(v: Value) -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Geometry(v)), DataType::Geometry)
}

fn int_arg(v: i32) -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Int(v)), DataType::Int)
}

fn text_arg(v: &str) -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Text(v.to_string())), DataType::Text)
}

fn null_arg() -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Null), DataType::Unknown)
}

fn row() -> Row {
    Row::new()
}

fn point(lon: f64, lat: f64) -> Value {
    json!({"type": "Point", "coordinates": [lon, lat]})
}

/// The `(x, y)` of a transformed Point result.
fn xy(literal: &Literal) -> (f64, f64) {
    let Literal::Geometry(v) = literal else {
        panic!("expected a GEOMETRY, got {literal:?}");
    };
    let c = v["coordinates"].as_array().expect("coordinates");
    (c[0].as_f64().unwrap(), c[1].as_f64().unwrap())
}

fn transform(geom: Value, srid: i32) -> Literal {
    StTransformFunction
        .evaluate(&[geom_arg(geom), int_arg(srid)], &row())
        .expect("ST_TRANSFORM")
}

fn assert_close(label: &str, got: (f64, f64), want: (f64, f64), tol: f64) {
    assert!(
        (got.0 - want.0).abs() <= tol && (got.1 - want.1).abs() <= tol,
        "{label}: got ({}, {}), expected ({}, {}) within {tol} m",
        got.0,
        got.1,
        want.0,
        want.1
    );
}

// --- ST_SRID -----------------------------------------------------------------

/// The regression this whole area exists for: it used to return 4326 always.
#[test]
fn st_srid_reports_the_real_label_not_a_constant() {
    let mut labelled = point(2_683_000.0, 1_247_000.0);
    labelled["srid"] = json!(2056);
    assert_eq!(
        StSridFunction
            .evaluate(&[geom_arg(labelled)], &row())
            .unwrap(),
        Literal::Int(2056)
    );
}

/// Unlabelled means 4326, which is what keeps every pre-existing query working.
#[test]
fn an_unlabelled_geometry_reports_4326() {
    assert_eq!(
        StSridFunction
            .evaluate(&[geom_arg(point(ZURICH.0, ZURICH.1))], &row())
            .unwrap(),
        Literal::Int(4326)
    );
}

/// 3785 and 900913 denote the same CRS as 3857. Reporting the alias verbatim would
/// make `ST_SRID(a) = ST_SRID(b)` false for two geometries in the same CRS.
#[test]
fn deprecated_web_mercator_aliases_are_canonicalised() {
    for alias in [3785, 900_913] {
        let mut g = point(0.0, 0.0);
        g["srid"] = json!(alias);
        assert_eq!(
            StSridFunction.evaluate(&[geom_arg(g)], &row()).unwrap(),
            Literal::Int(3857),
            "alias {alias}"
        );
    }
}

#[test]
fn st_srid_propagates_null_and_rejects_a_non_geometry() {
    assert_eq!(
        StSridFunction.evaluate(&[null_arg()], &row()).unwrap(),
        Literal::Null
    );
    // A bare JSON object is not "in WGS84"; claiming so would hide a modelling
    // error, so this is an error rather than 4326.
    let err = StSridFunction
        .evaluate(&[geom_arg(json!({"lat": 47.37, "lon": 8.54}))], &row())
        .unwrap_err();
    assert!(err.to_string().contains("type"), "{err}");
}

// --- ST_SETSRID: relabels, never moves ---------------------------------------

#[test]
fn st_setsrid_changes_the_label_and_nothing_else() {
    let out = StSetSridFunction
        .evaluate(
            &[geom_arg(point(2_683_000.0, 1_247_000.0)), int_arg(2056)],
            &row(),
        )
        .unwrap();
    let Literal::Geometry(v) = &out else { panic!() };
    assert_eq!(v["srid"], 2056);
    assert_eq!(
        v["coordinates"],
        json!([2_683_000.0, 1_247_000.0]),
        "ST_SETSRID must not move the geometry — that is ST_TRANSFORM's job"
    );
}

/// The distinction, asserted rather than merely documented: given the same inputs,
/// one function moves the coordinates and the other does not.
#[test]
fn setsrid_and_transform_differ_exactly_in_whether_coordinates_move() {
    let g = point(ZURICH.0, ZURICH.1);
    let relabelled = StSetSridFunction
        .evaluate(&[geom_arg(g.clone()), int_arg(3857)], &row())
        .unwrap();
    let moved = transform(g, 3857);

    assert_eq!(xy(&relabelled), ZURICH, "ST_SETSRID reinterprets");
    assert_close(
        "ST_TRANSFORM moves",
        xy(&moved),
        ZURICH_3857,
        MERCATOR_TOLERANCE_M,
    );
}

/// 4326 removes the member instead of writing it, so ordinary output stays
/// strictly RFC 7946 conformant (the RFC forbids declaring another CRS at all).
#[test]
fn labelling_as_4326_removes_the_member() {
    let mut g = point(1.0, 2.0);
    g["srid"] = json!(3857);
    let out = StSetSridFunction
        .evaluate(&[geom_arg(g), int_arg(4326)], &row())
        .unwrap();
    let Literal::Geometry(v) = &out else { panic!() };
    assert!(v.get("srid").is_none(), "{v}");
}

/// A label is a claim about the data, so any positive EPSG code is accepted even
/// when this build cannot *transform* it. Availability is ST_TRANSFORM's question.
#[test]
fn setsrid_accepts_a_code_this_build_cannot_transform() {
    let out = StSetSridFunction
        .evaluate(
            &[geom_arg(point(150_000.0, 170_000.0)), int_arg(31_370)],
            &row(),
        )
        .unwrap();
    let Literal::Geometry(v) = &out else { panic!() };
    assert_eq!(v["srid"], 31_370);
}

#[test]
fn setsrid_rejects_nonsense_codes_and_foreign_authorities() {
    for bad in [0, -1] {
        assert!(
            StSetSridFunction
                .evaluate(&[geom_arg(point(0.0, 0.0)), int_arg(bad)], &row())
                .is_err(),
            "SRID {bad} must be rejected"
        );
    }
    // ESRI:102100 is WebMercator in ESRI's registry but is NOT EPSG:102100.
    assert!(StSetSridFunction
        .evaluate(
            &[geom_arg(point(0.0, 0.0)), text_arg("ESRI:102100")],
            &row()
        )
        .is_err());
}

#[test]
fn textual_crs_forms_are_accepted_for_both_functions() {
    for form in ["EPSG:3857", "epsg:3857", "SRID=3857", "3857"] {
        let out = StSetSridFunction
            .evaluate(&[geom_arg(point(1.0, 2.0)), text_arg(form)], &row())
            .unwrap();
        let Literal::Geometry(v) = &out else { panic!() };
        assert_eq!(v["srid"], 3857, "form {form}");
    }
    let moved = StTransformFunction
        .evaluate(
            &[geom_arg(point(ZURICH.0, ZURICH.1)), text_arg("EPSG:3857")],
            &row(),
        )
        .unwrap();
    assert_close(
        "textual target",
        xy(&moved),
        ZURICH_3857,
        MERCATOR_TOLERANCE_M,
    );
}

// --- ST_TRANSFORM against external reference values --------------------------

#[test]
fn wgs84_to_web_mercator_matches_the_closed_form_definition() {
    assert_close(
        "Zurich 4326->3857",
        xy(&transform(point(ZURICH.0, ZURICH.1), 3857)),
        ZURICH_3857,
        MERCATOR_TOLERANCE_M,
    );
    assert_close(
        "Sydney 4326->3857",
        xy(&transform(point(SYDNEY.0, SYDNEY.1), 3857)),
        SYDNEY_3857,
        MERCATOR_TOLERANCE_M,
    );
}

/// The two structural landmarks of Pseudo-Mercator: the antimeridian sits at
/// exactly `a·π`, and so does the top of the usable latitude range. Getting the
/// scale factor wrong shows up here immediately.
#[test]
fn the_mercator_world_is_a_square_of_the_expected_size() {
    let (x, _) = xy(&transform(point(180.0, 0.0), 3857));
    assert!((x - MERCATOR_HALF_WIDTH).abs() < 1e-6, "{x}");

    let (_, y) = xy(&transform(point(0.0, 85.051_128_779_806_6), 3857));
    assert!((y - MERCATOR_HALF_WIDTH).abs() < 1e-3, "{y}");

    assert_eq!(xy(&transform(point(0.0, 0.0), 3857)).0, 0.0);
}

#[test]
fn web_mercator_back_to_wgs84_recovers_the_original_degrees() {
    let mut mercator = point(ZURICH_3857.0, ZURICH_3857.1);
    mercator["srid"] = json!(3857);
    let (lon, lat) = xy(&transform(mercator, 4326));
    assert!((lon - ZURICH.0).abs() < 1e-9, "{lon}");
    assert!((lat - ZURICH.1).abs() < 1e-9, "{lat}");
}

#[test]
fn wgs84_to_utm_matches_a_sixth_order_kruger_reference() {
    assert_close(
        "Zurich 4326->32632",
        xy(&transform(point(ZURICH.0, ZURICH.1), 32632)),
        ZURICH_UTM32N,
        UTM_TOLERANCE_M,
    );
    assert_close(
        "Sydney 4326->32756",
        xy(&transform(point(SYDNEY.0, SYDNEY.1), 32756)),
        SYDNEY_UTM56S,
        UTM_TOLERANCE_M,
    );
}

/// On a zone's central meridian the easting is exactly the 500 km false easting
/// and the northing is `k0 · M(φ)`. `k0·M(45°) = 4982950.4002264` from the exact
/// quadrature — a value that depends on the ellipsoid and the scale factor and on
/// nothing else, so it isolates those two from the series entirely.
#[test]
fn on_the_central_meridian_utm_reduces_to_the_meridian_arc() {
    let (e, n) = xy(&transform(point(9.0, 45.0), 32632));
    assert!((e - 500_000.0).abs() < 1e-6, "false easting: {e}");
    assert!(
        (n - 4_982_950.400_226_4).abs() < UTM_TOLERANCE_M,
        "k0*M(45): {n}"
    );

    let (e0, n0) = xy(&transform(point(9.0, 0.0), 32632));
    assert!((e0 - 500_000.0).abs() < 1e-6, "{e0}");
    assert!(n0.abs() < 1e-6, "the equator is northing zero: {n0}");
}

/// A swap is 4500 km and two UTM zones away, so it cannot masquerade as a rounding
/// difference. This is the assertion that would fail if anyone "fixed" the axis
/// order to match the EPSG authority's lat/lon definition of EPSG:4326.
#[test]
fn a_swapped_zurich_does_not_land_anywhere_near_the_right_answer() {
    let swapped = xy(&transform(point(ZURICH.1, ZURICH.0), 3857));
    let distance = (swapped.0 - ZURICH_3857.0).hypot(swapped.1 - ZURICH_3857.1);
    assert!(
        distance > 1_000_000.0,
        "a reversed argument order must not be within a megametre of the truth, \
         got {distance} m — is the axis convention still lon/lat?"
    );
}

/// The OGC URN form of EPSG:4326 is often claimed to imply lat/lon ordering. We
/// pin lon/lat for every form, deliberately, so the two must agree exactly.
#[test]
fn the_ogc_urn_form_does_not_flip_the_axes() {
    let numeric = xy(&transform(point(ZURICH.0, ZURICH.1), 3857));
    let urn = StTransformFunction
        .evaluate(
            &[
                geom_arg({
                    let mut g = point(ZURICH_3857.0, ZURICH_3857.1);
                    g["srid"] = json!("urn:ogc:def:crs:EPSG::3857");
                    g
                }),
                text_arg("urn:ogc:def:crs:EPSG::4326"),
            ],
            &row(),
        )
        .unwrap();
    let (lon, lat) = xy(&urn);
    assert!(
        (lon - ZURICH.0).abs() < 1e-9 && (lat - ZURICH.1).abs() < 1e-9,
        "{lon} {lat}"
    );
    assert_close(
        "numeric form unchanged",
        numeric,
        ZURICH_3857,
        MERCATOR_TOLERANCE_M,
    );
}

// --- every geometry type, altitude, emptiness --------------------------------

#[test]
fn every_geometry_type_transforms_including_nested_collections() {
    let cases: Vec<Value> = vec![
        point(ZURICH.0, ZURICH.1),
        json!({"type":"MultiPoint","coordinates":[[8.5,47.3],[8.6,47.4]]}),
        json!({"type":"LineString","coordinates":[[8.5,47.3],[8.6,47.4]]}),
        json!({"type":"MultiLineString","coordinates":[[[8.5,47.3],[8.6,47.4]]]}),
        json!({"type":"Polygon","coordinates":[
            [[8.5,47.3],[8.6,47.3],[8.6,47.4],[8.5,47.3]],
            [[8.52,47.32],[8.55,47.32],[8.55,47.35],[8.52,47.32]]
        ]}),
        json!({"type":"MultiPolygon","coordinates":[[[[8.5,47.3],[8.6,47.3],[8.6,47.4],[8.5,47.3]]]]}),
        json!({"type":"GeometryCollection","geometries":[
            {"type":"Point","coordinates":[8.54,47.37]},
            {"type":"GeometryCollection","geometries":[
                {"type":"LineString","coordinates":[[8.5,47.3],[8.6,47.4]]}
            ]}
        ]}),
    ];
    for case in cases {
        let type_name = case["type"].as_str().unwrap().to_string();
        let out = transform(case, 32632);
        let Literal::Geometry(v) = &out else { panic!() };
        assert_eq!(v["type"], type_name.as_str(), "type must be preserved");
        assert_eq!(v["srid"], 32632, "{type_name} must be relabelled");

        // Every ordinate must now be a UTM metre rather than a degree. Checking
        // the magnitude is what makes this a real assertion: a converter that
        // silently returned its input would leave values under 180.
        let mut ordinates = 0usize;
        for_each_ordinate(v, &mut |value| {
            ordinates += 1;
            assert!(
                value.abs() > 1_000.0,
                "{type_name}: {value} is still a degree, not a metre"
            );
        });
        assert!(ordinates >= 2, "{type_name}: no ordinates were visited");
        // A GeometryCollection is labelled once, at the top, not per member.
        assert_eq!(
            v.to_string().matches("\"srid\"").count(),
            1,
            "{type_name} should carry exactly one srid member: {v}"
        );
    }
}

/// Visit every numeric leaf of a geometry's `coordinates`, recursing through a
/// `GeometryCollection`'s members.
fn for_each_ordinate(geometry: &Value, f: &mut impl FnMut(f64)) {
    if let Some(members) = geometry.get("geometries").and_then(Value::as_array) {
        for member in members {
            for_each_ordinate(member, f);
        }
        return;
    }
    fn walk(node: &Value, f: &mut impl FnMut(f64)) {
        match node {
            Value::Number(n) => f(n.as_f64().unwrap()),
            Value::Array(items) => items.iter().for_each(|item| walk(item, f)),
            _ => {}
        }
    }
    if let Some(coords) = geometry.get("coordinates") {
        walk(coords, f);
    }
}

/// Polygon ring structure must survive: an interior ring stays an interior ring.
#[test]
fn polygon_holes_keep_their_nesting_depth() {
    let poly = json!({"type":"Polygon","coordinates":[
        [[8.5,47.3],[8.6,47.3],[8.6,47.4],[8.5,47.3]],
        [[8.52,47.32],[8.55,47.32],[8.55,47.35],[8.52,47.32]]
    ]});
    let out = transform(poly, 3857);
    let Literal::Geometry(v) = &out else { panic!() };
    let rings = v["coordinates"].as_array().unwrap();
    assert_eq!(rings.len(), 2);
    assert_eq!(rings[0].as_array().unwrap().len(), 4);
    assert_eq!(rings[1].as_array().unwrap().len(), 4);
}

/// Every transform available here is horizontal, so altitude must survive
/// untouched. The 2-D `geo` pipeline would have dropped it — which is exactly why
/// the reprojection walks JSON instead.
#[test]
fn altitude_rides_through_unchanged() {
    let g = json!({"type":"Point","coordinates":[ZURICH.0, ZURICH.1, 412.5]});
    let out = transform(g, 32632);
    let Literal::Geometry(v) = &out else { panic!() };
    let c = v["coordinates"].as_array().unwrap();
    assert_eq!(c.len(), 3, "the third ordinate must not be dropped: {v}");
    assert_eq!(c[2].as_f64().unwrap(), 412.5);
    assert!((c[0].as_f64().unwrap() - ZURICH_UTM32N.0).abs() < UTM_TOLERANCE_M);
}

#[test]
fn an_empty_geometry_transforms_to_itself() {
    for empty in [
        json!({"type":"GeometryCollection","geometries":[]}),
        json!({"type":"MultiPoint","coordinates":[]}),
        json!({"type":"Polygon","coordinates":[]}),
    ] {
        let type_name = empty["type"].as_str().unwrap().to_string();
        let out = transform(empty, 3857);
        let Literal::Geometry(v) = &out else { panic!() };
        assert_eq!(v["type"], type_name.as_str());
        assert_eq!(v["srid"], 3857);
    }
}

// --- failure modes: loud, never approximate ----------------------------------

/// The message must name the SRID and the Cargo feature. Silent passthrough would
/// yield a geometry wrong by hundreds of kilometres with nothing to indicate it.
#[test]
fn an_unsupported_srid_names_the_code_and_the_feature() {
    let err = StTransformFunction
        .evaluate(
            &[geom_arg(point(ZURICH.0, ZURICH.1)), int_arg(999_999)],
            &row(),
        )
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("999999"), "{msg}");
    assert!(
        msg.contains("proj") || msg.contains("features"),
        "must name the feature that would enable it: {msg}"
    );
}

/// The 85.05–90° band is the dangerous one: libproj returns a *finite* northing of
/// 242 528 680 m at the pole, twelve times the height of the whole Mercator world,
/// reported as success. A finite-value check cannot catch that, so the domain guard
/// lives above the backends — and this asserts ST_TRANSFORM inherits it.
#[test]
fn a_pole_against_web_mercator_is_rejected_not_silently_finite() {
    for lat in [86.0, 89.9, 90.0] {
        let err = StTransformFunction
            .evaluate(&[geom_arg(point(0.0, lat)), int_arg(3857)], &row())
            .unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("domain") || err.to_string().contains("85"),
            "lat {lat}: {err}"
        );
    }
}

/// All or nothing. A half-projected ring is a structurally valid polygon
/// describing nowhere, which is worse than an error.
#[test]
fn one_bad_coordinate_fails_the_whole_geometry() {
    let poly = json!({"type":"Polygon","coordinates":[[
        [8.5,47.3],[8.6,47.3],[0.0,89.5],[8.5,47.3]
    ]]});
    assert!(
        StTransformFunction
            .evaluate(&[geom_arg(poly), int_arg(3857)], &row())
            .is_err(),
        "a single out-of-domain vertex must fail the geometry"
    );
}

#[test]
fn transform_propagates_null_from_either_argument() {
    assert_eq!(
        StTransformFunction
            .evaluate(&[null_arg(), int_arg(3857)], &row())
            .unwrap(),
        Literal::Null
    );
    assert_eq!(
        StTransformFunction
            .evaluate(&[geom_arg(point(0.0, 0.0)), null_arg()], &row())
            .unwrap(),
        Literal::Null
    );
}

/// Transforming to the CRS a geometry is already in is a relabel, not an
/// "unsupported pair" — which is what makes `ST_TRANSFORM(g, 900913)` on 3857 data
/// behave sensibly.
#[test]
fn an_identity_transform_stamps_the_label_without_moving_anything() {
    let mut g = point(950_668.45, 6_002_678.0);
    g["srid"] = json!(3857);
    let out = transform(g, 900_913);
    let Literal::Geometry(v) = &out else { panic!() };
    assert_eq!(v["srid"], 3857, "alias canonicalised");
    assert_eq!(v["coordinates"], json!([950_668.45, 6_002_678.0]));
}

#[test]
fn a_wrong_arity_or_a_non_geometry_is_rejected() {
    assert!(StTransformFunction
        .evaluate(&[geom_arg(point(0.0, 0.0))], &row())
        .is_err());
    assert!(StSetSridFunction
        .evaluate(&[geom_arg(point(0.0, 0.0))], &row())
        .is_err());
    assert!(StTransformFunction
        .evaluate(&[geom_arg(json!({"type": "Nope"})), int_arg(3857)], &row())
        .is_err());
}

// --- SRID mismatch on binary operations --------------------------------------

/// The shared rule the ~20 binary ST_* functions use, exercised here so that the
/// wording and the unlabelled-adopts behaviour are pinned even before those
/// functions are migrated onto it.
#[test]
fn two_different_explicit_srids_are_an_error_naming_both_and_the_fix() {
    let mut a = point(8.54, 47.37);
    a["srid"] = json!(4326);
    let mut b = point(950_668.45, 6_002_678.0);
    b["srid"] = json!(3857);

    let err = raisin_geometry::resolve_pair_srid("ST_INTERSECTS", &a, &b, None).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("4326") && msg.contains("3857"), "{msg}");
    assert!(msg.contains("ST_INTERSECTS"), "{msg}");
    assert!(
        msg.contains("ST_TRANSFORM"),
        "must say how to fix it rather than transforming implicitly, which would \
         make a query's success depend on Cargo features: {msg}"
    );
}

#[test]
fn an_unlabelled_operand_adopts_the_labelled_one() {
    let bare = point(465_270.42, 5_246_384.78);
    let mut utm = point(465_000.0, 5_246_000.0);
    utm["srid"] = json!(32632);

    assert_eq!(
        raisin_geometry::resolve_pair_srid("ST_DWITHIN", &bare, &utm, None).unwrap(),
        raisin_geometry::Crs::from_srid(32632)
    );
    // Two unlabelled operands are 4326, which is what keeps existing queries
    // working with no changes at all.
    assert_eq!(
        raisin_geometry::resolve_pair_srid("ST_DWITHIN", &bare, &bare, None).unwrap(),
        raisin_geometry::Crs::WGS84
    );
}

// --- the guaranteed default-build coverage ----------------------------------

/// The set an operator gets with no Cargo features, no system libproj and no C
/// toolchain. This runs in CI unchanged, which is the point.
#[test]
fn the_default_build_covers_4326_3857_and_every_utm_zone() {
    assert!(raisin_proj::can_transform(
        raisin_geometry::Crs::WGS84,
        raisin_geometry::Crs::WEB_MERCATOR
    ));
    for zone in 1..=60u32 {
        for base in [32_600u32, 32_700] {
            let crs = raisin_geometry::Crs::from_srid(base + zone);
            assert!(
                raisin_proj::can_transform(raisin_geometry::Crs::WGS84, crs),
                "EPSG:{} must be in the built-in tier",
                base + zone
            );
        }
    }
}

/// A round trip through each guaranteed CRS. Weak on its own — hence every
/// reference-value test above — but it does catch an asymmetric inverse.
#[test]
fn round_trips_through_the_guaranteed_set_return_the_input() {
    /// The zone whose central meridian the point actually sits near. Forcing an
    /// arbitrary zone still round-trips, but only because the series degrades
    /// gracefully; measuring in the wrong zone is not a case worth asserting.
    fn best_zone(lon: f64, lat: f64) -> i32 {
        let zone = (((lon + 180.0) / 6.0).floor() as i32 + 1).clamp(1, 60);
        (if lat >= 0.0 { 32_600 } else { 32_700 }) + zone
    }

    for (lon, lat) in [ZURICH, SYDNEY, (-122.4194, 37.7749), (0.0, 0.0)] {
        for target in [3857, best_zone(lon, lat)] {
            let out = transform(point(lon, lat), target);
            let Literal::Geometry(projected) = out else {
                panic!()
            };
            let back = transform(projected, 4326);
            let (rlon, rlat) = xy(&back);
            assert!(
                (rlon - lon).abs() < 1e-7 && (rlat - lat).abs() < 1e-7,
                "({lon}, {lat}) via EPSG:{target} came back as ({rlon}, {rlat})"
            );
        }
    }
}
