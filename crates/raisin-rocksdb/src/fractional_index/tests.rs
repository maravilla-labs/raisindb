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
