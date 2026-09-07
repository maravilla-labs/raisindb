//! Math kernels: ABS, CEIL, FLOOR, TRUNC, SIGN, SQRT, CBRT, POWER, EXP, LN,
//! LOG, LOG10, MOD, DIV, PI, RANDOM, ROUND, GREATEST, LEAST.
//!
//! Integer inputs keep their integer type where PostgreSQL does (ABS, SIGN,
//! MOD, DIV); everything else returns DOUBLE.

use super::args::{arity, int, num, KernelError};
use super::compare::extreme;
use super::{Kernel, KernelCategory, KernelResult};
use crate::analyzer::Literal;
use std::cmp::Ordering;

macro_rules! kernel {
    ($name:expr, $aliases:expr, $sig:expr, $f:expr) => {
        Kernel {
            name: $name,
            aliases: $aliases,
            category: KernelCategory::Math,
            signature: $sig,
            deterministic: true,
            strict: true,
            func: $f,
        }
    };
}

pub(super) static KERNELS: &[Kernel] = &[
    kernel!("ABS", &[], "ABS(numeric) -> numeric", abs),
    kernel!("CEIL", &["CEILING"], "CEIL(numeric) -> DOUBLE", ceil),
    kernel!("FLOOR", &[], "FLOOR(numeric) -> DOUBLE", floor),
    kernel!("TRUNC", &[], "TRUNC(numeric[, scale]) -> DOUBLE", trunc),
    kernel!("SIGN", &[], "SIGN(numeric) -> numeric", sign),
    kernel!("SQRT", &[], "SQRT(numeric) -> DOUBLE", sqrt),
    kernel!("CBRT", &[], "CBRT(numeric) -> DOUBLE", cbrt),
    kernel!("POWER", &["POW"], "POWER(base, exponent) -> DOUBLE", power),
    kernel!("EXP", &[], "EXP(numeric) -> DOUBLE", exp),
    kernel!("LN", &[], "LN(numeric) -> DOUBLE", ln),
    kernel!("LOG", &[], "LOG([base,] numeric) -> DOUBLE", log),
    kernel!("LOG10", &[], "LOG10(numeric) -> DOUBLE", log10),
    kernel!("MOD", &[], "MOD(a, b) -> numeric", modulo),
    kernel!("DIV", &[], "DIV(a, b) -> BIGINT", div),
    kernel!("ROUND", &[], "ROUND(numeric[, scale]) -> DOUBLE", round),
    Kernel {
        name: "PI",
        aliases: &[],
        category: KernelCategory::Math,
        signature: "PI() -> DOUBLE",
        deterministic: true,
        strict: true,
        func: pi,
    },
    Kernel {
        name: "RANDOM",
        aliases: &[],
        category: KernelCategory::Math,
        signature: "RANDOM() -> DOUBLE",
        deterministic: false,
        strict: true,
        func: random,
    },
    Kernel {
        name: "GREATEST",
        aliases: &[],
        category: KernelCategory::Math,
        signature: "GREATEST(a, b, ...) -> a",
        deterministic: true,
        strict: false,
        func: |args| extreme("GREATEST", args, Ordering::Greater),
    },
    Kernel {
        name: "LEAST",
        aliases: &[],
        category: KernelCategory::Math,
        signature: "LEAST(a, b, ...) -> a",
        deterministic: true,
        strict: false,
        func: |args| extreme("LEAST", args, Ordering::Less),
    },
];

fn finite(name: &str, v: f64) -> KernelResult {
    if v.is_finite() {
        Ok(Literal::Double(v))
    } else {
        Err(format!("{}: result is out of range", name))
    }
}

fn abs(args: &[Literal]) -> KernelResult {
    arity("ABS", args, 1, 1)?;
    match &args[0] {
        Literal::Int(v) => Ok(Literal::Int(v.wrapping_abs())),
        Literal::BigInt(v) => Ok(Literal::BigInt(v.wrapping_abs())),
        _ => Ok(Literal::Double(num("ABS", args, 0)?.abs())),
    }
}

fn ceil(args: &[Literal]) -> KernelResult {
    arity("CEIL", args, 1, 1)?;
    Ok(Literal::Double(num("CEIL", args, 0)?.ceil()))
}

fn floor(args: &[Literal]) -> KernelResult {
    arity("FLOOR", args, 1, 1)?;
    Ok(Literal::Double(num("FLOOR", args, 0)?.floor()))
}

fn scale_of(name: &str, args: &[Literal]) -> Result<i32, KernelError> {
    if args.len() == 2 {
        Ok(int(name, args, 1)? as i32)
    } else {
        Ok(0)
    }
}

fn trunc(args: &[Literal]) -> KernelResult {
    arity("TRUNC", args, 1, 2)?;
    let v = num("TRUNC", args, 0)?;
    let m = 10f64.powi(scale_of("TRUNC", args)?);
    finite("TRUNC", (v * m).trunc() / m)
}

