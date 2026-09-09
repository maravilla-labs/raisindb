//! Plan-time metadata for the scalar kernels: arity bounds and return type.
//!
//! A [`Kernel`](super::Kernel) is a value-level function — it validates its own
//! arity and produces a [`Literal`](crate::analyzer::Literal). The analyzer
//! needs the same two facts BEFORE any value exists: how many arguments the
//! call may carry, and what type the call has. That is what lives here.
//!
//! This table is the second half of one definition, not a second definition:
//! [`tests`](super::tests) asserts that every kernel and every alias has an
//! entry, so adding a kernel without describing it here fails the build's test
//! run rather than silently making the function unresolvable in SQL.

use super::{lookup, KernelCategory};
use crate::analyzer::types::DataType;

/// What the analyzer needs to know about a scalar call before it runs.
#[derive(Debug, Clone)]
pub struct KernelSignature {
    /// Canonical (non-alias) kernel name.
    pub name: &'static str,
    pub min_args: usize,
    pub max_args: usize,
    /// Same inputs always give the same output — safe to constant-fold.
    pub deterministic: bool,
    /// Which family the kernel belongs to.
    pub category: KernelCategory,
    /// Type of the call, given the argument types.
    pub return_type: DataType,
}

/// Arity bounds for a canonical kernel name, or `None` if unknown.
///
/// `usize::MAX` as the upper bound means variadic.
pub fn arity_of(canonical: &str) -> Option<(usize, usize)> {
    let bounds = match canonical {
        // Math
        "PI" | "RANDOM" => (0, 0),
        "ABS" | "CEIL" | "FLOOR" | "SIGN" | "SQRT" | "CBRT" | "EXP" | "LN" | "LOG10" => (1, 1),
        "TRUNC" | "ROUND" | "LOG" => (1, 2),
        "POWER" | "MOD" | "DIV" => (2, 2),
        "GREATEST" | "LEAST" => (1, usize::MAX),

        // String
        "LENGTH" | "LOWER" | "UPPER" | "INITCAP" | "REVERSE" | "ASCII" | "CHR" | "MD5"
        | "SHA256" | "TO_HEX" | "PG_TYPEOF" => (1, 1),
        "LEFT" | "RIGHT" | "REPEAT" | "STARTS_WITH" | "ENDS_WITH" | "STRPOS" => (2, 2),
        "REPLACE" | "SPLIT_PART" => (3, 3),
        "SUBSTR" => (2, 3),
        "BTRIM" | "LTRIM" | "RTRIM" => (1, 2),
        "LPAD" | "RPAD" => (2, 3),
        "CONCAT" | "FORMAT" => (1, usize::MAX),
        "CONCAT_WS" => (2, usize::MAX),

        // Regex
        "REGEXP_LIKE" | "REGEXP_MATCH" => (2, 3),
        "REGEXP_REPLACE" => (3, 4),

        // Temporal
        "CURRENT_TIMESTAMP" | "CURRENT_DATE" | "CURRENT_TIME" => (0, 0),
        "AGE" | "TO_TIMESTAMP" => (1, 2),
        "DATE_TRUNC" | "DATE_PART" | "TO_CHAR" | "TO_DATE" => (2, 2),
        "MAKE_DATE" => (3, 3),
        "MAKE_TIMESTAMP" => (6, 6),

        _ => return None,
    };
    Some(bounds)
}

/// The declared type of a call to `canonical` with these argument types.
fn return_type_of(canonical: &str, args: &[DataType]) -> DataType {
    match canonical {
        // Numeric functions that preserve an integral input.
        "ABS" | "SIGN" => numeric_like(args.first()),
        // Integer quotient / integer remainder of two integers.
        "DIV" => DataType::BigInt,
        "MOD" => {
            if args.iter().all(is_integral) {
                DataType::BigInt
            } else {
                DataType::Double
            }
        }
        // Everything else numeric is DOUBLE.
        "CEIL" | "FLOOR" | "TRUNC" | "ROUND" | "SQRT" | "CBRT" | "POWER" | "EXP" | "LN" | "LOG"
        | "LOG10" | "PI" | "RANDOM" | "DATE_PART" => DataType::Double,

        // Counting and locating are INT.
        "LENGTH" | "ASCII" | "STRPOS" => DataType::Int,

        // Predicates.
        "STARTS_WITH" | "ENDS_WITH" | "REGEXP_LIKE" => DataType::Boolean,

        // `REGEXP_MATCH` yields the capture groups as a JSON array (or NULL).
        "REGEXP_MATCH" => DataType::JsonB,

        // Instants. A DATE is a TIMESTAMPTZ at midnight UTC — see
        // `scalar::temporal`, which explains why there is no separate type.
        "CURRENT_TIMESTAMP" | "CURRENT_DATE" | "DATE_TRUNC" | "TO_TIMESTAMP" | "TO_DATE"
        | "MAKE_DATE" | "MAKE_TIMESTAMP" => DataType::TimestampTz,

        "AGE" => DataType::Interval,

        // GREATEST / LEAST are typed by `analyze_variadic_scalar` from the
        // common type of their arguments; this is only the fallback.
        "GREATEST" | "LEAST" => args
            .first()
            .map(|t| t.base_type().clone())
            .unwrap_or(DataType::Unknown),

        // Everything remaining is text: the string kernels, the hashes,
        // CURRENT_TIME, TO_CHAR, FORMAT, PG_TYPEOF.
        _ => DataType::Text,
    }
}

fn is_integral(t: &DataType) -> bool {
    matches!(t.base_type(), DataType::Int | DataType::BigInt)
}

/// INT stays INT, BIGINT stays BIGINT, anything else widens to DOUBLE —
/// mirroring what `ABS` and `SIGN` actually return at run time.
fn numeric_like(arg: Option<&DataType>) -> DataType {
    match arg.map(DataType::base_type) {
        Some(DataType::Int) => DataType::Int,
        Some(DataType::BigInt) => DataType::BigInt,
        _ => DataType::Double,
    }
}

/// Resolve `name(arg_types)` against the kernel table.
///
/// Returns `None` when no kernel answers to `name`, so callers fall through to
/// their own dispatch. Returns `Some(Err(_))` for a kernel called with the
/// wrong number of arguments — a plan-time error, not a per-row one.
pub fn resolve(name: &str, arg_types: &[DataType]) -> Option<Result<KernelSignature, String>> {
    let kernel = lookup(name)?;
    let (min_args, max_args) = arity_of(kernel.name)?;

    if arg_types.len() < min_args || arg_types.len() > max_args {
        let expected = if min_args == max_args {
            min_args.to_string()
        } else if max_args == usize::MAX {
            format!("at least {}", min_args)
        } else {
            format!("{} to {}", min_args, max_args)
        };
        return Some(Err(format!(
            "{} expects {} argument(s), got {}",
            kernel.name,
            expected,
            arg_types.len()
        )));
    }

    Some(Ok(KernelSignature {
        name: kernel.name,
        min_args,
        max_args,
        deterministic: kernel.deterministic,
        category: kernel.category,
        return_type: return_type_of(kernel.name, arg_types),
    }))
}
