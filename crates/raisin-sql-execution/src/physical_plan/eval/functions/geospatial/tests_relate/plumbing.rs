//! NULL, arity, SRID and malformed-input plumbing.

use super::*;

#[test]
fn null_propagates_from_either_side() {
    let row = Row::new();
    for f in all_predicates() {
        assert_eq!(
            f.evaluate(&[null(), g(square())], &row).unwrap(),
            Literal::Null,
            "{}",
            f.name()
        );
        assert_eq!(
            f.evaluate(&[g(square()), null()], &row).unwrap(),
            Literal::Null,
            "{}",
            f.name()
        );
        assert_eq!(
            f.evaluate(&[null(), null()], &row).unwrap(),
            Literal::Null,
            "{}",
            f.name()
        );
    }
}

/// A NULL argument wins over a mistyped one, matching SQL's usual treatment and
/// the behaviour these functions had before.
#[test]
fn null_wins_over_a_mistyped_argument() {
    let row = Row::new();
    let bad = TypedExpr::new(Expr::Literal(Literal::Int(7)), DataType::Int);
    assert_eq!(
        StIntersectsFunction.evaluate(&[bad, null()], &row).unwrap(),
        Literal::Null
    );
}

#[test]
fn wrong_arity_is_rejected_by_every_predicate() {
    let row = Row::new();
    for f in all_predicates() {
        assert!(
            f.evaluate(&[g(square())], &row).is_err(),
            "{} accepted 1 argument",
            f.name()
        );
        assert!(
            f.evaluate(&[g(square()), g(square()), g(square())], &row)
                .is_err(),
            "{} accepted 3 arguments",
            f.name()
        );
    }
}

#[test]
fn a_non_geometry_argument_names_the_function() {
    let row = Row::new();
    let bad = TypedExpr::new(Expr::Literal(Literal::Int(7)), DataType::Int);
    let err = StOverlapsFunction
        .evaluate(&[bad, g(square())], &row)
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("ST_OVERLAPS"), "{msg}");
}

/// Malformed GeoJSON is an error, never a boolean. A predicate that answered
/// `false` for unparseable input would be indistinguishable from a real answer.
#[test]
fn malformed_geojson_is_an_error() {
    let row = Row::new();
    for junk in [
        json!({"type":"Nope","coordinates":[1,2]}),
        json!({"type":"Point","coordinates":[]}),
        json!({"type":"Point"}),
        json!({"type":"Point","coordinates":["x","y"]}),
    ] {
        assert!(
            StIntersectsFunction
                .evaluate(&[g(junk.clone()), g(square())], &row)
                .is_err(),
            "{junk} should be rejected"
        );
    }
}

/// A geometry carrying no `srid` adopts the other operand's, which is what keeps
/// existing queries working when one side is a stored labelled value and the
/// other is a literal built by `ST_POINT`.
#[test]
fn an_unlabelled_operand_adopts_the_other_srid() {
    let labelled = json!({"type":"Polygon","srid":32632,"coordinates":[[
        [0.0,0.0],[4.0,0.0],[4.0,4.0],[0.0,4.0],[0.0,0.0]]]});
    let bare = pt(2.0, 2.0);
    assert!(b(&StContainsFunction, labelled, bare));
}

/// Two *different* explicit SRIDs are a hard error telling the user to transform,
/// not a silently-planar comparison of incompatible coordinates.
#[test]
fn mismatched_explicit_srids_error_with_a_fix() {
    let row = Row::new();
    let a = json!({"type":"Point","coordinates":[8.54,47.37],"srid":4326});
    let c = json!({"type":"Point","coordinates":[950_000.0,6_000_000.0],"srid":3857});
    for f in all_predicates() {
        let err = f.evaluate(&[g(a.clone()), g(c.clone())], &row).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(f.name()), "{msg}");
        assert!(
            msg.contains("ST_TRANSFORM"),
            "must say how to fix it: {msg}"
        );
    }
}

/// Altitude is ignored by the 2-D predicates, exactly as in PostGIS. A 3-D point
/// vertically far above a polygon still lies inside it in plan.
#[test]
fn altitude_is_ignored_by_the_planar_predicates() {
    let high = json!({"type":"Point","coordinates":[2.0,2.0,4000.0]});
    assert!(b(&StContainsFunction, square(), high));
}

/// A `TEXT` argument holding GeoJSON is accepted, so `ST_ASGEOJSON` output and
/// plain string literals compose; unparseable text is an error.
#[test]
fn text_arguments_are_parsed_as_geojson() {
    let row = Row::new();
    let as_text = text(&square().to_string());
    assert_eq!(
        StContainsFunction
            .evaluate(&[as_text, g(pt(2.0, 2.0))], &row)
            .unwrap(),
        Literal::Boolean(true)
    );
    assert!(StContainsFunction
        .evaluate(&[text("not json"), g(pt(2.0, 2.0))], &row)
        .is_err());
}