fn round(args: &[Literal]) -> KernelResult {
    arity("ROUND", args, 1, 2)?;
    let v = num("ROUND", args, 0)?;
    let m = 10f64.powi(scale_of("ROUND", args)?);
    finite("ROUND", (v * m).round() / m)
}

fn sign(args: &[Literal]) -> KernelResult {
    arity("SIGN", args, 1, 1)?;
    match &args[0] {
        Literal::Int(v) => Ok(Literal::Int(v.signum())),
        Literal::BigInt(v) => Ok(Literal::BigInt(v.signum())),
        _ => {
            let v = num("SIGN", args, 0)?;
            Ok(Literal::Double(if v > 0.0 {
                1.0
            } else if v < 0.0 {
                -1.0
            } else {
                0.0
            }))
        }
    }
}

fn sqrt(args: &[Literal]) -> KernelResult {
    arity("SQRT", args, 1, 1)?;
    let v = num("SQRT", args, 0)?;
    if v < 0.0 {
        return Err("SQRT: cannot take square root of a negative number".into());
    }
    Ok(Literal::Double(v.sqrt()))
}

fn cbrt(args: &[Literal]) -> KernelResult {
    arity("CBRT", args, 1, 1)?;
    Ok(Literal::Double(num("CBRT", args, 0)?.cbrt()))
}

fn power(args: &[Literal]) -> KernelResult {
    arity("POWER", args, 2, 2)?;
    finite("POWER", num("POWER", args, 0)?.powf(num("POWER", args, 1)?))
}

fn exp(args: &[Literal]) -> KernelResult {
    arity("EXP", args, 1, 1)?;
    finite("EXP", num("EXP", args, 0)?.exp())
}

fn positive(name: &str, v: f64) -> Result<f64, KernelError> {
    if v <= 0.0 {
        Err(format!(
            "{}: cannot take logarithm of a non-positive number",
            name
        ))
    } else {
        Ok(v)
    }
}

fn ln(args: &[Literal]) -> KernelResult {
    arity("LN", args, 1, 1)?;
    Ok(Literal::Double(positive("LN", num("LN", args, 0)?)?.ln()))
}

fn log10(args: &[Literal]) -> KernelResult {
    arity("LOG10", args, 1, 1)?;
    Ok(Literal::Double(
        positive("LOG10", num("LOG10", args, 0)?)?.log10(),
    ))
}

/// `LOG(x)` is base 10; `LOG(b, x)` is base `b` — PostgreSQL argument order.
fn log(args: &[Literal]) -> KernelResult {
    arity("LOG", args, 1, 2)?;
    if args.len() == 1 {
        return Ok(Literal::Double(
            positive("LOG", num("LOG", args, 0)?)?.log10(),
        ));
    }
    let base = positive("LOG", num("LOG", args, 0)?)?;
    let x = positive("LOG", num("LOG", args, 1)?)?;
    finite("LOG", x.ln() / base.ln())
}

fn modulo(args: &[Literal]) -> KernelResult {
    arity("MOD", args, 2, 2)?;
    match (&args[0], &args[1]) {
        (Literal::Int(_) | Literal::BigInt(_), Literal::Int(_) | Literal::BigInt(_)) => {
            let (a, b) = (int("MOD", args, 0)?, int("MOD", args, 1)?);
            if b == 0 {
                return Err("MOD: division by zero".into());
            }
            Ok(Literal::BigInt(a.wrapping_rem(b)))
        }
        _ => {
            let (a, b) = (num("MOD", args, 0)?, num("MOD", args, 1)?);
            if b == 0.0 {
                return Err("MOD: division by zero".into());
            }
            Ok(Literal::Double(a % b))
        }
    }
}

/// Integer quotient, truncated toward zero (PostgreSQL `div`).
fn div(args: &[Literal]) -> KernelResult {
    arity("DIV", args, 2, 2)?;
    let (a, b) = (num("DIV", args, 0)?, num("DIV", args, 1)?);
    if b == 0.0 {
        return Err("DIV: division by zero".into());
    }
    Ok(Literal::BigInt((a / b).trunc() as i64))
}

fn pi(args: &[Literal]) -> KernelResult {
    arity("PI", args, 0, 0)?;
    Ok(Literal::Double(std::f64::consts::PI))
}

/// Uniform in `[0, 1)`. A splitmix64 stream seeded from the OS on first use;
/// not cryptographic, same as PostgreSQL's.
fn random(args: &[Literal]) -> KernelResult {
    arity("RANDOM", args, 0, 0)?;
    use std::hash::{BuildHasher, Hasher, RandomState};
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let mut seed = STATE.load(AtomicOrdering::Relaxed);
    if seed == 0 {
        seed = RandomState::new().build_hasher().finish() | 1;
    }
    let next = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    STATE.store(next, AtomicOrdering::Relaxed);
    let mut z = next;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    Ok(Literal::Double((z >> 11) as f64 / (1u64 << 53) as f64))
}
