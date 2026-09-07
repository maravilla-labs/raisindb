//! FORMAT kernel: PostgreSQL `format(fmt, args...)` with `%s`, `%I`, `%L`
//! and `%%`, plus the positional `%1$s` form. Width and flags are not
//! supported.

use super::args::{arity, text, to_text, KernelError};
use super::{Kernel, KernelCategory, KernelResult};
use crate::analyzer::Literal;

pub(super) static KERNELS: &[Kernel] = &[Kernel {
    name: "FORMAT",
    aliases: &[],
    category: KernelCategory::String,
    signature: "FORMAT(format, any, ...) -> TEXT",
    deterministic: true,
    strict: false,
    func: format,
}];

fn format(args: &[Literal]) -> KernelResult {
    arity("FORMAT", args, 1, usize::MAX)?;
    if matches!(args[0], Literal::Null) {
        return Ok(Literal::Null);
    }
    let fmt = text("FORMAT", args, 0)?;
    let values = &args[1..];
    let mut out = String::with_capacity(fmt.len());
    let mut next_arg = 0usize;
    let mut chars = fmt.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        // Optional explicit position `n$`.
        let mut digits = String::new();
        while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
            digits.push(*d);
            chars.next();
        }
        let position = if !digits.is_empty() {
            match chars.next() {
                Some('$') => Some(digits.parse::<usize>().unwrap_or(0)),
                _ => return Err("FORMAT: width specifiers are not supported".into()),
            }
        } else {
            None
        };
        let spec = chars
            .next()
            .ok_or_else(|| -> KernelError { "FORMAT: unterminated format specifier".into() })?;
        if spec == '%' {
            if position.is_some() {
                return Err("FORMAT: '%%' takes no position".into());
            }
            out.push('%');
            continue;
        }
        let index = match position {
            Some(0) => return Err("FORMAT: positions start at 1".into()),
            Some(p) => p - 1,
            None => {
                next_arg += 1;
                next_arg - 1
            }
        };
        let value = values.get(index).ok_or_else(|| -> KernelError {
            "FORMAT: too few arguments for format string".into()
        })?;
        match spec {
            's' => out.push_str(&to_text(value)),
            'I' => match value {
                Literal::Null => return Err("FORMAT: NULL cannot be used with %I".into()),
                other => out.push_str(&quote_identifier(&to_text(other))),
            },
            'L' => match value {
                Literal::Null => out.push_str("NULL"),
                other => out.push_str(&quote_literal(&to_text(other))),
            },
            other => {
                return Err(format!(
                    "FORMAT: unrecognised format specifier '%{}'",
                    other
                ))
            }
        }
    }
    Ok(Literal::Text(out))
}

/// Double-quote unless the name is a plain lowercase identifier.
fn quote_identifier(name: &str) -> String {
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !name.chars().next().is_some_and(|c| c.is_ascii_digit());
    if plain {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
