//! Tests for fractional indexing operations.

use super::*;

#[test]
fn test_first() {
    let first = first();
    assert!(!first.is_empty());
}

#[test]
fn test_inc_ordering() {
    let a = first();
    let b = inc(&a).unwrap();
    let c = inc(&b).unwrap();

    assert!(a < b);
    assert!(b < c);
}

#[test]
fn test_prev_ordering() {
    let b = first();
    let a = prev(&b).unwrap();

    assert!(a < b);
}

#[test]
fn test_mid_ordering() {
    let a = first();
    let c = inc(&a).unwrap();
    let b = mid(&a, &c).unwrap();

    assert!(a < b);
    assert!(b < c);
}

#[test]
fn test_between_first() {
    let first = between(None, None).unwrap();
    assert!(!first.is_empty());
}

#[test]
fn test_between_append() {
    let a = first();
    let b = between(Some(&a), None).unwrap();

    assert!(a < b);
}

#[test]
fn test_between_prepend() {
    let b = first();
    let a = between(None, Some(&b)).unwrap();

    assert!(a < b);
}

#[test]
fn test_between_insert() {
    let a = first();
    let c = inc(&a).unwrap();
    let b = between(Some(&a), Some(&c)).unwrap();

    assert!(a < b);
    assert!(b < c);
}

#[test]
fn test_sequential_appends() {
    let mut labels = vec![first()];

    // Create 100 sequential appends
    for _ in 0..100 {
        let last = labels.last().unwrap();
        let next = inc(last).unwrap();
        labels.push(next);
    }

    // Verify they're all in order
    let mut sorted = labels.clone();
    sorted.sort();
    assert_eq!(sorted, labels);
}

#[test]
fn test_no_collisions() {
    use std::collections::HashSet;

    let mut labels = HashSet::new();
    let mut current = first();

    // Generate 1000 sequential labels
    for _ in 0..1000 {
        assert!(labels.insert(current.clone()), "Collision detected!");
        current = inc(&current).unwrap();
    }

    assert_eq!(labels.len(), 1000);
}

#[test]
fn test_lexicographic_ordering() {
    let labels = vec![
        first(),
        inc(&first()).unwrap(),
        inc(&inc(&first()).unwrap()).unwrap(),
    ];

    let mut sorted = labels.clone();
    sorted.sort();
    assert_eq!(sorted, labels);
}

#[test]
fn test_is_approaching_exhaustion() {
    let normal = first();
    assert!(!is_approaching_exhaustion(&normal));

    // Create a very long label (unlikely in practice)
    let long = "a".repeat(48);
    assert!(is_approaching_exhaustion(&long));
}

#[test]
fn test_mid_error_on_invalid_order() {
    let a = first();
    let b = inc(&a).unwrap();

    // Should error if a >= b
    assert!(mid(&b, &a).is_err());
    assert!(mid(&a, &a).is_err());
}

/// Format an order label exactly the way the production write paths do.
fn label(frac: &str, hlc: u128) -> String {
    format!("{}{}{:016x}", frac, super::SEPARATOR, hlc)
}

/// Build a broad sample of sibling fractional indices covering every shape the
/// production paths can mint: sequential appends, prepends, repeated midpoint
/// insertion (drag-and-drop reordering), and deep one-sided descent — the case
/// that drives labels longest and is most likely to produce a prefix pair.
fn realistic_fracs() -> Vec<String> {
    let mut fracs = vec![first()];

    // Sequential appends (the common create path).
    for _ in 0..8 {
        let last = fracs.last().unwrap().clone();
        fracs.push(inc(&last).unwrap());
    }
    // Prepends.
    let mut cur = first();
    for _ in 0..8 {
        cur = prev(&cur).unwrap();
        fracs.push(cur.clone());
    }
    // Repeated insertion between every adjacent pair.
    fracs.sort();
    for _ in 0..3 {
        let mut expanded = Vec::new();
        for pair in fracs.windows(2) {
            expanded.push(pair[0].clone());
            if let Ok(m) = mid(&pair[0], &pair[1]) {
                expanded.push(m);
            }
        }
        expanded.push(fracs.last().unwrap().clone());
        fracs = expanded;
    }
    // Deep one-sided descent, both directions: always re-insert immediately
    // next to the same bound so labels grow as long as possible.
    let hi = inc(&first()).unwrap();
    let mut lo = first();
    for _ in 0..12 {
        match mid(&lo, &hi) {
            Ok(m) => {
                fracs.push(m.clone());
                lo = m;
            }
            Err(_) => break,
        }
    }
    let lo = first();
    let mut hi = inc(&first()).unwrap();
    for _ in 0..12 {
        match mid(&lo, &hi) {
            Ok(m) => {
                fracs.push(m.clone());
                hi = m;
            }
            Err(_) => break,
        }
    }

    fracs.sort();
    fracs.dedup();
    fracs
}

