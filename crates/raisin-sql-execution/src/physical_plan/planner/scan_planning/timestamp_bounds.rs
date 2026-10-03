//! Bounds on `created_at` / `updated_at` in the PROPERTY_INDEX's own unit.
//!
//! The index keys a timestamp by its MICROSECONDS (8-byte big-endian, written
//! by the timestamp writer and encoded for reads by the storage reader from a
//! `PropertyValue::Integer` of micros). The planner used to carry the bound as
//! a zero-padded NANOSECOND string and the executor converted it, which had
//! two faults:
//!
//! - a sub-microsecond bound was truncated, so `created_at >= T` with
//!   `T = m µs + 500 ns` included a node at exactly `m` µs — a row the SQL
//!   predicate rejects, and the predicate is CONSUMED by the scan, so nothing
//!   downstream removed it;
//! - the padded string compared lexicographically when two bounds were merged,
//!   which is wrong for negative (pre-1970) values.
//!
//! So the bound is converted here, once, to whole microseconds with the
//! rounding that keeps the predicate exact, and carried as decimal micros;
//! merging compares numerically. A literal integer is read as nanoseconds, as
//! it always was.

use std::cmp::Ordering;

/// The pseudo-properties whose index entries are microsecond timestamps.
pub(crate) fn is_timestamp_property(property_name: &str) -> bool {
    matches!(property_name, "__created_at" | "__updated_at")
}

/// A range bound of `nanos` as `(decimal micros, inclusive)`, rounded so that
/// exactly the microsecond values satisfying the original comparison pass.
///
/// `lower`: the bound of `>`/`>=`. A lower bound between two microseconds
/// rounds UP and becomes inclusive; an upper bound rounds DOWN and becomes
/// inclusive. A bound on a whole microsecond keeps its own inclusivity.
pub(crate) fn timestamp_bound(nanos: i128, lower: bool, inclusive: bool) -> (String, bool) {
    let floor = nanos.div_euclid(1000);
    let exact = nanos.rem_euclid(1000) == 0;
    let (micros, inclusive) = match (exact, lower) {
        (true, _) => (floor, inclusive),
        (false, true) => (floor + 1, true),
        (false, false) => (floor, true),
    };
    (micros.to_string(), inclusive)
}

/// `nanos` as decimal micros when it is a whole microsecond — the only values
/// an equality on a microsecond index can ever match.
pub(crate) fn exact_micros(nanos: i128) -> Option<String> {
    (nanos.rem_euclid(1000) == 0).then(|| nanos.div_euclid(1000).to_string())
}

/// Order two encoded bounds of `property_name`: numerically for timestamps,
/// as text (the index's own order for stored strings) otherwise.
pub(crate) fn cmp_bounds(property_name: &str, a: &str, b: &str) -> Ordering {
    if is_timestamp_property(property_name) {
        if let (Ok(a), Ok(b)) = (a.parse::<i64>(), b.parse::<i64>()) {
            return a.cmp(&b);
        }
    }
    a.cmp(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_microsecond_bounds_round_towards_the_predicate() {
        // >= 1500 ns: 1 µs (1000 ns) fails, 2 µs passes.
        assert_eq!(timestamp_bound(1500, true, true), ("2".into(), true));
        // > 1500 ns: same.
        assert_eq!(timestamp_bound(1500, true, false), ("2".into(), true));
        // <= 1500 ns: 1 µs passes, 2 µs fails.
        assert_eq!(timestamp_bound(1500, false, true), ("1".into(), true));
        // < 1500 ns: same.
        assert_eq!(timestamp_bound(1500, false, false), ("1".into(), true));
        // Whole microseconds keep their inclusivity.
        assert_eq!(timestamp_bound(2000, true, false), ("2".into(), false));
        assert_eq!(timestamp_bound(2000, false, false), ("2".into(), false));
        // Pre-1970.
        assert_eq!(timestamp_bound(-1500, true, true), ("-1".into(), true));
        assert_eq!(timestamp_bound(-1500, false, true), ("-2".into(), true));
    }

    #[test]
    fn timestamp_bounds_compare_numerically() {
        assert_eq!(cmp_bounds("__created_at", "-5", "3"), Ordering::Less);
        assert_eq!(cmp_bounds("__created_at", "10", "9"), Ordering::Greater);
        assert_eq!(cmp_bounds("sku", "10", "9"), Ordering::Less);
        assert_eq!(exact_micros(3000), Some("3".into()));
        assert_eq!(exact_micros(3001), None);
    }
}
