//! Argument extraction helpers shared by every kernel.
//!
//! Each helper names the function and the argument position in its error so
//! a user sees `SUBSTR: argument 2 must be an integer, got 'abc'` rather than
//! a bare type name.

use crate::analyzer::Literal;
use chrono::{DateTime, Utc};

/// A kernel error is a message; the execution layer wraps it in its own type.
pub type KernelError = String;

/// Reject a call whose argument count is outside `[min, max]`.
pub(super) fn arity(
    name: &str,
    args: &[Literal],
    min: usize,
    max: usize,
) -> Result<(), KernelError> {
    if args.len() < min || args.len() > max {
        let expected = if min == max {
            format!("{}", min)
        } else {
            format!("{} to {}", min, max)
        };
        return Err(format!(
            "{} expects {} argument(s), got {}",
            name,
            expected,
            args.len()
        ));
    }
    Ok(())
}

/// Text-like literal as `&str`. Paths and UUIDs are text for string purposes.
pub(super) fn text<'a>(name: &str, args: &'a [Literal], i: usize) -> Result<&'a str, KernelError> {
    match &args[i] {
        Literal::Text(s) | Literal::Path(s) | Literal::Uuid(s) => Ok(s.as_str()),
        other => Err(format!(
            "{}: argument {} must be text, got {}",
            name,
            i + 1,
            describe(other)
        )),
    }
}

/// Any literal rendered as text, the way `||` and CONCAT do it.
pub(super) fn to_text(lit: &Literal) -> String {
    match lit {
        Literal::Null => String::new(),
        Literal::Boolean(b) => b.to_string(),
        Literal::Int(i) => i.to_string(),
        Literal::BigInt(i) => i.to_string(),
        Literal::Double(f) => f.to_string(),
        Literal::Text(s) | Literal::Uuid(s) | Literal::Path(s) | Literal::Parameter(s) => s.clone(),
        Literal::JsonB(v) | Literal::Geometry(v) => v.to_string(),
        Literal::Vector(v) => format!("{:?}", v),
        Literal::Timestamp(ts) => ts.to_rfc3339(),
        Literal::Interval(d) => super::temporal::format_interval(d),
    }
}

/// Integer argument (INT or BIGINT; a whole-valued DOUBLE is accepted too).
pub(super) fn int(name: &str, args: &[Literal], i: usize) -> Result<i64, KernelError> {
    match &args[i] {
        Literal::Int(v) => Ok(*v as i64),
        Literal::BigInt(v) => Ok(*v),
        Literal::Double(f) if f.fract() == 0.0 => Ok(*f as i64),
        Literal::Text(s) => s.trim().parse::<i64>().map_err(|_| {
            format!(
                "{}: argument {} must be an integer, got '{}'",
                name,
                i + 1,
                s
            )
        }),
        other => Err(format!(
            "{}: argument {} must be an integer, got {}",
            name,
            i + 1,
            describe(other)
        )),
    }
}

/// Numeric argument widened to f64.
pub(super) fn num(name: &str, args: &[Literal], i: usize) -> Result<f64, KernelError> {
    match &args[i] {
        Literal::Int(v) => Ok(*v as f64),
        Literal::BigInt(v) => Ok(*v as f64),
        Literal::Double(f) => Ok(*f),
        Literal::Text(s) => s
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("{}: argument {} must be numeric, got '{}'", name, i + 1, s)),
        other => Err(format!(
            "{}: argument {} must be numeric, got {}",
            name,
            i + 1,
            describe(other)
        )),
    }
}

/// Timestamp argument: a TIMESTAMPTZ literal or text in any accepted format.
pub(super) fn timestamp(
    name: &str,
    args: &[Literal],
    i: usize,
) -> Result<DateTime<Utc>, KernelError> {
    match &args[i] {
        Literal::Timestamp(ts) => Ok(*ts),
        Literal::Text(s) => super::temporal::parse_timestamp(s).ok_or_else(|| {
            format!(
                "{}: argument {} is not a recognised timestamp: '{}'",
                name,
                i + 1,
                s
            )
        }),
        other => Err(format!(
            "{}: argument {} must be a timestamp, got {}",
            name,
            i + 1,
            describe(other)
        )),
    }
}

/// Short type name for error messages (`typeof` uses the PostgreSQL spelling).
pub(super) fn describe(lit: &Literal) -> &'static str {
    match lit {
        Literal::Null => "NULL",
        Literal::Boolean(_) => "boolean",
        Literal::Int(_) => "integer",
        Literal::BigInt(_) => "bigint",
        Literal::Double(_) => "double precision",
        Literal::Text(_) => "text",
        Literal::Uuid(_) => "uuid",
        Literal::Path(_) => "path",
        Literal::JsonB(_) => "jsonb",
        Literal::Vector(_) => "vector",
        Literal::Geometry(_) => "geometry",
        Literal::Timestamp(_) => "timestamp with time zone",
        Literal::Interval(_) => "interval",
        Literal::Parameter(_) => "parameter",
    }
}

/// Convert a 1-based SQL character position with PostgreSQL's clamping rules
/// into a `(start, len)` byte-agnostic char range for SUBSTR/SUBSTRING.
///
/// `SUBSTR('hello', 0, 3)` is `'he'` in PostgreSQL: the window starts before
/// the string and only the overlapping part is returned. A negative length is
/// an error, as it is there.
pub(super) fn char_window(
    name: &str,
    total: usize,
    start: i64,
    len: Option<i64>,
) -> Result<(usize, usize), KernelError> {
    if let Some(l) = len {
        if l < 0 {
            return Err(format!("{}: negative substring length not allowed", name));
        }
    }
    let end_excl = match len {
        Some(l) => start.saturating_add(l),
        None => i64::MAX,
    };
    let begin = start.max(1);
    if end_excl <= begin {
        return Ok((0, 0));
    }
    let begin0 = (begin - 1) as usize;
    if begin0 >= total {
        return Ok((0, 0));
    }
    let count = ((end_excl - begin) as usize).min(total - begin0);
    Ok((begin0, count))
}