/// The property that makes the `::` separator safe: the fractional encoding is
/// **prefix-free**, because the terminator byte (`0x80` → hex `"80"`) only ever
/// appears in final position. If this ever regresses, a shorter fractional part
/// could sort after a longer one extending it — see
/// [`full_label_order_matches_fractional_order`] for why that would corrupt
/// read order.
#[test]
fn fractional_encoding_is_prefix_free() {
    let fracs = realistic_fracs();
    assert!(
        fracs.len() > 50,
        "generator should produce a broad sample, got {}",
        fracs.len()
    );

    for a in &fracs {
        for b in &fracs {
            if a == b {
                continue;
            }
            assert!(
                !b.starts_with(a.as_str()),
                "fractional encoding must be prefix-free, but {a:?} is a prefix of {b:?}"
            );
        }
    }
}

/// The load-bearing invariant: `ORDERED_CHILDREN` sorts by the FULL label
/// (`{frac}::{hlc}`), while every ordering decision in the codebase is made on
/// `extract_fractional`. Those two orders must agree, or rows come back in a
/// different order than the one that was written.
///
/// This holds only because of [`fractional_encoding_is_prefix_free`]: distinct
/// labels always diverge inside the fractional part, so the comparison never
/// reaches the separator. It would break if the encoding gained prefix pairs,
/// since `':'` (0x3A) sorts AFTER `'0'..'9'` (0x30..0x39).
#[test]
fn full_label_order_matches_fractional_order() {
    let fracs = realistic_fracs();

    for a in &fracs {
        for b in &fracs {
            if a == b {
                continue;
            }
            // Deliberately opposed HLCs: the suffix must not be able to rescue
            // (or accidentally flip) the comparison.
            let la = label(a, 0x1111_1111_1111_1111);
            let lb = label(b, 0x2222_2222_2222_2222);

            assert_eq!(
                a.cmp(b),
                la.cmp(&lb),
                "full-label order disagrees with fractional order:\n  \
                 frac a = {a:?}\n  frac b = {b:?}\n  \
                 label a = {la:?}\n  label b = {lb:?}"
            );
        }
    }
}

/// Same-fractional collisions do happen (a branch merge copies labels
/// verbatim), and the HLC suffix must break the tie deterministically rather
/// than leaving two siblings incomparable.
#[test]
fn equal_fractionals_are_ordered_by_hlc_suffix() {
    let frac = first();
    let earlier = label(&frac, 0x0000_0000_0000_0001);
    let later = label(&frac, 0x0000_0000_0000_0002);

    assert!(earlier < later);
    assert_eq!(extract_fractional(&earlier), extract_fractional(&later));
}

#[test]
fn test_extract_fractional() {
    // Standard case: fractional::HLC
    assert_eq!(extract_fractional("a0b::1234567890abcdef"), "a0b");

    // No suffix: returns whole string
    assert_eq!(extract_fractional("a0b"), "a0b");

    // Empty string
    assert_eq!(extract_fractional(""), "");

    // Only separator
    assert_eq!(extract_fractional("::"), "");

    // Multiple separators: should use LAST one
    assert_eq!(extract_fractional("a::b::1234567890abcdef"), "a::b");
}
