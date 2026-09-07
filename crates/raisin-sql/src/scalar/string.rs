//! String kernels: LOWER, UPPER, LENGTH, CONCAT, CONCAT_WS, SUBSTR,
//! REPLACE, STRPOS, LEFT, RIGHT, REPEAT, REVERSE, SPLIT_PART, STARTS_WITH,
//! ENDS_WITH, INITCAP, ASCII, CHR.
//!
//! All positions and lengths are in CHARACTERS, as in PostgreSQL, never bytes.

use super::args::{arity, char_window, int, text, to_text, KernelError};
use super::{Kernel, KernelCategory, KernelResult};
use crate::analyzer::Literal;

macro_rules! kernel {
    ($name:expr, $aliases:expr, $sig:expr, $f:expr) => {
        kernel!($name, $aliases, $sig, $f, true)
    };
    ($name:expr, $aliases:expr, $sig:expr, $f:expr, $strict:expr) => {
        Kernel {
            name: $name,
            aliases: $aliases,
            category: KernelCategory::String,
            signature: $sig,
            deterministic: true,
            strict: $strict,
            func: $f,
        }
    };
}

pub(super) static KERNELS: &[Kernel] = &[
    kernel!("LOWER", &[], "LOWER(text) -> TEXT", lower),
    kernel!("UPPER", &[], "UPPER(text) -> TEXT", upper),
    kernel!(
        "LENGTH",
        &["CHAR_LENGTH", "CHARACTER_LENGTH"],
        "LENGTH(text) -> INT",
        length
    ),
    kernel!("CONCAT", &[], "CONCAT(any, ...) -> TEXT", concat, false),
    kernel!(
        "CONCAT_WS",
        &[],
        "CONCAT_WS(separator, any, ...) -> TEXT",
        concat_ws,
        false
    ),
    kernel!(
        "SUBSTR",
        &["SUBSTRING"],
        "SUBSTR(text, start[, length]) -> TEXT",
        substr
    ),
    kernel!("REPLACE", &[], "REPLACE(text, from, to) -> TEXT", replace),
    kernel!("STRPOS", &[], "STRPOS(text, substring) -> INT", strpos),
    kernel!("LEFT", &[], "LEFT(text, n) -> TEXT", left),
    kernel!("RIGHT", &[], "RIGHT(text, n) -> TEXT", right),
    kernel!("REPEAT", &[], "REPEAT(text, n) -> TEXT", repeat),
    kernel!("REVERSE", &[], "REVERSE(text) -> TEXT", reverse),
    kernel!(
        "SPLIT_PART",
        &[],
        "SPLIT_PART(text, delimiter, n) -> TEXT",
        split_part
    ),
    kernel!(
        "STARTS_WITH",
        &[],
        "STARTS_WITH(text, prefix) -> BOOLEAN",
        starts_with
    ),
    kernel!(
        "ENDS_WITH",
        &[],
        "ENDS_WITH(text, suffix) -> BOOLEAN",
        ends_with
    ),
    kernel!("INITCAP", &[], "INITCAP(text) -> TEXT", initcap),
    kernel!("ASCII", &[], "ASCII(text) -> INT", ascii),
    kernel!("CHR", &[], "CHR(code) -> TEXT", chr),
];

fn lower(args: &[Literal]) -> KernelResult {
    arity("LOWER", args, 1, 1)?;
    Ok(Literal::Text(text("LOWER", args, 0)?.to_lowercase()))
}

fn upper(args: &[Literal]) -> KernelResult {
    arity("UPPER", args, 1, 1)?;
    Ok(Literal::Text(text("UPPER", args, 0)?.to_uppercase()))
}

fn length(args: &[Literal]) -> KernelResult {
    arity("LENGTH", args, 1, 1)?;
    Ok(Literal::Int(text("LENGTH", args, 0)?.chars().count() as i32))
}

/// NULL arguments are skipped, not propagated (PostgreSQL CONCAT).
fn concat(args: &[Literal]) -> KernelResult {
    arity("CONCAT", args, 1, usize::MAX)?;
    let mut out = String::new();
    for a in args {
        if !matches!(a, Literal::Null) {
            out.push_str(&to_text(a));
        }
    }
    Ok(Literal::Text(out))
}

/// A NULL separator yields NULL; NULL elements are skipped.
fn concat_ws(args: &[Literal]) -> KernelResult {
    arity("CONCAT_WS", args, 2, usize::MAX)?;
    if matches!(args[0], Literal::Null) {
        return Ok(Literal::Null);
    }
    let sep = text("CONCAT_WS", args, 0)?;
    let parts: Vec<String> = args[1..]
        .iter()
        .filter(|a| !matches!(a, Literal::Null))
        .map(to_text)
        .collect();
    Ok(Literal::Text(parts.join(sep)))
}

fn substr(args: &[Literal]) -> KernelResult {
    arity("SUBSTR", args, 2, 3)?;
    let s = text("SUBSTR", args, 0)?;
    let start = int("SUBSTR", args, 1)?;
    let len = if args.len() == 3 {
        Some(int("SUBSTR", args, 2)?)
    } else {
        None
    };
    let total = s.chars().count();
    let (begin, count) = char_window("SUBSTR", total, start, len)?;
    Ok(Literal::Text(s.chars().skip(begin).take(count).collect()))
}

