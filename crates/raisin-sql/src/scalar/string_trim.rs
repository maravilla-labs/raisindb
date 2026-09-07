//! Trimming and padding kernels: BTRIM (TRIM), LTRIM, RTRIM, LPAD, RPAD.
//!
//! The analyzer rewrites the `TRIM([BOTH|LEADING|TRAILING] [chars] FROM x)`
//! syntax onto these three, so there is exactly one trim implementation.

use super::args::{arity, int, text, KernelError};
use super::{Kernel, KernelCategory, KernelResult};
use crate::analyzer::Literal;

macro_rules! kernel {
    ($name:expr, $aliases:expr, $sig:expr, $f:expr) => {
        Kernel {
            name: $name,
            aliases: $aliases,
            category: KernelCategory::String,
            signature: $sig,
            deterministic: true,
            strict: true,
            func: $f,
        }
    };
}

pub(super) static KERNELS: &[Kernel] = &[
    kernel!("BTRIM", &["TRIM"], "BTRIM(text[, chars]) -> TEXT", btrim),
    kernel!("LTRIM", &[], "LTRIM(text[, chars]) -> TEXT", ltrim),
    kernel!("RTRIM", &[], "RTRIM(text[, chars]) -> TEXT", rtrim),
    kernel!("LPAD", &[], "LPAD(text, length[, fill]) -> TEXT", lpad),
    kernel!("RPAD", &[], "RPAD(text, length[, fill]) -> TEXT", rpad),
];

fn trim_chars<'a>(name: &str, args: &'a [Literal]) -> Result<Vec<char>, KernelError> {
    if args.len() == 2 {
        Ok(text(name, args, 1)?.chars().collect())
    } else {
        Ok(vec![' '])
    }
}

fn btrim(args: &[Literal]) -> KernelResult {
    arity("BTRIM", args, 1, 2)?;
    let chars = trim_chars("BTRIM", args)?;
    Ok(Literal::Text(
        text("BTRIM", args, 0)?
            .trim_matches(|c| chars.contains(&c))
            .to_string(),
    ))
}

fn ltrim(args: &[Literal]) -> KernelResult {
    arity("LTRIM", args, 1, 2)?;
    let chars = trim_chars("LTRIM", args)?;
    Ok(Literal::Text(
        text("LTRIM", args, 0)?
            .trim_start_matches(|c| chars.contains(&c))
            .to_string(),
    ))
}

fn rtrim(args: &[Literal]) -> KernelResult {
    arity("RTRIM", args, 1, 2)?;
    let chars = trim_chars("RTRIM", args)?;
    Ok(Literal::Text(
        text("RTRIM", args, 0)?
            .trim_end_matches(|c| chars.contains(&c))
            .to_string(),
    ))
}

/// Pad to `length` characters; a string longer than that is truncated, as in
/// PostgreSQL. An empty fill string pads nothing.
fn pad(name: &str, args: &[Literal], left: bool) -> KernelResult {
    arity(name, args, 2, 3)?;
    let s = text(name, args, 0)?;
    let target = int(name, args, 1)?.max(0) as usize;
    if target > 64 * 1024 * 1024 {
        return Err(format!("{}: requested length exceeds 64 MiB", name));
    }
    let fill: Vec<char> = if args.len() == 3 {
        text(name, args, 2)?.chars().collect()
    } else {
        vec![' ']
    };
    let chars: Vec<char> = s.chars().collect();
    if chars.len() >= target {
        return Ok(Literal::Text(chars[..target].iter().collect()));
    }
    let missing = target - chars.len();
    let padding: String = if fill.is_empty() {
        String::new()
    } else {
        fill.iter().cycle().take(missing).collect()
    };
    let out: String = if left {
        padding + s
    } else {
        s.to_string() + &padding
    };
    Ok(Literal::Text(out))
}

fn lpad(args: &[Literal]) -> KernelResult {
    pad("LPAD", args, true)
}

fn rpad(args: &[Literal]) -> KernelResult {
    pad("RPAD", args, false)
}
