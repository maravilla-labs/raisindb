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

use serde_json::json;

use super::super::fixtures::expr;
use super::super::harness::Ctx;
use super::{ZURICH_3857_X, ZURICH_3857_Y, ZURICH_LAT, ZURICH_LON, ZURICH_UTM32_E, ZURICH_UTM32_N};

pub(super) async fn transform(ctx: &mut Ctx) {
    // 4326 -> 3857 against the closed-form reference. 0.5 m tolerance: far
    // tighter than any plausible formula error, loose enough for the last
    // decimals of the published value.
    ctx.near(
        "ST_TRANSFORM 4326->3857 easting matches the closed form",
        &format!("ST_X(ST_TRANSFORM(ST_POINT({ZURICH_LON}, {ZURICH_LAT}), 3857))"),
        ZURICH_3857_X,
        0.5,
    )
    .await;
    ctx.near(
        "ST_TRANSFORM 4326->3857 northing matches the closed form",
        &format!("ST_Y(ST_TRANSFORM(ST_POINT({ZURICH_LON}, {ZURICH_LAT}), 3857))"),
        ZURICH_3857_Y,
        0.5,
    )
    .await;
    ctx.eq(
        "ST_TRANSFORM relabels the result",
        &format!("ST_SRID(ST_TRANSFORM(ST_POINT({ZURICH_LON}, {ZURICH_LAT}), 3857))"),
        json!(3857),
    )
    .await;

    // Two structural landmarks that isolate the scale factor from the series.
    // The antimeridian sits at exactly a*pi = 20037508.34 m.
    ctx.near(
        "ST_TRANSFORM puts the antimeridian at a*pi",
        "ST_X(ST_TRANSFORM(ST_POINT(180, 0), 3857))",
        20_037_508.342_789_244,
        0.001,
    )
    .await;
    // The equator maps to y = 0 exactly.
    ctx.near(
        "ST_TRANSFORM puts the equator at y=0",
        "ST_Y(ST_TRANSFORM(ST_POINT(0, 0), 3857))",
        0.0,
        1e-6,
    )
    .await;

    // UTM zone 32N for Zurich. On a zone's central meridian the easting is
    // exactly 500000; Zurich is at 8.5417 E and zone 32's CM is 9 E, so it sits
    // just west of centre and the easting is just under 500000.
    ctx.near(
        "ST_TRANSFORM puts a central-meridian point at easting 500000",
        "ST_X(ST_TRANSFORM(ST_POINT(9, 47.3769), 32632))",
        500_000.0,
        0.001,
    )
    .await;
    // UTM zone 32N against the independent Krüger series. 1 mm tolerance: this
    // separates "a different truncation order" from "wrong".
    ctx.near(
        "ST_TRANSFORM 4326->UTM32N easting matches the Kruger series",
        &format!("ST_X(ST_TRANSFORM(ST_POINT({ZURICH_LON}, {ZURICH_LAT}), 32632))"),
        ZURICH_UTM32_E,
        0.001,
    )
    .await;
    ctx.near(
        "ST_TRANSFORM 4326->UTM32N northing matches the Kruger series",
        &format!("ST_Y(ST_TRANSFORM(ST_POINT({ZURICH_LON}, {ZURICH_LAT}), 32632))"),
        ZURICH_UTM32_N,
        0.001,
    )
    .await;

    // The inverse direction must also be right. The projected point has to be
    // built with ST_GEOMFROMGEOJSON rather than ST_POINT, because ST_POINT
    // validates its arguments as lon/lat unconditionally and rejects an easting
    // of 950857 — see the product gaps in the run summary.
    let z3857 = format!(
        r#"ST_GEOMFROMGEOJSON('{{"type":"Point","coordinates":[{ZURICH_3857_X},{ZURICH_3857_Y}],"srid":3857}}')"#
    );
    ctx.near(
        "ST_TRANSFORM 3857->4326 inverse returns the original longitude",
        &format!("ST_X(ST_TRANSFORM({z3857}, 4326))"),
        ZURICH_LON,
        1e-6,
    )
    .await;
    ctx.near(
        "ST_TRANSFORM 3857->4326 inverse returns the original latitude",
        &format!("ST_Y(ST_TRANSFORM({z3857}, 4326))"),
        ZURICH_LAT,
        1e-6,
    )
    .await;

    // Every geometry type must transform, including nested collections.
    for label in ["ls", "poly_hole", "mpt", "mls", "mpoly", "gc"] {
        ctx.eq(
            &format!("ST_TRANSFORM preserves the type of {label}"),
            &format!(
                "ST_GEOMETRYTYPE(ST_TRANSFORM({}, 3857)) = ST_GEOMETRYTYPE({})",
                expr(label),
                expr(label)
            ),
            json!(true),
        )
        .await;
    }
    // Altitude survives reprojection: all supported transforms are horizontal.
    ctx.eq(
        "ST_TRANSFORM preserves altitude",
        &format!("ST_Z(ST_TRANSFORM({}, 3857))", expr("pt3d")),
        json!(250.0),
    )
    .await;

    // GUARANTEED SET in a default build (no cargo feature, no system libproj):
    // 4326, 3857 and all 120 WGS84 UTM zones.
    for srid in [4326, 3857, 32601, 32632, 32660, 32701, 32732, 32760] {
        ctx.num_matches(
            &format!("ST_TRANSFORM to {srid} works in a default build"),
            &format!("ST_SRID(ST_TRANSFORM(ST_POINT(8.5417, 47.3769), {srid}))"),
            "the requested SRID",
            move |s| s as i64 == srid,
        )
        .await;
    }

    // An unsupported SRID must ERROR and name the cargo feature — never
    // silently return unprojected coordinates. EPSG:2056 (Swiss LV95) is not in
    // the guaranteed tier.
    ctx.errors_with(
        "ST_TRANSFORM to an unavailable SRID names the cargo feature",
        "ST_X(ST_TRANSFORM(ST_POINT(8.5417, 47.3769), 2056))",
        "feature",
    )
    .await;
    ctx.errors_with(
        "ST_TRANSFORM to an unavailable SRID names the SRID",
        "ST_X(ST_TRANSFORM(ST_POINT(8.5417, 47.3769), 2056))",
        "2056",
    )
    .await;

    // Out-of-domain input is rejected rather than returning the FINITE nonsense
    // libproj produces near the pole for Pseudo-Mercator.
    ctx.errors_with(
        "ST_TRANSFORM rejects a near-pole point for 3857",
        "ST_Y(ST_TRANSFORM(ST_POINT(0, 89.9), 3857))",
        "",
    )
    .await;
}
