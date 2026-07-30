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

//! The single linear component the vertex accessors operate on.
//!
//! `ST_STARTPOINT`, `ST_ENDPOINT`, `ST_POINTN` and `ST_LINEINTERPOLATEPOINT` all
//! answer a question about *one* path. Rather than each deciding independently
//! what to do with a `MultiLineString` — which is how they ended up with four
//! different behaviours, three of which silently returned NULL — they share this
//! one rule:
//!
//! * a `LineString` is that path;
//! * a `MultiLineString` or `GeometryCollection` holding **exactly one** linear
//!   component is that component (the answer must not depend on the spelling);
//! * anything else has no single path and the accessor returns SQL NULL, matching
//!   PostGIS, so a mixed-geometry column does not abort the query on its first
//!   non-linear row.

use geo::LineString;
use raisin_geometry::Geom;

use super::walk::for_each_line_string;

/// The one linear component of a geometry, if it has exactly one.
pub(super) fn sole_line(g: &Geom) -> Option<LineString<f64>> {
    let mut found: Option<LineString<f64>> = None;
    let mut count = 0usize;
    for_each_line_string(&g.geometry, &mut |ls| {
        count += 1;
        if count == 1 {
            found = Some(ls.clone());
        }
    });
    if count != 1 {
        return None;
    }
    found.filter(|ls| ls.0.len() >= 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{Geometry, MultiLineString, Point};

    fn line() -> LineString<f64> {
        LineString::from(vec![(0.0, 0.0), (1.0, 1.0)])
    }

    #[test]
    fn a_linestring_is_its_own_sole_line() {
        let g = Geom::wgs84(Geometry::LineString(line()));
        assert_eq!(sole_line(&g), Some(line()));
    }

    #[test]
    fn a_one_component_multilinestring_is_accepted() {
        let g = Geom::wgs84(Geometry::MultiLineString(MultiLineString(vec![line()])));
        assert_eq!(
            sole_line(&g),
            Some(line()),
            "the answer must not depend on the spelling"
        );
    }

    #[test]
    fn two_components_have_no_sole_line() {
        let g = Geom::wgs84(Geometry::MultiLineString(MultiLineString(vec![
            line(),
            line(),
        ])));
        assert_eq!(sole_line(&g), None);
    }

    #[test]
    fn non_linear_and_degenerate_geometries_have_none() {
        assert_eq!(
            sole_line(&Geom::wgs84(Geometry::Point(Point::new(1.0, 2.0)))),
            None
        );
        assert_eq!(
            sole_line(&Geom::wgs84(Geometry::LineString(LineString::from(vec![
                (0.0, 0.0)
            ])))),
            None,
            "a one-vertex line is not a path"
        );
    }
}
