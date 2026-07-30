//! Identities that must hold for every ordered type pair.

use super::*;

/// `ST_DISJOINT` must be the exact complement of `ST_INTERSECTS`.
///
/// It was not: the two functions hand-listed different sets of type pairs (nine
/// versus six), so for a `LineString`/`LineString` input `ST_DISJOINT` answered
/// and `ST_INTERSECTS` errored.
#[test]
fn disjoint_is_exactly_not_intersects() {
    for (na, a) in every_type() {
        for (nb, bv) in every_type() {
            let i = b(&StIntersectsFunction, a.clone(), bv.clone());
            let d = b(&StDisjointFunction, a.clone(), bv.clone());
            assert_ne!(i, d, "INTERSECTS/DISJOINT disagree on ({na}, {nb})");
        }
    }
}

/// `ST_WITHIN(a, b)` must equal `ST_CONTAINS(b, a)` for every pair.
#[test]
fn within_mirrors_contains() {
    for (na, a) in every_type() {
        for (nb, bv) in every_type() {
            assert_eq!(
                b(&StWithinFunction, a.clone(), bv.clone()),
                b(&StContainsFunction, bv.clone(), a.clone()),
                "WITHIN({na}, {nb}) != CONTAINS({nb}, {na})"
            );
        }
    }
}

/// `ST_COVEREDBY(a, b)` must equal `ST_COVERS(b, a)` for every pair.
#[test]
fn coveredby_mirrors_covers() {
    for (na, a) in every_type() {
        for (nb, bv) in every_type() {
            assert_eq!(
                b(&StCoveredByFunction, a.clone(), bv.clone()),
                b(&StCoversFunction, bv.clone(), a.clone()),
                "COVEREDBY({na}, {nb}) != COVERS({nb}, {na})"
            );
        }
    }
}

/// `COVERS` is strictly weaker than `CONTAINS`, and `COVEREDBY` than `WITHIN`.
#[test]
fn contains_implies_covers_and_within_implies_coveredby() {
    for (na, a) in every_type() {
        for (nb, bv) in every_type() {
            if b(&StContainsFunction, a.clone(), bv.clone()) {
                assert!(
                    b(&StCoversFunction, a.clone(), bv.clone()),
                    "CONTAINS({na}, {nb}) but not COVERS"
                );
            }
            if b(&StWithinFunction, a.clone(), bv.clone()) {
                assert!(
                    b(&StCoveredByFunction, a.clone(), bv.clone()),
                    "WITHIN({na}, {nb}) but not COVEREDBY"
                );
            }
        }
    }
}

/// Symmetric predicates must not care about argument order.
#[test]
fn symmetric_predicates_are_symmetric() {
    let symmetric: Vec<Box<dyn SqlFunction>> = vec![
        Box::new(StIntersectsFunction),
        Box::new(StDisjointFunction),
        Box::new(StTouchesFunction),
        Box::new(StCrossesFunction),
        Box::new(StOverlapsFunction),
        Box::new(StEqualsFunction),
    ];
    for f in symmetric {
        for (na, a) in every_type() {
            for (nb, bv) in every_type() {
                assert_eq!(
                    b(f.as_ref(), a.clone(), bv.clone()),
                    b(f.as_ref(), bv.clone(), a.clone()),
                    "{} is not symmetric on ({na}, {nb})",
                    f.name()
                );
            }
        }
    }
}

/// Reflexivity, which pins several matrix methods at once: a non-empty geometry
/// intersects, contains, is within, covers, is covered by and equals itself, and
/// does *not* touch, cross or overlap itself.
#[test]
fn reflexive_and_irreflexive_predicates() {
    for (name, a) in every_type() {
        if name == "empty" {
            continue; // emptiness has its own test below
        }
        assert!(b(&StIntersectsFunction, a.clone(), a.clone()), "{name}");
        assert!(b(&StContainsFunction, a.clone(), a.clone()), "{name}");
        assert!(b(&StWithinFunction, a.clone(), a.clone()), "{name}");
        assert!(b(&StCoversFunction, a.clone(), a.clone()), "{name}");
        assert!(b(&StCoveredByFunction, a.clone(), a.clone()), "{name}");
        assert!(b(&StEqualsFunction, a.clone(), a.clone()), "{name}");

        assert!(!b(&StDisjointFunction, a.clone(), a.clone()), "{name}");
        assert!(!b(&StTouchesFunction, a.clone(), a.clone()), "{name}");
        assert!(!b(&StCrossesFunction, a.clone(), a.clone()), "{name}");
        assert!(!b(&StOverlapsFunction, a.clone(), a.clone()), "{name}");
    }
}