fn replace(args: &[Literal]) -> KernelResult {
    arity("REPLACE", args, 3, 3)?;
    let s = text("REPLACE", args, 0)?;
    let from = text("REPLACE", args, 1)?;
    let to = text("REPLACE", args, 2)?;
    if from.is_empty() {
        return Ok(Literal::Text(s.to_string()));
    }
    Ok(Literal::Text(s.replace(from, to)))
}

/// 1-based character position of `sub` in `s`, 0 when absent.
pub(super) fn char_position(s: &str, sub: &str) -> i32 {
    match s.find(sub) {
        Some(byte_idx) => s[..byte_idx].chars().count() as i32 + 1,
        None => 0,
    }
}

fn strpos(args: &[Literal]) -> KernelResult {
    arity("STRPOS", args, 2, 2)?;
    Ok(Literal::Int(char_position(
        text("STRPOS", args, 0)?,
        text("STRPOS", args, 1)?,
    )))
}

/// Negative `n` drops the last `|n|` characters (PostgreSQL).
fn left(args: &[Literal]) -> KernelResult {
    arity("LEFT", args, 2, 2)?;
    let s = text("LEFT", args, 0)?;
    let n = int("LEFT", args, 1)?;
    let total = s.chars().count() as i64;
    let take = if n >= 0 { n } else { (total + n).max(0) };
    Ok(Literal::Text(s.chars().take(take as usize).collect()))
}

/// Negative `n` drops the first `|n|` characters (PostgreSQL).
fn right(args: &[Literal]) -> KernelResult {
    arity("RIGHT", args, 2, 2)?;
    let s = text("RIGHT", args, 0)?;
    let n = int("RIGHT", args, 1)?;
    let total = s.chars().count() as i64;
    let skip = if n >= 0 { (total - n).max(0) } else { -n };
    Ok(Literal::Text(s.chars().skip(skip as usize).collect()))
}

fn repeat(args: &[Literal]) -> KernelResult {
    arity("REPEAT", args, 2, 2)?;
    let s = text("REPEAT", args, 0)?;
    let n = int("REPEAT", args, 1)?.max(0) as usize;
    if s.len().saturating_mul(n) > 64 * 1024 * 1024 {
        return Err("REPEAT: result would exceed 64 MiB".into());
    }
    Ok(Literal::Text(s.repeat(n)))
}

fn reverse(args: &[Literal]) -> KernelResult {
    arity("REVERSE", args, 1, 1)?;
    Ok(Literal::Text(
        text("REVERSE", args, 0)?.chars().rev().collect(),
    ))
}

/// `n` counts from 1; negative `n` counts from the end. Out of range is ''.
fn split_part(args: &[Literal]) -> KernelResult {
    arity("SPLIT_PART", args, 3, 3)?;
    let s = text("SPLIT_PART", args, 0)?;
    let delim = text("SPLIT_PART", args, 1)?;
    let n = int("SPLIT_PART", args, 2)?;
    if n == 0 {
        return Err("SPLIT_PART: field position must not be zero".into());
    }
    if delim.is_empty() {
        return Ok(Literal::Text(if n == 1 || n == -1 {
            s.to_string()
        } else {
            String::new()
        }));
    }
    let parts: Vec<&str> = s.split(delim).collect();
    let idx = if n > 0 {
        (n - 1) as usize
    } else {
        match parts.len().checked_sub((-n) as usize) {
            Some(i) => i,
            None => return Ok(Literal::Text(String::new())),
        }
    };
    Ok(Literal::Text(
        parts.get(idx).map(|p| p.to_string()).unwrap_or_default(),
    ))
}

fn starts_with(args: &[Literal]) -> KernelResult {
    arity("STARTS_WITH", args, 2, 2)?;
    Ok(Literal::Boolean(
        text("STARTS_WITH", args, 0)?.starts_with(text("STARTS_WITH", args, 1)?),
    ))
}

fn ends_with(args: &[Literal]) -> KernelResult {
    arity("ENDS_WITH", args, 2, 2)?;
    Ok(Literal::Boolean(
        text("ENDS_WITH", args, 0)?.ends_with(text("ENDS_WITH", args, 1)?),
    ))
}

/// First letter of each word upper, the rest lower; a word is a run of
/// alphanumerics, matching PostgreSQL.
fn initcap(args: &[Literal]) -> KernelResult {
    arity("INITCAP", args, 1, 1)?;
    let mut out = String::new();
    let mut at_word_start = true;
    for c in text("INITCAP", args, 0)?.chars() {
        if c.is_alphanumeric() {
            if at_word_start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            at_word_start = false;
        } else {
            out.push(c);
            at_word_start = true;
        }
    }
    Ok(Literal::Text(out))
}

fn ascii(args: &[Literal]) -> KernelResult {
    arity("ASCII", args, 1, 1)?;
    Ok(Literal::Int(
        text("ASCII", args, 0)?
            .chars()
            .next()
            .map(|c| c as i32)
            .unwrap_or(0),
    ))
}

fn chr(args: &[Literal]) -> KernelResult {
    arity("CHR", args, 1, 1)?;
    let code = int("CHR", args, 0)?;
    let c = u32::try_from(code)
        .ok()
        .and_then(char::from_u32)
        .filter(|c| *c != '\0')
        .ok_or_else(|| -> KernelError { format!("CHR: {} is not a valid code point", code) })?;
    Ok(Literal::Text(c.to_string()))
}
