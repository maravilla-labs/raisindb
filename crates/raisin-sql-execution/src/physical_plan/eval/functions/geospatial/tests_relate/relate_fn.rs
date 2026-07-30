//! `ST_RELATE`: the raw DE-9IM matrix and pattern matching.

use super::*;

#[test]
fn relate_returns_the_nine_character_matrix() {
    let f = StRelateFunction;
    // Two identical squares: interiors 2-D, boundaries 1-D, no exterior overlap.
    match f
        .evaluate(&[g(square()), g(square())], &Row::new())
        .unwrap()
    {
        Literal::Text(m) => {
            assert_eq!(m.len(), 9, "got {m}");
            assert_eq!(m, "2FFF1FFF2", "identical polygons: {m}");
        }
        other => panic!("expected Text, got {other:?}"),
    }

    // Two disjoint points.
    match f
        .evaluate(&[g(pt(0.0, 0.0)), g(pt(9.0, 9.0))], &Row::new())
        .unwrap()
    {
        Literal::Text(m) => assert_eq!(m, "FF0FFF0F2", "disjoint points: {m}"),
        other => panic!("expected Text, got {other:?}"),
    }
}

/// The matrix `ST_RELATE` reports must be the same matrix the named predicates
/// read, or the two surfaces would be able to disagree.
#[test]
fn relate_matrix_agrees_with_the_named_predicates() {
    let f = StRelateFunction;
    for (na, a) in every_type() {
        for (nb, bv) in every_type() {
            let m = match f
                .evaluate(&[g(a.clone()), g(bv.clone())], &Row::new())
                .unwrap()
            {
                Literal::Text(m) => m,
                other => panic!("expected Text, got {other:?}"),
            };
            // [FF*FF****] is the disjoint mask.
            let disjoint_by_pattern = match f
                .evaluate(
                    &[g(a.clone()), g(bv.clone()), text("FF*FF****")],
                    &Row::new(),
                )
                .unwrap()
            {
                Literal::Boolean(v) => v,
                other => panic!("expected Boolean, got {other:?}"),
            };
            assert_eq!(
                disjoint_by_pattern,
                b(&StDisjointFunction, a.clone(), bv.clone()),
                "({na}, {nb}) matrix {m} disagrees with ST_DISJOINT"
            );
        }
    }
}

#[test]
fn relate_pattern_matching() {
    let f = StRelateFunction;
    let row = Row::new();
    // T*F**F*** is the ST_WITHIN mask.
    let args = vec![g(pt(2.0, 2.0)), g(square()), text("T*F**F***")];
    assert_eq!(f.evaluate(&args, &row).unwrap(), Literal::Boolean(true));

    let args = vec![g(pt(9.0, 9.0)), g(square()), text("T*F**F***")];
    assert_eq!(f.evaluate(&args, &row).unwrap(), Literal::Boolean(false));
}

/// A malformed pattern is an error, never a silent FALSE.
///
/// `TTTTTTTTX` is the case that needs our own eager validation: `geo`'s
/// `IntersectionMatrix::matches` parses lazily and returns `Ok(false)` the moment
/// a position fails, so it never sees the invalid `X` and reports a plausible
/// negative. Verified against geo 0.33.1 — it really does accept that string.
#[test]
fn relate_rejects_a_malformed_pattern_loudly() {
    let f = StRelateFunction;
    let row = Row::new();
    for bad in [
        "TOOSHORT",
        "waytoolongpattern",
        "TTTTTTTTX",
        "*********X",
        "XXXXXXXXX",
        "",
    ] {
        let args = vec![g(square()), g(square()), text(bad)];
        assert!(
            f.evaluate(&args, &row).is_err(),
            "pattern {bad:?} must be an error, not a silent false"
        );
    }
}

/// Lowercase `t`/`f` are legal DE-9IM, and the pattern is checked before the
/// geometries are even parsed — a bad pattern is a query bug, so it must be
/// reported even when the data would have short-circuited to NULL.
#[test]
fn relate_accepts_every_legal_pattern_character() {
    let f = StRelateFunction;
    let row = Row::new();
    for good in [
        "t*f**f***",
        "T*F**F***",
        "*********",
        "2FFF1FFF2",
        "012*TF*tf",
    ] {
        let args = vec![g(square()), g(square()), text(good)];
        assert!(
            matches!(f.evaluate(&args, &row), Ok(Literal::Boolean(_))),
            "pattern {good:?} should be accepted"
        );
    }
    // Bad pattern beats NULL data.
    assert!(f.evaluate(&[null(), null(), text("nope")], &row).is_err());
}

#[test]
fn relate_rejects_wrong_arity() {
    let f = StRelateFunction;
    let row = Row::new();
    assert!(f.evaluate(&[g(square())], &row).is_err());
    assert!(f
        .evaluate(
            &[g(square()), g(square()), text("T*F**F***"), g(square())],
            &row
        )
        .is_err());
}
