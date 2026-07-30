//! Type-pair coverage — the headline claim.

use super::*;

/// The requirement in one test: **every** predicate answers **every** ordered
/// pair of geometry types with a boolean.
///
/// This is the test the old implementation could not pass. `ST_CONTAINS`
/// supported two pairs out of forty-nine and returned
/// `Error::Validation("not supported for ...")` for the rest; `ST_OVERLAPS`
/// returned a silent `false`, which is worse.
#[test]
fn every_predicate_answers_every_ordered_type_pair() {
    let row = Row::new();
    for f in all_predicates() {
        for (name_a, a) in every_type() {
            for (name_b, bv) in every_type() {
                let result = f.evaluate(&[g(a.clone()), g(bv.clone())], &row);
                match result {
                    Ok(Literal::Boolean(_)) => {}
                    other => panic!(
                        "{}({name_a}, {name_b}) must yield a boolean, got {other:?}",
                        f.name()
                    ),
                }
            }
        }
    }
}

/// The specific pairs the audit named as broken, with the answers they must now
/// give. Each line is a bug that used to be an error or a wrong boolean.
#[test]
fn previously_unsupported_pairs_now_give_the_right_answer() {
    // ST_INTERSECTS had no Point/LineString case at all (it errored).
    assert!(b(&StIntersectsFunction, pt(2.0, 2.0), line()));
    assert!(!b(&StIntersectsFunction, pt(2.0, 3.0), line()));

    // ST_INTERSECTS had no LineString/LineString case (it errored).
    assert!(b(&StIntersectsFunction, line(), multiline()));

    // ST_CONTAINS was Polygon/Point and Polygon/Polygon only. A polygon
    // containing a line is one of the commonest real queries.
    let inner_line = json!({"type":"LineString","coordinates":[[1.0,1.0],[3.0,3.0]]});
    assert!(b(&StContainsFunction, square(), inner_line.clone()));
    // The square's diagonal is contained too: its endpoints sit on the square's
    // boundary, and CONTAINS forbids points in the *exterior*, not on the edge.
    assert!(b(&StContainsFunction, square(), line()));

    // ST_WITHIN lacked LineString entirely, and its doc comment claimed
    // point-in-polygon only.
    assert!(b(&StWithinFunction, inner_line, square()));

    // ST_CROSSES hardcoded false for any Point argument. A MultiPoint straddling
    // a polygon boundary genuinely crosses it.
    let straddling = json!({"type":"MultiPoint","coordinates":[[2.0,2.0],[9.0,9.0]]});
    assert!(
        b(&StCrossesFunction, straddling, square()),
        "some points inside, some outside: that is a crossing"
    );

    // ST_TOUCHES hardcoded false for any Point argument. A point on a line's
    // endpoint touches it.
    assert!(b(&StTouchesFunction, pt(0.0, 0.0), line()));
    assert!(
        !b(&StTouchesFunction, pt(2.0, 2.0), line()),
        "a point in the line's interior intersects but does not touch"
    );

    // ST_OVERLAPS silently returned false for every pair it did not name.
    let shifted = json!({"type":"MultiPolygon","coordinates":[
        [[[2.0,2.0],[6.0,2.0],[6.0,6.0],[2.0,6.0],[2.0,2.0]]]
    ]});
    assert!(
        b(&StOverlapsFunction, multipolygon(), shifted),
        "two MultiPolygons sharing part of their area overlap"
    );
}
