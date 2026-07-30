//! Boundary cases: where CONTAINS and COVERS must differ, and the
//! dimension rules behind CROSSES and OVERLAPS.

use super::*;

/// The reason both `CONTAINS` and `COVERS` exist. A polygon does not contain its
/// own boundary, but it does cover it.
#[test]
fn a_point_on_the_boundary_is_covered_but_not_contained() {
    let on_edge = pt(0.0, 2.0); // on the west edge of square()
    assert!(!b(&StContainsFunction, square(), on_edge.clone()));
    assert!(b(&StCoversFunction, square(), on_edge.clone()));
    assert!(!b(&StWithinFunction, on_edge.clone(), square()));
    assert!(b(&StCoveredByFunction, on_edge.clone(), square()));
    // It still intersects, and it touches (interiors are disjoint).
    assert!(b(&StIntersectsFunction, square(), on_edge.clone()));
    assert!(b(&StTouchesFunction, square(), on_edge));
}

#[test]
fn an_interior_point_is_both_contained_and_covered() {
    let inside = pt(2.0, 2.0);
    assert!(b(&StContainsFunction, square(), inside.clone()));
    assert!(b(&StCoversFunction, square(), inside.clone()));
    assert!(
        !b(&StTouchesFunction, square(), inside),
        "interiors meet, so it is not a touch"
    );
}

/// Two polygons sharing only an edge touch, do not overlap, and are not disjoint.
#[test]
fn edge_adjacent_polygons_touch_but_do_not_overlap() {
    let left = json!({"type":"Polygon","coordinates":[[
        [0.0,0.0],[2.0,0.0],[2.0,2.0],[0.0,2.0],[0.0,0.0]]]});
    let right = json!({"type":"Polygon","coordinates":[[
        [2.0,0.0],[4.0,0.0],[4.0,2.0],[2.0,2.0],[2.0,0.0]]]});
    assert!(b(&StTouchesFunction, left.clone(), right.clone()));
    assert!(!b(&StOverlapsFunction, left.clone(), right.clone()));
    assert!(!b(&StDisjointFunction, left.clone(), right.clone()));
    assert!(b(&StIntersectsFunction, left, right));
}

/// A polygon covers its own boundary, which `CONTAINS` cannot express.
#[test]
fn a_polygon_covers_its_own_boundary_ring() {
    let ring = json!({"type":"LineString","coordinates":[
        [0.0,0.0],[4.0,0.0],[4.0,4.0],[0.0,4.0],[0.0,0.0]]});
    assert!(b(&StCoversFunction, square(), ring.clone()));
    assert!(
        !b(&StContainsFunction, square(), ring.clone()),
        "the boundary is not in the interior"
    );
    assert!(b(&StTouchesFunction, square(), ring));
}

/// Two polygons cannot cross: their intersection has the same dimension as the
/// inputs. Two lines can.
#[test]
fn crossing_respects_dimension() {
    let shifted = json!({"type":"Polygon","coordinates":[[
        [2.0,2.0],[6.0,2.0],[6.0,6.0],[2.0,6.0],[2.0,2.0]]]});
    assert!(!b(&StCrossesFunction, square(), shifted.clone()));
    assert!(b(&StOverlapsFunction, square(), shifted));

    // Two lines meeting at a single interior point cross.
    let a = json!({"type":"LineString","coordinates":[[0.0,2.0],[4.0,2.0]]});
    let c = json!({"type":"LineString","coordinates":[[2.0,0.0],[2.0,4.0]]});
    assert!(b(&StCrossesFunction, a.clone(), c.clone()));
    assert!(
        !b(&StOverlapsFunction, a, c),
        "a 0-D meeting of two 1-D geometries is a crossing, not an overlap"
    );

    // Two collinear, partially shared lines overlap but do not cross.
    let a = json!({"type":"LineString","coordinates":[[0.0,0.0],[4.0,0.0]]});
    let c = json!({"type":"LineString","coordinates":[[2.0,0.0],[6.0,0.0]]});
    assert!(b(&StOverlapsFunction, a.clone(), c.clone()));
    assert!(!b(&StCrossesFunction, a, c));
}

/// Mixed dimensions can never overlap, by definition.
#[test]
fn mixed_dimension_pairs_never_overlap() {
    assert!(!b(&StOverlapsFunction, pt(2.0, 2.0), square()));
    assert!(!b(&StOverlapsFunction, line(), square()));
    assert!(!b(&StOverlapsFunction, pt(2.0, 2.0), line()));
}
