//! Ordering between literals of compatible types, for GREATEST / LEAST.

use super::args::{describe, KernelError};
use crate::analyzer::Literal;
use std::cmp::Ordering;

/// Compare two non-NULL literals. Numbers compare numerically across INT /
/// BIGINT / DOUBLE; text against text; timestamps against timestamps or
/// parseable text; booleans as false < true. Anything else is an error.
pub(super) fn compare(name: &str, a: &Literal, b: &Literal) -> Result<Ordering, KernelError> {
    use Literal::*;
    let ord = match (a, b) {
        (Int(_) | BigInt(_) | Double(_), Int(_) | BigInt(_) | Double(_)) => {
            let (x, y) = (as_f64(a), as_f64(b));
            x.partial_cmp(&y).unwrap_or(Ordering::Equal)
        }
        (Text(x) | Path(x) | Uuid(x), Text(y) | Path(y) | Uuid(y)) => x.cmp(y),
        (Boolean(x), Boolean(y)) => x.cmp(y),
        (Timestamp(x), Timestamp(y)) => x.cmp(y),
        (Timestamp(x), Text(s)) => match super::temporal::parse_timestamp(s) {
            Some(y) => x.cmp(&y),
            None => return Err(format!("{}: '{}' is not a timestamp", name, s)),
        },
        (Text(s), Timestamp(y)) => match super::temporal::parse_timestamp(s) {
            Some(x) => x.cmp(y),
            None => return Err(format!("{}: '{}' is not a timestamp", name, s)),
        },
        (Interval(x), Interval(y)) => x.cmp(y),
        _ => {
            return Err(format!(
                "{}: cannot compare {} with {}",
                name,
                describe(a),
                describe(b)
            ))
        }
    };
    Ok(ord)
}

fn as_f64(lit: &Literal) -> f64 {
    match lit {
        Literal::Int(v) => *v as f64,
        Literal::BigInt(v) => *v as f64,
        Literal::Double(f) => *f,
        _ => f64::NAN,
    }
}

/// GREATEST / LEAST: NULLs are ignored (PostgreSQL); all-NULL yields NULL.
pub(super) fn extreme(
    name: &str,
    args: &[Literal],
    want: Ordering,
) -> Result<Literal, KernelError> {
    if args.is_empty() {
        return Err(format!("{} expects at least 1 argument", name));
    }
    let mut best: Option<&Literal> = None;
    for arg in args {
        if matches!(arg, Literal::Null) {
            continue;
        }
        best = Some(match best {
            None => arg,
            Some(current) => {
                if compare(name, arg, current)? == want {
                    arg
                } else {
                    current
                }
            }
        });
    }
    Ok(best.cloned().unwrap_or(Literal::Null))
}
