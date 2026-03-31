//! Literal comparison and type conversion utilities
//!
//! Provides ordering for SQL literal values and conversion
//! from literals to PropertyValues for output rows.

use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::types::to_property_value;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::Literal;
use std::cmp::Ordering;

/// Compare two literals for ordering
///
/// Returns Less, Equal, or Greater.
/// NULL is considered less than any other value.
pub(crate) fn compare_literals(a: &Literal, b: &Literal) -> Ordering {
    use Literal::*;

    match (a, b) {
        // NULL comparisons
        (Null, Null) => Ordering::Equal,
        (Null, _) => Ordering::Less,
        (_, Null) => Ordering::Greater,

        // Boolean
        (Boolean(a), Boolean(b)) => a.cmp(b),

        // Numeric comparisons
        (Int(a), Int(b)) => a.cmp(b),
        (BigInt(a), BigInt(b)) => a.cmp(b),
        (Double(a), Double(b)) => {
            if a < b {
                Ordering::Less
            } else if a > b {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }

        // Cross-numeric comparisons
        (Int(a), BigInt(b)) => (*a as i64).cmp(b),
        (BigInt(a), Int(b)) => a.cmp(&(*b as i64)),
        (Int(a), Double(b)) => compare_f64(*a as f64, *b),
        (BigInt(a), Double(b)) => compare_f64(*a as f64, *b),
        (Double(a), Int(b)) => compare_f64(*a, *b as f64),
        (Double(a), BigInt(b)) => compare_f64(*a, *b as f64),

        // String comparisons
        (Text(a), Text(b)) => a.cmp(b),
        (Path(a), Path(b)) => a.cmp(b),
        (Uuid(a), Uuid(b)) => a.cmp(b),

        // JSON comparison (by string representation)
        (JsonB(a), JsonB(b)) => a.to_string().cmp(&b.to_string()),

        // Vector comparison (lexicographic)
        (Vector(a), Vector(b)) => {
            for (av, bv) in a.iter().zip(b.iter()) {
                if av < bv {
                    return Ordering::Less;
                } else if av > bv {
                    return Ordering::Greater;
                }
            }
            a.len().cmp(&b.len())
        }

        // Incompatible types: order by type tag
        _ => {
            let a_tag = discriminant_value(a);
            let b_tag = discriminant_value(b);
            a_tag.cmp(&b_tag)
        }
    }
}

/// Compare two f64 values with total ordering
fn compare_f64(a: f64, b: f64) -> Ordering {
    if a < b {
        Ordering::Less
    } else if a > b {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

/// Get a discriminant value for ordering literals of different types
fn discriminant_value(lit: &Literal) -> u8 {
    match lit {
        Literal::Null => 0,
        Literal::Boolean(_) => 1,
        Literal::Int(_) => 2,
        Literal::BigInt(_) => 3,
        Literal::Double(_) => 4,
        Literal::Text(_) => 5,
        Literal::Uuid(_) => 6,
        Literal::Path(_) => 7,
        Literal::JsonB(_) => 8,
        Literal::Vector(_) => 9,
        Literal::Geometry(_) => 10,
        Literal::Timestamp(_) => 11,
        Literal::Interval(_) => 12,
        Literal::Parameter(_) => 13,
    }
}

/// Convert a literal to PropertyValue
pub(crate) fn literal_to_property_value(lit: Literal) -> Result<PropertyValue, ExecutionError> {
    to_property_value(&lit).map_err(ExecutionError::Backend)
}
