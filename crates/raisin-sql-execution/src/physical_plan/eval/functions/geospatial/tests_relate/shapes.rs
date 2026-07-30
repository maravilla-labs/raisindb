//! Interior rings, Multi*, collections, empties, and topological equality.

use super::*;

/// A hole is really a hole: the polygon with a hole does not contain a point in
/// the hole, and the un-holed one does.
#[test]
fn interior_rings_are_honoured() {
    let in_hole = pt(2.0, 2.0);
    assert!(b(&StContainsFunction, square(), in_hole.clone()));
    assert!(
        !b(&StContainsFunction, donut(), in_hole.clone()),
        "the point sits in the hole"
    );
    assert!(!b(&StCoversFunction, donut(), in_hole.clone()));
    assert!(b(&StDisjointFunction, donut(), in_hole));

    // A point in the ring between the two boundaries IS contained.
    assert!(b(&StContainsFunction, donut(), pt(0.5, 0.5)));

    // A point exactly on the hole's boundary is covered but not contained.
    let on_hole_edge = pt(1.0, 2.0);
    assert!(!b(&StContainsFunction, donut(), on_hole_edge.clone()));
    assert!(b(&StCoversFunction, donut(), on_hole_edge));

    // The donut is within the solid square, and the square is not within it.
    assert!(b(&StCoveredByFunction, donut(), square()));
    assert!(!b(&StCoveredByFunction, square(), donut()));
}

/// `MultiPolygon` semantics: containment must consider every member.
#[test]
fn multipolygon_members_are_all_considered() {
    // A point in the *second* member is contained by the MultiPolygon.
    assert!(b(&StContainsFunction, multipolygon(), pt(12.0, 12.0)));
    // And one between the two members is not.
    assert!(b(&StDisjointFunction, multipolygon(), pt(7.0, 7.0)));
    // The MultiPolygon covers its first member.
    assert!(b(&StCoversFunction, multipolygon(), square()));
    assert!(!b(&StCoversFunction, square(), multipolygon()));
}

/// A `GeometryCollection` relates as the union of its members, including when
/// nested — which no previous code path could do at all.
#[test]
fn geometry_collections_including_nested_ones() {
    // collection() is a Point at (8,8) plus a line 0,0 -> 4,4. Both members
    // count, and nothing else does.
    assert!(b(&StIntersectsFunction, collection(), pt(8.0, 8.0)));
    assert!(b(&StIntersectsFunction, collection(), pt(3.0, 3.0)));
    assert!(b(&StDisjointFunction, collection(), pt(0.0, 4.0)));

    let nested = json!({"type":"GeometryCollection","geometries":[
        {"type":"GeometryCollection","geometries":[
            {"type":"Point","coordinates":[12.0,12.0]}
        ]},
        {"type":"Polygon","coordinates":[[
            [0.0,0.0],[4.0,0.0],[4.0,4.0],[0.0,4.0],[0.0,0.0]]]}
    ]});
    assert!(b(&StIntersectsFunction, nested.clone(), pt(12.0, 12.0)));
    assert!(b(&StIntersectsFunction, nested.clone(), pt(2.0, 2.0)));
    assert!(b(&StDisjointFunction, nested, pt(-1.0, -1.0)));
}

/// Emptiness propagates as a boolean, never as an error. These are the
/// JTS/PostGIS answers.
#[test]
fn empty_geometries_follow_jts_semantics() {
    for (name, other) in every_type() {
        // Nothing intersects an empty geometry, so everything is disjoint from it.
        assert!(
            b(&StDisjointFunction, empty(), other.clone()),
            "empty vs {name} must be disjoint"
        );
        assert!(!b(&StIntersectsFunction, empty(), other.clone()));
        assert!(!b(&StCoversFunction, other.clone(), empty()));
        assert!(!b(&StTouchesFunction, empty(), other.clone()));
        assert!(!b(&StOverlapsFunction, empty(), other.clone()));
        assert!(!b(&StCrossesFunction, empty(), other.clone()));
    }
    // Two empty geometries are topologically equal, and only those.
    assert!(b(&StEqualsFunction, empty(), empty()));
    assert!(!b(&StEqualsFunction, empty(), point()));
}

// ===========================================================================
// 5. Topological (not structural) equality
// ===========================================================================

/// `ST_EQUALS` compares point sets, not vertex arrays. Each of these was FALSE
/// under the old array-comparing implementation.
#[test]
fn equality_is_topological() {
    // A redundant collinear vertex changes nothing about the point set.
    let three = json!({"type":"LineString","coordinates":[[0.0,0.0],[1.0,1.0],[2.0,2.0]]});
    let two = json!({"type":"LineString","coordinates":[[0.0,0.0],[2.0,2.0]]});
    assert!(b(&StEqualsFunction, three, two));

    // A rotated ring is the same ring.
    let rotated = json!({"type":"Polygon","coordinates":[[
        [4.0,0.0],[4.0,4.0],[0.0,4.0],[0.0,0.0],[4.0,0.0]]]});
    assert!(b(&StEqualsFunction, square(), rotated));

    // Reversed winding is the same polygon.
    let reversed = json!({"type":"Polygon","coordinates":[[
        [0.0,0.0],[0.0,4.0],[4.0,4.0],[4.0,0.0],[0.0,0.0]]]});
    assert!(b(&StEqualsFunction, square(), reversed));

    // Differing type names do not preclude equality.
    let single = json!({"type":"MultiPoint","coordinates":[[1.0,1.0]]});
    assert!(b(&StEqualsFunction, point(), single));

    let single_poly = json!({"type":"MultiPolygon","coordinates":[[
        [[0.0,0.0],[4.0,0.0],[4.0,4.0],[0.0,4.0],[0.0,0.0]]]]});
    assert!(b(&StEqualsFunction, square(), single_poly));
}

/// Equality is exact — no coordinate epsilon.
///
/// The old implementation absorbed up to `1e-8` degrees (≈1.1 mm), reporting two
/// distinct locations as equal. That also made `ST_EQUALS` disagree with
/// `ST_COVERS`/`ST_COVEREDBY`, which have no such tolerance, so the identity
/// "equal iff each covers the other" did not hold.
#[test]
fn equality_has_no_coordinate_tolerance() {
    let a = pt(-122.4194, 37.7749);
    let drifted = pt(-122.4194000009, 37.7749000009);
    assert!(!b(&StEqualsFunction, a.clone(), drifted.clone()));
    // And it agrees with the covers-based definition.
    let mutual =
        b(&StCoversFunction, a.clone(), drifted.clone()) && b(&StCoveredByFunction, a, drifted);
    assert!(!mutual);
}
